//! Minimal 2D geometry.
//!
//! Aether Canvas only ever needs affine 2D transforms, so instead of pulling in
//! a general linear-algebra crate we keep a small, `serde`-friendly set of types
//! here. Everything is `Copy` and free of allocation.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// A 2D point or vector in floating point space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f32,
    /// Vertical component.
    pub y: f32,
}

/// Shorthand constructor for [`Vec2`].
#[inline]
pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

impl Vec2 {
    /// The origin.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    /// `(1, 1)`.
    pub const ONE: Self = Self { x: 1.0, y: 1.0 };

    /// Construct from components.
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Both components set to `v`.
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }

    /// Euclidean length.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Squared length (avoids the square root).
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    /// Distance to `other`.
    #[inline]
    pub fn distance(self, other: Self) -> f32 {
        (other - self).length()
    }

    /// Unit vector, or [`Vec2::ZERO`] when the vector is degenerate.
    #[inline]
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len <= f32::EPSILON {
            Self::ZERO
        } else {
            self / len
        }
    }

    /// Dot product.
    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    /// 2D cross product (z component of the 3D cross product).
    #[inline]
    pub fn cross(self, other: Self) -> f32 {
        self.x * other.y - self.y * other.x
    }

    /// Angle in radians measured from the positive x axis.
    #[inline]
    pub fn angle(self) -> f32 {
        self.y.atan2(self.x)
    }

    /// Rotate around the origin by `radians`.
    #[inline]
    pub fn rotated(self, radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }

    /// Component-wise minimum.
    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self::new(self.x.min(other.x), self.y.min(other.y))
    }

    /// Component-wise maximum.
    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self::new(self.x.max(other.x), self.y.max(other.y))
    }

    /// Linear interpolation towards `other`.
    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }

    /// Component-wise floor.
    #[inline]
    pub fn floor(self) -> Self {
        Self::new(self.x.floor(), self.y.floor())
    }

    /// True when both components are finite.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Vec2 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}
impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}
impl Sub for Vec2 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}
impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}
impl Mul<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}
impl Mul<Vec2> for Vec2 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Vec2) -> Self {
        Self::new(self.x * rhs.x, self.y * rhs.y)
    }
}
impl Div<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: f32) -> Self {
        Self::new(self.x / rhs, self.y / rhs)
    }
}
impl Neg for Vec2 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

/// An axis-aligned rectangle in floating point space.
///
/// Stored as min/max corners; `min` is inclusive, `max` exclusive in spirit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    /// Top-left corner.
    pub min: Vec2,
    /// Bottom-right corner.
    pub max: Vec2,
}

impl Rect {
    /// The empty rectangle at the origin.
    pub const ZERO: Self = Self {
        min: Vec2::ZERO,
        max: Vec2::ZERO,
    };

    /// From two corners (they are sorted).
    #[inline]
    pub fn from_corners(a: Vec2, b: Vec2) -> Self {
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }

    /// From an origin and a size.
    #[inline]
    pub fn from_min_size(min: Vec2, size: Vec2) -> Self {
        Self { min, max: min + size }
    }

    /// Width of the rectangle.
    #[inline]
    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    /// Height of the rectangle.
    #[inline]
    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// Width and height as a vector.
    #[inline]
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    /// Geometric centre.
    #[inline]
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    /// True when the rectangle has no area.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }

    /// True when `p` lies inside.
    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x < self.max.x && p.y >= self.min.y && p.y < self.max.y
    }

    /// Smallest rectangle containing both inputs.
    #[inline]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Grow the rectangle by `amount` on every side.
    #[inline]
    pub fn expanded(&self, amount: f32) -> Self {
        Self {
            min: self.min - Vec2::splat(amount),
            max: self.max + Vec2::splat(amount),
        }
    }

    /// Round outwards to whole pixels.
    #[inline]
    pub fn to_irect_outer(&self) -> IRect {
        IRect::from_bounds(
            self.min.x.floor() as i32,
            self.min.y.floor() as i32,
            self.max.x.ceil() as i32,
            self.max.y.ceil() as i32,
        )
    }
}

/// An axis-aligned rectangle in integer pixel space.
///
/// This is the workhorse type for dirty regions, tile bounds and selections:
/// `x`/`y` are inclusive, `x + width`/`y + height` exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IRect {
    /// Left edge (inclusive).
    pub x: i32,
    /// Top edge (inclusive).
    pub y: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
}

impl IRect {
    /// The empty rectangle.
    pub const EMPTY: Self = Self {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };

    /// From position and size. Negative sizes are clamped to zero.
    #[inline]
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: width.max(0),
            height: height.max(0),
        }
    }

    /// From inclusive/exclusive bounds.
    #[inline]
    pub fn from_bounds(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self::new(left, top, right - left, bottom - top)
    }

    /// A rectangle covering `(0, 0)` to `(width, height)`.
    #[inline]
    pub fn from_size(width: u32, height: u32) -> Self {
        Self::new(0, 0, width as i32, height as i32)
    }

    /// Right edge (exclusive).
    #[inline]
    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    /// Bottom edge (exclusive).
    #[inline]
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    /// Number of pixels covered.
    #[inline]
    pub fn area(&self) -> i64 {
        self.width as i64 * self.height as i64
    }

    /// True when the rectangle covers no pixels.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// True when the pixel `(px, py)` is covered.
    #[inline]
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Overlapping region, or an empty rectangle.
    #[inline]
    pub fn intersect(&self, other: &Self) -> Self {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            Self::EMPTY
        } else {
            Self::from_bounds(left, top, right, bottom)
        }
    }

    /// Smallest rectangle containing both inputs; empty inputs are ignored.
    #[inline]
    pub fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Self::from_bounds(
            self.x.min(other.x),
            self.y.min(other.y),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    /// Grow by `amount` pixels on each side (negative shrinks).
    #[inline]
    pub fn expanded(&self, amount: i32) -> Self {
        Self::from_bounds(
            self.x - amount,
            self.y - amount,
            self.right() + amount,
            self.bottom() + amount,
        )
    }

    /// Move by `(dx, dy)`.
    #[inline]
    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    /// Convert to a float rectangle.
    #[inline]
    pub fn to_rect(&self) -> Rect {
        Rect::from_min_size(
            Vec2::new(self.x as f32, self.y as f32),
            Vec2::new(self.width as f32, self.height as f32),
        )
    }
}

/// A 2D affine transform stored as the first two rows of a 3x3 matrix:
///
/// ```text
/// | a c tx |
/// | b d ty |
/// | 0 0  1 |
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    /// Row 0, column 0.
    pub a: f32,
    /// Row 1, column 0.
    pub b: f32,
    /// Row 0, column 1.
    pub c: f32,
    /// Row 1, column 1.
    pub d: f32,
    /// X translation.
    pub tx: f32,
    /// Y translation.
    pub ty: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform2D {
    /// The identity transform.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// Pure translation.
    #[inline]
    pub fn translation(t: Vec2) -> Self {
        Self {
            tx: t.x,
            ty: t.y,
            ..Self::IDENTITY
        }
    }

    /// Pure scale about the origin.
    #[inline]
    pub fn scale(s: Vec2) -> Self {
        Self {
            a: s.x,
            d: s.y,
            ..Self::IDENTITY
        }
    }

    /// Pure rotation about the origin.
    #[inline]
    pub fn rotation(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Skew (shear) given angles in radians.
    #[inline]
    pub fn skew(x_radians: f32, y_radians: f32) -> Self {
        Self {
            a: 1.0,
            b: y_radians.tan(),
            c: x_radians.tan(),
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Compose: apply `self` first, then `other`.
    #[inline]
    pub fn then(&self, other: &Self) -> Self {
        Self {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            tx: self.tx * other.a + self.ty * other.c + other.tx,
            ty: self.tx * other.b + self.ty * other.d + other.ty,
        }
    }

    /// Transform a point (translation applies).
    #[inline]
    pub fn apply(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.a * p.x + self.c * p.y + self.tx,
            self.b * p.x + self.d * p.y + self.ty,
        )
    }

    /// Transform a direction (translation ignored).
    #[inline]
    pub fn apply_vector(&self, v: Vec2) -> Vec2 {
        Vec2::new(self.a * v.x + self.c * v.y, self.b * v.x + self.d * v.y)
    }

    /// Matrix determinant.
    #[inline]
    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// Inverse transform, or `None` when the matrix is singular.
    pub fn inverse(&self) -> Option<Self> {
        let det = self.determinant();
        if det.abs() < 1e-9 {
            return None;
        }
        let inv_det = 1.0 / det;
        Some(Self {
            a: self.d * inv_det,
            b: -self.b * inv_det,
            c: -self.c * inv_det,
            d: self.a * inv_det,
            tx: (self.c * self.ty - self.d * self.tx) * inv_det,
            ty: (self.b * self.tx - self.a * self.ty) * inv_det,
        })
    }

    /// Axis-aligned bounds of the transformed rectangle.
    pub fn transform_rect(&self, rect: &Rect) -> Rect {
        let corners = [
            self.apply(rect.min),
            self.apply(Vec2::new(rect.max.x, rect.min.y)),
            self.apply(rect.max),
            self.apply(Vec2::new(rect.min.x, rect.max.y)),
        ];
        let mut min = corners[0];
        let mut max = corners[0];
        for c in &corners[1..] {
            min = min.min(*c);
            max = max.max(*c);
        }
        Rect { min, max }
    }

    /// True when this is (numerically) the identity.
    #[inline]
    pub fn is_identity(&self) -> bool {
        (self.a - 1.0).abs() < 1e-6
            && self.b.abs() < 1e-6
            && self.c.abs() < 1e-6
            && (self.d - 1.0).abs() < 1e-6
            && self.tx.abs() < 1e-6
            && self.ty.abs() < 1e-6
    }
}

/// Clamp `v` into `[lo, hi]`.
#[inline]
pub fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Linear interpolation between `a` and `b`.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn vec_ops() {
        let v = vec2(3.0, 4.0);
        assert!(approx(v.length(), 5.0));
        assert!(approx(v.normalized().length(), 1.0));
        assert_eq!(Vec2::ZERO.normalized(), Vec2::ZERO);
        assert!(approx(vec2(1.0, 0.0).rotated(std::f32::consts::FRAC_PI_2).y, 1.0));
    }

    #[test]
    fn irect_intersection_and_union() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), IRect::new(5, 5, 5, 5));
        assert_eq!(a.union(&b), IRect::new(0, 0, 15, 15));
        assert!(a.intersect(&IRect::new(100, 100, 2, 2)).is_empty());
        assert_eq!(IRect::EMPTY.union(&a), a);
    }

    #[test]
    fn transform_roundtrip() {
        let t = Transform2D::translation(vec2(10.0, -5.0))
            .then(&Transform2D::rotation(0.7))
            .then(&Transform2D::scale(vec2(2.0, 3.0)));
        let inv = t.inverse().expect("invertible");
        let p = vec2(12.5, -3.25);
        let back = inv.apply(t.apply(p));
        assert!(approx(back.x, p.x) && approx(back.y, p.y));
    }

    #[test]
    fn transform_order_is_self_then_other() {
        // Translate then scale must scale the translation as well.
        let t = Transform2D::translation(vec2(1.0, 0.0)).then(&Transform2D::scale(vec2(2.0, 2.0)));
        assert!(approx(t.apply(Vec2::ZERO).x, 2.0));
    }

    #[test]
    fn singular_matrix_has_no_inverse() {
        assert!(Transform2D::scale(vec2(0.0, 1.0)).inverse().is_none());
    }
}
