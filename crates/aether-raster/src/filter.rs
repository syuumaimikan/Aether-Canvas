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
