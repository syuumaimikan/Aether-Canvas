//! Application themes.
//!
//! A theme is a small set of tokens turned into an [`egui::Visuals`]. Keeping
//! the tokens in one place is what will let a future release load a theme from
//! a file instead of hard-coding it.

use egui::{Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};

/// Themes shipped with the application.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    /// Neutral dark grey; the default for long painting sessions.
    #[default]
    Dark,
    /// Light grey.
    Light,
    /// Maximum contrast for accessibility.
    HighContrast,
}

impl Theme {
    /// All themes in menu order.
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Light, Theme::HighContrast];

    /// Translation key for the theme's name.
    pub fn key(self) -> &'static str {
        match self {
            Theme::Dark => "theme.dark",
            Theme::Light => "theme.light",
            Theme::HighContrast => "theme.high_contrast",
        }
    }

    /// The colour shown behind the canvas.
    pub fn canvas_backdrop(self) -> Color32 {
        match self {
            Theme::Dark => Color32::from_rgb(30, 31, 34),
            Theme::Light => Color32::from_rgb(160, 162, 168),
            Theme::HighContrast => Color32::BLACK,
        }
    }

    /// Build the egui visuals for this theme.
    pub fn visuals(self) -> Visuals {
        let mut visuals = match self {
            Theme::Light => Visuals::light(),
            _ => Visuals::dark(),
        };
        match self {
            Theme::Dark => {
                visuals.panel_fill = Color32::from_rgb(38, 39, 43);
                visuals.window_fill = Color32::from_rgb(44, 45, 50);
                visuals.extreme_bg_color = Color32::from_rgb(24, 25, 28);
                visuals.selection.bg_fill = Color32::from_rgb(64, 110, 180);
            }
            Theme::Light => {
                visuals.panel_fill = Color32::from_rgb(238, 238, 240);
                visuals.window_fill = Color32::from_rgb(248, 248, 250);
                visuals.selection.bg_fill = Color32::from_rgb(120, 160, 220);
            }
            Theme::HighContrast => {
                visuals.panel_fill = Color32::BLACK;
                visuals.window_fill = Color32::BLACK;
                visuals.extreme_bg_color = Color32::BLACK;
                visuals.override_text_color = Some(Color32::WHITE);
                visuals.selection.bg_fill = Color32::from_rgb(0, 120, 215);
                visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::WHITE);
                visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(200));
                visuals.widgets.hovered.bg_stroke = Stroke::new(2.0, Color32::WHITE);
                visuals.widgets.active.bg_stroke = Stroke::new(2.0, Color32::WHITE);
            }
        }
        let radius = CornerRadius::same(3);
        visuals.widgets.noninteractive.corner_radius = radius;
        visuals.widgets.inactive.corner_radius = radius;
        visuals.widgets.hovered.corner_radius = radius;
        visuals.widgets.active.corner_radius = radius;
        visuals
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_has_a_translation_key() {
        for theme in Theme::ALL {
            assert!(theme.key().starts_with("theme."));
        }
    }

    #[test]
    fn high_contrast_forces_white_text() {
        assert_eq!(
            Theme::HighContrast.visuals().override_text_color,
            Some(Color32::WHITE)
        );
    }

    #[test]
    fn light_and_dark_differ() {
        assert_ne!(
            Theme::Light.visuals().panel_fill,
            Theme::Dark.visuals().panel_fill
        );
    }
}
