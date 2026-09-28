//! Crate-wide error type.
//!
//! Aether Canvas avoids `unwrap()`/`expect()` in library code. Every fallible
//! operation returns [`Result`], and errors carry enough context to be shown in
//! the UI without inspecting a log file.

use std::fmt;

/// Convenience alias used across the workspace.
pub type Result<T> = std::result::Result<T, AetherError>;

/// The single error type crossing crate boundaries.
///
/// Sub-systems define their own precise error enums (for example
/// `DocumentError`) and convert into this type at the boundary, so the
/// application layer only ever has to match on one enum.
#[derive(Debug, thiserror::Error)]
pub enum AetherError {
    /// Filesystem or stream failure.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// A document-level invariant was violated (missing layer, illegal move, ...).
    #[error("document error: {0}")]
    Document(String),

    /// Rasterisation / pixel buffer failure.
    #[error("raster error: {0}")]
    Raster(String),

    /// Rendering or compositing failure.
    #[error("render error: {0}")]
    Render(String),

    /// Project or asset (de)serialisation failure.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// A file could be read but its contents are not understood.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Project schema is newer than this build understands.
    #[error("unsupported project version {found} (this build supports up to {supported})")]
    UnsupportedVersion {
        /// Version stored in the file.
        found: u32,
        /// Highest version this build can read.
        supported: u32,
    },

    /// Asset system failure.
    #[error("asset error: {0}")]
    Asset(String),

    /// Plugin loading/execution failure.
    #[error("plugin error: {0}")]
    Plugin(String),

    /// A caller passed arguments that cannot be honoured.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// A rig (parameters, deformers, bones, physics, motions) is inconsistent
    /// or an edit to it cannot be performed.
    #[error("rig error: {0}")]
    Rig(String),
}

impl AetherError {
    /// Build a [`AetherError::Document`] from anything printable.
    pub fn document(msg: impl fmt::Display) -> Self {
        Self::Document(msg.to_string())
    }

    /// Build a [`AetherError::Raster`] from anything printable.
    pub fn raster(msg: impl fmt::Display) -> Self {
        Self::Raster(msg.to_string())
    }

    /// Build a [`AetherError::Render`] from anything printable.
    pub fn render(msg: impl fmt::Display) -> Self {
        Self::Render(msg.to_string())
    }

    /// Build a [`AetherError::Serialization`] from anything printable.
    pub fn serialization(msg: impl fmt::Display) -> Self {
        Self::Serialization(msg.to_string())
    }

    /// Build a [`AetherError::InvalidArgument`] from anything printable.
    pub fn invalid(msg: impl fmt::Display) -> Self {
        Self::InvalidArgument(msg.to_string())
    }

    /// Build a [`AetherError::Rig`] from anything printable.
    pub fn rig(msg: impl fmt::Display) -> Self {
        Self::Rig(msg.to_string())
    }
}
