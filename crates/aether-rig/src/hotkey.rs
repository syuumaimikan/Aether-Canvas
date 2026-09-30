//! Hotkeys: keys that play motions and switch expressions while a model
//! performs, as VTube Studio and other Live2D apps offer.
//!
//! A [`Hotkey`] binds a [`KeyChord`] (a key and its modifiers, written like
//! `Shift+1`) to a [`HotkeyAction`]. They are part of the rig, so they are
//! saved with the project and travel with exported runtime models; the
//! editor, the players and the engine plugins all read the same list and
//! call [`RigRuntime::trigger`](crate::runtime::RigRuntime::trigger).
//!
//! Actions name their motion or expression rather than index it, so
//! reordering the lists keeps the keys working, and renaming updates them
//! ([`rename_motion`], [`rename_expression`]).

use crate::rig::Rig;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A key with modifiers, such as `1`, `Shift+F2` or `Ctrl+Alt+Q`.
///
/// Keys are named as they are printed on the keyboard: letters `A`–`Z`,
/// digits `0`–`9`, `F1`–`F24`, and names for the rest (`Space`, `Enter`,
/// `Tab`, `Backspace`, `Insert`, `Delete`, `Home`, `End`, `PageUp`,
/// `PageDown`, the arrows `ArrowUp`…, and punctuation by its character:
/// `-`, `=`, `[`, `]`, `;`, `'`, `,`, `.`, `/`, `\`, `` ` ``).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyChord {
    /// The key, normalised (see [`KeyChord::normalise_key`]).
    pub key: String,
    /// Control (Command on macOS) held.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ctrl: bool,
    /// Shift held.
    #[serde(default, skip_serializing_if = "is_false")]
    pub shift: bool,
    /// Alt (Option on macOS) held.
    #[serde(default, skip_serializing_if = "is_false")]
    pub alt: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl KeyChord {
    /// A key without modifiers. The name is normalised; `None` for a name
    /// that is not a key.
    pub fn key(name: &str) -> Option<Self> {
        Some(Self {
            key: Self::normalise_key(name)?,
            ctrl: false,
            shift: false,
            alt: false,
        })
    }

    /// The same key with Shift.
    pub fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// The same key with Control.
    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    /// The same key with Alt.
    pub fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// Parse `Ctrl+Shift+1` and the like (any case, `+` or `-` between
    /// parts, `Cmd`/`Command`/`Control` for Ctrl, `Option` for Alt).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        // "Ctrl++" names the plus key.
        if text.ends_with("++") {
            parts.retain(|p| !p.is_empty());
            parts.push("+");
        }
        let key = parts.pop()?;
        let mut chord = Self::key(key)?;
        for modifier in parts {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" | "meta" => chord.ctrl = true,
                "shift" => chord.shift = true,
                "alt" | "option" | "opt" => chord.alt = true,
                _ => return None,
            }
        }
        Some(chord)
    }

    /// The canonical name of a key, accepting common spellings from
    /// keyboards and platforms: `a` → `A`, `Digit1`/`Num1`/`N1`/`Numpad1` →
    /// `1`, `f5` → `F5`, `space`/` ` → `Space`, `Esc` → `Escape`,
    /// `Left`/`LeftArrow` → `ArrowLeft`, `Minus` → `-`. `None` for modifier
    /// keys on their own and for anything unrecognised.
    pub fn normalise_key(name: &str) -> Option<String> {
        if name == " " {
            return Some("Space".into());
        }
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let mut chars = name.chars();
        let first = chars.next()?;
        if chars.next().is_none() {
            // One character: a letter, a digit or punctuation.
            if first.is_ascii_alphabetic() {
                return Some(first.to_ascii_uppercase().to_string());
            }
            if first.is_ascii_digit() {
                return Some(first.to_string());
            }
            return match first {
                '-' | '=' | '[' | ']' | ';' | '\'' | ',' | '.' | '/' | '\\' | '`' | '+' => {
                    Some(first.to_string())
                }
                _ => None,
            };
        }
        let lower = name.to_ascii_lowercase();
        // Digits spelled out by platforms: Digit1 (web), Num1 (egui), N1
        // (VTube Studio), Numpad1, Alpha1 (Unity), Key1.
        for prefix in ["digit", "numpad", "num", "alpha", "keypad", "key", "n"] {
            if let Some(rest) = lower.strip_prefix(prefix) {
                if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() {
                    return Some(rest.to_string());
                }
                if prefix == "key" && rest.len() == 1 && rest.as_bytes()[0].is_ascii_alphabetic() {
                    return Some(rest.to_ascii_uppercase());
                }
            }
        }
        if let Some(rest) = lower.strip_prefix('f') {
            if let Ok(n) = rest.parse::<u8>() {
                if (1..=24).contains(&n) {
                    return Some(format!("F{n}"));
                }
            }
        }
        let named = match lower.as_str() {
            "space" | "spacebar" => "Space",
            "enter" | "return" => "Enter",
            "tab" => "Tab",
            "backspace" => "Backspace",
            "escape" | "esc" => "Escape",
            "insert" | "ins" => "Insert",
            "delete" | "del" => "Delete",
            "home" => "Home",
            "end" => "End",
            "pageup" | "pgup" => "PageUp",
            "pagedown" | "pgdn" | "pgdown" => "PageDown",
            "arrowup" | "up" | "uparrow" => "ArrowUp",
            "arrowdown" | "down" | "downarrow" => "ArrowDown",
            "arrowleft" | "left" | "leftarrow" => "ArrowLeft",
            "arrowright" | "right" | "rightarrow" => "ArrowRight",
            "minus" => "-",
            "equal" | "equals" => "=",
            "plus" => "+",
            "bracketleft" | "openbracket" | "leftbracket" => "[",
            "bracketright" | "closebracket" | "rightbracket" => "]",
            "semicolon" => ";",
            "quote" | "apostrophe" => "'",
            "comma" => ",",
            "period" => ".",
            "slash" => "/",
            "backslash" => "\\",
            "backquote" | "backtick" | "grave" => "`",
            _ => return None,
        };
        Some(named.to_string())
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        f.write_str(&self.key)
    }
}

/// What a hotkey does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotkeyAction {
    /// Play the motion with this name. A motion that plays once plays over
    /// the idle loop and hands back to it; a looping motion becomes the idle
    /// loop (pressing its key again stops it).
    PlayMotion(String),
    /// Switch the expression with this name on or off. Several can be on
    /// at once.
    ToggleExpression(String),
    /// Switch every expression off.
    ClearExpressions,
    /// Stop every motion, the idle loop included.
    StopMotions,
    /// Stop motions and switch expressions off: back to the plain pose.
    Reset,
}

impl HotkeyAction {
    /// The motion or expression it names, if any.
    pub fn target(&self) -> Option<&str> {
        match self {
            Self::PlayMotion(name) | Self::ToggleExpression(name) => Some(name),
            _ => None,
        }
    }
}

/// A key bound to an action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hotkey {
    /// Shown in lists and logs; empty to show the action's target.
    #[serde(default)]
    pub name: String,
    /// The key.
    pub keys: KeyChord,
    /// What it does.
    pub action: HotkeyAction,
}

impl Hotkey {
    /// A hotkey named after its target.
    pub fn new(keys: KeyChord, action: HotkeyAction) -> Self {
        Self {
            name: String::new(),
            keys,
            action,
        }
    }

    /// The name to show: its own, or what it plays.
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match &self.action {
            HotkeyAction::PlayMotion(name) | HotkeyAction::ToggleExpression(name) => name.clone(),
            HotkeyAction::ClearExpressions => "Clear expressions".into(),
            HotkeyAction::StopMotions => "Stop motions".into(),
            HotkeyAction::Reset => "Reset".into(),
        }
    }

    /// True when its motion or expression exists (or it needs none).
    pub fn is_valid(&self, rig: &Rig) -> bool {
        match &self.action {
            HotkeyAction::PlayMotion(name) => rig.motions.iter().any(|m| &m.name == name),
            HotkeyAction::ToggleExpression(name) => rig.expressions.iter().any(|e| &e.name == name),
            _ => true,
        }
    }
}

/// The usual layout: motions on `1`–`9`, expressions on `Shift+1`–`Shift+9`,
/// and `0` back to the plain pose.
pub fn default_hotkeys(rig: &Rig) -> Vec<Hotkey> {
    let digit = |i: usize| KeyChord::key(&((i + 1) % 10).to_string()).expect("a digit is a key");
    let mut keys: Vec<Hotkey> = rig
        .motions
        .iter()
        .take(9)
        .enumerate()
        .map(|(i, m)| Hotkey::new(digit(i), HotkeyAction::PlayMotion(m.name.clone())))
        .collect();
    keys.extend(rig.expressions.iter().take(9).enumerate().map(|(i, e)| {
        Hotkey::new(
            digit(i).with_shift(),
            HotkeyAction::ToggleExpression(e.name.clone()),
        )
    }));
    if !keys.is_empty() {
        keys.push(Hotkey::new(digit(9), HotkeyAction::Reset));
    }
    keys
}

/// Point hotkeys that play motion `from` at `to` instead.
pub fn rename_motion(hotkeys: &mut [Hotkey], from: &str, to: &str) {
    for hotkey in hotkeys {
        if let HotkeyAction::PlayMotion(name) = &mut hotkey.action {
            if name == from {
                *name = to.to_string();
            }
        }
    }
}

/// Point hotkeys that toggle expression `from` at `to` instead.
pub fn rename_expression(hotkeys: &mut [Hotkey], from: &str, to: &str) {
    for hotkey in hotkeys {
        if let HotkeyAction::ToggleExpression(name) = &mut hotkey.action {
            if name == from {
                *name = to.to_string();
            }
        }
    }
}

/// Index of the first hotkey bound to `chord`.
pub fn find(hotkeys: &[Hotkey], chord: &KeyChord) -> Option<usize> {
    hotkeys.iter().position(|h| &h.keys == chord)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{Expression, Motion};

    #[test]
    fn keys_are_named_one_way_whatever_the_platform_calls_them() {
        for (name, expected) in [
            ("a", "A"),
            ("KeyQ", "Q"),
            ("Digit1", "1"),
            ("Num1", "1"),
            ("N1", "1"),
            ("Numpad7", "7"),
            ("Alpha3", "3"),
            ("f5", "F5"),
            ("F12", "F12"),
            (" ", "Space"),
            ("space", "Space"),
            ("Esc", "Escape"),
            ("LeftArrow", "ArrowLeft"),
            ("Minus", "-"),
            ("/", "/"),
        ] {
            assert_eq!(KeyChord::normalise_key(name).as_deref(), Some(expected), "{name}");
        }
        for bad in ["", "Shift", "Ctrl", "F25", "Banana", "é"] {
            assert_eq!(KeyChord::normalise_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn chords_parse_and_print_back() {
        let chord = KeyChord::parse("ctrl + shift + 1").expect("parses");
        assert!(chord.ctrl && chord.shift && !chord.alt);
        assert_eq!(chord.key, "1");
        assert_eq!(chord.to_string(), "Ctrl+Shift+1");
        assert_eq!(KeyChord::parse(&chord.to_string()), Some(chord));
        assert_eq!(
            KeyChord::parse("Alt+f2").map(|c| c.to_string()).as_deref(),
            Some("Alt+F2")
        );
        assert_eq!(
            KeyChord::parse("Ctrl++").map(|c| c.to_string()).as_deref(),
            Some("Ctrl++")
        );
        assert_eq!(KeyChord::parse("Hyper+1"), None);
        assert_eq!(KeyChord::parse("Shift+"), None);
    }

    #[test]
    fn defaults_put_motions_on_digits_and_expressions_on_shifted_digits() {
        let mut rig = Rig::new();
        for name in ["Idle", "Wave"] {
            rig.motions.push(Motion::new(name, 1.0, 30.0));
        }
        rig.expressions.push(Expression {
            name: "Smile".into(),
            entries: Vec::new(),
            fade: 0.25,
        });
        let keys = default_hotkeys(&rig);
        let shown: Vec<String> = keys.iter().map(|h| format!("{} {}", h.keys, h.label())).collect();
        assert_eq!(shown, ["1 Idle", "2 Wave", "Shift+1 Smile", "0 Reset"]);
        assert!(keys.iter().all(|h| h.is_valid(&rig)));
        assert!(default_hotkeys(&Rig::new()).is_empty());
    }

    #[test]
    fn renaming_follows_the_target() {
        let mut keys = vec![
            Hotkey::new(
                KeyChord::key("1").unwrap(),
                HotkeyAction::PlayMotion("Wave".into()),
            ),
            Hotkey::new(
                KeyChord::key("2").unwrap(),
                HotkeyAction::ToggleExpression("Wave".into()),
            ),
        ];
        rename_motion(&mut keys, "Wave", "Greeting");
        assert_eq!(keys[0].action, HotkeyAction::PlayMotion("Greeting".into()));
        assert_eq!(keys[1].action, HotkeyAction::ToggleExpression("Wave".into()));
        rename_expression(&mut keys, "Wave", "Happy");
        assert_eq!(keys[1].action, HotkeyAction::ToggleExpression("Happy".into()));
    }

    #[test]
    fn hotkeys_save_compactly() {
        let key = Hotkey::new(
            KeyChord::key("1").unwrap().with_shift(),
            HotkeyAction::ToggleExpression("Smile".into()),
        );
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(
            json,
            r#"{"name":"","keys":{"key":"1","shift":true},"action":{"ToggleExpression":"Smile"}}"#
        );
        assert_eq!(serde_json::from_str::<Hotkey>(&json).unwrap(), key);
    }
}
