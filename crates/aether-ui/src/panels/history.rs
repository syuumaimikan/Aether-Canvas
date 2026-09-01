//! History and document properties.

use crate::state::EditorState;
use egui::{RichText, Ui};

/// The undo history, with click-to-jump.
pub fn history_panel(ui: &mut Ui, state: &mut EditorState) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                state.history.can_undo(),
                egui::Button::new(state.language.tr("menu.edit.undo")),
            )
            .clicked()
        {
            let _ = state.history.undo(&mut state.doc);
        }
        if ui
            .add_enabled(
                state.history.can_redo(),
                egui::Button::new(state.language.tr("menu.edit.redo")),
            )
            .clicked()
        {
            let _ = state.history.redo(&mut state.doc);
        }
    });
    ui.separator();
    let entries = state.history.entries();
    let mut jump: Option<usize> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (index, entry) in entries.iter().enumerate() {
            if ui.selectable_label(entry.current, &entry.name).clicked() {
                jump = Some(index + 1);
            }
        }
    });
    if let Some(target) = jump {
        if let Err(error) = state.history.jump_to(&mut state.doc, target) {
            state.report_error("History", &error);
        }
    }
}

/// Document and selection information.
pub fn properties_panel(ui: &mut Ui, state: &mut EditorState) {
    egui::Grid::new("doc-properties").num_columns(2).show(ui, |ui| {
        ui.label("Document");
        ui.label(&state.doc.name);
        ui.end_row();
        ui.label("Size");
        ui.label(format!("{} × {}", state.doc.width, state.doc.height));
        ui.end_row();
        ui.label("Layers");
        ui.label(state.doc.layer_count().to_string());
        ui.end_row();
        ui.label("Zoom");
        ui.label(format!("{:.0}%", state.viewport.zoom * 100.0));
        ui.end_row();
        ui.label("Selection");
        match state.doc.selection.bounds() {
            Some(bounds) => ui.label(format!("{} × {}", bounds.width, bounds.height)),
            None => ui.label("None"),
        };
        ui.end_row();
        ui.label("File");
        ui.label(
            state
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "unsaved".into()),
        );
        ui.end_row();
    });

    ui.separator();
    ui.label(RichText::new("Canvas size").strong());
    let (mut width, mut height) = (state.doc.width, state.doc.height);
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut width).range(1..=16384));
        ui.label("×");
        ui.add(egui::DragValue::new(&mut height).range(1..=16384));
        if ui.button("Apply").clicked() && (width != state.doc.width || height != state.doc.height) {
            if let Err(error) = state.resize_canvas(width, height) {
                state.report_error("Resize", &error);
            }
        }
    });
}
