//! The brush engine.
//!
//! A stroke is not painted directly onto the layer. Instead:
//!
//! ```text
//! input samples -> spacing resampler -> dabs -> stroke buffer (Mask)
//!                                                     |
//!                             layer <- fill_masked <--+  (once, at stroke opacity)
//! ```
//!
//! Accumulating coverage in a stroke buffer first is what separates *flow*
//! (how much paint each dab lays down, which builds up along the stroke) from
//! *opacity* (the ceiling for the stroke as a whole). It also means a stroke
//! that crosses itself does not darken at the crossing — the behaviour artists
//! expect from a marker or an inking pen.

use crate::mask::Mask;
use aether_core::blend::BlendMode;
use aether_core::input::InputSample;
use aether_core::math::{IRect, Vec2};
use serde::{Deserialize, Serialize};

/// Shape of a single dab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum BrushTip {
    /// Circular / elliptical tip with a soft edge.
    #[default]
    Round,
    /// Hard-edged square tip (pixel art, calligraphy).
    Square,
    /// An arbitrary shape sampled from a coverage image.
    ///
    /// This is what makes a *pattern brush*: the dab is whatever the artist
    /// drew or imported, scaled to the brush size and rotated with the tip.
    Stamp(crate::mask::Mask),
}

/// Where a brush texture's grain comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TexturePattern {
    /// Procedural grain; cheap, and needs no asset to travel with the preset.
    Noise {
        /// Seed, so the grain is stable across strokes, undo and reloads.
        seed: u64,
    },
    /// A tiled coverage image, e.g. scanned paper or canvas weave.
    Custom(crate::mask::Mask),
}

/// Paper-like grain modulating how much paint a dab lays down.
///
/// The texture is evaluated in **canvas space**, not dab space, so the grain
/// stays fixed to the paper as the brush travels over it — the behaviour that
/// makes a textured brush read as a physical medium rather than as a pattern
/// stuck to the cursor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushTexture {
    /// The grain source.
    pub pattern: TexturePattern,
    /// Size of one grain cell (or one tile of the custom image) in pixels.
    pub scale: f32,
    /// How strongly the grain bites, `0..=1`.
    pub strength: f32,
}

impl Default for BrushTexture {
    fn default() -> Self {
        Self {
            pattern: TexturePattern::Noise { seed: 1 },
            scale: 6.0,
            strength: 0.5,
        }
    }
}

impl BrushTexture {
    /// The coverage multiplier at a canvas pixel, in `0..=1`.
    pub fn coverage_at(&self, x: i32, y: i32) -> f32 {
        let strength = self.strength.clamp(0.0, 1.0);
        if strength <= 0.0 {
            return 1.0;
        }
        let scale = self.scale.max(0.5);
        let value = match &self.pattern {
            TexturePattern::Noise { seed } => {
                // Bilinear value noise: smoother than per-pixel hashing, and
                // still O(1) with no stored buffer.
                let fx = x as f32 / scale;
                let fy = y as f32 / scale;
                let (x0, y0) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0, fy - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let n = |cx: i64, cy: i64| texture_noise(cx, cy, *seed);
                let top = n(x0, y0) + (n(x0 + 1, y0) - n(x0, y0)) * tx;
                let bottom = n(x0, y0 + 1) + (n(x0 + 1, y0 + 1) - n(x0, y0 + 1)) * tx;
                top + (bottom - top) * ty
            }
            TexturePattern::Custom(mask) => {
                if mask.width() == 0 || mask.height() == 0 {
                    return 1.0;
                }
                let tile_w = (mask.width() as f32 * scale / 32.0).max(1.0);
                let tile_h = (mask.height() as f32 * scale / 32.0).max(1.0);
                let u = (x as f32 / tile_w).rem_euclid(1.0) * mask.width() as f32;
                let v = (y as f32 / tile_h).rem_euclid(1.0) * mask.height() as f32;
                mask.get(u as i32, v as i32) as f32 / 255.0
            }
        };
        1.0 - strength * (1.0 - value.clamp(0.0, 1.0))
    }
}

/// Deterministic value noise in `0..=1`.
fn texture_noise(x: i64, y: i64, seed: u64) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed;
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    ((h >> 40) as f32) / 16_777_216.0
}

/// How live input drives brush parameters.
///
/// Each field is the amount of influence in `0..=1`: `0` means the parameter
/// ignores the input entirely, `1` means it scales all the way down to
/// `*_min` at zero input.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushDynamics {
    /// Pressure influence on dab diameter.
    pub size_pressure: f32,
    /// Pressure influence on dab coverage.
    pub flow_pressure: f32,
    /// Pressure influence on stroke opacity.
    pub opacity_pressure: f32,
    /// Speed influence on dab diameter (faster = thinner).
    pub size_velocity: f32,
    /// Smallest diameter multiplier reachable through dynamics.
    pub size_min: f32,
    /// Smallest coverage multiplier reachable through dynamics.
    pub flow_min: f32,
    /// Tilt influence on the dab's elongation.
    pub tilt_elongation: f32,
}

impl Default for BrushDynamics {
    fn default() -> Self {
        Self {
            size_pressure: 1.0,
            flow_pressure: 0.0,
            opacity_pressure: 0.0,
            size_velocity: 0.0,
            size_min: 0.05,
            flow_min: 0.0,
            tilt_elongation: 0.0,
        }
    }
}

/// A named, serialisable brush configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushPreset {
    /// Display name.
    pub name: String,
    /// Dab shape.
    pub tip: BrushTip,
    /// Diameter in document pixels.
    pub size: f32,
    /// Stroke opacity ceiling, `0..=1`.
    pub opacity: f32,
    /// Per-dab coverage, `0..=1`.
    pub flow: f32,
    /// Edge sharpness: `1` is a hard edge, `0` a full gradient.
    pub hardness: f32,
    /// Distance between dabs as a fraction of the diameter.
    pub spacing: f32,
    /// Tip rotation in radians.
    pub angle: f32,
    /// Tip aspect ratio, `0..=1` (`1` is circular).
    pub roundness: f32,
    /// Random per-dab offset as a fraction of the diameter.
    pub scatter: f32,
    /// Random per-dab size variation, `0..=1`.
    pub size_jitter: f32,
    /// Optional paper grain.
    #[serde(default)]
    pub texture: Option<BrushTexture>,
    /// How the stroke is combined with the layer.
    pub blend: BlendMode,
    /// Input mapping.
    pub dynamics: BrushDynamics,
    /// Input smoothing, `0..=1`; higher values lag the cursor for steadier lines.
    pub smoothing: f32,
}

impl Default for BrushPreset {
    fn default() -> Self {
        Self {
            name: "Round Pen".into(),
            tip: BrushTip::Round,
            size: 24.0,
            opacity: 1.0,
            flow: 1.0,
            hardness: 0.85,
            spacing: 0.08,
            angle: 0.0,
            roundness: 1.0,
            scatter: 0.0,
            size_jitter: 0.0,
            texture: None,
            blend: BlendMode::Normal,
            dynamics: BrushDynamics::default(),
            smoothing: 0.35,
        }
    }
}

impl BrushPreset {
    /// The built-in presets available on a fresh install.
    ///
    /// These are ordinary [`BrushPreset`] values with no special casing, so a
    /// user preset and a built-in behave identically.
    pub fn builtin() -> Vec<BrushPreset> {
        vec![
            BrushPreset {
                name: "Pencil".into(),
                size: 6.0,
                hardness: 0.9,
                flow: 0.85,
                spacing: 0.06,
                dynamics: BrushDynamics {
                    size_pressure: 0.6,
                    flow_pressure: 0.8,
                    size_min: 0.35,
                    ..Default::default()
                },
                ..Default::default()
            },
            BrushPreset {
                name: "Ink Pen".into(),
                size: 12.0,
                hardness: 1.0,
                flow: 1.0,
                spacing: 0.05,
                smoothing: 0.5,
                dynamics: BrushDynamics {
                    size_pressure: 1.0,
                    size_min: 0.05,
                    ..Default::default()
                },
                ..Default::default()
            },
            BrushPreset {
                name: "Soft Airbrush".into(),
                size: 80.0,
                hardness: 0.0,
                opacity: 0.6,
                flow: 0.12,
                spacing: 0.03,
                dynamics: BrushDynamics {
                    size_pressure: 0.2,
                    flow_pressure: 1.0,
                    size_min: 0.7,
                    flow_min: 0.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            BrushPreset {
                name: "Marker".into(),
                size: 30.0,
                hardness: 0.65,
                opacity: 0.8,
                flow: 1.0,
                spacing: 0.05,
                roundness: 0.35,
                angle: std::f32::consts::FRAC_PI_4,
                dynamics: BrushDynamics {
                    size_pressure: 0.15,
                    size_min: 0.85,
                    ..Default::default()
                },
                ..Default::default()
            },
            BrushPreset {
                name: "Watercolor".into(),
                size: 60.0,
                hardness: 0.15,
                opacity: 0.45,
                flow: 0.2,
                spacing: 0.1,
                scatter: 0.15,
                size_jitter: 0.25,
                texture: Some(BrushTexture {
                    pattern: TexturePattern::Noise { seed: 24 },
                    scale: 5.0,
                    strength: 0.6,
                }),
                dynamics: BrushDynamics {
                    size_pressure: 0.5,
                    flow_pressure: 0.7,
                    size_min: 0.4,
                    ..Default::default()
                },
                ..Default::default()
            },
            BrushPreset {
                name: "Pixel".into(),
                tip: BrushTip::Square,
                size: 1.0,
                hardness: 1.0,
                opacity: 1.0,
                flow: 1.0,
                spacing: 0.5,
                smoothing: 0.0,
                dynamics: BrushDynamics {
                    size_pressure: 0.0,
                    size_min: 1.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        ]
    }

    /// Clamp every parameter into its legal range.
    ///
    /// Called after deserialisation and after UI edits so no downstream code
    /// has to defend against a negative radius or a spacing of zero.
    pub fn sanitize(&mut self) {
        self.size = self.size.clamp(0.1, 5000.0);
        self.opacity = self.opacity.clamp(0.0, 1.0);
        self.flow = self.flow.clamp(0.0, 1.0);
        self.hardness = self.hardness.clamp(0.0, 1.0);
        self.spacing = self.spacing.clamp(0.01, 4.0);
        self.roundness = self.roundness.clamp(0.05, 1.0);
        self.scatter = self.scatter.clamp(0.0, 4.0);
        self.size_jitter = self.size_jitter.clamp(0.0, 1.0);
        self.smoothing = self.smoothing.clamp(0.0, 0.95);
        self.dynamics.size_min = self.dynamics.size_min.clamp(0.0, 1.0);
        self.dynamics.flow_min = self.dynamics.flow_min.clamp(0.0, 1.0);
    }
}

/// A tiny deterministic PRNG for scatter and jitter.
///
/// Deterministic on purpose: replaying a stroke (undo, then redo, or a
/// regression test) must produce identical pixels.
#[derive(Clone, Copy, Debug)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_f32(&mut self) -> f32 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 40) as f32) / (16_777_216.0)
    }

    /// Uniform in `-1..=1`.
    fn next_signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

/// Parameters of one resolved dab, after dynamics have been applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    /// Centre in document pixels.
    pub center: Vec2,
    /// Diameter in document pixels.
    pub size: f32,
    /// Coverage laid down by this dab, `0..=1`.
    pub flow: f32,
    /// Tip rotation in radians.
    pub angle: f32,
    /// Tip aspect ratio.
    pub roundness: f32,
}

/// Stateless dab rasteriser.
pub struct BrushEngine;

impl BrushEngine {
    /// Stamp one dab into `buffer`, accumulating coverage.
    ///
    /// Returns the touched rectangle. Coverage accumulates as source-over
    /// (`a' = a + f(1-a)`) so repeated dabs at low flow build up smoothly and
    /// a full-flow dab saturates immediately.
    pub fn stamp(
        buffer: &mut Mask,
        dab: &Dab,
        tip: &BrushTip,
        hardness: f32,
        texture: Option<&BrushTexture>,
    ) -> IRect {
        let radius = (dab.size * 0.5).max(0.0);
        if radius <= 0.0 || dab.flow <= 0.0 {
            return IRect::EMPTY;
        }
        // A 1px brush should still cover exactly one pixel.
        let reach = radius.max(0.5) + 1.0;
        let rect = IRect::from_bounds(
            (dab.center.x - reach).floor() as i32,
            (dab.center.y - reach).floor() as i32,
            (dab.center.x + reach).ceil() as i32,
            (dab.center.y + reach).ceil() as i32,
        )
        .intersect(&buffer.bounds());
        if rect.is_empty() {
            return IRect::EMPTY;
        }

        let (sin_a, cos_a) = (-dab.angle).sin_cos();
        let inv_roundness = 1.0 / dab.roundness.clamp(0.05, 1.0);
        let flow = dab.flow.clamp(0.0, 1.0);

        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                // Pixel centre relative to the dab, rotated into tip space.
                let dx = x as f32 + 0.5 - dab.center.x;
                let dy = y as f32 + 0.5 - dab.center.y;
                let rx = dx * cos_a - dy * sin_a;
                let ry = (dx * sin_a + dy * cos_a) * inv_roundness;

                let mut coverage = match tip {
                    BrushTip::Round => round_coverage(rx, ry, radius, hardness),
                    BrushTip::Square => square_coverage(rx, ry, radius),
                    BrushTip::Stamp(shape) => stamp_coverage(shape, rx, ry, radius),
                };
                if coverage <= 0.0 {
                    continue;
                }
                if let Some(texture) = texture {
                    coverage *= texture.coverage_at(x, y);
                    if coverage <= 0.0 {
                        continue;
                    }
                }
                let add = coverage * flow;
                let prev = buffer.get(x, y) as f32 / 255.0;
                let next = prev + add * (1.0 - prev);
                buffer.set(x, y, (next * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
        rect
    }
}

/// Antialiased circular falloff.
fn round_coverage(rx: f32, ry: f32, radius: f32, hardness: f32) -> f32 {
    let d = (rx * rx + ry * ry).sqrt();
    if radius <= 0.75 {
        // Sub-pixel brushes degrade to a single antialiased pixel.
        return (1.0 - d).clamp(0.0, 1.0);
    }
    // Inner plateau, then a smooth ramp to the rim. One pixel of the ramp is
    // always reserved for antialiasing so even a "hard" brush is not jagged.
    let inner = radius * hardness.clamp(0.0, 1.0);
    let inner = inner.min(radius - 1.0).max(0.0);
    if d <= inner {
        1.0
    } else if d >= radius {
        0.0
    } else {
        let t = (d - inner) / (radius - inner).max(1e-5);
        // smoothstep
        let t = t.clamp(0.0, 1.0);
        1.0 - (t * t * (3.0 - 2.0 * t))
    }
}

/// Coverage sampled from a stamp image, scaled to the dab and bilinearly filtered.
fn stamp_coverage(shape: &Mask, rx: f32, ry: f32, radius: f32) -> f32 {
    if shape.width() == 0 || shape.height() == 0 || radius <= 0.0 {
        return 0.0;
    }
    // Map dab space (-radius..radius) onto the image.
    let u = (rx / radius * 0.5 + 0.5) * shape.width() as f32 - 0.5;
    let v = (ry / radius * 0.5 + 0.5) * shape.height() as f32 - 0.5;
    let (x0, y0) = (u.floor(), v.floor());
    let (tx, ty) = (u - x0, v - y0);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let at = |x: i32, y: i32| shape.get(x, y) as f32 / 255.0;
    let top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * tx;
    (top + (bottom - top) * ty).clamp(0.0, 1.0)
}

/// Hard square falloff with a single antialiased pixel at the border.
fn square_coverage(rx: f32, ry: f32, radius: f32) -> f32 {
    let d = rx.abs().max(ry.abs());
    if radius <= 0.75 {
        return if d <= 0.5 { 1.0 } else { 0.0 };
    }
    if d <= radius - 0.5 {
        1.0
    } else if d >= radius + 0.5 {
        0.0
    } else {
        radius + 0.5 - d
    }
}

/// Live state of one in-progress stroke.
///
/// The stroke owns its coverage buffer; the document decides when to flush it
/// onto a layer (continuously for live preview, and once at stroke end for the
/// undoable command).
pub struct StrokeState {
    preset: BrushPreset,
    buffer: Mask,
    rng: Rng,
    last: Option<InputSample>,
    smoothed: Option<Vec2>,
    leftover: f32,
    dirty: IRect,
    dab_count: u32,
}

impl StrokeState {
    /// Begin a stroke covering a document of `width` x `height`.
    pub fn begin(preset: BrushPreset, width: u32, height: u32, seed: u64) -> Self {
        let mut preset = preset;
        preset.sanitize();
        Self {
            preset,
            buffer: Mask::new(width, height),
            rng: Rng::new(seed),
            last: None,
            smoothed: None,
            leftover: 0.0,
            dirty: IRect::EMPTY,
            dab_count: 0,
        }
    }

    /// The preset this stroke was started with.
    pub fn preset(&self) -> &BrushPreset {
        &self.preset
    }

    /// The accumulated coverage buffer.
    pub fn buffer(&self) -> &Mask {
        &self.buffer
    }

    /// Region touched since the stroke began.
    pub fn dirty(&self) -> IRect {
        self.dirty
    }

    /// How many dabs have been stamped.
    pub fn dab_count(&self) -> u32 {
        self.dab_count
    }

    /// True when the stroke has not put down any paint.
    pub fn is_empty(&self) -> bool {
        self.dab_count == 0
    }

    /// Feed one input sample, stamping every dab the spacing rule calls for.
    ///
    /// Returns the region touched by this call.
    pub fn push(&mut self, sample: InputSample) -> IRect {
        let position = self.smooth(sample.position);
        let sample = InputSample { position, ..sample };

        let mut touched = IRect::EMPTY;
        match self.last {
            None => {
                touched = touched.union(&self.stamp_at(&sample));
                self.leftover = 0.0;
            }
            Some(prev) => {
                let delta = sample.position - prev.position;
                let distance = delta.length();
                if distance <= f32::EPSILON {
                    return IRect::EMPTY;
                }
                let step = self.spacing_for(&sample).max(0.5);
                let mut travelled = self.leftover;
                while travelled + step <= distance {
                    travelled += step;
                    let t = travelled / distance;
                    let interpolated = prev.lerp(&sample, t);
                    touched = touched.union(&self.stamp_at(&interpolated));
                }
                self.leftover = travelled - distance;
            }
        }
        self.last = Some(sample);
        self.dirty = self.dirty.union(&touched);
        touched
    }

    /// Exponential smoothing of the cursor path (stabiliser).
    fn smooth(&mut self, position: Vec2) -> Vec2 {
        let s = self.preset.smoothing;
        let out = match self.smoothed {
            Some(prev) if s > 0.0 => prev.lerp(position, 1.0 - s),
            _ => position,
        };
        self.smoothed = Some(out);
        out
    }

    fn spacing_for(&self, sample: &InputSample) -> f32 {
        let size = self.size_for(sample);
        (size * self.preset.spacing).max(0.5)
    }

    fn size_for(&self, sample: &InputSample) -> f32 {
        let d = &self.preset.dynamics;
        let mut factor = 1.0;
        if d.size_pressure > 0.0 {
            let p = sample.pressure.clamp(0.0, 1.0);
            factor *= 1.0 - d.size_pressure * (1.0 - p);
        }
        if d.size_velocity > 0.0 {
            // Normalise speed with a soft knee at 1000 px/s.
            let v = (sample.velocity / 1000.0).clamp(0.0, 1.0);
            factor *= 1.0 - d.size_velocity * v;
        }
        (self.preset.size * factor.max(d.size_min)).max(0.1)
    }

    fn flow_for(&self, sample: &InputSample) -> f32 {
        let d = &self.preset.dynamics;
        let mut factor = 1.0;
        if d.flow_pressure > 0.0 {
            let p = sample.pressure.clamp(0.0, 1.0);
            factor *= 1.0 - d.flow_pressure * (1.0 - p);
        }
        (self.preset.flow * factor.max(d.flow_min)).clamp(0.0, 1.0)
    }

    /// Stroke opacity after pressure dynamics; used when flushing to a layer.
    pub fn opacity_for_last_sample(&self) -> f32 {
        let d = &self.preset.dynamics;
        match self.last {
            Some(s) if d.opacity_pressure > 0.0 => {
                (self.preset.opacity * (1.0 - d.opacity_pressure * (1.0 - s.pressure))).clamp(0.0, 1.0)
            }
            _ => self.preset.opacity,
        }
    }

    fn stamp_at(&mut self, sample: &InputSample) -> IRect {
        let mut size = self.size_for(sample);
        if self.preset.size_jitter > 0.0 {
            size *= 1.0 + self.rng.next_signed() * self.preset.size_jitter;
            size = size.max(0.1);
        }
        let mut center = sample.position;
        if self.preset.scatter > 0.0 {
            let amount = self.preset.size * self.preset.scatter * 0.5;
            center += Vec2::new(self.rng.next_signed() * amount, self.rng.next_signed() * amount);
        }
        let mut roundness = self.preset.roundness;
        let mut angle = self.preset.angle;
        let tilt = sample.tilt;
        if self.preset.dynamics.tilt_elongation > 0.0 && tilt.length() > 1e-3 {
            let amount = tilt.length().clamp(0.0, 1.0) * self.preset.dynamics.tilt_elongation;
            roundness = (roundness * (1.0 - amount)).max(0.05);
            angle = tilt.angle();
        }

        let dab = Dab {
            center,
            size,
            flow: self.flow_for(sample),
            angle,
            roundness,
        };
        self.dab_count += 1;
        BrushEngine::stamp(
            &mut self.buffer,
            &dab,
            &self.preset.tip,
            self.preset.hardness,
            self.preset.texture.as_ref(),
        )
    }

    /// Finish the stroke and hand back the accumulated coverage.
    pub fn finish(self) -> (Mask, IRect) {
        (self.buffer, self.dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(x: f32, y: f32) -> InputSample {
        InputSample::at(Vec2::new(x, y))
    }

    #[test]
    fn a_single_dab_covers_its_centre() {
        let mut mask = Mask::new(32, 32);
        let dab = Dab {
            center: Vec2::new(16.0, 16.0),
            size: 10.0,
            flow: 1.0,
            angle: 0.0,
            roundness: 1.0,
        };
        let rect = BrushEngine::stamp(&mut mask, &dab, &BrushTip::Round, 1.0, None);
        assert!(!rect.is_empty());
        assert_eq!(mask.get(16, 16), 255);
        assert_eq!(mask.get(31, 31), 0);
    }

    #[test]
    fn soft_brushes_fade_towards_the_rim() {
        let mut mask = Mask::new(64, 64);
        let dab = Dab {
            center: Vec2::new(32.0, 32.0),
            size: 40.0,
            flow: 1.0,
            angle: 0.0,
            roundness: 1.0,
        };
        BrushEngine::stamp(&mut mask, &dab, &BrushTip::Round, 0.0, None);
        let centre = mask.get(32, 32);
        let mid = mask.get(32 + 14, 32);
        assert!(centre > 250, "centre should be near-solid, got {centre}");
        assert!(
            mid > 0 && mid < centre,
            "expected falloff, centre={centre} mid={mid}"
        );
    }

    #[test]
    fn one_pixel_brush_covers_one_pixel() {
        let mut mask = Mask::new(8, 8);
        let dab = Dab {
            center: Vec2::new(4.5, 4.5),
            size: 1.0,
            flow: 1.0,
            angle: 0.0,
            roundness: 1.0,
        };
        BrushEngine::stamp(&mut mask, &dab, &BrushTip::Square, 1.0, None);
        assert_eq!(mask.get(4, 4), 255);
        assert_eq!(mask.get(5, 4), 0);
        assert_eq!(mask.get(3, 4), 0);
    }

    #[test]
    fn dabs_accumulate_but_saturate() {
        let mut mask = Mask::new(16, 16);
        let dab = Dab {
            center: Vec2::new(8.0, 8.0),
            size: 6.0,
            flow: 0.5,
            angle: 0.0,
            roundness: 1.0,
        };
        BrushEngine::stamp(&mut mask, &dab, &BrushTip::Round, 1.0, None);
        let first = mask.get(8, 8);
        BrushEngine::stamp(&mut mask, &dab, &BrushTip::Round, 1.0, None);
        let second = mask.get(8, 8);
        assert!(second > first, "flow should build up: {first} -> {second}");
        for _ in 0..20 {
            BrushEngine::stamp(&mut mask, &dab, &BrushTip::Round, 1.0, None);
        }
        assert_eq!(mask.get(8, 8), 255);
    }

    #[test]
    fn a_stroke_lays_down_a_continuous_line() {
        let preset = BrushPreset {
            size: 8.0,
            spacing: 0.1,
            smoothing: 0.0,
            ..Default::default()
        };
        let mut stroke = StrokeState::begin(preset, 128, 32, 1);
        stroke.push(sample(8.0, 16.0));
        stroke.push(sample(120.0, 16.0));
        assert!(
            stroke.dab_count() > 100,
            "expected many dabs, got {}",
            stroke.dab_count()
        );
        for x in (10..118).step_by(7) {
            assert_eq!(stroke.buffer().get(x, 16), 255, "gap at x={x}");
        }
    }

    #[test]
    fn spacing_controls_dab_count() {
        let dense = BrushPreset {
            size: 10.0,
            spacing: 0.1,
            smoothing: 0.0,
            ..Default::default()
        };
        let sparse = BrushPreset {
            size: 10.0,
            spacing: 1.0,
            smoothing: 0.0,
            ..Default::default()
        };
        let mut a = StrokeState::begin(dense, 128, 32, 1);
        a.push(sample(0.0, 16.0));
        a.push(sample(100.0, 16.0));
        let mut b = StrokeState::begin(sparse, 128, 32, 1);
        b.push(sample(0.0, 16.0));
        b.push(sample(100.0, 16.0));
        assert!(a.dab_count() > b.dab_count() * 5);
    }

    #[test]
    fn pressure_dynamics_thin_the_line() {
        let preset = BrushPreset {
            size: 20.0,
            spacing: 0.1,
            smoothing: 0.0,
            hardness: 1.0,
            dynamics: BrushDynamics {
                size_pressure: 1.0,
                size_min: 0.1,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut light = StrokeState::begin(preset.clone(), 64, 64, 1);
        light.push(sample(32.0, 32.0).with_pressure(0.1));
        let mut heavy = StrokeState::begin(preset, 64, 64, 1);
        heavy.push(sample(32.0, 32.0).with_pressure(1.0));
        assert!(light.dirty().width < heavy.dirty().width);
    }

    #[test]
    fn strokes_are_deterministic() {
        let preset = BrushPreset {
            scatter: 0.5,
            size_jitter: 0.5,
            smoothing: 0.0,
            ..Default::default()
        };
        let run = || {
            let mut s = StrokeState::begin(preset.clone(), 64, 64, 42);
            s.push(sample(10.0, 32.0));
            s.push(sample(50.0, 32.0));
            s.finish().0
        };
        assert!(run() == run(), "same seed must produce identical coverage");
    }

    #[test]
    fn zero_length_move_does_not_stamp_again() {
        let mut stroke = StrokeState::begin(BrushPreset::default(), 32, 32, 1);
        stroke.push(sample(16.0, 16.0));
        let before = stroke.dab_count();
        stroke.push(sample(16.0, 16.0));
        assert_eq!(stroke.dab_count(), before);
    }

    #[test]
    fn builtin_presets_are_sane() {
        for mut preset in BrushPreset::builtin() {
            let before = preset.clone();
            preset.sanitize();
            assert_eq!(before, preset, "preset {} needed clamping", preset.name);
        }
    }
}

#[cfg(test)]
mod texture_tests {
    use super::*;
    use aether_core::math::IRect;

    fn dab(size: f32) -> Dab {
        Dab {
            center: Vec2::new(32.0, 32.0),
            size,
            flow: 1.0,
            angle: 0.0,
            roundness: 1.0,
        }
    }

    #[test]
    fn noise_texture_breaks_up_solid_coverage() {
        let texture = BrushTexture {
            pattern: TexturePattern::Noise { seed: 3 },
            scale: 4.0,
            strength: 1.0,
        };
        let mut plain = Mask::new(64, 64);
        let mut grainy = Mask::new(64, 64);
        BrushEngine::stamp(&mut plain, &dab(30.0), &BrushTip::Round, 1.0, None);
        BrushEngine::stamp(&mut grainy, &dab(30.0), &BrushTip::Round, 1.0, Some(&texture));
        assert_ne!(plain, grainy, "the texture should change coverage");
        assert!(grainy.get(32, 32) <= plain.get(32, 32));
    }

    #[test]
    fn texture_is_fixed_to_the_canvas_not_the_dab() {
        let texture = BrushTexture {
            pattern: TexturePattern::Noise { seed: 5 },
            scale: 8.0,
            strength: 1.0,
        };
        // The same canvas pixel must get the same grain regardless of where the
        // dab that covers it was centred.
        let a = texture.coverage_at(40, 40);
        let b = texture.coverage_at(40, 40);
        assert_eq!(a, b);
        assert_ne!(texture.coverage_at(40, 40), texture.coverage_at(41, 96));
    }

    #[test]
    fn zero_strength_texture_is_transparent_to_the_engine() {
        let texture = BrushTexture {
            strength: 0.0,
            ..Default::default()
        };
        assert_eq!(texture.coverage_at(3, 9), 1.0);
    }

    #[test]
    fn a_stamp_tip_takes_the_shape_of_its_image() {
        // A shape covering only the left half of its image.
        let mut shape = Mask::new(16, 16);
        shape.fill_rect(IRect::new(0, 0, 8, 16), 255);
        let mut mask = Mask::new(64, 64);
        BrushEngine::stamp(&mut mask, &dab(32.0), &BrushTip::Stamp(shape), 1.0, None);
        assert!(mask.get(24, 32) > 200, "left half should be covered");
        assert_eq!(mask.get(44, 32), 0, "right half should be empty");
    }

    #[test]
    fn an_empty_stamp_paints_nothing() {
        let mut mask = Mask::new(32, 32);
        BrushEngine::stamp(
            &mut mask,
            &dab(16.0),
            &BrushTip::Stamp(Mask::new(0, 0)),
            1.0,
            None,
        );
        assert!(mask.is_empty());
    }

    #[test]
    fn presets_with_textures_survive_serde() {
        for preset in BrushPreset::builtin() {
            let text = serde_json::to_string(&preset).expect("serialize");
            let back: BrushPreset = serde_json::from_str(&text).expect("deserialize");
            assert_eq!(back, preset);
        }
    }

    #[test]
    fn presets_saved_before_textures_existed_still_load() {
        // `texture` is a later addition; older presets simply omit the field.
        let legacy = r#"{
            "name": "Old", "tip": "Round", "size": 10.0, "opacity": 1.0, "flow": 1.0,
            "hardness": 0.8, "spacing": 0.1, "angle": 0.0, "roundness": 1.0,
            "scatter": 0.0, "size_jitter": 0.0, "blend": "Normal",
            "dynamics": {"size_pressure":1.0,"flow_pressure":0.0,"opacity_pressure":0.0,
                         "size_velocity":0.0,"size_min":0.05,"flow_min":0.0,"tilt_elongation":0.0},
            "smoothing": 0.3
        }"#;
        let preset: BrushPreset = serde_json::from_str(legacy).expect("legacy preset should load");
        assert_eq!(preset.name, "Old");
        assert!(preset.texture.is_none());
    }
}
