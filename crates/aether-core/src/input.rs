//! Pointer, tablet and keyboard input.
//!
//! Tools never see raw window events. The UI layer normalises everything into
//! [`InputSample`]s in *document space* so a tool behaves identically whether
//! the canvas is zoomed, rotated or mirrored, and whether the input came from a
//! mouse, a pen or a touch screen.

use crate::math::Vec2;
use serde::{Deserialize, Serialize};

/// Which button generated an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PointerButton {
    /// Primary button (draw).
    Primary,
    /// Secondary button (context / colour pick).
    Secondary,
    /// Middle button (pan).
    Middle,
}

/// Keyboard modifiers active when an event was produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    /// Shift held (constrain / extend selection).
    pub shift: bool,
    /// Control or Command held (snap / secondary action).
    pub ctrl: bool,
    /// Alt / Option held (subtract / sample).
    pub alt: bool,
}

impl Modifiers {
    /// No modifier held.
    pub const NONE: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
    };

    /// True when no modifier is held.
    pub fn is_empty(self) -> bool {
        self == Self::NONE
    }
}

/// One sample of pointer state, already mapped into document coordinates.
///
/// Devices that cannot report a field fall back to a neutral value
/// (`pressure = 1.0`, `tilt = (0, 0)`), so tools never need to special-case
/// mouse input.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputSample {
    /// Position in document pixels.
    pub position: Vec2,
    /// Pen pressure, `0..=1`. Mice report `1.0`.
    pub pressure: f32,
    /// Pen tilt in `-1..=1` per axis.
    pub tilt: Vec2,
    /// Barrel rotation in radians.
    pub rotation: f32,
    /// Pointer speed in document pixels per second.
    pub velocity: f32,
    /// Seconds since the stroke began.
    pub time: f64,
    /// Modifier keys at sample time.
    pub modifiers: Modifiers,
}

impl InputSample {
    /// A neutral mouse-like sample at `position`.
    pub fn at(position: Vec2) -> Self {
        Self {
            position,
            pressure: 1.0,
            tilt: Vec2::ZERO,
            rotation: 0.0,
            velocity: 0.0,
            time: 0.0,
            modifiers: Modifiers::NONE,
        }
    }

    /// Builder-style pressure override.
    pub fn with_pressure(mut self, pressure: f32) -> Self {
        self.pressure = crate::math::clampf(pressure, 0.0, 1.0);
        self
    }

    /// Builder-style timestamp override.
    pub fn with_time(mut self, time: f64) -> Self {
        self.time = time;
        self
    }

    /// Interpolate between two samples; used to resample sparse device input.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            position: self.position.lerp(other.position, t),
            pressure: crate::math::lerp(self.pressure, other.pressure, t),
            tilt: self.tilt.lerp(other.tilt, t),
            rotation: crate::math::lerp(self.rotation, other.rotation, t),
            velocity: crate::math::lerp(self.velocity, other.velocity, t),
            time: self.time + (other.time - self.time) * t as f64,
            modifiers: other.modifiers,
        }
    }
}

/// The three phases of a pointer interaction a [`Tool`](trait@self::Tool) reacts to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerPhase {
    /// Button went down.
    Down,
    /// Pointer moved (button may or may not be held).
    Move,
    /// Button was released.
    Up,
}

/// A pointer event delivered to a tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    /// What happened.
    pub phase: PointerPhase,
    /// Which button, when applicable.
    pub button: PointerButton,
    /// Where and how hard.
    pub sample: InputSample,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_default_to_full_pressure() {
        let s = InputSample::at(Vec2::new(3.0, 4.0));
        assert_eq!(s.pressure, 1.0);
        assert!(s.modifiers.is_empty());
    }

    #[test]
    fn interpolation_walks_both_position_and_pressure() {
        let a = InputSample::at(Vec2::ZERO).with_pressure(0.0);
        let b = InputSample::at(Vec2::new(10.0, 0.0)).with_pressure(1.0);
        let mid = a.lerp(&b, 0.5);
        assert_eq!(mid.position.x, 5.0);
        assert_eq!(mid.pressure, 0.5);
    }
}
