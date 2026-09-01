//! The layer tree.
//!
//! Layers live in a flat map keyed by [`LayerId`]; parent/child structure is
//! expressed by the id lists inside group layers, plus a parent index kept in
//! sync for O(1) upward lookups.
//!
//! Two conventions matter and are easy to get backwards:
//!
//! * **Order is bottom-to-top.** `children[0]` is the layer furthest back.
//!   That is the order the compositor wants. The layer *panel* shows the
//!   reverse, which is what [`LayerTree::iter_ui_order`] provides.
//! * **`None` means "the root"**, so the same call sites work for top-level
//!   layers and for layers inside a group.

use crate::layer::{Layer, LayerContent};
use aether_core::{AetherError, LayerId, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A layer and all its descendants, detached from the tree.
///
/// Deleting a group has to remember the whole subtree so undo can put it back
/// exactly as it was, including ids.
#[derive(Clone, Debug, PartialEq)]
pub struct DetachedSubtree {
    /// The subtree root.
    pub root: LayerId,
    /// The root's former parent, or `None` for a top-level layer.
    pub parent: Option<LayerId>,
    /// The root's former index among its siblings.
    pub index: usize,
    /// Every layer in the subtree, root first.
    pub layers: Vec<Layer>,
}

/// Serialisable form of the tree; the parent index is rebuilt on load.
#[derive(Serialize, Deserialize)]
struct LayerTreeData {
    nodes: BTreeMap<LayerId, Layer>,
    roots: Vec<LayerId>,
}

/// Parent/child storage for layers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "LayerTreeData", into = "LayerTreeData")]
pub struct LayerTree {
    nodes: BTreeMap<LayerId, Layer>,
    roots: Vec<LayerId>,
    parents: BTreeMap<LayerId, LayerId>,
}

impl From<LayerTreeData> for LayerTree {
    fn from(data: LayerTreeData) -> Self {
        let mut tree = LayerTree {
            nodes: data.nodes,
            roots: data.roots,
            parents: BTreeMap::new(),
        };
        tree.rebuild_parents();
        tree
    }
}

impl From<LayerTree> for LayerTreeData {
    fn from(tree: LayerTree) -> Self {
        LayerTreeData {
            nodes: tree.nodes,
            roots: tree.roots,
        }
    }
}

impl LayerTree {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of layers, including nested ones.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when there are no layers at all.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// True when `id` exists.
    pub fn contains(&self, id: LayerId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Borrow a layer.
    pub fn get(&self, id: LayerId) -> Option<&Layer> {
        self.nodes.get(&id)
    }

    /// Borrow a layer mutably.
    pub fn get_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.nodes.get_mut(&id)
    }

    /// Borrow a layer, or fail with a descriptive error.
    pub fn try_get(&self, id: LayerId) -> Result<&Layer> {
        self.get(id)
            .ok_or_else(|| AetherError::document(format!("no such layer: {id}")))
    }

    /// Borrow a layer mutably, or fail with a descriptive error.
    pub fn try_get_mut(&mut self, id: LayerId) -> Result<&mut Layer> {
        self.nodes
            .get_mut(&id)
            .ok_or_else(|| AetherError::document(format!("no such layer: {id}")))
    }

    /// Every layer, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Layer> {
        self.nodes.values()
    }

    /// Every layer mutably, in id order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Layer> {
        self.nodes.values_mut()
    }

    /// Top-level layers, bottom-to-top.
    pub fn roots(&self) -> &[LayerId] {
        &self.roots
    }

    /// Children of `parent` (or the roots when `parent` is `None`), bottom-to-top.
    pub fn children_of(&self, parent: Option<LayerId>) -> &[LayerId] {
        match parent {
            None => &self.roots,
            Some(id) => self.nodes.get(&id).map(|l| l.children()).unwrap_or(&[]),
        }
    }

    /// The parent of `id`, or `None` when it is top level or missing.
    pub fn parent_of(&self, id: LayerId) -> Option<LayerId> {
        self.parents.get(&id).copied()
    }

    /// Position of `id` among its siblings.
    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        self.children_of(self.parent_of(id)).iter().position(|c| *c == id)
    }

    /// Insert `layer` under `parent` at `index` (clamped to the sibling count).
    pub fn insert(&mut self, layer: Layer, parent: Option<LayerId>, index: usize) -> Result<LayerId> {
        let id = layer.id;
        if self.nodes.contains_key(&id) {
            return Err(AetherError::document(format!("layer {id} already exists")));
        }
        if let Some(p) = parent {
            if !self.nodes.get(&p).map(|l| l.is_group()).unwrap_or(false) {
                return Err(AetherError::document(format!("{p} is not a group")));
            }
        }
        self.nodes.insert(id, layer);
        self.link(id, parent, index);
        Ok(id)
    }

    /// Add a layer at the top of the root stack.
    pub fn push_top(&mut self, layer: Layer) -> Result<LayerId> {
        let index = self.roots.len();
        self.insert(layer, None, index)
    }

    fn link(&mut self, id: LayerId, parent: Option<LayerId>, index: usize) {
        match parent {
            None => {
                let idx = index.min(self.roots.len());
                self.roots.insert(idx, id);
                self.parents.remove(&id);
            }
            Some(p) => {
                if let Some(Layer {
                    content: LayerContent::Group(g),
                    ..
                }) = self.nodes.get_mut(&p)
                {
                    let idx = index.min(g.children.len());
                    g.children.insert(idx, id);
                    self.parents.insert(id, p);
                }
            }
        }
    }

    fn unlink(&mut self, id: LayerId) -> Option<(Option<LayerId>, usize)> {
        let parent = self.parent_of(id);
        match parent {
            None => {
                let idx = self.roots.iter().position(|c| *c == id)?;
                self.roots.remove(idx);
                Some((None, idx))
            }
            Some(p) => {
                let idx = {
                    let Some(Layer {
                        content: LayerContent::Group(g),
                        ..
                    }) = self.nodes.get_mut(&p)
                    else {
                        return None;
                    };
                    let idx = g.children.iter().position(|c| *c == id)?;
                    g.children.remove(idx);
                    idx
                };
                self.parents.remove(&id);
                Some((Some(p), idx))
            }
        }
    }

    /// Ids of `id` and everything beneath it, parents before children.
    pub fn subtree_ids(&self, id: LayerId) -> Vec<LayerId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if !self.nodes.contains_key(&current) {
                continue;
            }
            out.push(current);
            if let Some(layer) = self.nodes.get(&current) {
                // Push in reverse so children come out bottom-to-top.
                for child in layer.children().iter().rev() {
                    stack.push(*child);
                }
            }
        }
        out
    }

    /// Remove `id` and its descendants, returning them for undo.
    pub fn remove_subtree(&mut self, id: LayerId) -> Result<DetachedSubtree> {
        if !self.nodes.contains_key(&id) {
            return Err(AetherError::document(format!("no such layer: {id}")));
        }
        let ids = self.subtree_ids(id);
        let (parent, index) = self
            .unlink(id)
            .ok_or_else(|| AetherError::document(format!("layer {id} is not linked into the tree")))?;
        let mut layers = Vec::with_capacity(ids.len());
        for lid in ids {
            if let Some(layer) = self.nodes.remove(&lid) {
                self.parents.remove(&lid);
                layers.push(layer);
            }
        }
        Ok(DetachedSubtree {
            root: id,
            parent,
            index,
            layers,
        })
    }

    /// Put a detached subtree back exactly where it came from.
    pub fn reattach(&mut self, subtree: DetachedSubtree) -> Result<()> {
        for layer in &subtree.layers {
            if self.nodes.contains_key(&layer.id) {
                return Err(AetherError::document(format!(
                    "layer {} already exists",
                    layer.id
                )));
            }
        }
        for layer in subtree.layers {
            self.nodes.insert(layer.id, layer);
        }
        self.link(subtree.root, subtree.parent, subtree.index);
        self.rebuild_parents();
        Ok(())
    }

    /// Move `id` to a new position.
    ///
    /// Refuses to move a group inside itself, which would orphan the subtree.
    pub fn move_node(&mut self, id: LayerId, new_parent: Option<LayerId>, index: usize) -> Result<()> {
        if !self.nodes.contains_key(&id) {
            return Err(AetherError::document(format!("no such layer: {id}")));
        }
        if let Some(p) = new_parent {
            if p == id || self.subtree_ids(id).contains(&p) {
                return Err(AetherError::document("cannot move a group into itself"));
            }
            if !self.nodes.get(&p).map(|l| l.is_group()).unwrap_or(false) {
                return Err(AetherError::document(format!("{p} is not a group")));
            }
        }
        let (old_parent, old_index) = self
            .unlink(id)
            .ok_or_else(|| AetherError::document(format!("layer {id} is not linked into the tree")))?;
        // Removing the layer shifts later siblings down by one.
        let adjusted = if old_parent == new_parent && old_index < index {
            index.saturating_sub(1)
        } else {
            index
        };
        self.link(id, new_parent, adjusted);
        Ok(())
    }

    /// Depth-first traversal in render order: bottom-to-top, children before
    /// the group's own composite step. Yields `(id, depth)`.
    pub fn iter_render_order(&self) -> Vec<(LayerId, usize)> {
        let mut out = Vec::with_capacity(self.nodes.len());
        self.walk(None, 0, &mut out);
        out
    }

    fn walk(&self, parent: Option<LayerId>, depth: usize, out: &mut Vec<(LayerId, usize)>) {
        for id in self.children_of(parent) {
            out.push((*id, depth));
            if self.nodes.get(id).map(|l| l.is_group()).unwrap_or(false) {
                self.walk(Some(*id), depth + 1, out);
            }
        }
    }

    /// Traversal in layer-panel order: top-to-bottom, group before its children.
    pub fn iter_ui_order(&self) -> Vec<(LayerId, usize)> {
        let mut out = Vec::with_capacity(self.nodes.len());
        self.walk_ui(None, 0, &mut out);
        out
    }

    fn walk_ui(&self, parent: Option<LayerId>, depth: usize, out: &mut Vec<(LayerId, usize)>) {
        for id in self.children_of(parent).iter().rev() {
            out.push((*id, depth));
            if self.nodes.get(id).map(|l| l.is_group()).unwrap_or(false) {
                self.walk_ui(Some(*id), depth + 1, out);
            }
        }
    }

    /// The sibling directly below `id`, if any.
    pub fn sibling_below(&self, id: LayerId) -> Option<LayerId> {
        let siblings = self.children_of(self.parent_of(id));
        let idx = siblings.iter().position(|c| *c == id)?;
        if idx == 0 {
            None
        } else {
            Some(siblings[idx - 1])
        }
    }

    /// The sibling directly above `id`, if any.
    pub fn sibling_above(&self, id: LayerId) -> Option<LayerId> {
        let siblings = self.children_of(self.parent_of(id));
        let idx = siblings.iter().position(|c| *c == id)?;
        siblings.get(idx + 1).copied()
    }

    /// Rebuild the parent index from the child lists.
    ///
    /// Used after deserialisation and after structural edits that touch several
    /// nodes at once.
    pub fn rebuild_parents(&mut self) {
        self.parents.clear();
        let links: Vec<(LayerId, LayerId)> = self
            .nodes
            .values()
            .flat_map(|layer| layer.children().iter().map(move |c| (*c, layer.id)))
            .collect();
        for (child, parent) in links {
            self.parents.insert(child, parent);
        }
    }

    /// Highest layer id in use, so an id generator can be resumed safely.
    pub fn max_id(&self) -> u64 {
        self.nodes.keys().map(|id| id.raw()).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::Layer;

    fn tree_with_three() -> (LayerTree, [LayerId; 3]) {
        let mut tree = LayerTree::new();
        let ids = [LayerId(1), LayerId(2), LayerId(3)];
        for (i, id) in ids.iter().enumerate() {
            tree.push_top(Layer::raster(*id, format!("L{i}"), 4, 4))
                .expect("insert");
        }
        (tree, ids)
    }

    #[test]
    fn insertion_order_is_bottom_to_top() {
        let (tree, ids) = tree_with_three();
        assert_eq!(tree.roots(), &ids);
        assert_eq!(tree.len(), 3);
        let ui: Vec<_> = tree.iter_ui_order().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ui, vec![ids[2], ids[1], ids[0]], "panel order is top-first");
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let (mut tree, ids) = tree_with_three();
        assert!(tree.push_top(Layer::raster(ids[0], "dup", 4, 4)).is_err());
    }

    #[test]
    fn nesting_tracks_parents_and_depth() {
        let mut tree = LayerTree::new();
        let group = LayerId(10);
        tree.push_top(Layer::group(group, "Folder")).expect("group");
        let child = LayerId(11);
        tree.insert(Layer::raster(child, "Inside", 4, 4), Some(group), 0)
            .expect("child");

        assert_eq!(tree.parent_of(child), Some(group));
        assert_eq!(tree.children_of(Some(group)), &[child]);
        let order = tree.iter_render_order();
        assert_eq!(order, vec![(group, 0), (child, 1)]);
    }

    #[test]
    fn cannot_insert_into_a_non_group() {
        let (mut tree, ids) = tree_with_three();
        let res = tree.insert(Layer::raster(LayerId(99), "x", 4, 4), Some(ids[0]), 0);
        assert!(res.is_err());
    }

    #[test]
    fn removing_a_group_takes_its_children() {
        let mut tree = LayerTree::new();
        let group = LayerId(1);
        tree.push_top(Layer::group(group, "G")).expect("group");
        tree.insert(Layer::raster(LayerId(2), "a", 4, 4), Some(group), 0)
            .expect("a");
        tree.insert(Layer::raster(LayerId(3), "b", 4, 4), Some(group), 1)
            .expect("b");

        let detached = tree.remove_subtree(group).expect("remove");
        assert_eq!(detached.layers.len(), 3);
        assert!(tree.is_empty());

        tree.reattach(detached).expect("reattach");
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.children_of(Some(group)), &[LayerId(2), LayerId(3)]);
        assert_eq!(tree.parent_of(LayerId(3)), Some(group));
    }

    #[test]
    fn move_reorders_within_the_same_parent() {
        let (mut tree, ids) = tree_with_three();
        // Move the bottom layer to the top.
        tree.move_node(ids[0], None, 3).expect("move");
        assert_eq!(tree.roots(), &[ids[1], ids[2], ids[0]]);
    }

    #[test]
    fn move_into_a_group_updates_the_parent() {
        let (mut tree, ids) = tree_with_three();
        let group = LayerId(50);
        tree.push_top(Layer::group(group, "G")).expect("group");
        tree.move_node(ids[0], Some(group), 0).expect("move");
        assert_eq!(tree.parent_of(ids[0]), Some(group));
        assert_eq!(tree.roots(), &[ids[1], ids[2], group]);
    }

    #[test]
    fn a_group_cannot_be_moved_into_itself() {
        let mut tree = LayerTree::new();
        let outer = LayerId(1);
        let inner = LayerId(2);
        tree.push_top(Layer::group(outer, "outer")).expect("outer");
        tree.insert(Layer::group(inner, "inner"), Some(outer), 0)
            .expect("inner");
        assert!(tree.move_node(outer, Some(inner), 0).is_err());
        assert!(tree.move_node(outer, Some(outer), 0).is_err());
    }

    #[test]
    fn siblings_are_found_in_both_directions() {
        let (tree, ids) = tree_with_three();
        assert_eq!(tree.sibling_below(ids[1]), Some(ids[0]));
        assert_eq!(tree.sibling_above(ids[1]), Some(ids[2]));
        assert_eq!(tree.sibling_below(ids[0]), None);
        assert_eq!(tree.sibling_above(ids[2]), None);
    }

    #[test]
    fn serde_round_trip_restores_parents() {
        let mut tree = LayerTree::new();
        let group = LayerId(1);
        tree.push_top(Layer::group(group, "G")).expect("group");
        tree.insert(Layer::raster(LayerId(2), "a", 2, 2), Some(group), 0)
            .expect("a");

        let text = serde_json::to_string(&tree).expect("serialize");
        let back: LayerTree = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back.parent_of(LayerId(2)), Some(group));
        assert_eq!(back.iter_render_order(), tree.iter_render_order());
        assert_eq!(back.max_id(), 2);
    }
}
