//! Dockable panels.
//!
//! Each panel is a free function taking `&mut Ui` and `&mut EditorState`. They
//! hold no state of their own, so a panel can be shown twice, moved between
//! docks, or left out of a workspace entirely without any bookkeeping.
//!
//! * [`tools`] – the tool palette and the active tool's options
//! * [`brush`] – brush parameters, presets and texture
//! * [`color`] – colour picker, palette, recent colours
//! * [`layers`] – the layer stack, layer properties and the effect stack
//! * [`history`] – undo history and document properties
//! * [`adjust`] – the adjustment and effect parameter editors, including the
//!   curve widget, shared by adjustment layers, layer effects and filters

pub mod adjust;
pub mod brush;
pub mod color;
pub mod effects;
pub mod helpers;
pub mod history;
pub mod layers;
pub mod tools;

pub use adjust::{adjustment_editor, curve_editor, effect_editor};

/// Display name for an effect, for window titles and list rows.
pub fn effect_name(kind: &aether_raster::effect::EffectKind) -> String {
    kind.name()
}
pub use brush::brush_panel;
pub use color::color_panel;
pub use color::swatch_grid;
pub use effects::effects_section;
pub use helpers::{blend_mode_combo, commit_controls, from_color32, to_color32};
pub use history::{history_panel, properties_panel};
pub use layers::layers_panel;
pub use tools::{tool_options, tools_panel};
