//! The transparency checkerboard.
//!
//! Drawn *behind* the composite so transparent areas read as empty rather than
//! as white. The pattern is generated in document space and scaled by the
//! viewport, so the squares stay a constant size on screen.

use aether_core::color::Rgba8;
use aether_core::math::IRect;
use aether_raster::composite::{composite_pixmap, CompositeOptions};
use aether_raster::Pixmap;

/// Light square of the default checkerboard.
pub const LIGHT: Rgba8 = Rgba8::rgb(0x50, 0x52, 0x57);
/// Dark square of the default checkerboard.
pub const DARK: Rgba8 = Rgba8::rgb(0x44, 0x46, 0x4a);
/// Default square size in pixels.
pub const CELL: u32 = 8;

/// Build a checkerboard of the given size.
pub fn checkerboard(width: u32, height: u32, cell: u32, light: Rgba8, dark: Rgba8) -> Pixmap {
    let mut pm = Pixmap::new(width, height);
    let cell = cell.max(1);
    for y in 0..height {
        for x in 0..width {
            let odd = ((x / cell) + (y / cell)) % 2 == 1;
            pm.set(x as i32, y as i32, if odd { dark } else { light });
        }
    }
    pm
}

/// Composite `image` over a fresh checkerboard, for previews and thumbnails.
pub fn over_checkerboard(image: &Pixmap, cell: u32) -> Pixmap {
    let mut out = checkerboard(image.width(), image.height(), cell, LIGHT, DARK);
    composite_pixmap(&mut out, image, &CompositeOptions::normal(), None);
    out
}

/// Composite `image` over a flat colour.
pub fn over_color(image: &Pixmap, color: Rgba8) -> Pixmap {
    let mut out = Pixmap::filled(image.width(), image.height(), color);
    composite_pixmap(&mut out, image, &CompositeOptions::normal(), None);
    out
}

/// Fill `region` of `target` with the checkerboard pattern.
pub fn fill_region(target: &mut Pixmap, region: IRect, cell: u32) {
    let area = region.intersect(&target.bounds());
    let cell = cell.max(1) as i32;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let odd = ((x / cell) + (y / cell)) % 2 == 1;
            target.set(x, y, if odd { DARK } else { LIGHT });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squares_alternate() {
        let pm = checkerboard(4, 4, 2, Rgba8::WHITE, Rgba8::BLACK);
        assert_eq!(pm.get(0, 0), Rgba8::WHITE);
        assert_eq!(pm.get(2, 0), Rgba8::BLACK);
        assert_eq!(pm.get(0, 2), Rgba8::BLACK);
        assert_eq!(pm.get(2, 2), Rgba8::WHITE);
    }

    #[test]
    fn a_zero_cell_size_does_not_divide_by_zero() {
        let pm = checkerboard(2, 2, 0, Rgba8::WHITE, Rgba8::BLACK);
        assert_eq!(pm.width(), 2);
    }

    #[test]
    fn opaque_content_hides_the_pattern() {
        let image = Pixmap::filled(4, 4, Rgba8::rgb(10, 20, 30));
        let out = over_checkerboard(&image, 2);
        assert_eq!(out.get(0, 0), Rgba8::rgb(10, 20, 30));
        assert_eq!(out.get(3, 3), Rgba8::rgb(10, 20, 30));
    }

    #[test]
    fn transparent_content_shows_the_pattern() {
        let image = Pixmap::new(4, 4);
        let out = over_checkerboard(&image, 2);
        assert_eq!(out.get(0, 0), LIGHT);
        assert_eq!(out.get(2, 0), DARK);
    }
}
