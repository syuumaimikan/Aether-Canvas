//! Parameter editors for adjustments and effects.
//!
//! One editor serves three places — adjustment layers, layer effects and the
//! destructive filter dialog — because all three edit the same value types.
//! Each function returns `true` when the user changed something, which is what
//! the caller uses to decide whether to push a command.

use aether_core::color::Rgba8;
use aether_raster::adjust::{Adjustment, Curve};
use aether_raster::effect::EffectKind;
use egui::{Color32, Pos2, Sense, Stroke, Ui, Vec2};

/// Side length of the curve widget, in points.
const CURVE_SIZE: f32 = 180.0;
/// How close, in points, the pointer must be to grab a control point.
const POINT_GRAB: f32 = 10.0;

/// An interactive tone-curve editor.
///
/// Click to add a point, drag to move one, right-click to remove one. The
/// curve is drawn by evaluating the same spline the pixel pipeline uses, so
/// what is on screen is exactly what will be applied.
pub fn curve_editor(ui: &mut Ui, curve: &mut Curve, id: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(CURVE_SIZE), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let to_screen = |x: f32, y: f32| Pos2::new(rect.min.x + x * rect.width(), rect.max.y - y * rect.height());
    let to_curve = |p: Pos2| {
        (
            ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0),
            ((rect.max.y - p.y) / rect.height()).clamp(0.0, 1.0),
        )
    };

    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let grid = Stroke::new(1.0, Color32::from_white_alpha(20));
    for i in 1..4 {
        let t = i as f32 / 4.0;
        painter.line_segment([to_screen(t, 0.0), to_screen(t, 1.0)], grid);
        painter.line_segment([to_screen(0.0, t), to_screen(1.0, t)], grid);
    }
    // The identity diagonal, for reference.
    painter.line_segment(
        [to_screen(0.0, 0.0), to_screen(1.0, 1.0)],
        Stroke::new(1.0, Color32::from_white_alpha(30)),
    );

    let samples: Vec<Pos2> = (0..=64)
        .map(|i| {
            let x = i as f32 / 64.0;
            to_screen(x, curve.eval(x))
        })
        .collect();
    painter.add(egui::Shape::line(
        samples,
        Stroke::new(1.5, Color32::from_rgb(160, 200, 255)),
    ));

    let mut changed = false;
    let grab_id = egui::Id::new(id);
    let mut dragged: Option<usize> = ui.data(|d| d.get_temp(grab_id)).flatten();

    if response.drag_started() || response.clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let hit = curve
                .points()
                .iter()
                .enumerate()
                .find(|(_, (x, y))| to_screen(*x, *y).distance(pointer) <= POINT_GRAB)
                .map(|(i, _)| i);
            match hit {
                Some(index) if response.secondary_clicked() => {
                    curve.remove_point(index);
                    changed = true;
                }
                Some(index) => dragged = Some(index),
                None => {
                    let (x, y) = to_curve(pointer);
                    curve.add_point(x, y);
                    changed = true;
                    dragged = curve.points().iter().position(|p| (p.0 - x).abs() < 1e-4);
                }
            }
        }
    }

    if response.dragged() {
        if let (Some(index), Some(pointer)) = (dragged, response.interact_pointer_pos()) {
            let (x, y) = to_curve(pointer);
            curve.move_point(index, x, y);
            changed = true;
        }
    }
    if response.drag_stopped() {
        dragged = None;
    }
    ui.data_mut(|d| d.insert_temp(grab_id, dragged));

    for (index, (x, y)) in curve.points().iter().enumerate() {
        let selected = dragged == Some(index);
        painter.circle_filled(
            to_screen(*x, *y),
            if selected { 5.0 } else { 3.5 },
            Color32::from_rgb(230, 240, 255),
        );
    }
    ui.small("Click to add a point, drag to shape, right-click to remove.");
    changed
}

/// Parameter widgets for one adjustment. Returns true when anything changed.
pub fn adjustment_editor(ui: &mut Ui, adjustment: &mut Adjustment, id_prefix: &str) -> bool {
    let mut changed = false;
    match adjustment {
        Adjustment::BrightnessContrast { brightness, contrast } => {
            changed |= ui
                .add(egui::Slider::new(brightness, -1.0..=1.0).text("Brightness"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(contrast, -1.0..=1.0).text("Contrast"))
                .changed();
        }
        Adjustment::Exposure { stops } => {
            changed |= ui
                .add(egui::Slider::new(stops, -5.0..=5.0).text("Stops"))
                .changed();
        }
        Adjustment::Gamma { gamma } => {
            changed |= ui
                .add(egui::Slider::new(gamma, 0.1..=5.0).text("Gamma"))
                .changed();
        }
        Adjustment::Levels {
            in_black,
            in_white,
            gamma,
            out_black,
            out_white,
        } => {
            changed |= ui
                .add(egui::Slider::new(in_black, 0.0..=1.0).text("Input black"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(in_white, 0.0..=1.0).text("Input white"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(gamma, 0.1..=5.0).text("Midtones"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(out_black, 0.0..=1.0).text("Output black"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(out_white, 0.0..=1.0).text("Output white"))
                .changed();
        }
        Adjustment::Curves {
            master,
            red,
            green,
            blue,
        } => {
            // One channel at a time keeps the panel narrow enough to dock.
            let key = egui::Id::new(format!("{id_prefix}-curve-channel"));
            let mut channel: usize = ui.data(|d| d.get_temp(key)).flatten().unwrap_or(0);
            ui.horizontal(|ui| {
                for (index, name) in ["RGB", "R", "G", "B"].iter().enumerate() {
                    if ui.selectable_label(channel == index, *name).clicked() {
                        channel = index;
                    }
                }
            });
            ui.data_mut(|d| d.insert_temp(key, Some(channel)));
            let curve = match channel {
                1 => red,
                2 => green,
                3 => blue,
                _ => master,
            };
            changed |= curve_editor(ui, curve, &format!("{id_prefix}-curve-{channel}"));
            if ui.button("Reset channel").clicked() {
                *curve = Curve::identity();
                changed = true;
            }
        }
        Adjustment::HueSaturation {
            hue,
            saturation,
            lightness,
        } => {
            changed |= ui
                .add(egui::Slider::new(hue, -180.0..=180.0).text("Hue"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(saturation, -1.0..=1.0).text("Saturation"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(lightness, -1.0..=1.0).text("Lightness"))
                .changed();
        }
        Adjustment::ColorBalance { red, green, blue } => {
            changed |= ui.add(egui::Slider::new(red, -1.0..=1.0).text("Red")).changed();
            changed |= ui
                .add(egui::Slider::new(green, -1.0..=1.0).text("Green"))
                .changed();
            changed |= ui.add(egui::Slider::new(blue, -1.0..=1.0).text("Blue")).changed();
        }
        Adjustment::Threshold { level } => {
            changed |= ui
                .add(egui::Slider::new(level, 0.0..=1.0).text("Level"))
                .changed();
        }
        Adjustment::Posterize { levels } => {
            changed |= ui.add(egui::Slider::new(levels, 2..=64).text("Levels")).changed();
        }
        Adjustment::Invert | Adjustment::Grayscale => {
            ui.label("No settings.");
        }
    }
    changed
}

/// Parameter widgets for one effect. Returns true when anything changed.
pub fn effect_editor(ui: &mut Ui, effect: &mut EffectKind, id_prefix: &str) -> bool {
    let mut changed = false;
    match effect {
        EffectKind::Blur { sigma } => {
            changed |= ui
                .add(egui::Slider::new(sigma, 0.0..=64.0).text("Radius"))
                .changed();
        }
        EffectKind::Sharpen { amount, radius } => {
            changed |= ui
                .add(egui::Slider::new(amount, 0.0..=3.0).text("Amount"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(radius, 0.5..=16.0).text("Radius"))
                .changed();
        }
        EffectKind::MotionBlur { angle, distance } => {
            let mut degrees = angle.to_degrees();
            if ui
                .add(egui::Slider::new(&mut degrees, -180.0..=180.0).text("Angle"))
                .changed()
            {
                *angle = degrees.to_radians();
                changed = true;
            }
            changed |= ui
                .add(egui::Slider::new(distance, 0.0..=200.0).text("Distance"))
                .changed();
        }
        EffectKind::Glow {
            radius,
            intensity,
            color,
        } => {
            changed |= ui
                .add(egui::Slider::new(radius, 0.0..=64.0).text("Radius"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(intensity, 0.0..=4.0).text("Intensity"))
                .changed();
            changed |= color_row(ui, "Color", color);
        }
        EffectKind::DropShadow {
            dx,
            dy,
            radius,
            color,
            opacity,
        } => {
            changed |= ui
                .add(egui::Slider::new(dx, -128.0..=128.0).text("Offset X"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(dy, -128.0..=128.0).text("Offset Y"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(radius, 0.0..=64.0).text("Softness"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(opacity, 0.0..=1.0).text("Opacity"))
                .changed();
            changed |= color_row(ui, "Color", color);
        }
        EffectKind::Outline { width, color } => {
            changed |= ui.add(egui::Slider::new(width, 1..=32).text("Width")).changed();
            changed |= color_row(ui, "Color", color);
        }
        EffectKind::ColorOverlay {
            color,
            opacity,
            blend,
        } => {
            changed |= color_row(ui, "Color", color);
            changed |= ui
                .add(egui::Slider::new(opacity, 0.0..=1.0).text("Opacity"))
                .changed();
            changed |= crate::panels::blend_mode_combo(ui, &format!("{id_prefix}-blend"), "Blend", blend);
        }
        EffectKind::Grain {
            amount,
            seed,
            monochrome,
        } => {
            changed |= ui
                .add(egui::Slider::new(amount, 0.0..=1.0).text("Amount"))
                .changed();
            changed |= ui.checkbox(monochrome, "Monochrome").changed();
            if ui.button("New seed").clicked() {
                *seed = seed.wrapping_add(0x9E37_79B9);
                changed = true;
            }
        }
        EffectKind::Adjust { adjustment } => {
            changed |= adjustment_editor(ui, adjustment, id_prefix);
        }
    }
    changed
}

/// A labelled colour swatch button.
fn color_row(ui: &mut Ui, label: &str, color: &mut Rgba8) -> bool {
    let mut value = Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if egui::color_picker::color_edit_button_srgba(ui, &mut value, egui::color_picker::Alpha::Opaque)
            .changed()
        {
            *color = Rgba8::new(value.r(), value.g(), value.b(), 255);
            changed = true;
        }
    });
    changed
}
