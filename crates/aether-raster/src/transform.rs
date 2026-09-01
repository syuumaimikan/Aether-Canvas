//! Affine resampling.
//!
//! Transforms are applied by **inverse mapping**: for every destination pixel
//! we ask where it came from in the source. That avoids the holes forward
//! mapping leaves behind when scaling up, and it is the same maths the GPU
//! path will use later, so CPU and GPU results stay consistent.

use crate::pixmap::Pixmap;
use aether_core::math::{IRect, Transform2D, Vec2};

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
