//! Compositing one buffer onto another.
//!
//! Every pixel that ends up on screen or in an exported file goes through
//! [`composite_pixmap`] or [`fill_masked`]. Both are thin, parallel wrappers
//! around [`aether_core::blend::composite`], which is the single reference
//! implementation of the blending maths.

use crate::mask::Mask;
use crate::pixmap::{Pixmap, BYTES_PER_PIXEL};
use aether_core::blend::{composite, BlendMode};
use aether_core::color::{Rgba, Rgba8};
use aether_core::math::IRect;
use rayon::prelude::*;

/// How a source buffer is combined into a destination buffer.
#[derive(Clone, Copy, Debug)]
pub struct CompositeOptions {
    /// Blend function to use.
    pub blend: BlendMode,
    /// Layer opacity in `0..=1`.
    pub opacity: f32,
    /// Where the source origin lands in destination coordinates.
    pub offset: (i32, i32),
    /// Restrict the write to this destination rectangle.
    pub region: Option<IRect>,
    /// When set, destination alpha is preserved ("lock transparent pixels").
    pub alpha_lock: bool,
}

impl Default for CompositeOptions {
    fn default() -> Self {
        Self {
            blend: BlendMode::Normal,
            opacity: 1.0,
            offset: (0, 0),
            region: None,
            alpha_lock: false,
        }
    }
}

impl CompositeOptions {
    /// Options for a plain source-over paste at full opacity.
    pub fn normal() -> Self {
        Self::default()
    }

    /// Builder-style blend mode.
    pub fn with_blend(mut self, blend: BlendMode) -> Self {
        self.blend = blend;
        self
    }

    /// Builder-style opacity.
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Builder-style destination offset.
    pub fn with_offset(mut self, x: i32, y: i32) -> Self {
        self.offset = (x, y);
        self
    }

    /// Builder-style write region.
    pub fn with_region(mut self, region: IRect) -> Self {
        self.region = Some(region);
        self
    }
}

/// Composite `src` onto `dst`.
///
/// `mask`, when given, is sampled in **destination** coordinates and scales the
/// source alpha, which is exactly how layer masks and selections behave.
///
/// Returns the destination rectangle that was actually written, so callers can
/// mark just that area dirty.
pub fn composite_pixmap(
    dst: &mut Pixmap,
    src: &Pixmap,
    opts: &CompositeOptions,
    mask: Option<&Mask>,
) -> IRect {
    let (ox, oy) = opts.offset;
    let mut region = IRect::new(ox, oy, src.width() as i32, src.height() as i32).intersect(&dst.bounds());
    if let Some(limit) = opts.region {
        region = region.intersect(&limit);
    }
    if region.is_empty() || opts.opacity <= 0.0 {
        return IRect::EMPTY;
    }

    let dst_width = dst.width();
    let row_bytes = dst_width as usize * BYTES_PER_PIXEL;
    let opacity = opts.opacity;
    let blend = opts.blend;
    let alpha_lock = opts.alpha_lock;

    dst.data_mut()
        .par_chunks_mut(row_bytes)
        .enumerate()
        .filter(|(y, _)| {
            let y = *y as i32;
            y >= region.y && y < region.bottom()
        })
        .for_each(|(y, row)| {
            let y = y as i32;
            let sy = y - oy;
            for x in region.x..region.right() {
                let sx = x - ox;
                let mut source = src.get(sx, sy).to_rgba();
                if source.a <= 0.0 && !alpha_lock {
                    continue;
                }
                let mut op = opacity;
                if let Some(m) = mask {
                    let cov = m.get(x, y);
                    if cov == 0 {
                        continue;
                    }
                    op *= cov as f32 / 255.0;
                }
                let o = x as usize * BYTES_PER_PIXEL;
                let backdrop = Rgba8::new(row[o], row[o + 1], row[o + 2], row[o + 3]).to_rgba();
                if alpha_lock {
                    if backdrop.a <= 0.0 {
                        continue;
                    }
                    // Keep the destination silhouette: blend colour only.
                    source = source.with_alpha(source.a);
                }
                let mut out = composite(backdrop, source, blend, op);
                if alpha_lock {
                    out = out.with_alpha(backdrop.a);
                }
                let px = out.to_rgba8();
                row[o] = px.r;
                row[o + 1] = px.g;
                row[o + 2] = px.b;
                row[o + 3] = px.a;
            }
        });

    region
}

/// Composite a flat `color` onto `dst` through `mask`.
///
/// This is the second half of a brush stroke: dabs accumulate coverage into a
/// [`Mask`], then the whole stroke is laid down in one pass so overlapping dabs
/// never darken each other.
pub fn fill_masked(dst: &mut Pixmap, color: Rgba, mask: &Mask, opts: &CompositeOptions) -> IRect {
    let mut region = mask.bounds().intersect(&dst.bounds());
    if let Some(limit) = opts.region {
        region = region.intersect(&limit);
    }
    if region.is_empty() || opts.opacity <= 0.0 {
        return IRect::EMPTY;
    }

    let row_bytes = dst.width() as usize * BYTES_PER_PIXEL;
    let opacity = opts.opacity;
    let blend = opts.blend;
    let alpha_lock = opts.alpha_lock;

    dst.data_mut()
        .par_chunks_mut(row_bytes)
        .enumerate()
        .filter(|(y, _)| {
            let y = *y as i32;
            y >= region.y && y < region.bottom()
        })
        .for_each(|(y, row)| {
            let y = y as i32;
            for x in region.x..region.right() {
                let cov = mask.get(x, y);
                if cov == 0 {
                    continue;
                }
                let o = x as usize * BYTES_PER_PIXEL;
                let backdrop = Rgba8::new(row[o], row[o + 1], row[o + 2], row[o + 3]).to_rgba();
                if alpha_lock && backdrop.a <= 0.0 {
                    continue;
                }
                let op = opacity * (cov as f32 / 255.0);
                let mut out = composite(backdrop, color, blend, op);
                if alpha_lock {
                    out = out.with_alpha(backdrop.a);
                }
                let px = out.to_rgba8();
                row[o] = px.r;
                row[o + 1] = px.g;
                row[o + 2] = px.b;
                row[o + 3] = px.a;
            }
        });

    region
}

/// Erase from `dst` through `mask`: destination alpha is reduced by coverage.
///
/// Erasing has to touch alpha directly rather than blending a colour, which is
/// why it is a separate kernel instead of a blend mode.
pub fn erase_masked(dst: &mut Pixmap, mask: &Mask, opacity: f32, region: Option<IRect>) -> IRect {
    let mut area = mask.bounds().intersect(&dst.bounds());
    if let Some(limit) = region {
        area = area.intersect(&limit);
    }
    if area.is_empty() || opacity <= 0.0 {
        return IRect::EMPTY;
    }
    let row_bytes = dst.width() as usize * BYTES_PER_PIXEL;
    let opacity = opacity.clamp(0.0, 1.0);

    dst.data_mut()
        .par_chunks_mut(row_bytes)
        .enumerate()
        .filter(|(y, _)| {
            let y = *y as i32;
            y >= area.y && y < area.bottom()
        })
        .for_each(|(y, row)| {
            let y = y as i32;
            for x in area.x..area.right() {
                let cov = mask.get(x, y);
                if cov == 0 {
                    continue;
                }
                let o = x as usize * BYTES_PER_PIXEL;
                let keep = 1.0 - opacity * (cov as f32 / 255.0);
                row[o + 3] = (row[o + 3] as f32 * keep).round().clamp(0.0, 255.0) as u8;
            }
        });

    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;

    #[test]
    fn opaque_source_replaces_destination() {
        let mut dst = Pixmap::filled(4, 4, Rgba8::BLACK);
        let src = Pixmap::filled(4, 4, Rgba8::WHITE);
        let written = composite_pixmap(&mut dst, &src, &CompositeOptions::normal(), None);
        assert_eq!(written, IRect::from_size(4, 4));
        assert_eq!(dst.get(2, 2), Rgba8::WHITE);
    }

    #[test]
    fn offset_and_clipping_are_respected() {
        let mut dst = Pixmap::new(8, 8);
        let src = Pixmap::filled(4, 4, Rgba8::WHITE);
        let written = composite_pixmap(
            &mut dst,
            &src,
            &CompositeOptions::normal().with_offset(6, 6),
            None,
        );
        assert_eq!(written, IRect::new(6, 6, 2, 2));
        assert_eq!(dst.get(7, 7), Rgba8::WHITE);
        assert_eq!(dst.get(5, 5), Rgba8::TRANSPARENT);
    }

    #[test]
    fn region_limits_the_write() {
        let mut dst = Pixmap::new(8, 8);
        let src = Pixmap::filled(8, 8, Rgba8::WHITE);
        let opts = CompositeOptions::normal().with_region(IRect::new(0, 0, 2, 2));
        composite_pixmap(&mut dst, &src, &opts, None);
        assert_eq!(dst.get(1, 1), Rgba8::WHITE);
        assert_eq!(dst.get(3, 3), Rgba8::TRANSPARENT);
    }

    #[test]
    fn mask_scales_source_alpha() {
        let mut dst = Pixmap::filled(4, 1, Rgba8::BLACK);
        let src = Pixmap::filled(4, 1, Rgba8::WHITE);
        let mut mask = Mask::new(4, 1);
        mask.set(0, 0, 255);
        mask.set(1, 0, 128);
        composite_pixmap(&mut dst, &src, &CompositeOptions::normal(), Some(&mask));
        assert_eq!(dst.get(0, 0), Rgba8::WHITE);
        assert!((dst.get(1, 0).r as i32 - 128).abs() <= 2);
        assert_eq!(dst.get(2, 0), Rgba8::BLACK);
    }

    #[test]
    fn opacity_blends_halfway() {
        let mut dst = Pixmap::filled(2, 2, Rgba8::BLACK);
        let src = Pixmap::filled(2, 2, Rgba8::WHITE);
        composite_pixmap(
            &mut dst,
            &src,
            &CompositeOptions::normal().with_opacity(0.5),
            None,
        );
        assert!((dst.get(0, 0).r as i32 - 128).abs() <= 2);
    }

    #[test]
    fn alpha_lock_keeps_the_silhouette() {
        let mut dst = Pixmap::new(4, 1);
        dst.set(0, 0, Rgba8::BLACK);
        let src = Pixmap::filled(4, 1, Rgba8::WHITE);
        let mut opts = CompositeOptions::normal();
        opts.alpha_lock = true;
        composite_pixmap(&mut dst, &src, &opts, None);
        assert_eq!(dst.get(0, 0), Rgba8::WHITE);
        assert_eq!(
            dst.get(1, 0),
            Rgba8::TRANSPARENT,
            "transparent pixels must stay empty"
        );
    }

    #[test]
    fn fill_masked_paints_through_coverage() {
        let mut dst = Pixmap::new(4, 1);
        let mut mask = Mask::new(4, 1);
        mask.set(1, 0, 255);
        let written = fill_masked(
            &mut dst,
            Rgba::rgb(1.0, 0.0, 0.0),
            &mask,
            &CompositeOptions::normal(),
        );
        assert_eq!(dst.get(1, 0), Rgba8::new(255, 0, 0, 255));
        assert_eq!(dst.get(0, 0), Rgba8::TRANSPARENT);
        assert!(!written.is_empty());
    }

    #[test]
    fn erase_reduces_alpha_only() {
        let mut dst = Pixmap::filled(2, 1, Rgba8::new(10, 20, 30, 255));
        let mut mask = Mask::new(2, 1);
        mask.set(0, 0, 255);
        erase_masked(&mut dst, &mask, 1.0, None);
        assert_eq!(dst.get(0, 0).a, 0);
        assert_eq!(dst.get(0, 0).r, 10, "colour channels must be untouched");
        assert_eq!(dst.get(1, 0).a, 255);
    }

    #[test]
    fn partial_erase_halves_alpha() {
        let mut dst = Pixmap::filled(1, 1, Rgba8::new(0, 0, 0, 200));
        let mask = Mask::filled(1, 1, 255);
        erase_masked(&mut dst, &mask, 0.5, None);
        assert_eq!(dst.get(0, 0).a, 100);
    }
}
