//! Sample characters for Aether Canvas.
//!
//! * [`luna`] — Luna, rigged from a finished illustration and a sheet of
//!   parts: how most characters arrive. The face's moving parts are layers
//!   cut from the sheet; the rest of her moves by deforming the
//!   illustration through a stack of warp deformers.
//! * [`paint`] — a small vector painter: anti-aliased paths, gradients,
//!   tapered strokes, soft shapes, and shading that stays inside the paint.
//! * [`character`] — Aether-chan, an anime-style character painted in
//!   parts with it.
//! * [`rig`] — Aether-chan rigged: auto-rigged from her layer names, then
//!   given smiling eyes, expressions and motions by hand.
//!
//! [`luna::luna`] and [`aether_chan`] build the whole thing; the editor
//! offers them from Help ▸ Open sample character, and the tutorials start
//! from them.

pub mod character;
pub mod luna;
pub mod paint;
pub mod rig;

pub use luna::luna;
pub use rig::aether_chan;
