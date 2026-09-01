//! Selections.
//!
//! A selection is a [`Mask`] the size of the canvas plus the bookkeeping that
//! makes it behave like a selection: combine modes, edge operations, and a
//! cached bounding box so tools can skip work outside it.
//!
//! "No selection" and "everything selected" are deliberately different states:
//! the first lets tools take their fast path, the second is what the user gets
//! after Select All and can then be modified.

use aether_core::math::{IRect, Vec2};
use aether_raster::fill;
use aether_raster::Mask;
use serde::{Deserialize, Serialize};

/// How a new region combines with the existing selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionMode {
    /// Discard the old selection.
    #[default]
    Replace,
    /// Union.
    Add,
    /// Difference.
    Subtract,
    /// Intersection.
    Intersect,
}

/// The active selection, if any.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    mask: Option<Mask>,
}

impl Selection {
    /// No selection: every tool operates on the whole canvas.
    pub fn none() -> Self {
        Self { mask: None }
    }

    /// True when a selection is active.
    pub fn is_active(&self) -> bool {
        self.mask.is_some()
    }

    /// The coverage mask, when active.
    pub fn mask(&self) -> Option<&Mask> {
        self.mask.as_ref()
    }

    /// Bounding box of the selection, or `None` when inactive or empty.
    pub fn bounds(&self) -> Option<IRect> {
        let bounds = self.mask.as_ref()?.coverage_bounds();
        if bounds.is_empty() {
            None
        } else {
            Some(bounds)
        }
    }

    /// Clear the selection.
    pub fn clear(&mut self) {
        self.mask = None;
    }

    /// Select the whole canvas.
    pub fn select_all(&mut self, width: u32, height: u32) {
        self.mask = Some(Mask::filled(width, height, 255));
    }

    /// Replace the selection with an explicit mask.
    pub fn set_mask(&mut self, mask: Option<Mask>) {
        self.mask = mask;
    }

    /// Combine `incoming` into the current selection.
    pub fn combine(&mut self, incoming: Mask, mode: SelectionMode) {
        match mode {
            SelectionMode::Replace => self.mask = Some(incoming),
            SelectionMode::Add => match &mut self.mask {
                Some(current) => current.union_with(&incoming),
                None => self.mask = Some(incoming),
            },
            SelectionMode::Subtract => {
                if let Some(current) = &mut self.mask {
                    current.subtract(&incoming);
                }
            }
            SelectionMode::Intersect => match &mut self.mask {
                Some(current) => current.intersect_with(&incoming),
                // Intersecting with "everything" yields the incoming region.
                None => self.mask = Some(incoming),
            },
        }
        // An empty result is the same as having no selection.
        if self.mask.as_ref().map(|m| m.is_empty()).unwrap_or(false) {
            self.mask = None;
        }
    }

    /// Select a rectangle.
    pub fn select_rect(&mut self, width: u32, height: u32, rect: IRect, mode: SelectionMode) {
        self.combine(fill::rect_mask(width, height, rect), mode);
    }

    /// Select an ellipse inscribed in `rect`.
    pub fn select_ellipse(&mut self, width: u32, height: u32, rect: IRect, mode: SelectionMode) {
        self.combine(fill::ellipse_mask(width, height, rect), mode);
    }

    /// Select a polygon or freehand lasso outline.
    pub fn select_polygon(&mut self, width: u32, height: u32, points: &[Vec2], mode: SelectionMode) {
        self.combine(fill::polygon_mask(width, height, points), mode);
    }

    /// Invert the selection over a canvas of `width` x `height`.
    pub fn invert(&mut self, width: u32, height: u32) {
        match &mut self.mask {
            Some(mask) => {
                mask.invert();
                if mask.is_empty() {
                    self.mask = None;
                }
            }
            None => {
                // Inverting "everything" gives nothing selectable; keep an
                // empty mask so the user sees the change rather than silently
                // getting the whole canvas back.
                self.mask = Some(Mask::new(width, height));
            }
        }
    }

    /// Soften the selection edge.
    pub fn feather(&mut self, radius: f32) {
        if let Some(mask) = &mut self.mask {
            mask.feather(radius);
        }
    }

    /// Grow the selection.
    pub fn expand(&mut self, pixels: i32) {
        if let Some(mask) = &mut self.mask {
            mask.expand(pixels);
        }
    }

    /// Shrink the selection.
    pub fn contract(&mut self, pixels: i32) {
        if let Some(mask) = &mut self.mask {
            mask.contract(pixels);
            if mask.is_empty() {
                self.mask = None;
            }
        }
    }

    /// Resize the selection buffer to a new canvas size.
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(mask) = &self.mask {
            self.mask = Some(mask.resized(width, height));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_selection_by_default() {
        let sel = Selection::none();
        assert!(!sel.is_active());
        assert_eq!(sel.bounds(), None);
    }

    #[test]
    fn select_all_then_clear() {
        let mut sel = Selection::none();
        sel.select_all(8, 8);
        assert!(sel.is_active());
        assert_eq!(sel.bounds(), Some(IRect::from_size(8, 8)));
        sel.clear();
        assert!(!sel.is_active());
    }

    #[test]
    fn rectangles_combine_by_mode() {
        let mut sel = Selection::none();
        sel.select_rect(16, 16, IRect::new(0, 0, 8, 8), SelectionMode::Replace);
        sel.select_rect(16, 16, IRect::new(8, 0, 8, 8), SelectionMode::Add);
        assert_eq!(sel.bounds(), Some(IRect::new(0, 0, 16, 8)));

        sel.select_rect(16, 16, IRect::new(0, 0, 8, 8), SelectionMode::Subtract);
        assert_eq!(sel.bounds(), Some(IRect::new(8, 0, 8, 8)));

        sel.select_rect(16, 16, IRect::new(8, 0, 4, 8), SelectionMode::Intersect);
        assert_eq!(sel.bounds(), Some(IRect::new(8, 0, 4, 8)));
    }

    #[test]
    fn subtracting_everything_deactivates_the_selection() {
        let mut sel = Selection::none();
        sel.select_rect(8, 8, IRect::new(0, 0, 8, 8), SelectionMode::Replace);
        sel.select_rect(8, 8, IRect::new(0, 0, 8, 8), SelectionMode::Subtract);
        assert!(!sel.is_active(), "an empty selection means no selection");
    }

    #[test]
    fn invert_swaps_inside_and_outside() {
        let mut sel = Selection::none();
        sel.select_rect(8, 8, IRect::new(0, 0, 4, 8), SelectionMode::Replace);
        sel.invert(8, 8);
        assert_eq!(sel.bounds(), Some(IRect::new(4, 0, 4, 8)));
    }

    #[test]
    fn contract_can_empty_the_selection() {
        let mut sel = Selection::none();
        sel.select_rect(16, 16, IRect::new(4, 4, 2, 2), SelectionMode::Replace);
        sel.contract(3);
        assert!(!sel.is_active());
    }

    #[test]
    fn feathering_softens_the_edge() {
        let mut sel = Selection::none();
        sel.select_rect(32, 1, IRect::new(0, 0, 16, 1), SelectionMode::Replace);
        sel.feather(3.0);
        let mask = sel.mask().expect("mask");
        let edge = mask.get(16, 0);
        assert!(edge > 0 && edge < 255, "expected a soft edge, got {edge}");
    }
}
