//! # aether-player-wgpu
//!
//! Draws [`aether_player`] models with [wgpu], on Vulkan, Metal, DirectX 12,
//! OpenGL or WebGPU. It is how the editor previews poses on the GPU, and the
//! way into Rust engines built on wgpu (Bevy, and anything else that hands
//! out a device, a queue and a render pass).
//!
//! ```no_run
//! # fn frame(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView,
//! #          player: &mut aether_player::Player, textures: &[aether_raster::Pixmap]) {
//! use aether_player_wgpu::{GpuPlayer, View};
//! let mut gpu = GpuPlayer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm, player.model(), textures);
//! player.tick(1.0 / 60.0);
//! let size = (1024, 768);
//! gpu.render(device, queue, player, target, size, View::fit(player.model(), size), Some([0.0; 4]));
//! # }
//! ```
//!
//! Blending matches the reference software renderer: textures are
//! premultiplied on upload and every blend is premultiplied (multiply takes
//! two passes to be exact over translucent pixels), and clipping masks are
//! rendered into a texture array and read back pixel for pixel. Render into
//! a non-sRGB target (`Rgba8Unorm` or `Bgra8Unorm`) to blend in the same
//! space as the editor.

use aether_player::{BlendKind, Model, Player};
use aether_raster::Pixmap;

/// Size of one draw's uniform slot: the largest dynamic-offset alignment
/// any backend asks for.
const SLOT: u64 = 256;
/// Bytes of uniform data per draw (four vec4s).
const DRAW_BYTES: usize = 64;
/// Format of the clipping-mask layers.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How document pixels land on the target: `target = document · scale + offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Target pixels per document pixel.
    pub scale: f32,
    /// Where the document's origin lands, target pixels.
    pub offset: [f32; 2],
}

impl View {
    /// Document pixels are target pixels.
    pub const IDENTITY: View = View {
        scale: 1.0,
        offset: [0.0, 0.0],
    };

    /// The whole canvas, as large as fits, centred.
    pub fn fit(model: &Model, target: (u32, u32)) -> Self {
        let (w, h) = (model.width.max(1) as f32, model.height.max(1) as f32);
        let scale = (target.0 as f32 / w).min(target.1 as f32 / h);
        View {
            scale,
            offset: [
                (target.0 as f32 - w * scale) * 0.5,
                (target.1 as f32 - h * scale) * 0.5,
            ],
        }
    }
}

struct GpuPart {
    positions: wgpu::Buffer,
    uvs: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    texture: usize,
}

struct Pipelines {
    normal: wgpu::RenderPipeline,
    multiply_colour: wgpu::RenderPipeline,
    multiply_alpha: wgpu::RenderPipeline,
    screen: wgpu::RenderPipeline,
    add: wgpu::RenderPipeline,
    mask: wgpu::RenderPipeline,
}

struct MaskArray {
    texture: wgpu::Texture,
    group: wgpu::BindGroup,
    size: (u32, u32),
    layers: u32,
}

/// A draw call planned by [`GpuPlayer::prepare`].
struct Planned {
    part: usize,
    slot: u32,
    blend: BlendKind,
}

/// A model's GPU resources, and the frame last prepared.
pub struct GpuPlayer {
    parts: Vec<GpuPart>,
    textures: Vec<wgpu::BindGroup>,
    pipelines: Pipelines,
    draw_layout: wgpu::BindGroupLayout,
    mask_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    uniform_slots: u64,
    draw_group: wgpu::BindGroup,
    /// Bound when a draw has no mask, and while masks themselves are drawn.
    no_mask: wgpu::BindGroup,
    masks: Option<MaskArray>,
    planned: Vec<Planned>,
    use_masks: bool,
}

fn premultiplied(pixmap: &Pixmap) -> Vec<u8> {
    let mut out = pixmap.data().to_vec();
    for px in out.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    out
}

fn blend(
    color: (wgpu::BlendFactor, wgpu::BlendFactor),
    alpha: (wgpu::BlendFactor, wgpu::BlendFactor),
) -> wgpu::BlendState {
    let component = |(src_factor, dst_factor)| wgpu::BlendComponent {
        src_factor,
        dst_factor,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component(color),
        alpha: component(alpha),
    }
}

impl GpuPlayer {
    /// Upload a model: its meshes and texture pages (decoded, in
    /// `model.textures` order). `format` is the format of the targets it
    /// will draw into.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        model: &Model,
        textures: &[Pixmap],
    ) -> Self {
        use wgpu::util::DeviceExt;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("aether player"),
            source: wgpu::ShaderSource::Wgsl(include_str!("player.wgsl").into()),
        });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aether draw"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(DRAW_BYTES as u64),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aether texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aether masks"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("aether player"),
            bind_group_layouts: &[Some(&draw_layout), Some(&texture_layout), Some(&mask_layout)],
            immediate_size: 0,
        });

        let vertex_buffers = [
            wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2],
            },
            wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![1 => Float32x2],
            },
        ];
        let pipeline = |label: &str, entry: &str, target: wgpu::TextureFormat, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &vertex_buffers,
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::BlendFactor as F;
        let over = (F::One, F::OneMinusSrcAlpha);
        let pipelines = Pipelines {
            normal: pipeline("aether normal", "fs_part", format, blend(over, over)),
            // Premultiplied multiply is Cs·Cd + Cs·(1−αd) + Cd·(1−αs): the
            // first pass adds Cs·Cd + Cd·(1−αs) leaving alpha alone, so the
            // second still sees αd when it adds Cs·(1−αd) and the alpha.
            multiply_colour: pipeline(
                "aether multiply 1",
                "fs_part",
                format,
                blend((F::Dst, F::OneMinusSrcAlpha), (F::Zero, F::One)),
            ),
            multiply_alpha: pipeline(
                "aether multiply 2",
                "fs_part",
                format,
                blend((F::OneMinusDstAlpha, F::One), over),
            ),
            screen: pipeline(
                "aether screen",
                "fs_part",
                format,
                blend((F::One, F::OneMinusSrc), over),
            ),
            add: pipeline("aether add", "fs_part", format, blend((F::One, F::One), over)),
            mask: pipeline("aether mask", "fs_mask", MASK_FORMAT, blend(over, over)),
        };

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("aether texels"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let textures = textures
            .iter()
            .map(|pixmap| {
                let size = wgpu::Extent3d {
                    width: pixmap.width().max(1),
                    height: pixmap.height().max(1),
                    depth_or_array_layers: 1,
                };
                let texture = device.create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some("aether texture page"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &if pixmap.is_empty() {
                        vec![0; 4]
                    } else {
                        premultiplied(pixmap)
                    },
                );
                let view = texture.create_view(&Default::default());
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("aether texture page"),
                    layout: &texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                })
            })
            .collect();

        let parts = model
            .parts
            .iter()
            .map(|part| {
                let uvs: Vec<f32> = part.uvs.iter().flat_map(|uv| [uv.x, uv.y]).collect();
                let indices: Vec<u32> = part.triangles.iter().flatten().copied().collect();
                GpuPart {
                    positions: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("aether positions"),
                        size: (part.vertices.len().max(1) * 8) as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    uvs: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("aether uvs"),
                        contents: bytemuck::cast_slice(if uvs.is_empty() { &[0.0f32; 2] } else { &uvs }),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("aether indices"),
                        contents: bytemuck::cast_slice(if indices.is_empty() {
                            &[0u32; 3]
                        } else {
                            &indices
                        }),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    index_count: indices.len() as u32,
                    texture: part.texture as usize,
                }
            })
            .collect();

        let (uniforms, draw_group) = Self::uniform_buffer(device, &draw_layout, 64);
        let no_mask = Self::mask_array(device, &mask_layout, (1, 1), 1).group;
        Self {
            parts,
            textures,
            pipelines,
            draw_layout,
            mask_layout,
            uniforms,
            uniform_slots: 64,
            draw_group,
            no_mask,
            masks: None,
            planned: Vec::new(),
            use_masks: false,
        }
    }

    fn uniform_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        slots: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("aether draws"),
            size: slots * SLOT,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("aether draws"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(DRAW_BYTES as u64),
                }),
            }],
        });
        (buffer, group)
    }

    fn mask_array(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        size: (u32, u32),
        layers: u32,
    ) -> MaskArray {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("aether masks"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: layers.max(1),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MASK_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("aether masks"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        MaskArray {
            texture,
            group,
            size,
            layers,
        }
    }

    /// Upload the player's current pose and plan the frame: positions,
    /// per-draw uniforms, and the clipping masks, which are rendered here
    /// into `encoder` (they need passes of their own). Call before
    /// [`GpuPlayer::paint`]; `target_size` is the size of the target the
    /// paint pass draws into.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        player: &Player,
        target_size: (u32, u32),
        view: View,
    ) {
        for (i, part) in self.parts.iter().enumerate() {
            let positions = player.positions(i);
            if !positions.is_empty() {
                queue.write_buffer(&part.positions, 0, bytemuck::cast_slice(positions));
            }
        }
        let (w, h) = (target_size.0.max(1) as f32, target_size.1.max(1) as f32);
        let clip = [
            2.0 * view.scale / w,
            -2.0 * view.scale / h,
            2.0 * view.offset[0] / w - 1.0,
            1.0 - 2.0 * view.offset[1] / h,
        ];

        let items: Vec<_> = player
            .draw_list()
            .iter()
            .filter(|d| d.opacity > 0.0 && (d.part as usize) < self.parts.len())
            .collect();
        // One mask layer per distinct (part, opacity) in use.
        let mut mask_keys: Vec<(u32, u32)> = Vec::new();
        for item in &items {
            if let Some(base) = item.mask_part() {
                let key = (base as u32, item.mask_opacity.to_bits());
                if !mask_keys.contains(&key) {
                    mask_keys.push(key);
                }
            }
        }

        let slots = (items.len() + mask_keys.len()) as u64;
        if slots > self.uniform_slots {
            let capacity = slots.next_power_of_two();
            let (buffer, group) = Self::uniform_buffer(device, &self.draw_layout, capacity);
            self.uniforms = buffer;
            self.draw_group = group;
            self.uniform_slots = capacity;
        }
        let mut bytes = vec![0u8; (slots.max(1) * SLOT) as usize];
        let mut write = |slot: usize, values: [f32; 16]| {
            let at = slot * SLOT as usize;
            bytes[at..at + DRAW_BYTES].copy_from_slice(bytemuck::cast_slice(&values));
        };
        self.planned.clear();
        for (slot, item) in items.iter().enumerate() {
            let layer = item.mask_part().and_then(|base| {
                mask_keys
                    .iter()
                    .position(|k| *k == (base as u32, item.mask_opacity.to_bits()))
            });
            write(
                slot,
                [
                    clip[0],
                    clip[1],
                    clip[2],
                    clip[3],
                    item.multiply[0],
                    item.multiply[1],
                    item.multiply[2],
                    item.opacity,
                    item.screen[0],
                    item.screen[1],
                    item.screen[2],
                    if layer.is_some() { 1.0 } else { 0.0 },
                    layer.unwrap_or(0) as f32,
                    0.0,
                    0.0,
                    0.0,
                ],
            );
            self.planned.push(Planned {
                part: item.part as usize,
                slot: slot as u32,
                blend: item.blend_kind(),
            });
        }
        for (i, (_, opacity)) in mask_keys.iter().enumerate() {
            let opacity = f32::from_bits(*opacity);
            write(
                items.len() + i,
                [
                    clip[0], clip[1], clip[2], clip[3], 1.0, 1.0, 1.0, opacity, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                    0.0, 0.0,
                ],
            );
        }
        queue.write_buffer(&self.uniforms, 0, &bytes);

        self.use_masks = !mask_keys.is_empty();
        if !self.use_masks {
            return;
        }
        let layers = mask_keys.len() as u32;
        let fits = self
            .masks
            .as_ref()
            .is_some_and(|m| m.size == target_size && m.layers >= layers);
        if !fits {
            self.masks = Some(Self::mask_array(
                device,
                &self.mask_layout,
                target_size,
                layers.next_power_of_two(),
            ));
        }
        let masks = self.masks.as_ref().expect("mask array");
        for (layer, (base, _)) in mask_keys.iter().enumerate() {
            let view = masks.texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("aether mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines.mask);
            self.draw_part(
                &mut pass,
                *base as usize,
                (items.len() + layer) as u32,
                &self.no_mask,
            );
        }
    }

    fn draw_part(&self, pass: &mut wgpu::RenderPass<'_>, part: usize, slot: u32, masks: &wgpu::BindGroup) {
        let gpu = &self.parts[part];
        let Some(texture) = self.textures.get(gpu.texture) else {
            return;
        };
        if gpu.index_count == 0 {
            return;
        }
        pass.set_bind_group(0, &self.draw_group, &[slot * SLOT as u32]);
        pass.set_bind_group(1, texture, &[]);
        pass.set_bind_group(2, masks, &[]);
        pass.set_vertex_buffer(0, gpu.positions.slice(..));
        pass.set_vertex_buffer(1, gpu.uvs.slice(..));
        pass.set_index_buffer(gpu.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..gpu.index_count, 0, 0..1);
    }

    /// Draw the prepared frame into a render pass over a target of the size
    /// given to [`GpuPlayer::prepare`] and the format given to
    /// [`GpuPlayer::new`].
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
        let masks = match (&self.masks, self.use_masks) {
            (Some(m), true) => &m.group,
            _ => &self.no_mask,
        };
        for draw in &self.planned {
            let pipelines: &[&wgpu::RenderPipeline] = match draw.blend {
                BlendKind::Normal => &[&self.pipelines.normal],
                BlendKind::Multiply => &[&self.pipelines.multiply_colour, &self.pipelines.multiply_alpha],
                BlendKind::Screen => &[&self.pipelines.screen],
                BlendKind::Add => &[&self.pipelines.add],
            };
            for pipeline in pipelines {
                pass.set_pipeline(pipeline);
                self.draw_part(pass, draw.part, draw.slot, masks);
            }
        }
    }

    /// Prepare and paint in one go, into `target`, clearing it to `clear`
    /// (premultiplied RGBA) first unless it is `None`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        player: &Player,
        target: &wgpu::TextureView,
        target_size: (u32, u32),
        view: View,
        clear: Option<[f64; 4]>,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("aether frame"),
        });
        self.prepare(device, queue, &mut encoder, player, target_size, view);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("aether frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: match clear {
                            Some([r, g, b, a]) => wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                            None => wgpu::LoadOp::Load,
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.paint(&mut pass);
        }
        queue.submit([encoder.finish()]);
    }
}
