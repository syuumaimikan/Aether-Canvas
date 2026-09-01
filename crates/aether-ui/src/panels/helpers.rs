//! Small widgets shared by several panels.

use crate::state::EditorState;
use aether_core::blend::BlendMode;
use aether_core::color::{Rgba, Rgba8};
use egui::{Color32, Ui};

pub fn blend_mode_combo(ui: &mut Ui, id: &str, label: &str, value: &mut BlendMode) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(value.name())
        .show_ui(ui, |ui| {
            for mode in BlendMode::ALL {
                if ui.selectable_label(*value == mode, mode.name()).clicked() {
                    *value = mode;
                    changed = true;
                }
            }
        });
    ui.label(label);
    changed
}

pub fn to_color32(color: Rgba) -> Color32 {
    let c = color.to_rgba8();
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

pub fn from_color32(color: Color32) -> Rgba {
    Rgba8::new(color.r(), color.g(), color.b(), color.a()).to_rgba()
}

/// Apply / cancel buttons for the modal tools.
pub fn commit_controls(ui: &mut Ui, state: &mut EditorState) {
    let pending = state.has_pending_tool_edit();
    ui.separator();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(pending, egui::Button::new("Apply (Enter)"))
            .clicked()
        {
            state.commit_tool();
        }
        if ui
            .add_enabled(pending, egui::Button::new("Cancel (Esc)"))
            .clicked()
        {
            state.cancel_tool();
        }
    });
}
