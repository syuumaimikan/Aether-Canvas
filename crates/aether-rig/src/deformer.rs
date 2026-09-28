//! Deformers.
//!
//! A deformer is a bendable space. Meshes and other deformers placed inside
//! one are carried along with it, which is how a rig is built in layers: a
//! warp over the whole face turns the head, rotation pivots inside it swing
//! the eyebrows, and the meshes at the bottom only need small keyforms of
//! their own.
//!
//! * A **warp** deformer is a lattice of `(cols + 1) × (rows + 1)` points over
//!   a rest rectangle. Points inside the rectangle move with the lattice,
//!   interpolated bilinearly or — in smooth mode — with a bicubic Catmull-Rom
//!   patch, which bends curved features (a cheek, a sleeve) without the
//!   visible creases a bilinear lattice leaves along its cell lines. Points
//!   outside follow the nearest edge.
//! * A **rotation** deformer is a pivot with an angle, a scale and a
//!   translation — cheap and exact for things that swing.
//!
//! All rest geometry is in document space. Each deformer maps a rest-space
//! point to its deformed position; nesting composes those maps from the inside
//! out.

use crate::flat;
use crate::keyform::{Blend, BlendShape, KeyformGrid, ParamSource};
use crate::NodeRef;
use aether_core::math::{Rect, Vec2};
use aether_core::{AetherError, DeformerId, Result};
use serde::{Deserialize, Serialize};

fn one() -> f32 {
    1.0
}

/// A warp lattice at one keyform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpForm {
    /// Offset of each lattice point from rest, row-major.
    #[serde(with = "flat")]
    pub offsets: Vec<Vec2>,
    /// Opacity passed on to everything inside.
    #[serde(default = "one")]
    pub opacity: f32,
}

impl WarpForm {
    /// The undeformed lattice of `points` points.
    pub fn rest(points: usize) -> Self {
        Self {
            offsets: vec![Vec2::ZERO; points],
            opacity: 1.0,
        }
    }
}

impl Blend for WarpForm {
    fn zeroed(&self) -> Self {
        Self {
            offsets: vec![Vec2::ZERO; self.offsets.len()],
            opacity: 0.0,
        }
    }

    fn add_scaled(&mut self, other: &Self, weight: f32) {
        for (a, b) in self.offsets.iter_mut().zip(&other.offsets) {
            *a += *b * weight;
        }
        self.opacity += other.opacity * weight;
    }
}

/// A rotation pivot at one keyform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RotationForm {
    /// Movement of the pivot from its rest position.
    pub offset: Vec2,
    /// Rotation in degrees (clockwise on screen).
    pub angle: f32,
    /// Uniform scale about the pivot.
    #[serde(default = "one")]
    pub scale: f32,
    /// Opacity passed on to everything inside.
    #[serde(default = "one")]
    pub opacity: f32,
}

impl RotationForm {
    /// The identity.
    pub fn rest() -> Self {
        Self {
            offset: Vec2::ZERO,
            angle: 0.0,
            scale: 1.0,
            opacity: 1.0,
        }
    }
}

impl Blend for RotationForm {
    fn zeroed(&self) -> Self {
        Self {
            offset: Vec2::ZERO,
            angle: 0.0,
            scale: 0.0,
            opacity: 0.0,
        }
    }

    fn add_scaled(&mut self, other: &Self, weight: f32) {
        self.offset += other.offset * weight;
        self.angle += other.angle * weight;
        self.scale += other.scale * weight;
        self.opacity += other.opacity * weight;
    }
}

/// A lattice deformer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpDeformer {
    /// Rest rectangle, in document space.
    pub rect: Rect,
    /// Cells across (lattice points across = `cols + 1`).
    pub cols: usize,
    /// Cells down.
    pub rows: usize,
    /// Bicubic instead of bilinear interpolation inside cells.
    #[serde(default)]
    pub smooth: bool,
    /// Lattice keyforms.
    pub keyforms: KeyformGrid<WarpForm>,
    /// Additive corrections.
    #[serde(default)]
    pub blend_shapes: Vec<BlendShape<WarpForm>>,
}

impl WarpDeformer {
    /// A rest lattice over `rect`.
    pub fn new(rect: Rect, cols: usize, rows: usize) -> Self {
        let cols = cols.clamp(1, 64);
        let rows = rows.clamp(1, 64);
        Self {
            rect,
            cols,
            rows,
            smooth: true,
            keyforms: KeyformGrid::constant(WarpForm::rest((cols + 1) * (rows + 1))),
            blend_shapes: Vec::new(),
        }
    }

    /// Lattice points across.
    pub fn points_x(&self) -> usize {
        self.cols + 1
    }

    /// Lattice points down.
    pub fn points_y(&self) -> usize {
        self.rows + 1
    }

    /// Total lattice points.
    pub fn point_count(&self) -> usize {
        self.points_x() * self.points_y()
    }

    /// Rest position of lattice point `(i, j)`.
    pub fn rest_point(&self, i: usize, j: usize) -> Vec2 {
        let size = self.rect.size();
        Vec2::new(
            self.rect.min.x + size.x * i as f32 / self.cols as f32,
            self.rect.min.y + size.y * j as f32 / self.rows as f32,
        )
    }

    /// Every rest lattice point, row-major.
    pub fn rest_points(&self) -> Vec<Vec2> {
        let mut out = Vec::with_capacity(self.point_count());
        for j in 0..self.points_y() {
            for i in 0..self.points_x() {
                out.push(self.rest_point(i, j));
            }
        }
        out
    }

    /// Check the lattice and keyform sizes.
    pub fn validate(&self) -> Result<()> {
        if !(self.rect.width() > 0.0 && self.rect.height() > 0.0) {
            return Err(AetherError::rig("a warp deformer needs a non-empty rectangle"));
        }
        self.keyforms.validate()?;
        let n = self.point_count();
        let bad = self.keyforms.forms.iter().any(|f| f.offsets.len() != n)
            || self
                .blend_shapes
                .iter()
                .any(|s| s.grid.validate().is_err() || s.grid.forms.iter().any(|f| f.offsets.len() != n));
        if bad {
            return Err(AetherError::rig("a warp keyform does not match its lattice size"));
        }
        Ok(())
    }

    /// Change the lattice resolution, resampling every keyform so the current
    /// deformation is preserved as closely as the new lattice allows.
    pub fn resize_lattice(&mut self, cols: usize, rows: usize) {
        let cols = cols.clamp(1, 64);
        let rows = rows.clamp(1, 64);
        if cols == self.cols && rows == self.rows {
            return;
        }
        let old = self.clone();
        self.cols = cols;
        self.rows = rows;
        let targets = self.rest_points();
        let resample = |form: &WarpForm| -> WarpForm {
            let state = WarpState {
                rect: old.rect,
                cols: old.cols,
                rows: old.rows,
                smooth: old.smooth,
                offsets: form.offsets.clone(),
            };
            WarpForm {
                offsets: targets.iter().map(|p| state.displacement(*p)).collect(),
                opacity: form.opacity,
            }
        };
        self.keyforms.for_each_form(|f| *f = resample(f));
        for shape in &mut self.blend_shapes {
            shape.grid.for_each_form(|f| *f = resample(f));
        }
    }
}

/// A pivot deformer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RotationDeformer {
    /// Rest pivot, in document space.
    pub origin: Vec2,
    /// Keyforms.
    pub keyforms: KeyformGrid<RotationForm>,
    /// Additive corrections.
    #[serde(default)]
    pub blend_shapes: Vec<BlendShape<RotationForm>>,
}

impl RotationDeformer {
    /// A pivot at `origin` with no rotation.
    pub fn new(origin: Vec2) -> Self {
        Self {
            origin,
            keyforms: KeyformGrid::constant(RotationForm::rest()),
            blend_shapes: Vec::new(),
        }
    }
}

/// The two kinds of deformer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DeformerKind {
    /// A lattice.
    Warp(WarpDeformer),
    /// A pivot.
    Rotation(RotationDeformer),
}

/// A named deformer in the rig hierarchy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Deformer {
    /// Stable identity.
    pub id: DeformerId,
    /// Display name.
    pub name: String,
    /// The deformer or bone this one lives inside.
    #[serde(default)]
    pub parent: Option<NodeRef>,
    /// The deformation itself.
    pub kind: DeformerKind,
}

impl Deformer {
    /// A warp deformer.
    pub fn warp(id: DeformerId, name: impl Into<String>, rect: Rect, cols: usize, rows: usize) -> Self {
        Self {
            id,
            name: name.into(),
            parent: None,
            kind: DeformerKind::Warp(WarpDeformer::new(rect, cols, rows)),
        }
    }

    /// A rotation deformer.
    pub fn rotation(id: DeformerId, name: impl Into<String>, origin: Vec2) -> Self {
        Self {
            id,
            name: name.into(),
            parent: None,
            kind: DeformerKind::Rotation(RotationDeformer::new(origin)),
        }
    }

    /// Check internal consistency.
    pub fn validate(&self) -> Result<()> {
        match &self.kind {
            DeformerKind::Warp(w) => w.validate(),
            DeformerKind::Rotation(r) => {
                r.keyforms.validate()?;
                for s in &r.blend_shapes {
                    s.grid.validate()?;
                }
                Ok(())
            }
        }
    }

    /// Every parameter this deformer's keyforms or blend shapes use.
    pub fn params(&self) -> Vec<aether_core::ParameterId> {
        let mut out: Vec<_> = match &self.kind {
            DeformerKind::Warp(w) => w
                .keyforms
                .axes
                .iter()
                .map(|a| a.param)
                .chain(
                    w.blend_shapes
                        .iter()
                        .flat_map(|s| s.grid.axes.iter().map(|a| a.param)),
                )
                .collect(),
            DeformerKind::Rotation(r) => r
                .keyforms
                .axes
                .iter()
                .map(|a| a.param)
                .chain(
                    r.blend_shapes
                        .iter()
                        .flat_map(|s| s.grid.axes.iter().map(|a| a.param)),
                )
                .collect(),
        };
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Evaluate this deformer's own state (ignoring its parents).
    pub fn evaluate(&self, source: &dyn ParamSource) -> DeformerState {
        match &self.kind {
            DeformerKind::Warp(w) => {
                let mut form = w.keyforms.evaluate(source);
                for shape in &w.blend_shapes {
                    shape.accumulate(&mut form, source);
                }
                DeformerState {
                    opacity: form.opacity.clamp(0.0, 1.0),
                    map: DeformerMap::Warp(WarpState {
                        rect: w.rect,
                        cols: w.cols,
                        rows: w.rows,
                        smooth: w.smooth,
                        offsets: form.offsets,
                    }),
                }
            }
            DeformerKind::Rotation(r) => {
                let mut form = r.keyforms.evaluate(source);
                for shape in &r.blend_shapes {
                    shape.accumulate(&mut form, source);
                }
                DeformerState {
                    opacity: form.opacity.clamp(0.0, 1.0),
                    map: DeformerMap::Rotation(RotationState {
                        origin: r.origin,
                        offset: form.offset,
                        angle: form.angle.to_radians(),
                        scale: form.scale,
                    }),
                }
            }
        }
    }
}

/// A deformer evaluated at the current parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct DeformerState {
    /// Opacity passed down to children (before parents' opacity).
    pub opacity: f32,
    /// The point mapping.
    pub map: DeformerMap,
}

/// An evaluated point mapping.
#[derive(Clone, Debug, PartialEq)]
pub enum DeformerMap {
    /// Lattice mapping.
    Warp(WarpState),
    /// Pivot mapping.
    Rotation(RotationState),
}

impl DeformerMap {
    /// Map a rest-space point through this deformer alone.
    pub fn apply(&self, p: Vec2) -> Vec2 {
        match self {
            DeformerMap::Warp(w) => p + w.displacement(p),
            DeformerMap::Rotation(r) => r.apply(p),
        }
    }
}

/// An evaluated warp lattice.
#[derive(Clone, Debug, PartialEq)]
pub struct WarpState {
    /// Rest rectangle.
    pub rect: Rect,
    /// Cells across.
    pub cols: usize,
    /// Cells down.
    pub rows: usize,
    /// Bicubic interpolation.
    pub smooth: bool,
    /// Lattice offsets, row-major.
    pub offsets: Vec<Vec2>,
}

impl WarpState {
    fn offset(&self, i: isize, j: isize) -> Vec2 {
        let px = self.cols as isize + 1;
        let py = self.rows as isize + 1;
        let i = i.clamp(0, px - 1) as usize;
        let j = j.clamp(0, py - 1) as usize;
        self.offsets
            .get(j * px as usize + i)
            .copied()
            .unwrap_or(Vec2::ZERO)
    }

    /// How far the content at rest position `p` moves.
    pub fn displacement(&self, p: Vec2) -> Vec2 {
        let size = self.rect.size();
        if size.x <= 0.0 || size.y <= 0.0 || self.offsets.is_empty() {
            return Vec2::ZERO;
        }
        let gx = ((p.x - self.rect.min.x) / size.x).clamp(0.0, 1.0) * self.cols as f32;
        let gy = ((p.y - self.rect.min.y) / size.y).clamp(0.0, 1.0) * self.rows as f32;
        let i = (gx.floor() as isize).min(self.cols as isize - 1).max(0);
        let j = (gy.floor() as isize).min(self.rows as isize - 1).max(0);
        let tx = gx - i as f32;
        let ty = gy - j as f32;
        if self.smooth {
            let wx = catmull_rom(tx);
            let wy = catmull_rom(ty);
            let mut acc = Vec2::ZERO;
            for (dj, wyj) in wy.iter().enumerate() {
                for (di, wxi) in wx.iter().enumerate() {
                    acc += self.offset(i + di as isize - 1, j + dj as isize - 1) * (wxi * wyj);
                }
            }
            acc
        } else {
            let a = self.offset(i, j);
            let b = self.offset(i + 1, j);
            let c = self.offset(i, j + 1);
            let d = self.offset(i + 1, j + 1);
            let top = a * (1.0 - tx) + b * tx;
            let bottom = c * (1.0 - tx) + d * tx;
            top * (1.0 - ty) + bottom * ty
        }
    }

    /// Deformed lattice points (before any parent), row-major.
    pub fn points(&self) -> Vec<Vec2> {
        let mut out = Vec::with_capacity(self.offsets.len());
        let size = self.rect.size();
        for j in 0..=self.rows {
            for i in 0..=self.cols {
                let rest = Vec2::new(
                    self.rect.min.x + size.x * i as f32 / self.cols as f32,
                    self.rect.min.y + size.y * j as f32 / self.rows as f32,
                );
                out.push(rest + self.offset(i as isize, j as isize));
            }
        }
        out
    }
}

/// Catmull-Rom weights for the four neighbouring samples.
fn catmull_rom(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

/// An evaluated pivot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotationState {
    /// Rest pivot.
    pub origin: Vec2,
    /// Pivot movement.
    pub offset: Vec2,
    /// Angle in radians.
    pub angle: f32,
    /// Uniform scale.
    pub scale: f32,
}

impl RotationState {
    /// Map a rest-space point.
    pub fn apply(&self, p: Vec2) -> Vec2 {
        self.origin + self.offset + (p - self.origin).rotated(self.angle) * self.scale
    }

    /// Where the pivot ends up.
    pub fn pivot(&self) -> Vec2 {
        self.origin + self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyform::{AxisInput, KeyAxis};
    use aether_core::math::vec2;
    use aether_core::ParameterId;

    fn source(value: f32) -> impl ParamSource {
        move |_: ParameterId| Some(AxisInput { value, cycle: None })
    }

    fn rect() -> Rect {
        Rect::from_corners(vec2(0.0, 0.0), vec2(100.0, 100.0))
    }

    #[test]
    fn an_undeformed_warp_is_the_identity() {
        let d = Deformer::warp(DeformerId(1), "w", rect(), 4, 4);
        let state = d.evaluate(&source(0.0));
        for p in [vec2(0.0, 0.0), vec2(33.0, 71.0), vec2(150.0, -20.0)] {
            assert_eq!(state.map.apply(p), p);
        }
    }

    #[test]
    fn warp_lattice_offsets_carry_points_along() {
        let mut w = WarpDeformer::new(rect(), 2, 2);
        w.smooth = false;
        for o in &mut w.keyforms.forms[0].offsets {
            *o = vec2(5.0, -3.0);
        }
        let state = WarpState {
            rect: w.rect,
            cols: w.cols,
            rows: w.rows,
            smooth: false,
            offsets: w.keyforms.forms[0].offsets.clone(),
        };
        assert_eq!(state.displacement(vec2(10.0, 90.0)), vec2(5.0, -3.0));
        // Points outside follow the nearest edge.
        assert_eq!(state.displacement(vec2(-50.0, 50.0)), vec2(5.0, -3.0));
    }

    #[test]
    fn smooth_warps_interpolate_a_single_point_softly() {
        let mut w = WarpDeformer::new(rect(), 4, 4);
        // Lift only the centre lattice point.
        let centre = 2 * 5 + 2;
        w.keyforms.forms[0].offsets[centre] = vec2(0.0, -10.0);
        let smooth = WarpState {
            rect: w.rect,
            cols: 4,
            rows: 4,
            smooth: true,
            offsets: w.keyforms.forms[0].offsets.clone(),
        };
        let linear = WarpState {
            smooth: false,
            ..smooth.clone()
        };
        // Both pass exactly through the lattice point…
        assert!((smooth.displacement(vec2(50.0, 50.0)).y + 10.0).abs() < 1e-4);
        assert!((linear.displacement(vec2(50.0, 50.0)).y + 10.0).abs() < 1e-4);
        // …but the smooth one has zero slope at the peak.
        let h = 0.5;
        let slope = |s: &WarpState| {
            (s.displacement(vec2(50.0 + h, 50.0)).y - s.displacement(vec2(50.0 - h, 50.0)).y) / (2.0 * h)
        };
        assert!(slope(&smooth).abs() < 1e-3);
        let left = linear.displacement(vec2(50.0 - h, 50.0)).y;
        assert!(left > -10.0, "linear falls off immediately");
    }

    #[test]
    fn rotation_deformers_rotate_about_the_pivot() {
        let mut d = RotationDeformer::new(vec2(10.0, 10.0));
        d.keyforms
            .add_axis(KeyAxis::new(ParameterId(1), [0.0, 1.0]).expect("axis"))
            .expect("axis");
        d.keyforms.forms[1].angle = 90.0;
        let deformer = Deformer {
            id: DeformerId(1),
            name: "r".into(),
            parent: None,
            kind: DeformerKind::Rotation(d),
        };
        let state = deformer.evaluate(&source(1.0));
        let p = state.map.apply(vec2(20.0, 10.0));
        assert!(
            (p.x - 10.0).abs() < 1e-4 && (p.y - 20.0).abs() < 1e-4,
            "got {p:?}"
        );
        let half = deformer.evaluate(&source(0.5));
        let q = half.map.apply(vec2(20.0, 10.0));
        assert!(
            (q.distance(vec2(10.0, 10.0)) - 10.0).abs() < 1e-3,
            "rotation keeps the radius"
        );
    }

    #[test]
    fn resizing_a_lattice_preserves_a_uniform_offset() {
        let mut w = WarpDeformer::new(rect(), 2, 2);
        for o in &mut w.keyforms.forms[0].offsets {
            *o = vec2(3.0, 4.0);
        }
        w.resize_lattice(5, 3);
        w.validate().expect("valid");
        assert_eq!(w.point_count(), 24);
        assert!(w.keyforms.forms[0]
            .offsets
            .iter()
            .all(|o| (o.x - 3.0).abs() < 1e-4 && (o.y - 4.0).abs() < 1e-4));
    }
}
