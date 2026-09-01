//! Undo / redo history.
//!
//! The history is two stacks of [`Command`] objects. Executing a command pushes
//! it onto the undo stack and clears the redo stack; undoing moves it across,
//! redoing moves it back.
//!
//! Two details make this behave the way artists expect rather than the way a
//! naive stack does:
//!
//! * **Coalescing** — dragging a slider emits a command per frame. Commands
//!   that report the same [`Command::coalesce_key`] and arrive back to back are
//!   folded into one history entry.
//! * **Live edits** — a brush stroke has already been drawn to the screen by
//!   the time it ends, so [`History::push_applied`] records it without
//!   re-running it.

use crate::command::Command;
use crate::document::Document;
use aether_core::Result;

/// One entry as shown in the history panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The command's label.
    pub name: String,
    /// True for the state the document is currently at.
    pub current: bool,
}

/// Undo/redo stacks with a bounded depth.
#[derive(Debug)]
pub struct History {
    undo_stack: Vec<Box<dyn Command>>,
    redo_stack: Vec<Box<dyn Command>>,
    limit: usize,
    /// Set while an undo or redo is running, so nothing re-enters.
    running: bool,
    /// Incremented on every change; lets the UI cheaply detect "unsaved work".
    revision: u64,
    saved_revision: u64,
}

impl Default for History {
    fn default() -> Self {
        Self::new(200)
    }
}

impl History {
    /// A history keeping at most `limit` undo steps.
    pub fn new(limit: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            limit: limit.max(1),
            running: false,
            revision: 0,
            saved_revision: 0,
        }
    }

    /// Run `command` and record it.
    pub fn execute(&mut self, doc: &mut Document, mut command: Box<dyn Command>) -> Result<()> {
        command.apply(doc)?;
        self.record(command);
        Ok(())
    }

    /// Record a command whose effect is already visible in the document.
    ///
    /// Used by interactive tools that paint live and only build the command
    /// when the gesture ends.
    pub fn push_applied(&mut self, command: Box<dyn Command>) {
        self.record(command);
    }

    fn record(&mut self, command: Box<dyn Command>) {
        if self.running {
            return;
        }
        self.redo_stack.clear();
        // Fold into the previous entry when both opt in with the same key.
        if let (Some(key), Some(last)) = (command.coalesce_key(), self.undo_stack.last_mut()) {
            if last.coalesce_key().as_deref() == Some(key.as_str()) && last.absorb(command.as_ref()) {
                self.revision += 1;
                return;
            }
        }
        self.undo_stack.push(command);
        if self.undo_stack.len() > self.limit {
            self.undo_stack.remove(0);
        }
        self.revision += 1;
    }

    /// True when there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// True when there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Label of the next undo step.
    pub fn undo_name(&self) -> Option<String> {
        self.undo_stack.last().map(|c| c.name())
    }

    /// Label of the next redo step.
    pub fn redo_name(&self) -> Option<String> {
        self.redo_stack.last().map(|c| c.name())
    }

    /// Undo one step.
    ///
    /// A command that fails to undo is dropped rather than retried: leaving a
    /// half-reversed command on the stack would corrupt every later step.
    pub fn undo(&mut self, doc: &mut Document) -> Result<bool> {
        let Some(mut command) = self.undo_stack.pop() else {
            return Ok(false);
        };
        self.running = true;
        let result = command.undo(doc);
        self.running = false;
        result?;
        self.redo_stack.push(command);
        self.revision += 1;
        Ok(true)
    }

    /// Redo one step.
    pub fn redo(&mut self, doc: &mut Document) -> Result<bool> {
        let Some(mut command) = self.redo_stack.pop() else {
            return Ok(false);
        };
        self.running = true;
        let result = command.apply(doc);
        self.running = false;
        result?;
        self.undo_stack.push(command);
        self.revision += 1;
        Ok(true)
    }

    /// Undo or redo until the document is at history position `index`.
    ///
    /// `index` counts applied commands: `0` is the state before anything was
    /// done, `len()` is the newest state.
    pub fn jump_to(&mut self, doc: &mut Document, index: usize) -> Result<()> {
        let target = index.min(self.undo_stack.len() + self.redo_stack.len());
        while self.undo_stack.len() > target {
            if !self.undo(doc)? {
                break;
            }
        }
        while self.undo_stack.len() < target {
            if !self.redo(doc)? {
                break;
            }
        }
        Ok(())
    }

    /// Entries for the history panel, oldest first.
    pub fn entries(&self) -> Vec<HistoryEntry> {
        let mut out: Vec<HistoryEntry> = self
            .undo_stack
            .iter()
            .map(|c| HistoryEntry {
                name: c.name(),
                current: false,
            })
            .collect();
        if let Some(last) = out.last_mut() {
            last.current = true;
        }
        // Redo entries are stored newest-first; show them in chronological order.
        out.extend(self.redo_stack.iter().rev().map(|c| HistoryEntry {
            name: c.name(),
            current: false,
        }));
        out
    }

    /// How many steps can be undone.
    pub fn depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.revision += 1;
        self.saved_revision = self.revision;
    }

    /// A counter that changes on every edit.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Record that the document has just been saved.
    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    /// True when there are edits since the last save.
    pub fn has_unsaved_changes(&self) -> bool {
        self.revision != self.saved_revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{AddLayerCommand, LayerProperty, RegionEdit, SetLayerPropertyCommand};
    use crate::layer::Layer;
    use aether_core::color::Rgba8;
    use aether_core::math::IRect;

    fn doc() -> Document {
        Document::new(16, 16, "test")
    }

    fn paint(d: &mut Document, h: &mut History, color: Rgba8) {
        let layer = d.active_layer;
        let cmd = RegionEdit::capture(d, layer, IRect::new(0, 0, 4, 4), "Paint", |pm| {
            pm.fill_rect(IRect::new(0, 0, 4, 4), color);
            Ok(())
        })
        .expect("capture");
        h.push_applied(Box::new(cmd));
    }

    #[test]
    fn empty_history_has_nothing_to_do() {
        let mut d = doc();
        let mut h = History::default();
        assert!(!h.can_undo() && !h.can_redo());
        assert!(!h.undo(&mut d).expect("undo"));
        assert!(!h.redo(&mut d).expect("redo"));
    }

    #[test]
    fn undo_then_redo_restores_pixels() {
        let mut d = doc();
        let mut h = History::default();
        let layer = d.active_layer;
        paint(&mut d, &mut h, Rgba8::WHITE);

        assert!(h.can_undo());
        h.undo(&mut d).expect("undo");
        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(1, 1)),
            Some(Rgba8::TRANSPARENT)
        );
        assert!(h.can_redo());
        h.redo(&mut d).expect("redo");
        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(1, 1)),
            Some(Rgba8::WHITE)
        );
    }

    #[test]
    fn a_new_edit_clears_the_redo_stack() {
        let mut d = doc();
        let mut h = History::default();
        paint(&mut d, &mut h, Rgba8::WHITE);
        h.undo(&mut d).expect("undo");
        assert!(h.can_redo());
        paint(&mut d, &mut h, Rgba8::BLACK);
        assert!(!h.can_redo(), "a new edit invalidates the redo branch");
    }

    #[test]
    fn many_steps_undo_in_order() {
        let mut d = doc();
        let mut h = History::default();
        let layer = d.active_layer;
        for i in 1..=5u8 {
            paint(&mut d, &mut h, Rgba8::new(i * 10, 0, 0, 255));
        }
        assert_eq!(h.depth(), 5);
        for _ in 0..5 {
            h.undo(&mut d).expect("undo");
        }
        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(0, 0)),
            Some(Rgba8::TRANSPARENT)
        );
        for _ in 0..5 {
            h.redo(&mut d).expect("redo");
        }
        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(0, 0)),
            Some(Rgba8::new(50, 0, 0, 255))
        );
    }

    #[test]
    fn the_limit_drops_the_oldest_entries() {
        let mut d = doc();
        let mut h = History::new(3);
        for i in 1..=6u8 {
            paint(&mut d, &mut h, Rgba8::new(i, 0, 0, 255));
        }
        assert_eq!(h.depth(), 3);
    }

    #[test]
    fn slider_drags_collapse_into_one_entry() {
        let mut d = doc();
        let mut h = History::default();
        let id = d.active_layer;
        for step in [0.9f32, 0.8, 0.7, 0.6] {
            h.execute(
                &mut d,
                Box::new(SetLayerPropertyCommand::new(id, LayerProperty::Opacity(step))),
            )
            .expect("execute");
        }
        assert_eq!(h.depth(), 1, "a slider drag should be one undo step");
        h.undo(&mut d).expect("undo");
        assert_eq!(d.layers.get(id).map(|l| l.opacity), Some(1.0));
    }

    #[test]
    fn unrelated_commands_between_drags_break_coalescing() {
        let mut d = doc();
        let mut h = History::default();
        let id = d.active_layer;
        h.execute(
            &mut d,
            Box::new(SetLayerPropertyCommand::new(id, LayerProperty::Opacity(0.8))),
        )
        .expect("execute");
        h.execute(
            &mut d,
            Box::new(SetLayerPropertyCommand::new(id, LayerProperty::Visible(false))),
        )
        .expect("execute");
        h.execute(
            &mut d,
            Box::new(SetLayerPropertyCommand::new(id, LayerProperty::Opacity(0.5))),
        )
        .expect("execute");
        assert_eq!(h.depth(), 3);
    }

    #[test]
    fn jump_to_moves_in_both_directions() {
        let mut d = doc();
        let mut h = History::default();
        let layer = d.active_layer;
        paint(&mut d, &mut h, Rgba8::new(10, 0, 0, 255));
        paint(&mut d, &mut h, Rgba8::new(20, 0, 0, 255));
        paint(&mut d, &mut h, Rgba8::new(30, 0, 0, 255));

        h.jump_to(&mut d, 1).expect("jump back");
        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(0, 0)),
            Some(Rgba8::new(10, 0, 0, 255))
        );
        h.jump_to(&mut d, 3).expect("jump forward");
        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(0, 0)),
            Some(Rgba8::new(30, 0, 0, 255))
        );
    }

    #[test]
    fn structural_and_pixel_edits_interleave_correctly() {
        let mut d = doc();
        let mut h = History::default();
        paint(&mut d, &mut h, Rgba8::WHITE);
        let new_id = d.next_layer_id();
        let add = AddLayerCommand::above_active(&d, Layer::raster(new_id, "Second", 16, 16));
        h.execute(&mut d, Box::new(add)).expect("add layer");
        paint(&mut d, &mut h, Rgba8::BLACK);

        assert_eq!(d.layer_count(), 2);
        h.undo(&mut d).expect("undo paint");
        h.undo(&mut d).expect("undo add");
        assert_eq!(d.layer_count(), 1);
        h.undo(&mut d).expect("undo first paint");
        assert_eq!(
            d.layers
                .get(d.active_layer)
                .and_then(|l| l.pixmap())
                .map(|p| p.get(0, 0)),
            Some(Rgba8::TRANSPARENT)
        );
    }

    #[test]
    fn entries_mark_the_current_state() {
        let mut d = doc();
        let mut h = History::default();
        paint(&mut d, &mut h, Rgba8::WHITE);
        paint(&mut d, &mut h, Rgba8::BLACK);
        h.undo(&mut d).expect("undo");
        let entries = h.entries();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].current, "the first entry is the state we are at");
        assert!(!entries[1].current);
    }

    #[test]
    fn saved_marker_tracks_edits() {
        let mut d = doc();
        let mut h = History::default();
        assert!(!h.has_unsaved_changes());
        paint(&mut d, &mut h, Rgba8::WHITE);
        assert!(h.has_unsaved_changes());
        h.mark_saved();
        assert!(!h.has_unsaved_changes());
        h.undo(&mut d).expect("undo");
        assert!(
            h.has_unsaved_changes(),
            "undoing past the save point is a change too"
        );
    }
}
