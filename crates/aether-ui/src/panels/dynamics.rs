//! The dynamics panel: physics, procedural behaviours, drivers and
//! expressions — everything that makes a rig move without keyframes.

use crate::icons;
use crate::state::EditorState;
use aether_core::ParameterId;
use aether_document::rig::behaviour::{Breath, LookAxis, LookTarget};
use aether_document::rig::physics::{InputKind, OutputKind, PhysicsInput, PhysicsOutput, PhysicsParticle};
use aether_document::rig::{Behaviours, PhysicsGroup, Rig};
use egui::{Color32, RichText, Ui};

/// The dynamics panel.
pub fn dynamics_panel(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut state.rig.simulate, lang.tr("param.simulate"))
            .on_hover_text(lang.tr("param.simulate_hint"));
        if ui.button(lang.tr("dyn.reset")).clicked() {
            state.rig.runtime.reset();
        }
        ui.checkbox(&mut state.rig.follow_pointer, lang.tr("dyn.follow_pointer"))
            .on_hover_text(lang.tr("dyn.follow_pointer_hint"));
        if ui
            .button(lang.tr("dyn.standard"))
            .on_hover_text(lang.tr("dyn.standard_hint"))
            .clicked()
        {
            match state.add_standard_dynamics() {
                Ok(n) => state.report(format!("{} {n}", lang.tr("dyn.groups_added"))),
                Err(e) => state.report_error("Physics", &e),
            }
        }
    });
    for issue in state.rig.runtime.issues.clone() {
        ui.colored_label(
            Color32::from_rgb(240, 130, 100),
            format!("{} {}", icons::WARNING, issue.message),
        );
    }
    ui.separator();
    egui::ScrollArea::vertical().id_salt("dynamics").show(ui, |ui| {
        egui::CollapsingHeader::new(lang.tr("dyn.physics"))
            .id_salt("dyn-physics")
            .default_open(true)
            .show(ui, |ui| physics_section(ui, state));
        egui::CollapsingHeader::new(lang.tr("dyn.behaviours"))
            .id_salt("dyn-behaviours")
            .default_open(true)
            .show(ui, |ui| behaviours_section(ui, state));
        egui::CollapsingHeader::new(lang.tr("dyn.drivers"))
            .id_salt("dyn-drivers")
            .show(ui, |ui| drivers_section(ui, state));
        egui::CollapsingHeader::new(lang.tr("dyn.expressions"))
            .id_salt("dyn-expressions")
            .show(ui, |ui| expressions_section(ui, state));
    });
}

fn commit_rig(state: &mut EditorState, label: &str, key: String, rig: Rig) {
    let result = state.edit_rig(label, Some(key), move |target, _| {
        let values = std::mem::take(&mut target.values);
        *target = rig;
        target.values = values;
        Ok(())
    });
    if let Err(e) = result {
        state.report_error(label, &e);
    }
}

fn param_pick(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, rig: &Rig, value: &mut ParameterId) {
    let text = rig
        .parameter(*value)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "—".into());
    egui::ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(110.0)
        .show_ui(ui, |ui| {
            for p in &rig.parameters {
                ui.selectable_value(value, p.id, &p.name);
            }
        });
}

fn physics_section(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    let original = state.doc.rig.clone();
    let mut rig = original.clone();
    let mut remove: Option<usize> = None;
    if ui.button(lang.tr("dyn.add_group")).clicked() {
        let first = rig.parameters.first().map(|p| p.id).unwrap_or(ParameterId::NONE);
        let mut group = PhysicsGroup::sway(format!("Chain {}", rig.physics.len() + 1), &[], first);
        group.inputs.push(PhysicsInput {
            param: first,
            kind: InputKind::X,
            weight: 1.0,
            invert: false,
        });
        rig.physics.push(group);
    }
    for (g, group) in rig.physics.iter_mut().enumerate() {
        ui.push_id(("physics", g), |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut group.enabled, "");
                    ui.text_edit_singleline(&mut group.name);
                    if ui.small_button(icons::DELETE).clicked() {
                        remove = Some(g);
                    }
                });
                ui.label(RichText::new(lang.tr("dyn.inputs")).strong());
                let mut drop_input = None;
                for (i, input) in group.inputs.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        param_pick(ui, ("in", g, i), &original, &mut input.param);
                        egui::ComboBox::from_id_salt(("in-kind", g, i))
                            .selected_text(input_kind_name(input.kind, lang))
                            .show_ui(ui, |ui| {
                                for kind in InputKind::ALL {
                                    ui.selectable_value(&mut input.kind, kind, input_kind_name(kind, lang));
                                }
                            });
                        ui.add(
                            egui::DragValue::new(&mut input.weight)
                                .range(-2.0..=2.0)
                                .speed(0.01),
                        );
                        ui.checkbox(&mut input.invert, lang.tr("dyn.invert"));
                        if ui.small_button(icons::CLOSE).clicked() {
                            drop_input = Some(i);
                        }
                    });
                }
                if let Some(i) = drop_input {
                    group.inputs.remove(i);
                }
                if ui.small_button(lang.tr("dyn.add_input")).clicked() {
                    let param = original
                        .parameters
                        .first()
                        .map(|p| p.id)
                        .unwrap_or(ParameterId::NONE);
                    group.inputs.push(PhysicsInput {
                        param,
                        kind: InputKind::X,
                        weight: 0.5,
                        invert: false,
                    });
                }
                ui.label(RichText::new(lang.tr("dyn.outputs")).strong());
                let mut drop_output = None;
                let particles = group.particles.len().max(1);
                for (i, output) in group.outputs.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        param_pick(ui, ("out", g, i), &original, &mut output.param);
                        egui::ComboBox::from_id_salt(("out-kind", g, i))
                            .selected_text(output_kind_name(output.kind, lang))
                            .show_ui(ui, |ui| {
                                for kind in OutputKind::ALL {
                                    ui.selectable_value(&mut output.kind, kind, output_kind_name(kind, lang));
                                }
                            });
                        ui.add(
                            egui::DragValue::new(&mut output.particle)
                                .range(1..=particles)
                                .prefix("#"),
                        );
                        ui.add(
                            egui::DragValue::new(&mut output.scale)
                                .range(-5.0..=5.0)
                                .speed(0.01),
                        );
                        ui.checkbox(&mut output.invert, lang.tr("dyn.invert"));
                        if ui.small_button(icons::CLOSE).clicked() {
                            drop_output = Some(i);
                        }
                    });
                }
                if let Some(i) = drop_output {
                    group.outputs.remove(i);
                }
                if ui.small_button(lang.tr("dyn.add_output")).clicked() {
                    let param = original
                        .parameters
                        .first()
                        .map(|p| p.id)
                        .unwrap_or(ParameterId::NONE);
                    group.outputs.push(PhysicsOutput {
                        param,
                        particle: particles,
                        kind: OutputKind::Angle,
                        scale: 1.0,
                        invert: false,
                    });
                }
                ui.label(RichText::new(lang.tr("dyn.chain")).strong());
                let mut count = group.particles.len();
                ui.horizontal(|ui| {
                    ui.label(lang.tr("dyn.links"));
                    ui.add(egui::DragValue::new(&mut count).range(1..=12));
                });
                if count != group.particles.len() {
                    let template = group
                        .particles
                        .last()
                        .cloned()
                        .unwrap_or_else(|| PhysicsParticle::new(12.0));
                    group.particles.resize(count, template);
                    for o in &mut group.outputs {
                        o.particle = o.particle.clamp(1, count);
                    }
                }
                if let Some(first) = group.particles.first().cloned() {
                    // One set of link settings for the whole chain keeps the
                    // panel manageable; per-link values are kept in the file.
                    let mut link = first.clone();
                    ui.add(egui::Slider::new(&mut link.length, 1.0..=60.0).text(lang.tr("dyn.length")));
                    ui.add(egui::Slider::new(&mut link.damping, 0.0..=10.0).text(lang.tr("rig.damping")));
                    ui.add(
                        egui::Slider::new(&mut link.stiffness, 0.0..=200.0).text(lang.tr("rig.stiffness")),
                    );
                    ui.add(
                        egui::Slider::new(&mut link.max_angle, 5.0..=180.0).text(lang.tr("dyn.max_angle")),
                    );
                    if link != first {
                        for p in &mut group.particles {
                            *p = link.clone();
                        }
                    }
                }
                ui.add(egui::Slider::new(&mut group.gravity.y, 0.0..=4000.0).text(lang.tr("rig.gravity")));
                ui.add(egui::Slider::new(&mut group.wind.x, -3000.0..=3000.0).text(lang.tr("dyn.wind")));
                ui.add(egui::Slider::new(&mut group.turbulence, 0.0..=2.0).text(lang.tr("dyn.turbulence")));
                ui.add(
                    egui::Slider::new(&mut group.translation_range, 0.0..=40.0).text(lang.tr("dyn.travel")),
                );
                ui.add(egui::Slider::new(&mut group.angle_range, 0.0..=60.0).text(lang.tr("dyn.tilt")));
                if let Err(e) = group.validate() {
                    ui.colored_label(Color32::from_rgb(240, 130, 100), e.to_string());
                }
            });
        });
    }
    if let Some(g) = remove {
        rig.physics.remove(g);
    }
    if rig.physics != original.physics {
        commit_rig(state, "Physics", "physics".into(), rig);
    }
}

fn input_kind_name(kind: InputKind, lang: crate::i18n::Language) -> &'static str {
    match kind {
        InputKind::X => lang.tr("dyn.kind.x"),
        InputKind::Y => lang.tr("dyn.kind.y"),
        InputKind::Angle => lang.tr("dyn.kind.angle"),
    }
}

fn output_kind_name(kind: OutputKind, lang: crate::i18n::Language) -> &'static str {
    match kind {
        OutputKind::X => lang.tr("dyn.kind.x"),
        OutputKind::Y => lang.tr("dyn.kind.y"),
        OutputKind::Angle => lang.tr("dyn.kind.angle"),
    }
}

fn behaviours_section(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    let original = state.doc.rig.clone();
    let mut b: Behaviours = original.behaviours.clone();
    if ui.button(lang.tr("dyn.standard_behaviours")).clicked() {
        b = Behaviours::standard(&original.parameters);
    }

    // Blink.
    ui.checkbox(&mut b.blink.enabled, RichText::new(lang.tr("dyn.blink")).strong());
    ui.horizontal_wrapped(|ui| {
        for p in &original.parameters {
            let mut on = b.blink.params.contains(&p.id);
            if ui.toggle_value(&mut on, &p.name).changed() {
                if on {
                    b.blink.params.push(p.id);
                } else {
                    b.blink.params.retain(|x| *x != p.id);
                }
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(lang.tr("dyn.interval"));
        ui.add(
            egui::DragValue::new(&mut b.blink.min_interval)
                .range(0.2..=20.0)
                .speed(0.05),
        );
        ui.label("–");
        ui.add(
            egui::DragValue::new(&mut b.blink.max_interval)
                .range(0.2..=30.0)
                .speed(0.05),
        );
        ui.label("s");
    });
    ui.add(egui::Slider::new(&mut b.blink.double_chance, 0.0..=1.0).text(lang.tr("dyn.double_blink")));

    // Breath.
    ui.separator();
    ui.label(RichText::new(lang.tr("dyn.breath")).strong());
    let mut drop = None;
    for (i, breath) in b.breath.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            param_pick(ui, ("breath", i), &original, &mut breath.param);
            ui.add(
                egui::DragValue::new(&mut breath.period)
                    .range(0.2..=20.0)
                    .speed(0.05)
                    .suffix(" s"),
            );
            ui.add(
                egui::DragValue::new(&mut breath.amplitude)
                    .range(0.0..=2.0)
                    .speed(0.01),
            );
            if ui.small_button(icons::CLOSE).clicked() {
                drop = Some(i);
            }
        });
    }
    if let Some(i) = drop {
        b.breath.remove(i);
    }
    if ui.small_button(lang.tr("dyn.add_breath")).clicked() {
        if let Some(p) = original.parameters.first() {
            b.breath.push(Breath {
                param: p.id,
                period: 3.5,
                amplitude: 0.5,
                offset: 0.0,
                phase: 0.0,
            });
        }
    }

    // Look-at.
    ui.separator();
    ui.checkbox(&mut b.look.enabled, RichText::new(lang.tr("dyn.look")).strong());
    let mut drop = None;
    for (i, target) in b.look.targets.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            param_pick(ui, ("look", i), &original, &mut target.param);
            ui.selectable_value(&mut target.axis, LookAxis::X, "X");
            ui.selectable_value(&mut target.axis, LookAxis::Y, "Y");
            ui.add(
                egui::DragValue::new(&mut target.gain)
                    .range(-2.0..=2.0)
                    .speed(0.01),
            );
            if ui.small_button(icons::CLOSE).clicked() {
                drop = Some(i);
            }
        });
    }
    if let Some(i) = drop {
        b.look.targets.remove(i);
    }
    if ui.small_button(lang.tr("dyn.add_look")).clicked() {
        if let Some(p) = original.parameters.first() {
            b.look.targets.push(LookTarget {
                param: p.id,
                axis: LookAxis::X,
                gain: 1.0,
            });
        }
    }
    ui.add(egui::Slider::new(&mut b.look.smoothing, 0.0..=1.0).text(lang.tr("dyn.smoothing")));

    // Lip sync (live).
    ui.separator();
    ui.checkbox(
        &mut b.lip_sync.enabled,
        RichText::new(lang.tr("dyn.lip_sync")).strong(),
    )
    .on_hover_text(lang.tr("dyn.lip_sync_hint"));
    ui.add(egui::Slider::new(&mut b.lip_sync.gain, 0.1..=4.0).text(lang.tr("timeline.gain")));

    if b != original.behaviours {
        let mut rig = original;
        rig.behaviours = b;
        commit_rig(state, "Behaviours", "behaviours".into(), rig);
    }
}

fn drivers_section(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.label(RichText::new(lang.tr("dyn.drivers_hint")).weak().small());
    let drivers = state.doc.rig.drivers.clone();
    for (i, driver) in drivers.iter().enumerate() {
        let name = state
            .doc
            .rig
            .parameter(driver.target)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            let mut enabled = driver.enabled;
            if ui.checkbox(&mut enabled, "").changed() {
                let r = state.edit_rig("Driver", None, |rig, _| {
                    if let Some(d) = rig.drivers.get_mut(i) {
                        d.enabled = enabled;
                    }
                    Ok(())
                });
                if let Err(e) = r {
                    state.report_error("Driver", &e);
                }
            }
            ui.label(RichText::new(format!("{name} =")).strong());
            if ui
                .add(
                    egui::Label::new(RichText::new(&driver.expression).monospace())
                        .sense(egui::Sense::click()),
                )
                .on_hover_text(lang.tr("dyn.edit_driver"))
                .clicked()
            {
                state.rig.editing_driver = Some((driver.target, driver.expression.clone()));
            }
        });
    }
    ui.label(RichText::new(lang.tr("dyn.add_driver_hint")).weak().small());
}

fn expressions_section(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.rig.name_draft).hint_text(lang.tr("dyn.expression_name")),
        );
        if ui
            .button(lang.tr("dyn.capture"))
            .on_hover_text(lang.tr("dyn.capture_hint"))
            .clicked()
        {
            let name = if state.rig.name_draft.trim().is_empty() {
                format!("Expression {}", state.doc.rig.expressions.len() + 1)
            } else {
                state.rig.name_draft.clone()
            };
            match state.capture_expression(&name) {
                Ok(()) => state.rig.name_draft.clear(),
                Err(e) => state.report_error("Expression", &e),
            }
        }
    });
    let active = state.rig.runtime.active_expression();
    let names: Vec<String> = state.doc.rig.expressions.iter().map(|e| e.name.clone()).collect();
    let mut remove = None;
    for (i, name) in names.iter().enumerate() {
        ui.horizontal(|ui| {
            let on = active == Some(i);
            if ui
                .selectable_label(on, name)
                .on_hover_text(lang.tr("dyn.preview_expression"))
                .clicked()
            {
                state.rig.runtime.set_expression(if on { None } else { Some(i) });
                state.rig.simulate = true;
            }
            if ui.small_button(icons::DELETE).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        let r = state.edit_rig("Delete expression", None, |rig, _| {
            if i < rig.expressions.len() {
                rig.expressions.remove(i);
            }
            Ok(())
        });
        state.rig.runtime.set_expression(None);
        if let Err(e) = r {
            state.report_error("Expression", &e);
        }
    }
}
