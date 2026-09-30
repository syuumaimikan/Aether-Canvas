//! The hotkeys panel: keys that play motions and switch expressions, as in
//! VTube Studio. They are saved with the model, fire in the live preview
//! here, and work the same in every player.

use crate::hotkeys::{action_kind, ACTIONS};
use crate::icons;
use crate::state::EditorState;
use aether_document::rig::hotkey::{self, HotkeyAction};
use egui::{Color32, RichText, Ui};

const WARN: Color32 = Color32::from_rgb(240, 130, 100);

/// The hotkeys panel.
pub fn hotkeys_panel(ui: &mut Ui, state: &mut EditorState) {
    let lang = state.language;
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut state.rig.simulate, lang.tr("hotkey.live"))
            .on_hover_text(lang.tr("hotkey.live_hint"));
        if ui
            .button(format!("{} {}", icons::ADD, lang.tr("hotkey.add")))
            .clicked()
        {
            if let Err(e) = state.add_hotkey() {
                state.report_error("Hotkey", &e);
            }
        }
        if ui
            .button(lang.tr("hotkey.defaults"))
            .on_hover_text(lang.tr("hotkey.defaults_hint"))
            .clicked()
        {
            match state.assign_default_hotkeys() {
                Ok(n) => state.report(format!("{} {n}", lang.tr("hotkey.assigned"))),
                Err(e) => state.report_error("Hotkey", &e),
            }
        }
    });
    ui.label(RichText::new(lang.tr("hotkey.hint")).weak().small());
    ui.separator();

    let rig = state.doc.rig.clone();
    if rig.hotkeys.is_empty() {
        ui.label(RichText::new(lang.tr("hotkey.none")).weak());
        return;
    }
    let motions: Vec<String> = rig.motions.iter().map(|m| m.name.clone()).collect();
    let expressions: Vec<String> = rig.expressions.iter().map(|e| e.name.clone()).collect();
    let active = state.rig.runtime.active_expressions();

    let mut change: Option<(usize, HotkeyAction)> = None;
    let mut remove = None;
    let mut fire = None;
    // Everything a key can do, as one list: each motion, each expression,
    // then the rest.
    let choices: Vec<HotkeyAction> = motions
        .iter()
        .map(|m| HotkeyAction::PlayMotion(m.clone()))
        .chain(
            expressions
                .iter()
                .map(|e| HotkeyAction::ToggleExpression(e.clone())),
        )
        .chain([
            HotkeyAction::ClearExpressions,
            HotkeyAction::StopMotions,
            HotkeyAction::Reset,
        ])
        .collect();
    let describe = |action: &HotkeyAction| -> String {
        match action {
            HotkeyAction::PlayMotion(name) => format!("{} {name}", icons::PLAY),
            HotkeyAction::ToggleExpression(name) => format!("{} {name}", icons::MASK),
            other => lang.tr(ACTIONS[action_kind(other)]).to_string(),
        }
    };
    egui::ScrollArea::vertical().id_salt("hotkeys").show(ui, |ui| {
        let width = (ui.available_width() - 150.0).clamp(90.0, 260.0);
        egui::Grid::new("hotkey-grid")
            .num_columns(4)
            .striped(true)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for (i, key) in rig.hotkeys.iter().enumerate() {
                    // The keys: click, then press the new ones.
                    let capturing = state.rig.capture_hotkey == Some(i);
                    let text = if capturing {
                        RichText::new(lang.tr("hotkey.press")).italics()
                    } else {
                        RichText::new(key.keys.to_string()).monospace().strong()
                    };
                    if ui
                        .add(
                            egui::Button::new(text)
                                .selected(capturing)
                                .min_size(egui::vec2(64.0, 0.0)),
                        )
                        .on_hover_text(lang.tr("hotkey.change_keys"))
                        .clicked()
                    {
                        state.rig.capture_hotkey = if capturing { None } else { Some(i) };
                    }

                    // What it does.
                    let shown = if key.is_valid(&rig) {
                        RichText::new(describe(&key.action))
                    } else {
                        RichText::new(format!("{} {}", icons::WARNING, describe(&key.action))).color(WARN)
                    };
                    let mut chosen = key.action.clone();
                    egui::ComboBox::from_id_salt(("hotkey-action", i))
                        .width(width)
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            let mut last_kind = None;
                            for choice in &choices {
                                let kind = action_kind(choice);
                                if last_kind.is_some_and(|k| k != kind && kind <= 2) {
                                    ui.separator();
                                }
                                last_kind = Some(kind);
                                ui.selectable_value(&mut chosen, choice.clone(), describe(choice))
                                    .on_hover_text(lang.tr(ACTIONS[kind]));
                            }
                        })
                        .response
                        .on_hover_text(lang.tr(ACTIONS[action_kind(&key.action)]));
                    if chosen != key.action {
                        change = Some((i, chosen));
                    }

                    // Try it; lit while its expression shows, or when it
                    // fired last.
                    let on = match &key.action {
                        HotkeyAction::ToggleExpression(name) => rig
                            .expressions
                            .iter()
                            .position(|e| &e.name == name)
                            .is_some_and(|x| active.contains(&x)),
                        _ => state.rig.last_hotkey == Some(i) && state.rig.simulate,
                    };
                    if ui
                        .add(egui::Button::new(icons::PLAY).selected(on))
                        .on_hover_text(lang.tr("hotkey.try"))
                        .clicked()
                    {
                        fire = Some(i);
                    }
                    if ui.small_button(icons::DELETE).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });

        // Problems worth knowing before going live.
        for (i, key) in rig.hotkeys.iter().enumerate() {
            if hotkey::find(&rig.hotkeys, &key.keys) != Some(i) {
                ui.colored_label(
                    WARN,
                    format!("{} {} {}", icons::WARNING, key.keys, lang.tr("hotkey.duplicate")),
                );
            } else if state.is_editor_shortcut(&key.keys) {
                ui.label(
                    RichText::new(format!("{} {}", key.keys, lang.tr("hotkey.shortcut_clash")))
                        .weak()
                        .small(),
                );
            }
        }
    });

    if let Some((i, action)) = change {
        if let Err(e) = state.set_hotkey_action(i, action) {
            state.report_error("Hotkey", &e);
        }
    }
    if let Some(i) = fire {
        state.trigger_hotkey(i);
    }
    if let Some(i) = remove {
        if let Err(e) = state.delete_hotkey(i) {
            state.report_error("Hotkey", &e);
        }
    }
}
