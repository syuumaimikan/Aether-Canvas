//! Flood fill and colour-range selection.
//!
//! Both tools answer the same question — "which pixels are similar enough to
//! this one?" — and both answer it by producing a [`Mask`] rather than writing
//! colour directly. The caller then decides what to do with the region: fill
//! it, turn it into a selection, or feather it first.

use crate::mask::Mask;
use crate::pixmap::Pixmap;
use aether_core::color::Rgba8;
use aether_core::math::IRect;

/// How far a pixel may differ and still be considered part of the region.
///
/// `0` means an exact match; `1` matches everything.
pub fn color_distance(a: Rgba8, b: Rgba8) -> f32 {
    let dr = (a.r as f32 - b.r as f32).abs();
    let dg = (a.g as f32 - b.g as f32).abs();
    let db = (a.b as f32 - b.b as f32).abs();
    let da = (a.a as f32 - b.a as f32).abs();
    // Max-channel distance matches what artists perceive as "same colour"
    // better than a euclidean distance does for flat-coloured line art.
    dr.max(dg).max(db).max(da) / 255.0
}

/// Contiguous flood fill starting at `(x, y)`.
///
/// Uses a scanline span filler: each iteration fills a whole horizontal run and
/// only pushes the rows above and below, which keeps the work list small even
/// on large flat areas.
///
/// `tolerance` is in `0..=1`. Returns full-coverage for matching pixels.
pub fn flood_fill_mask(src: &Pixmap, x: i32, y: i32, tolerance: f32) -> Mask {
    let mut mask = Mask::new(src.width(), src.height());
    if !src.bounds().contains(x, y) {
        return mask;
    }
    let target = src.get(x, y);
    let tol = tolerance.clamp(0.0, 1.0);
    let width = src.width() as i32;
    let height = src.height() as i32;

    let matches = |px: i32, py: i32| -> bool { color_distance(src.get(px, py), target) <= tol };

    let mut stack = vec![(x, y)];
    while let Some((sx, sy)) = stack.pop() {
        if sy < 0 || sy >= height {
            continue;
        }
        if mask.get(sx, sy) != 0 || !matches(sx, sy) {
            continue;
        }
        // Walk left and right to the ends of this span.
        let mut left = sx;
        while left > 0 && mask.get(left - 1, sy) == 0 && matches(left - 1, sy) {
            left -= 1;
        }
        let mut right = sx;
        while right + 1 < width && mask.get(right + 1, sy) == 0 && matches(right + 1, sy) {
            right += 1;
        }
        for px in left..=right {
            mask.set(px, sy, 255);
        }
        // Seed the neighbouring rows once per contiguous run.
        for (ny, _) in [(sy - 1, ()), (sy + 1, ())] {
            if ny < 0 || ny >= height {
                continue;
            }
            let mut px = left;
            while px <= right {
                if mask.get(px, ny) == 0 && matches(px, ny) {
                    stack.push((px, ny));
                    while px <= right && matches(px, ny) {
                        px += 1;
                    }
                }
                px += 1;
            }
        }
    }
    mask
}

/// Select every pixel in the image similar to `target`, ignoring connectivity.
///
/// This is the "select colour range" / global fill mode.
pub fn color_range_mask(src: &Pixmap, target: Rgba8, tolerance: f32) -> Mask {
    let mut mask = Mask::new(src.width(), src.height());
    let tol = tolerance.clamp(0.0, 1.0);
    for y in 0..src.height() as i32 {
        for x in 0..src.width() as i32 {
            if color_distance(src.get(x, y), target) <= tol {
                mask.set(x, y, 255);
            }
        }
    }
    mask
}

/// A rectangular selection mask.
pub fn rect_mask(width: u32, height: u32, rect: IRect) -> Mask {
    let mut mask = Mask::new(width, height);
    mask.fill_rect(rect, 255);
    mask
}

/// An antialiased elliptical selection mask inscribed in `rect`.
pub fn ellipse_mask(width: u32, height: u32, rect: IRect) -> Mask {
    let mut mask = Mask::new(width, height);
    if rect.is_empty() {
        return mask;
    }
    let cx = rect.x as f32 + rect.width as f32 * 0.5;
    let cy = rect.y as f32 + rect.height as f32 * 0.5;
    let rx = (rect.width as f32 * 0.5).max(0.5);
    let ry = (rect.height as f32 * 0.5).max(0.5);
    let area = rect.intersect(&mask.bounds());
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let nx = (x as f32 + 0.5 - cx) / rx;
            let ny = (y as f32 + 0.5 - cy) / ry;
            let d = (nx * nx + ny * ny).sqrt();
            // One pixel of antialiasing at the rim, scaled into normalised space.
            let feather = 1.0 / rx.min(ry);
            let coverage = ((1.0 - d) / feather + 0.5).clamp(0.0, 1.0);
            if coverage > 0.0 {
                mask.set(x, y, (coverage * 255.0).round() as u8);
            }
        }
    }
    mask
}

/// A polygon selection mask using the even-odd rule (lasso, polygon tool).
pub fn polygon_mask(width: u32, height: u32, points: &[aether_core::math::Vec2]) -> Mask {
    let mut mask = Mask::new(width, height);
    if points.len() < 3 {
        return mask;
    }
    for y in 0..height as i32 {
        let py = y as f32 + 0.5;
        let mut crossings: Vec<f32> = Vec::new();
        for i in 0..points.len() {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            if (a.y <= py && b.y > py) || (b.y <= py && a.y > py) {
                let t = (py - a.y) / (b.y - a.y);
                crossings.push(a.x + t * (b.x - a.x));
            }
        }
        crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in crossings.chunks_exact(2) {
            let start = pair[0].ceil().max(0.0) as i32;
            let end = pair[1].floor().min(width as f32) as i32;
            for x in start..=end.min(width as i32 - 1) {
                mask.set(x, y, 255);
            }
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flood_fill_stops_at_a_border() {
        let mut pm = Pixmap::filled(16, 16, Rgba8::WHITE);
        // Vertical black wall down the middle.
        pm.fill_rect(IRect::new(8, 0, 1, 16), Rgba8::BLACK);
        let mask = flood_fill_mask(&pm, 2, 2, 0.0);
        assert_eq!(mask.get(0, 0), 255);
        assert_eq!(mask.get(7, 5), 255);
        assert_eq!(mask.get(8, 5), 0, "the wall itself must not fill");
        assert_eq!(mask.get(12, 5), 0, "fill must not cross the wall");
    }

    #[test]
    fn flood_fill_respects_tolerance() {
        let mut pm = Pixmap::filled(8, 1, Rgba8::rgb(100, 100, 100));
        pm.set(4, 0, Rgba8::rgb(110, 110, 110));
        assert_eq!(flood_fill_mask(&pm, 0, 0, 0.0).get(5, 0), 0);
        assert_eq!(flood_fill_mask(&pm, 0, 0, 0.1).get(5, 0), 255);
    }

    #[test]
    fn flood_fill_outside_bounds_is_empty() {
        let pm = Pixmap::filled(4, 4, Rgba8::WHITE);
        assert!(flood_fill_mask(&pm, -1, 0, 0.5).is_empty());
    }

    #[test]
    fn flood_fill_covers_a_whole_flat_area() {
        let pm = Pixmap::filled(64, 64, Rgba8::WHITE);
        let mask = flood_fill_mask(&pm, 32, 32, 0.0);
        assert_eq!(mask.coverage_bounds(), IRect::from_size(64, 64));
    }

    #[test]
    fn color_range_ignores_connectivity() {
        let mut pm = Pixmap::filled(8, 1, Rgba8::BLACK);
        pm.set(0, 0, Rgba8::WHITE);
        pm.set(7, 0, Rgba8::WHITE);
        let mask = color_range_mask(&pm, Rgba8::WHITE, 0.0);
        assert_eq!((mask.get(0, 0), mask.get(7, 0), mask.get(3, 0)), (255, 255, 0));
    }

    #[test]
    fn ellipse_mask_is_round() {
        let mask = ellipse_mask(64, 64, IRect::new(0, 0, 64, 64));
        assert_eq!(mask.get(32, 32), 255);
        assert_eq!(mask.get(1, 1), 0, "corners are outside the ellipse");
        assert!(mask.get(32, 1) > 0, "top centre is inside");
    }

    #[test]
    fn polygon_mask_fills_a_triangle() {
        use aether_core::math::Vec2;
        let pts = [Vec2::new(0.0, 0.0), Vec2::new(16.0, 0.0), Vec2::new(0.0, 16.0)];
        let mask = polygon_mask(16, 16, &pts);
        assert_eq!(mask.get(1, 1), 255);
        assert_eq!(mask.get(15, 15), 0);
    }
}
