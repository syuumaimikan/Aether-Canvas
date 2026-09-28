//! Render the editor to a PNG without a window or a GPU.
//!
//! egui produces textured, vertex-coloured triangles; this example runs the
//! whole application for a few frames, tessellates the last one and
//! rasterises it in software. It is how the rigging UI is checked visually
//! on machines with no display — and how the screenshots in the docs are
//! made.
//!
//! ```text
//! cargo run --release --example rig_demo -- demo
//! cargo run --release --example ui_screenshot -- demo/aether-chan.aether rigging shot.png
//! ```
//!
//! The second argument picks the workspace: `rigging`, `animation`,
//! `illustration`, `pixel`, `compositing`. A fourth argument `ja` renders the
//! Japanese UI (using the system CJK font, as the application does).

use aether_core::color::Rgba8;
use aether_raster::Pixmap;
use aether_ui::state::Workspace;
use aether_ui::AetherApp;
use egui::epaint::{ImageData, Primitive};
use egui::TextureId;
use std::collections::HashMap;

const WIDTH: f32 = 1600.0;
const HEIGHT: f32 = 1000.0;

/// A premultiplied RGBA texture.
struct Texture {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 4]>,
}

impl Texture {
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = (u * self.width as f32 - 0.5).clamp(0.0, self.width as f32 - 1.0);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, self.height as f32 - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| self.pixels[y * self.width + x];
        std::array::from_fn(|c| {
            let top = at(x0, y0)[c] * (1.0 - tx) + at(x1, y0)[c] * tx;
            let bottom = at(x0, y1)[c] * (1.0 - tx) + at(x1, y1)[c] * tx;
            top * (1.0 - ty) + bottom * ty
        })
    }
}

fn apply_delta(textures: &mut HashMap<TextureId, Texture>, id: TextureId, delta: &egui::epaint::ImageDelta) {
    let ImageData::Color(image) = &delta.image;
    let pixels: Vec<[f32; 4]> = image
        .pixels
        .iter()
        .map(|c| {
            [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
                c.a() as f32 / 255.0,
            ]
        })
        .collect();
    match delta.pos {
        None => {
            textures.insert(
                id,
                Texture {
                    width: image.size[0],
                    height: image.size[1],
                    pixels,
                },
            );
        }
        Some([px, py]) => {
            if let Some(texture) = textures.get_mut(&id) {
                for y in 0..image.size[1] {
                    for x in 0..image.size[0] {
                        let (tx, ty) = (px + x, py + y);
                        if tx < texture.width && ty < texture.height {
                            texture.pixels[ty * texture.width + tx] = pixels[y * image.size[0] + x];
                        }
                    }
                }
            }
        }
    }
}

fn rasterize(
    frame: &mut [[f32; 4]],
    width: usize,
    height: usize,
    primitives: &[egui::ClippedPrimitive],
    textures: &HashMap<TextureId, Texture>,
) {
    for primitive in primitives {
        let Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        let Some(texture) = textures.get(&mesh.texture_id) else {
            continue;
        };
        let clip = primitive.clip_rect;
        let cx0 = clip.min.x.max(0.0) as i32;
        let cy0 = clip.min.y.max(0.0) as i32;
        let cx1 = (clip.max.x.min(width as f32)).ceil() as i32;
        let cy1 = (clip.max.y.min(height as f32)).ceil() as i32;
        for tri in mesh.indices.chunks_exact(3) {
            let v = [
                mesh.vertices[tri[0] as usize],
                mesh.vertices[tri[1] as usize],
                mesh.vertices[tri[2] as usize],
            ];
            let area = (v[1].pos.x - v[0].pos.x) * (v[2].pos.y - v[0].pos.y)
                - (v[1].pos.y - v[0].pos.y) * (v[2].pos.x - v[0].pos.x);
            if area.abs() < 1e-6 {
                continue;
            }
            let min_x = v
                .iter()
                .map(|p| p.pos.x)
                .fold(f32::MAX, f32::min)
                .floor()
                .max(cx0 as f32) as i32;
            let max_x = v
                .iter()
                .map(|p| p.pos.x)
                .fold(f32::MIN, f32::max)
                .ceil()
                .min(cx1 as f32) as i32;
            let min_y = v
                .iter()
                .map(|p| p.pos.y)
                .fold(f32::MAX, f32::min)
                .floor()
                .max(cy0 as f32) as i32;
            let max_y = v
                .iter()
                .map(|p| p.pos.y)
                .fold(f32::MIN, f32::max)
                .ceil()
                .min(cy1 as f32) as i32;
            for y in min_y..max_y {
                for x in min_x..max_x {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((v[1].pos.x - px) * (v[2].pos.y - py) - (v[1].pos.y - py) * (v[2].pos.x - px))
                        / area;
                    let w1 = ((v[2].pos.x - px) * (v[0].pos.y - py) - (v[2].pos.y - py) * (v[0].pos.x - px))
                        / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                    let t = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                    let texel = texture.sample(u, t);
                    let mut src = [0.0f32; 4];
                    for c in 0..4 {
                        let vc = |i: usize| v[i].color.to_array()[c] as f32 / 255.0;
                        src[c] = texel[c] * (w0 * vc(0) + w1 * vc(1) + w2 * vc(2));
                    }
                    let dst = &mut frame[y as usize * width + x as usize];
                    for c in 0..4 {
                        dst[c] = src[c] + dst[c] * (1.0 - src[3]);
                    }
                }
            }
        }
    }
}

fn main() -> aether_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let project = args.next();
    let workspace = match args.next().as_deref() {
        Some("animation") => Workspace::Animation,
        Some("illustration") => Workspace::Illustration,
        Some("pixel") => Workspace::PixelArt,
        Some("compositing") => Workspace::Compositing,
        _ => Workspace::Rigging,
    };
    let out = args.next().unwrap_or_else(|| "aether-ui.png".into());
    let japanese = args.next().as_deref() == Some("ja");

    let doc = match &project {
        Some(path) => aether_io::load_project(path)?,
        None => aether_document::Document::new(800, 600, "Untitled"),
    };
    let mut app = AetherApp::with_document(doc);
    if japanese {
        app.state_mut().language = aether_ui::Language::Japanese;
    }
    app.set_workspace(workspace);
    {
        use aether_document::rig::RigNode;
        let state = app.state_mut();
        let set = |state: &mut aether_ui::EditorState, name: &str, v: f32| {
            if let Some(id) = state.doc.rig.parameter_named(name).map(|p| p.id) {
                state.doc.rig.set_value(id, v);
            }
        };
        set(state, "AngleX", 18.0);
        set(state, "AngleY", 6.0);
        set(state, "MouthOpenY", 0.6);
        if let Some(head) = state
            .doc
            .rig
            .deformers
            .iter()
            .find(|d| d.name == "Head")
            .map(|d| d.id)
        {
            state.rig.tool.selection = Some(RigNode::Deformer(head));
        }
        if workspace == Workspace::Animation && !state.doc.rig.motions.is_empty() {
            state.rig.motion = Some(0);
            state.rig.animate = true;
            state.rig.playhead = 1.2;
            let first = state.doc.rig.motions[0].tracks.first().map(|t| (t.param, 1usize));
            state.rig.selected_key = first;
        }
        state.select_tool(aether_ui::tools::ToolId::Deform);
    }

    let ctx = egui::Context::default();
    aether_ui::fonts::install_cjk_fallback(&ctx);
    let mut textures: HashMap<TextureId, Texture> = HashMap::new();
    let mut last = None;
    for frame in 0..6 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WIDTH, HEIGHT),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| app.draw(ui));
        for (id, delta) in &output.textures_delta.set {
            apply_delta(&mut textures, *id, delta);
        }
        for id in &output.textures_delta.free {
            textures.remove(id);
        }
        if frame == 1 {
            // The canvas knows its size after the first frame.
            app.state_mut().zoom_fit();
        }
        last = Some(output);
    }
    let Some(output) = last else {
        return Ok(());
    };
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut frame = vec![[0.0f32, 0.0, 0.0, 1.0]; w * h];
    rasterize(&mut frame, w, h, &primitives, &textures);

    let mut image = Pixmap::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let p = frame[y * w + x];
            let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            image.set(
                x as i32,
                y as i32,
                Rgba8::new(to8(p[0]), to8(p[1]), to8(p[2]), 255),
            );
        }
    }
    aether_io::save_png(&image, &out)?;
    println!("wrote {out}");
    Ok(())
}
