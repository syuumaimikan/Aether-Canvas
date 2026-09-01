//! # aether-core
//!
//! Foundational, dependency-light types shared by every other Aether Canvas crate.
//!
//! Nothing in this crate knows about documents, GPUs or user interfaces. It provides
//! the vocabulary the rest of the application speaks:
//!
//! * [`math`] – 2D geometry: [`Vec2`], [`Rect`], [`IRect`], [`Transform2D`].
//! * [`color`] – [`Rgba`] (linear-agnostic straight-alpha float color), [`Rgba8`],
//!   [`Hsv`], [`Hsl`] and conversions.
//! * [`blend`] – the [`BlendMode`] set and the W3C compositing formulas behind it.
//! * [`id`] – typed, process-unique identifiers.
//! * [`input`] – pointer/tablet input samples (pressure, tilt, velocity).
//! * [`error`] – the crate-wide [`AetherError`] enum and [`Result`] alias.

pub mod blend;
pub mod color;
pub mod error;
pub mod id;
pub mod input;
pub mod math;

pub use blend::BlendMode;
pub use color::{Hsl, Hsv, Rgba, Rgba8};
pub use error::{AetherError, Result};
pub use id::{AssetId, CompositionId, DocumentId, LayerId, ParameterId};
pub use input::{InputSample, Modifiers, PointerButton};
pub use math::{IRect, Rect, Transform2D, Vec2};
