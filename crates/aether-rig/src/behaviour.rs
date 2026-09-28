//! Procedural behaviours.
//!
//! Some motion should never have to be keyed: a character that stops blinking
//! or breathing looks dead, and one that does not look at what it is talking
//! to looks distracted. These behaviours generate that motion continuously,
//! on top of whatever is playing:
//!
//! * [`AutoBlink`] — randomised blinks, with occasional double blinks.
//! * [`Breath`] — slow sinusoidal tracks on any parameters.
//! * [`LookAt`] — head, eyes and body follow a target point, with smoothing.
//! * [`LipSync`] — mouth opening and form from a live audio level.
//!
//! Behaviours are configuration here; their state (timers, smoothing) lives in
//! [`BehaviourState`], owned by the runtime.

use crate::param::{ParamValues, Parameter};
use aether_core::math::Vec2;
use aether_core::ParameterId;
use serde::{Deserialize, Serialize};

/// Randomised blinking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoBlink {
    /// Whether blinking runs.
    pub enabled: bool,
    /// Eye-open parameters (closed at their minimum).
    pub params: Vec<ParameterId>,
    /// Shortest gap between blinks, seconds.
    pub min_interval: f32,
    /// Longest gap between blinks, seconds.
    pub max_interval: f32,
    /// Closing time.
    pub close: f32,
    /// Time held shut.
    pub hold: f32,
    /// Opening time.
    pub open: f32,
    /// Chance that a blink is immediately followed by another.
    pub double_chance: f32,
}

impl Default for AutoBlink {
    fn default() -> Self {
        Self {
            enabled: false,
            params: Vec::new(),
            min_interval: 2.0,
            max_interval: 6.0,
            close: 0.07,
            hold: 0.04,
            open: 0.12,
            double_chance: 0.15,
        }
    }
}

/// One breathing track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Breath {
    /// The parameter moved.
    pub param: ParameterId,
    /// Seconds per breath.
    pub period: f32,
    /// Swing, as a fraction of the parameter's range from its default.
    pub amplitude: f32,
    /// Centre offset, same units.
    pub offset: f32,
    /// Phase shift, as a fraction of the period.
    pub phase: f32,
}

/// Which way a look-at target moves a parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LookAxis {
    /// Horizontal.
    X,
    /// Vertical (up is positive).
    Y,
}

/// One parameter that follows the look target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LookTarget {
    /// The parameter.
    pub param: ParameterId,
    /// Axis followed.
    pub axis: LookAxis,
    /// Gain on the normalised target (1 = full range at the edge).
    pub gain: f32,
}

/// Follow a point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LookAt {
    /// Whether it runs.
    pub enabled: bool,
    /// Parameters that follow.
    pub targets: Vec<LookTarget>,
    /// Time constant of the smoothing, seconds.
    pub smoothing: f32,
}

impl Default for LookAt {
    fn default() -> Self {
        Self {
            enabled: false,
            targets: Vec::new(),
            smoothing: 0.15,
        }
    }
}

/// Mouth from audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LipSync {
    /// Whether it runs.
    pub enabled: bool,
    /// Mouth opening parameter.
    pub mouth_open: Option<ParameterId>,
    /// Mouth form parameter (smile-ish for bright sounds).
    pub mouth_form: Option<ParameterId>,
    /// Level gain.
    pub gain: f32,
    /// Time constant, seconds.
    pub smoothing: f32,
}

impl Default for LipSync {
    fn default() -> Self {
        Self {
            enabled: false,
            mouth_open: None,
            mouth_form: None,
            gain: 1.0,
            smoothing: 0.06,
        }
    }
}

/// Every behaviour of a rig.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Behaviours {
    /// Blinking.
    #[serde(default)]
    pub blink: AutoBlink,
    /// Breathing tracks.
    #[serde(default)]
    pub breath: Vec<Breath>,
    /// Look-at.
    #[serde(default)]
    pub look: LookAt,
    /// Lip sync.
    #[serde(default)]
    pub lip_sync: LipSync,
}

impl Behaviours {
    /// Drop every reference to a deleted parameter.
    pub fn forget_parameter(&mut self, id: ParameterId) {
        self.blink.params.retain(|p| *p != id);
        self.breath.retain(|b| b.param != id);
        self.look.targets.retain(|t| t.param != id);
        if self.lip_sync.mouth_open == Some(id) {
            self.lip_sync.mouth_open = None;
        }
        if self.lip_sync.mouth_form == Some(id) {
            self.lip_sync.mouth_form = None;
        }
    }

    /// Sensible defaults wired to the standard parameter names that exist in
    /// `parameters`.
    pub fn standard(parameters: &[Parameter]) -> Self {
        let find = |name: &str| parameters.iter().find(|p| p.name == name).map(|p| p.id);
        let mut b = Behaviours::default();
        b.blink.params = ["EyeLOpen", "EyeROpen"].iter().filter_map(|n| find(n)).collect();
        b.blink.enabled = !b.blink.params.is_empty();
        if let Some(breath) = find("Breath") {
            b.breath.push(Breath {
                param: breath,
                period: 3.4,
                amplitude: 0.5,
                offset: 0.5,
                phase: 0.0,
            });
        }
        for (name, axis, gain) in [
            ("AngleX", LookAxis::X, 1.0),
            ("AngleY", LookAxis::Y, 1.0),
            ("EyeBallX", LookAxis::X, 1.0),
            ("EyeBallY", LookAxis::Y, 1.0),
            ("BodyAngleX", LookAxis::X, 0.4),
        ] {
            if let Some(param) = find(name) {
                b.look.targets.push(LookTarget { param, axis, gain });
            }
        }
        b.look.enabled = false;
        b.lip_sync.mouth_open = find("MouthOpenY");
        b.lip_sync.mouth_form = find("MouthForm");
        b
    }
}

/// External signals behaviours react to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BehaviourInputs {
    /// Where to look, `-1..=1` on each axis (y up), or `None` for "ahead".
    pub look: Option<Vec2>,
    /// Voice loudness, `0..=1`.
    pub audio_level: f32,
    /// Voice brightness, `-1..=1`.
    pub audio_brightness: f32,
}

/// Timers and smoothing for [`Behaviours`].
#[derive(Clone, Debug, PartialEq)]
pub struct BehaviourState {
    time: f64,
    next_blink: f64,
    blink_start: Option<f64>,
    rng: u64,
    look: Vec2,
    mouth: f32,
    form: f32,
}

impl Default for BehaviourState {
    fn default() -> Self {
        Self::new(0x5EED)
    }
}

impl BehaviourState {
    /// State with a given random seed (blink timing is reproducible).
    pub fn new(seed: u64) -> Self {
        Self {
            time: 0.0,
            next_blink: 1.5,
            blink_start: None,
            rng: seed.max(1),
            look: Vec2::ZERO,
            mouth: 0.0,
            form: 0.0,
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Current eye closure in `0..=1` (for tests and overlays).
    pub fn blink_closure(&self, blink: &AutoBlink) -> f32 {
        let Some(start) = self.blink_start else {
            return 0.0;
        };
        let t = (self.time - start) as f32;
        let close = blink.close.max(1e-3);
        let hold = blink.hold.max(0.0);
        let open = blink.open.max(1e-3);
        if t < close {
            t / close
        } else if t < close + hold {
            1.0
        } else if t < close + hold + open {
            1.0 - (t - close - hold) / open
        } else {
            0.0
        }
    }

    /// Advance by `dt` and apply every enabled behaviour to `values`.
    pub fn update(
        &mut self,
        behaviours: &Behaviours,
        parameters: &[Parameter],
        inputs: &BehaviourInputs,
        values: &mut ParamValues,
        dt: f32,
    ) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 1.0) } else { 0.0 };
        self.time += dt as f64;
        let param = |id: ParameterId| parameters.iter().find(|p| p.id == id);

        // Blink.
        let blink = &behaviours.blink;
        if blink.enabled && !blink.params.is_empty() {
            let length = (blink.close + blink.hold + blink.open) as f64;
            if let Some(start) = self.blink_start {
                if self.time - start >= length {
                    self.blink_start = None;
                    let gap = if self.random() < blink.double_chance {
                        0.08
                    } else {
                        let lo = blink.min_interval.max(0.1);
                        let hi = blink.max_interval.max(lo);
                        lo + (hi - lo) * self.random()
                    };
                    self.next_blink = self.time + gap as f64;
                }
            } else if self.time >= self.next_blink {
                self.blink_start = Some(self.time);
            }
            let closure = self.blink_closure(blink);
            if closure > 0.0 {
                for &id in &blink.params {
                    if let Some(p) = param(id) {
                        let current = values.get(&id).copied().unwrap_or(p.default);
                        values.insert(id, p.clamp(current + (p.min - current) * closure));
                    }
                }
            }
        }

        // Breath.
        for breath in &behaviours.breath {
            let Some(p) = param(breath.param) else { continue };
            let period = breath.period.max(0.1) as f64;
            let phase = (self.time / period + breath.phase as f64) * std::f64::consts::TAU;
            let n = breath.offset + breath.amplitude * phase.sin() as f32 * 0.5;
            let delta = p.from_normalized(n.clamp(-1.0, 1.0)) - p.default;
            let current = values.get(&p.id).copied().unwrap_or(p.default);
            values.insert(p.id, p.clamp(current + delta));
        }

        // Look-at.
        let look = &behaviours.look;
        let target = inputs.look.unwrap_or(Vec2::ZERO);
        let k = if look.smoothing > 0.0 {
            1.0 - (-dt / look.smoothing).exp()
        } else {
            1.0
        };
        self.look += (target - self.look) * k;
        if look.enabled {
            for t in &look.targets {
                let Some(p) = param(t.param) else { continue };
                let n = match t.axis {
                    LookAxis::X => self.look.x,
                    LookAxis::Y => self.look.y,
                } * t.gain;
                let delta = p.from_normalized(n.clamp(-1.0, 1.0)) - p.default;
                let current = values.get(&p.id).copied().unwrap_or(p.default);
                values.insert(p.id, p.clamp(current + delta));
            }
        }

        // Lip sync.
        let lip = &behaviours.lip_sync;
        let k = if lip.smoothing > 0.0 {
            1.0 - (-dt / lip.smoothing).exp()
        } else {
            1.0
        };
        self.mouth += ((inputs.audio_level * lip.gain).clamp(0.0, 1.0) - self.mouth) * k;
        self.form += (inputs.audio_brightness.clamp(-1.0, 1.0) - self.form) * k;
        if lip.enabled {
            if let Some(p) = lip.mouth_open.and_then(param) {
                let current = values.get(&p.id).copied().unwrap_or(p.default);
                values.insert(p.id, p.clamp(current.max(p.min + (p.max - p.min) * self.mouth)));
            }
            if self.mouth > 0.05 {
                if let Some(p) = lip.mouth_form.and_then(param) {
                    values.insert(p.id, p.from_normalized(self.form));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Vec<Parameter> {
        crate::param::StandardParam::ALL
            .iter()
            .enumerate()
            .map(|(i, s)| s.instantiate(ParameterId(i as u64 + 1)))
            .collect()
    }

    fn id(params: &[Parameter], name: &str) -> ParameterId {
        params.iter().find(|p| p.name == name).expect("param").id
    }

    #[test]
    fn standard_behaviours_wire_the_standard_parameters() {
        let p = params();
        let b = Behaviours::standard(&p);
        assert!(b.blink.enabled);
        assert_eq!(b.blink.params.len(), 2);
        assert_eq!(b.breath.len(), 1);
        assert!(b.lip_sync.mouth_open.is_some());
    }

    #[test]
    fn eyes_blink_shut_and_reopen() {
        let p = params();
        let b = Behaviours::standard(&p);
        let eye = id(&p, "EyeLOpen");
        let mut state = BehaviourState::new(42);
        let mut min_seen = 1.0f32;
        let mut blinks = 0;
        let mut closed = false;
        for _ in 0..(20 * 60) {
            let mut values = ParamValues::new();
            state.update(&b, &p, &BehaviourInputs::default(), &mut values, 1.0 / 60.0);
            let v = values.get(&eye).copied().unwrap_or(1.0);
            min_seen = min_seen.min(v);
            if v < 0.1 && !closed {
                blinks += 1;
                closed = true;
            } else if v > 0.9 {
                closed = false;
            }
        }
        assert!(min_seen < 0.05, "the eyes close fully");
        assert!((3..=15).contains(&blinks), "{blinks} blinks in 20 s");
    }

    #[test]
    fn breathing_oscillates_around_its_offset() {
        let p = params();
        let b = Behaviours::standard(&p);
        let breath = id(&p, "Breath");
        let mut state = BehaviourState::new(1);
        let mut seen = Vec::new();
        for _ in 0..240 {
            let mut values = ParamValues::new();
            state.update(&b, &p, &BehaviourInputs::default(), &mut values, 1.0 / 60.0);
            seen.push(values[&breath]);
        }
        let lo = seen.iter().copied().fold(f32::MAX, f32::min);
        let hi = seen.iter().copied().fold(f32::MIN, f32::max);
        assert!(lo < 0.35 && hi > 0.65, "breath swings: {lo}..{hi}");
    }

    #[test]
    fn look_at_turns_toward_the_target_smoothly() {
        let p = params();
        let mut b = Behaviours::standard(&p);
        b.look.enabled = true;
        b.blink.enabled = false;
        let angle = id(&p, "AngleX");
        let mut state = BehaviourState::new(1);
        let inputs = BehaviourInputs {
            look: Some(Vec2::new(1.0, 0.0)),
            ..Default::default()
        };
        let mut values = ParamValues::new();
        state.update(&b, &p, &inputs, &mut values, 1.0 / 60.0);
        let first = values[&angle];
        assert!(first > 0.0 && first < 30.0, "smoothing: {first}");
        for _ in 0..120 {
            values.clear();
            state.update(&b, &p, &inputs, &mut values, 1.0 / 60.0);
        }
        assert!((values[&angle] - 30.0).abs() < 0.5);
    }

    #[test]
    fn lip_sync_opens_the_mouth_with_the_voice() {
        let p = params();
        let mut b = Behaviours::standard(&p);
        b.lip_sync.enabled = true;
        let mouth = id(&p, "MouthOpenY");
        let mut state = BehaviourState::new(1);
        let loud = BehaviourInputs {
            audio_level: 0.9,
            audio_brightness: 0.5,
            ..Default::default()
        };
        let mut values = ParamValues::new();
        for _ in 0..30 {
            values.clear();
            state.update(&b, &p, &loud, &mut values, 1.0 / 60.0);
        }
        assert!(values[&mouth] > 0.8);
        for _ in 0..60 {
            values.clear();
            state.update(&b, &p, &BehaviourInputs::default(), &mut values, 1.0 / 60.0);
        }
        assert!(values.get(&mouth).copied().unwrap_or(0.0) < 0.05);
    }

    #[test]
    fn forgetting_a_parameter_unwires_it() {
        let p = params();
        let mut b = Behaviours::standard(&p);
        let eye = id(&p, "EyeLOpen");
        b.forget_parameter(eye);
        assert!(!b.blink.params.contains(&eye));
    }
}
