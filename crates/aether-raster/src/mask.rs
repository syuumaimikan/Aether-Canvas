//! Single-channel coverage buffers.
//!
//! One type serves three jobs that are all "how much of this pixel counts":
//!
//! * **layer masks** – hide parts of a layer non-destructively;
//! * **selections** – restrict every edit to a region, with soft edges;
//! * **stroke buffers** – accumulate a brush stroke's coverage so overlapping
//!   dabs inside one stroke do not darken each other.

use aether_core::math::IRect;
use aether_core::{AetherError, Result};
use serde::{Deserialize, Serialize};

/// An 8-bit coverage buffer: `0` = excluded, `255` = fully included.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mask {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl std::fmt::Debug for Mask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mask")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl Mask {
    /// An empty (fully excluded) mask.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0u8; width as usize * height as usize],
        }
    }

    /// A mask filled with `value`.
    pub fn filled(width: u32, height: u32, value: u8) -> Self {
        Self {
            width,
            height,
            data: vec![value; width as usize * height as usize],
        }
    }

    /// Wrap existing coverage bytes.
    pub fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let expected = width as usize * height as usize;
        if data.len() != expected {
            return Err(AetherError::raster(format!(
                "mask buffer is {} bytes but {width}x{height} needs {expected}",
                data.len()
            )));
        }
        Ok(Self { width, height, data })
    }

    /// Width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The full extent.
    #[inline]
    pub fn bounds(&self) -> IRect {
        IRect::from_size(self.width, self.height)
    }

    /// Raw coverage bytes.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Raw coverage bytes, mutable.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Coverage at `(x, y)`; outside the buffer reads as `0`.
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return 0;
        }
        self.data[y as usize * self.width as usize + x as usize]
    }

    /// Set coverage at `(x, y)`; outside the buffer is ignored.
    #[inline]
    pub fn set(&mut self, x: i32, y: i32, value: u8) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        let w = self.width as usize;
        self.data[y as usize * w + x as usize] = value;
    }

    /// Raise coverage at `(x, y)` to at least `value`.
    ///
    /// This is what makes a brush stroke behave like one stamp: dabs inside a
    /// stroke take the maximum coverage rather than accumulating.
    #[inline]
    pub fn max_at(&mut self, x: i32, y: i32, value: u8) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        let w = self.width as usize;
        let slot = &mut self.data[y as usize * w + x as usize];
        if value > *slot {
            *slot = value;
        }
    }

    /// Set every pixel to `value`.
    pub fn fill(&mut self, value: u8) {
        self.data.fill(value);
    }

    /// Set the pixels inside `rect` to `value`.
    pub fn fill_rect(&mut self, rect: IRect, value: u8) {
        let r = rect.intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        for y in r.y..r.bottom() {
            let start = y as usize * self.width as usize + r.x as usize;
            self.data[start..start + r.width as usize].fill(value);
        }
    }

    /// Set every pixel to `0`.
    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    /// True when no pixel has coverage.
    pub fn is_empty(&self) -> bool {
        self.data.iter().all(|v| *v == 0)
    }

    /// Invert coverage (select the complement).
    pub fn invert(&mut self) {
        for v in self.data.iter_mut() {
            *v = 255 - *v;
        }
    }

    /// Union with `other` (add to selection).
    pub fn union_with(&mut self, other: &Mask) {
        self.combine(other, |a, b| a.max(b));
    }

    /// Remove `other` (subtract from selection).
    pub fn subtract(&mut self, other: &Mask) {
        self.combine(other, |a, b| a.saturating_sub(b));
    }

    /// Keep only what both cover (intersect).
    pub fn intersect_with(&mut self, other: &Mask) {
        self.combine(other, |a, b| ((a as u16 * b as u16) / 255) as u8);
    }

    fn combine(&mut self, other: &Mask, f: impl Fn(u8, u8) -> u8) {
        for y in 0..self.height {
            for x in 0..self.width {
                let a = self.get(x as i32, y as i32);
                let b = other.get(x as i32, y as i32);
                self.set(x as i32, y as i32, f(a, b));
            }
        }
    }

    /// Bounding box of everything with non-zero coverage.
    pub fn coverage_bounds(&self) -> IRect {
        let mut left = self.width as i32;
        let mut top = self.height as i32;
        let mut right = 0i32;
        let mut bottom = 0i32;
        for y in 0..self.height as i32 {
            let mut row_has = false;
            for x in 0..self.width as i32 {
                if self.get(x, y) != 0 {
                    row_has = true;
                    left = left.min(x);
                    right = right.max(x + 1);
                }
            }
            if row_has {
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
        if right <= left || bottom <= top {
            IRect::EMPTY
        } else {
            IRect::from_bounds(left, top, right, bottom)
        }
    }

    /// Soften the mask edge with a separable box blur approximating a gaussian.
    ///
    /// `radius` is in pixels; three box passes give a close enough gaussian for
    /// feathering while staying O(n) per pass.
    pub fn feather(&mut self, radius: f32) {
        if radius <= 0.0 || self.data.is_empty() {
            return;
        }
        let r = radius.round().max(1.0) as i32;
        for _ in 0..3 {
            self.box_blur_pass(r);
        }
    }

    fn box_blur_pass(&mut self, r: i32) {
        let (w, h) = (self.width as i32, self.height as i32);
        // Horizontal
        let mut tmp = vec![0u8; self.data.len()];
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut n = 0u32;
                for dx in -r..=r {
                    let sx = x + dx;
                    if sx >= 0 && sx < w {
                        sum += self.get(sx, y) as u32;
                        n += 1;
                    }
                }
                tmp[(y * w + x) as usize] = (sum / n.max(1)) as u8;
            }
        }
        // Vertical
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut n = 0u32;
                for dy in -r..=r {
                    let sy = y + dy;
                    if sy >= 0 && sy < h {
                        sum += tmp[(sy * w + x) as usize] as u32;
                        n += 1;
                    }
                }
                self.data[(y * w + x) as usize] = (sum / n.max(1)) as u8;
            }
        }
    }

    /// Grow the covered region by `pixels` (morphological dilation).
    pub fn expand(&mut self, pixels: i32) {
        self.morph(pixels, true);
    }

    /// Shrink the covered region by `pixels` (morphological erosion).
    pub fn contract(&mut self, pixels: i32) {
        self.morph(pixels, false);
    }

    fn morph(&mut self, pixels: i32, dilate: bool) {
        if pixels <= 0 {
            return;
        }
        for _ in 0..pixels {
            let src = self.data.clone();
            let (w, h) = (self.width as i32, self.height as i32);
            for y in 0..h {
                for x in 0..w {
                    let mut v = src[(y * w + x) as usize];
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let (nx, ny) = (x + dx, y + dy);
                        let n = if nx < 0 || ny < 0 || nx >= w || ny >= h {
                            if dilate {
                                continue;
                            } else {
                                0
                            }
                        } else {
                            src[(ny * w + nx) as usize]
                        };
                        v = if dilate { v.max(n) } else { v.min(n) };
                    }
                    self.data[(y * w + x) as usize] = v;
                }
            }
        }
    }

    /// Build a mask from a pixel buffer's alpha channel.
    ///
    /// This is how a clipping group gets its shape: the base layer's alpha
    /// becomes the coverage every clipped layer above it is limited to.
    pub fn from_alpha(pixmap: &crate::pixmap::Pixmap) -> Mask {
        let mut mask = Mask::new(pixmap.width(), pixmap.height());
        for y in 0..pixmap.height() as i32 {
            for x in 0..pixmap.width() as i32 {
                mask.set(x, y, pixmap.get(x, y).a);
            }
        }
        mask
    }

    /// Multiply this mask by another, in place.
    pub fn multiply(&mut self, other: &Mask) {
        self.intersect_with(other);
    }

    /// Resize to a new size, anchoring existing coverage at the origin.
    pub fn resized(&self, width: u32, height: u32) -> Mask {
        let mut out = Mask::new(width, height);
        for y in 0..height.min(self.height) as i32 {
            for x in 0..width.min(self.width) as i32 {
                out.set(x, y, self.get(x, y));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_defaults_to_zero() {
        let m = Mask::new(4, 4);
        assert!(m.is_empty());
        assert_eq!(m.get(2, 2), 0);
        assert_eq!(m.get(-1, 0), 0);
    }

    #[test]
    fn max_at_keeps_the_strongest_dab() {
        let mut m = Mask::new(2, 2);
        m.max_at(0, 0, 100);
        m.max_at(0, 0, 40);
        assert_eq!(m.get(0, 0), 100);
        m.max_at(0, 0, 200);
        assert_eq!(m.get(0, 0), 200);
    }

    #[test]
    fn boolean_ops() {
        let mut a = Mask::new(4, 1);
        a.fill_rect(IRect::new(0, 0, 2, 1), 255);
        let mut b = Mask::new(4, 1);
        b.fill_rect(IRect::new(1, 0, 2, 1), 255);

        let mut u = a.clone();
        u.union_with(&b);
        assert_eq!((u.get(0, 0), u.get(2, 0), u.get(3, 0)), (255, 255, 0));

        let mut s = a.clone();
        s.subtract(&b);
        assert_eq!((s.get(0, 0), s.get(1, 0)), (255, 0));

        let mut i = a.clone();
        i.intersect_with(&b);
        assert_eq!((i.get(0, 0), i.get(1, 0), i.get(2, 0)), (0, 255, 0));
    }

    #[test]
    fn invert_flips_coverage() {
        let mut m = Mask::filled(2, 2, 255);
        m.invert();
        assert!(m.is_empty());
    }

    #[test]
    fn feather_softens_a_hard_edge() {
        let mut m = Mask::new(16, 1);
        m.fill_rect(IRect::new(0, 0, 8, 1), 255);
        m.feather(2.0);
        let edge = m.get(8, 0);
        assert!(edge > 0 && edge < 255, "edge should be partial, got {edge}");
        assert_eq!(m.get(15, 0), 0);
    }

    #[test]
    fn expand_and_contract_are_roughly_inverse() {
        let mut m = Mask::new(16, 16);
        m.fill_rect(IRect::new(4, 4, 8, 8), 255);
        m.expand(2);
        assert_eq!(m.get(2, 8), 255);
        m.contract(2);
        assert_eq!(m.get(2, 8), 0);
        assert_eq!(m.get(8, 8), 255);
    }

    #[test]
    fn coverage_bounds_tracks_content() {
        let mut m = Mask::new(8, 8);
        assert!(m.coverage_bounds().is_empty());
        m.set(3, 5, 200);
        assert_eq!(m.coverage_bounds(), IRect::new(3, 5, 1, 1));
    }
}

#[cfg(test)]
mod alpha_tests {
    use super::*;
    use crate::pixmap::Pixmap;
    use aether_core::color::Rgba8;

    #[test]
    fn from_alpha_copies_the_alpha_channel() {
        let mut pm = Pixmap::new(2, 1);
        pm.set(0, 0, Rgba8::new(255, 0, 0, 128));
        let mask = Mask::from_alpha(&pm);
        assert_eq!(mask.get(0, 0), 128);
        assert_eq!(mask.get(1, 0), 0);
    }
}
