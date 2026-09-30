//! Motions, the animator and expressions.
//!
//! A [`Motion`] is a clip: one [`Track`] of keyframes per animated parameter.
//! Because every moving thing in a rig is a parameter, one timeline animates
//! meshes, deformers, bones, opacity, tint and draw order alike — and a motion
//! authored for one rig plays on any other rig that uses the same parameter
//! names.
//!
//! Keyframes carry their own interpolation for the segment that follows
//! them: step, linear, cubic Bézier (with overshoot allowed), the classic
//! ease families, and physically based spring, elastic and bounce curves that
//! would otherwise have to be faked with dozens of keys.
//!
//! The [`Animator`] plays several motions at once in layers, each either
//! overriding or adding to what is below, with crossfades on start and stop.
//! [`Expression`]s are named parameter presets (smile, angry, surprised)
//! blended on top with add, multiply or overwrite semantics.

use crate::param::{ParamValues, Parameter};
use aether_core::{AetherError, ParameterId, Result};
use serde::{Deserialize, Serialize};

/// How a segment between two keyframes is shaped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    /// Hold the value until the next key.
    Step,
    /// Straight line.
    #[default]
    Linear,
    /// Cubic Bézier timing curve through `(0,0)`, `(x1,y1)`, `(x2,y2)`,
    /// `(1,1)`. `y` may leave `0..=1` for anticipation and overshoot.
    Bezier {
        /// First handle, time fraction.
        x1: f32,
        /// First handle, value fraction.
        y1: f32,
        /// Second handle, time fraction.
        x2: f32,
        /// Second handle, value fraction.
        y2: f32,
    },
    /// Slow start.
    EaseIn,
    /// Slow end.
    EaseOut,
    /// Slow start and end.
    EaseInOut,
    /// Pulls back before moving.
    BackIn,
    /// Overshoots and settles.
    BackOut,
    /// Rings like a plucked string at the end.
    ElasticOut,
    /// Bounces to rest.
    BounceOut,
    /// A damped spring toward the next key.
    Spring {
        /// Oscillations over the segment.
        frequency: f32,
        /// Damping ratio (1 = no overshoot).
        damping: f32,
    },
}

impl Easing {
    /// The presets offered in menus (Bézier and spring use their defaults).
    pub const PRESETS: [Easing; 11] = [
        Easing::Step,
        Easing::Linear,
        Easing::Bezier {
            x1: 0.33,
            y1: 0.0,
            x2: 0.67,
            y2: 1.0,
        },
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::BackIn,
        Easing::BackOut,
        Easing::ElasticOut,
        Easing::BounceOut,
        Easing::Spring {
            frequency: 2.5,
            damping: 0.35,
        },
    ];

    /// Display name.
    pub fn name(&self) -> &'static str {
        match self {
            Easing::Step => "Step",
            Easing::Linear => "Linear",
            Easing::Bezier { .. } => "Bézier",
            Easing::EaseIn => "Ease in",
            Easing::EaseOut => "Ease out",
            Easing::EaseInOut => "Ease in-out",
            Easing::BackIn => "Back in",
            Easing::BackOut => "Back out",
            Easing::ElasticOut => "Elastic",
            Easing::BounceOut => "Bounce",
            Easing::Spring { .. } => "Spring",
        }
    }

    /// Map segment progress `t` in `0..=1` to value progress.
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match *self {
            Easing::Step => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Easing::Linear => t,
            Easing::Bezier { x1, y1, x2, y2 } => {
                cubic_bezier(x1.clamp(0.0, 1.0), y1, x2.clamp(0.0, 1.0), y2, t)
            }
            Easing::EaseIn => t * t * t,
            Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
            Easing::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Easing::BackIn => {
                let c = 1.70158;
                (c + 1.0) * t * t * t - c * t * t
            }
            Easing::BackOut => {
                let c = 1.70158;
                let u = t - 1.0;
                1.0 + (c + 1.0) * u * u * u + c * u * u
            }
            Easing::ElasticOut => {
                if t <= 0.0 || t >= 1.0 {
                    t
                } else {
                    let c = std::f32::consts::TAU / 3.0;
                    2f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c).sin() + 1.0
                }
            }
            Easing::BounceOut => bounce_out(t),
            Easing::Spring { frequency, damping } => {
                if t >= 1.0 {
                    return 1.0;
                }
                let zeta = damping.clamp(0.02, 1.0);
                let omega = std::f32::consts::TAU * frequency.clamp(0.1, 20.0);
                let s = |t: f32| {
                    let wd = omega * (1.0 - zeta * zeta).max(1e-4).sqrt();
                    1.0 - (-zeta * omega * t).exp() * ((wd * t).cos() + zeta * omega / wd * (wd * t).sin())
                };
                // Force the curve to land exactly on the next key.
                let end = s(1.0);
                s(t) + (1.0 - end) * t * t * t
            }
        }
    }
}

fn bounce_out(t: f32) -> f32 {
    let n = 7.5625;
    let d = 2.75;
    if t < 1.0 / d {
        n * t * t
    } else if t < 2.0 / d {
        let t = t - 1.5 / d;
        n * t * t + 0.75
    } else if t < 2.5 / d {
        let t = t - 2.25 / d;
        n * t * t + 0.9375
    } else {
        let t = t - 2.625 / d;
        n * t * t + 0.984375
    }
}

/// Evaluate a CSS-style timing curve: solve `x(s) = t` then return `y(s)`.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let bez = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let dbez = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * a + 6.0 * u * s * (b - a) + 3.0 * s * s * (1.0 - b)
    };
    // Newton, falling back to bisection.
    let mut s = t;
    for _ in 0..8 {
        let x = bez(x1, x2, s) - t;
        let d = dbez(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - x / d).clamp(0.0, 1.0);
    }
    if (bez(x1, x2, s) - t).abs() > 1e-4 {
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..40 {
            s = (lo + hi) * 0.5;
            if bez(x1, x2, s) < t {
                lo = s;
            } else {
                hi = s;
            }
        }
    }
    bez(y1, y2, s)
}

/// One key.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Seconds from the start of the motion.
    pub time: f32,
    /// Parameter value.
    pub value: f32,
    /// Shape of the segment from this key to the next.
    #[serde(default)]
    pub easing: Easing,
}

/// Keyframes for one parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// The animated parameter.
    pub param: ParameterId,
    /// Keys sorted by time.
    pub keys: Vec<Keyframe>,
    /// Whether the track plays.
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Two keys closer than this in time are the same key.
pub const TIME_EPSILON: f32 = 1e-4;

impl Track {
    /// An empty track.
    pub fn new(param: ParameterId) -> Self {
        Self {
            param,
            keys: Vec::new(),
            enabled: true,
        }
    }

    /// Value at `time` (holding the first and last keys outside them).
    pub fn sample(&self, time: f32) -> Option<f32> {
        let first = self.keys.first()?;
        if time <= first.time {
            return Some(first.value);
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if time <= b.time {
                let span = b.time - a.time;
                let t = if span > 1e-6 { (time - a.time) / span } else { 1.0 };
                return Some(a.value + (b.value - a.value) * a.easing.apply(t));
            }
        }
        self.keys.last().map(|k| k.value)
    }

    /// Insert a key, replacing one at the same time. Returns its index.
    pub fn set_key(&mut self, time: f32, value: f32) -> usize {
        if let Some(i) = self
            .keys
            .iter()
            .position(|k| (k.time - time).abs() < TIME_EPSILON)
        {
            self.keys[i].value = value;
            return i;
        }
        // A new key inherits the easing of the segment it splits.
        let easing = self
            .keys
            .iter()
            .rev()
            .find(|k| k.time < time)
            .map(|k| k.easing)
            .unwrap_or_default();
        let index = self
            .keys
            .iter()
            .position(|k| k.time > time)
            .unwrap_or(self.keys.len());
        self.keys.insert(index, Keyframe { time, value, easing });
        index
    }

    /// The key at `time`, if any.
    pub fn key_at(&self, time: f32) -> Option<usize> {
        self.keys
            .iter()
            .position(|k| (k.time - time).abs() < TIME_EPSILON)
    }

    /// Move a key in time, keeping the list sorted. Returns its new index.
    pub fn move_key(&mut self, index: usize, time: f32) -> Option<usize> {
        if index >= self.keys.len() {
            return None;
        }
        let mut key = self.keys.remove(index);
        key.time = time.max(0.0);
        self.keys.retain(|k| (k.time - key.time).abs() >= TIME_EPSILON);
        let at = self
            .keys
            .iter()
            .position(|k| k.time > key.time)
            .unwrap_or(self.keys.len());
        self.keys.insert(at, key);
        Some(at)
    }
}

/// A named event on the timeline (a sound cue, a hook for the host app).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionEvent {
    /// Seconds from the start.
    pub time: f32,
    /// Event name.
    pub name: String,
}

/// An animation clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    /// Display name.
    pub name: String,
    /// Length in seconds.
    pub duration: f32,
    /// Frame rate used for snapping and export.
    pub fps: f32,
    /// Whether playback wraps.
    pub looping: bool,
    /// Seconds to blend in when started.
    #[serde(default)]
    pub fade_in: f32,
    /// Seconds to blend out when stopped.
    #[serde(default)]
    pub fade_out: f32,
    /// Parameter tracks.
    pub tracks: Vec<Track>,
    /// Timeline events.
    #[serde(default)]
    pub events: Vec<MotionEvent>,
}

impl Motion {
    /// An empty clip.
    pub fn new(name: impl Into<String>, duration: f32, fps: f32) -> Self {
        Self {
            name: name.into(),
            duration: duration.max(1.0 / 120.0),
            fps: fps.clamp(1.0, 240.0),
            looping: true,
            fade_in: 0.3,
            fade_out: 0.3,
            tracks: Vec::new(),
            events: Vec::new(),
        }
    }

    /// The track for `param`.
    pub fn track(&self, param: ParameterId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.param == param)
    }

    /// The track for `param`, created if needed.
    pub fn track_mut(&mut self, param: ParameterId) -> &mut Track {
        if let Some(i) = self.tracks.iter().position(|t| t.param == param) {
            return &mut self.tracks[i];
        }
        self.tracks.push(Track::new(param));
        let last = self.tracks.len() - 1;
        &mut self.tracks[last]
    }

    /// Wrap or clamp a playhead time into the clip.
    pub fn local_time(&self, time: f32) -> f32 {
        if self.looping && self.duration > 0.0 {
            time.rem_euclid(self.duration)
        } else {
            time.clamp(0.0, self.duration)
        }
    }

    /// Frame index nearest to `time`.
    pub fn frame_at(&self, time: f32) -> u32 {
        (time * self.fps).round().max(0.0) as u32
    }

    /// Number of frames.
    pub fn frame_count(&self) -> u32 {
        (self.duration * self.fps).round().max(1.0) as u32
    }

    /// Write this motion's values at `time` into `values`, blended by
    /// `weight`, overriding (`additive == false`) or adding to them.
    pub fn apply(
        &self,
        parameters: &[Parameter],
        time: f32,
        weight: f32,
        additive: bool,
        values: &mut ParamValues,
    ) {
        let time = self.local_time(time);
        let weight = weight.clamp(0.0, 1.0);
        for track in &self.tracks {
            if !track.enabled {
                continue;
            }
            let Some(sample) = track.sample(time) else {
                continue;
            };
            let Some(p) = parameters.iter().find(|p| p.id == track.param) else {
                continue;
            };
            let current = values.get(&p.id).copied().unwrap_or(p.default);
            let value = if additive {
                current + (sample - p.default) * weight
            } else {
                current + (sample - current) * weight
            };
            values.insert(p.id, p.clamp(value));
        }
    }

    /// Record every parameter's current value as a key at `time`.
    pub fn key_all(&mut self, parameters: &[Parameter], values: &ParamValues, time: f32) {
        for p in parameters {
            let v = values.get(&p.id).copied().unwrap_or(p.default);
            self.track_mut(p.id).set_key(time, v);
        }
    }

    /// Check keys are sorted and finite.
    pub fn validate(&self) -> Result<()> {
        for track in &self.tracks {
            if track
                .keys
                .windows(2)
                .any(|w| w[1].time < w[0].time || !w[0].value.is_finite())
            {
                return Err(AetherError::rig(format!(
                    "motion '{}' has unsorted keys",
                    self.name
                )));
            }
        }
        Ok(())
    }

    /// Sort keys and drop non-finite ones.
    pub fn repair(&mut self) {
        for track in &mut self.tracks {
            track.keys.retain(|k| k.time.is_finite() && k.value.is_finite());
            track.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        }
        if !(self.duration.is_finite() && self.duration > 0.0) {
            self.duration = 1.0;
        }
        if !(self.fps.is_finite() && self.fps > 0.0) {
            self.fps = 30.0;
        }
    }
}

/// How an animator layer combines with what is below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MotionBlend {
    /// Replace the values (weighted by fade).
    #[default]
    Override,
    /// Add the motion's offset from each parameter's default.
    Additive,
}

/// One playing motion.
#[derive(Clone, Debug, PartialEq)]
pub struct Playback {
    /// Index into the rig's motions.
    pub motion: usize,
    /// Playhead, seconds.
    pub time: f32,
    /// Playback speed.
    pub speed: f32,
    /// Layer weight.
    pub weight: f32,
    /// Combination mode.
    pub blend: MotionBlend,
    fade: f32,
    fade_in: f32,
    fade_out: Option<f32>,
}

impl Playback {
    /// Current fade factor.
    pub fn fade(&self) -> f32 {
        self.fade
    }
}

/// Plays motions in layers with crossfades.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animator {
    /// Active layers, bottom first.
    pub layers: Vec<Playback>,
}

impl Animator {
    /// Start a motion. An overriding motion fades out other overriding ones.
    pub fn play(&mut self, motions: &[Motion], index: usize, blend: MotionBlend) {
        let Some(motion) = motions.get(index) else {
            return;
        };
        if blend == MotionBlend::Override {
            for layer in &mut self.layers {
                if layer.blend == MotionBlend::Override && layer.fade_out.is_none() {
                    let out = motions.get(layer.motion).map(|m| m.fade_out).unwrap_or(0.3);
                    layer.fade_out = Some(out.max(motion.fade_in).max(1e-3));
                }
            }
        }
        self.layers.push(Playback {
            motion: index,
            time: 0.0,
            speed: 1.0,
            weight: 1.0,
            blend,
            fade: if motion.fade_in > 0.0 { 0.0 } else { 1.0 },
            fade_in: motion.fade_in,
            fade_out: None,
        });
    }

    /// Fade out every layer.
    pub fn stop_all(&mut self, motions: &[Motion]) {
        for layer in &mut self.layers {
            if layer.fade_out.is_none() {
                let out = motions.get(layer.motion).map(|m| m.fade_out).unwrap_or(0.3);
                layer.fade_out = Some(out.max(1e-3));
            }
        }
    }

    /// True when nothing is playing.
    pub fn is_idle(&self) -> bool {
        self.layers.is_empty()
    }

    /// True while motion `index` plays and is not fading out.
    pub fn is_playing(&self, index: usize) -> bool {
        self.layers
            .iter()
            .any(|l| l.motion == index && l.fade_out.is_none())
    }

    /// True while any motion plays and is not fading out.
    pub fn has_active(&self) -> bool {
        self.layers.iter().any(|l| l.fade_out.is_none())
    }

    /// Fade out motion `index` wherever it plays.
    pub fn stop(&mut self, motions: &[Motion], index: usize) {
        for layer in &mut self.layers {
            if layer.motion == index && layer.fade_out.is_none() {
                let out = motions.get(index).map(|m| m.fade_out).unwrap_or(0.3);
                layer.fade_out = Some(out.max(1e-3));
            }
        }
    }

    /// Advance and apply every layer to `values`.
    pub fn update(
        &mut self,
        motions: &[Motion],
        parameters: &[Parameter],
        values: &mut ParamValues,
        dt: f32,
    ) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        for layer in &mut self.layers {
            let Some(motion) = motions.get(layer.motion) else {
                layer.fade = 0.0;
                layer.fade_out = Some(0.0);
                continue;
            };
            layer.time += dt * layer.speed;
            match layer.fade_out {
                Some(duration) => {
                    layer.fade -= if duration > 0.0 { dt / duration } else { 1.0 };
                }
                None => {
                    if layer.fade < 1.0 {
                        layer.fade += if layer.fade_in > 0.0 {
                            dt / layer.fade_in
                        } else {
                            1.0
                        };
                    }
                    if !motion.looping && layer.time >= motion.duration {
                        layer.fade_out = Some(motion.fade_out.max(1e-3));
                    }
                }
            }
            layer.fade = layer.fade.clamp(0.0, 1.0);
            let weight = smooth(layer.fade) * layer.weight;
            motion.apply(
                parameters,
                layer.time,
                weight,
                layer.blend == MotionBlend::Additive,
                values,
            );
        }
        self.layers.retain(|l| !(l.fade_out.is_some() && l.fade <= 0.0));
    }
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// How an expression entry combines with the current value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpressionBlend {
    /// Add to the value.
    #[default]
    Add,
    /// Multiply the value.
    Multiply,
    /// Replace the value.
    Overwrite,
}

/// One parameter in an expression.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpressionEntry {
    /// The parameter.
    pub param: ParameterId,
    /// Amount (added, multiplied by, or written).
    pub value: f32,
    /// Combination mode.
    pub blend: ExpressionBlend,
}

/// A named facial expression: a sparse parameter preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Expression {
    /// Display name.
    pub name: String,
    /// Entries.
    pub entries: Vec<ExpressionEntry>,
    /// Blend time in seconds.
    #[serde(default = "default_fade")]
    pub fade: f32,
}

fn default_fade() -> f32 {
    0.25
}

impl Expression {
    /// Capture every parameter that differs from its default as an
    /// overwrite entry.
    pub fn capture(name: impl Into<String>, parameters: &[Parameter], values: &ParamValues) -> Self {
        let entries = parameters
            .iter()
            .filter_map(|p| {
                let v = values.get(&p.id).copied()?;
                ((v - p.default).abs() > 1e-5).then_some(ExpressionEntry {
                    param: p.id,
                    value: v,
                    blend: ExpressionBlend::Overwrite,
                })
            })
            .collect();
        Self {
            name: name.into(),
            entries,
            fade: default_fade(),
        }
    }

    /// Apply at `weight` (0..1) to `values`.
    pub fn apply(&self, parameters: &[Parameter], weight: f32, values: &mut ParamValues) {
        let weight = weight.clamp(0.0, 1.0);
        for entry in &self.entries {
            let Some(p) = parameters.iter().find(|p| p.id == entry.param) else {
                continue;
            };
            let current = values.get(&p.id).copied().unwrap_or(p.default);
            let target = match entry.blend {
                ExpressionBlend::Add => current + entry.value,
                ExpressionBlend::Multiply => current * entry.value,
                ExpressionBlend::Overwrite => entry.value,
            };
            values.insert(p.id, p.clamp(current + (target - current) * weight));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: ParameterId = ParameterId(1);

    fn params() -> Vec<Parameter> {
        vec![Parameter::new(P, "AngleX", -30.0, 30.0, 0.0)]
    }

    #[test]
    fn every_easing_starts_at_zero_and_ends_at_one() {
        for easing in Easing::PRESETS {
            let start = easing.apply(0.0);
            let end = easing.apply(1.0);
            assert!(start.abs() < 1e-4, "{} starts at {start}", easing.name());
            assert!((end - 1.0).abs() < 1e-4, "{} ends at {end}", easing.name());
        }
    }

    #[test]
    fn easings_have_their_character() {
        assert!(Easing::EaseIn.apply(0.25) < 0.25);
        assert!(Easing::EaseOut.apply(0.25) > 0.25);
        assert!(Easing::BackIn.apply(0.2) < 0.0, "back-in anticipates");
        assert!(Easing::BackOut.apply(0.8) > 1.0, "back-out overshoots");
        let spring = Easing::Spring {
            frequency: 3.0,
            damping: 0.2,
        };
        assert!(
            (0..100).any(|i| spring.apply(i as f32 / 100.0) > 1.02),
            "an underdamped spring overshoots"
        );
        let linear_bezier = Easing::Bezier {
            x1: 0.25,
            y1: 0.25,
            x2: 0.75,
            y2: 0.75,
        };
        assert!((linear_bezier.apply(0.3) - 0.3).abs() < 1e-3);
        assert_eq!(Easing::Step.apply(0.99), 0.0);
    }

    #[test]
    fn tracks_interpolate_and_hold_at_the_ends() {
        let mut track = Track::new(P);
        track.set_key(0.0, 0.0);
        track.set_key(1.0, 10.0);
        assert_eq!(track.sample(-1.0), Some(0.0));
        assert_eq!(track.sample(0.5), Some(5.0));
        assert_eq!(track.sample(5.0), Some(10.0));
        track.set_key(1.0, 20.0);
        assert_eq!(track.keys.len(), 2, "same time replaces the key");
        track.set_key(0.5, 3.0);
        assert_eq!(track.keys[1].time, 0.5, "keys stay sorted");
        let moved = track.move_key(1, 2.0).expect("move");
        assert_eq!(moved, 2);
        assert!(track.keys.windows(2).all(|w| w[0].time < w[1].time));
    }

    #[test]
    fn looping_motions_wrap_and_one_shots_clamp() {
        let mut motion = Motion::new("m", 2.0, 30.0);
        motion.track_mut(P).set_key(0.0, 0.0);
        motion.track_mut(P).set_key(2.0, 20.0);
        let mut values = ParamValues::new();
        motion.apply(&params(), 3.0, 1.0, false, &mut values);
        assert_eq!(values[&P], 10.0);
        motion.looping = false;
        motion.apply(&params(), 3.0, 1.0, false, &mut values);
        assert_eq!(values[&P], 20.0);
        assert_eq!(motion.frame_count(), 60);
    }

    #[test]
    fn additive_motions_add_offsets_from_default() {
        let mut motion = Motion::new("nod", 1.0, 30.0);
        motion.track_mut(P).set_key(0.0, 5.0);
        let mut values: ParamValues = [(P, 10.0)].into_iter().collect();
        motion.apply(&params(), 0.0, 1.0, true, &mut values);
        assert_eq!(values[&P], 15.0);
    }

    #[test]
    fn the_animator_crossfades_between_motions() {
        let mut a = Motion::new("a", 1.0, 30.0);
        a.fade_in = 0.0;
        a.fade_out = 0.5;
        a.track_mut(P).set_key(0.0, -20.0);
        let mut b = Motion::new("b", 1.0, 30.0);
        b.fade_in = 0.5;
        b.track_mut(P).set_key(0.0, 20.0);
        let motions = vec![a, b];
        let mut animator = Animator::default();
        animator.play(&motions, 0, MotionBlend::Override);
        let mut values = ParamValues::new();
        animator.update(&motions, &params(), &mut values, 0.1);
        assert_eq!(values[&P], -20.0);
        animator.play(&motions, 1, MotionBlend::Override);
        let mut mid = ParamValues::new();
        animator.update(&motions, &params(), &mut mid, 0.25);
        assert!(mid[&P] > -20.0 && mid[&P] < 20.0, "mid-crossfade: {}", mid[&P]);
        for _ in 0..20 {
            values.clear();
            animator.update(&motions, &params(), &mut values, 0.1);
        }
        assert!((values[&P] - 20.0).abs() < 1e-3);
        assert_eq!(animator.layers.len(), 1, "the faded-out layer is dropped");
    }

    #[test]
    fn one_shot_motions_end_and_fade_away() {
        let mut m = Motion::new("once", 0.5, 30.0);
        m.looping = false;
        m.fade_out = 0.1;
        m.track_mut(P).set_key(0.0, 10.0);
        let motions = vec![m];
        let mut animator = Animator::default();
        animator.play(&motions, 0, MotionBlend::Override);
        let mut values = ParamValues::new();
        for _ in 0..20 {
            animator.update(&motions, &params(), &mut values, 0.1);
        }
        assert!(animator.is_idle());
    }

    #[test]
    fn expressions_capture_and_blend() {
        let mut values: ParamValues = [(P, 12.0)].into_iter().collect();
        let smile = Expression::capture("smile", &params(), &values);
        assert_eq!(smile.entries.len(), 1);
        values.clear();
        smile.apply(&params(), 0.5, &mut values);
        assert_eq!(values[&P], 6.0);
        let add = Expression {
            name: "add".into(),
            entries: vec![ExpressionEntry {
                param: P,
                value: 100.0,
                blend: ExpressionBlend::Add,
            }],
            fade: 0.1,
        };
        add.apply(&params(), 1.0, &mut values);
        assert_eq!(values[&P], 30.0, "results are clamped to the range");
    }
}
