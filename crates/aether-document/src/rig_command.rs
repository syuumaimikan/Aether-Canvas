//! Undoable rig edits.
//!
//! A rig is small next to pixel data — vertex arrays and keyform numbers — so
//! rig edits are recorded as whole before/after snapshots. That keeps every
//! rig operation (adding a key, meshing a layer, dragging a vertex, retuning
//! physics) on one well-tested command instead of dozens of bespoke ones.
//!
//! Two things are deliberately *not* part of the snapshot:
//!
//! * **Parameter values.** Moving a slider is posing, like scrubbing a
//!   timeline; undoing a keyform edit must not also yank the sliders back to
//!   where they were when the edit was made.
//! * **Simulation output**, which the runtime recomputes every frame.

use crate::command::Command;
use crate::document::Document;
use aether_core::Result;
use aether_rig::Rig;
use std::any::Any;

/// Replace the document's rig, keeping the current pose.
#[derive(Debug)]
pub struct SetRigCommand {
    label: String,
    before: Option<Rig>,
    after: Rig,
    coalesce: Option<String>,
}

impl SetRigCommand {
    /// Set the rig to `after`; the current rig is captured on apply.
    pub fn new(label: impl Into<String>, after: Rig) -> Self {
        Self {
            label: label.into(),
            before: None,
            after,
            coalesce: None,
        }
    }

    /// A command for an edit that is already visible in the document (an
    /// interactive tool edits the rig live and records it when the gesture
    /// ends). Push it with [`History::push_applied`](crate::History::push_applied).
    pub fn applied(label: impl Into<String>, before: Rig, after: Rig) -> Self {
        Self {
            label: label.into(),
            before: Some(before),
            after,
            coalesce: None,
        }
    }

    /// Merge consecutive commands with the same key into one history entry,
    /// for slider drags in rig inspectors.
    pub fn coalescing(mut self, key: impl Into<String>) -> Self {
        self.coalesce = Some(key.into());
        self
    }

    /// True when the edit changed nothing.
    pub fn is_noop(&self) -> bool {
        self.before
            .as_ref()
            .map(|b| same_structure(b, &self.after))
            .unwrap_or(false)
    }

    fn install(doc: &mut Document, rig: &Rig) {
        let values = std::mem::take(&mut doc.rig.values);
        let dynamics = std::mem::take(&mut doc.rig.dynamics);
        doc.rig = rig.clone();
        doc.rig.values = values;
        doc.rig.dynamics = dynamics;
        doc.mark_all_dirty();
    }
}

/// Equal apart from pose state.
fn same_structure(a: &Rig, b: &Rig) -> bool {
    let strip = |r: &Rig| {
        let mut r = r.clone();
        r.values.clear();
        r.dynamics = Default::default();
        r
    };
    strip(a) == strip(b)
}

impl Command for SetRigCommand {
    fn name(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        if self.before.is_none() {
            self.before = Some(doc.rig.clone());
        }
        Self::install(doc, &self.after);
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        if let Some(before) = &self.before {
            Self::install(doc, before);
        }
        Ok(())
    }

    fn coalesce_key(&self) -> Option<String> {
        self.coalesce.clone()
    }

    fn absorb(&mut self, newer: &dyn Command) -> bool {
        let Some(other) = newer.as_any().downcast_ref::<SetRigCommand>() else {
            return false;
        };
        self.after = other.after.clone();
        self.label = other.label.clone();
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::History;
    use aether_core::math::{vec2, Rect};
    use aether_rig::{ArtMesh, Parameter};

    fn doc() -> Document {
        Document::new(64, 64, "rig")
    }

    #[test]
    fn rig_edits_undo_and_redo() {
        let mut d = doc();
        let mut history = History::default();
        let layer = d.active_layer;
        let mut rig = d.rig.clone();
        rig.set_mesh(ArtMesh::quad(
            layer,
            Rect::from_corners(vec2(0.0, 0.0), vec2(64.0, 64.0)),
        ));
        history
            .execute(&mut d, Box::new(SetRigCommand::new("Mesh", rig)))
            .expect("execute");
        assert!(d.rig.mesh(layer).is_some());
        history.undo(&mut d).expect("undo");
        assert!(d.rig.mesh(layer).is_none());
        history.redo(&mut d).expect("redo");
        assert!(d.rig.mesh(layer).is_some());
    }

    #[test]
    fn undo_keeps_the_current_pose() {
        let mut d = doc();
        let mut history = History::default();
        let id = d.ids.parameter();
        let mut rig = d.rig.clone();
        rig.add_parameter(Parameter::new(id, "AngleX", -30.0, 30.0, 0.0))
            .expect("param");
        history
            .execute(&mut d, Box::new(SetRigCommand::new("Add parameter", rig)))
            .expect("execute");
        d.rig.set_value(id, 12.0);
        let mut rig = d.rig.clone();
        rig.parameter_mut(id).expect("param").group = "Face".into();
        history
            .execute(&mut d, Box::new(SetRigCommand::new("Group", rig)))
            .expect("execute");
        d.rig.set_value(id, 20.0);
        history.undo(&mut d).expect("undo");
        assert_eq!(d.rig.value(id), 20.0, "undo is not a slider move");
        assert_eq!(d.rig.parameter(id).map(|p| p.group.as_str()), Some(""));
    }

    #[test]
    fn slider_drags_coalesce() {
        let mut d = doc();
        let mut history = History::default();
        for i in 0..5 {
            let mut rig = d.rig.clone();
            rig.behaviours.blink.max_interval = i as f32;
            history
                .execute(
                    &mut d,
                    Box::new(SetRigCommand::new("Blink", rig).coalescing("blink")),
                )
                .expect("execute");
        }
        history.undo(&mut d).expect("undo");
        assert_eq!(
            d.rig.behaviours.blink.max_interval, 6.0,
            "one undo restores the start"
        );
        assert!(!history.can_undo());
    }

    #[test]
    fn documents_with_rigs_survive_a_reload() {
        let mut d = doc();
        let id = d.ids.parameter();
        d.rig
            .add_parameter(Parameter::new(id, "AngleX", -30.0, 30.0, 0.0))
            .expect("param");
        let text = serde_json::to_string(&d).expect("serialize");
        let mut back: Document = serde_json::from_str(&text).expect("deserialize");
        back.repair();
        assert!(back.rig.parameter(id).is_some());
        assert!(back.next_layer_id().raw() > id.raw(), "rig ids are reserved too");
    }
}
