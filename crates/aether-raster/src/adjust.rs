//! Colour adjustment kernels.
//!
//! These are the maths behind adjustment layers and the filter menu. They are
//! deliberately parameter objects rather than closures: an [`Adjustment`] is
//! serialisable, so an adjustment layer stores *what to do*, not the pixels it
//! produced, and stays fully non-destructive.

use crate::pixmap::Pixmap;
use aether_core::color::{Hsl, Rgba, Rgba8};
use aether_core::math::{clampf, IRect};
use serde::{Deserialize, Serialize};

/// A tone curve defined by control points in `0..=1`.
///
/// Points are kept sorted on the x axis and evaluated with a Catmull-Rom spline
/// clamped to the unit range, which gives the smooth, non-overshooting response
/// artists expect from a curves widget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    points: Vec<(f32, f32)>,
}

impl Default for Curve {
    fn default() -> Self {
        Self::identity()
    }
}

impl Curve {
    /// The straight line `y = x`.
    pub fn identity() -> Self {
        Self {
            points: vec![(0.0, 0.0), (1.0, 1.0)],
        }
    }

    /// Build from control points; they are sorted and clamped.
    pub fn new(mut points: Vec<(f32, f32)>) -> Self {
        if points.len() < 2 {
            return Self::identity();
        }
        for p in points.iter_mut() {
            p.0 = clampf(p.0, 0.0, 1.0);
            p.1 = clampf(p.1, 0.0, 1.0);
        }
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        Self { points }
    }

    /// The control points.
    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// Add a control point, keeping the list sorted.
    pub fn add_point(&mut self, x: f32, y: f32) {
        let p = (clampf(x, 0.0, 1.0), clampf(y, 0.0, 1.0));
        let idx = self
            .points
            .iter()
            .position(|q| q.0 > p.0)
            .unwrap_or(self.points.len());
        self.points.insert(idx, p);
    }

    /// Remove the control point at `index`, unless only two remain.
    pub fn remove_point(&mut self, index: usize) {
        if self.points.len() > 2 && index < self.points.len() {
            self.points.remove(index);
        }
    }

    /// True when this curve is the identity.
    pub fn is_identity(&self) -> bool {
        *self == Self::identity()
    }

    /// Evaluate the curve at `x`.
    pub fn eval(&self, x: f32) -> f32 {
        let x = clampf(x, 0.0, 1.0);
        let pts = &self.points;
        if x <= pts[0].0 {
            return pts[0].1;
        }
        if x >= pts[pts.len() - 1].0 {
            return pts[pts.len() - 1].1;
        }
        let i = pts.iter().rposition(|p| p.0 <= x).unwrap_or(0);
        let p1 = pts[i];
        let p2 = pts[(i + 1).min(pts.len() - 1)];
        let span = (p2.0 - p1.0).max(1e-6);
        let t = (x - p1.0) / span;
        // Phantom endpoints keep the spline's tangents linear at the ends, so a
        // two-point curve evaluates to a straight line (the identity curve).
        let p0 = if i == 0 { 2.0 * p1.1 - p2.1 } else { pts[i - 1].1 };
        let p3 = if i + 2 > pts.len() - 1 {
            2.0 * p2.1 - p1.1
        } else {
            pts[i + 2].1
        };
        let v = catmull_rom(p0, p1.1, p2.1, p3, t);
        clampf(v, 0.0, 1.0)
    }

    /// A 256-entry lookup table for fast 8-bit application.
    pub fn to_lut(&self) -> [u8; 256] {
        let mut lut = [0u8; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            *slot = (self.eval(i as f32 / 255.0) * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        lut
    }
}

fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// A non-destructive colour operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Adjustment {
    /// Additive brightness and S-curve contrast, both in `-1..=1`.
    BrightnessContrast {
        /// Brightness offset.
        brightness: f32,
        /// Contrast amount.
        contrast: f32,
    },
    /// Exposure in stops.
    Exposure {
        /// Stops of exposure; `+1` doubles the light.
        stops: f32,
    },
    /// Power-law gamma.
    Gamma {
        /// Gamma exponent; values below 1 brighten.
        gamma: f32,
    },
    /// Input/output black and white points plus a midtone gamma.
    Levels {
        /// Input black point.
        in_black: f32,
        /// Input white point.
        in_white: f32,
        /// Midtone gamma.
        gamma: f32,
        /// Output black point.
        out_black: f32,
        /// Output white point.
        out_white: f32,
    },
    /// Per-channel tone curves.
    Curves {
        /// Applied to all channels.
        master: Curve,
        /// Red channel curve.
        red: Curve,
        /// Green channel curve.
        green: Curve,
        /// Blue channel curve.
        blue: Curve,
    },
    /// Hue rotation and saturation/lightness scaling.
    HueSaturation {
        /// Hue rotation in degrees.
        hue: f32,
        /// Saturation change in `-1..=1`.
        saturation: f32,
        /// Lightness change in `-1..=1`.
        lightness: f32,
    },
    /// Per-channel offsets in `-1..=1`.
    ColorBalance {
        /// Red offset.
        red: f32,
        /// Green offset.
        green: f32,
        /// Blue offset.
        blue: f32,
    },
    /// Invert RGB, keeping alpha.
    Invert,
    /// Desaturate using Rec. 709 luminance.
    Grayscale,
    /// Two-tone threshold.
    Threshold {
        /// Luminance split point.
        level: f32,
    },
    /// Quantise each channel to `levels` steps.
    Posterize {
        /// Number of levels per channel, `2..=255`.
        levels: u8,
    },
}

impl Adjustment {
    /// Human-readable name for the UI and the layer list.
    pub fn name(&self) -> &'static str {
        match self {
            Adjustment::BrightnessContrast { .. } => "Brightness / Contrast",
            Adjustment::Exposure { .. } => "Exposure",
            Adjustment::Gamma { .. } => "Gamma",
            Adjustment::Levels { .. } => "Levels",
            Adjustment::Curves { .. } => "Curves",
            Adjustment::HueSaturation { .. } => "Hue / Saturation",
            Adjustment::ColorBalance { .. } => "Color Balance",
            Adjustment::Invert => "Invert",
            Adjustment::Grayscale => "Grayscale",
            Adjustment::Threshold { .. } => "Threshold",
            Adjustment::Posterize { .. } => "Posterize",
        }
    }

    /// Default parameters for each kind, used when adding a new adjustment layer.
    pub fn default_brightness_contrast() -> Self {
        Adjustment::BrightnessContrast {
            brightness: 0.0,
            contrast: 0.0,
        }
    }

    /// Neutral levels.
    pub fn default_levels() -> Self {
        Adjustment::Levels {
            in_black: 0.0,
            in_white: 1.0,
            gamma: 1.0,
            out_black: 0.0,
            out_white: 1.0,
        }
    }

    /// Neutral curves.
    pub fn default_curves() -> Self {
        Adjustment::Curves {
            master: Curve::identity(),
            red: Curve::identity(),
            green: Curve::identity(),
            blue: Curve::identity(),
        }
    }

    /// Neutral hue/saturation.
    pub fn default_hue_saturation() -> Self {
        Adjustment::HueSaturation {
            hue: 0.0,
            saturation: 0.0,
            lightness: 0.0,
        }
    }

    /// Apply to a single colour.
    pub fn apply_color(&self, c: Rgba) -> Rgba {
        match self {
            Adjustment::BrightnessContrast { brightness, contrast } => {
                let b = clampf(*brightness, -1.0, 1.0);
                let k = clampf(*contrast, -1.0, 1.0);
                // Standard contrast slope; +1 is a hard S, -1 flattens to grey.
                let slope = if k >= 0.0 { 1.0 / (1.0 - k * 0.99) } else { 1.0 + k };
                let f = |v: f32| clampf((v + b - 0.5) * slope + 0.5, 0.0, 1.0);
                Rgba::new(f(c.r), f(c.g), f(c.b), c.a)
            }
            Adjustment::Exposure { stops } => {
                let m = 2f32.powf(*stops);
                Rgba::new(c.r * m, c.g * m, c.b * m, c.a)
            }
            Adjustment::Gamma { gamma } => {
                let g = gamma.max(0.01);
                Rgba::new(c.r.powf(1.0 / g), c.g.powf(1.0 / g), c.b.powf(1.0 / g), c.a)
            }
            Adjustment::Levels {
                in_black,
                in_white,
                gamma,
                out_black,
                out_white,
            } => {
                let span = (in_white - in_black).abs().max(1e-4);
                let g = gamma.max(0.01);
                let f = |v: f32| {
                    let t = clampf((v - in_black) / span, 0.0, 1.0).powf(1.0 / g);
                    clampf(out_black + t * (out_white - out_black), 0.0, 1.0)
                };
                Rgba::new(f(c.r), f(c.g), f(c.b), c.a)
            }
            Adjustment::Curves {
                master,
                red,
                green,
                blue,
            } => Rgba::new(
                master.eval(red.eval(c.r)),
                master.eval(green.eval(c.g)),
                master.eval(blue.eval(c.b)),
                c.a,
            ),
            Adjustment::HueSaturation {
                hue,
                saturation,
                lightness,
            } => {
                let mut hsl = Hsl::from_rgba(c);
                hsl.h = (hsl.h + hue).rem_euclid(360.0);
                hsl.s = clampf(hsl.s * (1.0 + clampf(*saturation, -1.0, 1.0)), 0.0, 1.0);
                let l = clampf(*lightness, -1.0, 1.0);
                hsl.l = if l >= 0.0 {
                    hsl.l + (1.0 - hsl.l) * l
                } else {
                    hsl.l * (1.0 + l)
                };
                hsl.to_rgba()
            }
            Adjustment::ColorBalance { red, green, blue } => {
                Rgba::new(c.r + red, c.g + green, c.b + blue, c.a)
            }
            Adjustment::Invert => Rgba::new(1.0 - c.r, 1.0 - c.g, 1.0 - c.b, c.a),
            Adjustment::Grayscale => {
                let l = c.luminance();
                Rgba::new(l, l, l, c.a)
            }
            Adjustment::Threshold { level } => {
                let v = if c.luminance() >= *level { 1.0 } else { 0.0 };
                Rgba::new(v, v, v, c.a)
            }
            Adjustment::Posterize { levels } => {
                let n = (*levels).max(2) as f32;
                let q = |v: f32| (v * (n - 1.0)).round() / (n - 1.0);
                Rgba::new(q(c.r), q(c.g), q(c.b), c.a)
            }
        }
    }

    /// Apply in place to every pixel of `pixmap` inside `region`.
    ///
    /// Fully transparent pixels are skipped: their colour is undefined, and
    /// touching them would make a later "trim transparent" produce different
    /// results.
    pub fn apply(&self, pixmap: &mut Pixmap, region: Option<IRect>) {
        let area = region
            .unwrap_or_else(|| pixmap.bounds())
            .intersect(&pixmap.bounds());
        if area.is_empty() {
            return;
        }
        // Channel-independent adjustments can run through a 256-entry LUT.
        if let Some(lut) = self.to_channel_luts() {
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    let c = pixmap.get(x, y);
                    if c.a == 0 {
                        continue;
                    }
                    pixmap.set(
                        x,
                        y,
                        Rgba8::new(
                            lut[0][c.r as usize],
                            lut[1][c.g as usize],
                            lut[2][c.b as usize],
                            c.a,
                        ),
                    );
                }
            }
            return;
        }
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let c = pixmap.get(x, y);
                if c.a == 0 {
                    continue;
                }
                pixmap.set(x, y, self.apply_color(c.to_rgba()).to_rgba8());
            }
        }
    }

    /// Per-channel lookup tables when the adjustment treats channels independently.
    fn to_channel_luts(&self) -> Option<[[u8; 256]; 3]> {
        let independent = matches!(
            self,
            Adjustment::BrightnessContrast { .. }
                | Adjustment::Exposure { .. }
                | Adjustment::Gamma { .. }
                | Adjustment::Levels { .. }
                | Adjustment::Curves { .. }
                | Adjustment::Invert
                | Adjustment::Posterize { .. }
                | Adjustment::ColorBalance { .. }
        );
        if !independent {
            return None;
        }
        let mut luts = [[0u8; 256]; 3];
        for (i, value) in (0..256).enumerate() {
            let v = value as f32 / 255.0;
            let out = self.apply_color(Rgba::new(v, v, v, 1.0));
            let to_u8 = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
            luts[0][i] = to_u8(out.r);
            luts[1][i] = to_u8(out.g);
            luts[2][i] = to_u8(out.b);
        }
        Some(luts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 6e-3
    }

    #[test]
    fn identity_curve_is_a_no_op() {
        let c = Curve::identity();
        for v in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert!(approx(c.eval(v), v), "{v} -> {}", c.eval(v));
        }
        assert!(c.is_identity());
    }

    #[test]
    fn curve_passes_through_its_control_points() {
        let c = Curve::new(vec![(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)]);
        assert!(approx(c.eval(0.5), 0.8));
        assert!(approx(c.eval(0.0), 0.0));
        assert!(approx(c.eval(1.0), 1.0));
    }

    #[test]
    fn curve_stays_in_range() {
        let c = Curve::new(vec![(0.0, 1.0), (0.5, 0.0), (1.0, 1.0)]);
        for i in 0..=100 {
            let v = c.eval(i as f32 / 100.0);
            assert!((0.0..=1.0).contains(&v), "curve escaped range: {v}");
        }
    }

    #[test]
    fn invert_flips_colour_but_keeps_alpha() {
        let out = Adjustment::Invert.apply_color(Rgba::new(0.2, 0.4, 0.6, 0.5));
        assert!(approx(out.r, 0.8) && approx(out.g, 0.6) && approx(out.b, 0.4));
        assert!(approx(out.a, 0.5));
    }

    #[test]
    fn grayscale_matches_luminance() {
        let c = Rgba::rgb(0.8, 0.2, 0.4);
        let out = Adjustment::Grayscale.apply_color(c);
        assert!(approx(out.r, c.luminance()));
        assert!(approx(out.r, out.g) && approx(out.g, out.b));
    }

    #[test]
    fn exposure_doubles_per_stop() {
        let out = Adjustment::Exposure { stops: 1.0 }.apply_color(Rgba::gray(0.25));
        assert!(approx(out.r, 0.5));
    }

    #[test]
    fn levels_stretch_the_range() {
        let a = Adjustment::Levels {
            in_black: 0.25,
            in_white: 0.75,
            gamma: 1.0,
            out_black: 0.0,
            out_white: 1.0,
        };
        assert!(approx(a.apply_color(Rgba::gray(0.25)).r, 0.0));
        assert!(approx(a.apply_color(Rgba::gray(0.75)).r, 1.0));
        assert!(approx(a.apply_color(Rgba::gray(0.5)).r, 0.5));
    }

    #[test]
    fn hue_rotation_moves_red_to_green() {
        let out = Adjustment::HueSaturation {
            hue: 120.0,
            saturation: 0.0,
            lightness: 0.0,
        }
        .apply_color(Rgba::rgb(1.0, 0.0, 0.0));
        assert!(out.g > 0.9 && out.r < 0.1, "got {out:?}");
    }

    #[test]
    fn saturation_minus_one_desaturates() {
        let out = Adjustment::HueSaturation {
            hue: 0.0,
            saturation: -1.0,
            lightness: 0.0,
        }
        .apply_color(Rgba::rgb(1.0, 0.0, 0.0));
        assert!(approx(out.r, out.g) && approx(out.g, out.b));
    }

    #[test]
    fn posterize_quantises() {
        let out = Adjustment::Posterize { levels: 2 }.apply_color(Rgba::gray(0.6));
        assert!(approx(out.r, 1.0));
        let out = Adjustment::Posterize { levels: 2 }.apply_color(Rgba::gray(0.4));
        assert!(approx(out.r, 0.0));
    }

    #[test]
    fn transparent_pixels_are_left_alone() {
        let mut pm = Pixmap::new(2, 1);
        pm.set(0, 0, Rgba8::new(10, 20, 30, 255));
        Adjustment::Invert.apply(&mut pm, None);
        assert_eq!(pm.get(0, 0), Rgba8::new(245, 235, 225, 255));
        assert_eq!(pm.get(1, 0), Rgba8::TRANSPARENT);
    }

    #[test]
    fn lut_path_matches_the_reference_path() {
        // Curves is LUT-accelerated; verify it agrees with per-pixel evaluation.
        let adj = Adjustment::Curves {
            master: Curve::new(vec![(0.0, 0.1), (0.5, 0.7), (1.0, 0.9)]),
            red: Curve::identity(),
            green: Curve::identity(),
            blue: Curve::identity(),
        };
        let mut pm = Pixmap::new(4, 1);
        for i in 0..4 {
            pm.set(i, 0, Rgba8::new((i * 60) as u8, 128, 200, 255));
        }
        let mut expected = pm.clone();
        for i in 0..4 {
            let c = expected.get(i, 0).to_rgba();
            expected.set(i, 0, adj.apply_color(c).to_rgba8());
        }
        adj.apply(&mut pm, None);
        for i in 0..4 {
            let (a, b) = (pm.get(i, 0), expected.get(i, 0));
            assert!(
                (a.r as i32 - b.r as i32).abs() <= 1 && (a.g as i32 - b.g as i32).abs() <= 1,
                "lut vs reference mismatch at {i}: {a:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn region_limits_the_effect() {
        let mut pm = Pixmap::filled(4, 1, Rgba8::BLACK);
        Adjustment::Invert.apply(&mut pm, Some(IRect::new(0, 0, 2, 1)));
        assert_eq!(pm.get(0, 0), Rgba8::WHITE);
        assert_eq!(pm.get(3, 0), Rgba8::BLACK);
    }
}
