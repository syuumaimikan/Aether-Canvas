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
//! * [`transform`] – affine and perspective resampling.
//! * [`warp`] – free-form deformation for the warp handles and liquify.
//! * [`mesh`] – textured triangle meshes, the renderer behind rigged layers.
//! * [`adjust`] – colour adjustment kernels shared by adjustment layers and filters.
//! * [`filter`] – blur, sharpen, motion blur, grain and silhouette kernels.
//! * [`effect`] – the non-destructive layer effect stack.

pub mod adjust;
pub mod brush;
pub mod composite;
pub mod effect;
pub mod fill;
pub mod filter;
pub mod mask;
pub mod mesh;
pub mod pixmap;
pub mod tile;
pub mod transform;
pub mod warp;

pub use brush::{BrushDynamics, BrushEngine, BrushPreset, BrushTip, StrokeState};
pub use composite::{composite_pixmap, CompositeOptions};
pub use effect::{EffectKind, LayerEffect};
pub use mask::Mask;
pub use mesh::{draw_textured_mesh, mesh_coverage, MeshDrawOptions};
pub use pixmap::Pixmap;
pub use tile::{DirtyRegion, TileIter, TILE_SIZE};
pub use warp::DisplacementField;
