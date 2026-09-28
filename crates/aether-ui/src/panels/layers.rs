//! The layer stack, the active layer's properties and its effect stack.

use crate::icons;
use crate::panels::{blend_mode_combo, effects_section};
use crate::state::EditorState;
use aether_core::LayerId;
use aether_document::command::LayerProperty;
use aether_document::layer::{ColorLabel, LayerContent};
use egui::{Color32, RichText, Ui};

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
                    ui.label(RichText::new(icons::MASK).small())
                        .on_hover_text("Has a mask");
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
        if ui
            .button(icons::ADD)
            .on_hover_text(state.tr("layer.add"))
            .clicked()
        {
            if let Err(error) = state.add_layer() {
                state.report_error("Add layer", &error);
            }
        }
        if ui
            .button(icons::GROUP)
            .on_hover_text(state.tr("layer.add_group"))
            .clicked()
        {
            if let Err(error) = state.add_group() {
                state.report_error("Add group", &error);
            }
        }
        if ui
            .button(icons::DUPLICATE)
            .on_hover_text(state.tr("layer.duplicate"))
            .clicked()
        {
            let id = state.doc.active_layer;
            if let Err(error) = state.duplicate_layer(id) {
                state.report_error("Duplicate layer", &error);
            }
        }
        if ui
            .button(icons::DELETE)
            .on_hover_text(state.tr("layer.delete"))
            .clicked()
        {
            let id = state.doc.active_layer;
            if let Err(error) = state.delete_layer(id) {
                state.report_error("Delete layer", &error);
            }
        }
        if ui
            .button(icons::UP)
            .on_hover_text(state.tr("layer.move_up"))
            .clicked()
        {
            let id = state.doc.active_layer;
            if let Err(error) = state.move_layer_up(id) {
                state.report_error("Move layer", &error);
            }
        }
        if ui
            .button(icons::DOWN)
            .on_hover_text(state.tr("layer.move_down"))
            .clicked()
        {
            let id = state.doc.active_layer;
            if let Err(error) = state.move_layer_down(id) {
                state.report_error("Move layer", &error);
            }
        }
        if ui
            .button(icons::MERGE_DOWN)
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
    // Adjustment layers carry their parameters in their content, so they get
    // their editor right here rather than in a separate dialog.
    if let Some(LayerContent::Adjustment(content)) = state.doc.layers.get(id).map(|l| l.content.clone()) {
        ui.separator();
        let mut adjustment = content.adjustment.clone();
        if crate::panels::adjustment_editor(ui, &mut adjustment, &format!("adj-{}", id.raw()))
            && adjustment != content.adjustment
        {
            if let Err(error) = state.set_adjustment(id, adjustment) {
                state.report_error("Adjustment", &error);
            }
        }
    }

    ui.separator();
    effects_section(ui, state, id);

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
