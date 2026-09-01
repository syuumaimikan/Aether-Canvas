//! Free-form deformation.
//!
//! Both the warp handles of the transform tool and the liquify brush move
//! pixels without a matrix, so they share one representation: a
//! [`DisplacementField`] — a coarse grid of offsets that says how far the
//! content under each node has moved.
//!
//! Storing offsets on a grid rather than per pixel is what keeps this
//! affordable: a 4000×4000 canvas at the default cell size is a few megabytes,
//! and sampling is a bilinear lookup. The transform tool drives a 4×4 grid from
//! its handles; liquify paints into a fine one.
//!
//! The field describes how *content* moves, so rendering samples the source at
//! `p - field(p)`.

use crate::pixmap::Pixmap;
use crate::transform::Interpolation;
use aether_core::math::{IRect, Vec2};
use serde::{Deserialize, Serialize};

/// Default grid spacing in pixels for a liquify session.
pub const DEFAULT_CELL: i32 = 4;

/// A grid of content offsets covering a rectangle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplacementField {
    rect: IRect,
    cols: usize,
    rows: usize,
    offsets: Vec<Vec2>,
}

impl DisplacementField {
    /// A field with no displacement, `cols` × `rows` nodes across `rect`.
    ///
    /// At least two nodes per axis are required for bilinear sampling.
    pub fn new(rect: IRect, cols: usize, rows: usize) -> Self {
        let cols = cols.max(2);
        let rows = rows.max(2);
        Self {
            rect,
            cols,
            rows,
            offsets: vec![Vec2::ZERO; cols * rows],
        }
    }

    /// A field whose node spacing is about `cell` pixels.
    pub fn with_cell_size(rect: IRect, cell: i32) -> Self {
        let cell = cell.max(1);
        let cols = (rect.width / cell).max(1) as usize + 1;
        let rows = (rect.height / cell).max(1) as usize + 1;
        Self::new(rect, cols, rows)
    }

    /// The area the field covers.
    pub fn rect(&self) -> IRect {
        self.rect
    }

    /// Nodes across.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Nodes down.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The offset stored at a node.
    pub fn node(&self, col: usize, row: usize) -> Vec2 {
        self.offsets
            .get(row.min(self.rows - 1) * self.cols + col.min(self.cols - 1))
            .copied()
            .unwrap_or(Vec2::ZERO)
    }

    /// Set the offset at a node.
    pub fn set_node(&mut self, col: usize, row: usize, offset: Vec2) {
        if col < self.cols && row < self.rows {
            let index = row * self.cols + col;
            self.offsets[index] = offset;
        }
    }

    /// Document position of a node when undeformed.
    pub fn node_position(&self, col: usize, row: usize) -> Vec2 {
        let fx = col as f32 / (self.cols - 1) as f32;
        let fy = row as f32 / (self.rows - 1) as f32;
        Vec2::new(
            self.rect.x as f32 + fx * self.rect.width as f32,
            self.rect.y as f32 + fy * self.rect.height as f32,
        )
    }

    /// True when nothing has been displaced.
    pub fn is_identity(&self) -> bool {
        self.offsets.iter().all(|o| o.length_squared() < 1e-8)
    }

    /// Reset every node.
    pub fn clear(&mut self) {
        self.offsets.fill(Vec2::ZERO);
    }

    /// The displacement at a document position, bilinearly interpolated.
    pub fn sample(&self, p: Vec2) -> Vec2 {
        if self.rect.width <= 0 || self.rect.height <= 0 {
            return Vec2::ZERO;
        }
        let fx =
            ((p.x - self.rect.x as f32) / self.rect.width as f32).clamp(0.0, 1.0) * (self.cols - 1) as f32;
        let fy =
            ((p.y - self.rect.y as f32) / self.rect.height as f32).clamp(0.0, 1.0) * (self.rows - 1) as f32;
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(self.cols - 1);
        let y1 = (y0 + 1).min(self.rows - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        let top = self.node(x0, y0).lerp(self.node(x1, y0), tx);
        let bottom = self.node(x0, y1).lerp(self.node(x1, y1), tx);
        top.lerp(bottom, ty)
    }

    /// Nodes within `radius` of `center`, with their falloff weight.
    fn nodes_in_range(&self, center: Vec2, radius: f32) -> Vec<(usize, f32)> {
        let radius = radius.max(1.0);
        let mut out = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let position = self.node_position(col, row);
                let d = position.distance(center);
                if d >= radius {
                    continue;
                }
                // Smooth falloff: full strength at the centre, zero at the rim.
                let t = 1.0 - d / radius;
                out.push((row * self.cols + col, t * t * (3.0 - 2.0 * t)));
            }
        }
        out
    }

    /// Drag content under the cursor along `delta`.
    pub fn push(&mut self, center: Vec2, radius: f32, delta: Vec2, strength: f32) {
        let strength = strength.clamp(0.0, 1.0);
        for (index, weight) in self.nodes_in_range(center, radius) {
            self.offsets[index] += delta * (weight * strength);
        }
    }

    /// Rotate content around the cursor. Positive `angle` turns clockwise.
    pub fn twirl(&mut self, center: Vec2, radius: f32, angle: f32, strength: f32) {
        let strength = strength.clamp(0.0, 1.0);
        for (index, weight) in self.nodes_in_range(center, radius) {
            let col = index % self.cols;
            let row = index / self.cols;
            let position = self.node_position(col, row) + self.offsets[index];
            let arm = position - center;
            let rotated = arm.rotated(angle * weight * strength);
            self.offsets[index] += rotated - arm;
        }
    }

    /// Pull content towards the cursor, or push it away with a negative amount.
    pub fn pinch(&mut self, center: Vec2, radius: f32, amount: f32, strength: f32) {
        let strength = strength.clamp(0.0, 1.0);
        for (index, weight) in self.nodes_in_range(center, radius) {
            let col = index % self.cols;
            let row = index / self.cols;
            let position = self.node_position(col, row) + self.offsets[index];
            let arm = position - center;
            self.offsets[index] -= arm * (amount * weight * strength);
        }
    }

    /// Fade the displacement back towards zero — the liquify "restore" brush.
    pub fn relax(&mut self, center: Vec2, radius: f32, strength: f32) {
        let strength = strength.clamp(0.0, 1.0);
        for (index, weight) in self.nodes_in_range(center, radius) {
            self.offsets[index] = self.offsets[index] * (1.0 - weight * strength);
        }
    }

    /// Bounding box of the pixels this field can move, grown by the largest
    /// displacement so callers know what to redraw.
    pub fn affected_bounds(&self) -> IRect {
        let max = self
            .offsets
            .iter()
            .map(|o| o.length())
            .fold(0.0f32, f32::max)
            .ceil() as i32;
        self.rect.expanded(max + 1)
    }

    /// Resample `src` through this field.
    pub fn apply(&self, src: &Pixmap, interpolation: Interpolation) -> Pixmap {
        self.apply_region(src, interpolation, None)
    }

    /// Resample only `region` of `src`.
    ///
    /// A liquify brush only disturbs pixels near the cursor, so re-rendering
    /// that neighbourhood from the original is both correct — the field is
    /// cumulative, not incremental — and cheap.
    pub fn apply_region(&self, src: &Pixmap, interpolation: Interpolation, region: Option<IRect>) -> Pixmap {
        let mut out = src.clone();
        if self.is_identity() {
            return out;
        }
        let mut area = self.affected_bounds().intersect(&src.bounds());
        if let Some(limit) = region {
            area = area.intersect(&limit);
        }
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let source = p - self.sample(p);
                let c = match interpolation {
                    Interpolation::Nearest => src.sample_nearest(source.x, source.y),
                    Interpolation::Bilinear => src.sample_bilinear(source.x, source.y),
                };
                out.set(x, y, c);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;

    fn field() -> DisplacementField {
        DisplacementField::new(IRect::from_size(64, 64), 5, 5)
    }

    #[test]
    fn a_new_field_is_the_identity() {
        let f = field();
        assert!(f.is_identity());
        assert_eq!(f.sample(Vec2::new(10.0, 10.0)), Vec2::ZERO);
    }

    #[test]
    fn node_positions_span_the_rect() {
        let f = field();
        assert_eq!(f.node_position(0, 0), Vec2::new(0.0, 0.0));
        assert_eq!(f.node_position(4, 4), Vec2::new(64.0, 64.0));
    }

    #[test]
    fn sampling_interpolates_between_nodes() {
        let mut f = field();
        f.set_node(0, 0, Vec2::new(10.0, 0.0));
        f.set_node(1, 0, Vec2::new(20.0, 0.0));
        let mid = f.sample(Vec2::new(8.0, 0.0));
        assert!((mid.x - 15.0).abs() < 0.5, "expected ~15, got {}", mid.x);
    }

    #[test]
    fn push_moves_content_and_falls_off_with_distance() {
        let mut f = field();
        f.push(Vec2::new(32.0, 32.0), 20.0, Vec2::new(8.0, 0.0), 1.0);
        let centre = f.sample(Vec2::new(32.0, 32.0)).x;
        let edge = f.sample(Vec2::new(48.0, 32.0)).x;
        assert!(centre > edge, "falloff missing: centre={centre} edge={edge}");
        assert!(
            f.sample(Vec2::new(63.0, 63.0)).length() < 1.0,
            "should not reach the corner"
        );
    }

    #[test]
    fn relax_undoes_a_push() {
        let mut f = field();
        f.push(Vec2::new(32.0, 32.0), 30.0, Vec2::new(10.0, 0.0), 1.0);
        assert!(!f.is_identity());
        for _ in 0..40 {
            f.relax(Vec2::new(32.0, 32.0), 60.0, 1.0);
        }
        assert!(
            f.sample(Vec2::new(32.0, 32.0)).length() < 0.5,
            "relax should fade the warp out"
        );
    }

    #[test]
    fn twirl_rotates_around_the_centre() {
        let mut f = field();
        f.twirl(Vec2::new(32.0, 32.0), 30.0, std::f32::consts::FRAC_PI_2, 1.0);
        let sample = f.sample(Vec2::new(42.0, 32.0));
        assert!(sample.length() > 1.0, "twirl produced no displacement");
    }

    #[test]
    fn pinch_pulls_towards_the_centre() {
        let mut f = field();
        f.pinch(Vec2::new(32.0, 32.0), 30.0, 0.5, 1.0);
        // Content to the right of the centre moves left, towards it.
        assert!(f.sample(Vec2::new(42.0, 32.0)).x < 0.0);
    }

    #[test]
    fn applying_the_identity_field_is_a_no_op() {
        let pm = Pixmap::filled(32, 32, Rgba8::WHITE);
        assert_eq!(field().apply(&pm, Interpolation::Bilinear), pm);
    }

    #[test]
    fn applying_a_push_moves_pixels() {
        let mut pm = Pixmap::new(64, 64);
        pm.fill_rect(IRect::new(28, 28, 8, 8), Rgba8::WHITE);
        let mut f = field();
        f.push(Vec2::new(32.0, 32.0), 40.0, Vec2::new(12.0, 0.0), 1.0);
        let out = f.apply(&pm, Interpolation::Bilinear);
        assert!(out.get(44, 32).a > 0, "content should have moved right");
        assert_eq!(out.get(29, 32).a, 0, "and left its old position");
    }

    #[test]
    fn a_field_round_trips_through_serde() {
        let mut f = field();
        f.push(Vec2::new(10.0, 10.0), 20.0, Vec2::new(3.0, 4.0), 1.0);
        let text = serde_json::to_string(&f).expect("serialize");
        let back: DisplacementField = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back, f);
    }

    #[test]
    fn cell_sizing_produces_a_sane_grid() {
        let f = DisplacementField::with_cell_size(IRect::from_size(256, 128), 32);
        assert_eq!((f.cols(), f.rows()), (9, 5));
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;
    use aether_core::color::Rgba8;

    #[test]
    fn region_limited_application_leaves_the_rest_alone() {
        let mut pm = Pixmap::new(64, 64);
        pm.fill_rect(IRect::new(0, 0, 64, 64), Rgba8::WHITE);
        pm.fill_rect(IRect::new(8, 8, 4, 4), Rgba8::BLACK);
        pm.fill_rect(IRect::new(48, 48, 4, 4), Rgba8::BLACK);

        let mut f = DisplacementField::new(IRect::from_size(64, 64), 9, 9);
        f.push(Vec2::new(10.0, 10.0), 30.0, Vec2::new(6.0, 0.0), 1.0);
        f.push(Vec2::new(50.0, 50.0), 30.0, Vec2::new(6.0, 0.0), 1.0);

        let limited = f.apply_region(&pm, Interpolation::Nearest, Some(IRect::new(0, 0, 32, 32)));
        assert_ne!(limited.get(14, 10), pm.get(14, 10), "inside the region it warps");
        assert_eq!(
            limited.get(54, 50),
            pm.get(54, 50),
            "outside the region it does not"
        );
    }
}
