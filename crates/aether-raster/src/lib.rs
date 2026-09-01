//! # aether-raster
//!
//! Everything that touches pixels: storage, compositing, painting and
//! pixel-space image operations.
//!
//! * [`Pixmap`] – an 8-bit straight-alpha RGBA buffer, the storage behind every
//!   raster layer.
//! * [`Mask`] – an 8-bit coverage buffer used for layer masks, selections and
//!   the brush stroke accumulation buffer.
//! * [`tile`] – the tiling constants and iterators used to split work for the
//!   thread pool and to upload only dirty regions to the GPU.
//! * [`composite`] – blending one buffer onto another.
//! * [`brush`] – the dab-based brush engine (spacing, dynamics, hardness).
//! * [`fill`] – flood fill.
//! * [`transform`] – affine resampling.
//! * [`adjust`] – colour adjustment kernels shared by adjustment layers and filters.
//! * [`filter`] – separable blur and sharpen kernels.

pub mod adjust;
pub mod brush;
pub mod composite;
pub mod fill;
pub mod filter;
pub mod mask;
pub mod pixmap;
pub mod tile;
pub mod transform;

pub use brush::{BrushDynamics, BrushEngine, BrushPreset, BrushTip, StrokeState};
pub use composite::{composite_pixmap, CompositeOptions};
pub use mask::Mask;
pub use pixmap::Pixmap;
pub use tile::{DirtyRegion, TileIter, TILE_SIZE};
