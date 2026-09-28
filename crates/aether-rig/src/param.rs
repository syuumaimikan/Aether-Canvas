//! Parameters.
//!
//! A parameter is a named number with a range — `AngleX` from -30 to 30,
//! `EyeLOpen` from 0 to 1. Everything that moves in a rig is ultimately a
//! function of parameter values: keyforms interpolate across them, physics and
//! drivers write them, motions animate them and the editor's sliders set them.
//! Keeping one currency for "pose" is what lets all of those systems compose.
//!
//! Parameter *definitions* are part of the rig and edited through commands.
//! Parameter *values* ([`ParamValues`]) are pose state, like a timeline
//! playhead: they are saved with the project so it reopens as it was left, but
//! scrubbing a slider is not an undoable edit to the artwork.

use aether_core::ParameterId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Current value of every parameter that is not at its default.
pub type ParamValues = BTreeMap<ParameterId, f32>;

/// One rig parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// Stable identity.
    pub id: ParameterId,
    /// Display name, also what driver expressions refer to.
    pub name: String,
    /// Lowest value.
    pub min: f32,
    /// Highest value.
    pub max: f32,
    /// Resting value.
    pub default: f32,
    /// Folder in the parameter panel ("Face", "Body", ...).
    #[serde(default)]
    pub group: String,
    /// When true the range wraps: `max` is the same pose as `min`, as for a
    /// full turn. Keyform interpolation then crosses the seam smoothly.
    #[serde(default)]
    pub cyclic: bool,
}

impl Parameter {
    /// A parameter with the given range; the default is clamped into it.
    pub fn new(id: ParameterId, name: impl Into<String>, min: f32, max: f32, default: f32) -> Self {
        let mut p = Self {
            id,
            name: name.into(),
            min,
            max,
            default,
            group: String::new(),
            cyclic: false,
        };
        p.sanitize();
        p
    }

    /// Builder: put the parameter in a panel group.
    pub fn in_group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }

    /// Repair a range that is inverted, empty or not finite.
    pub fn sanitize(&mut self) {
        if !self.min.is_finite() {
            self.min = 0.0;
        }
        if !self.max.is_finite() {
            self.max = 1.0;
        }
        if self.max < self.min {
            std::mem::swap(&mut self.min, &mut self.max);
        }
        if (self.max - self.min).abs() < 1e-6 {
            self.max = self.min + 1.0;
        }
        if !self.default.is_finite() {
            self.default = self.min;
        }
        self.default = self.default.clamp(self.min, self.max);
    }

    /// Length of the range.
    pub fn span(&self) -> f32 {
        self.max - self.min
    }

    /// Bring a value into range: wrapped for cyclic parameters, clamped
    /// otherwise.
    pub fn clamp(&self, value: f32) -> f32 {
        if !value.is_finite() {
            return self.default;
        }
        if self.cyclic {
            let span = self.span();
            self.min + (value - self.min).rem_euclid(span)
        } else {
            value.clamp(self.min, self.max)
        }
    }

    /// Map a value onto `-1..=1`, with the default at zero.
    ///
    /// Each side of the default is scaled separately, so `EyeOpen` (0..1,
    /// default 1) still reaches -1 when fully closed.
    pub fn normalized(&self, value: f32) -> f32 {
        let value = self.clamp(value);
        if value >= self.default {
            let side = self.max - self.default;
            if side <= 1e-6 {
                0.0
            } else {
                (value - self.default) / side
            }
        } else {
            let side = self.default - self.min;
            if side <= 1e-6 {
                0.0
            } else {
                (value - self.default) / side
            }
        }
    }

    /// Inverse of [`Parameter::normalized`].
    pub fn from_normalized(&self, n: f32) -> f32 {
        let n = n.clamp(-1.0, 1.0);
        if n >= 0.0 {
            self.default + n * (self.max - self.default)
        } else {
            self.default + n * (self.default - self.min)
        }
    }

    /// Where the range wraps, for cyclic parameters: `(min, span)`.
    pub fn cycle(&self) -> Option<(f32, f32)> {
        self.cyclic.then(|| (self.min, self.span()))
    }
}

/// The conventional parameter set, named the way face-tracking software and
/// existing rigs expect.
///
/// Using these names is optional, but it means a tracker, a lip-sync source or
/// an imported motion can drive a new rig without any mapping step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StandardParam {
    /// Parameter name.
    pub name: &'static str,
    /// Panel group.
    pub group: &'static str,
    /// Minimum.
    pub min: f32,
    /// Maximum.
    pub max: f32,
    /// Default.
    pub default: f32,
}

impl StandardParam {
    const fn new(name: &'static str, group: &'static str, min: f32, max: f32, default: f32) -> Self {
        Self {
            name,
            group,
            min,
            max,
            default,
        }
    }

    /// The standard set, in panel order.
    pub const ALL: [StandardParam; 22] = [
        StandardParam::new("AngleX", "Face", -30.0, 30.0, 0.0),
        StandardParam::new("AngleY", "Face", -30.0, 30.0, 0.0),
        StandardParam::new("AngleZ", "Face", -30.0, 30.0, 0.0),
        StandardParam::new("EyeLOpen", "Eyes", 0.0, 1.0, 1.0),
        StandardParam::new("EyeLSmile", "Eyes", 0.0, 1.0, 0.0),
        StandardParam::new("EyeROpen", "Eyes", 0.0, 1.0, 1.0),
        StandardParam::new("EyeRSmile", "Eyes", 0.0, 1.0, 0.0),
        StandardParam::new("EyeBallX", "Eyes", -1.0, 1.0, 0.0),
        StandardParam::new("EyeBallY", "Eyes", -1.0, 1.0, 0.0),
        StandardParam::new("BrowLY", "Brows", -1.0, 1.0, 0.0),
        StandardParam::new("BrowRY", "Brows", -1.0, 1.0, 0.0),
        StandardParam::new("MouthForm", "Mouth", -1.0, 1.0, 0.0),
        StandardParam::new("MouthOpenY", "Mouth", 0.0, 1.0, 0.0),
        StandardParam::new("Cheek", "Face", 0.0, 1.0, 0.0),
        StandardParam::new("BodyAngleX", "Body", -10.0, 10.0, 0.0),
        StandardParam::new("BodyAngleY", "Body", -10.0, 10.0, 0.0),
        StandardParam::new("BodyAngleZ", "Body", -10.0, 10.0, 0.0),
        StandardParam::new("Breath", "Body", 0.0, 1.0, 0.0),
        StandardParam::new("HairFront", "Hair", -1.0, 1.0, 0.0),
        StandardParam::new("HairSide", "Hair", -1.0, 1.0, 0.0),
        StandardParam::new("HairBack", "Hair", -1.0, 1.0, 0.0),
        StandardParam::new("ArmSwing", "Body", -1.0, 1.0, 0.0),
    ];

    /// Look up a standard parameter by name.
    pub fn named(name: &str) -> Option<StandardParam> {
        Self::ALL.iter().copied().find(|p| p.name == name)
    }

    /// Instantiate as a rig parameter.
    pub fn instantiate(self, id: ParameterId) -> Parameter {
        Parameter::new(id, self.name, self.min, self.max, self.default).in_group(self.group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_ranges_are_repaired() {
        let p = Parameter::new(ParameterId(1), "p", 5.0, -5.0, 99.0);
        assert_eq!((p.min, p.max, p.default), (-5.0, 5.0, 5.0));
        let flat = Parameter::new(ParameterId(2), "flat", 1.0, 1.0, 1.0);
        assert!(flat.span() > 0.0, "an empty range cannot be keyed");
    }

    #[test]
    fn normalisation_scales_each_side_of_the_default() {
        let eye = Parameter::new(ParameterId(1), "EyeOpen", 0.0, 1.0, 1.0);
        assert_eq!(eye.normalized(0.0), -1.0);
        assert_eq!(eye.normalized(1.0), 0.0);
        let angle = Parameter::new(ParameterId(2), "AngleX", -30.0, 30.0, 0.0);
        assert_eq!(angle.normalized(15.0), 0.5);
        assert_eq!(angle.from_normalized(-0.5), -15.0);
    }

    #[test]
    fn cyclic_parameters_wrap_instead_of_clamping() {
        let mut turn = Parameter::new(ParameterId(1), "Turn", 0.0, 360.0, 0.0);
        assert_eq!(turn.clamp(400.0), 360.0);
        turn.cyclic = true;
        assert!((turn.clamp(400.0) - 40.0).abs() < 1e-4);
        assert!((turn.clamp(-30.0) - 330.0).abs() < 1e-4);
    }

    #[test]
    fn the_standard_set_has_unique_names() {
        let mut names: Vec<&str> = StandardParam::ALL.iter().map(|p| p.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), StandardParam::ALL.len());
        let eye = StandardParam::named("EyeLOpen").expect("standard");
        assert_eq!(eye.instantiate(ParameterId(3)).default, 1.0);
    }
}
