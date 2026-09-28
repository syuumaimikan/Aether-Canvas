//! # aether-render
//!
//! Turning a document into pixels.
//!
//! * [`Compositor`] – walks the layer tree and produces a flat [`Pixmap`].
//!   Groups, clipping, masks, adjustment layers and blend modes are all
//!   resolved here, and rigged layers are redrawn through their posed meshes.
//! * [`RenderCache`] – keeps the last composite and re-renders only the region
//!   the document reports as dirty.
//! * [`Viewport`] – the canvas view transform: pan, zoom, rotation and mirror,
//!   plus the screen/document coordinate conversions tools need.
//! * [`checker`] – the transparency checkerboard.
//!
//! The compositor is CPU-side and deliberately independent of any GPU API. It
//! is the reference implementation the GPU render graph is validated against,
//! and it is what file export uses, so what an artist sees is what gets saved.

pub mod cache;
pub mod checker;
pub mod compositor;
pub mod viewport;

pub use cache::RenderCache;
pub use compositor::{Compositor, CustomContentRenderer, RenderOptions};
pub use viewport::Viewport;

pub use aether_raster::Pixmap;
