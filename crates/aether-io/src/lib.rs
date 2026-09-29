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
//! * [`psd`] – layered Photoshop documents in and out, the usual hand-off
//!   format for artwork that is about to be rigged.
//! * [`live2d`] – Live2D Cubism motions (`.motion3.json`) and expressions
//!   (`.exp3.json`), in and out.
//! * [`runtime_model`] – runtime models (`model.json` + texture atlases) for
//!   `aether-player` in games, apps and on the web.
//!
//! The project format carries a schema version and goes through
//! [`project::migrate`] on load, so older files keep opening as the format
//! grows through later phases.

pub mod animation;
pub mod archive;
pub mod brush_io;
pub mod image_io;
pub mod library;
pub mod live2d;
pub mod live2d_export;
pub mod live2d_model;
pub mod project;
pub mod psd;
pub mod runtime_model;

pub use brush_io::{load_presets, save_presets};
pub use image_io::{export_image, load_image, save_png, ExportSettings, ImageFormat};
pub use project::{load_project, save_project, SCHEMA_VERSION};
