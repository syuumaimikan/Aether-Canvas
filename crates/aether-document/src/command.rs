//! The command system.
//!
//! Every edit an artist can make is a [`Command`]: an object that knows how to
//! apply itself to a [`Document`] *and* how to put the document back the way it
//! was. Undo is therefore not a special case anywhere in the codebase — it is
//! the same objects run backwards by [`History`](crate::History).
//!
//! Commands store the smallest state that makes both directions exact. A brush
//! stroke stores the before and after pixels of just the rectangle it touched,
//! not a copy of the layer; deleting a group stores the detached subtree so the
//! layers come back with their original ids and nesting.

use crate::document::Document;
use crate::layer::{ColorLabel, Layer, LayerContent};
use crate::selection::Selection;
use crate::tree::DetachedSubtree;
use aether_core::blend::BlendMode;
use aether_core::math::IRect;
use aether_core::{AetherError, LayerId, Result};
use aether_raster::adjust::Adjustment;
use aether_raster::{LayerEffect, Mask, Pixmap};
use std::any::Any;
use std::fmt::Debug;

/// One undoable edit.
pub trait Command: Debug + Send {
    /// Label shown in the history panel.
    fn name(&self) -> String;

    /// Perform the edit.
    fn apply(&mut self, doc: &mut Document) -> Result<()>;

    /// Reverse the edit. Only ever called after a successful [`Command::apply`].
    fn undo(&mut self, doc: &mut Document) -> Result<()>;

    /// Key identifying a run of commands that should collapse into one history
    /// entry, such as one drag of an opacity slider. `None` disables merging.
    fn coalesce_key(&self) -> Option<String> {
        None
    }

    /// Fold a newer command with the same coalesce key into this one.
    ///
    /// Returns `false` when the merge is not possible, in which case the newer
    /// command is pushed as its own history entry.
    fn absorb(&mut self, _newer: &dyn Command) -> bool {
        false
    }

    /// Downcast support for [`Command::absorb`].
    fn as_any(&self) -> &dyn Any;
}

/// Replace a rectangle of a raster layer's pixels.
///
/// This is the workhorse behind brush strokes, erasing, fills, filters and
/// "clear layer": anything that changes pixels without changing structure.
#[derive(Debug)]
pub struct RegionEdit {
    label: String,
    layer: LayerId,
    rect: IRect,
    before: Pixmap,
    after: Pixmap,
}

impl RegionEdit {
    /// Build from explicit before/after patches.
    pub fn new(label: impl Into<String>, layer: LayerId, rect: IRect, before: Pixmap, after: Pixmap) -> Self {
        Self {
            label: label.into(),
            layer,
            rect,
            before,
            after,
        }
    }

    /// Snapshot `rect` of `layer`, run `edit`, and capture the result.
    ///
    /// The edit is applied immediately; push the returned command with
    /// [`History::push_applied`](crate::History::push_applied).
    pub fn capture(
        doc: &mut Document,
        layer: LayerId,
        rect: IRect,
        label: impl Into<String>,
        edit: impl FnOnce(&mut Pixmap) -> Result<()>,
    ) -> Result<Self> {
        let bounds = doc.bounds();
        let rect = rect.intersect(&bounds);
        if rect.is_empty() {
            return Err(AetherError::invalid("edit region is empty"));
        }
        let target = doc
            .layers
            .try_get_mut(layer)?
            .pixmap_mut()
            .ok_or_else(|| AetherError::document("target layer holds no pixels"))?;
        let before = target.copy_rect(rect);
        edit(target)?;
        let after = target.copy_rect(rect);
        doc.mark_dirty(rect);
        Ok(Self {
            label: label.into(),
            layer,
            rect,
            before,
            after,
        })
    }

    /// The region this edit covers.
    pub fn rect(&self) -> IRect {
        self.rect
    }

    /// True when the edit changed nothing, so it should not enter the history.
    pub fn is_noop(&self) -> bool {
        self.before == self.after
    }

    fn write(&self, doc: &mut Document, patch: &Pixmap) -> Result<()> {
        let target = doc
            .layers
            .try_get_mut(self.layer)?
            .pixmap_mut()
            .ok_or_else(|| AetherError::document("target layer holds no pixels"))?;
        target.paste_rect(patch, self.rect.x, self.rect.y);
        doc.mark_dirty(self.rect);
        Ok(())
    }
}

impl Command for RegionEdit {
    fn name(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        let patch = self.after.clone();
        self.write(doc, &patch)
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        let patch = self.before.clone();
        self.write(doc, &patch)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Insert a layer into the tree.
#[derive(Debug)]
pub struct AddLayerCommand {
    layer: Option<Layer>,
    id: LayerId,
    parent: Option<LayerId>,
    index: usize,
    previous_active: LayerId,
}

impl AddLayerCommand {
    /// Add `layer` under `parent` at `index`.
    pub fn new(layer: Layer, parent: Option<LayerId>, index: usize) -> Self {
        Self {
            id: layer.id,
            layer: Some(layer),
            parent,
            index,
            previous_active: LayerId::NONE,
        }
    }

    /// Add `layer` directly above the document's active layer.
    pub fn above_active(doc: &Document, layer: Layer) -> Self {
        let (parent, index) = doc.insert_point();
        Self::new(layer, parent, index)
    }

    /// The id the new layer will have.
    pub fn layer_id(&self) -> LayerId {
        self.id
    }
}

impl Command for AddLayerCommand {
    fn name(&self) -> String {
        format!("Add Layer {}", self.id)
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        let layer = self
            .layer
            .take()
            .ok_or_else(|| AetherError::document("add-layer command has no layer to insert"))?;
        self.previous_active = doc.active_layer;
        doc.layers.insert(layer, self.parent, self.index)?;
        doc.active_layer = self.id;
        doc.mark_all_dirty();
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        let detached = doc.layers.remove_subtree(self.id)?;
        self.layer = detached.layers.into_iter().next();
        doc.active_layer = self.previous_active;
        doc.ensure_active_layer();
        doc.mark_all_dirty();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Remove a layer (and its descendants) from the tree.
#[derive(Debug)]
pub struct DeleteLayerCommand {
    id: LayerId,
    detached: Option<DetachedSubtree>,
    previous_active: LayerId,
}

impl DeleteLayerCommand {
    /// Delete `id` and everything inside it.
    pub fn new(id: LayerId) -> Self {
        Self {
            id,
            detached: None,
            previous_active: LayerId::NONE,
        }
    }
}

impl Command for DeleteLayerCommand {
    fn name(&self) -> String {
        format!("Delete Layer {}", self.id)
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        self.previous_active = doc.active_layer;
        self.detached = Some(doc.layers.remove_subtree(self.id)?);
        doc.ensure_active_layer();
        doc.mark_all_dirty();
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        let detached = self
            .detached
            .take()
            .ok_or_else(|| AetherError::document("delete-layer command has nothing to restore"))?;
        doc.layers.reattach(detached)?;
        doc.active_layer = self.previous_active;
        doc.ensure_active_layer();
        doc.mark_all_dirty();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Move a layer to a new parent and/or position.
#[derive(Debug)]
pub struct MoveLayerCommand {
    id: LayerId,
    to_parent: Option<LayerId>,
    to_index: usize,
    from_parent: Option<LayerId>,
    from_index: usize,
}

impl MoveLayerCommand {
    /// Move `id` under `parent` at `index`.
    pub fn new(id: LayerId, parent: Option<LayerId>, index: usize) -> Self {
        Self {
            id,
            to_parent: parent,
            to_index: index,
            from_parent: None,
            from_index: 0,
        }
    }
}

impl Command for MoveLayerCommand {
    fn name(&self) -> String {
        format!("Move Layer {}", self.id)
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        self.from_parent = doc.layers.parent_of(self.id);
        self.from_index = doc
            .layers
            .index_of(self.id)
            .ok_or_else(|| AetherError::document(format!("layer {} is not in the tree", self.id)))?;
        doc.layers.move_node(self.id, self.to_parent, self.to_index)?;
        doc.mark_all_dirty();
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        doc.layers.move_node(self.id, self.from_parent, self.from_index)?;
        doc.mark_all_dirty();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A single settable layer property.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerProperty {
    /// Display name.
    Name(String),
    /// Visibility.
    Visible(bool),
    /// Edit lock.
    Locked(bool),
    /// Transparency lock.
    AlphaLock(bool),
    /// Opacity in `0..=1`.
    Opacity(f32),
    /// Blend mode.
    Blend(BlendMode),
    /// Clip to the layer below.
    Clipping(bool),
    /// Organisational colour tag.
    Label(ColorLabel),
    /// Whether the layer mask is applied.
    MaskEnabled(bool),
}

impl LayerProperty {
    /// Stable discriminant used for history coalescing.
    fn key(&self) -> &'static str {
        match self {
            LayerProperty::Name(_) => "name",
            LayerProperty::Visible(_) => "visible",
            LayerProperty::Locked(_) => "locked",
            LayerProperty::AlphaLock(_) => "alpha_lock",
            LayerProperty::Opacity(_) => "opacity",
            LayerProperty::Blend(_) => "blend",
            LayerProperty::Clipping(_) => "clipping",
            LayerProperty::Label(_) => "label",
            LayerProperty::MaskEnabled(_) => "mask_enabled",
        }
    }

    fn read(layer: &Layer, like: &LayerProperty) -> LayerProperty {
        match like {
            LayerProperty::Name(_) => LayerProperty::Name(layer.name.clone()),
            LayerProperty::Visible(_) => LayerProperty::Visible(layer.visible),
            LayerProperty::Locked(_) => LayerProperty::Locked(layer.locked),
            LayerProperty::AlphaLock(_) => LayerProperty::AlphaLock(layer.alpha_lock),
            LayerProperty::Opacity(_) => LayerProperty::Opacity(layer.opacity),
            LayerProperty::Blend(_) => LayerProperty::Blend(layer.blend_mode),
            LayerProperty::Clipping(_) => LayerProperty::Clipping(layer.clipping),
            LayerProperty::Label(_) => LayerProperty::Label(layer.color_label),
            LayerProperty::MaskEnabled(_) => LayerProperty::MaskEnabled(layer.mask_enabled),
        }
    }

    fn write(&self, layer: &mut Layer) {
        match self {
            LayerProperty::Name(v) => layer.name = v.clone(),
            LayerProperty::Visible(v) => layer.visible = *v,
            LayerProperty::Locked(v) => layer.locked = *v,
            LayerProperty::AlphaLock(v) => layer.alpha_lock = *v,
            LayerProperty::Opacity(v) => layer.opacity = v.clamp(0.0, 1.0),
            LayerProperty::Blend(v) => layer.blend_mode = *v,
            LayerProperty::Clipping(v) => layer.clipping = *v,
            LayerProperty::Label(v) => layer.color_label = *v,
            LayerProperty::MaskEnabled(v) => layer.mask_enabled = *v,
        }
    }
}

/// Change one property of one layer.
#[derive(Debug)]
pub struct SetLayerPropertyCommand {
    id: LayerId,
    after: LayerProperty,
    before: Option<LayerProperty>,
}

impl SetLayerPropertyCommand {
    /// Set `property` on layer `id`.
    pub fn new(id: LayerId, property: LayerProperty) -> Self {
        Self {
            id,
            after: property,
            before: None,
        }
    }
}

impl Command for SetLayerPropertyCommand {
    fn name(&self) -> String {
        format!("Set {} on {}", self.after.key(), self.id)
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        let layer = doc.layers.try_get_mut(self.id)?;
        if self.before.is_none() {
            self.before = Some(LayerProperty::read(layer, &self.after));
        }
        self.after.write(layer);
        doc.mark_all_dirty();
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        let before = self
            .before
            .clone()
            .ok_or_else(|| AetherError::document("property command was never applied"))?;
        let layer = doc.layers.try_get_mut(self.id)?;
        before.write(layer);
        doc.mark_all_dirty();
        Ok(())
    }

    fn coalesce_key(&self) -> Option<String> {
        // Dragging a slider produces a command per frame; collapse them.
        match self.after {
            LayerProperty::Opacity(_) | LayerProperty::Name(_) => {
                Some(format!("prop:{}:{}", self.id, self.after.key()))
            }
            _ => None,
        }
    }

    fn absorb(&mut self, newer: &dyn Command) -> bool {
        let Some(other) = newer.as_any().downcast_ref::<SetLayerPropertyCommand>() else {
            return false;
        };
        if other.id != self.id || other.after.key() != self.after.key() {
            return false;
        }
        // Keep our original "before" and take their newer "after".
        self.after = other.after.clone();
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Attach, replace or remove a layer mask.
#[derive(Debug)]
pub struct SetLayerMaskCommand {
    id: LayerId,
    after: Option<Mask>,
    before: Option<Option<Mask>>,
}

impl SetLayerMaskCommand {
    /// Set (or clear, with `None`) the mask on layer `id`.
    pub fn new(id: LayerId, mask: Option<Mask>) -> Self {
        Self {
            id,
            after: mask,
            before: None,
        }
    }
}

impl Command for SetLayerMaskCommand {
    fn name(&self) -> String {
        if self.after.is_some() {
            format!("Set Mask on {}", self.id)
        } else {
            format!("Remove Mask from {}", self.id)
        }
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        let layer = doc.layers.try_get_mut(self.id)?;
        if self.before.is_none() {
            self.before = Some(layer.mask.clone());
        }
        layer.mask = self.after.clone();
        doc.mark_all_dirty();
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        let before = self
            .before
            .clone()
            .ok_or_else(|| AetherError::document("mask command was never applied"))?;
        doc.layers.try_get_mut(self.id)?.mask = before;
        doc.mark_all_dirty();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Retune an adjustment layer.
///
/// Adjustment parameters live in the layer's content rather than in its common
/// properties, so they get their own command — with the same coalescing so a
/// slider drag is one history entry.
#[derive(Debug)]
pub struct SetAdjustmentCommand {
    id: LayerId,
    after: Adjustment,
    before: Option<Adjustment>,
}

impl SetAdjustmentCommand {
    /// Set the adjustment on layer `id`.
    pub fn new(id: LayerId, adjustment: Adjustment) -> Self {
        Self {
            id,
            after: adjustment,
            before: None,
        }
    }
}

impl Command for SetAdjustmentCommand {
    fn name(&self) -> String {
        format!("Adjust {}", self.after.name())
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        let layer = doc.layers.try_get_mut(self.id)?;
        let LayerContent::Adjustment(content) = &mut layer.content else {
            return Err(AetherError::document("that layer is not an adjustment layer"));
        };
        if self.before.is_none() {
            self.before = Some(content.adjustment.clone());
        }
        content.adjustment = self.after.clone();
        doc.mark_all_dirty();
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        let before = self
            .before
            .clone()
            .ok_or_else(|| AetherError::document("adjustment command was never applied"))?;
        let layer = doc.layers.try_get_mut(self.id)?;
        let LayerContent::Adjustment(content) = &mut layer.content else {
            return Err(AetherError::document("that layer is not an adjustment layer"));
        };
        content.adjustment = before;
        doc.mark_all_dirty();
        Ok(())
    }

    fn coalesce_key(&self) -> Option<String> {
        Some(format!("adjustment:{}", self.id))
    }

    fn absorb(&mut self, newer: &dyn Command) -> bool {
        let Some(other) = newer.as_any().downcast_ref::<SetAdjustmentCommand>() else {
            return false;
        };
        if other.id != self.id {
            return false;
        }
        self.after = other.after.clone();
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Replace a layer's effect stack.
///
/// The whole stack is swapped in one command rather than one command per
/// tweak: an effect's parameters are tiny, and treating the stack as a value
/// means adding, removing, reordering and retuning all undo identically.
#[derive(Debug)]
pub struct SetLayerEffectsCommand {
    label: String,
    id: LayerId,
    after: Vec<LayerEffect>,
    before: Option<Vec<LayerEffect>>,
}

impl SetLayerEffectsCommand {
    /// Set the effect stack on layer `id`.
    pub fn new(label: impl Into<String>, id: LayerId, effects: Vec<LayerEffect>) -> Self {
        Self {
            label: label.into(),
            id,
            after: effects,
            before: None,
        }
    }
}

impl Command for SetLayerEffectsCommand {
    fn name(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        let layer = doc.layers.try_get_mut(self.id)?;
        if self.before.is_none() {
            self.before = Some(layer.effects.clone());
        }
        layer.effects = self.after.clone();
        doc.mark_all_dirty();
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        let before = self
            .before
            .clone()
            .ok_or_else(|| AetherError::document("effect command was never applied"))?;
        doc.layers.try_get_mut(self.id)?.effects = before;
        doc.mark_all_dirty();
        Ok(())
    }

    fn coalesce_key(&self) -> Option<String> {
        // Dragging an effect slider emits a command per frame.
        Some(format!("effects:{}", self.id))
    }

    fn absorb(&mut self, newer: &dyn Command) -> bool {
        let Some(other) = newer.as_any().downcast_ref::<SetLayerEffectsCommand>() else {
            return false;
        };
        if other.id != self.id {
            return false;
        }
        self.after = other.after.clone();
        self.label = other.label.clone();
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Change the selection.
#[derive(Debug)]
pub struct SetSelectionCommand {
    label: String,
    after: Selection,
    before: Option<Selection>,
}

impl SetSelectionCommand {
    /// Replace the document's selection with `selection`.
    pub fn new(label: impl Into<String>, selection: Selection) -> Self {
        Self {
            label: label.into(),
            after: selection,
            before: None,
        }
    }
}

impl Command for SetSelectionCommand {
    fn name(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        if self.before.is_none() {
            self.before = Some(doc.selection.clone());
        }
        doc.selection = self.after.clone();
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        doc.selection = self
            .before
            .clone()
            .ok_or_else(|| AetherError::document("selection command was never applied"))?;
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Everything a resize would discard, kept so it can be put back.
#[derive(Clone, Debug)]
struct CanvasSnapshot {
    width: u32,
    height: u32,
    pixels: Vec<(LayerId, Pixmap)>,
    masks: Vec<(LayerId, Option<Mask>)>,
}

/// Resize the canvas, keeping content anchored at the top-left.
#[derive(Debug)]
pub struct ResizeCanvasCommand {
    width: u32,
    height: u32,
    before: Option<CanvasSnapshot>,
}

impl ResizeCanvasCommand {
    /// Resize to `width` x `height`.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
            before: None,
        }
    }
}

impl Command for ResizeCanvasCommand {
    fn name(&self) -> String {
        format!("Resize Canvas to {}x{}", self.width, self.height)
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        if self.before.is_none() {
            // Shrinking is lossy, so keep the pixels we are about to discard.
            let pixels = doc
                .layers
                .iter()
                .filter_map(|l| l.pixmap().map(|p| (l.id, p.clone())))
                .collect();
            let masks = doc.layers.iter().map(|l| (l.id, l.mask.clone())).collect();
            self.before = Some(CanvasSnapshot {
                width: doc.width,
                height: doc.height,
                pixels,
                masks,
            });
        }
        doc.resize_canvas(self.width, self.height);
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        let snapshot = self
            .before
            .clone()
            .ok_or_else(|| AetherError::document("resize command was never applied"))?;
        doc.resize_canvas(snapshot.width, snapshot.height);
        for (id, pixmap) in snapshot.pixels {
            if let Some(layer) = doc.layers.get_mut(id) {
                if let Some(target) = layer.pixmap_mut() {
                    *target = pixmap;
                }
            }
        }
        for (id, mask) in snapshot.masks {
            if let Some(layer) = doc.layers.get_mut(id) {
                layer.mask = mask;
            }
        }
        doc.mark_all_dirty();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Several commands that undo and redo as one history entry.
#[derive(Debug)]
pub struct Transaction {
    label: String,
    commands: Vec<Box<dyn Command>>,
}

impl Transaction {
    /// An empty transaction.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            commands: Vec::new(),
        }
    }

    /// Add a command to the batch.
    pub fn push(&mut self, command: Box<dyn Command>) {
        self.commands.push(command);
    }

    /// True when the batch contains nothing.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// How many commands are batched.
    pub fn len(&self) -> usize {
        self.commands.len()
    }
}

impl Command for Transaction {
    fn name(&self) -> String {
        self.label.clone()
    }

    fn apply(&mut self, doc: &mut Document) -> Result<()> {
        for (i, command) in self.commands.iter_mut().enumerate() {
            if let Err(err) = command.apply(doc) {
                // Roll back the part that succeeded so the document is never
                // left half-edited.
                for done in self.commands[..i].iter_mut().rev() {
                    let _ = done.undo(doc);
                }
                return Err(err);
            }
        }
        Ok(())
    }

    fn undo(&mut self, doc: &mut Document) -> Result<()> {
        for command in self.commands.iter_mut().rev() {
            command.undo(doc)?;
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;

    fn doc() -> Document {
        Document::new(16, 16, "test")
    }

    #[test]
    fn region_edit_round_trips() {
        let mut d = doc();
        let layer = d.active_layer;
        let mut cmd = RegionEdit::capture(&mut d, layer, IRect::new(2, 2, 4, 4), "Fill", |pm| {
            pm.fill_rect(IRect::new(2, 2, 4, 4), Rgba8::WHITE);
            Ok(())
        })
        .expect("capture");

        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(3, 3)),
            Some(Rgba8::WHITE)
        );
        cmd.undo(&mut d).expect("undo");
        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(3, 3)),
            Some(Rgba8::TRANSPARENT)
        );
        cmd.apply(&mut d).expect("redo");
        assert_eq!(
            d.layers.get(layer).and_then(|l| l.pixmap()).map(|p| p.get(3, 3)),
            Some(Rgba8::WHITE)
        );
    }

    #[test]
    fn region_edit_detects_a_noop() {
        let mut d = doc();
        let layer = d.active_layer;
        let cmd = RegionEdit::capture(&mut d, layer, IRect::new(0, 0, 4, 4), "Nothing", |_| Ok(()))
            .expect("capture");
        assert!(cmd.is_noop());
    }

    #[test]
    fn add_and_undo_layer() {
        let mut d = doc();
        let before = d.layer_count();
        let id = d.next_layer_id();
        let mut cmd = AddLayerCommand::above_active(&d, Layer::raster(id, "New", 16, 16));
        cmd.apply(&mut d).expect("apply");
        assert_eq!(d.layer_count(), before + 1);
        assert_eq!(d.active_layer, id);
        cmd.undo(&mut d).expect("undo");
        assert_eq!(d.layer_count(), before);
        assert!(!d.layers.contains(id));
        cmd.apply(&mut d).expect("redo");
        assert!(d.layers.contains(id));
    }

    #[test]
    fn delete_restores_pixels_and_position() {
        let mut d = doc();
        let first = d.active_layer;
        let second = d.add_raster_layer("Second");
        if let Some(pm) = d.layers.get_mut(second).and_then(|l| l.pixmap_mut()) {
            pm.fill(Rgba8::WHITE);
        }
        let mut cmd = DeleteLayerCommand::new(second);
        cmd.apply(&mut d).expect("apply");
        assert_eq!(d.layer_count(), 1);
        assert_eq!(d.active_layer, first);
        cmd.undo(&mut d).expect("undo");
        assert_eq!(d.layers.roots(), &[first, second]);
        assert_eq!(
            d.layers.get(second).and_then(|l| l.pixmap()).map(|p| p.get(0, 0)),
            Some(Rgba8::WHITE),
            "pixels must survive delete + undo"
        );
    }

    #[test]
    fn property_commands_restore_the_old_value() {
        let mut d = doc();
        let id = d.active_layer;
        let mut cmd = SetLayerPropertyCommand::new(id, LayerProperty::Opacity(0.25));
        cmd.apply(&mut d).expect("apply");
        assert_eq!(d.layers.get(id).map(|l| l.opacity), Some(0.25));
        cmd.undo(&mut d).expect("undo");
        assert_eq!(d.layers.get(id).map(|l| l.opacity), Some(1.0));
    }

    #[test]
    fn opacity_commands_coalesce() {
        let mut d = doc();
        let id = d.active_layer;
        let mut first = SetLayerPropertyCommand::new(id, LayerProperty::Opacity(0.9));
        first.apply(&mut d).expect("apply");
        let mut second = SetLayerPropertyCommand::new(id, LayerProperty::Opacity(0.4));
        second.apply(&mut d).expect("apply");

        assert_eq!(first.coalesce_key(), second.coalesce_key());
        assert!(first.absorb(&second));
        first.undo(&mut d).expect("undo");
        assert_eq!(
            d.layers.get(id).map(|l| l.opacity),
            Some(1.0),
            "the merged command must undo to the value before the whole drag"
        );
    }

    #[test]
    fn different_layers_do_not_coalesce() {
        let mut d = doc();
        let a = d.active_layer;
        let b = d.add_raster_layer("Other");
        let mut first = SetLayerPropertyCommand::new(a, LayerProperty::Opacity(0.5));
        let second = SetLayerPropertyCommand::new(b, LayerProperty::Opacity(0.5));
        assert_ne!(first.coalesce_key(), second.coalesce_key());
        assert!(!first.absorb(&second));
    }

    #[test]
    fn resize_undo_restores_cropped_pixels() {
        let mut d = doc();
        let layer = d.active_layer;
        if let Some(pm) = d.layers.get_mut(layer).and_then(|l| l.pixmap_mut()) {
            pm.fill(Rgba8::WHITE);
        }
        let mut cmd = ResizeCanvasCommand::new(4, 4);
        cmd.apply(&mut d).expect("apply");
        assert_eq!((d.width, d.height), (4, 4));
        cmd.undo(&mut d).expect("undo");
        assert_eq!((d.width, d.height), (16, 16));
        assert_eq!(
            d.layers
                .get(layer)
                .and_then(|l| l.pixmap())
                .map(|p| p.get(15, 15)),
            Some(Rgba8::WHITE),
            "cropped pixels must come back"
        );
    }

    #[test]
    fn transactions_apply_and_undo_as_one() {
        let mut d = doc();
        let id = d.active_layer;
        let mut tx = Transaction::new("Batch");
        tx.push(Box::new(SetLayerPropertyCommand::new(
            id,
            LayerProperty::Opacity(0.5),
        )));
        tx.push(Box::new(SetLayerPropertyCommand::new(
            id,
            LayerProperty::Visible(false),
        )));
        tx.apply(&mut d).expect("apply");
        assert_eq!(
            d.layers.get(id).map(|l| (l.opacity, l.visible)),
            Some((0.5, false))
        );
        tx.undo(&mut d).expect("undo");
        assert_eq!(
            d.layers.get(id).map(|l| (l.opacity, l.visible)),
            Some((1.0, true))
        );
    }

    #[test]
    fn a_failing_transaction_rolls_back() {
        let mut d = doc();
        let id = d.active_layer;
        let mut tx = Transaction::new("Batch");
        tx.push(Box::new(SetLayerPropertyCommand::new(
            id,
            LayerProperty::Opacity(0.5),
        )));
        // A command targeting a layer that does not exist must fail.
        tx.push(Box::new(SetLayerPropertyCommand::new(
            LayerId(9999),
            LayerProperty::Visible(false),
        )));
        assert!(tx.apply(&mut d).is_err());
        assert_eq!(
            d.layers.get(id).map(|l| l.opacity),
            Some(1.0),
            "the successful half must have been rolled back"
        );
    }

    #[test]
    fn mask_commands_round_trip() {
        let mut d = doc();
        let id = d.active_layer;
        let mut cmd = SetLayerMaskCommand::new(id, Some(Mask::filled(16, 16, 128)));
        cmd.apply(&mut d).expect("apply");
        assert!(d.layers.get(id).and_then(|l| l.mask.as_ref()).is_some());
        cmd.undo(&mut d).expect("undo");
        assert!(d.layers.get(id).and_then(|l| l.mask.as_ref()).is_none());
    }
}

#[cfg(test)]
mod effect_command_tests {
    use super::*;
    use aether_raster::{EffectKind, LayerEffect};

    fn doc() -> Document {
        Document::new(16, 16, "test")
    }

    #[test]
    fn setting_effects_round_trips() {
        let mut d = doc();
        let id = d.active_layer;
        let effects = vec![LayerEffect::new(EffectKind::Blur { sigma: 3.0 })];
        let mut cmd = SetLayerEffectsCommand::new("Add Blur", id, effects.clone());
        cmd.apply(&mut d).expect("apply");
        assert_eq!(d.layers.get(id).map(|l| l.effects.clone()), Some(effects));
        cmd.undo(&mut d).expect("undo");
        assert_eq!(d.layers.get(id).map(|l| l.effects.len()), Some(0));
    }

    #[test]
    fn effect_edits_coalesce_into_one_history_entry() {
        let mut d = doc();
        let id = d.active_layer;
        let mut first = SetLayerEffectsCommand::new(
            "Blur",
            id,
            vec![LayerEffect::new(EffectKind::Blur { sigma: 2.0 })],
        );
        first.apply(&mut d).expect("apply");
        let mut second = SetLayerEffectsCommand::new(
            "Blur",
            id,
            vec![LayerEffect::new(EffectKind::Blur { sigma: 9.0 })],
        );
        second.apply(&mut d).expect("apply");
        assert!(first.absorb(&second));
        first.undo(&mut d).expect("undo");
        assert!(d.layers.get(id).map(|l| l.effects.is_empty()).unwrap_or(false));
    }

    #[test]
    fn layers_report_whether_effects_are_active() {
        let mut d = doc();
        let id = d.active_layer;
        let layer = d.layers.get_mut(id).expect("layer");
        assert!(!layer.has_effects());
        layer.effects.push(LayerEffect {
            kind: EffectKind::Blur { sigma: 1.0 },
            enabled: false,
        });
        assert!(!layer.has_effects(), "a disabled effect does not count");
        layer.effects[0].enabled = true;
        assert!(layer.has_effects());
    }
}

#[cfg(test)]
mod adjustment_command_tests {
    use super::*;
    use crate::layer::Layer;
    use aether_raster::adjust::Adjustment;

    #[test]
    fn retuning_an_adjustment_layer_round_trips() {
        let mut d = Document::new(8, 8, "test");
        let id = d.next_layer_id();
        d.layers
            .push_top(Layer::adjustment(id, "Levels", Adjustment::default_levels()))
            .expect("insert");

        let mut cmd = SetAdjustmentCommand::new(id, Adjustment::Invert);
        cmd.apply(&mut d).expect("apply");
        match &d.layers.get(id).expect("layer").content {
            LayerContent::Adjustment(a) => assert_eq!(a.adjustment, Adjustment::Invert),
            other => panic!("unexpected content: {other:?}"),
        }
        cmd.undo(&mut d).expect("undo");
        match &d.layers.get(id).expect("layer").content {
            LayerContent::Adjustment(a) => assert_eq!(a.adjustment, Adjustment::default_levels()),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn adjusting_a_raster_layer_is_refused() {
        let mut d = Document::new(8, 8, "test");
        let id = d.active_layer;
        let mut cmd = SetAdjustmentCommand::new(id, Adjustment::Invert);
        assert!(cmd.apply(&mut d).is_err());
    }
}
