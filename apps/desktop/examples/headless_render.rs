//! Build a small document in code, composite it and write a PNG — no window.
//!
//! Two things this demonstrates:
//!
//! * the whole pipeline (document → brush → effects → compositor → encoder) works
//!   without a GPU or a display, which is what makes it testable in CI;
//! * the public API is usable as a library, so batch jobs and, later, plugins
//!   and scripts can drive the same code the UI does.
//!
//! Run with:
//!
//! ```text
//! cargo run --example headless_render -- out.png
//! ```

use aether_core::blend::BlendMode;
use aether_core::color::{Rgba, Rgba8};
use aether_core::input::InputSample;
use aether_core::math::{IRect, Vec2};
use aether_document::layer::Layer;
use aether_document::{Background, Document};
use aether_raster::adjust::Adjustment;
use aether_raster::composite::{fill_masked, CompositeOptions};
use aether_raster::effect::{EffectKind, LayerEffect};
use aether_raster::{BrushPreset, StrokeState};
use aether_render::Compositor;

fn main() -> aether_core::Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "aether-headless.png".to_string());

    let mut doc = Document::new(512, 512, "Headless Demo");
    doc.background = Background::Solid(Rgba8::rgb(250, 248, 244));

    // A painted stroke on the base layer.
    let base = doc.active_layer;
    paint_arc(&mut doc, base, Rgba::rgb(0.14, 0.33, 0.62), 40.0, 0.0)?;

    // A second layer, blended.
    let accent = doc.next_layer_id();
    doc.layers.push_top(Layer::raster(accent, "Accent", 512, 512))?;
    doc.active_layer = accent;
    paint_arc(&mut doc, accent, Rgba::rgb(0.92, 0.45, 0.18), 18.0, 0.35)?;
    if let Some(layer) = doc.layers.get_mut(accent) {
        layer.blend_mode = BlendMode::Multiply;
        layer.opacity = 0.85;
    }

    // Non-destructive layer effects: the stroke's own pixels are untouched.
    if let Some(layer) = doc.layers.get_mut(base) {
        layer.effects = vec![
            LayerEffect::new(EffectKind::DropShadow {
                dx: 10.0,
                dy: 12.0,
                radius: 8.0,
                color: Rgba8::rgb(20, 30, 60),
                opacity: 0.45,
            }),
            LayerEffect::new(EffectKind::Glow {
                radius: 16.0,
                intensity: 0.8,
                color: Rgba8::rgb(120, 190, 255),
            }),
        ];
    }

    // A non-destructive adjustment on top of everything.
    let adjust = doc.next_layer_id();
    doc.layers.push_top(Layer::adjustment(
        adjust,
        "Warmth",
        Adjustment::HueSaturation {
            hue: -6.0,
            saturation: 0.15,
            lightness: 0.02,
        },
    ))?;

    let composite = Compositor::new().render(&doc);
    aether_io::save_png(&composite, &out)?;
    println!("wrote {out} ({}x{})", composite.width(), composite.height());

    // And the project itself, so it can be opened in the editor.
    let project_path = std::path::Path::new(&out).with_extension("aether");
    aether_io::save_project(&doc, &project_path)?;
    println!("wrote {}", project_path.display());
    Ok(())
}

/// Paint a tapering arc with the brush engine.
///
/// `phase` rotates the arc so two calls produce overlapping strokes, which is
/// what makes the multiply blend on the accent layer visible.
fn paint_arc(
    doc: &mut Document,
    layer: aether_core::LayerId,
    color: Rgba,
    size: f32,
    phase: f32,
) -> aether_core::Result<()> {
    let preset = BrushPreset {
        size,
        hardness: 0.8,
        spacing: 0.05,
        smoothing: 0.0,
        ..Default::default()
    };
    let mut stroke = StrokeState::begin(preset, doc.width, doc.height, 7);
    let center = Vec2::new(doc.width as f32 * 0.5, doc.height as f32 * 0.5);
    let radius = doc.width as f32 * 0.32;
    for step in 0..=160 {
        let t = step as f32 / 160.0;
        let angle = phase + std::f32::consts::PI * (0.75 + t * 1.3);
        let p = center + Vec2::new(radius * angle.cos(), radius * angle.sin());
        stroke.push(
            InputSample::at(p)
                .with_pressure(0.25 + 0.75 * t)
                .with_time(t as f64),
        );
    }
    let opacity = stroke.opacity_for_last_sample();
    let (mask, dirty) = stroke.finish();
    let bounds = IRect::from_size(doc.width, doc.height);
    let target = doc
        .layers
        .get_mut(layer)
        .and_then(|l| l.pixmap_mut())
        .ok_or_else(|| aether_core::AetherError::document("layer holds no pixels"))?;
    let opts = CompositeOptions {
        blend: BlendMode::Normal,
        opacity,
        offset: (0, 0),
        region: Some(dirty.intersect(&bounds)),
        alpha_lock: false,
    };
    fill_masked(target, color, &mask, &opts);
    doc.mark_dirty(dirty);
    Ok(())
}
