//! The parameter panel: posing the rig and keying objects.
//!
//! Every parameter is a slider, grouped into folders. For the selected rig
//! object the slider shows diamonds where it has keys, snaps onto them, and a
//! key button beside it adds or removes the key at the current value — the
//! loop that builds up a rig: pick an object, put the slider on a key, shape
//! the object with the deform tool. In animate mode the same sliders key the
//! motion at the playhead instead.

use crate::icons;
use crate::state::EditorState;
use aether_core::ParameterId;
use aether_document::rig::{Parameter, RigNode};
use egui::{Color32, RichText, Stroke, Ui};

/// Fraction of a parameter's range within which a slider snaps onto a key.
const SNAP: f32 = 0.025;

/// The parameter panel.
pub fn parameters_panel(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(lang.tr("param.add"))
            .on_hover_text(lang.tr("param.add_hint"))
            .clicked()
        {
            match state.add_parameter("Param", -1.0, 1.0, 0.0) {
                Ok(id) => state.rig.editing_parameter = state.doc.rig.parameter(id).cloned(),
                Err(e) => state.report_error("Add parameter", &e),
            }
        }
        if ui
            .button(lang.tr("param.standard"))
            .on_hover_text(lang.tr("param.standard_hint"))
            .clicked()
        {
            if let Err(e) = state.add_standard_parameters() {
                state.report_error("Standard parameters", &e);
            }
        }
        if ui.button(lang.tr("param.reset_pose")).clicked() {
            state.reset_pose();
        }
        ui.checkbox(&mut state.rig.simulate, lang.tr("param.simulate"))
            .on_hover_text(lang.tr("param.simulate_hint"));
    });

    selection_banner(ui, state);
    ui.separator();

    if state.doc.rig.parameters.is_empty() {
        ui.label(RichText::new(lang.tr("param.none")).weak());
    }

    // Folders in first-appearance order.
    let mut groups: Vec<String> = Vec::new();
    for p in &state.doc.rig.parameters {
        if !groups.contains(&p.group) {
            groups.push(p.group.clone());
        }
    }
    egui::ScrollArea::vertical()
        .id_salt("param-scroll")
        .show(ui, |ui| {
            for group in groups {
                let title = if group.is_empty() {
                    lang.tr("param.group_general").to_string()
                } else {
                    group.clone()
                };
                egui::CollapsingHeader::new(title)
                    .id_salt(("param-group", group.clone()))
                    .default_open(true)
                    .show(ui, |ui| {
                        let ids: Vec<ParameterId> = state
                            .doc
                            .rig
                            .parameters
                            .iter()
                            .filter(|p| p.group == group)
                            .map(|p| p.id)
                            .collect();
                        for id in ids {
                            parameter_row(ui, state, id);
                        }
                    });
            }
        });

    parameter_dialog(ui.ctx(), state);
    driver_dialog(ui.ctx(), state);
}

/// What is being keyed, and in which mode.
fn selection_banner(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    let name = state
        .rig
        .tool
        .selection
        .map(|n| state.doc.rig.node_name(n))
        .unwrap_or_else(|| "—".into());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(lang.tr("param.keys_for")).weak());
        ui.label(RichText::new(name).strong());
    });
    if let (Some(node), Some(index)) = (state.rig.tool.selection, state.rig.tool.blend_shape) {
        let shape_name = blend_shape_name(state, node, index);
        ui.horizontal(|ui| {
            ui.colored_label(
                Color32::from_rgb(230, 170, 90),
                format!("{} {shape_name}", lang.tr("param.editing_shape")),
            );
            if ui
                .small_button(icons::CLOSE)
                .on_hover_text(lang.tr("param.edit_base"))
                .clicked()
            {
                state.rig.tool.blend_shape = None;
            }
        });
    }
    if state.rig.animate && state.rig.motion.is_some() {
        let time = state.snapped_playhead();
        ui.colored_label(
            Color32::from_rgb(120, 190, 250),
            format!("{} {time:.2}s", lang.tr("param.animate_banner")),
        );
    }
}

fn blend_shape_name(state: &EditorState, node: RigNode, index: usize) -> String {
    let rig = &state.doc.rig;
    match node {
        RigNode::Mesh(layer) => rig
            .mesh(layer)
            .and_then(|m| m.blend_shapes.get(index))
            .map(|s| s.name.clone()),
        RigNode::Deformer(id) => rig.deformer(id).and_then(|d| match &d.kind {
            aether_document::rig::DeformerKind::Warp(w) => w.blend_shapes.get(index).map(|s| s.name.clone()),
            aether_document::rig::DeformerKind::Rotation(r) => {
                r.blend_shapes.get(index).map(|s| s.name.clone())
            }
        }),
        RigNode::Bone(_) => None,
    }
    .unwrap_or_default()
}

/// Keys of the selection's axis for `id`, if it is bound.
fn selection_keys(state: &EditorState, id: ParameterId) -> Option<Vec<f32>> {
    let node = state.rig.tool.selection?;
    let grid = state.doc.rig.grid(node, state.rig.tool.blend_shape)?;
    grid.axes().iter().find(|a| a.param == id).map(|a| a.keys.clone())
}

fn parameter_row(ui: &mut Ui, state: &mut EditorState, id: ParameterId) {
    let lang = state.language;
    let Some(p) = state.doc.rig.parameter(id).cloned() else {
        return;
    };
    let authored = state.doc.rig.value(id);
    let effective = state.doc.rig.effective_value(id);
    let animating = state.rig.animate && state.rig.motion.is_some();
    let shown = if animating { effective } else { authored };
    let keys = selection_keys(state, id);
    let driven = state.doc.rig.drivers.iter().any(|d| d.target == id && d.enabled);
    let tracked = state
        .rig
        .motion
        .and_then(|m| state.doc.rig.motions.get(m))
        .map(|m| m.track(id).is_some())
        .unwrap_or(false);

    ui.horizontal(|ui| {
        let mut label = RichText::new(&p.name);
        if keys.is_some() {
            label = label.strong().color(Color32::from_rgb(250, 205, 110));
        }
        let name = ui
            .add_sized(
                [96.0, 18.0],
                egui::Label::new(label).truncate().sense(egui::Sense::click()),
            )
            .on_hover_text(format!("{} … {} (default {})", p.min, p.max, p.default));
        name.context_menu(|ui| parameter_menu(ui, state, id));
        if name.double_clicked() {
            state.rig.editing_parameter = Some(p.clone());
        }

        let mut value = shown;
        let slider = ui.add(
            egui::Slider::new(&mut value, p.min..=p.max)
                .clamping(egui::SliderClamping::Always)
                .max_decimals(2),
        );
        if slider.changed() {
            // Snap onto the selection's keys, so keyforms are easy to reach.
            if let Some(keys) = &keys {
                if let Some(k) = keys.iter().find(|k| (*k - value).abs() <= p.span() * SNAP) {
                    value = *k;
                }
            }
            if let Err(e) = state.set_parameter_value(id, value) {
                state.report_error("Parameter", &e);
            }
        }

        // Key diamonds (selection) and the live value (drivers/physics).
        let rail_width = ui.spacing().slider_width;
        let rail = egui::Rect::from_min_size(slider.rect.min, egui::vec2(rail_width, slider.rect.height()));
        let x_of = |v: f32| rail.left() + 6.0 + (v - p.min) / p.span() * (rail.width() - 12.0);
        let painter = ui.painter_at(slider.rect.expand(4.0));
        if let Some(keys) = &keys {
            for k in keys {
                let c = egui::pos2(x_of(*k), rail.bottom() - 1.0);
                let on = (k - authored).abs() < 1e-4;
                let color = if on {
                    Color32::from_rgb(255, 210, 90)
                } else {
                    Color32::from_rgb(200, 160, 80)
                };
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        c + egui::vec2(0.0, -4.0),
                        c + egui::vec2(4.0, 0.0),
                        c + egui::vec2(0.0, 4.0),
                        c + egui::vec2(-4.0, 0.0),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        if !animating && (effective - authored).abs() > p.span() * 0.002 {
            painter.circle_filled(
                egui::pos2(x_of(effective), rail.top() + 2.0),
                3.0,
                Color32::from_rgb(120, 200, 255),
            );
        }

        // Key toggle for the selection.
        if state.rig.tool.selection.is_some() && !animating {
            let on_key = keys
                .as_ref()
                .map(|k| k.iter().any(|x| (x - authored).abs() < 1e-4))
                .unwrap_or(false);
            let (glyph, hint) = if on_key {
                (icons::KEY_ON, lang.tr("param.remove_key"))
            } else {
                (icons::KEY_OFF, lang.tr("param.add_key"))
            };
            if ui.small_button(glyph).on_hover_text(hint).clicked() {
                let result = if on_key {
                    state.remove_key_here(id)
                } else {
                    state.add_key_here(id)
                };
                if let Err(e) = result {
                    state.report_error("Key", &e);
                }
            }
        }
        if driven {
            ui.label(RichText::new(icons::DRIVEN).color(Color32::from_rgb(160, 220, 160)))
                .on_hover_text(lang.tr("param.driven"));
        }
        if tracked {
            ui.label(
                RichText::new(icons::ANIMATED)
                    .small()
                    .color(Color32::from_rgb(120, 190, 250)),
            )
            .on_hover_text(lang.tr("param.animated"));
        }
    });
}

fn parameter_menu(ui: &mut Ui, state: &mut EditorState, id: ParameterId) {
    let lang = state.language;
    let has_selection = state.rig.tool.selection.is_some();
    let mut result: Option<(&str, aether_core::Result<()>)> = None;
    if ui
        .add_enabled(has_selection, egui::Button::new(lang.tr("param.menu.three_keys")))
        .clicked()
    {
        result = Some(("Add keys", state.bind_three_keys(id)));
        ui.close();
    }
    if ui
        .add_enabled(has_selection, egui::Button::new(lang.tr("param.add_key")))
        .clicked()
    {
        result = Some(("Add key", state.add_key_here(id)));
        ui.close();
    }
    if ui
        .add_enabled(has_selection, egui::Button::new(lang.tr("param.remove_key")))
        .clicked()
    {
        result = Some(("Remove key", state.remove_key_here(id)));
        ui.close();
    }
    if ui
        .add_enabled(has_selection, egui::Button::new(lang.tr("param.menu.mirror")))
        .on_hover_text(lang.tr("param.menu.mirror_hint"))
        .clicked()
    {
        result = Some(("Mirror key", state.mirror_key(id)));
        ui.close();
    }
    if ui
        .add_enabled(
            has_selection,
            egui::Button::new(lang.tr("param.menu.blend_shape")),
        )
        .on_hover_text(lang.tr("param.menu.blend_shape_hint"))
        .clicked()
    {
        result = Some(("Blend shape", state.add_blend_shape(id)));
        ui.close();
    }
    if ui
        .add_enabled(has_selection, egui::Button::new(lang.tr("param.menu.unbind")))
        .clicked()
    {
        result = Some(("Unbind", state.unbind_parameter(id)));
        ui.close();
    }
    ui.separator();
    if ui.button(lang.tr("param.menu.edit")).clicked() {
        state.rig.editing_parameter = state.doc.rig.parameter(id).cloned();
        ui.close();
    }
    if ui.button(lang.tr("param.menu.driver")).clicked() {
        let existing = state
            .doc
            .rig
            .drivers
            .iter()
            .find(|d| d.target == id)
            .map(|d| d.expression.clone())
            .unwrap_or_default();
        state.rig.editing_driver = Some((id, existing));
        ui.close();
    }
    if ui.button(lang.tr("param.menu.delete")).clicked() {
        result = Some(("Delete parameter", state.remove_parameter(id)));
        ui.close();
    }
    if let Some((context, Err(e))) = result {
        state.report_error(context, &e);
    }
}

/// Edit name, range, group and wrapping of a parameter.
fn parameter_dialog(ctx: &egui::Context, state: &mut EditorState) {
    let Some(mut draft) = state.rig.editing_parameter.clone() else {
        return;
    };
    let lang = state.language;
    let mut open = true;
    let mut apply = false;
    let mut close = false;
    egui::Window::new(lang.tr("param.dialog.title"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new("param-dialog").num_columns(2).show(ui, |ui| {
                ui.label(lang.tr("dialog.name"));
                ui.text_edit_singleline(&mut draft.name);
                ui.end_row();
                ui.label(lang.tr("param.dialog.group"));
                ui.text_edit_singleline(&mut draft.group);
                ui.end_row();
                ui.label(lang.tr("param.dialog.min"));
                ui.add(egui::DragValue::new(&mut draft.min).speed(0.1));
                ui.end_row();
                ui.label(lang.tr("param.dialog.max"));
                ui.add(egui::DragValue::new(&mut draft.max).speed(0.1));
                ui.end_row();
                ui.label(lang.tr("param.dialog.default"));
                ui.add(egui::DragValue::new(&mut draft.default).speed(0.1));
                ui.end_row();
                ui.label(lang.tr("param.dialog.cyclic"));
                ui.checkbox(&mut draft.cyclic, "");
                ui.end_row();
            });
            if aether_document::rig::expr::is_reserved(&draft.name) {
                ui.colored_label(Color32::from_rgb(240, 150, 90), lang.tr("param.dialog.reserved"));
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(lang.tr("dialog.apply")).clicked() {
                    apply = true;
                }
                if ui.button(lang.tr("dialog.cancel")).clicked() {
                    close = true;
                }
            });
        });
    if apply {
        match state.update_parameter(draft.clone()) {
            Ok(()) => state.rig.editing_parameter = None,
            Err(e) => {
                state.report_error("Edit parameter", &e);
                state.rig.editing_parameter = Some(draft);
            }
        }
    } else if close || !open {
        state.rig.editing_parameter = None;
    } else {
        state.rig.editing_parameter = Some(draft);
    }
}

/// Write or remove the driver expression of a parameter.
fn driver_dialog(ctx: &egui::Context, state: &mut EditorState) {
    let Some((target, mut text)) = state.rig.editing_driver.clone() else {
        return;
    };
    let lang = state.language;
    let name = state
        .doc
        .rig
        .parameter(target)
        .map(|p: &Parameter| p.name.clone())
        .unwrap_or_default();
    let check = aether_document::rig::driver::validate_expression(&state.doc.rig.parameters, &text);
    let mut open = true;
    let mut apply = false;
    let mut remove = false;
    let mut close = false;
    egui::Window::new(format!("{} — {name}", lang.tr("param.driver.title")))
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(lang.tr("param.driver.help"));
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .code_editor()
                    .desired_rows(2)
                    .desired_width(320.0),
            );
            match &check {
                Ok(()) if !text.trim().is_empty() => {
                    ui.colored_label(Color32::from_rgb(140, 210, 140), lang.tr("param.driver.ok"));
                }
                Ok(()) => {}
                Err(e) => {
                    ui.colored_label(Color32::from_rgb(240, 120, 100), e.to_string());
                }
            }
            ui.label(
                RichText::new(aether_document::rig::expr::builtin_names().join(" · "))
                    .small()
                    .weak(),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        check.is_ok() && !text.trim().is_empty(),
                        egui::Button::new(lang.tr("dialog.apply")),
                    )
                    .clicked()
                {
                    apply = true;
                }
                if ui.button(lang.tr("param.driver.remove")).clicked() {
                    remove = true;
                }
                if ui.button(lang.tr("dialog.cancel")).clicked() {
                    close = true;
                }
            });
        });
    if apply {
        match state.add_driver(target, &text) {
            Ok(()) => state.rig.editing_driver = None,
            Err(e) => state.report_error("Driver", &e),
        }
    } else if remove {
        if let Err(e) = state.edit_rig("Remove driver", None, |rig, _| {
            rig.drivers.retain(|d| d.target != target);
            Ok(())
        }) {
            state.report_error("Driver", &e);
        }
        state.rig.editing_driver = None;
    } else if close || !open {
        state.rig.editing_driver = None;
    } else {
        state.rig.editing_driver = Some((target, text));
    }
}
