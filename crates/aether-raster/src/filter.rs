//! Spatial filters.
//!
//! Kernels that need neighbouring pixels rather than just the pixel itself.
//! They operate on premultiplied values internally so transparent regions do
//! not bleed undefined colour into the result.

use crate::pixmap::Pixmap;
use aether_core::color::Rgba8;
use aether_core::math::IRect;

/// Box blur with `radius` pixels, applied separably.
pub fn box_blur(src: &Pixmap, radius: i32, region: Option<IRect>) -> Pixmap {
    let mut out = src.clone();
    if radius <= 0 || src.is_empty() {
        return out;
    }
    let area = region.unwrap_or_else(|| src.bounds()).intersect(&src.bounds());
    if area.is_empty() {
        return out;
    }
    // Horizontal pass into a scratch buffer, then vertical back into `out`.
    let mut scratch = src.clone();
    blur_axis(src, &mut scratch, radius, area, true);
    blur_axis(&scratch, &mut out, radius, area, false);
    out
}

fn blur_axis(src: &Pixmap, dst: &mut Pixmap, radius: i32, area: IRect, horizontal: bool) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let mut acc = [0f32; 4];
            let mut n = 0f32;
            for d in -radius..=radius {
                let (sx, sy) = if horizontal { (x + d, y) } else { (x, y + d) };
                if !src.bounds().contains(sx, sy) {
                    continue;
                }
                let c = src.get(sx, sy);
                let a = c.a as f32 / 255.0;
                acc[0] += c.r as f32 * a;
                acc[1] += c.g as f32 * a;
                acc[2] += c.b as f32 * a;
                acc[3] += a;
                n += 1.0;
            }
            if n == 0.0 {
                continue;
            }
            let alpha = acc[3] / n;
            if acc[3] <= 1e-5 {
                dst.set(x, y, Rgba8::TRANSPARENT);
                continue;
            }
            dst.set(
                x,
                y,
                Rgba8::new(
                    (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8,
                    (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8,
                    (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8,
                    (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
                ),
            );
        }
    }
}

/// Gaussian blur approximated by three successive box blurs.
///
/// Three passes is the standard approximation — the error against a true
/// gaussian is under 3%, and the cost stays linear in the radius.
pub fn gaussian_blur(src: &Pixmap, sigma: f32, region: Option<IRect>) -> Pixmap {
    if sigma <= 0.0 {
        return src.clone();
    }
    // Box radius that matches the requested sigma over three passes.
    let radius = ((sigma * 3.0 * (std::f32::consts::PI * 2.0).sqrt() / 4.0 + 0.5) / 2.0)
        .round()
        .max(1.0) as i32;
    let mut out = box_blur(src, radius, region);
    out = box_blur(&out, radius, region);
    box_blur(&out, radius, region)
}

/// Unsharp-mask sharpening: `amount` of the difference against a blurred copy.
pub fn sharpen(src: &Pixmap, amount: f32, radius: f32, region: Option<IRect>) -> Pixmap {
    let blurred = gaussian_blur(src, radius.max(0.5), region);
    let mut out = src.clone();
    let area = region.unwrap_or_else(|| src.bounds()).intersect(&src.bounds());
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let s = src.get(x, y);
            if s.a == 0 {
                continue;
            }
            let b = blurred.get(x, y);
            let f = |sv: u8, bv: u8| {
                (sv as f32 + (sv as f32 - bv as f32) * amount)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            out.set(x, y, Rgba8::new(f(s.r, b.r), f(s.g, b.g), f(s.b, b.b), s.a));
        }
    }
    out
}

/// Shift every pixel by `(dx, dy)`; vacated area becomes transparent.
pub fn offset(src: &Pixmap, dx: i32, dy: i32) -> Pixmap {
    let mut out = Pixmap::new(src.width(), src.height());
    out.paste_rect(&src.copy_rect(src.bounds()), dx, dy);
    out
}

/// Replace every pixel's colour with `color`, keeping the original alpha.
///
/// This is how a shadow, a glow or an outline gets its shape: the layer's
/// silhouette is its alpha channel, and the effect only supplies the colour.
pub fn colorize_alpha(src: &Pixmap, color: Rgba8, strength: f32) -> Pixmap {
    let mut out = Pixmap::new(src.width(), src.height());
    let strength = strength.clamp(0.0, 4.0);
    for y in 0..src.height() as i32 {
        for x in 0..src.width() as i32 {
            let a = src.get(x, y).a;
            if a == 0 {
                continue;
            }
            let a = ((a as f32 * strength).round()).clamp(0.0, 255.0) as u8;
            out.set(x, y, Rgba8::new(color.r, color.g, color.b, a));
        }
    }
    out
}

/// Add monochrome or coloured grain.
///
/// The noise is generated from a hashed pixel coordinate rather than a running
/// RNG, so the same seed always produces the same grain — required for undo and
/// for tests to be reproducible.
pub fn grain(src: &Pixmap, amount: f32, seed: u64, monochrome: bool, region: Option<IRect>) -> Pixmap {
    let mut out = src.clone();
    if amount <= 0.0 {
        return out;
    }
    let area = region.unwrap_or_else(|| src.bounds()).intersect(&src.bounds());
    let amount = amount.clamp(0.0, 1.0) * 255.0;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let c = src.get(x, y);
            if c.a == 0 {
                continue;
            }
            let n = |channel: u64| -> f32 { (hash_noise(x as u64, y as u64, seed ^ channel) - 0.5) * amount };
            let (dr, dg, db) = if monochrome {
                let v = n(0);
                (v, v, v)
            } else {
                (n(0), n(1), n(2))
            };
            let ch = |v: u8, d: f32| (v as f32 + d).round().clamp(0.0, 255.0) as u8;
            out.set(x, y, Rgba8::new(ch(c.r, dr), ch(c.g, dg), ch(c.b, db), c.a));
        }
    }
    out
}

/// Directional blur along `angle` (radians) over `distance` pixels.
pub fn motion_blur(src: &Pixmap, angle: f32, distance: f32, region: Option<IRect>) -> Pixmap {
    let mut out = src.clone();
    if distance <= 0.5 {
        return out;
    }
    let area = region.unwrap_or_else(|| src.bounds()).intersect(&src.bounds());
    let steps = (distance.round() as i32).max(1);
    let (sin, cos) = angle.sin_cos();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let mut acc = [0f32; 4];
            let mut n = 0f32;
            for step in -steps..=steps {
                let t = step as f32 * 0.5;
                let sx = x as f32 + 0.5 + cos * t;
                let sy = y as f32 + 0.5 + sin * t;
                let c = src.sample_bilinear(sx, sy);
                let a = c.a as f32 / 255.0;
                acc[0] += c.r as f32 * a;
                acc[1] += c.g as f32 * a;
                acc[2] += c.b as f32 * a;
                acc[3] += a;
                n += 1.0;
            }
            if acc[3] <= 1e-5 {
                out.set(x, y, Rgba8::TRANSPARENT);
                continue;
            }
            out.set(
                x,
                y,
                Rgba8::new(
                    (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8,
                    (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8,
                    (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8,
                    ((acc[3] / n) * 255.0).round().clamp(0.0, 255.0) as u8,
                ),
            );
        }
    }
    out
}

/// Deterministic value noise in `0..=1` from a coordinate and a seed.
fn hash_noise(x: u64, y: u64, seed: u64) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ y.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed;
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    ((h >> 40) as f32) / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_softens_a_hard_edge() {
        let mut pm = Pixmap::filled(16, 1, Rgba8::BLACK);
        pm.fill_rect(IRect::new(8, 0, 8, 1), Rgba8::WHITE);
        let out = box_blur(&pm, 2, None);
        let edge = out.get(8, 0);
        assert!(edge.r > 0 && edge.r < 255, "expected a soft edge, got {edge:?}");
    }

    #[test]
    fn blur_preserves_a_flat_field() {
        let pm = Pixmap::filled(8, 8, Rgba8::rgb(60, 120, 180));
        let out = gaussian_blur(&pm, 2.0, None);
        assert_eq!(out.get(4, 4), Rgba8::rgb(60, 120, 180));
    }

    #[test]
    fn zero_radius_is_a_no_op() {
        let pm = Pixmap::filled(4, 4, Rgba8::WHITE);
        assert_eq!(box_blur(&pm, 0, None), pm);
        assert_eq!(gaussian_blur(&pm, 0.0, None), pm);
    }

    #[test]
    fn transparent_areas_stay_transparent() {
        let mut pm = Pixmap::new(16, 1);
        pm.set(8, 0, Rgba8::new(255, 0, 0, 255));
        let out = gaussian_blur(&pm, 1.5, None);
        assert_eq!(out.get(0, 0).a, 0);
        assert!(out.get(9, 0).a > 0, "blur should spread alpha to neighbours");
        assert!(out.get(9, 0).r > 200, "colour must not bleed towards black");
    }

    #[test]
    fn sharpen_increases_local_contrast() {
        let mut pm = Pixmap::filled(16, 1, Rgba8::rgb(100, 100, 100));
        pm.fill_rect(IRect::new(8, 0, 8, 1), Rgba8::rgb(150, 150, 150));
        let out = sharpen(&pm, 1.0, 1.0, None);
        assert!(out.get(8, 0).r >= pm.get(8, 0).r);
    }
}

#[cfg(test)]
mod effect_kernel_tests {
    use super::*;

    #[test]
    fn offset_moves_content_and_leaves_transparency_behind() {
        let mut pm = Pixmap::new(8, 8);
        pm.set(1, 1, Rgba8::WHITE);
        let out = offset(&pm, 2, 3);
        assert_eq!(out.get(3, 4), Rgba8::WHITE);
        assert_eq!(out.get(1, 1), Rgba8::TRANSPARENT);
    }

    #[test]
    fn colorize_keeps_the_silhouette() {
        let mut pm = Pixmap::new(4, 1);
        pm.set(0, 0, Rgba8::new(10, 20, 30, 128));
        let out = colorize_alpha(&pm, Rgba8::rgb(255, 0, 0), 1.0);
        assert_eq!(out.get(0, 0), Rgba8::new(255, 0, 0, 128));
        assert_eq!(out.get(1, 0), Rgba8::TRANSPARENT);
    }

    #[test]
    fn grain_is_deterministic_and_bounded() {
        let pm = Pixmap::filled(16, 16, Rgba8::rgb(128, 128, 128));
        let a = grain(&pm, 0.3, 7, true, None);
        let b = grain(&pm, 0.3, 7, true, None);
        assert_eq!(a, b, "the same seed must give the same grain");
        assert_ne!(a, grain(&pm, 0.3, 8, true, None));
        assert_ne!(a, pm);
        assert_eq!(a.get(3, 3).a, 255, "grain must not touch alpha");
    }

    #[test]
    fn grain_leaves_transparent_pixels_alone() {
        let pm = Pixmap::new(8, 8);
        assert_eq!(grain(&pm, 1.0, 1, false, None), pm);
    }

    #[test]
    fn motion_blur_smears_along_the_angle() {
        let mut pm = Pixmap::new(32, 32);
        pm.fill_rect(IRect::new(15, 15, 2, 2), Rgba8::WHITE);
        let out = motion_blur(&pm, 0.0, 8.0, None);
        assert!(out.get(20, 16).a > 0, "should smear horizontally");
        assert_eq!(out.get(16, 24).a, 0, "should not smear vertically");
    }
}
