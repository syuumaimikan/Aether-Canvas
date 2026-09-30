//! # aether-ui
//!
//! The desktop user interface: docking layout, panels, canvas widget and tools.
//!
//! The UI owns no artwork state of its own. Everything it edits lives in
//! [`aether_document::Document`], and every edit it makes goes through the
//! command system, so the same operations are reachable from tests, from
//! scripts and later from plugins.

pub mod app;
pub mod canvas;
pub mod dock;
pub mod fonts;
pub mod gpu_preview;
pub mod hotkeys;
pub mod i18n;
pub mod icons;
pub mod library;
pub mod panels;
pub mod rigging;
pub mod shortcuts;
pub mod state;
pub mod theme;
pub mod tools;
pub mod tutorial;

pub use app::AetherApp;
pub use i18n::Language;
pub use state::EditorState;
pub use theme::Theme;
