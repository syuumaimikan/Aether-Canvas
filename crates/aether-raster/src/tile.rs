//! Tiling.
//!
//! Aether Canvas renders and uploads in fixed 256x256 tiles. Tiles are the unit
//! of parallelism for the compositor, the unit of texture upload for the GPU
//! canvas, and the granularity of the render cache: a stroke that touches
//! twenty pixels only ever re-composites and re-uploads the one or two tiles it
//! actually landed in.

use aether_core::math::IRect;

/// Edge length of a tile in pixels.
pub const TILE_SIZE: i32 = 256;

/// Tile coordinates (tile-space, not pixel-space).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileIndex {
    /// Tile column.
    pub tx: i32,
    /// Tile row.
    pub ty: i32,
}

impl TileIndex {
    /// The pixel rectangle this tile covers.
    pub fn rect(self) -> IRect {
        IRect::new(self.tx * TILE_SIZE, self.ty * TILE_SIZE, TILE_SIZE, TILE_SIZE)
    }
}

/// Iterator over every tile overlapping a pixel rectangle.
pub struct TileIter {
    rect: IRect,
    tx: i32,
    ty: i32,
    tx_end: i32,
    ty_end: i32,
    tx_start: i32,
}

impl TileIter {
    /// Iterate the tiles covering `rect`.
    pub fn new(rect: IRect) -> Self {
        if rect.is_empty() {
            return Self {
                rect,
                tx: 0,
                ty: 0,
                tx_end: 0,
                ty_end: 0,
                tx_start: 0,
            };
        }
        let tx_start = rect.x.div_euclid(TILE_SIZE);
        let ty_start = rect.y.div_euclid(TILE_SIZE);
        let tx_end = (rect.right() - 1).div_euclid(TILE_SIZE) + 1;
        let ty_end = (rect.bottom() - 1).div_euclid(TILE_SIZE) + 1;
        Self {
            rect,
            tx: tx_start,
            ty: ty_start,
            tx_end,
            ty_end,
            tx_start,
        }
    }
}

impl Iterator for TileIter {
    type Item = (TileIndex, IRect);

    fn next(&mut self) -> Option<Self::Item> {
        if self.ty >= self.ty_end || self.tx_end <= self.tx_start {
            return None;
        }
        let index = TileIndex {
            tx: self.tx,
            ty: self.ty,
        };
        let clipped = index.rect().intersect(&self.rect);
        self.tx += 1;
        if self.tx >= self.tx_end {
            self.tx = self.tx_start;
            self.ty += 1;
        }
        Some((index, clipped))
    }
}

/// Accumulates the region touched since the last flush.
///
/// The region is kept as a single bounding rectangle: cheap to merge, and
/// tile-aligned expansion means a scattered stroke still only re-uploads the
/// tiles it crossed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyRegion {
    rect: IRect,
}

impl DirtyRegion {
    /// An empty region.
    pub const fn new() -> Self {
        Self { rect: IRect::EMPTY }
    }

    /// Mark `rect` as needing redraw.
    pub fn add(&mut self, rect: IRect) {
        if rect.is_empty() {
            return;
        }
        self.rect = self.rect.union(&rect);
    }

    /// Merge another region in.
    pub fn merge(&mut self, other: &DirtyRegion) {
        self.add(other.rect);
    }

    /// The accumulated bounding rectangle.
    pub fn bounds(&self) -> IRect {
        self.rect
    }

    /// True when nothing is dirty.
    pub fn is_empty(&self) -> bool {
        self.rect.is_empty()
    }

    /// The dirty rectangle grown outwards to whole tiles.
    pub fn tile_aligned(&self) -> IRect {
        if self.rect.is_empty() {
            return IRect::EMPTY;
        }
        IRect::from_bounds(
            self.rect.x.div_euclid(TILE_SIZE) * TILE_SIZE,
            self.rect.y.div_euclid(TILE_SIZE) * TILE_SIZE,
            ((self.rect.right() - 1).div_euclid(TILE_SIZE) + 1) * TILE_SIZE,
            ((self.rect.bottom() - 1).div_euclid(TILE_SIZE) + 1) * TILE_SIZE,
        )
    }

    /// Iterate the dirty tiles.
    pub fn tiles(&self) -> TileIter {
        TileIter::new(self.rect)
    }

    /// Forget everything and return what was dirty.
    pub fn take(&mut self) -> IRect {
        std::mem::replace(&mut self.rect, IRect::EMPTY)
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.rect = IRect::EMPTY;
    }
}

/// Split `rect` into horizontal bands of at most [`TILE_SIZE`] rows.
///
/// Row bands (rather than square tiles) keep each work item contiguous in
/// memory, which is what the compositor's `rayon` split wants.
pub fn row_bands(rect: IRect) -> Vec<IRect> {
    if rect.is_empty() {
        return Vec::new();
    }
    let mut bands = Vec::new();
    let mut y = rect.y;
    while y < rect.bottom() {
        let h = TILE_SIZE.min(rect.bottom() - y);
        bands.push(IRect::new(rect.x, y, rect.width, h));
        y += h;
    }
    bands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_iteration_covers_the_rect_exactly_once() {
        let rect = IRect::new(10, 10, 600, 300);
        let tiles: Vec<_> = TileIter::new(rect).collect();
        assert_eq!(tiles.len(), 3 * 2);
        let covered: i64 = tiles.iter().map(|(_, r)| r.area()).sum();
        assert_eq!(covered, rect.area());
    }

    #[test]
    fn empty_rect_yields_no_tiles() {
        assert_eq!(TileIter::new(IRect::EMPTY).count(), 0);
    }

    #[test]
    fn negative_coordinates_tile_correctly() {
        let tiles: Vec<_> = TileIter::new(IRect::new(-10, -10, 20, 20)).collect();
        assert_eq!(tiles.len(), 4);
        assert!(tiles.iter().any(|(i, _)| *i == TileIndex { tx: -1, ty: -1 }));
    }

    #[test]
    fn dirty_region_accumulates_and_aligns() {
        let mut dirty = DirtyRegion::new();
        assert!(dirty.is_empty());
        dirty.add(IRect::new(5, 5, 2, 2));
        dirty.add(IRect::new(300, 300, 1, 1));
        assert_eq!(dirty.bounds(), IRect::from_bounds(5, 5, 301, 301));
        assert_eq!(dirty.tile_aligned(), IRect::from_bounds(0, 0, 512, 512));
        assert_eq!(dirty.take(), IRect::from_bounds(5, 5, 301, 301));
        assert!(dirty.is_empty());
    }

    #[test]
    fn bands_partition_the_rect() {
        let rect = IRect::new(0, 0, 100, 600);
        let bands = row_bands(rect);
        assert_eq!(bands.len(), 3);
        assert_eq!(bands.iter().map(|b| b.area()).sum::<i64>(), rect.area());
    }
}
