//! The timeline: motions, transport, keyframes, lip sync and export.
//!
//! Each animated parameter is a row of keyframe diamonds. Click the ruler to
//! move the playhead, click a diamond to select it (and set its easing), drag
//! it to retime it, double-click a row to key that parameter at the pointer.
//! The selected track's curve is drawn underneath, so easing choices are
//! visible rather than guessed.

use crate::icons;
use crate::rigging::ExportKind;
use crate::state::EditorState;
use aether_core::ParameterId;
use aether_document::rig::motion::Easing;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui};

const NAME_WIDTH: f32 = 110.0;
const ROW_HEIGHT: f32 = 18.0;
const RULER_HEIGHT: f32 = 20.0;

/// The timeline panel.
pub fn timeline_panel(ui: &mut Ui, state: &mut EditorState) {
    motion_bar(ui, state);
    transport(ui, state);
    let Some(m) = state.rig.motion.filter(|m| *m < state.doc.rig.motions.len()) else {
        ui.separator();
        ui.label(RichText::new(state.tr("timeline.empty")).weak());
        extras(ui, state);
        return;
    };
    key_bar(ui, state, m);
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("timeline-rows")
        .max_height((ui.available_height() - 150.0).max(120.0))
        .show(ui, |ui| tracks(ui, state, m));
    curve(ui, state, m);
    extras(ui, state);
}

fn motion_bar(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.horizontal_wrapped(|ui| {
        let current = state.rig.motion;
        let text = current
            .and_then(|m| state.doc.rig.motions.get(m))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| lang.tr("timeline.no_motion").to_string());
        let mut chosen = current;
        egui::ComboBox::from_id_salt("motion-pick")
            .selected_text(text)
            .show_ui(ui, |ui| {
                for (i, motion) in state.doc.rig.motions.iter().enumerate() {
                    ui.selectable_value(&mut chosen, Some(i), &motion.name);
                }
            });
        if chosen != current {
            state.rig.motion = chosen;
            state.rig.playhead = 0.0;
            state.rig.selected_key = None;
            state.rig.animate = chosen.is_some();
        }
        if ui.button(lang.tr("timeline.new")).clicked() {
            let name = format!("Motion {}", state.doc.rig.motions.len() + 1);
            if let Err(e) = state.add_motion(&name) {
                state.report_error("Motion", &e);
            }
        }
        if ui
            .button(lang.tr("timeline.import_live2d"))
            .on_hover_text(lang.tr("timeline.import_live2d_hint"))
            .clicked()
        {
            state.import_live2d_motion_via_dialog();
        }
        if let Some(m) = state.rig.motion.filter(|m| *m < state.doc.rig.motions.len()) {
            if ui.button(lang.tr("timeline.delete")).clicked() {
                if let Err(e) = state.delete_motion(m) {
                    state.report_error("Motion", &e);
                }
                return;
            }
            if ui
                .button(lang.tr("timeline.export_live2d"))
                .on_hover_text(lang.tr("timeline.export_live2d_hint"))
                .clicked()
            {
                state.export_live2d_motion_via_dialog(m);
            }
            let mut motion = state.doc.rig.motions[m].clone();
            let before = motion.clone();
            ui.add(egui::TextEdit::singleline(&mut motion.name).desired_width(100.0));
            ui.label(lang.tr("timeline.length"));
            ui.add(
                egui::DragValue::new(&mut motion.duration)
                    .range(0.1..=600.0)
                    .speed(0.05)
                    .suffix(" s"),
            );
            ui.label(lang.tr("timeline.fps"));
            ui.add(egui::DragValue::new(&mut motion.fps).range(1.0..=120.0));
            ui.checkbox(&mut motion.looping, lang.tr("timeline.loop"));
            ui.label(lang.tr("timeline.fade"));
            ui.add(
                egui::DragValue::new(&mut motion.fade_in)
                    .range(0.0..=5.0)
                    .speed(0.01)
                    .suffix(" s"),
            );
            if motion != before {
                let r = state.edit_rig("Motion settings", Some(format!("motion:{m}")), |rig, _| {
                    if let Some(slot) = rig.motions.get_mut(m) {
                        *slot = motion;
                    }
                    Ok(())
                });
                if let Err(e) = r {
                    state.report_error("Motion", &e);
                }
            }
        }
    });
}

fn transport(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(icons::TO_START)
            .on_hover_text(lang.tr("timeline.start"))
            .clicked()
        {
            state.rig.playhead = 0.0;
            state.rig.animate = state.rig.motion.is_some();
        }
        if ui
            .button(icons::PREV_FRAME)
            .on_hover_text(lang.tr("timeline.prev_frame"))
            .clicked()
        {
            state.step_frames(-1);
        }
        let play = if state.rig.playing {
            icons::PAUSE
        } else {
            icons::PLAY
        };
        if ui.button(play).on_hover_text(lang.tr("timeline.play")).clicked() {
            state.toggle_playback();
        }
        if ui
            .button(icons::NEXT_FRAME)
            .on_hover_text(lang.tr("timeline.next_frame"))
            .clicked()
        {
            state.step_frames(1);
        }
        if ui
            .button(icons::STOP)
            .on_hover_text(lang.tr("timeline.stop"))
            .clicked()
        {
            state.rig.playing = false;
            state.rig.playhead = 0.0;
        }
        let (duration, fps) = state
            .rig
            .motion
            .and_then(|m| state.doc.rig.motions.get(m))
            .map(|m| (m.duration, m.fps))
            .unwrap_or((0.0, 30.0));
        ui.label(format!(
            "{:.2} / {:.2} s  ({} {})",
            state.rig.playhead,
            duration,
            lang.tr("timeline.frame"),
            (state.rig.playhead * fps).round() as i64
        ));
        ui.separator();
        ui.checkbox(&mut state.rig.animate, lang.tr("timeline.animate"))
            .on_hover_text(lang.tr("timeline.animate_hint"));
        ui.checkbox(&mut state.rig.auto_key, lang.tr("timeline.auto_key"));
        ui.checkbox(&mut state.rig.simulate, lang.tr("param.simulate"));
        if ui.button(lang.tr("timeline.key_all")).clicked() {
            if let Err(e) = state.key_all_at_playhead() {
                state.report_error("Key", &e);
            }
        }
    });
}

fn key_bar(ui: &mut Ui, state: &mut EditorState, m: usize) {
    let lang = state.language;
    let Some((param, index)) = state.rig.selected_key else {
        ui.label(RichText::new(lang.tr("timeline.key_hint")).weak().small());
        return;
    };
    let Some(key) = state.doc.rig.motions[m]
        .track(param)
        .and_then(|t| t.keys.get(index))
        .copied()
    else {
        state.rig.selected_key = None;
        return;
    };
    ui.horizontal_wrapped(|ui| {
        let name = state
            .doc
            .rig
            .parameter(param)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        ui.label(RichText::new(format!("{name} @ {:.2}s = {:.2}", key.time, key.value)).strong());
        ui.label(lang.tr("timeline.easing"));
        let mut easing = key.easing;
        egui::ComboBox::from_id_salt("key-easing")
            .selected_text(easing.name())
            .show_ui(ui, |ui| {
                for preset in Easing::PRESETS {
                    let same_kind = std::mem::discriminant(&preset) == std::mem::discriminant(&easing);
                    if ui.selectable_label(same_kind, preset.name()).clicked() && !same_kind {
                        easing = preset;
                    }
                }
            });
        // Parameters of the parametric curves.
        match &mut easing {
            Easing::Bezier { x1, y1, x2, y2 } => {
                ui.add(
                    egui::DragValue::new(x1)
                        .range(0.0..=1.0)
                        .speed(0.01)
                        .prefix("x1 "),
                );
                ui.add(
                    egui::DragValue::new(y1)
                        .range(-1.0..=2.0)
                        .speed(0.01)
                        .prefix("y1 "),
                );
                ui.add(
                    egui::DragValue::new(x2)
                        .range(0.0..=1.0)
                        .speed(0.01)
                        .prefix("x2 "),
                );
                ui.add(
                    egui::DragValue::new(y2)
                        .range(-1.0..=2.0)
                        .speed(0.01)
                        .prefix("y2 "),
                );
            }
            Easing::Spring { frequency, damping } => {
                ui.add(
                    egui::DragValue::new(frequency)
                        .range(0.1..=10.0)
                        .speed(0.05)
                        .prefix("f "),
                );
                ui.add(
                    egui::DragValue::new(damping)
                        .range(0.02..=1.0)
                        .speed(0.01)
                        .prefix("ζ "),
                );
            }
            _ => {}
        }
        if easing != key.easing {
            if let Err(e) = state.set_key_easing(param, index, easing) {
                state.report_error("Easing", &e);
            }
        }
        if ui.button(lang.tr("timeline.delete_key")).clicked() {
            if let Err(e) = state.delete_keyframe(param, index) {
                state.report_error("Key", &e);
            }
        }
    });
}

/// Map time to x within the key area.
fn time_x(area: Rect, time: f32, duration: f32) -> f32 {
    area.left() + (time / duration.max(1e-3)).clamp(0.0, 1.0) * area.width()
}

fn tracks(ui: &mut Ui, state: &mut EditorState, m: usize) {
    let lang = state.language;
    let motion = state.doc.rig.motions[m].clone();
    let duration = motion.duration.max(1e-3);
    // Rows: every parameter that has a track, then the rest on demand.
    let mut rows: Vec<ParameterId> = motion.tracks.iter().map(|t| t.param).collect();
    let show_all = ui.memory(|mem| {
        mem.data
            .get_temp::<bool>(egui::Id::new("timeline-all"))
            .unwrap_or(false)
    });
    if show_all {
        for p in &state.doc.rig.parameters {
            if !rows.contains(&p.id) {
                rows.push(p.id);
            }
        }
    }
    let width = ui.available_width();
    let height = RULER_HEIGHT + ROW_HEIGHT * rows.len().max(1) as f32;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let area = Rect::from_min_max(Pos2::new(rect.left() + NAME_WIDTH, rect.top()), rect.max);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);

    // Ruler with second and frame ticks.
    let ruler = Rect::from_min_size(area.min, egui::vec2(area.width(), RULER_HEIGHT));
    painter.rect_filled(ruler, 0.0, ui.visuals().faint_bg_color);
    let frames = (duration * motion.fps).round() as i32;
    let frame_px = area.width() / frames.max(1) as f32;
    for f in 0..=frames {
        let t = f as f32 / motion.fps;
        let x = time_x(area, t, duration);
        let whole = (t.fract() < 0.5 / motion.fps) || (1.0 - t.fract() < 0.5 / motion.fps);
        if whole {
            painter.line_segment(
                [Pos2::new(x, ruler.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(28)),
            );
            painter.text(
                Pos2::new(x + 2.0, ruler.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{:.0}s", t),
                egui::FontId::proportional(10.0),
                ui.visuals().weak_text_color(),
            );
        } else if frame_px > 4.0 {
            painter.line_segment(
                [Pos2::new(x, ruler.bottom() - 4.0), Pos2::new(x, ruler.bottom())],
                Stroke::new(1.0, Color32::from_white_alpha(50)),
            );
        }
    }

    // Rows.
    let mut hovered_key: Option<(ParameterId, usize)> = None;
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    for (row, id) in rows.iter().enumerate() {
        let top = rect.top() + RULER_HEIGHT + row as f32 * ROW_HEIGHT;
        let row_rect = Rect::from_min_size(Pos2::new(rect.left(), top), egui::vec2(width, ROW_HEIGHT));
        if row % 2 == 1 {
            painter.rect_filled(row_rect, 0.0, Color32::from_white_alpha(6));
        }
        let name = state
            .doc
            .rig
            .parameter(*id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        painter.text(
            Pos2::new(rect.left() + 4.0, top + ROW_HEIGHT * 0.5),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(11.0),
            ui.visuals().text_color(),
        );
        if let Some(track) = motion.track(*id) {
            for (k, key) in track.keys.iter().enumerate() {
                let c = Pos2::new(time_x(area, key.time, duration), top + ROW_HEIGHT * 0.5);
                let selected = state.rig.selected_key == Some((*id, k));
                let color = if selected {
                    Color32::from_rgb(255, 214, 90)
                } else {
                    Color32::from_rgb(170, 190, 230)
                };
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        c + egui::vec2(0.0, -5.0),
                        c + egui::vec2(5.0, 0.0),
                        c + egui::vec2(0.0, 5.0),
                        c + egui::vec2(-5.0, 0.0),
                    ],
                    color,
                    Stroke::new(1.0, Color32::from_black_alpha(120)),
                ));
                if let Some(p) = pointer {
                    if p.distance(c) <= 6.0 {
                        hovered_key = Some((*id, k));
                    }
                }
            }
        }
    }

    // Playhead.
    let x = time_x(area, state.rig.playhead, duration);
    painter.line_segment(
        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
        Stroke::new(2.0, Color32::from_rgb(240, 90, 90)),
    );

    // Interaction.
    let time_at = |p: Pos2| ((p.x - area.left()) / area.width()).clamp(0.0, 1.0) * duration;
    let snap = |t: f32| (t * motion.fps).round() / motion.fps;
    if let Some(p) = pointer {
        let in_ruler = p.y < rect.top() + RULER_HEIGHT && p.x >= area.left();
        let drag_key_id = egui::Id::new("timeline-drag-key");
        if response.drag_started() {
            let target = if in_ruler { None } else { hovered_key };
            ui.memory_mut(|mem| mem.data.insert_temp(drag_key_id, target));
            if let Some(key) = target {
                state.rig.selected_key = Some(key);
            }
        }
        let dragging_key = ui.memory(|mem| {
            mem.data
                .get_temp::<Option<(ParameterId, usize)>>(drag_key_id)
                .flatten()
        });
        if response.dragged() {
            match dragging_key {
                Some((param, index)) => {
                    let t = snap(time_at(p));
                    match state.move_keyframe(param, index, t) {
                        Ok(new_index) => {
                            ui.memory_mut(|mem| {
                                mem.data.insert_temp(drag_key_id, Some((param, new_index)));
                            });
                        }
                        Err(e) => state.report_error("Move key", &e),
                    }
                }
                None if p.x >= area.left() => {
                    state.rig.playhead = snap(time_at(p));
                    state.rig.playing = false;
                    state.rig.animate = true;
                }
                None => {}
            }
        }
        if response.drag_stopped() {
            ui.memory_mut(|mem| mem.data.remove::<Option<(ParameterId, usize)>>(drag_key_id));
        }
        if response.clicked() && p.x >= area.left() {
            match hovered_key.filter(|_| !in_ruler) {
                Some(key) => state.rig.selected_key = Some(key),
                None => {
                    state.rig.playhead = snap(time_at(p));
                    state.rig.playing = false;
                    state.rig.animate = true;
                }
            }
        }
        if response.double_clicked() && !in_ruler && hovered_key.is_none() {
            let row = ((p.y - rect.top() - RULER_HEIGHT) / ROW_HEIGHT).floor();
            if row >= 0.0 {
                if let Some(id) = rows.get(row as usize).copied() {
                    let t = snap(time_at(p));
                    let value = state.doc.rig.effective_value(id);
                    let r = state.edit_rig("Add key", None, |rig, _| {
                        if let Some(motion) = rig.motions.get_mut(m) {
                            motion.track_mut(id).set_key(t, value);
                        }
                        Ok(())
                    });
                    if let Err(e) = r {
                        state.report_error("Key", &e);
                    }
                }
            }
        }
    }

    let mut all = show_all;
    if ui.checkbox(&mut all, lang.tr("timeline.show_all")).changed() {
        ui.memory_mut(|mem| mem.data.insert_temp(egui::Id::new("timeline-all"), all));
    }
}

/// The selected track's value curve.
fn curve(ui: &mut Ui, state: &mut EditorState, m: usize) {
    let Some((param, selected)) = state.rig.selected_key else {
        return;
    };
    let motion = &state.doc.rig.motions[m];
    let (Some(track), Some(p)) = (motion.track(param), state.doc.rig.parameter(param)) else {
        return;
    };
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 70.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let area = Rect::from_min_max(
        Pos2::new(rect.left() + NAME_WIDTH, rect.top() + 6.0),
        rect.max - egui::vec2(0.0, 6.0),
    );
    let y_of = |v: f32| area.bottom() - (v - p.min) / p.span() * area.height();
    let duration = motion.duration.max(1e-3);
    let points: Vec<Pos2> = (0..=240)
        .map(|i| {
            let t = duration * i as f32 / 240.0;
            let v = track.sample(t).unwrap_or(p.default);
            Pos2::new(time_x(area, t, duration), y_of(v))
        })
        .collect();
    painter.line_segment(
        [
            Pos2::new(area.left(), y_of(p.default)),
            Pos2::new(area.right(), y_of(p.default)),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(25)),
    );
    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.5, Color32::from_rgb(120, 200, 255)),
    ));
    for (k, key) in track.keys.iter().enumerate() {
        let c = Pos2::new(time_x(area, key.time, duration), y_of(key.value));
        let color = if k == selected {
            Color32::from_rgb(255, 214, 90)
        } else {
            Color32::WHITE
        };
        painter.circle_filled(c, 3.0, color);
    }
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        &p.name,
        egui::FontId::proportional(11.0),
        ui.visuals().text_color(),
    );
}

/// Lip sync and export.
fn extras(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.separator();
    egui::CollapsingHeader::new(lang.tr("timeline.lip_sync"))
        .id_salt("timeline-lip")
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button(lang.tr("timeline.load_wav")).clicked() {
                    state.load_audio_via_dialog();
                }
                match &state.rig.audio {
                    Some((name, clip)) => ui.label(format!("{name} · {:.1} s", clip.duration())),
                    None => ui.label(RichText::new(lang.tr("timeline.no_audio")).weak()),
                };
                ui.add(egui::Slider::new(&mut state.rig.lip_gain, 0.2..=3.0).text(lang.tr("timeline.gain")));
                if ui
                    .button(lang.tr("timeline.bake"))
                    .on_hover_text(lang.tr("timeline.bake_hint"))
                    .clicked()
                {
                    if let Err(e) = state.bake_lip_sync() {
                        state.report_error("Lip sync", &e);
                    }
                }
            });
        });
    egui::CollapsingHeader::new(lang.tr("timeline.export"))
        .id_salt("timeline-export")
        .show(ui, |ui| {
            let export = &mut state.rig.export;
            ui.horizontal_wrapped(|ui| {
                ui.label(lang.tr("timeline.fps"));
                ui.add(egui::DragValue::new(&mut export.fps).range(1.0..=60.0));
                ui.add(egui::Slider::new(&mut export.scale, 0.1..=2.0).text(lang.tr("timeline.scale")));
                ui.checkbox(&mut export.simulate, lang.tr("timeline.export_physics"));
                ui.add(
                    egui::DragValue::new(&mut export.warmup)
                        .range(0.0..=10.0)
                        .speed(0.05)
                        .prefix(lang.tr("timeline.warmup"))
                        .suffix(" s"),
                );
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button(lang.tr("timeline.export_gif")).clicked() {
                    state.export_animation_via_dialog(ExportKind::Gif);
                }
                if ui
                    .button(lang.tr("timeline.export_apng"))
                    .on_hover_text(lang.tr("timeline.export_apng_hint"))
                    .clicked()
                {
                    state.export_animation_via_dialog(ExportKind::Apng);
                }
                if ui.button(lang.tr("timeline.export_png")).clicked() {
                    state.export_animation_via_dialog(ExportKind::PngSequence);
                }
                if ui.button(lang.tr("timeline.export_sheet")).clicked() {
                    state.export_animation_via_dialog(ExportKind::SpriteSheet);
                }
            });
        });
}
