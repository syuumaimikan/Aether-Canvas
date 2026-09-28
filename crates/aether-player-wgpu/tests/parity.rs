//! The GPU renderer must draw what the reference software renderer draws.
//!
//! Needs a wgpu adapter; a software one (Mesa's lavapipe or llvmpipe) is
//! enough. Without any adapter the test says so and passes.

use aether_core::blend::BlendMode;
use aether_core::color::Rgba8;
use aether_core::math::{IRect, Rect, Vec2};
use aether_core::LayerId;
use aether_document::layer::Layer;
use aether_document::rig::{ArtMesh, Parameter, RigNode};
use aether_document::Document;
use aether_io::runtime_model::{export_model, ModelExportOptions};
use aether_player::{cpu, Player};
use aether_player_wgpu::{GpuPlayer, View};
use aether_raster::Pixmap;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let name = format!("{} ({:?})", adapter.get_info().name, adapter.get_info().backend);
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((device, queue, name))
}

fn target(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Read a target back as premultiplied RGBA bytes.
fn read(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, size: (u32, u32)) -> Vec<u8> {
    let row = (size.0 * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (row * size.1) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size.1),
            },
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let mapped = buffer.slice(..).get_mapped_range();
    let mut out = Vec::with_capacity((size.0 * size.1 * 4) as usize);
    for y in 0..size.1 as usize {
        let start = y * row as usize;
        out.extend_from_slice(&mapped[start..start + size.0 as usize * 4]);
    }
    out
}

fn premultiply(pixmap: &Pixmap) -> Vec<u8> {
    let mut out = pixmap.data().to_vec();
    for px in out.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    out
}

fn add_layer(doc: &mut Document, name: &str, rect: IRect, color: Rgba8, blend: BlendMode) -> LayerId {
    let id = doc.ids.layer();
    let mut layer = Layer::raster(id, name, doc.width, doc.height);
    layer.blend_mode = blend;
    if let Some(pixels) = layer.pixmap_mut() {
        pixels.fill_rect(rect, color);
        // A soft edge, so filtering and alpha are exercised too.
        for x in rect.x..rect.right() {
            pixels.set(x, rect.y, Rgba8::new(color.r, color.g, color.b, 90));
        }
    }
    doc.layers.insert(layer, None, usize::MAX).expect("insert");
    id
}

/// Every drawing path: a rigged mesh keyed for movement, opacity, tint and
/// draw order; a layer clipped to it; multiply, screen and add layers; and
/// translucency.
fn scene() -> Document {
    let mut doc = Document::empty(160, 120, "gpu");
    add_layer(
        &mut doc,
        "Back",
        IRect::new(0, 0, 160, 120),
        Rgba8::new(235, 225, 205, 255),
        BlendMode::Normal,
    );
    let arm = add_layer(
        &mut doc,
        "Arm",
        IRect::new(20, 30, 60, 26),
        Rgba8::new(210, 70, 70, 255),
        BlendMode::Normal,
    );
    let stripe = add_layer(
        &mut doc,
        "Stripe",
        IRect::new(0, 36, 160, 8),
        Rgba8::new(40, 40, 210, 230),
        BlendMode::Normal,
    );
    doc.layers.get_mut(stripe).unwrap().clipping = true;
    add_layer(
        &mut doc,
        "Shade",
        IRect::new(90, 20, 50, 70),
        Rgba8::new(120, 140, 255, 255),
        BlendMode::Multiply,
    );
    add_layer(
        &mut doc,
        "Glow",
        IRect::new(10, 70, 60, 40),
        Rgba8::new(200, 90, 20, 200),
        BlendMode::Screen,
    );
    add_layer(
        &mut doc,
        "Spark",
        IRect::new(100, 80, 40, 30),
        Rgba8::new(60, 60, 30, 255),
        BlendMode::Add,
    );
    let ghost = add_layer(
        &mut doc,
        "Ghost",
        IRect::new(60, 10, 40, 40),
        Rgba8::new(20, 160, 90, 255),
        BlendMode::Normal,
    );
    doc.layers.get_mut(ghost).unwrap().opacity = 0.45;

    let swing = doc
        .rig
        .add_parameter(Parameter::new(doc.ids.parameter(), "Swing", 0.0, 1.0, 0.0))
        .unwrap();
    let rect = Rect::from_min_size(Vec2::new(16.0, 26.0), Vec2::new(68.0, 34.0));
    let mut vertices = Vec::new();
    for j in 0..=3 {
        for i in 0..=6 {
            vertices.push(Vec2::new(
                rect.min.x + rect.width() * i as f32 / 6.0,
                rect.min.y + rect.height() * j as f32 / 3.0,
            ));
        }
    }
    let mut triangles = Vec::new();
    for j in 0..3u32 {
        for i in 0..6u32 {
            let a = j * 7 + i;
            triangles.push([a, a + 1, a + 8]);
            triangles.push([a, a + 8, a + 7]);
        }
    }
    doc.rig.set_mesh(ArtMesh::new(arm, vertices, triangles));
    doc.rig
        .bind_parameter(RigNode::Mesh(arm), swing, &[0.0, 1.0])
        .unwrap();
    let form = &mut doc.rig.mesh_mut(arm).unwrap().keyforms.forms[1];
    for (k, o) in form.offsets.iter_mut().enumerate() {
        // Bend: the far end rises.
        let column = (k % 7) as f32 / 6.0;
        *o = Vec2::new(18.0 * column, -24.0 * column * column);
    }
    form.opacity = 0.8;
    form.multiply = [1.0, 0.75, 0.6];
    form.screen = [0.0, 0.1, 0.2];
    form.draw_order = 6.0;
    doc
}

#[test]
fn the_gpu_draws_what_the_reference_renderer_draws() {
    let Some((device, queue, adapter)) = device() else {
        eprintln!("no wgpu adapter here; skipping");
        return;
    };
    eprintln!("adapter: {adapter}");
    let doc = scene();
    let export = export_model(&doc, &ModelExportOptions::default());
    assert!(export.warnings.is_empty(), "{:?}", export.warnings);
    let mut player = Player::new(export.model.clone()).unwrap();
    let size = (doc.width, doc.height);
    let mut gpu = GpuPlayer::new(&device, &queue, FORMAT, player.model(), &export.textures);
    let texture = target(&device, size);
    let view = texture.create_view(&Default::default());
    let swing = player.parameter_index("Swing").unwrap();

    for value in [0.0, 0.4, 1.0] {
        player.set_parameter(swing, value);
        player.update();
        gpu.render(
            &device,
            &queue,
            &player,
            &view,
            size,
            View::IDENTITY,
            Some([0.0; 4]),
        );
        let gpu_pixels = read(&device, &queue, &texture, size);
        let reference = premultiply(&cpu::render(&player, &export.textures));
        let (mut worst, mut total, mut over8) = (0u8, 0u64, 0usize);
        for (a, b) in gpu_pixels.iter().zip(&reference) {
            let d = a.abs_diff(*b);
            worst = worst.max(d);
            total += d as u64;
            if d > 8 {
                over8 += 1;
            }
        }
        let mean = total as f64 / reference.len() as f64;
        eprintln!("swing {value}: worst {worst}, mean {mean:.4}, over 8: {over8}");
        assert!(mean < 0.25, "swing {value}: mean difference {mean}");
        assert!(
            over8 * 1000 < reference.len(),
            "swing {value}: {over8} channels differ by more than 8"
        );
    }
}

#[test]
fn views_scale_and_place_the_canvas() {
    let Some((device, queue, _)) = device() else {
        eprintln!("no wgpu adapter here; skipping");
        return;
    };
    let doc = scene();
    let export = export_model(&doc, &ModelExportOptions::default());
    let player = Player::new(export.model.clone()).unwrap();
    // Twice as wide as the canvas's shape: centred, with empty bands.
    let size = (400, 120);
    let fit = View::fit(player.model(), size);
    assert_eq!(fit.scale, 1.0);
    assert_eq!(fit.offset, [120.0, 0.0]);
    let mut gpu = GpuPlayer::new(&device, &queue, FORMAT, player.model(), &export.textures);
    let texture = target(&device, size);
    let view = texture.create_view(&Default::default());
    gpu.render(&device, &queue, &player, &view, size, fit, Some([0.0; 4]));
    let pixels = read(&device, &queue, &texture, size);
    let alpha = |x: usize, y: usize| pixels[(y * size.0 as usize + x) * 4 + 3];
    assert_eq!(alpha(10, 60), 0, "left band empty");
    assert_eq!(alpha(390, 60), 0, "right band empty");
    assert_eq!(alpha(200, 60), 255, "canvas drawn in the middle");
}
