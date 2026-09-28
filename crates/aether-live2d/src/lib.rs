//! Live2D Cubism models, without Cubism Core.
//!
//! * [`moc3`] reads and writes `.moc3` files (every version, 3.0 to 5.3) into
//!   an owned, validated [`Moc`](moc3::Moc).
//! * [`model`] evaluates a `Moc` at a set of parameter values: keyform
//!   interpolation, blend shapes, warp and rotation deformers, glue, part and
//!   drawable opacity, multiply/screen colours, and draw and render order.
//!   It follows Cubism Core's arithmetic step for step, and the test suite
//!   holds it to Cubism Core's own output on Live2D's sample models.
//!
//! The format knowledge and the evaluation order come from Purism Core, an
//! open reimplementation of Cubism Core (Copyright (c) 2025, 2026 Sakura
//! Motion Project, MIT License), and are checked against Live2D Cubism Core
//! itself by `tests/cubism_oracle.rs`. "Live2D" and "Cubism" are trademarks
//! of Live2D Inc.; this crate is not affiliated with it.

// Core clamps with fminf(fmaxf(x, lo), hi), which maps NaN to the lower
// bound; `max().min()` does the same, `clamp` would keep the NaN.
#![allow(clippy::manual_clamp)]

pub mod moc3;
pub mod model;

pub use moc3::{Moc, MocError};
pub use model::Model;
