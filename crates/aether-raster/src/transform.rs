//! Affine resampling.
//!
//! Transforms are applied by **inverse mapping**: for every destination pixel
//! we ask where it came from in the source. That avoids the holes forward
//! mapping leaves behind when scaling up, and it is the same maths the GPU
//! path will use later, so CPU and GPU results stay consistent.

use crate::pixmap::Pixmap;
use aether_core::math::{IRect, Rect, Transform2D, Vec2};

/// How source pixels are sampled during a transform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    /// Nearest neighbour — keeps pixel art crisp.
    Nearest,
    /// Bilinear — smooth, the default for photographic and painted content.
    #[default]
    Bilinear,
}

/// Apply `transform` to `src`, producing a buffer of `(out_width, out_height)`.
///
/// `transform` maps *source* coordinates to *destination* coordinates. A
/// singular transform yields an empty result rather than an error, because the
/// interactive transform tool can pass through degenerate states while the user
/// drags a handle.
pub fn transform_pixmap(
    src: &Pixmap,
    transform: &Transform2D,
    out_width: u32,
    out_height: u32,
    interpolation: Interpolation,
) -> Pixmap {
    let mut out = Pixmap::new(out_width, out_height);
    let Some(inverse) = transform.inverse() else {
        return out;
    };
    if src.is_empty() || out.is_empty() {
        return out;
    }

    // Only visit destination pixels the source can actually reach.
    let source_bounds = src.bounds().to_rect();
    let target = transform
        .transform_rect(&source_bounds)
        .to_irect_outer()
        .expanded(1)
        .intersect(&out.bounds());
    if target.is_empty() {
        return out;
    }

    for y in target.y..target.bottom() {
        for x in target.x..target.right() {
            let p = inverse.apply(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
            let c = match interpolation {
                Interpolation::Nearest => src.sample_nearest(p.x, p.y),
                Interpolation::Bilinear => src.sample_bilinear(p.x, p.y),
            };
            if c.a != 0 {
                out.set(x, y, c);
            }
        }
    }
    out
}

/// Bounds of `rect` after `transform`, rounded outwards.
pub fn transformed_bounds(rect: IRect, transform: &Transform2D) -> IRect {
    transform.transform_rect(&rect.to_rect()).to_irect_outer()
}

/// Build the transform that maps `from` onto `to` (used by the transform tool's
/// bounding-box handles).
pub fn rect_to_rect(from: IRect, to: IRect) -> Transform2D {
    if from.is_empty() {
        return Transform2D::IDENTITY;
    }
    let sx = to.width as f32 / from.width as f32;
    let sy = to.height as f32 / from.height as f32;
    Transform2D::translation(Vec2::new(-(from.x as f32), -(from.y as f32)))
        .then(&Transform2D::scale(Vec2::new(sx, sy)))
        .then(&Transform2D::translation(Vec2::new(to.x as f32, to.y as f32)))
}

/// A projective (perspective) transform: the full 3x3 homography.
///
/// An affine transform keeps parallel lines parallel, which is exactly what a
/// perspective drag must *not* do. The extra row is what lets the transform
/// tool pull one corner of a selection and have the opposite edge foreshorten.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Perspective {
    /// Row-major 3x3 matrix.
    pub m: [f32; 9],
}

impl Default for Perspective {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Perspective {
    /// The identity transform.
    pub const IDENTITY: Self = Self {
        m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
    };

    /// Build the transform mapping the four `src` corners onto the four `dst`
    /// corners, in the same order.
    ///
    /// Returns `None` when the correspondence is degenerate — three collinear
    /// corners, or a quad dragged inside out — which the transform tool can hit
    /// mid-drag.
    pub fn from_quads(src: [Vec2; 4], dst: [Vec2; 4]) -> Option<Self> {
        // Solve the classic 8x8 system for h0..h7 (h8 is fixed at 1).
        let mut a = [[0f32; 9]; 8];
        for i in 0..4 {
            let (sx, sy) = (src[i].x, src[i].y);
            let (dx, dy) = (dst[i].x, dst[i].y);
            a[i * 2] = [sx, sy, 1.0, 0.0, 0.0, 0.0, -sx * dx, -sy * dx, dx];
            a[i * 2 + 1] = [0.0, 0.0, 0.0, sx, sy, 1.0, -sx * dy, -sy * dy, dy];
        }
        let solution = solve_8x8(&mut a)?;
        Some(Self {
            m: [
                solution[0],
                solution[1],
                solution[2],
                solution[3],
                solution[4],
                solution[5],
                solution[6],
                solution[7],
                1.0,
            ],
        })
    }

    /// Promote an affine transform.
    pub fn from_affine(t: &Transform2D) -> Self {
        Self {
            m: [t.a, t.c, t.tx, t.b, t.d, t.ty, 0.0, 0.0, 1.0],
        }
    }

    /// Map a point, dividing through by the homogeneous coordinate.
    pub fn apply(&self, p: Vec2) -> Vec2 {
        let m = &self.m;
        let w = m[6] * p.x + m[7] * p.y + m[8];
        if w.abs() < 1e-9 {
            return Vec2::new(f32::INFINITY, f32::INFINITY);
        }
        Vec2::new(
            (m[0] * p.x + m[1] * p.y + m[2]) / w,
            (m[3] * p.x + m[4] * p.y + m[5]) / w,
        )
    }

    /// The inverse transform, or `None` for a singular matrix.
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.m;
        let c = [
            m[4] * m[8] - m[5] * m[7],
            m[2] * m[7] - m[1] * m[8],
            m[1] * m[5] - m[2] * m[4],
            m[5] * m[6] - m[3] * m[8],
            m[0] * m[8] - m[2] * m[6],
            m[2] * m[3] - m[0] * m[5],
            m[3] * m[7] - m[4] * m[6],
            m[1] * m[6] - m[0] * m[7],
            m[0] * m[4] - m[1] * m[3],
        ];
        let det = m[0] * c[0] + m[1] * c[3] + m[2] * c[6];
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let mut out = [0f32; 9];
        for i in 0..9 {
            out[i] = c[i] * inv;
        }
        Some(Self { m: out })
    }

    /// True when this is (numerically) the identity.
    pub fn is_identity(&self) -> bool {
        Self::IDENTITY
            .m
            .iter()
            .zip(self.m.iter())
            .all(|(a, b)| (a - b).abs() < 1e-6)
    }
}

/// Gaussian elimination with partial pivoting on an 8x9 augmented matrix.
fn solve_8x8(a: &mut [[f32; 9]; 8]) -> Option<[f32; 8]> {
    for col in 0..8 {
        // Pivot on the largest remaining magnitude for numerical stability.
        let mut pivot = col;
        for row in col + 1..8 {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-9 {
            return None;
        }
        a.swap(col, pivot);
        let divisor = a[col][col];
        for value in a[col].iter_mut() {
            *value /= divisor;
        }
        for row in 0..8 {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            if factor == 0.0 {
                continue;
            }
            let pivot_row = a[col];
            for (value, pivot) in a[row].iter_mut().zip(pivot_row.iter()).skip(col) {
                *value -= factor * pivot;
            }
        }
    }
    let mut out = [0f32; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = a[i][8];
        if !slot.is_finite() {
            return None;
        }
    }
    Some(out)
}

/// Resample `src` through a perspective transform.
///
/// Like the affine path this maps backwards from each destination pixel, so a
/// strongly foreshortened result has no holes.
pub fn perspective_pixmap(
    src: &Pixmap,
    transform: &Perspective,
    out_width: u32,
    out_height: u32,
    interpolation: Interpolation,
) -> Pixmap {
    let mut out = Pixmap::new(out_width, out_height);
    let Some(inverse) = transform.inverse() else {
        return out;
    };
    if src.is_empty() || out.is_empty() {
        return out;
    }

    // Bound the work by the transformed source rectangle.
    let corners = [
        Vec2::new(0.0, 0.0),
        Vec2::new(src.width() as f32, 0.0),
        Vec2::new(src.width() as f32, src.height() as f32),
        Vec2::new(0.0, src.height() as f32),
    ]
    .map(|p| transform.apply(p));
    if corners.iter().any(|c| !c.is_finite()) {
        return out;
    }
    let mut min = corners[0];
    let mut max = corners[0];
    for c in &corners[1..] {
        min = min.min(*c);
        max = max.max(*c);
    }
    let target = Rect { min, max }
        .to_irect_outer()
        .expanded(1)
        .intersect(&out.bounds());
    if target.is_empty() {
        return out;
    }

    for y in target.y..target.bottom() {
        for x in target.x..target.right() {
            let p = inverse.apply(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
            if !p.is_finite() {
                continue;
            }
            let c = match interpolation {
                Interpolation::Nearest => src.sample_nearest(p.x, p.y),
                Interpolation::Bilinear => src.sample_bilinear(p.x, p.y),
            };
            if c.a != 0 {
                out.set(x, y, c);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;

    #[test]
    fn identity_transform_preserves_content() {
        let mut src = Pixmap::new(8, 8);
        src.set(3, 4, Rgba8::WHITE);
        let out = transform_pixmap(&src, &Transform2D::IDENTITY, 8, 8, Interpolation::Nearest);
        assert_eq!(out.get(3, 4), Rgba8::WHITE);
        assert_eq!(out.get(0, 0), Rgba8::TRANSPARENT);
    }

    #[test]
    fn translation_moves_pixels() {
        let mut src = Pixmap::new(8, 8);
        src.set(1, 1, Rgba8::WHITE);
        let t = Transform2D::translation(Vec2::new(3.0, 2.0));
        let out = transform_pixmap(&src, &t, 8, 8, Interpolation::Nearest);
        assert_eq!(out.get(4, 3), Rgba8::WHITE);
        assert_eq!(out.get(1, 1), Rgba8::TRANSPARENT);
    }

    #[test]
    fn nearest_scaling_keeps_hard_edges() {
        let mut src = Pixmap::new(2, 2);
        src.set(0, 0, Rgba8::WHITE);
        let t = Transform2D::scale(Vec2::new(4.0, 4.0));
        let out = transform_pixmap(&src, &t, 8, 8, Interpolation::Nearest);
        assert_eq!(out.get(0, 0), Rgba8::WHITE);
        assert_eq!(out.get(3, 3), Rgba8::WHITE);
        assert_eq!(out.get(4, 4), Rgba8::TRANSPARENT);
    }

    #[test]
    fn singular_transform_yields_empty_output() {
        let src = Pixmap::filled(4, 4, Rgba8::WHITE);
        let out = transform_pixmap(
            &src,
            &Transform2D::scale(Vec2::new(0.0, 1.0)),
            4,
            4,
            Interpolation::Bilinear,
        );
        assert!(out.opaque_bounds().is_empty());
    }

    #[test]
    fn rect_to_rect_maps_corners() {
        let t = rect_to_rect(IRect::new(0, 0, 10, 10), IRect::new(5, 5, 20, 20));
        let p = t.apply(Vec2::new(0.0, 0.0));
        assert!((p.x - 5.0).abs() < 1e-4 && (p.y - 5.0).abs() < 1e-4);
        let q = t.apply(Vec2::new(10.0, 10.0));
        assert!((q.x - 25.0).abs() < 1e-4);
    }

    #[test]
    fn rotation_by_90_degrees_moves_a_corner_pixel() {
        let mut src = Pixmap::new(4, 4);
        src.set(3, 0, Rgba8::WHITE);
        let t = Transform2D::translation(Vec2::new(-2.0, -2.0))
            .then(&Transform2D::rotation(std::f32::consts::FRAC_PI_2))
            .then(&Transform2D::translation(Vec2::new(2.0, 2.0)));
        let out = transform_pixmap(&src, &t, 4, 4, Interpolation::Nearest);
        assert_eq!(
            out.get(3, 3),
            Rgba8::WHITE,
            "corner should rotate to the opposite side"
        );
    }
}

#[cfg(test)]
mod perspective_tests {
    use super::*;
    use aether_core::color::Rgba8;

    fn quad(x0: f32, y0: f32, x1: f32, y1: f32) -> [Vec2; 4] {
        [
            Vec2::new(x0, y0),
            Vec2::new(x1, y0),
            Vec2::new(x1, y1),
            Vec2::new(x0, y1),
        ]
    }

    fn approx(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() < 1e-2 && (a.y - b.y).abs() < 1e-2
    }

    #[test]
    fn an_unchanged_quad_gives_the_identity() {
        let q = quad(0.0, 0.0, 10.0, 10.0);
        let p = Perspective::from_quads(q, q).expect("solvable");
        assert!(p.is_identity(), "{:?}", p.m);
    }

    #[test]
    fn corners_land_where_they_were_dragged() {
        let src = quad(0.0, 0.0, 100.0, 100.0);
        let dst = [
            Vec2::new(20.0, 10.0),
            Vec2::new(120.0, 0.0),
            Vec2::new(90.0, 140.0),
            Vec2::new(10.0, 100.0),
        ];
        let p = Perspective::from_quads(src, dst).expect("solvable");
        for (s, d) in src.iter().zip(dst.iter()) {
            assert!(
                approx(p.apply(*s), *d),
                "{:?} -> {:?}, wanted {:?}",
                s,
                p.apply(*s),
                d
            );
        }
    }

    #[test]
    fn perspective_is_not_affine() {
        // A trapezoid narrow at the top reads as a surface receding away from
        // the viewer. The far half compresses, so the centre of the source
        // lands above the middle of the destination — an affine transform
        // would leave it exactly at y = 50.
        let src = quad(0.0, 0.0, 100.0, 100.0);
        let dst = [
            Vec2::new(25.0, 0.0),
            Vec2::new(75.0, 0.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(0.0, 100.0),
        ];
        let p = Perspective::from_quads(src, dst).expect("solvable");
        let middle = p.apply(Vec2::new(50.0, 50.0));
        assert!(middle.y < 45.0, "expected foreshortening, got y={}", middle.y);
        assert!(
            (middle.x - 50.0).abs() < 1e-3,
            "the axis of symmetry must not shift"
        );
    }

    #[test]
    fn inverse_round_trips() {
        let src = quad(0.0, 0.0, 100.0, 100.0);
        let dst = [
            Vec2::new(10.0, 5.0),
            Vec2::new(130.0, 20.0),
            Vec2::new(110.0, 150.0),
            Vec2::new(0.0, 120.0),
        ];
        let p = Perspective::from_quads(src, dst).expect("solvable");
        let inv = p.inverse().expect("invertible");
        let point = Vec2::new(37.0, 61.0);
        assert!(approx(inv.apply(p.apply(point)), point));
    }

    #[test]
    fn a_degenerate_quad_is_rejected() {
        let src = quad(0.0, 0.0, 100.0, 100.0);
        let collapsed = [Vec2::ZERO; 4];
        assert!(Perspective::from_quads(src, collapsed).is_none());
    }

    #[test]
    fn resampling_fills_the_destination_quad() {
        let src = Pixmap::filled(32, 32, Rgba8::WHITE);
        let transform = Perspective::from_quads(
            quad(0.0, 0.0, 32.0, 32.0),
            [
                Vec2::new(10.0, 10.0),
                Vec2::new(60.0, 4.0),
                Vec2::new(55.0, 60.0),
                Vec2::new(8.0, 50.0),
            ],
        )
        .expect("solvable");
        let out = perspective_pixmap(&src, &transform, 64, 64, Interpolation::Bilinear);
        assert!(out.get(32, 32).a > 0, "inside the quad");
        assert_eq!(out.get(2, 2).a, 0, "outside the quad");
    }

    #[test]
    fn an_affine_transform_promotes_cleanly() {
        let affine = Transform2D::translation(Vec2::new(5.0, -3.0));
        let p = Perspective::from_affine(&affine);
        assert!(approx(
            p.apply(Vec2::new(1.0, 1.0)),
            affine.apply(Vec2::new(1.0, 1.0))
        ));
    }
}
