//! Canvas view transform.
//!
//! The viewport maps between **document space** (pixels of the artwork, origin
//! top-left) and **screen space** (pixels of the widget). Every tool works in
//! document space, so rotating or mirroring the canvas never changes how a
//! tool behaves — it only changes this matrix.

use aether_core::math::{Rect, Transform2D, Vec2};
use serde::{Deserialize, Serialize};

/// Smallest and largest zoom the UI allows.
pub const MIN_ZOOM: f32 = 0.01;
/// Largest zoom the UI allows.
pub const MAX_ZOOM: f32 = 64.0;

/// Pan, zoom, rotation and mirror for one canvas view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// Document point shown at the centre of the view.
    pub center: Vec2,
    /// Scale factor; `1.0` is 100%.
    pub zoom: f32,
    /// Rotation in radians.
    pub rotation: f32,
    /// Horizontal mirror, for checking a drawing's balance.
    pub mirror: bool,
    /// Size of the view widget in screen pixels.
    pub size: Vec2,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            center: Vec2::ZERO,
            zoom: 1.0,
            rotation: 0.0,
            mirror: false,
            size: Vec2::new(800.0, 600.0),
        }
    }
}

impl Viewport {
    /// A viewport showing the whole document.
    pub fn fitted(doc_width: u32, doc_height: u32, view: Vec2) -> Self {
        let mut vp = Self {
            size: view,
            ..Default::default()
        };
        vp.fit(doc_width, doc_height);
        vp
    }

    /// Document-to-screen transform.
    pub fn transform(&self) -> Transform2D {
        let mirror = if self.mirror { -1.0 } else { 1.0 };
        Transform2D::translation(-self.center)
            .then(&Transform2D::scale(Vec2::new(self.zoom * mirror, self.zoom)))
            .then(&Transform2D::rotation(self.rotation))
            .then(&Transform2D::translation(self.size * 0.5))
    }

    /// Screen-to-document transform, or `None` at a degenerate zoom.
    pub fn inverse_transform(&self) -> Option<Transform2D> {
        self.transform().inverse()
    }

    /// Map a document point to the screen.
    pub fn doc_to_screen(&self, p: Vec2) -> Vec2 {
        self.transform().apply(p)
    }

    /// Map a screen point into the document.
    ///
    /// Falls back to the document origin when the transform is degenerate,
    /// which can only happen if zoom is pushed to zero by a malformed session
    /// file.
    pub fn screen_to_doc(&self, p: Vec2) -> Vec2 {
        self.inverse_transform().map(|t| t.apply(p)).unwrap_or(Vec2::ZERO)
    }

    /// Set zoom, clamped to the supported range.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Multiply the zoom while keeping `anchor` (a screen point) fixed.
    ///
    /// This is what makes wheel-zoom feel right: the pixel under the cursor
    /// stays under the cursor.
    pub fn zoom_at(&mut self, anchor: Vec2, factor: f32) {
        let before = self.screen_to_doc(anchor);
        self.set_zoom(self.zoom * factor);
        let after = self.screen_to_doc(anchor);
        self.center += before - after;
    }

    /// Pan by a screen-space delta.
    pub fn pan_by_screen(&mut self, delta: Vec2) {
        let Some(inv) = self.inverse_transform() else {
            return;
        };
        // Only the linear part applies to a delta.
        self.center -= inv.apply_vector(delta);
    }

    /// Frame the whole document with a small margin.
    pub fn fit(&mut self, doc_width: u32, doc_height: u32) {
        let doc = Vec2::new(doc_width.max(1) as f32, doc_height.max(1) as f32);
        if self.size.x <= 1.0 || self.size.y <= 1.0 {
            return;
        }
        let scale = (self.size.x / doc.x).min(self.size.y / doc.y) * 0.92;
        self.rotation = 0.0;
        self.set_zoom(scale);
        self.center = doc * 0.5;
    }

    /// Reset zoom to 100% and centre the document.
    pub fn reset(&mut self, doc_width: u32, doc_height: u32) {
        self.zoom = 1.0;
        self.rotation = 0.0;
        self.mirror = false;
        self.center = Vec2::new(doc_width as f32 * 0.5, doc_height as f32 * 0.5);
    }

    /// The document-space rectangle currently visible, rounded outwards.
    pub fn visible_document_rect(&self) -> Rect {
        let Some(inv) = self.inverse_transform() else {
            return Rect::ZERO;
        };
        inv.transform_rect(&Rect::from_min_size(Vec2::ZERO, self.size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_centre_of_the_view_shows_the_centre_point() {
        let vp = Viewport {
            center: Vec2::new(50.0, 40.0),
            size: Vec2::new(800.0, 600.0),
            ..Default::default()
        };
        let screen = vp.doc_to_screen(Vec2::new(50.0, 40.0));
        assert!(approx(screen.x, 400.0) && approx(screen.y, 300.0));
    }

    #[test]
    fn screen_and_document_conversions_are_inverses() {
        let vp = Viewport {
            center: Vec2::new(37.0, -12.0),
            zoom: 2.5,
            rotation: 0.6,
            mirror: true,
            size: Vec2::new(1024.0, 768.0),
        };
        let p = Vec2::new(123.0, 45.0);
        let back = vp.screen_to_doc(vp.doc_to_screen(p));
        assert!(approx(back.x, p.x) && approx(back.y, p.y), "{back:?} != {p:?}");
    }

    #[test]
    fn zoom_is_clamped() {
        let mut vp = Viewport::default();
        vp.set_zoom(1000.0);
        assert_eq!(vp.zoom, MAX_ZOOM);
        vp.set_zoom(0.0);
        assert_eq!(vp.zoom, MIN_ZOOM);
    }

    #[test]
    fn zooming_keeps_the_anchor_pixel_under_the_cursor() {
        let mut vp = Viewport {
            center: Vec2::new(100.0, 100.0),
            size: Vec2::new(800.0, 600.0),
            ..Default::default()
        };
        let anchor = Vec2::new(200.0, 150.0);
        let before = vp.screen_to_doc(anchor);
        vp.zoom_at(anchor, 2.0);
        let after = vp.screen_to_doc(anchor);
        assert!(
            approx(before.x, after.x) && approx(before.y, after.y),
            "{before:?} vs {after:?}"
        );
    }

    #[test]
    fn fit_frames_the_whole_document() {
        let mut vp = Viewport {
            size: Vec2::new(800.0, 600.0),
            ..Default::default()
        };
        vp.fit(1600, 1200);
        assert!(vp.zoom < 0.5 && vp.zoom > 0.4, "unexpected zoom {}", vp.zoom);
        let visible = vp.visible_document_rect();
        assert!(
            visible.min.x < 0.0 && visible.max.x > 1600.0,
            "document not fully visible: {visible:?}"
        );
    }

    #[test]
    fn panning_moves_the_view_the_expected_amount() {
        let mut vp = Viewport {
            center: Vec2::new(100.0, 100.0),
            zoom: 2.0,
            ..Default::default()
        };
        vp.pan_by_screen(Vec2::new(20.0, 0.0));
        assert!(approx(vp.center.x, 90.0), "got {}", vp.center.x);
    }

    #[test]
    fn mirroring_flips_horizontally_only() {
        let vp = Viewport {
            center: Vec2::new(0.0, 0.0),
            mirror: true,
            size: Vec2::new(100.0, 100.0),
            ..Default::default()
        };
        let p = vp.doc_to_screen(Vec2::new(10.0, 10.0));
        assert!(approx(p.x, 40.0), "x should mirror, got {}", p.x);
        assert!(approx(p.y, 60.0), "y should not mirror, got {}", p.y);
    }
}
