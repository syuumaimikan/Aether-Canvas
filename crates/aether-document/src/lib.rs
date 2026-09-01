//! # aether-document
//!
//! The document model: what a project *is*, independent of how it is drawn or
//! displayed.
//!
//! * [`Layer`] and [`LayerContent`] – one node of the layer tree. Raster,
//!   group, adjustment, fill and plugin-defined content all share the same
//!   common properties (opacity, blend mode, mask, clipping...).
//! * [`LayerTree`] – parent/child storage with stable [`LayerId`]s.
//! * [`Document`] – canvas size, layer tree, selection and metadata.
//! * [`Command`] / [`History`] – every mutation an artist can make is a
//!   command that knows how to undo itself.
//!
//! Nothing here touches the GPU or the filesystem: a document can be built,
//! edited and verified in a unit test with no window and no device.

pub mod command;
pub mod document;
pub mod history;
pub mod layer;
pub mod selection;
pub mod tree;

pub use command::{Command, LayerProperty};
pub use document::{Background, Document, DocumentMetadata};
pub use history::{History, HistoryEntry};
pub use layer::{
    AdjustmentContent, FillContent, GroupContent, Layer, LayerContent, LayerKind, RasterContent,
};
pub use selection::Selection;
pub use tree::LayerTree;

pub use aether_core::{AetherError, LayerId, Result};
