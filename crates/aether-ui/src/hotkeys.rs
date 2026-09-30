//! Hotkeys in the editor.
//!
//! A model's hotkeys (see [`aether_document::rig::hotkey`]) play its motions
//! and switch its expressions. In the editor they fire in the live preview,
//! exactly as they will in the players: pressing one turns the preview on.
//! A hotkey on a key the editor also uses for a shortcut (`B` for the brush)
//! fires only while the preview runs, so painting keeps its keys.

use crate::shortcuts::Binding;
use crate::state::EditorState;
use aether_core::{AetherError, Result};
use aether_document::rig::hotkey::{self, Hotkey, HotkeyAction, KeyChord};

/// The chord an egui key press makes, or `None` for keys hotkeys cannot
/// use.
pub fn chord(key: egui::Key, modifiers: egui::Modifiers) -> Option<KeyChord> {
    let mut chord = KeyChord::key(key.name())?;
    chord.ctrl = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
    chord.shift = modifiers.shift;
    chord.alt = modifiers.alt;
    Some(chord)
}

/// The chords a key press can mean, best first. With Shift held the key
/// printed on the keycap is the physical one (`Shift+1`, where the layout
/// types "!"); otherwise the key as the layout types it.
pub fn chords(key: egui::Key, physical: Option<egui::Key>, modifiers: egui::Modifiers) -> Vec<KeyChord> {
    let mut keys = vec![key];
    if let Some(physical) = physical.filter(|p| *p != key) {
        if modifiers.shift {
            keys.insert(0, physical);
        } else {
            keys.push(physical);
        }
    }
    let mut out: Vec<KeyChord> = Vec::new();
    for k in keys {
        if let Some(c) = chord(k, modifiers) {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// The chord of an editor shortcut.
fn binding_chord(binding: Binding) -> Option<KeyChord> {
    chord(binding.key, binding.modifiers)
}

/// The kinds of action, in the order the panel offers them.
pub const ACTIONS: [&str; 5] = [
    "hotkey.play_motion",
    "hotkey.toggle_expression",
    "hotkey.clear_expressions",
    "hotkey.stop_motions",
    "hotkey.reset",
];

/// Which of [`ACTIONS`] an action is.
pub fn action_kind(action: &HotkeyAction) -> usize {
    match action {
        HotkeyAction::PlayMotion(_) => 0,
        HotkeyAction::ToggleExpression(_) => 1,
        HotkeyAction::ClearExpressions => 2,
        HotkeyAction::StopMotions => 3,
        HotkeyAction::Reset => 4,
    }
}

impl EditorState {
    /// Carry out hotkey `index` in the live preview (which it switches on).
    /// Returns false when its motion or expression is missing.
    pub fn trigger_hotkey(&mut self, index: usize) -> bool {
        let Some(hotkey) = self.doc.rig.hotkeys.get(index).cloned() else {
            return false;
        };
        // The runtime plays it; the timeline stops showing its playhead.
        self.rig.simulate = true;
        self.rig.animate = false;
        self.rig.playing = false;
        let fired = self.rig.runtime.trigger(&self.doc.rig, &hotkey.action);
        if fired {
            self.rig.last_hotkey = Some(index);
            self.status = format!("{} {}: {}", self.tr("hotkey.fired"), hotkey.keys, hotkey.label());
        } else {
            self.status = format!("{}: {}", self.tr("hotkey.missing"), hotkey.label());
        }
        fired
    }

    /// Keys were pressed: fire the hotkey bound to them. Keys that are also
    /// editor shortcuts fire only while the live preview runs. Returns the
    /// hotkey's index when one fired.
    pub fn press_hotkey(&mut self, keys: &KeyChord) -> Option<usize> {
        let index = hotkey::find(&self.doc.rig.hotkeys, keys)?;
        if !self.rig.simulate && self.is_editor_shortcut(keys) {
            return None;
        }
        self.trigger_hotkey(index).then_some(index)
    }

    /// True when `keys` also work an editor shortcut.
    pub fn is_editor_shortcut(&self, keys: &KeyChord) -> bool {
        self.shortcuts
            .all()
            .iter()
            .any(|(_, binding)| binding_chord(*binding).as_ref() == Some(keys))
    }

    /// Change the hotkey list as one undoable step.
    pub fn edit_hotkeys(
        &mut self,
        label: &str,
        coalesce: Option<String>,
        edit: impl FnOnce(&mut Vec<Hotkey>),
    ) -> Result<()> {
        self.edit_rig(label, coalesce, |rig, _| {
            edit(&mut rig.hotkeys);
            Ok(())
        })
    }

    /// Add a hotkey on the first free digit (then function key): playing
    /// the first motion that has none, else switching the first expression
    /// that has none (on a shifted digit).
    pub fn add_hotkey(&mut self) -> Result<usize> {
        let rig = &self.doc.rig;
        let used = |action: &HotkeyAction| rig.hotkeys.iter().any(|h| &h.action == action);
        let action = rig
            .motions
            .iter()
            .map(|m| HotkeyAction::PlayMotion(m.name.clone()))
            .find(|a| !used(a))
            .or_else(|| {
                rig.expressions
                    .iter()
                    .map(|e| HotkeyAction::ToggleExpression(e.name.clone()))
                    .find(|a| !used(a))
            })
            .unwrap_or(HotkeyAction::Reset);
        let taken = |c: &KeyChord| hotkey::find(&rig.hotkeys, c).is_some();
        let shifted = matches!(action, HotkeyAction::ToggleExpression(_));
        let keys = (1..=10)
            .map(|d| (d % 10).to_string())
            .chain((1..=12).map(|f| format!("F{f}")))
            .filter_map(|k| KeyChord::key(&k))
            .map(|c| if shifted { c.with_shift() } else { c })
            .find(|c| !taken(c))
            .ok_or_else(|| AetherError::rig("every digit and function key is taken"))?;
        let index = rig.hotkeys.len();
        self.edit_hotkeys("Add hotkey", None, |keys_list| {
            keys_list.push(Hotkey::new(keys, action));
        })?;
        Ok(index)
    }

    /// Replace the hotkeys with the usual layout: motions on 1–9,
    /// expressions on Shift+1–9, 0 to reset. Returns how many there are.
    pub fn assign_default_hotkeys(&mut self) -> Result<usize> {
        let keys = hotkey::default_hotkeys(&self.doc.rig);
        if keys.is_empty() {
            return Err(AetherError::rig("make a motion or an expression first"));
        }
        let count = keys.len();
        self.edit_hotkeys("Default hotkeys", None, |list| *list = keys)?;
        Ok(count)
    }

    /// Set hotkey `index`'s keys.
    pub fn set_hotkey_keys(&mut self, index: usize, keys: KeyChord) -> Result<()> {
        if index >= self.doc.rig.hotkeys.len() {
            return Err(AetherError::rig("no such hotkey"));
        }
        self.edit_hotkeys("Hotkey keys", None, |list| list[index].keys = keys)
    }

    /// Set hotkey `index`'s action.
    pub fn set_hotkey_action(&mut self, index: usize, action: HotkeyAction) -> Result<()> {
        if index >= self.doc.rig.hotkeys.len() {
            return Err(AetherError::rig("no such hotkey"));
        }
        self.edit_hotkeys("Hotkey action", None, |list| list[index].action = action)
    }

    /// Delete hotkey `index`.
    pub fn delete_hotkey(&mut self, index: usize) -> Result<()> {
        if index >= self.doc.rig.hotkeys.len() {
            return Err(AetherError::rig("no such hotkey"));
        }
        self.edit_hotkeys("Delete hotkey", None, |list| {
            list.remove(index);
        })?;
        self.rig.capture_hotkey = None;
        self.rig.last_hotkey = None;
        Ok(())
    }

    /// While a hotkey waits for its keys: take the first key pressed this
    /// frame (Escape cancels). Returns true while capturing, so the key
    /// works nothing else.
    pub fn capture_hotkey_keys(&mut self, ctx: &egui::Context) -> bool {
        let Some(index) = self.rig.capture_hotkey else {
            return false;
        };
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *physical_key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, physical, modifiers)) = pressed else {
            return true;
        };
        ctx.input_mut(|i| i.consume_key(modifiers, key));
        self.rig.capture_hotkey = None;
        if key == egui::Key::Escape && modifiers.is_none() {
            return true;
        }
        match chords(key, physical, modifiers).into_iter().next() {
            Some(keys) => {
                if let Err(e) = self.set_hotkey_keys(index, keys) {
                    self.report_error("Hotkey", &e);
                }
            }
            None => self.report(self.tr("hotkey.unusable").to_string()),
        }
        true
    }

    /// Fire hotkeys for the keys pressed this frame, consuming them. Keys
    /// typed into a text field are left alone.
    pub fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        if self.doc.rig.hotkeys.is_empty() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed: Vec<(egui::Key, Option<egui::Key>, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        physical_key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    } => Some((*key, *physical_key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for (key, physical, modifiers) in pressed {
            let fired = chords(key, physical, modifiers)
                .iter()
                .any(|keys| self.press_hotkey(keys).is_some());
            if fired {
                ctx.input_mut(|i| i.consume_key(modifiers, key));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_document::rig::{Expression, Motion};

    fn state() -> EditorState {
        let mut state = EditorState::default();
        state
            .edit_rig("Setup", None, |rig, ids| {
                rig.add_standard_parameters(ids);
                rig.motions.push(Motion::new("Wave", 1.0, 30.0));
                rig.expressions.push(Expression {
                    name: "Smile".into(),
                    entries: Vec::new(),
                    fade: 0.25,
                });
                Ok(())
            })
            .unwrap();
        state
    }

    #[test]
    fn egui_keys_become_chords() {
        let c = chord(egui::Key::Num1, egui::Modifiers::SHIFT).unwrap();
        assert_eq!(c.to_string(), "Shift+1");
        let c = chord(egui::Key::F5, egui::Modifiers::COMMAND).unwrap();
        assert_eq!(c.to_string(), "Ctrl+F5");
        assert_eq!(
            chord(egui::Key::OpenBracket, egui::Modifiers::NONE).unwrap().key,
            "["
        );
    }

    #[test]
    fn shifted_digits_are_the_digit_keys() {
        // A US layout types "!" for Shift+1; the keycap says 1.
        let shift = egui::Modifiers::SHIFT;
        let c = chords(egui::Key::Exclamationmark, Some(egui::Key::Num1), shift);
        assert_eq!(c.iter().map(|c| c.to_string()).collect::<Vec<_>>(), ["Shift+1"]);
        // Without Shift the layout's key comes first (AZERTY's A is QWERTY's Q).
        let c = chords(egui::Key::A, Some(egui::Key::Q), egui::Modifiers::NONE);
        assert_eq!(c.iter().map(|c| c.to_string()).collect::<Vec<_>>(), ["A", "Q"]);
    }

    #[test]
    fn hotkeys_fire_in_the_live_preview() {
        let mut state = state();
        assert_eq!(state.assign_default_hotkeys().unwrap(), 3);
        state.rig.animate = true;
        let one = KeyChord::key("1").unwrap();
        assert_eq!(state.press_hotkey(&one), Some(0));
        assert!(
            state.rig.simulate && !state.rig.animate,
            "the preview runs the motion"
        );
        assert!(state.rig.runtime.animator.is_playing(0));
        assert_eq!(
            state.press_hotkey(&KeyChord::key("1").unwrap().with_shift()),
            Some(1)
        );
        assert_eq!(state.rig.runtime.active_expressions(), vec![0]);
        assert!(state.status.contains("Shift+1"), "{}", state.status);
        assert_eq!(state.press_hotkey(&KeyChord::key("5").unwrap()), None);
    }

    #[test]
    fn editor_shortcut_keys_wait_for_the_preview() {
        let mut state = state();
        state
            .edit_hotkeys("Setup", None, |list| {
                list.push(Hotkey::new(
                    KeyChord::key("B").unwrap(),
                    HotkeyAction::PlayMotion("Wave".into()),
                ))
            })
            .unwrap();
        let b = KeyChord::key("B").unwrap();
        assert!(state.is_editor_shortcut(&b), "B picks the brush");
        assert_eq!(state.press_hotkey(&b), None, "painting keeps its keys");
        state.rig.simulate = true;
        assert_eq!(state.press_hotkey(&b), Some(0), "the preview takes them");
    }

    #[test]
    fn hotkeys_are_edited_with_undo() {
        let mut state = state();
        let first = state.add_hotkey().unwrap();
        assert_eq!(state.doc.rig.hotkeys[first].keys.to_string(), "1");
        let second = state.add_hotkey().unwrap();
        let added = &state.doc.rig.hotkeys[second];
        assert_eq!(
            (added.keys.to_string(), added.label()),
            ("Shift+1".to_string(), "Smile".to_string()),
            "the next one switches the expression, on a shifted digit"
        );
        state
            .set_hotkey_keys(first, KeyChord::parse("F2").unwrap())
            .unwrap();
        state.set_hotkey_action(first, HotkeyAction::Reset).unwrap();
        assert_eq!(state.doc.rig.hotkeys[first].label(), "Reset");
        state.delete_hotkey(second).unwrap();
        assert_eq!(state.doc.rig.hotkeys.len(), 1);
        state.history.undo(&mut state.doc).expect("undo");
        assert_eq!(state.doc.rig.hotkeys.len(), 2, "undo brings it back");
    }
}
