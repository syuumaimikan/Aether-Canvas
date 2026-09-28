//! Pendulum physics.
//!
//! Hair, ribbons, earrings and sleeves should keep moving after the head
//! stops. A [`PhysicsGroup`] models that as a chain of particles hanging from
//! an anchor:
//!
//! * **Inputs** read parameters (a head turn, a body tilt) and move or rotate
//!   the anchor.
//! * The chain is integrated with Verlet at a **fixed 120 Hz step**, so the
//!   motion is identical at 30, 60 or 144 frames per second and a render
//!   exported frame-by-frame matches what played live.
//! * Each particle has damping, a stiffness that pulls it back toward the
//!   shape it rests in (short bangs are stiff, long hair is not), and an
//!   optional angle limit. Gravity, wind with turbulence, and circular
//!   colliders act on the whole chain.
//! * **Outputs** turn a particle's swing angle or displacement back into
//!   parameter values, which the chain's keyforms then draw.
//!
//! Everything is expressed in "physics units" relative to the anchor; the
//! numbers only need to be self-consistent, not match document pixels.

use crate::expr::value_noise;
use crate::param::{ParamValues, Parameter};
use aether_core::math::Vec2;
use aether_core::{AetherError, ParameterId, Result};
use serde::{Deserialize, Serialize};

/// Simulation step, seconds.
pub const STEP: f32 = 1.0 / 120.0;
/// Most simulated time consumed per update (a long stall is not replayed).
pub const MAX_FRAME: f32 = 0.25;

fn yes() -> bool {
    true
}

/// What an input parameter does to the anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputKind {
    /// Move the anchor sideways.
    X,
    /// Move the anchor vertically.
    Y,
    /// Rotate the anchor's frame.
    Angle,
}

impl InputKind {
    /// All kinds, in menu order.
    pub const ALL: [InputKind; 3] = [InputKind::X, InputKind::Y, InputKind::Angle];
}

/// One input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicsInput {
    /// The parameter read (normalised to -1..1 around its default).
    pub param: ParameterId,
    /// What it drives.
    pub kind: InputKind,
    /// Contribution, usually `0..=1`.
    pub weight: f32,
    /// Flip the direction.
    #[serde(default)]
    pub invert: bool,
}

/// What an output reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputKind {
    /// The particle's swing angle away from rest (45° = full range).
    Angle,
    /// Sideways displacement relative to the chain length.
    X,
    /// Vertical displacement relative to the chain length.
    Y,
}

impl OutputKind {
    /// All kinds, in menu order.
    pub const ALL: [OutputKind; 3] = [OutputKind::Angle, OutputKind::X, OutputKind::Y];
}

/// One output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicsOutput {
    /// The parameter written.
    pub param: ParameterId,
    /// Which particle (1 = first after the anchor).
    pub particle: usize,
    /// What is measured.
    pub kind: OutputKind,
    /// Gain.
    pub scale: f32,
    /// Flip the direction.
    #[serde(default)]
    pub invert: bool,
}

/// One link of the chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicsParticle {
    /// Distance from the previous particle.
    pub length: f32,
    /// Fraction of velocity lost per second.
    pub damping: f32,
    /// Pull back toward the rest shape, per second².
    pub stiffness: f32,
    /// Largest bend from the previous segment, degrees (180 = free).
    #[serde(default = "free_angle")]
    pub max_angle: f32,
}

fn free_angle() -> f32 {
    180.0
}

impl PhysicsParticle {
    /// A particle with typical hair settings.
    pub fn new(length: f32) -> Self {
        Self {
            length,
            damping: 2.2,
            stiffness: 18.0,
            max_angle: 180.0,
        }
    }
}

/// A circle the chain cannot enter, in the anchor's frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collider {
    /// Centre relative to the anchor (x right, y down at rest).
    pub center: Vec2,
    /// Radius.
    pub radius: f32,
}

/// A simulated chain with its inputs and outputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicsGroup {
    /// Display name.
    pub name: String,
    /// Whether it runs.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Parameters that move the anchor.
    pub inputs: Vec<PhysicsInput>,
    /// Parameters written from the chain.
    pub outputs: Vec<PhysicsOutput>,
    /// The chain, from the anchor outwards.
    pub particles: Vec<PhysicsParticle>,
    /// Constant acceleration (y down).
    pub gravity: Vec2,
    /// Constant wind acceleration.
    #[serde(default)]
    pub wind: Vec2,
    /// Gusting added to the wind, as a fraction of its strength.
    #[serde(default)]
    pub turbulence: f32,
    /// Anchor travel for a fully deflected X or Y input.
    pub translation_range: f32,
    /// Anchor rotation for a fully deflected angle input, degrees.
    pub angle_range: f32,
    /// Obstacles.
    #[serde(default)]
    pub colliders: Vec<Collider>,
}

impl PhysicsGroup {
    /// A hair-sway chain driven by the usual head and body parameters.
    ///
    /// `inputs` are `(param, kind, weight)` triples; the output writes the
    /// swing of the chain's middle particle to `output`.
    pub fn sway(
        name: impl Into<String>,
        inputs: &[(ParameterId, InputKind, f32)],
        output: ParameterId,
    ) -> Self {
        let particles = vec![
            PhysicsParticle::new(12.0),
            PhysicsParticle::new(12.0),
            PhysicsParticle::new(12.0),
        ];
        Self {
            name: name.into(),
            enabled: true,
            inputs: inputs
                .iter()
                .map(|&(param, kind, weight)| PhysicsInput {
                    param,
                    kind,
                    weight,
                    invert: false,
                })
                .collect(),
            outputs: vec![PhysicsOutput {
                param: output,
                particle: 2,
                kind: OutputKind::Angle,
                scale: 1.0,
                invert: false,
            }],
            particles,
            gravity: Vec2::new(0.0, 1400.0),
            wind: Vec2::ZERO,
            turbulence: 0.0,
            translation_range: 6.0,
            angle_range: 20.0,
            colliders: Vec::new(),
        }
    }

    /// Check indices and ranges.
    pub fn validate(&self) -> Result<()> {
        if self.particles.is_empty() {
            return Err(AetherError::rig(format!(
                "physics group '{}' has no particles",
                self.name
            )));
        }
        for output in &self.outputs {
            if output.particle == 0 || output.particle > self.particles.len() {
                return Err(AetherError::rig(format!(
                    "an output of '{}' reads particle {}, which does not exist",
                    self.name, output.particle
                )));
            }
        }
        Ok(())
    }

    fn total_length(&self, upto: usize) -> f32 {
        self.particles
            .iter()
            .take(upto)
            .map(|p| p.length.abs().max(1e-3))
            .sum()
    }
}

/// Live state of one chain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChainState {
    pos: Vec<Vec2>,
    prev: Vec<Vec2>,
    /// Anchor translation and angle at the previous step.
    input: Option<(Vec2, f32)>,
}

impl ChainState {
    /// Positions of the anchor and particles, for drawing.
    pub fn points(&self) -> &[Vec2] {
        &self.pos
    }
}

/// The physics simulation for a rig: one chain state per group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhysicsRuntime {
    chains: Vec<ChainState>,
    accumulator: f32,
    time: f64,
}

fn param_normalized(parameters: &[Parameter], values: &ParamValues, id: ParameterId) -> Option<f32> {
    let p = parameters.iter().find(|p| p.id == id)?;
    Some(p.normalized(values.get(&id).copied().unwrap_or(p.default)))
}

impl PhysicsRuntime {
    /// A fresh simulation (chains start hanging at rest).
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all motion.
    pub fn reset(&mut self) {
        self.chains.clear();
        self.accumulator = 0.0;
        self.time = 0.0;
    }

    /// The live chains, for overlays.
    pub fn chains(&self) -> &[ChainState] {
        &self.chains
    }

    /// Advance by `dt` seconds, reading inputs from and writing outputs to
    /// `values`.
    pub fn update(
        &mut self,
        groups: &[PhysicsGroup],
        parameters: &[Parameter],
        values: &mut ParamValues,
        dt: f32,
    ) {
        if self.chains.len() != groups.len() {
            self.chains = vec![ChainState::default(); groups.len()];
        }
        let dt = if dt.is_finite() {
            dt.clamp(0.0, MAX_FRAME)
        } else {
            0.0
        };
        self.accumulator += dt;

        // Anchor targets for this frame.
        let targets: Vec<(Vec2, f32)> = groups
            .iter()
            .map(|g| anchor_input(g, parameters, values))
            .collect();

        let mut steps = 0;
        while self.accumulator >= STEP {
            self.accumulator -= STEP;
            self.time += STEP as f64;
            steps += 1;
            for (i, group) in groups.iter().enumerate() {
                if !group.enabled || group.validate().is_err() {
                    continue;
                }
                step_chain(&mut self.chains[i], group, targets[i], STEP, self.time);
            }
            if steps > 1000 {
                self.accumulator = 0.0;
                break;
            }
        }

        for (i, group) in groups.iter().enumerate() {
            if !group.enabled || group.validate().is_err() {
                continue;
            }
            let chain = &mut self.chains[i];
            if chain.pos.is_empty() {
                // Not stepped yet (dt below one step): settle at rest.
                init_chain(chain, group, targets[i]);
            }
            write_outputs(chain, group, targets[i], parameters, values);
        }
    }
}

fn anchor_input(group: &PhysicsGroup, parameters: &[Parameter], values: &ParamValues) -> (Vec2, f32) {
    let mut translation = Vec2::ZERO;
    let mut angle = 0.0f32;
    for input in &group.inputs {
        let Some(n) = param_normalized(parameters, values, input.param) else {
            continue;
        };
        let n = if input.invert { -n } else { n } * input.weight;
        match input.kind {
            InputKind::X => translation.x += n * group.translation_range,
            InputKind::Y => translation.y += n * group.translation_range,
            InputKind::Angle => angle += (n * group.angle_range).to_radians(),
        }
    }
    (translation, angle)
}

fn rest_direction(angle: f32) -> Vec2 {
    Vec2::new(0.0, 1.0).rotated(angle)
}

fn init_chain(chain: &mut ChainState, group: &PhysicsGroup, (anchor, angle): (Vec2, f32)) {
    let dir = rest_direction(angle);
    let mut pos = Vec::with_capacity(group.particles.len() + 1);
    pos.push(anchor);
    let mut p = anchor;
    for particle in &group.particles {
        p += dir * particle.length.abs().max(1e-3);
        pos.push(p);
    }
    chain.prev = pos.clone();
    chain.pos = pos;
    chain.input = Some((anchor, angle));
}

fn step_chain(chain: &mut ChainState, group: &PhysicsGroup, target: (Vec2, f32), dt: f32, time: f64) {
    if chain.pos.len() != group.particles.len() + 1 {
        init_chain(chain, group, target);
    }
    // Move the anchor smoothly toward this frame's target.
    let (last_anchor, last_angle) = chain.input.unwrap_or(target);
    let follow = 1.0 - (-dt * 60.0).exp();
    let anchor = last_anchor + (target.0 - last_anchor) * follow;
    let angle = last_angle + (target.1 - last_angle) * follow;
    chain.input = Some((anchor, angle));

    let frame_down = rest_direction(angle);
    let gust = if group.turbulence > 0.0 {
        let t = time * 0.8;
        1.0 + group.turbulence * value_noise(t, 7) as f32
    } else {
        1.0
    };
    let external = group.gravity + group.wind * gust;

    chain.pos[0] = anchor;
    chain.prev[0] = anchor;
    for i in 1..chain.pos.len() {
        let particle = &group.particles[i - 1];
        let parent_dir = if i == 1 {
            frame_down
        } else {
            (chain.pos[i - 1] - chain.pos[i - 2]).normalized()
        };
        let rest_target = chain.pos[i - 1] + parent_dir * particle.length.abs().max(1e-3);
        let damping = (1.0 - particle.damping.max(0.0) * dt).clamp(0.0, 1.0);
        let velocity = (chain.pos[i] - chain.prev[i]) * damping;
        let acceleration = external + (rest_target - chain.pos[i]) * particle.stiffness.max(0.0);
        chain.prev[i] = chain.pos[i];
        chain.pos[i] += velocity + acceleration * (dt * dt);
    }

    // Constraints: segment length, angle limits, colliders.
    for _ in 0..2 {
        for i in 1..chain.pos.len() {
            let particle = &group.particles[i - 1];
            let length = particle.length.abs().max(1e-3);
            let parent_dir = if i == 1 {
                frame_down
            } else {
                (chain.pos[i - 1] - chain.pos[i - 2]).normalized()
            };
            let mut dir = chain.pos[i] - chain.pos[i - 1];
            if dir.length_squared() < 1e-12 {
                dir = parent_dir;
            }
            let mut dir = dir.normalized();
            let limit = particle.max_angle.clamp(0.0, 180.0).to_radians();
            if limit < std::f32::consts::PI {
                let bend = crate::skeleton::wrap_angle(dir.angle() - parent_dir.angle());
                if bend.abs() > limit {
                    dir = parent_dir.rotated(limit * bend.signum());
                }
            }
            chain.pos[i] = chain.pos[i - 1] + dir * length;
            for collider in &group.colliders {
                let center = anchor + collider.center.rotated(angle);
                let mut offset = chain.pos[i] - center;
                if offset.length_squared() < 1e-12 {
                    // Dead centre: push out sideways to the chain.
                    offset = Vec2::new(-dir.y, dir.x) * 1e-3;
                }
                let d = offset.length();
                if d < collider.radius {
                    chain.pos[i] = center + offset * (collider.radius / d);
                }
            }
        }
    }
}

fn write_outputs(
    chain: &ChainState,
    group: &PhysicsGroup,
    (_, target_angle): (Vec2, f32),
    parameters: &[Parameter],
    values: &mut ParamValues,
) {
    let (anchor, angle) = chain.input.unwrap_or((Vec2::ZERO, target_angle));
    let frame_down = rest_direction(angle);
    for output in &group.outputs {
        let k = output.particle;
        if k == 0 || k >= chain.pos.len() {
            continue;
        }
        let n = match output.kind {
            OutputKind::Angle => {
                let dir = chain.pos[k] - chain.pos[k - 1];
                let swing = crate::skeleton::wrap_angle(dir.angle() - frame_down.angle());
                swing.to_degrees() / 45.0
            }
            OutputKind::X | OutputKind::Y => {
                let reach = group.total_length(k);
                let rest = anchor + frame_down * reach;
                let local = (chain.pos[k] - rest).rotated(-angle);
                let d = if output.kind == OutputKind::X {
                    local.x
                } else {
                    local.y
                };
                d / reach
            }
        };
        let n = if output.invert { -n } else { n } * output.scale;
        let Some(p) = parameters.iter().find(|p| p.id == output.param) else {
            continue;
        };
        // Screen-clockwise swing reads as positive; negate so a chain
        // swinging right (lagging a head moving left) reports positive X.
        values.insert(p.id, p.from_normalized(-n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Vec<Parameter>, PhysicsGroup) {
        let params = vec![
            Parameter::new(ParameterId(1), "AngleX", -30.0, 30.0, 0.0),
            Parameter::new(ParameterId(2), "AngleZ", -30.0, 30.0, 0.0),
            Parameter::new(ParameterId(3), "HairFront", -1.0, 1.0, 0.0),
        ];
        let group = PhysicsGroup::sway(
            "Front hair",
            &[
                (ParameterId(1), InputKind::X, 1.0),
                (ParameterId(2), InputKind::Angle, 1.0),
            ],
            ParameterId(3),
        );
        (params, group)
    }

    fn run(fps: f32, seconds: f32, input: f32) -> Vec<f32> {
        let (params, group) = setup();
        let mut physics = PhysicsRuntime::new();
        let mut values = ParamValues::new();
        let frames = (fps * seconds).round() as usize;
        // Settle at rest, then change the input at exactly t = 0 whatever
        // the frame rate.
        physics.update(std::slice::from_ref(&group), &params, &mut values, 0.0);
        values.insert(ParameterId(1), input);
        let mut out = Vec::new();
        for _ in 0..frames {
            values.insert(ParameterId(1), input);
            physics.update(std::slice::from_ref(&group), &params, &mut values, 1.0 / fps);
            out.push(values.get(&ParameterId(3)).copied().unwrap_or(0.0));
        }
        out
    }

    #[test]
    fn a_still_rig_stays_at_rest() {
        let out = run(60.0, 2.0, 0.0);
        assert!(out.iter().all(|v| v.abs() < 1e-3), "{:?}", &out[..10]);
    }

    #[test]
    fn a_head_turn_makes_the_hair_swing_and_settle() {
        let out = run(60.0, 6.0, 30.0);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.1, "the hair should visibly swing (peak {peak})");
        assert!(
            peak < 0.95,
            "a firm head turn does not saturate the output (peak {peak})"
        );
        let tail = &out[out.len() - 30..];
        let spread = tail.iter().fold(0.0f32, |m, v| m.max((v - tail[0]).abs()));
        assert!(spread < 0.02, "the swing dies down (still moving {spread})");
    }

    #[test]
    fn the_simulation_does_not_depend_on_frame_rate() {
        let slow = run(30.0, 1.0, 30.0);
        let fast = run(120.0, 1.0, 30.0);
        let a = *slow.last().expect("frames");
        let b = *fast.last().expect("frames");
        assert!((a - b).abs() < 0.03, "30 fps ended at {a}, 120 fps at {b}");
    }

    #[test]
    fn stiff_chains_follow_a_tilted_head_and_limp_ones_hang() {
        let (params, mut group) = setup();
        let settle = |group: &PhysicsGroup| {
            let mut physics = PhysicsRuntime::new();
            let mut values: ParamValues = [(ParameterId(2), 30.0)].into_iter().collect();
            for _ in 0..600 {
                physics.update(std::slice::from_ref(group), &params, &mut values, 1.0 / 60.0);
            }
            values[&ParameterId(3)]
        };
        for p in &mut group.particles {
            p.stiffness = 2000.0;
        }
        let stiff = settle(&group);
        for p in &mut group.particles {
            p.stiffness = 0.0;
        }
        let limp = settle(&group);
        assert!(
            stiff.abs() < 0.1,
            "stiff hair keeps its shape relative to the head: {stiff}"
        );
        assert!(limp.abs() > 0.3, "limp hair hangs with gravity: {limp}");
    }

    #[test]
    fn angle_limits_and_colliders_are_respected() {
        let (params, mut group) = setup();
        group.gravity = Vec2::ZERO;
        group.wind = Vec2::new(5000.0, 0.0);
        for p in &mut group.particles {
            p.max_angle = 10.0;
            p.stiffness = 0.0;
        }
        let mut physics = PhysicsRuntime::new();
        let mut values = ParamValues::new();
        for _ in 0..240 {
            physics.update(std::slice::from_ref(&group), &params, &mut values, 1.0 / 60.0);
        }
        let chain = &physics.chains()[0];
        let first = (chain.points()[1] - chain.points()[0]).angle().to_degrees();
        assert!(
            (first - 80.0).abs() < 1.0,
            "the first link stops 10° from vertical: {first}"
        );

        group.wind = Vec2::ZERO;
        group.gravity = Vec2::new(0.0, 1000.0);
        group.colliders = vec![Collider {
            center: Vec2::new(0.0, 24.0),
            radius: 8.0,
        }];
        for p in &mut group.particles {
            p.max_angle = 180.0;
        }
        let mut physics = PhysicsRuntime::new();
        values.insert(ParameterId(1), 1.0);
        for _ in 0..240 {
            physics.update(std::slice::from_ref(&group), &params, &mut values, 1.0 / 60.0);
        }
        let chain = &physics.chains()[0];
        for p in chain.points().iter().skip(1) {
            let d = p.distance(chain.points()[0] + Vec2::new(0.0, 24.0));
            assert!(d >= 7.9, "a particle entered the collider ({d})");
        }
    }

    #[test]
    fn bad_groups_are_reported_and_skipped() {
        let (params, mut group) = setup();
        group.outputs[0].particle = 99;
        assert!(group.validate().is_err());
        let mut physics = PhysicsRuntime::new();
        let mut values = ParamValues::new();
        physics.update(std::slice::from_ref(&group), &params, &mut values, 0.1);
        assert!(values.is_empty());
    }
}
