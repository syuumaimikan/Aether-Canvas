//! GPU pose preview.
//!
//! Posing or playing a rig changes every deformed layer every frame, and
//! re-compositing that on the CPU is what limits the frame rate on large
//! canvases. While a rig tool is in use, the canvas instead shows the pose
//! drawn on the GPU by [`aether_player_wgpu`], from a runtime model of the
//! document kept in step with every edit; the composite catches up once when
//! the preview ends.
//!
//! The preview stands in only when it is exact: the document must export
//! with no approximations and use no effects, masks or layer transforms
//! (which the runtime bakes rather than applies after deformation). Anything
//! else keeps the CPU composite, so what the canvas shows never changes
//! depending on how it was drawn.

use crate::state::EditorState;
use crate::tools::ToolId;
use aether_document::layer::Layer;
use aether_io::runtime_model::{export_model, ModelExportOptions};
use aether_player::Player;
use aether_player_wgpu::{GpuPlayer, View};
use eframe::egui_wgpu;
use std::sync::Arc;

/// Format of the preview texture: plain 8-bit, blended as the compositor does.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// What a model was built from: the edit history's revision and the canvas size.
type Key = (u64, u32, u32);

struct Built {
    key: Key,
    player: Player,
    gpu: GpuPlayer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

/// Draws rig poses on the GPU for the canvas.
pub struct GpuPreview {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    built: Option<Built>,
    texture: Option<egui::TextureId>,
    /// A document state that cannot be previewed exactly; not retried until
    /// something changes.
    rejected: Option<Key>,
}

/// True when the runtime would draw a layer differently from the compositor.
fn inexact(layer: &Layer) -> bool {
    layer.visible && (layer.has_effects() || layer.active_mask().is_some() || !layer.transform.is_identity())
}

impl GpuPreview {
    /// A preview drawing with `device`, showing its frames through egui's
    /// `renderer`.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    ) -> Self {
        Self {
            device,
            queue,
            renderer,
            built: None,
            texture: None,
            rejected: None,
        }
    }

    /// A preview on eframe's own device.
    pub fn from_render_state(render_state: &egui_wgpu::RenderState) -> Self {
        Self::new(
            render_state.device.clone(),
            render_state.queue.clone(),
            render_state.renderer.clone(),
        )
    }

    /// Whether posing is what the user is doing: a rig tool is in hand (or
    /// the view is being panned) and the canvas shows poses, not the rest
    /// view.
    fn wanted(state: &EditorState) -> bool {
        !state.rig.rest_view
            && !state.doc.rig.is_inert()
            && matches!(
                state.tools.active_id(),
                ToolId::Deform | ToolId::Bone | ToolId::Pan
            )
    }

    /// Draw the current pose and return the texture to show, or `None` when
    /// the canvas should show the CPU composite instead.
    pub fn refresh(&mut self, state: &EditorState) -> Option<egui::TextureId> {
        if !Self::wanted(state) {
            return None;
        }
        let key = (state.history.revision(), state.doc.width, state.doc.height);
        if self.rejected == Some(key) {
            return None;
        }
        let current = match &mut self.built {
            // Same document state; take the rig's current pose and edits.
            Some(built) => built.key == key && built.player.sync_rig(&state.doc.rig),
            None => false,
        };
        if !current && !self.build(state, key) {
            self.rejected = Some(key);
            return None;
        }
        let built = self.built.as_mut()?;
        let clear = match state.doc.background {
            aether_document::Background::Solid(c) => {
                let a = c.a as f64 / 255.0;
                [
                    c.r as f64 / 255.0 * a,
                    c.g as f64 / 255.0 * a,
                    c.b as f64 / 255.0 * a,
                    a,
                ]
            }
            aether_document::Background::Transparent => [0.0; 4],
        };
        built.gpu.render(
            &self.device,
            &self.queue,
            &built.player,
            &built.view,
            built.size,
            View::IDENTITY,
            Some(clear),
        );
        self.texture
    }

    /// Build a model of the document as it is now. False when it cannot be
    /// previewed exactly.
    fn build(&mut self, state: &EditorState, key: Key) -> bool {
        if state.doc.layers.iter().any(inexact) {
            return false;
        }
        let export = export_model(&state.doc, &ModelExportOptions::default());
        if !export.warnings.is_empty() {
            return false;
        }
        let Ok(mut player) = Player::new(export.model) else {
            return false;
        };
        if !player.sync_rig(&state.doc.rig) {
            return false;
        }
        let gpu = GpuPlayer::new(
            &self.device,
            &self.queue,
            FORMAT,
            player.model(),
            &export.textures,
        );
        let size = (state.doc.width.max(1), state.doc.height.max(1));
        let reuse = self.built.take().filter(|b| b.size == size);
        let (target, view) = match reuse {
            Some(old) => (old.target, old.view),
            None => {
                let target = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("aether pose preview"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = target.create_view(&Default::default());
                let filter = match state.view_interpolation {
                    aether_raster::transform::Interpolation::Nearest => wgpu::FilterMode::Nearest,
                    aether_raster::transform::Interpolation::Bilinear => wgpu::FilterMode::Linear,
                };
                let mut renderer = self.renderer.write();
                match self.texture {
                    Some(id) => {
                        renderer.update_egui_texture_from_wgpu_texture(&self.device, &view, filter, id)
                    }
                    None => {
                        self.texture = Some(renderer.register_native_texture(&self.device, &view, filter))
                    }
                }
                (target, view)
            }
        };
        self.built = Some(Built {
            key,
            player,
            gpu,
            target,
            view,
            size,
        });
        true
    }

    /// The texture the last frame was drawn into.
    pub fn target(&self) -> Option<&wgpu::Texture> {
        self.built.as_ref().map(|b| &b.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;
    use aether_core::math::IRect;
    use aether_document::command::LayerProperty;
    use aether_document::Document;
    use aether_render::Compositor;

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    fn preview() -> Option<GpuPreview> {
        let (device, queue) = gpu()?;
        let renderer = egui_wgpu::Renderer::new(&device, FORMAT, egui_wgpu::RendererOptions::default());
        Some(GpuPreview::new(
            device,
            queue,
            Arc::new(egui::mutex::RwLock::new(renderer)),
        ))
    }

    /// A small face, auto-rigged, posed, with the Deform tool in hand.
    fn posed_state() -> EditorState {
        let mut doc = Document::empty(160, 200, "face");
        doc.background = aether_document::Background::Solid(Rgba8::new(240, 236, 228, 255));
        let mut state = EditorState::new(doc);
        for (name, rect, color) in [
            (
                "Face",
                IRect::new(40, 40, 80, 100),
                Rgba8::new(250, 222, 200, 255),
            ),
            ("Eye L", IRect::new(58, 80, 16, 10), Rgba8::new(40, 70, 170, 255)),
            ("Eye R", IRect::new(88, 80, 16, 10), Rgba8::new(40, 70, 170, 255)),
            ("Mouth", IRect::new(70, 115, 22, 5), Rgba8::new(180, 60, 70, 255)),
            (
                "Front hair",
                IRect::new(36, 30, 90, 30),
                Rgba8::new(90, 60, 150, 255),
            ),
        ] {
            let id = state.add_layer().expect("layer");
            let layer = state.doc.layers.get_mut(id).unwrap();
            layer.name = name.to_string();
            layer.pixmap_mut().unwrap().fill_rect(rect, color);
        }
        state.auto_rig().expect("auto rig");
        for (name, value) in [
            ("AngleX", 25.0),
            ("AngleZ", -12.0),
            ("EyeLOpen", 0.2),
            ("MouthOpenY", 0.8),
        ] {
            let id = state.doc.rig.parameter_named(name).unwrap().id;
            state.doc.rig.set_value(id, value);
        }
        state.select_tool(ToolId::Deform);
        state.tick_rig(0.0);
        state
    }

    fn read(preview: &GpuPreview, size: (u32, u32)) -> Vec<u8> {
        let texture = preview.target().expect("a frame");
        let row = (size.0 * 4).div_ceil(256) * 256;
        let buffer = preview.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * size.1) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = preview.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size.1),
                },
            },
            texture.size(),
        );
        preview.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        preview
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        let mapped = buffer.slice(..).get_mapped_range();
        (0..size.1 as usize)
            .flat_map(|y| mapped[y * row as usize..y * row as usize + size.0 as usize * 4].to_vec())
            .collect()
    }

    #[test]
    fn the_preview_shows_what_the_compositor_shows() {
        let Some(mut preview) = preview() else {
            eprintln!("no wgpu adapter here; skipping");
            return;
        };
        let state = posed_state();
        assert!(
            preview.refresh(&state).is_some(),
            "an exact document previews on the GPU"
        );
        let gpu = read(&preview, (state.doc.width, state.doc.height));
        let cpu = Compositor::new().render(&state.doc);
        let (mut total, mut over8) = (0u64, 0usize);
        for (g, c) in gpu.chunks_exact(4).zip(cpu.data().chunks_exact(4)) {
            // The background is opaque, so straight and premultiplied agree.
            for k in 0..4 {
                let d = g[k].abs_diff(c[k]);
                total += d as u64;
                over8 += (d > 8) as usize;
            }
        }
        let mean = total as f64 / gpu.len() as f64;
        assert!(
            mean < 0.25 && over8 * 1000 < gpu.len(),
            "mean {mean}, over 8: {over8}"
        );
    }

    #[test]
    fn the_preview_follows_poses_and_edits() {
        let Some(mut preview) = preview() else {
            return;
        };
        let mut state = posed_state();
        preview.refresh(&state).expect("preview");
        let size = (state.doc.width, state.doc.height);
        let before = read(&preview, size);
        let angle = state.doc.rig.parameter_named("AngleX").unwrap().id;
        state.doc.rig.set_value(angle, -25.0);
        // As in the editor's frame loop: the rig ticks, then the canvas draws.
        state.tick_rig(0.0);
        preview.refresh(&state).expect("preview");
        assert!(read(&preview, size) != before, "a new pose draws");

        // An edit to the artwork rebuilds the preview from it.
        let face = state.doc.layers.iter().find(|l| l.name == "Face").unwrap().id;
        let before = read(&preview, size);
        state
            .set_layer_property(face, LayerProperty::Opacity(0.5))
            .expect("edit");
        state.tick_rig(0.0);
        preview.refresh(&state).expect("preview");
        assert!(read(&preview, size) != before, "an edit rebuilds the preview");
    }

    #[test]
    fn inexact_documents_and_painting_keep_the_cpu_composite() {
        let Some(mut preview) = preview() else {
            return;
        };
        let mut state = posed_state();
        state.select_tool(ToolId::Brush);
        assert!(
            preview.refresh(&state).is_none(),
            "painting shows the live composite"
        );
        state.select_tool(ToolId::Deform);
        assert!(preview.refresh(&state).is_some());
        let face = state.doc.layers.iter().find(|l| l.name == "Face").unwrap().id;
        state
            .set_layer_property(face, LayerProperty::Blend(aether_core::blend::BlendMode::Overlay))
            .expect("edit");
        assert!(
            preview.refresh(&state).is_none(),
            "overlay has no exact GPU equivalent"
        );
    }

    #[test]
    fn the_composite_catches_up_when_the_preview_ends() {
        let mut state = posed_state();
        state.refresh();
        state.rig.gpu_preview = true;
        let angle = state.doc.rig.parameter_named("AngleX").unwrap().id;
        state.doc.rig.set_value(angle, -20.0);
        state.tick_rig(0.0);
        assert!(state.doc.dirty().is_empty(), "the GPU draws poses; no CPU work");
        state.rig.gpu_preview = false;
        state.tick_rig(0.0);
        assert_eq!(state.doc.dirty(), state.doc.bounds(), "one full catch-up");
    }
}
