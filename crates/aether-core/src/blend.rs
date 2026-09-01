//! Layer blend modes.
//!
//! The implementation follows the W3C *Compositing and Blending Level 1*
//! specification, which is the same model used by SVG, CSS and every major
//! raster editor, so results match what artists expect when they move files
//! between tools.
//!
//! For each pixel the backdrop `Cb` (already composited layers below) and the
//! source `Cs` (this layer) are combined in two steps:
//!
//! 1. a *blend function* `B(Cb, Cs)` – the part that differs per mode;
//! 2. *source-over* compositing that mixes the blended colour in according to
//!    the source and backdrop alphas.
//!
//! Colours here are straight-alpha and in `0..=1`.

use crate::color::Rgba;
use crate::math::clampf;
use serde::{Deserialize, Serialize};

/// How a layer combines with everything beneath it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlendMode {
    /// Plain source-over.
    #[default]
    Normal,
    /// Darkens by multiplying.
    Multiply,
    /// Lightens; the inverse of [`BlendMode::Multiply`].
    Screen,
    /// Multiply or screen depending on the backdrop.
    Overlay,
    /// Keeps the darker channel.
    Darken,
    /// Keeps the lighter channel.
    Lighten,
    /// Brightens the backdrop to reflect the source.
    ColorDodge,
    /// Darkens the backdrop to reflect the source.
    ColorBurn,
    /// Multiply or screen depending on the source.
    HardLight,
    /// Gentler [`BlendMode::HardLight`].
    SoftLight,
    /// Absolute difference.
    Difference,
    /// Lower contrast [`BlendMode::Difference`].
    Exclusion,
    /// Additive light.
    Add,
    /// Backdrop minus source.
    Subtract,
    /// Backdrop divided by source.
    Divide,
    /// Additive darkening.
    LinearBurn,
    /// Dodge/burn driven by the source.
    VividLight,
    /// Add/subtract driven by the source.
    LinearLight,
    /// Lighten/darken by distance from mid grey.
    PinLight,
    /// Source hue, backdrop saturation and luminosity.
    Hue,
    /// Source saturation, backdrop hue and luminosity.
    Saturation,
    /// Source hue and saturation, backdrop luminosity.
    Color,
    /// Source luminosity, backdrop hue and saturation.
    Luminosity,
}

impl BlendMode {
    /// Every mode, in the order the UI presents them.
    pub const ALL: [BlendMode; 23] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Add,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::LinearBurn,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    /// Stable identifier used in the project file and in the UI.
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::HardLight => "Hard Light",
            BlendMode::SoftLight => "Soft Light",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Add => "Add",
            BlendMode::Subtract => "Subtract",
            BlendMode::Divide => "Divide",
            BlendMode::LinearBurn => "Linear Burn",
            BlendMode::VividLight => "Vivid Light",
            BlendMode::LinearLight => "Linear Light",
            BlendMode::PinLight => "Pin Light",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Luminosity => "Luminosity",
        }
    }

    /// True for the four modes that need all three channels at once.
    #[inline]
    pub fn is_non_separable(self) -> bool {
        matches!(
            self,
            BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity
        )
    }

    /// Apply the blend function `B(Cb, Cs)` to one channel.
    ///
    /// Only valid for separable modes; non-separable modes route through
    /// [`BlendMode::blend_rgb`].
    #[inline]
    pub fn blend_channel(self, cb: f32, cs: f32) -> f32 {
        match self {
            BlendMode::Normal => cs,
            BlendMode::Multiply => cb * cs,
            BlendMode::Screen => cb + cs - cb * cs,
            BlendMode::Overlay => BlendMode::HardLight.blend_channel(cs, cb),
            BlendMode::Darken => cb.min(cs),
            BlendMode::Lighten => cb.max(cs),
            BlendMode::ColorDodge => {
                if cb <= 0.0 {
                    0.0
                } else if cs >= 1.0 {
                    1.0
                } else {
                    (cb / (1.0 - cs)).min(1.0)
                }
            }
            BlendMode::ColorBurn => {
                if cb >= 1.0 {
                    1.0
                } else if cs <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - cb) / cs).min(1.0)
                }
            }
            BlendMode::HardLight => {
                if cs <= 0.5 {
                    cb * (2.0 * cs)
                } else {
                    let s = 2.0 * cs - 1.0;
                    cb + s - cb * s
                }
            }
            BlendMode::SoftLight => {
                if cs <= 0.5 {
                    cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
                } else {
                    let d = if cb <= 0.25 {
                        ((16.0 * cb - 12.0) * cb + 4.0) * cb
                    } else {
                        cb.sqrt()
                    };
                    cb + (2.0 * cs - 1.0) * (d - cb)
                }
            }
            BlendMode::Difference => (cb - cs).abs(),
            BlendMode::Exclusion => cb + cs - 2.0 * cb * cs,
            BlendMode::Add => (cb + cs).min(1.0),
            BlendMode::Subtract => (cb - cs).max(0.0),
            BlendMode::Divide => {
                if cs <= 0.0 {
                    1.0
                } else {
                    (cb / cs).min(1.0)
                }
            }
            BlendMode::LinearBurn => (cb + cs - 1.0).max(0.0),
            BlendMode::VividLight => {
                if cs <= 0.5 {
                    BlendMode::ColorBurn.blend_channel(cb, (2.0 * cs).min(1.0))
                } else {
                    BlendMode::ColorDodge.blend_channel(cb, (2.0 * (cs - 0.5)).min(1.0))
                }
            }
            BlendMode::LinearLight => clampf(cb + 2.0 * cs - 1.0, 0.0, 1.0),
            BlendMode::PinLight => {
                if cs <= 0.5 {
                    cb.min(2.0 * cs)
                } else {
                    cb.max(2.0 * cs - 1.0)
                }
            }
            // Handled by blend_rgb; falling back to the source keeps this total.
            BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => cs,
        }
    }

    /// Apply the blend function to a whole RGB triple.
    pub fn blend_rgb(self, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
        match self {
            BlendMode::Hue => set_lum(&set_sat(&cs, sat(&cb)), lum(&cb)),
            BlendMode::Saturation => set_lum(&set_sat(&cb, sat(&cs)), lum(&cb)),
            BlendMode::Color => set_lum(&cs, lum(&cb)),
            BlendMode::Luminosity => set_lum(&cb, lum(&cs)),
            _ => [
                self.blend_channel(cb[0], cs[0]),
                self.blend_channel(cb[1], cs[1]),
                self.blend_channel(cb[2], cs[2]),
            ],
        }
    }
}

/// Composite `source` over `backdrop` using `mode` at `opacity` (`0..=1`).
///
/// This is the single reference implementation; the raster fast paths in
/// `aether-raster` are validated against it in tests.
pub fn composite(backdrop: Rgba, source: Rgba, mode: BlendMode, opacity: f32) -> Rgba {
    let sa = clampf(source.a * clampf(opacity, 0.0, 1.0), 0.0, 1.0);
    if sa <= 0.0 {
        return backdrop;
    }
    let ba = backdrop.a;
    let cb = [backdrop.r, backdrop.g, backdrop.b];
    let cs = [source.r, source.g, source.b];

    // Blend only where a backdrop exists; elsewhere the source shows through.
    let blended = if ba > 0.0 && mode != BlendMode::Normal {
        mode.blend_rgb(cb, cs)
    } else {
        cs
    };
    let mixed = [
        cs[0] + ba * (blended[0] - cs[0]),
        cs[1] + ba * (blended[1] - cs[1]),
        cs[2] + ba * (blended[2] - cs[2]),
    ];

    let out_a = sa + ba * (1.0 - sa);
    if out_a <= 0.0 {
        return Rgba::TRANSPARENT;
    }
    let ch = |i: usize| (sa * mixed[i] + ba * (1.0 - sa) * cb[i]) / out_a;
    Rgba::new(ch(0), ch(1), ch(2), out_a)
}

#[inline]
fn lum(c: &[f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: &[f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = *c;
    if n < 0.0 {
        let d = (l - n).max(f32::EPSILON);
        for v in out.iter_mut() {
            *v = l + (*v - l) * l / d;
        }
    }
    if x > 1.0 {
        let d = (x - l).max(f32::EPSILON);
        for v in out.iter_mut() {
            *v = l + (*v - l) * (1.0 - l) / d;
        }
    }
    out
}

fn set_lum(c: &[f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(&[c[0] + d, c[1] + d, c[2] + d])
}

#[inline]
fn sat(c: &[f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: &[f32; 3], s: f32) -> [f32; 3] {
    // Order the channels, rescale the mid/max, then put them back.
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|a, b| c[*a].partial_cmp(&c[*b]).unwrap_or(std::cmp::Ordering::Equal));
    let (imin, imid, imax) = (idx[0], idx[1], idx[2]);
    let mut out = [0.0f32; 3];
    if c[imax] > c[imin] {
        out[imid] = (c[imid] - c[imin]) * s / (c[imax] - c[imin]);
        out[imax] = s;
    }
    out[imin] = 0.0;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn assert_rgb(actual: Rgba, expect: [f32; 3]) {
        assert!(
            approx(actual.r, expect[0]) && approx(actual.g, expect[1]) && approx(actual.b, expect[2]),
            "expected {expect:?}, got {actual:?}"
        );
    }

    #[test]
    fn normal_over_opaque_backdrop_replaces() {
        let out = composite(Rgba::BLACK, Rgba::WHITE, BlendMode::Normal, 1.0);
        assert_rgb(out, [1.0, 1.0, 1.0]);
        assert!(approx(out.a, 1.0));
    }

    #[test]
    fn half_opacity_is_a_midpoint() {
        let out = composite(Rgba::BLACK, Rgba::WHITE, BlendMode::Normal, 0.5);
        assert_rgb(out, [0.5, 0.5, 0.5]);
    }

    #[test]
    fn multiply_and_screen_are_duals() {
        let cb = 0.6;
        let cs = 0.3;
        assert!(approx(BlendMode::Multiply.blend_channel(cb, cs), 0.18));
        assert!(approx(
            BlendMode::Screen.blend_channel(cb, cs),
            1.0 - (1.0 - cb) * (1.0 - cs)
        ));
    }

    #[test]
    fn overlay_is_hardlight_with_swapped_operands() {
        for cb in [0.0, 0.2, 0.5, 0.8, 1.0] {
            for cs in [0.0, 0.3, 0.7, 1.0] {
                assert!(approx(
                    BlendMode::Overlay.blend_channel(cb, cs),
                    BlendMode::HardLight.blend_channel(cs, cb)
                ));
            }
        }
    }

    #[test]
    fn dodge_and_burn_clamp_at_the_extremes() {
        assert!(approx(BlendMode::ColorDodge.blend_channel(0.0, 0.9), 0.0));
        assert!(approx(BlendMode::ColorDodge.blend_channel(0.5, 1.0), 1.0));
        assert!(approx(BlendMode::ColorBurn.blend_channel(1.0, 0.1), 1.0));
        assert!(approx(BlendMode::ColorBurn.blend_channel(0.5, 0.0), 0.0));
    }

    #[test]
    fn transparent_source_is_a_no_op() {
        let backdrop = Rgba::new(0.3, 0.4, 0.5, 0.8);
        assert_eq!(
            composite(backdrop, Rgba::TRANSPARENT, BlendMode::Multiply, 1.0),
            backdrop
        );
        assert_eq!(
            composite(backdrop, Rgba::WHITE, BlendMode::Multiply, 0.0),
            backdrop
        );
    }

    #[test]
    fn blending_over_nothing_keeps_the_source() {
        // With an empty backdrop, every mode must fall back to the source colour,
        // otherwise painting on an empty layer would darken or lighten itself.
        for mode in BlendMode::ALL {
            let out = composite(Rgba::TRANSPARENT, Rgba::rgb(0.2, 0.6, 0.9), mode, 1.0);
            assert_rgb(out, [0.2, 0.6, 0.9]);
        }
    }

    #[test]
    fn luminosity_takes_source_lightness() {
        let backdrop = Rgba::rgb(0.8, 0.2, 0.2);
        let source = Rgba::gray(0.5);
        let out = composite(backdrop, source, BlendMode::Luminosity, 1.0);
        let l = 0.3 * out.r + 0.59 * out.g + 0.11 * out.b;
        assert!(approx(l, 0.5), "luminosity mismatch: {out:?}");
    }

    #[test]
    fn color_mode_keeps_backdrop_luminosity() {
        let backdrop = Rgba::gray(0.35);
        let source = Rgba::rgb(0.9, 0.1, 0.4);
        let out = composite(backdrop, source, BlendMode::Color, 1.0);
        let l = 0.3 * out.r + 0.59 * out.g + 0.11 * out.b;
        assert!(approx(l, 0.35), "expected luminosity 0.35, got {l} from {out:?}");
    }

    #[test]
    fn alpha_composition_matches_porter_duff() {
        let out = composite(
            Rgba::new(0.0, 0.0, 0.0, 0.5),
            Rgba::new(1.0, 1.0, 1.0, 0.5),
            BlendMode::Normal,
            1.0,
        );
        assert!(approx(out.a, 0.75));
    }

    #[test]
    fn all_modes_stay_in_range() {
        let samples = [0.0f32, 0.13, 0.5, 0.87, 1.0];
        for mode in BlendMode::ALL {
            for &cb in &samples {
                for &cs in &samples {
                    let out = composite(Rgba::rgb(cb, cb, cb), Rgba::rgb(cs, cs, cs), mode, 1.0);
                    for v in [out.r, out.g, out.b, out.a] {
                        assert!(
                            (0.0..=1.0).contains(&v),
                            "{mode:?} produced {v} for cb={cb} cs={cs}"
                        );
                    }
                }
            }
        }
    }
}
