//! Incremental composite cache.
//!
//! Re-compositing a 4000x4000 document for every brush dab would be hopeless.
//! The cache keeps the last full composite and asks the document what changed:
//! only that rectangle is re-rendered, and only the tiles it covers need to be
//! re-uploaded to the GPU.

use crate::compositor::{Compositor, RenderOptions};
use aether_core::math::IRect;
use aether_document::Document;
use aether_raster::tile::{TileIndex, TileIter};
use aether_raster::Pixmap;

/// The last composited image plus what changed since the consumer last looked.
#[derive(Debug)]
pub struct RenderCache {
    image: Pixmap,
    /// Region of `image` that changed since [`RenderCache::take_updated`].
    updated: IRect,
    valid: bool,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderCache {
    /// An empty, invalid cache.
    pub fn new() -> Self {
        Self {
            image: Pixmap::new(0, 0),
            updated: IRect::EMPTY,
            valid: false,
        }
    }

    /// The cached composite.
    pub fn image(&self) -> &Pixmap {
        &self.image
    }

    /// True when the cache holds a usable composite.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Force a full re-render on the next update.
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// Re-composite whatever the document reports as dirty.
    ///
    /// Returns the region that changed, which the caller uses to decide what to
    /// re-upload. A resized document, or a cache that has never been filled,
    /// re-renders everything.
    pub fn update(&mut self, doc: &mut Document, compositor: &Compositor) -> IRect {
        let needs_full = !self.valid || self.image.width() != doc.width || self.image.height() != doc.height;

        let region = if needs_full {
            doc.take_dirty();
            self.image = Pixmap::new(doc.width, doc.height);
            doc.bounds()
        } else {
            doc.take_dirty()
        };

        if region.is_empty() {
            return IRect::EMPTY;
        }

        compositor.render_into(
            doc,
            &mut self.image,
            &RenderOptions::default().with_region(region),
        );
        self.valid = true;
        self.updated = self.updated.union(&region);
        region
    }

    /// The accumulated changed region, cleared by the call.
    pub fn take_updated(&mut self) -> IRect {
        std::mem::replace(&mut self.updated, IRect::EMPTY)
    }

    /// Tiles overlapping the accumulated changed region.
    pub fn updated_tiles(&self) -> impl Iterator<Item = (TileIndex, IRect)> {
        TileIter::new(self.updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;
    use aether_document::command::RegionEdit;
    use aether_document::Document;

    fn doc() -> Document {
        let mut d = Document::new(64, 64, "cache");
        d.take_dirty();
        d
    }

    #[test]
    fn the_first_update_renders_everything() {
        let mut d = doc();
        let mut cache = RenderCache::new();
        let region = cache.update(&mut d, &Compositor::new());
        assert_eq!(region, d.bounds());
        assert!(cache.is_valid());
    }

    #[test]
    fn a_clean_document_needs_no_work() {
        let mut d = doc();
        let mut cache = RenderCache::new();
        cache.update(&mut d, &Compositor::new());
        assert!(cache.update(&mut d, &Compositor::new()).is_empty());
    }

    #[test]
    fn only_the_dirty_region_is_recomposited() {
        let mut d = doc();
        let mut cache = RenderCache::new();
        let compositor = Compositor::new();
        cache.update(&mut d, &compositor);
        cache.take_updated();

        let layer = d.active_layer;
        let _ = RegionEdit::capture(&mut d, layer, IRect::new(4, 4, 8, 8), "Paint", |pm| {
            pm.fill_rect(IRect::new(4, 4, 8, 8), Rgba8::WHITE);
            Ok(())
        })
        .expect("capture");

        let region = cache.update(&mut d, &compositor);
        assert_eq!(region, IRect::new(4, 4, 8, 8));
        assert_eq!(cache.image().get(5, 5), Rgba8::WHITE);
        assert_eq!(cache.image().get(20, 20), Rgba8::TRANSPARENT);
    }

    #[test]
    fn incremental_updates_match_a_full_render() {
        let mut d = doc();
        let compositor = Compositor::new();
        let mut cache = RenderCache::new();
        cache.update(&mut d, &compositor);

        let layer = d.active_layer;
        for i in 0..4 {
            let rect = IRect::new(i * 8, i * 8, 8, 8);
            let _ = RegionEdit::capture(&mut d, layer, rect, "Paint", |pm| {
                pm.fill_rect(rect, Rgba8::new(20 * i as u8 + 10, 0, 0, 255));
                Ok(())
            })
            .expect("capture");
            cache.update(&mut d, &compositor);
        }

        let full = compositor.render(&d);
        assert_eq!(
            cache.image(),
            &full,
            "incremental composite drifted from a full render"
        );
    }

    #[test]
    fn resizing_the_document_forces_a_full_render() {
        let mut d = doc();
        let compositor = Compositor::new();
        let mut cache = RenderCache::new();
        cache.update(&mut d, &compositor);
        d.resize_canvas(32, 32);
        let region = cache.update(&mut d, &compositor);
        assert_eq!(region, IRect::from_size(32, 32));
        assert_eq!(cache.image().width(), 32);
    }

    #[test]
    fn updated_tiles_cover_the_changed_area() {
        let mut d = Document::new(512, 512, "tiles");
        let compositor = Compositor::new();
        let mut cache = RenderCache::new();
        cache.update(&mut d, &compositor);
        cache.take_updated();

        let layer = d.active_layer;
        let _ = RegionEdit::capture(&mut d, layer, IRect::new(300, 10, 4, 4), "Paint", |pm| {
            pm.fill_rect(IRect::new(300, 10, 4, 4), Rgba8::WHITE);
            Ok(())
        })
        .expect("capture");
        cache.update(&mut d, &compositor);

        let tiles: Vec<_> = cache.updated_tiles().collect();
        assert_eq!(
            tiles.len(),
            1,
            "a small edit should touch one tile, got {tiles:?}"
        );
    }
}
