//! The document.
//!
//! A [`Document`] owns a canvas size, a [`LayerTree`], the current selection
//! and the id allocator that keeps layer ids unique for the lifetime of the
//! project — including across save and load.
//!
//! Direct mutation methods exist (they are what commands are built from), but
//! anything an artist can trigger should go through
//! [`History`](crate::History) so it can be undone.

use crate::layer::{Layer, LayerContent};
use crate::selection::Selection;
use crate::tree::LayerTree;
use aether_core::color::{ColorModel, Rgba8};
use aether_core::id::IdGenerator;
use aether_core::math::IRect;
use aether_core::{AetherError, DocumentId, LayerId, Result};
use aether_raster::tile::DirtyRegion;
use aether_raster::Pixmap;
use serde::{Deserialize, Serialize};

/// What shows through where every layer is transparent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Background {
    /// Nothing: the canvas keeps its alpha, and the UI draws a checkerboard.
    #[default]
    Transparent,
    /// A solid colour baked into the composite.
    Solid(Rgba8),
}

/// Authoring metadata stored with the project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentMetadata {
    /// Unix timestamp of creation, in seconds.
    pub created: u64,
    /// Unix timestamp of the last save, in seconds.
    pub modified: u64,
    /// Free-form author string.
    pub author: String,
    /// Free-form description.
    pub description: String,
    /// Pixels per inch, used by print-oriented exporters.
    pub dpi: f32,
}

impl DocumentMetadata {
    /// Metadata stamped with the current time.
    pub fn now() -> Self {
        let now = current_unix_time();
        Self {
            created: now,
            modified: now,
            author: String::new(),
            description: String::new(),
            dpi: 72.0,
        }
    }

    /// Update the modification timestamp.
    pub fn touch(&mut self) {
        self.modified = current_unix_time();
    }
}

fn current_unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A canvas, its layers and everything editing state needs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    /// Identity within a project.
    pub id: DocumentId,
    /// Display name (also the default file name).
    pub name: String,
    /// Canvas width in pixels.
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
    /// What shows through transparent areas.
    pub background: Background,
    /// Authoring colour model.
    pub color_model: ColorModel,
    /// The layers.
    pub layers: LayerTree,
    /// Which layer editing tools act on.
    pub active_layer: LayerId,
    /// The current selection.
    pub selection: Selection,
    /// Allocator for new layer ids.
    pub ids: IdGenerator,
    /// Authoring metadata.
    pub metadata: DocumentMetadata,

    /// Region changed since the renderer last caught up. Never serialised: it
    /// is a cache-invalidation hint, not part of the artwork.
    #[serde(skip)]
    dirty: DirtyRegion,
}

impl Document {
    /// An empty document with no layers.
    pub fn empty(width: u32, height: u32, name: impl Into<String>) -> Self {
        let ids = IdGenerator::new();
        Self {
            id: ids.document(),
            name: name.into(),
            width: width.max(1),
            height: height.max(1),
            background: Background::Transparent,
            color_model: ColorModel::Rgba8,
            layers: LayerTree::new(),
            active_layer: LayerId::NONE,
            selection: Selection::none(),
            ids,
            metadata: DocumentMetadata::now(),
            dirty: DirtyRegion::new(),
        }
    }

    /// A new document with one empty raster layer, ready to paint on.
    pub fn new(width: u32, height: u32, name: impl Into<String>) -> Self {
        let mut doc = Self::empty(width, height, name);
        let id = doc.add_raster_layer("Layer 1");
        doc.active_layer = id;
        doc.mark_dirty(doc.bounds());
        doc
    }

    /// The canvas rectangle.
    pub fn bounds(&self) -> IRect {
        IRect::from_size(self.width, self.height)
    }

    /// Allocate an id for a layer that is about to be created.
    pub fn next_layer_id(&self) -> LayerId {
        self.ids.layer()
    }

    /// Create and insert an empty raster layer above the active layer.
    ///
    /// Returns the new layer's id. This is the low-level helper; the undoable
    /// entry point is [`crate::command::AddLayerCommand`].
    pub fn add_raster_layer(&mut self, name: impl Into<String>) -> LayerId {
        let id = self.next_layer_id();
        let layer = Layer::raster(id, name, self.width, self.height);
        let (parent, index) = self.insert_point();
        // push_top / insert cannot fail here: the id is fresh and the parent
        // was taken from the tree itself.
        let _ = self.layers.insert(layer, parent, index);
        self.active_layer = id;
        id
    }

    /// Where a newly created layer should go: directly above the active layer,
    /// inside the same group.
    pub fn insert_point(&self) -> (Option<LayerId>, usize) {
        if self.active_layer.is_none() || !self.layers.contains(self.active_layer) {
            return (None, self.layers.roots().len());
        }
        let parent = self.layers.parent_of(self.active_layer);
        let index = self
            .layers
            .index_of(self.active_layer)
            .map(|i| i + 1)
            .unwrap_or(0);
        (parent, index)
    }

    /// The active layer, if it still exists.
    pub fn active(&self) -> Option<&Layer> {
        self.layers.get(self.active_layer)
    }

    /// The active layer for editing.
    pub fn active_mut(&mut self) -> Option<&mut Layer> {
        self.layers.get_mut(self.active_layer)
    }

    /// The active layer's pixel buffer, if it is a raster layer that accepts edits.
    ///
    /// Returns a descriptive error rather than `None` so the UI can explain
    /// *why* a stroke did nothing.
    pub fn paint_target(&mut self) -> Result<&mut Pixmap> {
        let id = self.active_layer;
        let layer = self
            .layers
            .get_mut(id)
            .ok_or_else(|| AetherError::document("no active layer"))?;
        if layer.locked {
            return Err(AetherError::document(format!("layer '{}' is locked", layer.name)));
        }
        if !layer.visible {
            return Err(AetherError::document(format!("layer '{}' is hidden", layer.name)));
        }
        match &mut layer.content {
            LayerContent::Raster(r) => Ok(&mut r.pixmap),
            other => Err(AetherError::document(format!(
                "layer '{}' is a {} layer and cannot be painted on",
                layer.name,
                match other {
                    LayerContent::Group(_) => "group",
                    LayerContent::Adjustment(_) => "adjustment",
                    LayerContent::Fill(_) => "fill",
                    _ => "non-raster",
                }
            ))),
        }
    }

    /// Select a layer, if it exists.
    pub fn set_active_layer(&mut self, id: LayerId) -> bool {
        if self.layers.contains(id) {
            self.active_layer = id;
            true
        } else {
            false
        }
    }

    /// Pick a sensible active layer after a deletion.
    pub fn ensure_active_layer(&mut self) {
        if self.layers.contains(self.active_layer) {
            return;
        }
        self.active_layer = self
            .layers
            .iter_ui_order()
            .first()
            .map(|(id, _)| *id)
            .unwrap_or(LayerId::NONE);
    }

    /// Change the canvas size, resizing every layer buffer.
    pub fn resize_canvas(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        for layer in self.layers.iter_mut() {
            layer.resize_canvas(self.width, self.height);
        }
        self.selection.resize(self.width, self.height);
        self.mark_dirty(self.bounds());
    }

    /// Mark a region as needing re-composite.
    pub fn mark_dirty(&mut self, rect: IRect) {
        self.dirty.add(rect.intersect(&self.bounds()));
    }

    /// Mark the whole canvas dirty.
    pub fn mark_all_dirty(&mut self) {
        let bounds = self.bounds();
        self.dirty.add(bounds);
    }

    /// The region changed since the last [`Document::take_dirty`].
    pub fn dirty(&self) -> IRect {
        self.dirty.bounds()
    }

    /// Consume and reset the dirty region.
    pub fn take_dirty(&mut self) -> IRect {
        self.dirty.take()
    }

    /// Restore invariants after loading from disk.
    ///
    /// A file may have been written by an older build, hand-edited, or produced
    /// by a plugin, so nothing here is assumed: the parent index is rebuilt,
    /// the id allocator is advanced past every id in use, and the active layer
    /// is re-pointed if it went missing.
    pub fn repair(&mut self) {
        self.layers.rebuild_parents();
        self.ids.reserve_at_least(self.layers.max_id().max(self.id.raw()));
        self.ensure_active_layer();
        self.selection.resize(self.width, self.height);
        self.mark_all_dirty();
    }

    /// Total number of layers, including nested ones.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_document_has_one_paintable_layer() {
        let mut doc = Document::new(64, 48, "Untitled");
        assert_eq!((doc.width, doc.height), (64, 48));
        assert_eq!(doc.layer_count(), 1);
        assert!(doc.active().is_some());
        assert!(doc.paint_target().is_ok());
    }

    #[test]
    fn zero_sized_documents_are_clamped() {
        let doc = Document::new(0, 0, "tiny");
        assert_eq!((doc.width, doc.height), (1, 1));
    }

    #[test]
    fn new_layers_go_above_the_active_one() {
        let mut doc = Document::new(8, 8, "d");
        let first = doc.active_layer;
        let second = doc.add_raster_layer("Layer 2");
        assert_eq!(doc.layers.roots(), &[first, second]);
        assert_eq!(doc.active_layer, second);
    }

    #[test]
    fn painting_on_a_locked_layer_is_refused() {
        let mut doc = Document::new(8, 8, "d");
        if let Some(layer) = doc.active_mut() {
            layer.locked = true;
        }
        let err = doc.paint_target().expect_err("locked layers must refuse edits");
        assert!(err.to_string().contains("locked"), "unhelpful error: {err}");
    }

    #[test]
    fn painting_on_a_group_is_refused_with_an_explanation() {
        let mut doc = Document::empty(8, 8, "d");
        let id = doc.next_layer_id();
        doc.layers.push_top(Layer::group(id, "G")).expect("group");
        doc.active_layer = id;
        let err = doc.paint_target().expect_err("groups hold no pixels");
        assert!(err.to_string().contains("group"), "unhelpful error: {err}");
    }

    #[test]
    fn deleting_the_active_layer_picks_a_new_one() {
        let mut doc = Document::new(8, 8, "d");
        let first = doc.active_layer;
        let second = doc.add_raster_layer("Layer 2");
        doc.layers.remove_subtree(second).expect("remove");
        doc.ensure_active_layer();
        assert_eq!(doc.active_layer, first);
    }

    #[test]
    fn ids_stay_unique_after_a_reload() {
        let mut doc = Document::new(8, 8, "d");
        doc.add_raster_layer("Layer 2");
        let text = serde_json::to_string(&doc).expect("serialize");
        let mut back: Document = serde_json::from_str(&text).expect("deserialize");
        back.repair();
        let fresh = back.next_layer_id();
        assert!(
            !back.layers.contains(fresh),
            "reloaded document reissued an id in use"
        );
    }

    #[test]
    fn resizing_grows_every_layer() {
        let mut doc = Document::new(8, 8, "d");
        doc.add_raster_layer("Layer 2");
        doc.resize_canvas(16, 12);
        for layer in doc.layers.iter() {
            assert_eq!(layer.pixmap().map(|p| (p.width(), p.height())), Some((16, 12)));
        }
    }

    #[test]
    fn dirty_region_accumulates_and_clamps_to_the_canvas() {
        let mut doc = Document::new(8, 8, "d");
        doc.take_dirty();
        doc.mark_dirty(IRect::new(-4, -4, 6, 6));
        assert_eq!(doc.dirty(), IRect::new(0, 0, 2, 2));
        assert_eq!(doc.take_dirty(), IRect::new(0, 0, 2, 2));
        assert!(doc.dirty().is_empty());
    }
}
