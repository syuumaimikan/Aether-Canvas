//! Keyboard shortcuts.
//!
//! Every command the menus expose is an [`Action`], and the binding table maps
//! keys to actions rather than the other way round. That makes rebinding a data
//! change, lets the UI show the current key next to each menu item, and makes
//! conflict detection a lookup instead of a scan through widget code.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};

/// Something the user can trigger from a menu or the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Action {
    /// Create a new document.
    NewDocument,
    /// Open a project.
    OpenDocument,
    /// Save the current project.
    Save,
    /// Save under a new name.
    SaveAs,
    /// Export a flattened PNG.
    ExportPng,
    /// Undo one step.
    Undo,
    /// Redo one step.
    Redo,
    /// Erase the active layer's pixels.
    ClearLayer,
    /// Select the whole canvas.
    SelectAll,
    /// Drop the selection.
    Deselect,
    /// Invert the selection.
    InvertSelection,
    /// Add a raster layer.
    AddLayer,
    /// Delete the active layer.
    DeleteLayer,
    /// Duplicate the active layer.
    DuplicateLayer,
    /// Merge the active layer into the one below.
    MergeDown,
    /// Zoom in one step.
    ZoomIn,
    /// Zoom out one step.
    ZoomOut,
    /// Fit the document in the window.
    ZoomFit,
    /// Reset to 100%.
    ZoomReset,
    /// Rotate the view anticlockwise.
    RotateLeft,
    /// Rotate the view clockwise.
    RotateRight,
    /// Reset view rotation.
    ResetRotation,
    /// Mirror the view.
    MirrorView,
    /// Toggle the pixel grid.
    TogglePixelGrid,
    /// Select the brush tool.
    ToolBrush,
    /// Select the eraser.
    ToolEraser,
    /// Select the bucket fill tool.
    ToolBucket,
    /// Select the eyedropper.
    ToolEyedropper,
    /// Select the rectangular marquee.
    ToolRectSelect,
    /// Select the move tool.
    ToolMove,
    /// Select the transform tool.
    ToolTransform,
    /// Select the liquify tool.
    ToolLiquify,
    /// Select the pan tool.
    ToolPan,
    /// Make the brush larger.
    BrushLarger,
    /// Make the brush smaller.
    BrushSmaller,
    /// Swap primary and secondary colours.
    SwapColors,
}

impl Action {
    /// Every action, in the order the shortcut editor lists them.
    pub const ALL: [Action; 36] = [
        Action::NewDocument,
        Action::OpenDocument,
        Action::Save,
        Action::SaveAs,
        Action::ExportPng,
        Action::Undo,
        Action::Redo,
        Action::ClearLayer,
        Action::SelectAll,
        Action::Deselect,
        Action::InvertSelection,
        Action::AddLayer,
        Action::DeleteLayer,
        Action::DuplicateLayer,
        Action::MergeDown,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomFit,
        Action::ZoomReset,
        Action::RotateLeft,
        Action::RotateRight,
        Action::ResetRotation,
        Action::MirrorView,
        Action::TogglePixelGrid,
        Action::ToolBrush,
        Action::ToolEraser,
        Action::ToolBucket,
        Action::ToolEyedropper,
        Action::ToolRectSelect,
        Action::ToolMove,
        Action::ToolTransform,
        Action::ToolLiquify,
        Action::ToolPan,
        Action::BrushLarger,
        Action::BrushSmaller,
        Action::SwapColors,
    ];

    /// Human-readable label (English; menus use the i18n table instead).
    pub fn label(self) -> &'static str {
        match self {
            Action::NewDocument => "New Document",
            Action::OpenDocument => "Open",
            Action::Save => "Save",
            Action::SaveAs => "Save As",
            Action::ExportPng => "Export PNG",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::ClearLayer => "Clear Layer",
            Action::SelectAll => "Select All",
            Action::Deselect => "Deselect",
            Action::InvertSelection => "Invert Selection",
            Action::AddLayer => "Add Layer",
            Action::DeleteLayer => "Delete Layer",
            Action::DuplicateLayer => "Duplicate Layer",
            Action::MergeDown => "Merge Down",
            Action::ZoomIn => "Zoom In",
            Action::ZoomOut => "Zoom Out",
            Action::ZoomFit => "Fit to Window",
            Action::ZoomReset => "Actual Size",
            Action::RotateLeft => "Rotate View Left",
            Action::RotateRight => "Rotate View Right",
            Action::ResetRotation => "Reset Rotation",
            Action::MirrorView => "Mirror Canvas",
            Action::TogglePixelGrid => "Toggle Pixel Grid",
            Action::ToolBrush => "Brush Tool",
            Action::ToolEraser => "Eraser Tool",
            Action::ToolBucket => "Bucket Tool",
            Action::ToolEyedropper => "Eyedropper Tool",
            Action::ToolRectSelect => "Rectangle Select",
            Action::ToolMove => "Move Tool",
            Action::ToolTransform => "Transform Tool",
            Action::ToolLiquify => "Liquify Tool",
            Action::ToolPan => "Pan Tool",
            Action::BrushLarger => "Increase Brush Size",
            Action::BrushSmaller => "Decrease Brush Size",
            Action::SwapColors => "Swap Colors",
        }
    }
}

/// One key combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    /// The key itself.
    pub key: Key,
    /// Required modifiers.
    pub modifiers: Modifiers,
}

impl Binding {
    /// A plain key with no modifiers.
    pub fn key(key: Key) -> Self {
        Self {
            key,
            modifiers: Modifiers::NONE,
        }
    }

    /// Ctrl (Cmd on macOS) plus a key.
    pub fn ctrl(key: Key) -> Self {
        Self {
            key,
            modifiers: Modifiers::COMMAND,
        }
    }

    /// Ctrl+Shift plus a key.
    pub fn ctrl_shift(key: Key) -> Self {
        Self {
            key,
            modifiers: Modifiers::COMMAND.plus(Modifiers::SHIFT),
        }
    }

    /// Shift plus a key.
    pub fn shift(key: Key) -> Self {
        Self {
            key,
            modifiers: Modifiers::SHIFT,
        }
    }

    /// As an egui shortcut, for `consume_shortcut` and for display.
    pub fn to_egui(self) -> KeyboardShortcut {
        KeyboardShortcut::new(self.modifiers, self.key)
    }

    /// Display form, e.g. `Ctrl+Shift+S`.
    pub fn display(self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.command || self.modifiers.ctrl {
            parts.push("Ctrl");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        let name = self.key.name();
        parts.push(name);
        parts.join("+")
    }
}

/// The user's key map.
#[derive(Clone, Debug)]
pub struct ShortcutMap {
    bindings: Vec<(Action, Binding)>,
}

impl Default for ShortcutMap {
    fn default() -> Self {
        Self::standard()
    }
}

impl ShortcutMap {
    /// The out-of-the-box bindings.
    ///
    /// They follow the conventions shared by every major raster editor, so
    /// muscle memory transfers.
    pub fn standard() -> Self {
        use Action::*;
        let bindings = vec![
            (NewDocument, Binding::ctrl(Key::N)),
            (OpenDocument, Binding::ctrl(Key::O)),
            (Save, Binding::ctrl(Key::S)),
            (SaveAs, Binding::ctrl_shift(Key::S)),
            (ExportPng, Binding::ctrl(Key::E)),
            (Undo, Binding::ctrl(Key::Z)),
            (Redo, Binding::ctrl_shift(Key::Z)),
            (ClearLayer, Binding::key(Key::Delete)),
            (SelectAll, Binding::ctrl(Key::A)),
            (Deselect, Binding::ctrl(Key::D)),
            (InvertSelection, Binding::ctrl_shift(Key::I)),
            (AddLayer, Binding::ctrl_shift(Key::N)),
            (DeleteLayer, Binding::ctrl_shift(Key::Delete)),
            (DuplicateLayer, Binding::ctrl(Key::J)),
            (MergeDown, Binding::ctrl(Key::M)),
            (ZoomIn, Binding::ctrl(Key::Plus)),
            (ZoomOut, Binding::ctrl(Key::Minus)),
            (ZoomFit, Binding::ctrl(Key::Num0)),
            (ZoomReset, Binding::ctrl(Key::Num1)),
            (RotateLeft, Binding::shift(Key::Comma)),
            (RotateRight, Binding::shift(Key::Period)),
            (ResetRotation, Binding::shift(Key::Slash)),
            (MirrorView, Binding::shift(Key::M)),
            (TogglePixelGrid, Binding::ctrl(Key::G)),
            (ToolBrush, Binding::key(Key::B)),
            (ToolEraser, Binding::key(Key::E)),
            (ToolBucket, Binding::key(Key::G)),
            (ToolEyedropper, Binding::key(Key::I)),
            (ToolRectSelect, Binding::key(Key::M)),
            (ToolMove, Binding::key(Key::V)),
            (ToolTransform, Binding::ctrl(Key::T)),
            (ToolLiquify, Binding::shift(Key::L)),
            (ToolPan, Binding::key(Key::H)),
            (BrushLarger, Binding::key(Key::CloseBracket)),
            (BrushSmaller, Binding::key(Key::OpenBracket)),
            (SwapColors, Binding::key(Key::X)),
        ];
        Self { bindings }
    }

    /// The binding for `action`, if any.
    pub fn binding(&self, action: Action) -> Option<Binding> {
        self.bindings.iter().find(|(a, _)| *a == action).map(|(_, b)| *b)
    }

    /// Display string for `action`, or an empty string when unbound.
    pub fn display(&self, action: Action) -> String {
        self.binding(action).map(|b| b.display()).unwrap_or_default()
    }

    /// All bindings, in action order.
    pub fn all(&self) -> &[(Action, Binding)] {
        &self.bindings
    }

    /// Actions already using `binding`, ignoring `except`.
    pub fn conflicts(&self, binding: Binding, except: Action) -> Vec<Action> {
        self.bindings
            .iter()
            .filter(|(a, b)| *a != except && *b == binding)
            .map(|(a, _)| *a)
            .collect()
    }

    /// Rebind `action`.
    ///
    /// Any other action holding the same combination loses it, because two
    /// actions on one key would make the first one silently win.
    pub fn rebind(&mut self, action: Action, binding: Binding) {
        self.bindings.retain(|(a, b)| *a == action || *b != binding);
        match self.bindings.iter_mut().find(|(a, _)| *a == action) {
            Some(slot) => slot.1 = binding,
            None => self.bindings.push((action, binding)),
        }
    }

    /// Remove the binding for `action`.
    pub fn unbind(&mut self, action: Action) {
        self.bindings.retain(|(a, _)| *a != action);
    }

    /// Restore the defaults.
    pub fn reset(&mut self) {
        *self = Self::standard();
    }

    /// Consume any pressed shortcut and return its action.
    ///
    /// Consuming matters: without it a shortcut would also reach the widget
    /// under the cursor, so `E` would both pick the eraser and type an "e" into
    /// a focused text field.
    pub fn consume(&self, ctx: &egui::Context) -> Option<Action> {
        // Text entry takes priority over single-key tool shortcuts.
        let typing = ctx.egui_wants_keyboard_input();
        for (action, binding) in &self.bindings {
            if typing && binding.modifiers.is_none() {
                continue;
            }
            let shortcut = binding.to_egui();
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                return Some(*action);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_bind_the_common_actions() {
        let map = ShortcutMap::standard();
        assert_eq!(map.binding(Action::Save), Some(Binding::ctrl(Key::S)));
        assert_eq!(map.binding(Action::Undo), Some(Binding::ctrl(Key::Z)));
        assert_eq!(map.binding(Action::ToolBrush), Some(Binding::key(Key::B)));
    }

    #[test]
    fn every_action_has_a_default_binding() {
        let map = ShortcutMap::standard();
        let missing: Vec<Action> = Action::ALL
            .into_iter()
            .filter(|a| map.binding(*a).is_none())
            .collect();
        assert!(missing.is_empty(), "unbound actions: {missing:?}");
    }

    #[test]
    fn the_defaults_have_no_duplicate_bindings() {
        let map = ShortcutMap::standard();
        for (action, binding) in map.all() {
            let clash = map.conflicts(*binding, *action);
            assert!(
                clash.is_empty(),
                "{action:?} clashes with {clash:?} on {}",
                binding.display()
            );
        }
    }

    #[test]
    fn rebinding_takes_the_key_from_its_previous_owner() {
        let mut map = ShortcutMap::standard();
        map.rebind(Action::ToolBucket, Binding::key(Key::B));
        assert_eq!(map.binding(Action::ToolBucket), Some(Binding::key(Key::B)));
        assert_eq!(
            map.binding(Action::ToolBrush),
            None,
            "the old owner must lose the key"
        );
    }

    #[test]
    fn conflicts_are_reported_before_rebinding() {
        let map = ShortcutMap::standard();
        let clash = map.conflicts(Binding::ctrl(Key::S), Action::ExportPng);
        assert_eq!(clash, vec![Action::Save]);
    }

    #[test]
    fn reset_restores_the_defaults() {
        let mut map = ShortcutMap::standard();
        map.unbind(Action::Save);
        map.reset();
        assert!(map.binding(Action::Save).is_some());
    }

    #[test]
    fn display_includes_modifiers() {
        assert_eq!(Binding::ctrl_shift(Key::S).display(), "Ctrl+Shift+S");
        assert_eq!(Binding::key(Key::B).display(), "B");
    }
}
