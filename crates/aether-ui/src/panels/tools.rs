//! The tool palette and the options for the active tool.

use crate::panels::brush::brush_controls;
use crate::panels::commit_controls;
use crate::state::EditorState;
use crate::tools::{LiquifyMode, ToolId, TransformMode};
use aether_document::selection::SelectionMode;
use egui::{RichText, Ui};

/// Toolbar: one button per tool, with its shortcut in the tooltip.
pub fn tools_panel(ui: &mut Ui, state: &mut EditorState) {
    let active = state.tools.active_id();
    let ids = state.tools.ids();
    ui.horizontal_wrapped(|ui| {
        for id in ids {
            let label = state.tr(id.label_key());
            let selected = id == active;
            let button = egui::Button::new(RichText::new(id.glyph()).size(18.0)).selected(selected);
            let response = ui.add_sized([34.0, 34.0], button);
            if response.clicked() {
                state.select_tool(id);
            }
            response.on_hover_text(label);
        }
    });
    ui.separator();
    tool_options(ui, state);
}

/// Options for the active tool.
pub fn tool_options(ui: &mut Ui, state: &mut EditorState) {
    ui.label(RichText::new(state.tr("tool.options")).strong());
    match state.tools.active_id() {
        ToolId::Brush | ToolId::Eraser => {
            brush_controls(ui, state);
        }
        ToolId::Bucket | ToolId::MagicWand => {
            let label = state.language.tr("tool.tolerance");
            ui.add(egui::Slider::new(&mut state.tolerance, 0.0..=1.0).text(label));
            ui.checkbox(&mut state.sample_all_layers, "Sample all layers");
        }
        ToolId::Eyedropper => {
            ui.checkbox(&mut state.sample_all_layers, "Sample all layers");
        }
        ToolId::RectSelect | ToolId::EllipseSelect | ToolId::Lasso => {
            selection_mode_controls(ui, state);
        }
        ToolId::Transform => transform_options(ui, state),
        ToolId::Liquify => liquify_options(ui, state),
        ToolId::Mesh => mesh_options(ui, state),
        ToolId::Deform => deform_options(ui, state),
        ToolId::Bone => {
            ui.label(state.tr("tool.bone_hint"));
        }
        ToolId::Move | ToolId::Pan | ToolId::Custom(_) => {
            ui.label("—");
        }
    }
}

fn mesh_options(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.label(lang.tr("tool.mesh_hint"));
    ui.add(
        egui::Slider::new(&mut state.rig.tool.mesh_density, 0.3..=4.0)
            .text(lang.tr("rig.density"))
            .max_decimals(1),
    );
    ui.horizontal_wrapped(|ui| {
        if ui.button(lang.tr("rig.mesh_layer")).clicked() {
            let layer = state.doc.active_layer;
            if let Err(e) = state.mesh_layer(layer) {
                state.report_error("Mesh", &e);
            }
        }
        let layer = state.doc.active_layer;
        if state.doc.rig.mesh(layer).is_some() && ui.button(lang.tr("tool.mesh_remove")).clicked() {
            if let Err(e) = state.remove_mesh(layer) {
                state.report_error("Mesh", &e);
            }
        }
    });
}

fn deform_options(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.label(lang.tr("tool.deform_hint"));
    ui.add(
        egui::Slider::new(&mut state.rig.tool.radius, 1.0..=400.0)
            .text(lang.tr("tool.deform_radius"))
            .logarithmic(true),
    );
    let target = state
        .rig
        .tool
        .selection
        .map(|n| state.doc.rig.node_name(n))
        .unwrap_or_else(|| "—".into());
    ui.label(format!("{} {target}", lang.tr("param.keys_for")));
    if state.rig.tool.blend_shape.is_some() {
        ui.colored_label(
            egui::Color32::from_rgb(230, 170, 90),
            lang.tr("param.editing_shape"),
        );
    }
}

fn transform_options(ui: &mut Ui, state: &mut EditorState) {
    ui.horizontal_wrapped(|ui| {
        for mode in TransformMode::ALL {
            if ui
                .selectable_label(state.tool_settings.transform_mode == mode, mode.label())
                .clicked()
            {
                state.tool_settings.transform_mode = mode;
            }
        }
    });
    ui.label("Drag a corner to scale, the ring outside to rotate, inside to move.");
    ui.label("Shift keeps the aspect ratio and snaps rotation.");
    commit_controls(ui, state);
}

fn liquify_options(ui: &mut Ui, state: &mut EditorState) {
    ui.horizontal_wrapped(|ui| {
        for mode in LiquifyMode::ALL {
            if ui
                .selectable_label(state.tool_settings.liquify_mode == mode, mode.label())
                .clicked()
            {
                state.tool_settings.liquify_mode = mode;
            }
        }
    });
    ui.add(egui::Slider::new(&mut state.tool_settings.liquify_radius, 4.0..=400.0).text("Radius"));
    ui.add(egui::Slider::new(&mut state.tool_settings.liquify_strength, 0.05..=1.0).text("Strength"));
    commit_controls(ui, state);
}

fn selection_mode_controls(ui: &mut Ui, state: &mut EditorState) {
    ui.horizontal(|ui| {
        for (mode, label) in [
            (SelectionMode::Replace, "New"),
            (SelectionMode::Add, "Add"),
            (SelectionMode::Subtract, "Subtract"),
            (SelectionMode::Intersect, "Intersect"),
        ] {
            if ui.selectable_label(state.selection_mode == mode, label).clicked() {
                state.selection_mode = mode;
            }
        }
    });
}
