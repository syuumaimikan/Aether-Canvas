//! Dockable panels.
//!
//! Each panel is a free function taking `&mut Ui` and `&mut EditorState`. They
//! hold no state, so a panel can be shown twice, moved between docks, or left
//! out of a workspace entirely without any bookkeeping.

use crate::state::EditorState;
use crate::tools::ToolId;
use aether_core::blend::BlendMode;
use aether_core::color::{Rgba, Rgba8};
use aether_core::LayerId;
use aether_document::command::LayerProperty;
use aether_document::layer::ColorLabel;
use aether_document::selection::SelectionMode;
use egui::{Color32, RichText, Ui};

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
        ToolId::Move | ToolId::Pan | ToolId::Custom(_) => {
            ui.label("—");
        }
    }
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
    ui.separator();
    brush_controls(ui, state);
}

fn brush_controls(ui: &mut Ui, state: &mut EditorState) {
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
}

fn blend_mode_combo(ui: &mut Ui, id: &str, label: &str, value: &mut BlendMode) -> bool {
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
        if ui.button("⇄").on_hover_text(state.tr("color.swap")).clicked() {
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

fn swatch_grid(ui: &mut Ui, colors: &[Rgba8], mut on_pick: impl FnMut(Rgba8)) {
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

/// The layer stack.
pub fn layers_panel(ui: &mut Ui, state: &mut EditorState) {
    layer_toolbar(ui, state);
    ui.separator();
    active_layer_controls(ui, state);
    ui.separator();

    let rows = state.doc.layers.iter_ui_order();
    let active = state.doc.active_layer;
    let mut clicked: Option<LayerId> = None;
    let mut toggles: Vec<(LayerId, bool)> = Vec::new();

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, depth) in rows {
            let Some(layer) = state.doc.layers.get(id) else {
                continue;
            };
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 12.0);
                let mut visible = layer.visible;
                if ui
                    .checkbox(&mut visible, "")
                    .on_hover_text(state.language.tr("layer.visible"))
                    .changed()
                {
                    toggles.push((id, visible));
                }
                if let Some(swatch) = layer.color_label.swatch() {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 16.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 1.0, Color32::from_rgb(swatch.r, swatch.g, swatch.b));
                }
                let mut title = RichText::new(format!("{} · {}", layer.name, layer.kind().label()));
                if layer.clipping {
                    title = title.italics();
                }
                if layer.locked {
                    title = title.color(Color32::from_gray(140));
                }
                if ui.selectable_label(id == active, title).clicked() {
                    clicked = Some(id);
                }
                if layer.mask.is_some() {
                    ui.label(RichText::new("🎭").small()).on_hover_text("Has a mask");
                }
            });
        }
    });

    for (id, visible) in toggles {
        if let Err(error) = state.set_layer_property(id, LayerProperty::Visible(visible)) {
            state.report_error("Visibility", &error);
        }
    }
    if let Some(id) = clicked {
        state.doc.set_active_layer(id);
    }
}

fn layer_toolbar(ui: &mut Ui, state: &mut EditorState) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("＋").on_hover_text(state.tr("layer.add")).clicked() {
            if let Err(error) = state.add_layer() {
                state.report_error("Add layer", &error);
            }
        }
        if ui
            .button("🗀")
            .on_hover_text(state.tr("layer.add_group"))
            .clicked()
        {
            if let Err(error) = state.add_group() {
                state.report_error("Add group", &error);
            }
        }
        if ui
            .button("⧉")
            .on_hover_text(state.tr("layer.duplicate"))
            .clicked()
        {
            let id = state.doc.active_layer;
            if let Err(error) = state.duplicate_layer(id) {
                state.report_error("Duplicate layer", &error);
            }
        }
        if ui.button("🗑").on_hover_text(state.tr("layer.delete")).clicked() {
            let id = state.doc.active_layer;
            if let Err(error) = state.delete_layer(id) {
                state.report_error("Delete layer", &error);
            }
        }
        if ui.button("⬆").on_hover_text(state.tr("layer.move_up")).clicked() {
            let id = state.doc.active_layer;
            if let Err(error) = state.move_layer_up(id) {
                state.report_error("Move layer", &error);
            }
        }
        if ui
            .button("⬇")
            .on_hover_text(state.tr("layer.move_down"))
            .clicked()
        {
            let id = state.doc.active_layer;
            if let Err(error) = state.move_layer_down(id) {
                state.report_error("Move layer", &error);
            }
        }
        if ui
            .button("⬓")
            .on_hover_text(state.tr("layer.merge_down"))
            .clicked()
        {
            let id = state.doc.active_layer;
            if let Err(error) = state.merge_down(id) {
                state.report_error("Merge down", &error);
            }
        }
    });
}

fn active_layer_controls(ui: &mut Ui, state: &mut EditorState) {
    let id = state.doc.active_layer;
    let Some(layer) = state.doc.layers.get(id) else {
        ui.label("No layer selected");
        return;
    };
    let mut opacity = layer.opacity;
    let mut blend = layer.blend_mode;
    let mut locked = layer.locked;
    let mut alpha_lock = layer.alpha_lock;
    let mut clipping = layer.clipping;
    let mut name = layer.name.clone();
    let mut label = layer.color_label;
    let has_mask = layer.mask.is_some();

    let mut changes: Vec<LayerProperty> = Vec::new();
    if ui.text_edit_singleline(&mut name).changed() {
        changes.push(LayerProperty::Name(name));
    }
    if ui
        .add(egui::Slider::new(&mut opacity, 0.0..=1.0).text(state.language.tr("layer.opacity")))
        .changed()
    {
        changes.push(LayerProperty::Opacity(opacity));
    }
    if blend_mode_combo(ui, "layer-blend", state.language.tr("layer.blend"), &mut blend) {
        changes.push(LayerProperty::Blend(blend));
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .checkbox(&mut locked, state.language.tr("layer.lock"))
            .changed()
        {
            changes.push(LayerProperty::Locked(locked));
        }
        if ui
            .checkbox(&mut alpha_lock, state.language.tr("layer.alpha_lock"))
            .changed()
        {
            changes.push(LayerProperty::AlphaLock(alpha_lock));
        }
        if ui
            .checkbox(&mut clipping, state.language.tr("layer.clipping"))
            .changed()
        {
            changes.push(LayerProperty::Clipping(clipping));
        }
    });
    ui.horizontal_wrapped(|ui| {
        for option in ColorLabel::ALL {
            let color = option
                .swatch()
                .map(|c| Color32::from_rgb(c.r, c.g, c.b))
                .unwrap_or(Color32::from_gray(70));
            let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 2.0, color);
            if label == option {
                ui.painter().rect_stroke(
                    rect,
                    2.0,
                    egui::Stroke::new(2.0, Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }
            if response.clicked() {
                label = option;
                changes.push(LayerProperty::Label(option));
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button(state.tr("layer.mask_add")).clicked() {
            if let Err(error) = state.add_mask_from_selection(id) {
                state.report_error("Add mask", &error);
            }
        }
        if has_mask && ui.button(state.tr("layer.mask_remove")).clicked() {
            if let Err(error) = state.remove_mask(id) {
                state.report_error("Remove mask", &error);
            }
        }
    });

    for change in changes {
        if let Err(error) = state.set_layer_property(id, change) {
            state.report_error("Layer property", &error);
        }
    }
}

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

fn to_color32(color: Rgba) -> Color32 {
    let c = color.to_rgba8();
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn from_color32(color: Color32) -> Rgba {
    Rgba8::new(color.r(), color.g(), color.b(), color.a()).to_rgba()
}
