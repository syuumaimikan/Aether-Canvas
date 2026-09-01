//! The RGBA8 pixel buffer.

use aether_core::color::{Rgba, Rgba8};
use aether_core::math::IRect;
use aether_core::{AetherError, Result};
use serde::{Deserialize, Serialize};

/// Bytes per pixel in a [`Pixmap`].
pub const BYTES_PER_PIXEL: usize = 4;

/// A rectangular 8-bit RGBA image with **straight** (non-premultiplied) alpha.
///
/// Rows are tightly packed: pixel `(x, y)` starts at
/// `(y * width + x) * BYTES_PER_PIXEL`. This is the layout `image`, `png` and
/// `wgpu` all expect, so exporting and texture upload are memcpy-cheap.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pixmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl std::fmt::Debug for Pixmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pixmap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl Pixmap {
    /// A fully transparent buffer.
    ///
    /// Zero-sized buffers are legal and behave as no-ops for every drawing
    /// operation, which keeps callers free of edge-case checks.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0u8; width as usize * height as usize * BYTES_PER_PIXEL],
        }
    }

    /// A buffer filled with a single colour.
    pub fn filled(width: u32, height: u32, color: Rgba8) -> Self {
        let mut pm = Self::new(width, height);
        pm.fill(color);
        pm
    }

    /// Wrap existing bytes.
    ///
    /// Returns [`AetherError::Raster`] when `data` is not exactly
    /// `width * height * 4` bytes long.
    pub fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let expected = width as usize * height as usize * BYTES_PER_PIXEL;
        if data.len() != expected {
            return Err(AetherError::raster(format!(
                "pixel buffer is {} bytes but {width}x{height} needs {expected}",
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

    /// True when the buffer has no pixels.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The full extent of the buffer.
    #[inline]
    pub fn bounds(&self) -> IRect {
        IRect::from_size(self.width, self.height)
    }

    /// Raw bytes.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Raw bytes, mutable.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Consume the buffer and return its bytes.
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Byte offset of pixel `(x, y)` without bounds checking.
    #[inline]
    fn offset(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width as usize + x as usize) * BYTES_PER_PIXEL
    }

    /// Read a pixel; out-of-range coordinates read as transparent.
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Rgba8 {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return Rgba8::TRANSPARENT;
        }
        let o = self.offset(x as u32, y as u32);
        Rgba8::new(self.data[o], self.data[o + 1], self.data[o + 2], self.data[o + 3])
    }

    /// Write a pixel; out-of-range coordinates are ignored.
    #[inline]
    pub fn set(&mut self, x: i32, y: i32, color: Rgba8) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        let o = self.offset(x as u32, y as u32);
        self.data[o] = color.r;
        self.data[o + 1] = color.g;
        self.data[o + 2] = color.b;
        self.data[o + 3] = color.a;
    }

    /// One row of pixels as bytes.
    #[inline]
    pub fn row(&self, y: u32) -> &[u8] {
        let start = self.offset(0, y);
        &self.data[start..start + self.width as usize * BYTES_PER_PIXEL]
    }

    /// One row of pixels as mutable bytes.
    #[inline]
    pub fn row_mut(&mut self, y: u32) -> &mut [u8] {
        let start = self.offset(0, y);
        let len = self.width as usize * BYTES_PER_PIXEL;
        &mut self.data[start..start + len]
    }

    /// Overwrite every pixel with `color`.
    pub fn fill(&mut self, color: Rgba8) {
        let px = color.to_array();
        for chunk in self.data.chunks_exact_mut(BYTES_PER_PIXEL) {
            chunk.copy_from_slice(&px);
        }
    }

    /// Reset every pixel to transparent.
    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Overwrite the pixels inside `rect` (clipped to the buffer) with `color`.
    pub fn fill_rect(&mut self, rect: IRect, color: Rgba8) {
        let r = rect.intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        let px = color.to_array();
        for y in r.y..r.bottom() {
            let start = self.offset(r.x as u32, y as u32);
            let end = start + r.width as usize * BYTES_PER_PIXEL;
            for chunk in self.data[start..end].chunks_exact_mut(BYTES_PER_PIXEL) {
                chunk.copy_from_slice(&px);
            }
        }
    }

    /// Copy `rect` out into a new buffer; areas outside the source are transparent.
    pub fn copy_rect(&self, rect: IRect) -> Pixmap {
        let mut out = Pixmap::new(rect.width.max(0) as u32, rect.height.max(0) as u32);
        if out.is_empty() {
            return out;
        }
        let src = rect.intersect(&self.bounds());
        if src.is_empty() {
            return out;
        }
        for y in src.y..src.bottom() {
            let so = self.offset(src.x as u32, y as u32);
            let dx = (src.x - rect.x) as u32;
            let dy = (y - rect.y) as u32;
            let dof = out.offset(dx, dy);
            let len = src.width as usize * BYTES_PER_PIXEL;
            out.data[dof..dof + len].copy_from_slice(&self.data[so..so + len]);
        }
        out
    }

    /// Paste `src` at `(x, y)`, replacing destination pixels (no blending).
    ///
    /// This is the restore path used by undo: it must reinstate alpha exactly.
    pub fn paste_rect(&mut self, src: &Pixmap, x: i32, y: i32) {
        let target = IRect::new(x, y, src.width as i32, src.height as i32).intersect(&self.bounds());
        if target.is_empty() {
            return;
        }
        for ty in target.y..target.bottom() {
            let sy = (ty - y) as u32;
            let sx = (target.x - x) as u32;
            let so = src.offset(sx, sy);
            let dof = self.offset(target.x as u32, ty as u32);
            let len = target.width as usize * BYTES_PER_PIXEL;
            self.data[dof..dof + len].copy_from_slice(&src.data[so..so + len]);
        }
    }

    /// Bounding box of all pixels with non-zero alpha, or an empty rect.
    pub fn opaque_bounds(&self) -> IRect {
        let mut left = self.width as i32;
        let mut top = self.height as i32;
        let mut right = 0i32;
        let mut bottom = 0i32;
        for y in 0..self.height {
            let row = self.row(y);
            let mut row_has = false;
            for x in 0..self.width {
                if row[x as usize * BYTES_PER_PIXEL + 3] != 0 {
                    row_has = true;
                    left = left.min(x as i32);
                    right = right.max(x as i32 + 1);
                }
            }
            if row_has {
                top = top.min(y as i32);
                bottom = bottom.max(y as i32 + 1);
            }
        }
        if right <= left || bottom <= top {
            IRect::EMPTY
        } else {
            IRect::from_bounds(left, top, right, bottom)
        }
    }

    /// Nearest-neighbour sample in pixel coordinates.
    #[inline]
    pub fn sample_nearest(&self, x: f32, y: f32) -> Rgba8 {
        self.get(x.floor() as i32, y.floor() as i32)
    }

    /// Bilinear sample in pixel coordinates (pixel centres at `+0.5`).
    ///
    /// Interpolation happens on premultiplied values so transparent pixels do
    /// not bleed their (undefined) colour into the result.
    pub fn sample_bilinear(&self, x: f32, y: f32) -> Rgba8 {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (x0, y0) = (x0 as i32, y0 as i32);

        let mut acc = [0.0f32; 4];
        let weights = [
            ((x0, y0), (1.0 - tx) * (1.0 - ty)),
            ((x0 + 1, y0), tx * (1.0 - ty)),
            ((x0, y0 + 1), (1.0 - tx) * ty),
            ((x0 + 1, y0 + 1), tx * ty),
        ];
        for ((px, py), w) in weights {
            if w <= 0.0 {
                continue;
            }
            let c = self.get(px, py);
            let a = c.a as f32 / 255.0;
            acc[0] += c.r as f32 * a * w;
            acc[1] += c.g as f32 * a * w;
            acc[2] += c.b as f32 * a * w;
            acc[3] += a * w;
        }
        if acc[3] <= 1e-6 {
            return Rgba8::TRANSPARENT;
        }
        Rgba8::new(
            (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8,
            (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8,
            (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8,
            (acc[3] * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }

    /// Read a pixel as floating point colour.
    #[inline]
    pub fn get_rgba(&self, x: i32, y: i32) -> Rgba {
        self.get(x, y).to_rgba()
    }

    /// Resize the buffer, keeping existing content anchored at `(0, 0)`.
    pub fn resized_canvas(&self, width: u32, height: u32) -> Pixmap {
        let mut out = Pixmap::new(width, height);
        out.paste_rect(
            &self.copy_rect(IRect::from_size(width.min(self.width), height.min(self.height))),
            0,
            0,
        );
        out
    }

    /// Mirror horizontally.
    pub fn flipped_horizontal(&self) -> Pixmap {
        let mut out = Pixmap::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                out.set(
                    (self.width - 1 - x) as i32,
                    y as i32,
                    self.get(x as i32, y as i32),
                );
            }
        }
        out
    }

    /// Mirror vertically.
    pub fn flipped_vertical(&self) -> Pixmap {
        let mut out = Pixmap::new(self.width, self.height);
        for y in 0..self.height {
            let src = self.row(y);
            out.row_mut(self.height - 1 - y).copy_from_slice(src);
        }
        out
    }

    /// Rotate 90 degrees clockwise (width and height swap).
    pub fn rotated_90_cw(&self) -> Pixmap {
        let mut out = Pixmap::new(self.height, self.width);
        for y in 0..self.height {
            for x in 0..self.width {
                out.set(
                    (self.height - 1 - y) as i32,
                    x as i32,
                    self.get(x as i32, y as i32),
                );
            }
        }
        out
    }

    /// Box-filtered downscale / bilinear upscale to `(width, height)`.
    pub fn scaled(&self, width: u32, height: u32) -> Pixmap {
        let mut out = Pixmap::new(width, height);
        if out.is_empty() || self.is_empty() {
            return out;
        }
        let sx = self.width as f32 / width as f32;
        let sy = self.height as f32 / height as f32;
        for y in 0..height {
            for x in 0..width {
                let c = if sx > 1.0 || sy > 1.0 {
                    self.box_sample(x as f32 * sx, y as f32 * sy, sx, sy)
                } else {
                    self.sample_bilinear((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy)
                };
                out.set(x as i32, y as i32, c);
            }
        }
        out
    }

    /// Average the source pixels covering one destination pixel.
    fn box_sample(&self, x: f32, y: f32, sx: f32, sy: f32) -> Rgba8 {
        let x0 = x.floor().max(0.0) as u32;
        let y0 = y.floor().max(0.0) as u32;
        let x1 = ((x + sx).ceil() as u32).min(self.width).max(x0 + 1);
        let y1 = ((y + sy).ceil() as u32).min(self.height).max(y0 + 1);
        let mut acc = [0.0f64; 4];
        let mut n = 0.0f64;
        for py in y0..y1.min(self.height) {
            for px in x0..x1.min(self.width) {
                let c = self.get(px as i32, py as i32);
                let a = c.a as f64 / 255.0;
                acc[0] += c.r as f64 * a;
                acc[1] += c.g as f64 * a;
                acc[2] += c.b as f64 * a;
                acc[3] += a;
                n += 1.0;
            }
        }
        if n == 0.0 || acc[3] <= 1e-9 {
            return Rgba8::TRANSPARENT;
        }
        Rgba8::new(
            (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8,
            (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8,
            (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8,
            ((acc[3] / n) * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }

    /// Multiply every pixel's alpha by `factor`.
    pub fn scale_alpha(&mut self, factor: f32) {
        let f = factor.clamp(0.0, 1.0);
        for chunk in self.data.chunks_exact_mut(BYTES_PER_PIXEL) {
            chunk[3] = (chunk[3] as f32 * f).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_pixmap_is_transparent() {
        let pm = Pixmap::new(4, 3);
        assert_eq!(pm.width(), 4);
        assert_eq!(pm.height(), 3);
        assert!(pm.data().iter().all(|b| *b == 0));
        assert_eq!(pm.get(0, 0), Rgba8::TRANSPARENT);
    }

    #[test]
    fn out_of_bounds_access_is_safe() {
        let mut pm = Pixmap::new(2, 2);
        pm.set(-1, 0, Rgba8::WHITE);
        pm.set(5, 5, Rgba8::WHITE);
        assert_eq!(pm.get(-1, 0), Rgba8::TRANSPARENT);
        assert_eq!(pm.get(99, 99), Rgba8::TRANSPARENT);
        assert!(pm.data().iter().all(|b| *b == 0));
    }

    #[test]
    fn from_raw_rejects_wrong_length() {
        assert!(Pixmap::from_raw(2, 2, vec![0; 16]).is_ok());
        assert!(Pixmap::from_raw(2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn fill_rect_clips_to_bounds() {
        let mut pm = Pixmap::new(4, 4);
        pm.fill_rect(IRect::new(2, 2, 10, 10), Rgba8::WHITE);
        assert_eq!(pm.get(3, 3), Rgba8::WHITE);
        assert_eq!(pm.get(1, 1), Rgba8::TRANSPARENT);
    }

    #[test]
    fn copy_and_paste_roundtrip() {
        let mut pm = Pixmap::new(8, 8);
        pm.fill_rect(IRect::new(2, 2, 3, 3), Rgba8::rgb(10, 20, 30));
        let patch = pm.copy_rect(IRect::new(1, 1, 5, 5));
        let mut other = Pixmap::new(8, 8);
        other.paste_rect(&patch, 1, 1);
        assert_eq!(other.get(3, 3), Rgba8::rgb(10, 20, 30));
        assert_eq!(other.get(0, 0), Rgba8::TRANSPARENT);
    }

    #[test]
    fn copy_rect_outside_source_is_transparent() {
        let pm = Pixmap::filled(4, 4, Rgba8::WHITE);
        let patch = pm.copy_rect(IRect::new(2, 2, 4, 4));
        assert_eq!(patch.get(0, 0), Rgba8::WHITE);
        assert_eq!(patch.get(3, 3), Rgba8::TRANSPARENT);
    }

    #[test]
    fn opaque_bounds_finds_content() {
        let mut pm = Pixmap::new(16, 16);
        assert!(pm.opaque_bounds().is_empty());
        pm.set(5, 7, Rgba8::WHITE);
        pm.set(9, 3, Rgba8::WHITE);
        assert_eq!(pm.opaque_bounds(), IRect::from_bounds(5, 3, 10, 8));
    }

    #[test]
    fn bilinear_sampling_averages_neighbours() {
        let mut pm = Pixmap::new(2, 1);
        pm.set(0, 0, Rgba8::new(0, 0, 0, 255));
        pm.set(1, 0, Rgba8::new(255, 255, 255, 255));
        let mid = pm.sample_bilinear(1.0, 0.5);
        assert!((mid.r as i32 - 128).abs() <= 1, "got {mid:?}");
    }

    #[test]
    fn transparent_neighbours_do_not_bleed_color() {
        let mut pm = Pixmap::new(2, 1);
        pm.set(0, 0, Rgba8::new(255, 0, 0, 255));
        // pixel 1 stays transparent black
        let mid = pm.sample_bilinear(1.0, 0.5);
        assert_eq!(mid.r, 255, "colour should stay red, got {mid:?}");
        assert!((mid.a as i32 - 128).abs() <= 1);
    }

    #[test]
    fn flips_and_rotation() {
        let mut pm = Pixmap::new(2, 1);
        pm.set(0, 0, Rgba8::WHITE);
        assert_eq!(pm.flipped_horizontal().get(1, 0), Rgba8::WHITE);
        let rot = pm.rotated_90_cw();
        assert_eq!((rot.width(), rot.height()), (1, 2));
        assert_eq!(rot.get(0, 0), Rgba8::WHITE);
    }

    #[test]
    fn downscale_halves_dimensions() {
        let pm = Pixmap::filled(8, 8, Rgba8::rgb(100, 150, 200));
        let small = pm.scaled(4, 4);
        assert_eq!((small.width(), small.height()), (4, 4));
        assert_eq!(small.get(1, 1), Rgba8::rgb(100, 150, 200));
    }

    #[test]
    fn zero_sized_pixmaps_are_inert() {
        let mut pm = Pixmap::new(0, 0);
        assert!(pm.is_empty());
        pm.fill(Rgba8::WHITE);
        pm.fill_rect(IRect::new(0, 0, 4, 4), Rgba8::WHITE);
        assert!(pm.opaque_bounds().is_empty());
    }
}
