//! Brush settings and presets.

use crate::panels::blend_mode_combo;
use crate::state::{EditorState, PressureSource};
use aether_raster::brush::{BrushTexture, TexturePattern};
use egui::{RichText, Ui};

/// Brush parameters and presets.
pub fn brush_panel(ui: &mut Ui, state: &mut EditorState) {
    ui.label(RichText::new(state.tr("brush.presets")).strong());
    let presets = state.brush_presets.clone();
    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
        for preset in presets {
            let selected = state.brush.name == preset.name;
            if ui.selectable_label(selected, &preset.name).clicked() {
                state.brush = preset.clone();
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button(state.tr("brush.save_preset")).clicked() {
            state.export_brush_presets_via_dialog();
        }
        if ui.button(state.tr("brush.load_preset")).clicked() {
            state.import_brush_presets_via_dialog();
        }
        if ui
            .button("＋")
            .on_hover_text("Save the current settings as a preset")
            .clicked()
        {
            state.add_current_brush_as_preset();
        }
    });
    ui.separator();
    brush_controls(ui, state);
}

pub fn brush_controls(ui: &mut Ui, state: &mut EditorState) {
    let brush = &mut state.brush;
    ui.add(
        egui::Slider::new(&mut brush.size, 0.5..=500.0)
            .logarithmic(true)
            .text(state.language.tr("brush.size")),
    );
    ui.add(egui::Slider::new(&mut brush.opacity, 0.0..=1.0).text(state.language.tr("brush.opacity")));
    ui.add(egui::Slider::new(&mut brush.flow, 0.0..=1.0).text(state.language.tr("brush.flow")));
    ui.add(egui::Slider::new(&mut brush.hardness, 0.0..=1.0).text(state.language.tr("brush.hardness")));
    ui.add(egui::Slider::new(&mut brush.spacing, 0.01..=1.0).text(state.language.tr("brush.spacing")));
    ui.add(egui::Slider::new(&mut brush.smoothing, 0.0..=0.95).text(state.language.tr("brush.smoothing")));
    blend_mode_combo(
        ui,
        "brush-blend",
        state.language.tr("brush.blend"),
        &mut brush.blend,
    );
    ui.horizontal(|ui| {
        ui.label("Pressure → size");
        ui.add(egui::Slider::new(&mut brush.dynamics.size_pressure, 0.0..=1.0).show_value(false));
    });
    ui.horizontal(|ui| {
        ui.label("Speed → size");
        ui.add(egui::Slider::new(&mut brush.dynamics.size_velocity, 0.0..=1.0).show_value(false));
    });
    brush.sanitize();

    ui.separator();
    texture_controls(ui, state);

    ui.separator();
    ui.horizontal(|ui| {
        ui.label(state.language.tr("brush.pressure"));
        for source in PressureSource::ALL {
            let label = state.language.tr(source.key());
            if ui
                .selectable_label(state.pressure_source == source, label)
                .clicked()
            {
                state.pressure_source = source;
            }
        }
    });
}

/// Paper-grain controls for the current brush.
fn texture_controls(ui: &mut Ui, state: &mut EditorState) {
    let mut enabled = state.brush.texture.is_some();
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut enabled, state.language.tr("brush.texture"))
            .changed()
        {
            state.brush.texture = if enabled {
                Some(BrushTexture::default())
            } else {
                None
            };
        }
        if let Some(texture) = &mut state.brush.texture {
            if let TexturePattern::Noise { seed } = &mut texture.pattern {
                if ui.button("New seed").clicked() {
                    *seed = seed.wrapping_add(0x9E37_79B9);
                }
            }
        }
    });
    if let Some(texture) = &mut state.brush.texture {
        ui.add(egui::Slider::new(&mut texture.scale, 1.0..=64.0).text("Grain size"));
        ui.add(egui::Slider::new(&mut texture.strength, 0.0..=1.0).text("Grain strength"));
    }
}
