//! Non-destructive layer effects.
//!
//! An effect is a serialisable description of an operation, not the pixels it
//! produced. A layer stores a *stack* of them, and the compositor evaluates the
//! stack every time it draws the layer:
//!
//! ```text
//! layer pixels ─▶ effect 1 ─▶ effect 2 ─▶ effect 3 ─▶ blended into the backdrop
//! ```
//!
//! Because the stack is data, effects can be reordered, disabled, retuned or
//! removed at any point without the original pixels ever changing — and undo
//! only has to remember the small parameter list, not a copy of the layer.
//!
//! Effects that draw *outside* the artwork (shadows, glows, outlines) return a
//! full-canvas buffer with the original composited on top, so the layer's own
//! blend mode still applies to the finished result.

use crate::adjust::Adjustment;
use crate::composite::{composite_pixmap, CompositeOptions};
use crate::filter;
use crate::pixmap::Pixmap;
use aether_core::blend::BlendMode;
use aether_core::color::Rgba8;
use serde::{Deserialize, Serialize};

/// One entry in a layer's effect stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EffectKind {
    /// Gaussian blur.
    Blur {
        /// Blur radius as a gaussian sigma, in pixels.
        sigma: f32,
    },
    /// Unsharp-mask sharpening.
    Sharpen {
        /// How much of the difference to add back.
        amount: f32,
        /// Radius of the comparison blur.
        radius: f32,
    },
    /// Directional blur.
    MotionBlur {
        /// Direction in radians.
        angle: f32,
        /// Length of the smear in pixels.
        distance: f32,
    },
    /// A soft halo around the artwork.
    Glow {
        /// Spread of the halo.
        radius: f32,
        /// Brightness multiplier of the halo.
        intensity: f32,
        /// Halo colour.
        color: Rgba8,
    },
    /// An offset, blurred silhouette behind the artwork.
    DropShadow {
        /// Horizontal offset in pixels.
        dx: f32,
        /// Vertical offset in pixels.
        dy: f32,
        /// Softness of the shadow.
        radius: f32,
        /// Shadow colour.
        color: Rgba8,
        /// Shadow strength in `0..=1`.
        opacity: f32,
    },
    /// A band of colour hugging the artwork's edge.
    Outline {
        /// Thickness in pixels.
        width: i32,
        /// Outline colour.
        color: Rgba8,
    },
    /// Flood the artwork's silhouette with a colour.
    ColorOverlay {
        /// The colour to lay over the layer.
        color: Rgba8,
        /// Strength in `0..=1`.
        opacity: f32,
        /// How the colour combines with the layer.
        blend: BlendMode,
    },
    /// Film grain.
    Grain {
        /// Grain strength in `0..=1`.
        amount: f32,
        /// Seed, so the grain is stable across redraws and undo.
        seed: u64,
        /// Whether the grain is colourless.
        monochrome: bool,
    },
    /// Any colour adjustment, applied to this layer alone.
    ///
    /// The same operations an adjustment layer applies to everything below it,
    /// scoped to one layer instead.
    Adjust {
        /// The adjustment and its parameters.
        adjustment: Adjustment,
    },
}

impl EffectKind {
    /// Display name for the effect list.
    pub fn name(&self) -> String {
        match self {
            EffectKind::Blur { .. } => "Blur".into(),
            EffectKind::Sharpen { .. } => "Sharpen".into(),
            EffectKind::MotionBlur { .. } => "Motion Blur".into(),
            EffectKind::Glow { .. } => "Glow".into(),
            EffectKind::DropShadow { .. } => "Drop Shadow".into(),
            EffectKind::Outline { .. } => "Outline".into(),
            EffectKind::ColorOverlay { .. } => "Color Overlay".into(),
            EffectKind::Grain { .. } => "Grain".into(),
            EffectKind::Adjust { adjustment } => adjustment.name().to_string(),
        }
    }

    /// Sensible starting parameters, used when adding an effect from the menu.
    pub fn presets() -> Vec<EffectKind> {
        vec![
            EffectKind::Blur { sigma: 4.0 },
            EffectKind::Sharpen {
                amount: 0.6,
                radius: 1.5,
            },
            EffectKind::MotionBlur {
                angle: 0.0,
                distance: 12.0,
            },
            EffectKind::Glow {
                radius: 12.0,
                intensity: 1.2,
                color: Rgba8::rgb(255, 220, 150),
            },
            EffectKind::DropShadow {
                dx: 6.0,
                dy: 6.0,
                radius: 6.0,
                color: Rgba8::BLACK,
                opacity: 0.6,
            },
            EffectKind::Outline {
                width: 2,
                color: Rgba8::BLACK,
            },
            EffectKind::ColorOverlay {
                color: Rgba8::rgb(255, 120, 60),
                opacity: 0.5,
                blend: BlendMode::Normal,
            },
            EffectKind::Grain {
                amount: 0.15,
                seed: 1,
                monochrome: true,
            },
            EffectKind::Adjust {
                adjustment: Adjustment::default_brightness_contrast(),
            },
        ]
    }
}

/// An effect plus the switch that turns it off without losing its settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerEffect {
    /// What the effect does.
    pub kind: EffectKind,
    /// Whether it currently contributes.
    pub enabled: bool,
}

impl LayerEffect {
    /// An enabled effect.
    pub fn new(kind: EffectKind) -> Self {
        Self { kind, enabled: true }
    }

    /// Display name.
    pub fn name(&self) -> String {
        self.kind.name()
    }

    /// Run this effect over `src`, returning a new buffer of the same size.
    pub fn apply(&self, src: &Pixmap) -> Pixmap {
        if !self.enabled || src.is_empty() {
            return src.clone();
        }
        match &self.kind {
            EffectKind::Blur { sigma } => filter::gaussian_blur(src, *sigma, None),
            EffectKind::Sharpen { amount, radius } => filter::sharpen(src, *amount, *radius, None),
            EffectKind::MotionBlur { angle, distance } => filter::motion_blur(src, *angle, *distance, None),
            EffectKind::Glow {
                radius,
                intensity,
                color,
            } => {
                let silhouette = filter::colorize_alpha(src, *color, 1.0);
                let mut glow = filter::gaussian_blur(&silhouette, *radius, None);
                // Boosting after the blur is what makes the halo read as light
                // rather than as a blurred copy of the layer.
                glow = filter::colorize_alpha(&glow, *color, *intensity);
                stack(glow, src)
            }
            EffectKind::DropShadow {
                dx,
                dy,
                radius,
                color,
                opacity,
            } => {
                let silhouette = filter::colorize_alpha(src, *color, opacity.clamp(0.0, 1.0));
                let moved = filter::offset(&silhouette, dx.round() as i32, dy.round() as i32);
                let shadow = filter::gaussian_blur(&moved, *radius, None);
                stack(shadow, src)
            }
            EffectKind::Outline { width, color } => {
                let mut ring = crate::mask::Mask::from_alpha(src);
                ring.expand((*width).max(0));
                let mut inner = crate::mask::Mask::from_alpha(src);
                // Only the band outside the artwork is drawn, so a semi
                // transparent layer does not get a bar of colour through it.
                inner.invert();
                ring.multiply(&inner);
                let mut band = Pixmap::new(src.width(), src.height());
                for y in 0..src.height() as i32 {
                    for x in 0..src.width() as i32 {
                        let coverage = ring.get(x, y);
                        if coverage > 0 {
                            band.set(x, y, Rgba8::new(color.r, color.g, color.b, coverage));
                        }
                    }
                }
                stack(band, src)
            }
            EffectKind::ColorOverlay {
                color,
                opacity,
                blend,
            } => {
                let mut out = src.clone();
                let overlay = Pixmap::filled(src.width(), src.height(), *color);
                let opts = CompositeOptions {
                    blend: *blend,
                    opacity: opacity.clamp(0.0, 1.0),
                    offset: (0, 0),
                    region: None,
                    // Keep the layer's silhouette: an overlay must not paint
                    // over the empty parts of the canvas.
                    alpha_lock: true,
                };
                composite_pixmap(&mut out, &overlay, &opts, None);
                out
            }
            EffectKind::Grain {
                amount,
                seed,
                monochrome,
            } => filter::grain(src, *amount, *seed, *monochrome, None),
            EffectKind::Adjust { adjustment } => {
                let mut out = src.clone();
                adjustment.apply(&mut out, None);
                out
            }
        }
    }
}

/// Put `src` on top of `under`, returning the combined buffer.
fn stack(mut under: Pixmap, src: &Pixmap) -> Pixmap {
    composite_pixmap(&mut under, src, &CompositeOptions::normal(), None);
    under
}

/// Run a whole stack in order.
///
/// Returns the input unchanged when the stack is empty or entirely disabled,
/// so the compositor's common case costs nothing.
pub fn apply_stack(src: &Pixmap, effects: &[LayerEffect]) -> Pixmap {
    if effects.iter().all(|e| !e.enabled) {
        return src.clone();
    }
    let mut current = src.clone();
    for effect in effects {
        if effect.enabled {
            current = effect.apply(&current);
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::math::IRect;

    fn square() -> Pixmap {
        let mut pm = Pixmap::new(64, 64);
        pm.fill_rect(IRect::new(24, 24, 16, 16), Rgba8::new(200, 200, 200, 255));
        pm
    }

    #[test]
    fn an_empty_stack_changes_nothing() {
        let pm = square();
        assert_eq!(apply_stack(&pm, &[]), pm);
    }

    #[test]
    fn a_disabled_effect_changes_nothing() {
        let pm = square();
        let effects = vec![LayerEffect {
            kind: EffectKind::Blur { sigma: 5.0 },
            enabled: false,
        }];
        assert_eq!(apply_stack(&pm, &effects), pm);
    }

    #[test]
    fn blur_softens_the_edge() {
        let pm = square();
        let out = apply_stack(&pm, &[LayerEffect::new(EffectKind::Blur { sigma: 3.0 })]);
        let edge = out.get(23, 32).a;
        assert!(edge > 0 && edge < 255, "expected a soft edge, got {edge}");
    }

    #[test]
    fn a_drop_shadow_appears_behind_and_offset() {
        let pm = square();
        let out = apply_stack(
            &pm,
            &[LayerEffect::new(EffectKind::DropShadow {
                dx: 8.0,
                dy: 8.0,
                radius: 2.0,
                color: Rgba8::BLACK,
                opacity: 1.0,
            })],
        );
        assert_eq!(
            out.get(32, 32),
            Rgba8::new(200, 200, 200, 255),
            "the artwork stays on top"
        );
        let shadow = out.get(45, 45);
        assert!(
            shadow.a > 0 && shadow.r < 60,
            "expected a dark shadow, got {shadow:?}"
        );
        assert_eq!(out.get(5, 5).a, 0, "the shadow must not cover the whole canvas");
    }

    #[test]
    fn a_glow_spreads_outwards_from_the_shape() {
        let pm = square();
        let out = apply_stack(
            &pm,
            &[LayerEffect::new(EffectKind::Glow {
                radius: 6.0,
                intensity: 1.5,
                color: Rgba8::rgb(255, 0, 0),
            })],
        );
        let halo = out.get(20, 32);
        assert!(halo.a > 0 && halo.r > halo.b, "expected a red halo, got {halo:?}");
        assert_eq!(
            out.get(32, 32),
            Rgba8::new(200, 200, 200, 255),
            "the artwork is unchanged"
        );
    }

    #[test]
    fn an_outline_hugs_the_edge_without_covering_the_artwork() {
        let pm = square();
        let out = apply_stack(
            &pm,
            &[LayerEffect::new(EffectKind::Outline {
                width: 3,
                color: Rgba8::rgb(255, 0, 0),
            })],
        );
        assert_eq!(
            out.get(22, 32),
            Rgba8::new(255, 0, 0, 255),
            "outline outside the shape"
        );
        assert_eq!(
            out.get(32, 32),
            Rgba8::new(200, 200, 200, 255),
            "artwork untouched"
        );
        assert_eq!(out.get(5, 5).a, 0, "outline stays near the shape");
    }

    #[test]
    fn a_colour_overlay_respects_the_silhouette() {
        let pm = square();
        let out = apply_stack(
            &pm,
            &[LayerEffect::new(EffectKind::ColorOverlay {
                color: Rgba8::rgb(255, 0, 0),
                opacity: 1.0,
                blend: BlendMode::Normal,
            })],
        );
        assert_eq!(out.get(32, 32), Rgba8::new(255, 0, 0, 255));
        assert_eq!(out.get(5, 5).a, 0, "empty canvas must stay empty");
    }

    #[test]
    fn an_adjustment_effect_only_touches_its_own_layer() {
        let pm = square();
        let out = apply_stack(
            &pm,
            &[LayerEffect::new(EffectKind::Adjust {
                adjustment: Adjustment::Invert,
            })],
        );
        assert_eq!(out.get(32, 32), Rgba8::new(55, 55, 55, 255));
        assert_eq!(out.get(5, 5).a, 0);
    }

    #[test]
    fn effects_apply_in_order() {
        let pm = square();
        let blur_then_invert = apply_stack(
            &pm,
            &[
                LayerEffect::new(EffectKind::Blur { sigma: 3.0 }),
                LayerEffect::new(EffectKind::Adjust {
                    adjustment: Adjustment::Invert,
                }),
            ],
        );
        let invert_then_blur = apply_stack(
            &pm,
            &[
                LayerEffect::new(EffectKind::Adjust {
                    adjustment: Adjustment::Invert,
                }),
                LayerEffect::new(EffectKind::Blur { sigma: 3.0 }),
            ],
        );
        assert_ne!(blur_then_invert, invert_then_blur, "order must matter");
    }

    #[test]
    fn every_preset_round_trips_through_serde_and_runs() {
        let pm = square();
        for kind in EffectKind::presets() {
            let effect = LayerEffect::new(kind);
            let text = serde_json::to_string(&effect).expect("serialize");
            let back: LayerEffect = serde_json::from_str(&text).expect("deserialize");
            assert_eq!(back, effect);
            let out = back.apply(&pm);
            assert_eq!((out.width(), out.height()), (pm.width(), pm.height()));
        }
    }
}
