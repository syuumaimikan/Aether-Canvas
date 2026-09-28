//! # aether-player
//!
//! The runtime for Aether Canvas models: load a model exported from the
//! editor, animate it, and draw it — in a game, an app, a server, or a web
//! page.
//!
//! * [`model`] — the `model.json` format: parts (textured meshes), the draw
//!   tree and the rig.
//! * [`player`] — [`Player`]: parameters, motions, expressions, look-at, lip
//!   sync, physics, motion events, hit testing, and a per-frame draw list.
//! * [`cpu`] — a software renderer, the reference for GPU renderers.
//! * [`ffi`] — the C ABI, which is also the WebAssembly interface used by
//!   the JavaScript/WebGL player in `runtime/web`.
//!
//! The player evaluates the very same rig code as the editor, so a model
//! moves identically everywhere.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use aether_player::Player;
//! let json = std::fs::read_to_string("model/model.json")?;
//! let mut player = Player::from_json(&json)?;
//! if let Some(idle) = player.motion_index("Idle") {
//!     player.play_motion(idle, false);
//! }
//! player.tick(1.0 / 60.0);
//! for item in player.draw_list() {
//!     let part = &player.model().parts[item.part as usize];
//!     let positions = player.positions(item.part as usize);
//!     // Upload `positions`, `part.uvs` and `part.triangles`, then draw with
//!     // `item.opacity`, `item.blend_kind()`, tint and mask.
//!     let _ = (part, positions);
//! }
//! # Ok(()) }
//! ```

pub mod cpu;
pub mod ffi;
pub mod model;
pub mod player;

pub use model::{BlendKind, Model, ModelError, Node, Part, Texture};
pub use player::{DrawItem, FiredEvent, Player, Stage};
