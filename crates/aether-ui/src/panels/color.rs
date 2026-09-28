//! Colour picker, palette and recent colours.

use crate::panels::{from_color32, to_color32};
use crate::state::EditorState;
use aether_core::color::Rgba8;
use egui::{Color32, RichText, Ui};

/// Colour picker, palette and recent colours.
pub fn color_panel(ui: &mut Ui, state: &mut EditorState) {
    let mut primary = to_color32(state.primary);
    ui.horizontal(|ui| {
        ui.label(state.tr("color.primary"));
        if egui::color_picker::color_edit_button_srgba(ui, &mut primary, egui::color_picker::Alpha::Opaque)
            .changed()
        {
            state.primary = from_color32(primary);
        }
        let mut secondary = to_color32(state.secondary);
        ui.label(state.tr("color.secondary"));
        if egui::color_picker::color_edit_button_srgba(ui, &mut secondary, egui::color_picker::Alpha::Opaque)
            .changed()
        {
            state.secondary = from_color32(secondary);
        }
        if ui
            .button(crate::icons::SWAP)
            .on_hover_text(state.tr("color.swap"))
            .clicked()
        {
            std::mem::swap(&mut state.primary, &mut state.secondary);
        }
    });

    let mut hex = state.primary.to_rgba8().to_hex();
    ui.horizontal(|ui| {
        ui.label(state.tr("color.hex"));
        if ui.text_edit_singleline(&mut hex).changed() {
            if let Some(color) = Rgba8::from_hex(&hex) {
                state.primary = color.to_rgba().with_alpha(1.0);
            }
        }
    });

    egui::color_picker::color_picker_color32(ui, &mut primary, egui::color_picker::Alpha::Opaque);
    let picked = from_color32(primary);
    if picked != state.primary {
        state.primary = picked;
    }

    ui.separator();
    ui.label(RichText::new(state.tr("color.palette")).strong());
    let palette = state.palette.clone();
    swatch_grid(ui, &palette, |color| {
        state.primary = color.to_rgba().with_alpha(1.0)
    });

    if !state.recent_colors.is_empty() {
        ui.separator();
        ui.label("Recent");
        let recent = state.recent_colors.clone();
        swatch_grid(ui, &recent, |color| {
            state.primary = color.to_rgba().with_alpha(1.0)
        });
    }
}

pub fn swatch_grid(ui: &mut Ui, colors: &[Rgba8], mut on_pick: impl FnMut(Rgba8)) {
    ui.horizontal_wrapped(|ui| {
        for color in colors {
            let c = Color32::from_rgb(color.r, color.g, color.b);
            let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 2.0, c);
            ui.painter().rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(1.0, Color32::from_black_alpha(80)),
                egui::StrokeKind::Inside,
            );
            if response.clicked() {
                on_pick(*color);
            }
            response.on_hover_text(color.to_hex());
        }
    });
}
