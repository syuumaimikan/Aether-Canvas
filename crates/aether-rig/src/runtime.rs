//! The rig runtime: everything that depends on time.
//!
//! Evaluating a rig is a pure function of parameter values. The runtime is the
//! part that *produces* those values over time and keeps the state that
//! requires — motion playheads, blink timers, pendulum and jiggle
//! simulations. Each tick runs a fixed pipeline:
//!
//! ```text
//! authored values
//!   → timeline scrub or animator (motions, crossfades)
//!   → expressions
//!   → behaviours (blink, breath, look-at, lip sync)
//!   → drivers
//!   → pendulum physics
//!   = values used for rendering
//!   → jiggle (per-vertex springs on the posed meshes)
//! ```
//!
//! and writes the result into [`Rig::dynamics`], which evaluation picks up.
//! Stopping the runtime clears it, returning the rig to its authored pose.

use crate::behaviour::{BehaviourInputs, BehaviourState};
use crate::driver::{self, DriverIssue};
use crate::hotkey::{self, HotkeyAction, KeyChord};
use crate::motion::{Animator, MotionBlend};
use crate::param::ParamValues;
use crate::physics::PhysicsRuntime;
use crate::rig::{Evaluator, Rig};
use aether_core::math::Vec2;
use aether_core::LayerId;
use std::collections::BTreeMap;

/// Fixed step for jiggle, seconds.
const JIGGLE_STEP: f32 = 1.0 / 120.0;

/// Which stages run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeSettings {
    /// Motions and the timeline scrub.
    pub motions: bool,
    /// Blink, breath, look-at and lip sync.
    pub behaviours: bool,
    /// Parameter drivers.
    pub drivers: bool,
    /// Pendulum physics.
    pub physics: bool,
    /// Mesh jiggle.
    pub jiggle: bool,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            motions: true,
            behaviours: true,
            drivers: true,
            physics: true,
            jiggle: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct JiggleState {
    pos: Vec<Vec2>,
    vel: Vec<Vec2>,
}

/// Time-dependent state for playing a rig.
#[derive(Clone, Debug, Default)]
pub struct RigRuntime {
    /// Enabled stages.
    pub settings: RuntimeSettings,
    /// Motion layers.
    pub animator: Animator,
    /// Look target and audio level.
    pub inputs: BehaviourInputs,
    /// When set, motion `.0` is shown at time `.1` (timeline scrubbing)
    /// instead of whatever the animator is playing.
    pub scrub: Option<(usize, f32)>,
    /// Problems reported by drivers on the last tick.
    pub issues: Vec<DriverIssue>,
    /// A looping motion that plays whenever no other motion does: a motion
    /// that plays once hands back to it when it ends. Hotkeys set it.
    pub idle: Option<usize>,
    behaviours: BehaviourState,
    physics: PhysicsRuntime,
    jiggle: BTreeMap<LayerId, JiggleState>,
    jiggle_accumulator: f32,
    /// Expression weights: index → (current, target).
    expressions: BTreeMap<usize, (f32, f32)>,
    /// Live2D models' physics and pose fades.
    cubism: Vec<crate::cubism::CubismRuntime>,
    time: f64,
}

impl RigRuntime {
    /// A runtime with every stage enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seconds simulated so far.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// The pendulum simulation, for drawing chains.
    pub fn physics(&self) -> &PhysicsRuntime {
        &self.physics
    }

    /// Forget all simulated motion (chains hang still, jiggle stops).
    pub fn reset(&mut self) {
        self.physics.reset();
        self.cubism.clear();
        self.jiggle.clear();
        self.jiggle_accumulator = 0.0;
    }

    /// Stop everything and return the rig to its authored pose.
    pub fn stop(&mut self, rig: &mut Rig) {
        self.animator = Animator::default();
        self.scrub = None;
        self.idle = None;
        self.reset();
        rig.dynamics = Default::default();
    }

    /// Start a motion on the animator.
    pub fn play(&mut self, rig: &Rig, motion: usize) {
        self.scrub = None;
        self.animator.play(&rig.motions, motion, MotionBlend::Override);
    }

    /// Fade to expression `index`, or out of all expressions for `None`.
    pub fn set_expression(&mut self, index: Option<usize>) {
        for (i, (_, target)) in self.expressions.iter_mut() {
            *target = if Some(*i) == index { 1.0 } else { 0.0 };
        }
        if let Some(i) = index {
            self.expressions.entry(i).or_insert((0.0, 1.0)).1 = 1.0;
        }
    }

    /// Switch expression `index` on or off, leaving the others as they are.
    pub fn toggle_expression(&mut self, index: usize) {
        let weight = self.expressions.entry(index).or_insert((0.0, 0.0));
        weight.1 = if weight.1 > 0.5 { 0.0 } else { 1.0 };
    }

    /// The expressions switched on (fading in or shown), in index order.
    pub fn active_expressions(&self) -> Vec<usize> {
        self.expressions
            .iter()
            .filter(|(_, (_, target))| *target > 0.5)
            .map(|(i, _)| *i)
            .collect()
    }

    /// Carry out a hotkey's action (see [`HotkeyAction`]). Returns false,
    /// doing nothing, when the motion or expression it names does not exist.
    pub fn trigger(&mut self, rig: &Rig, action: &HotkeyAction) -> bool {
        match action {
            HotkeyAction::PlayMotion(name) => {
                let Some(index) = rig.motions.iter().position(|m| &m.name == name) else {
                    return false;
                };
                self.scrub = None;
                if rig.motions[index].looping {
                    if self.idle == Some(index) && self.animator.is_playing(index) {
                        self.idle = None;
                        self.animator.stop(&rig.motions, index);
                        return true;
                    }
                    self.idle = Some(index);
                }
                self.animator.play(&rig.motions, index, MotionBlend::Override);
            }
            HotkeyAction::ToggleExpression(name) => {
                let Some(index) = rig.expressions.iter().position(|e| &e.name == name) else {
                    return false;
                };
                self.toggle_expression(index);
            }
            HotkeyAction::ClearExpressions => self.set_expression(None),
            HotkeyAction::StopMotions => {
                self.idle = None;
                self.animator.stop_all(&rig.motions);
            }
            HotkeyAction::Reset => {
                self.idle = None;
                self.animator.stop_all(&rig.motions);
                self.set_expression(None);
            }
        }
        true
    }

    /// Press `keys`: trigger the rig's hotkey bound to them. Returns its
    /// index when one fired.
    pub fn press(&mut self, rig: &Rig, keys: &KeyChord) -> Option<usize> {
        let index = hotkey::find(&rig.hotkeys, keys)?;
        let action = rig.hotkeys[index].action.clone();
        self.trigger(rig, &action).then_some(index)
    }

    /// The expression currently fading in, if any.
    pub fn active_expression(&self) -> Option<usize> {
        self.expressions
            .iter()
            .find(|(_, (_, target))| *target > 0.5)
            .map(|(i, _)| *i)
    }

    /// Advance `dt` seconds and write the result into `rig.dynamics`.
    pub fn tick(&mut self, rig: &mut Rig, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        self.time += dt as f64;
        let mut values: ParamValues = rig.values.clone();

        if self.settings.motions {
            match self.scrub {
                Some((motion, time)) => {
                    if let Some(m) = rig.motions.get(motion) {
                        m.apply(&rig.parameters, time, 1.0, false, &mut values);
                    }
                }
                None => {
                    if let Some(idle) = self.idle {
                        if idle >= rig.motions.len() {
                            self.idle = None;
                        } else if !self.animator.has_active() {
                            self.animator.play(&rig.motions, idle, MotionBlend::Override);
                        }
                    }
                    self.animator
                        .update(&rig.motions, &rig.parameters, &mut values, dt)
                }
            }
        }

        for (index, (current, target)) in self.expressions.iter_mut() {
            let fade = rig.expressions.get(*index).map(|e| e.fade).unwrap_or(0.25);
            let step = if fade > 0.0 { dt / fade } else { 1.0 };
            if *current < *target {
                *current = (*current + step).min(*target);
            } else {
                *current = (*current - step).max(*target);
            }
            if let Some(expression) = rig.expressions.get(*index) {
                if *current > 0.0 {
                    expression.apply(&rig.parameters, *current, &mut values);
                }
            }
        }
        self.expressions
            .retain(|_, (current, target)| *current > 0.0 || *target > 0.0);

        if self.settings.behaviours {
            self.behaviours
                .update(&rig.behaviours, &rig.parameters, &self.inputs, &mut values, dt);
        }

        self.issues = if self.settings.drivers {
            driver::apply(&rig.parameters, &rig.drivers, &mut values, self.time as f32)
        } else {
            Vec::new()
        };

        if self.settings.physics {
            self.physics
                .update(&rig.physics, &rig.parameters, &mut values, dt);
        }

        // Live2D models: their own physics, then pose fades.
        self.cubism
            .retain(|state| rig.cubism.iter().any(|m| state.matches(m)));
        let mut cubism_parts = BTreeMap::new();
        for model in &rig.cubism {
            let index = match self.cubism.iter().position(|s| s.matches(model)) {
                Some(i) => i,
                None => {
                    self.cubism.push(crate::cubism::CubismRuntime::new(model));
                    self.cubism.len() - 1
                }
            };
            let state = &mut self.cubism[index];
            state.step(model, &rig.parameters, &mut values, dt, self.settings.physics);
            cubism_parts.insert(model.layer, state.part_opacities().to_vec());
        }

        let offsets = if self.settings.jiggle {
            self.step_jiggle(rig, &values, dt)
        } else {
            self.jiggle.clear();
            BTreeMap::new()
        };

        rig.dynamics.values = Some(values);
        rig.dynamics.offsets = offsets;
        rig.dynamics.cubism_parts = cubism_parts;
    }

    fn step_jiggle(&mut self, rig: &Rig, values: &ParamValues, dt: f32) -> BTreeMap<LayerId, Vec<Vec2>> {
        let mut out = BTreeMap::new();
        if !rig
            .meshes
            .iter()
            .any(|m| m.jiggle.as_ref().is_some_and(|j| j.enabled))
        {
            self.jiggle.clear();
            return out;
        }
        let eval = Evaluator::with_values(rig, values);
        self.jiggle_accumulator += dt;
        let mut steps = 0;
        while self.jiggle_accumulator >= JIGGLE_STEP && steps < 60 {
            self.jiggle_accumulator -= JIGGLE_STEP;
            steps += 1;
        }
        if steps == 60 {
            self.jiggle_accumulator = 0.0;
        }
        for mesh in &rig.meshes {
            let Some(jiggle) = mesh.jiggle.as_ref().filter(|j| j.enabled) else {
                self.jiggle.remove(&mesh.layer);
                continue;
            };
            let targets = eval.mesh_pose(mesh).positions;
            let state = self.jiggle.entry(mesh.layer).or_default();
            if state.pos.len() != targets.len() {
                state.pos = targets.clone();
                state.vel = vec![Vec2::ZERO; targets.len()];
            }
            let k = jiggle.stiffness.max(0.0);
            let c = jiggle.damping.max(0.0);
            let limit = jiggle.max_offset.max(0.0);
            for _ in 0..steps {
                for (v, target) in targets.iter().enumerate() {
                    let w = jiggle.weights.get(v).copied().unwrap_or(0.0);
                    if w <= 0.0 {
                        state.pos[v] = *target;
                        state.vel[v] = Vec2::ZERO;
                        continue;
                    }
                    let acc = (*target - state.pos[v]) * k - state.vel[v] * c + jiggle.gravity;
                    state.vel[v] += acc * JIGGLE_STEP;
                    state.pos[v] += state.vel[v] * JIGGLE_STEP;
                    let off = state.pos[v] - *target;
                    let d = off.length();
                    if d > limit && d > 0.0 {
                        state.pos[v] = *target + off * (limit / d);
                    }
                }
            }
            let offsets: Vec<Vec2> = targets
                .iter()
                .enumerate()
                .map(|(v, t)| {
                    (state.pos[v] - *t) * jiggle.weights.get(v).copied().unwrap_or(0.0).clamp(0.0, 1.0)
                })
                .collect();
            out.insert(mesh.layer, offsets);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::Driver;
    use crate::keyform::KeyAxis;
    use crate::mesh::{ArtMesh, Jiggle};
    use crate::motion::Motion;
    use crate::param::Parameter;
    use crate::rig::RigNode;
    use aether_core::math::{vec2, Rect};
    use aether_core::ParameterId;

    fn rig() -> Rig {
        let mut rig = Rig::new();
        rig.add_parameter(Parameter::new(ParameterId(1), "AngleX", -30.0, 30.0, 0.0))
            .expect("param");
        rig.add_parameter(Parameter::new(ParameterId(2), "BodyAngleX", -10.0, 10.0, 0.0))
            .expect("param");
        let mut mesh = ArtMesh::quad(LayerId(9), Rect::from_corners(vec2(0.0, 0.0), vec2(10.0, 10.0)));
        mesh.keyforms
            .add_axis(KeyAxis::new(ParameterId(1), [-30.0, 30.0]).expect("axis"))
            .expect("axis");
        for o in &mut mesh.keyforms.forms[0].offsets {
            *o = vec2(-30.0, 0.0);
        }
        for o in &mut mesh.keyforms.forms[1].offsets {
            *o = vec2(30.0, 0.0);
        }
        rig.set_mesh(mesh);
        rig
    }

    #[test]
    fn scrubbing_shows_the_motion_at_the_playhead() {
        let mut rig = rig();
        let mut motion = Motion::new("turn", 1.0, 30.0);
        motion.track_mut(ParameterId(1)).set_key(0.0, -30.0);
        motion.track_mut(ParameterId(1)).set_key(1.0, 30.0);
        rig.motions.push(motion);
        let mut runtime = RigRuntime::new();
        runtime.scrub = Some((0, 0.5));
        runtime.tick(&mut rig, 0.0);
        assert!((rig.effective_value(ParameterId(1))).abs() < 1e-4);
        assert_eq!(rig.value(ParameterId(1)), 0.0, "authored values are untouched");
        runtime.stop(&mut rig);
        assert!(rig.dynamics.values.is_none());
    }

    /// A rig with a looping sway (AngleX ±30), a one-shot nod (BodyAngleX
    /// to 10) and two expressions, bound to keys.
    fn performer() -> Rig {
        use crate::hotkey::{Hotkey, HotkeyAction, KeyChord};
        use crate::motion::{Expression, ExpressionBlend, ExpressionEntry};
        let mut rig = rig();
        let mut sway = Motion::new("Sway", 2.0, 30.0);
        sway.looping = true;
        sway.fade_in = 0.0;
        for (t, v) in [(0.0, 30.0), (2.0, 30.0)] {
            sway.track_mut(ParameterId(1)).set_key(t, v);
        }
        let mut nod = Motion::new("Nod", 0.5, 30.0);
        nod.looping = false;
        nod.fade_in = 0.0;
        nod.fade_out = 0.1;
        for (t, v) in [(0.0, 10.0), (0.5, 10.0)] {
            nod.track_mut(ParameterId(2)).set_key(t, v);
        }
        rig.motions = vec![sway, nod];
        for (name, param, value) in [("Left", 1, -20.0), ("Lean", 2, -5.0)] {
            rig.expressions.push(Expression {
                name: name.into(),
                entries: vec![ExpressionEntry {
                    param: ParameterId(param),
                    value,
                    blend: ExpressionBlend::Add,
                }],
                fade: 0.0,
            });
        }
        let key = |k: &str| KeyChord::parse(k).expect("key");
        rig.hotkeys = vec![
            Hotkey::new(key("1"), HotkeyAction::PlayMotion("Sway".into())),
            Hotkey::new(key("2"), HotkeyAction::PlayMotion("Nod".into())),
            Hotkey::new(key("Shift+1"), HotkeyAction::ToggleExpression("Left".into())),
            Hotkey::new(key("Shift+2"), HotkeyAction::ToggleExpression("Lean".into())),
            Hotkey::new(key("0"), HotkeyAction::Reset),
            Hotkey::new(key("9"), HotkeyAction::PlayMotion("Missing".into())),
        ];
        rig
    }

    #[test]
    fn hotkeys_play_motions_over_an_idle_loop() {
        use crate::hotkey::KeyChord;
        let mut rig = performer();
        let mut runtime = RigRuntime::new();
        let press =
            |runtime: &mut RigRuntime, rig: &Rig, k: &str| runtime.press(rig, &KeyChord::parse(k).unwrap());
        let run = |runtime: &mut RigRuntime, rig: &mut Rig, seconds: f32| {
            for _ in 0..(seconds * 60.0) as usize {
                runtime.tick(rig, 1.0 / 60.0);
            }
        };

        // 1 starts the looping sway: it becomes the idle loop.
        assert_eq!(press(&mut runtime, &rig, "1"), Some(0));
        run(&mut runtime, &mut rig, 0.2);
        assert!((rig.effective_value(ParameterId(1)) - 30.0).abs() < 1e-3);

        // 2 plays the nod over it, and the sway comes back when it ends.
        assert_eq!(press(&mut runtime, &rig, "2"), Some(1));
        run(&mut runtime, &mut rig, 0.3);
        assert!(rig.effective_value(ParameterId(2)) > 9.0, "the nod plays");
        run(&mut runtime, &mut rig, 1.5);
        assert!(
            rig.effective_value(ParameterId(2)).abs() < 1e-3,
            "the nod is over"
        );
        assert!(
            (rig.effective_value(ParameterId(1)) - 30.0).abs() < 1e-3,
            "and the sway is back: {}",
            rig.effective_value(ParameterId(1))
        );

        // Pressing 1 again stops the loop.
        press(&mut runtime, &rig, "1");
        run(&mut runtime, &mut rig, 1.0);
        assert!(rig.effective_value(ParameterId(1)).abs() < 1e-3);
        assert!(runtime.animator.is_idle());

        // Unbound keys and missing motions do nothing.
        assert_eq!(press(&mut runtime, &rig, "5"), None);
        assert_eq!(press(&mut runtime, &rig, "9"), None);
    }

    #[test]
    fn expression_hotkeys_toggle_and_stack() {
        use crate::hotkey::KeyChord;
        let mut rig = performer();
        let mut runtime = RigRuntime::new();
        let press =
            |runtime: &mut RigRuntime, rig: &Rig, k: &str| runtime.press(rig, &KeyChord::parse(k).unwrap());
        press(&mut runtime, &rig, "Shift+1");
        press(&mut runtime, &rig, "Shift+2");
        runtime.tick(&mut rig, 1.0 / 60.0);
        assert_eq!(runtime.active_expressions(), vec![0, 1], "both are on");
        assert!((rig.effective_value(ParameterId(1)) + 20.0).abs() < 1e-3);
        assert!((rig.effective_value(ParameterId(2)) + 5.0).abs() < 1e-3);
        press(&mut runtime, &rig, "Shift+1");
        runtime.tick(&mut rig, 1.0 / 60.0);
        assert_eq!(runtime.active_expressions(), vec![1], "the first is off again");
        assert!(rig.effective_value(ParameterId(1)).abs() < 1e-3);
        press(&mut runtime, &rig, "1");
        press(&mut runtime, &rig, "0");
        for _ in 0..60 {
            runtime.tick(&mut rig, 1.0 / 60.0);
        }
        assert!(runtime.active_expressions().is_empty(), "0 resets");
        assert!(runtime.animator.is_idle() && runtime.idle.is_none());
    }

    #[test]
    fn drivers_run_inside_the_runtime() {
        let mut rig = rig();
        rig.drivers.push(Driver::new(ParameterId(2), "AngleX / 3"));
        rig.set_value(ParameterId(1), 30.0);
        let mut runtime = RigRuntime::new();
        runtime.tick(&mut rig, 1.0 / 60.0);
        assert_eq!(rig.effective_value(ParameterId(2)), 10.0);
        assert!(runtime.issues.is_empty());
    }

    #[test]
    fn jiggle_lags_behind_and_settles() {
        let mut rig = rig();
        if let Some(mesh) = rig.mesh_mut(LayerId(9)) {
            mesh.jiggle = Some(Jiggle::new(4));
        }
        let mut runtime = RigRuntime::new();
        runtime.tick(&mut rig, 1.0 / 60.0);
        // Snap the pose far to the right.
        rig.set_value(ParameterId(1), 30.0);
        runtime.tick(&mut rig, 1.0 / 60.0);
        let lag = rig.dynamics.offsets[&LayerId(9)][0];
        assert!(lag.x < -1.0, "vertices lag behind a sudden move: {lag:?}");
        for _ in 0..600 {
            runtime.tick(&mut rig, 1.0 / 60.0);
        }
        let settled = rig.dynamics.offsets[&LayerId(9)][0];
        assert!(settled.length() < 0.05, "and settle: {settled:?}");
        let pose = rig.evaluate();
        assert!((pose.meshes[&LayerId(9)].positions[0].x - 30.0).abs() < 0.1);
    }

    #[test]
    fn expressions_fade_in_and_out() {
        let mut rig = rig();
        rig.expressions.push(crate::motion::Expression {
            name: "look right".into(),
            entries: vec![crate::motion::ExpressionEntry {
                param: ParameterId(1),
                value: 30.0,
                blend: crate::motion::ExpressionBlend::Overwrite,
            }],
            fade: 0.5,
        });
        let mut runtime = RigRuntime::new();
        runtime.set_expression(Some(0));
        runtime.tick(&mut rig, 0.25);
        let half = rig.effective_value(ParameterId(1));
        assert!(half > 5.0 && half < 25.0, "fading in: {half}");
        runtime.tick(&mut rig, 0.5);
        assert!((rig.effective_value(ParameterId(1)) - 30.0).abs() < 1e-3);
        runtime.set_expression(None);
        for _ in 0..10 {
            runtime.tick(&mut rig, 0.1);
        }
        assert!(rig.effective_value(ParameterId(1)).abs() < 1e-3);
        assert_eq!(runtime.active_expression(), None);
    }

    #[test]
    fn disabled_stages_do_nothing() {
        let mut rig = rig();
        rig.drivers.push(Driver::new(ParameterId(2), "10"));
        let mut runtime = RigRuntime::new();
        runtime.settings.drivers = false;
        runtime.tick(&mut rig, 0.1);
        assert_eq!(rig.effective_value(ParameterId(2)), 0.0);
        let _ = RigNode::Mesh(LayerId(9));
    }
}
