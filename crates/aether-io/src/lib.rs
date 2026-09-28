//! # aether-io
//!
//! Reading and writing files.
//!
//! * [`project`] – the `.aether` project container: a plain ZIP holding a JSON
//!   manifest plus PNG-encoded pixel data. Open, documented and inspectable
//!   with ordinary tools; artwork should never be locked inside a format only
//!   one application understands.
//! * [`image_io`] – importing and exporting PNG, JPEG, WebP, TIFF, BMP and GIF.
//! * [`brush_io`] – brush presets as plain, shareable JSON.
//! * [`animation`] – rendering rig motions to PNG sequences, animated GIFs
//!   and sprite sheets.
//!
//! The project format carries a schema version and goes through
//! [`project::migrate`] on load, so older files keep opening as the format
//! grows through later phases.

pub mod animation;
pub mod brush_io;
pub mod image_io;
pub mod project;

pub use brush_io::{load_presets, save_presets};
pub use image_io::{export_image, load_image, save_png, ExportSettings, ImageFormat};
pub use project::{load_project, save_project, SCHEMA_VERSION};
