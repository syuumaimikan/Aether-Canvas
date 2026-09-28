//! Rendering and exporting animation.
//!
//! Frames are produced by running the same [`RigRuntime`] and compositor the
//! canvas uses, at a fixed frame step, on a private copy of the document — so
//! an export matches playback exactly, including physics and behaviours, and
//! never disturbs the open document.
//!
//! Three outputs are supported:
//!
//! * a **PNG sequence**, lossless with full alpha, for video tools;
//! * an **animated GIF**, for sharing anywhere (flattened over a background
//!   colour, since GIF has only on/off transparency);
//! * a **sprite sheet** PNG plus a JSON atlas describing each frame's
//!   rectangle, for game engines.

use crate::image_io::{encode_pixmap_png, save_png};
use aether_core::color::Rgba8;
use aether_core::{AetherError, Result};
use aether_document::rig::RigRuntime;
use aether_document::Document;
use aether_raster::composite::{composite_pixmap, CompositeOptions};
use aether_raster::Pixmap;
use aether_render::Compositor;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// What to render.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationSettings {
    /// Motion to play (by index), or `None` to record the rig idling with
    /// its behaviours and physics only.
    pub motion: Option<usize>,
    /// Frames per second.
    pub fps: f32,
    /// Length in seconds; `None` uses the motion's duration (or 3 s).
    pub duration: Option<f32>,
    /// Run physics, jiggle, drivers and behaviours.
    pub simulate: bool,
    /// Seconds simulated before the first frame so chains start settled.
    pub warmup: f32,
    /// Output scale (1 = document size).
    pub scale: f32,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            motion: None,
            fps: 30.0,
            duration: None,
            simulate: true,
            warmup: 1.0,
            scale: 1.0,
        }
    }
}

impl AnimationSettings {
    /// The effective length in seconds for `doc`.
    pub fn length(&self, doc: &Document) -> f32 {
        self.duration
            .or_else(|| {
                self.motion
                    .and_then(|m| doc.rig.motions.get(m))
                    .map(|m| m.duration)
            })
            .unwrap_or(3.0)
            .clamp(1.0 / 240.0, 600.0)
    }

    /// Number of frames that will be rendered.
    pub fn frame_count(&self, doc: &Document) -> usize {
        ((self.length(doc) * self.fps.clamp(1.0, 240.0)).round() as usize).max(1)
    }
}

/// Render every frame of an animation.
pub fn render_frames(
    doc: &Document,
    compositor: &Compositor,
    settings: &AnimationSettings,
) -> Result<Vec<Pixmap>> {
    let fps = settings.fps.clamp(1.0, 240.0);
    if let Some(m) = settings.motion {
        if m >= doc.rig.motions.len() {
            return Err(AetherError::rig("the chosen motion does not exist"));
        }
    }
    let frames = settings.frame_count(doc);
    if frames > 20_000 {
        return Err(AetherError::invalid("that is too many frames to render at once"));
    }
    let dt = 1.0 / fps;
    let mut work = doc.clone();
    let mut runtime = RigRuntime::new();
    runtime.settings.physics = settings.simulate;
    runtime.settings.jiggle = settings.simulate;
    runtime.settings.behaviours = settings.simulate;
    runtime.settings.drivers = true;

    // Let physics settle into the starting pose.
    let warmup_steps = (settings.warmup.max(0.0) * fps).round() as usize;
    for _ in 0..warmup_steps {
        runtime.scrub = settings.motion.map(|m| (m, 0.0));
        runtime.tick(&mut work.rig, dt);
    }

    let mut out = Vec::with_capacity(frames);
    for frame in 0..frames {
        let time = frame as f32 * dt;
        runtime.scrub = settings.motion.map(|m| (m, time));
        runtime.tick(&mut work.rig, if frame == 0 { 0.0 } else { dt });
        let image = compositor.render(&work);
        out.push(scale_frame(image, settings.scale));
    }
    Ok(out)
}

fn scale_frame(image: Pixmap, scale: f32) -> Pixmap {
    let scale = if scale.is_finite() {
        scale.clamp(0.05, 4.0)
    } else {
        1.0
    };
    if (scale - 1.0).abs() < 1e-3 {
        return image;
    }
    let w = ((image.width() as f32 * scale).round() as u32).max(1);
    let h = ((image.height() as f32 * scale).round() as u32).max(1);
    image.scaled(w, h)
}

/// Write frames as `<stem>_0000.png`, `<stem>_0001.png`, ... into `dir`.
pub fn export_png_sequence(frames: &[Pixmap], dir: impl AsRef<Path>, stem: &str) -> Result<Vec<PathBuf>> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir)?;
    let digits = frames.len().max(1).to_string().len().max(4);
    let mut paths = Vec::with_capacity(frames.len());
    for (i, frame) in frames.iter().enumerate() {
        let path = dir.join(format!("{stem}_{i:0digits$}.png"));
        save_png(frame, &path)?;
        paths.push(path);
    }
    Ok(paths)
}

/// Flatten a frame over a solid colour.
pub fn flatten(frame: &Pixmap, background: Rgba8) -> Pixmap {
    let mut out = Pixmap::filled(frame.width(), frame.height(), background);
    composite_pixmap(&mut out, frame, &CompositeOptions::normal(), None);
    out
}

/// Encode frames as a looping animated GIF, flattened over `background`.
pub fn encode_gif(frames: &[Pixmap], fps: f32, background: Rgba8) -> Result<Vec<u8>> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, RgbaImage};
    if frames.is_empty() {
        return Err(AetherError::invalid("there are no frames to export"));
    }
    let delay_ms = (1000.0 / fps.clamp(1.0, 100.0)).round() as u32;
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new_with_speed(&mut bytes, 10);
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| AetherError::UnsupportedFormat(format!("could not encode GIF: {e}")))?;
        for frame in frames {
            let flat = flatten(frame, background);
            let image = RgbaImage::from_raw(flat.width(), flat.height(), flat.into_data())
                .ok_or_else(|| AetherError::raster("frame buffer has the wrong size"))?;
            encoder
                .encode_frame(Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay_ms, 1),
                ))
                .map_err(|e| AetherError::UnsupportedFormat(format!("could not encode GIF: {e}")))?;
        }
    }
    Ok(bytes)
}

/// Write an animated GIF.
pub fn export_gif(frames: &[Pixmap], path: impl AsRef<Path>, fps: f32, background: Rgba8) -> Result<()> {
    let bytes = encode_gif(frames, fps, background)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// One frame's place in a sprite sheet.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetFrame {
    /// Left edge, pixels.
    pub x: u32,
    /// Top edge, pixels.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

/// The JSON written next to a sprite sheet.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetAtlas {
    /// Image file name.
    pub image: String,
    /// Playback rate.
    pub fps: f32,
    /// Frames in order.
    pub frames: Vec<SheetFrame>,
}

/// Pack frames into a grid; returns the sheet and each frame's rectangle.
pub fn pack_sprite_sheet(frames: &[Pixmap], columns: Option<u32>) -> Result<(Pixmap, Vec<SheetFrame>)> {
    let first = frames
        .first()
        .ok_or_else(|| AetherError::invalid("there are no frames to export"))?;
    let (fw, fh) = (first.width(), first.height());
    let n = frames.len() as u32;
    let columns = columns
        .unwrap_or_else(|| (n as f32).sqrt().ceil() as u32)
        .clamp(1, n);
    let rows = n.div_ceil(columns);
    if fw as u64 * columns as u64 > 16384 || fh as u64 * rows as u64 > 16384 {
        return Err(AetherError::invalid(
            "the sprite sheet would exceed 16384 pixels; lower the scale or the frame count",
        ));
    }
    let mut sheet = Pixmap::new(fw * columns, fh * rows);
    let mut rects = Vec::with_capacity(frames.len());
    for (i, frame) in frames.iter().enumerate() {
        let (c, r) = (i as u32 % columns, i as u32 / columns);
        sheet.paste_rect(frame, (c * fw) as i32, (r * fh) as i32);
        rects.push(SheetFrame {
            x: c * fw,
            y: r * fh,
            w: fw,
            h: fh,
        });
    }
    Ok((sheet, rects))
}

/// Write a sprite sheet PNG and its JSON atlas (same name, `.json`).
pub fn export_sprite_sheet(frames: &[Pixmap], path: impl AsRef<Path>, fps: f32) -> Result<PathBuf> {
    let path = path.as_ref();
    let (sheet, rects) = pack_sprite_sheet(frames, None)?;
    std::fs::write(path, encode_pixmap_png(&sheet)?)?;
    let atlas = SheetAtlas {
        image: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        fps,
        frames: rects,
    };
    let json_path = path.with_extension("json");
    let json = serde_json::to_vec_pretty(&atlas)
        .map_err(|e| AetherError::serialization(format!("could not write the atlas: {e}")))?;
    std::fs::write(&json_path, json)?;
    Ok(json_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::math::{vec2, Rect};
    use aether_document::rig::{ArtMesh, KeyAxis, Motion, Parameter};

    /// A red square that slides right over one second.
    fn animated_doc() -> Document {
        let mut doc = Document::new(48, 16, "anim");
        let layer = doc.active_layer;
        if let Some(pm) = doc.layers.get_mut(layer).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(aether_core::math::IRect::new(0, 4, 8, 8), Rgba8::rgb(255, 0, 0));
        }
        let param = doc.ids.parameter();
        doc.rig
            .add_parameter(Parameter::new(param, "Slide", 0.0, 1.0, 0.0))
            .expect("param");
        let mut mesh = ArtMesh::quad(layer, Rect::from_corners(vec2(0.0, 0.0), vec2(12.0, 16.0)));
        mesh.keyforms
            .add_axis(KeyAxis::new(param, [0.0, 1.0]).expect("axis"))
            .expect("axis");
        for o in &mut mesh.keyforms.forms[1].offsets {
            *o = vec2(32.0, 0.0);
        }
        doc.rig.set_mesh(mesh);
        let mut motion = Motion::new("slide", 1.0, 10.0);
        motion.track_mut(param).set_key(0.0, 0.0);
        motion.track_mut(param).set_key(1.0, 1.0);
        doc.rig.motions.push(motion);
        doc
    }

    #[test]
    fn frames_follow_the_motion() {
        let doc = animated_doc();
        let settings = AnimationSettings {
            motion: Some(0),
            fps: 10.0,
            ..Default::default()
        };
        let frames = render_frames(&doc, &Compositor::new(), &settings).expect("render");
        assert_eq!(frames.len(), 10);
        assert_eq!(frames[0].get(4, 8), Rgba8::rgb(255, 0, 0));
        // Frame 5 is t = 0.5: halfway, 16 px right.
        assert_eq!(frames[5].get(20, 8), Rgba8::rgb(255, 0, 0));
        assert_eq!(frames[5].get(2, 8).a, 0);
        assert_eq!(doc.rig.dynamics.values, None, "the open document is untouched");
    }

    #[test]
    fn a_missing_motion_is_an_error() {
        let doc = animated_doc();
        let settings = AnimationSettings {
            motion: Some(7),
            ..Default::default()
        };
        assert!(render_frames(&doc, &Compositor::new(), &settings).is_err());
    }

    #[test]
    fn gifs_encode_and_decode_with_every_frame() {
        let doc = animated_doc();
        let settings = AnimationSettings {
            motion: Some(0),
            fps: 10.0,
            scale: 0.5,
            ..Default::default()
        };
        let frames = render_frames(&doc, &Compositor::new(), &settings).expect("render");
        assert_eq!((frames[0].width(), frames[0].height()), (24, 8));
        let bytes = encode_gif(&frames, 10.0, Rgba8::WHITE).expect("gif");
        use image::AnimationDecoder;
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).expect("decoder");
        let decoded = decoder.into_frames().collect_frames().expect("frames");
        assert_eq!(decoded.len(), 10);
    }

    #[test]
    fn sprite_sheets_lay_frames_out_in_a_grid() {
        let frames: Vec<Pixmap> = (0..5)
            .map(|i| Pixmap::filled(4, 3, Rgba8::rgb(i * 40, 0, 0)))
            .collect();
        let (sheet, rects) = pack_sprite_sheet(&frames, None).expect("pack");
        assert_eq!((sheet.width(), sheet.height()), (12, 6));
        assert_eq!(
            rects[4],
            SheetFrame {
                x: 4,
                y: 3,
                w: 4,
                h: 3
            }
        );
        assert_eq!(sheet.get(5, 4), Rgba8::rgb(160, 0, 0));
    }

    #[test]
    fn sequences_and_sheets_are_written_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let frames: Vec<Pixmap> = (0..3).map(|_| Pixmap::filled(4, 4, Rgba8::WHITE)).collect();
        let paths = export_png_sequence(&frames, dir.path(), "walk").expect("sequence");
        assert_eq!(paths.len(), 3);
        assert!(paths[2].ends_with("walk_0002.png"));
        let json = export_sprite_sheet(&frames, dir.path().join("walk.png"), 12.0).expect("sheet");
        let text = std::fs::read_to_string(json).expect("json");
        assert!(text.contains("\"fps\": 12.0"));
    }
}
