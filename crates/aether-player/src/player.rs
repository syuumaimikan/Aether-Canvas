//! Playing a model: time, input, and the per-frame draw list.

use crate::model::{BlendKind, Model, ModelError, Node};
use crate::tracking::{face_targets, FaceFrame, Tracking, TrackingSettings};
use aether_core::math::{Rect, Vec2};
use aether_rig::motion::MotionBlend;
use aether_rig::runtime::RuntimeSettings;
use aether_rig::{Parameter, RigRuntime};

/// One draw call, in back-to-front order.
///
/// The layout is fixed (`repr(C)`) because the C and WebAssembly APIs hand
/// out the draw list as an array of these.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct DrawItem {
    /// Part to draw.
    pub part: u32,
    /// Part whose coverage clips this one, or `-1`.
    pub mask: i32,
    /// [`BlendKind`] as a number.
    pub blend: u32,
    /// Final opacity.
    pub opacity: f32,
    /// Opacity to draw the mask part's coverage with (its keyed opacity).
    pub mask_opacity: f32,
    /// Multiply tint, applied to straight colour.
    pub multiply: [f32; 3],
    /// Screen tint, applied after multiply.
    pub screen: [f32; 3],
}

impl DrawItem {
    /// The blend mode.
    pub fn blend_kind(&self) -> BlendKind {
        BlendKind::from_code(self.blend).unwrap_or_default()
    }

    /// The clipping part, if any.
    pub fn mask_part(&self) -> Option<usize> {
        usize::try_from(self.mask).ok()
    }
}

/// A timeline event that a playing motion passed over.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredEvent {
    /// Index of the motion.
    pub motion: usize,
    /// Event name.
    pub name: String,
}

/// Which runtime stage to switch, for [`Player::set_stage`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Motions.
    Motions,
    /// Blink, breath, look-at, lip sync.
    Behaviours,
    /// Expression drivers.
    Drivers,
    /// Pendulum physics.
    Physics,
    /// Mesh jiggle.
    Jiggle,
}

impl Stage {
    /// The stage for a numeric code, as used by the C API.
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Motions),
            1 => Some(Self::Behaviours),
            2 => Some(Self::Drivers),
            3 => Some(Self::Physics),
            4 => Some(Self::Jiggle),
            _ => None,
        }
    }
}

/// Keyed appearance of one part at the current pose.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    opacity: f32,
    multiply: [f32; 3],
    screen: [f32; 3],
    draw_order: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            multiply: [1.0; 3],
            screen: [0.0; 3],
            draw_order: 0.0,
        }
    }
}

/// A loaded model, ready to animate and draw.
///
/// Call [`Player::tick`] once per frame, then draw [`Player::draw_list`] in
/// order: each item names a part whose triangles
/// ([`crate::Part::triangles`]) are drawn with texture coordinates
/// ([`crate::Part::uvs`]) at [`Player::positions`].
#[derive(Clone, Debug)]
pub struct Player {
    model: Model,
    runtime: RigRuntime,
    positions: Vec<Vec<f32>>,
    looks: Vec<Look>,
    draw_list: Vec<DrawItem>,
    events: Vec<FiredEvent>,
    tracking: Tracking,
}

impl Player {
    /// Load a validated model.
    pub fn new(model: Model) -> Result<Self, ModelError> {
        model.validate()?;
        let parts = model.parts.len();
        let mut player = Self {
            model,
            runtime: RigRuntime::new(),
            positions: vec![Vec::new(); parts],
            looks: vec![Look::default(); parts],
            draw_list: Vec::new(),
            events: Vec::new(),
            tracking: Tracking::default(),
        };
        player.update();
        Ok(player)
    }

    /// Parse `model.json` and load it.
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        Self::new(Model::from_json(text)?)
    }

    /// The model being played.
    pub fn model(&self) -> &Model {
        &self.model
    }

    // ---- parameters -------------------------------------------------------

    /// Parameter definitions, in panel order.
    pub fn parameters(&self) -> &[Parameter] {
        &self.model.rig.parameters
    }

    /// Index of the parameter with this name.
    pub fn parameter_index(&self, name: &str) -> Option<usize> {
        self.parameters().iter().position(|p| p.name == name)
    }

    /// Set a parameter's base value (clamped to its range). Motions,
    /// behaviours, drivers and physics layer on top of it at the next
    /// [`Player::tick`] or [`Player::update`].
    pub fn set_parameter(&mut self, index: usize, value: f32) {
        if let Some(id) = self.parameters().get(index).map(|p| p.id) {
            self.model.rig.set_value(id, value);
        }
    }

    /// A parameter's value as last drawn (after motions, physics and so on).
    pub fn parameter_value(&self, index: usize) -> f32 {
        self.parameters()
            .get(index)
            .map(|p| self.model.rig.effective_value(p.id))
            .unwrap_or(0.0)
    }

    /// Every parameter back to its default, simulations at rest, motions
    /// stopped.
    pub fn reset(&mut self) {
        self.stop_tracking();
        self.runtime.stop(&mut self.model.rig);
        self.model.rig.reset_values();
        self.update();
    }

    // ---- motions and expressions -----------------------------------------

    /// Index of the motion with this name.
    pub fn motion_index(&self, name: &str) -> Option<usize> {
        self.model.rig.motions.iter().position(|m| m.name == name)
    }

    /// Start a motion. An overriding motion crossfades from the current one;
    /// an additive motion layers on top. Returns false for a bad index.
    pub fn play_motion(&mut self, index: usize, additive: bool) -> bool {
        if index >= self.model.rig.motions.len() {
            return false;
        }
        let blend = if additive {
            MotionBlend::Additive
        } else {
            MotionBlend::Override
        };
        self.runtime.scrub = None;
        self.runtime.animator.play(&self.model.rig.motions, index, blend);
        true
    }

    /// Fade every playing motion out.
    pub fn stop_motions(&mut self) {
        self.runtime.animator.stop_all(&self.model.rig.motions);
    }

    /// True while any motion is playing or fading.
    pub fn is_playing(&self) -> bool {
        !self.runtime.animator.is_idle()
    }

    /// Index of the expression with this name.
    pub fn expression_index(&self, name: &str) -> Option<usize> {
        self.model.rig.expressions.iter().position(|e| e.name == name)
    }

    /// Fade to an expression, or out of all of them for `None`.
    pub fn set_expression(&mut self, index: Option<usize>) {
        let index = index.filter(|&i| i < self.model.rig.expressions.len());
        self.runtime.set_expression(index);
    }

    // ---- inputs -----------------------------------------------------------

    /// Look towards a point, `-1..=1` on each axis with y up, or straight
    /// ahead for `None`. Drives the look-at behaviour, switching it on the
    /// first time a target arrives.
    pub fn look_at(&mut self, target: Option<(f32, f32)>) {
        let target = target
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .map(|(x, y)| Vec2::new(x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0)));
        if target.is_some() {
            self.model.rig.behaviours.look.enabled = true;
        }
        self.runtime.inputs.look = target;
    }

    /// Voice loudness (`0..=1`) and brightness (`-1..=1`), for lip sync.
    /// Lip sync switches on the first time a sound arrives.
    pub fn set_audio(&mut self, level: f32, brightness: f32) {
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        if finite(level) > 0.0 {
            self.model.rig.behaviours.lip_sync.enabled = true;
        }
        self.runtime.inputs.audio_level = finite(level).clamp(0.0, 1.0);
        self.runtime.inputs.audio_brightness = finite(brightness).clamp(-1.0, 1.0);
    }

    /// Switch a runtime stage on or off.
    pub fn set_stage(&mut self, stage: Stage, enabled: bool) {
        let s: &mut RuntimeSettings = &mut self.runtime.settings;
        match stage {
            Stage::Motions => s.motions = enabled,
            Stage::Behaviours => s.behaviours = enabled,
            Stage::Drivers => s.drivers = enabled,
            Stage::Physics => s.physics = enabled,
            Stage::Jiggle => s.jiggle = enabled,
        }
    }

    // ---- face tracking ----------------------------------------------------

    /// Feed one face-tracker sample (see [`crate::tracking`] for the
    /// conventions). The head, eyes, brows and mouth follow it, smoothed over
    /// the following ticks. While tracking, auto-blink is paused so it does
    /// not fight the tracked eyes, and look-at input is ignored.
    pub fn track_face(&mut self, frame: &FaceFrame) {
        if self.tracking.blink_was_enabled.is_none() {
            let blink = &mut self.model.rig.behaviours.blink;
            self.tracking.blink_was_enabled = Some(blink.enabled);
            blink.enabled = false;
        }
        self.runtime.inputs.look = None;
        self.tracking.targets = face_targets(
            &self.model.rig,
            frame,
            &self.tracking.neutral,
            &self.tracking.settings,
        );
        self.tracking.latest = Some(frame.clone());
    }

    /// Take the latest tracked face as the resting one: the angles and
    /// expressions measured now become the model's neutral pose.
    pub fn calibrate_tracking(&mut self) {
        if let Some(latest) = self.tracking.latest.clone() {
            self.tracking.neutral = latest.clone();
            self.track_face(&latest);
        }
    }

    /// Stop following the tracker: tracked parameters return to their
    /// defaults and auto-blink resumes if it was on.
    pub fn stop_tracking(&mut self) {
        if let Some(enabled) = self.tracking.blink_was_enabled.take() {
            self.model.rig.behaviours.blink.enabled = enabled;
        }
        for (id, _) in std::mem::take(&mut self.tracking.current) {
            if let Some(default) = self.model.rig.parameter(id).map(|p| p.default) {
                self.model.rig.set_value(id, default);
            }
        }
        self.tracking.targets.clear();
        self.tracking.latest = None;
    }

    /// True while a tracker is driving the model.
    pub fn is_tracking(&self) -> bool {
        self.tracking.latest.is_some()
    }

    /// How tracking maps onto the model (mirroring, smoothing, gains).
    pub fn tracking_settings_mut(&mut self) -> &mut TrackingSettings {
        &mut self.tracking.settings
    }

    // ---- time -------------------------------------------------------------

    /// Advance `dt` seconds and recompute the pose.
    pub fn tick(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        if self.is_tracking() {
            self.tracking.step(dt);
            for &(id, value) in &self.tracking.current {
                self.model.rig.set_value(id, value);
            }
        }
        self.collect_events(dt);
        self.runtime.tick(&mut self.model.rig, dt);
        self.update();
    }

    /// Recompute the pose without advancing time (after changing parameters
    /// on a paused player, say).
    pub fn update(&mut self) {
        let pose = self.model.rig.evaluate();
        for (i, part) in self.model.parts.iter().enumerate() {
            let out = &mut self.positions[i];
            out.clear();
            let (points, look) = match pose.mesh(part.layer) {
                Some(mesh) if mesh.positions.len() == part.vertices.len() => (
                    &mesh.positions,
                    Look {
                        opacity: mesh.opacity,
                        multiply: mesh.multiply,
                        screen: mesh.screen,
                        draw_order: mesh.draw_order,
                    },
                ),
                _ => (&part.vertices, Look::default()),
            };
            match &part.transform {
                Some(t) => out.extend(points.iter().flat_map(|&p| {
                    let q = t.apply(p);
                    [q.x, q.y]
                })),
                None => out.extend(points.iter().flat_map(|p| [p.x, p.y])),
            }
            self.looks[i] = look;
        }
        self.draw_list.clear();
        emit(&self.model.tree, &self.model, &self.looks, &mut self.draw_list);
    }

    /// Events passed since the previous tick. They are replaced on every
    /// tick, so read them each frame.
    pub fn events(&self) -> &[FiredEvent] {
        &self.events
    }

    fn collect_events(&mut self, dt: f32) {
        self.events.clear();
        if dt <= 0.0 || !self.runtime.settings.motions || self.runtime.scrub.is_some() {
            return;
        }
        for layer in &self.runtime.animator.layers {
            let Some(motion) = self.model.rig.motions.get(layer.motion) else {
                continue;
            };
            let t0 = layer.time;
            let t1 = t0 + dt * layer.speed;
            for event in &motion.events {
                let fires = if motion.looping && motion.duration > 0.0 {
                    // Occurrences at `event.time + k * duration` in [t0, t1).
                    let k = |t: f32| ((t - event.time) / motion.duration).ceil();
                    k(t1) > k(t0)
                } else {
                    t0 <= event.time && event.time < t1
                };
                if fires {
                    self.events.push(FiredEvent {
                        motion: layer.motion,
                        name: event.name.clone(),
                    });
                }
            }
        }
    }

    // ---- output -----------------------------------------------------------

    /// Draw calls for the current pose, back to front.
    pub fn draw_list(&self) -> &[DrawItem] {
        &self.draw_list
    }

    /// A part's posed vertex positions in document pixels, flat
    /// `[x0, y0, x1, y1, ...]`.
    pub fn positions(&self, part: usize) -> &[f32] {
        self.positions.get(part).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The topmost visible part under a document-space point, for tap and
    /// click interactions. Clipped parts only count inside their base.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        let p = Vec2::new(x, y);
        self.draw_list
            .iter()
            .rev()
            .filter(|item| item.opacity > 1e-3)
            .find(|item| {
                self.contains(item.part as usize, p)
                    && item.mask_part().is_none_or(|base| self.contains(base, p))
            })
            .map(|item| item.part as usize)
    }

    fn contains(&self, part: usize, p: Vec2) -> bool {
        let (Some(info), Some(pos)) = (self.model.parts.get(part), self.positions.get(part)) else {
            return false;
        };
        let at = |i: u32| Vec2::new(pos[i as usize * 2], pos[i as usize * 2 + 1]);
        info.triangles.iter().any(|t| {
            let (a, b, c) = (at(t[0]), at(t[1]), at(t[2]));
            let d1 = (b - a).cross(p - a);
            let d2 = (c - b).cross(p - b);
            let d3 = (a - c).cross(p - c);
            let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            !(negative && positive)
        })
    }

    /// Bounding box of everything visible at the current pose.
    pub fn bounds(&self) -> Rect {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for item in self.draw_list.iter().filter(|i| i.opacity > 1e-3) {
            for c in self.positions(item.part as usize).chunks_exact(2) {
                min = min.min(Vec2::new(c[0], c[1]));
                max = max.max(Vec2::new(c[0], c[1]));
            }
        }
        if min.x > max.x {
            Rect::ZERO
        } else {
            Rect::from_corners(min, max)
        }
    }
}

/// Append the draw calls for `nodes` (siblings), sorted by keyed draw order.
fn emit(nodes: &[Node], model: &Model, looks: &[Look], out: &mut Vec<DrawItem>) {
    let key = |node: &Node| match node {
        Node::Group { index, .. } => *index as f32,
        Node::Part { index, part, .. } => *index as f32 + looks[*part as usize].draw_order,
    };
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    // Stable, so the tree order stands wherever no offsets are keyed.
    order.sort_by(|&a, &b| key(&nodes[a]).total_cmp(&key(&nodes[b])));

    let item = |part: u32, mask: Option<u32>| {
        let info = &model.parts[part as usize];
        let look = looks[part as usize];
        DrawItem {
            part,
            mask: mask.map(|m| m as i32).unwrap_or(-1),
            blend: info.blend as u32,
            opacity: (info.opacity * look.opacity).clamp(0.0, 1.0),
            mask_opacity: mask
                .map(|m| looks[m as usize].opacity.clamp(0.0, 1.0))
                .unwrap_or(0.0),
            multiply: look.multiply,
            screen: look.screen,
        }
    };
    for i in order {
        match &nodes[i] {
            Node::Group { children, .. } => emit(children, model, looks, out),
            Node::Part { part, clipped, .. } => {
                out.push(item(*part, None));
                out.extend(clipped.iter().map(|&c| item(c, Some(*part))));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Part, Texture};
    use aether_core::id::IdGenerator;
    use aether_core::LayerId;
    use aether_rig::motion::{Motion, MotionEvent};
    use aether_rig::{ArtMesh, Parameter};

    fn square(layer: u64, x: f32) -> Part {
        let v = |px: f32, py: f32| Vec2::new(x + px, py);
        Part {
            layer: LayerId(layer),
            name: format!("square {layer}"),
            texture: 0,
            vertices: vec![v(0.0, 0.0), v(10.0, 0.0), v(10.0, 10.0), v(0.0, 10.0)],
            uvs: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            opacity: 1.0,
            blend: BlendKind::Normal,
            transform: None,
        }
    }

    /// Two squares side by side; the second is rigged to slide right with
    /// parameter "Slide" and rise above the first when it is at 1.
    fn model() -> Model {
        let ids = IdGenerator::new();
        let mut model = Model::new("test", 64, 32);
        model.textures.push(Texture {
            file: "t.png".into(),
            width: 8,
            height: 8,
        });
        model.parts = vec![square(1, 0.0), square(2, 5.0)];
        model.tree = vec![
            Node::Part {
                index: 0,
                part: 1,
                clipped: vec![],
            },
            Node::Part {
                index: 1,
                part: 0,
                clipped: vec![],
            },
        ];
        let rig = &mut model.rig;
        let slide = rig
            .add_parameter(Parameter::new(ids.parameter(), "Slide", 0.0, 1.0, 0.0))
            .unwrap();
        let mut mesh = ArtMesh::new(
            LayerId(2),
            model.parts[1].vertices.clone(),
            model.parts[1].triangles.clone(),
        );
        mesh.name = "slider".into();
        rig.set_mesh(mesh);
        rig.bind_parameter(aether_rig::RigNode::Mesh(LayerId(2)), slide, &[0.0, 1.0])
            .unwrap();
        let form = &mut rig.mesh_mut(LayerId(2)).unwrap().keyforms.forms[1];
        form.offsets.iter_mut().for_each(|o| *o = Vec2::new(20.0, 0.0));
        form.draw_order = 2.0;
        form.opacity = 0.5;
        let mut motion = Motion::new("wave", 1.0, 30.0);
        motion.looping = true;
        motion.track_mut(slide).set_key(0.0, 0.0);
        motion.track_mut(slide).set_key(1.0, 1.0);
        motion.track_mut(slide).keys[0].easing = aether_rig::Easing::Linear;
        motion.events.push(MotionEvent {
            time: 0.5,
            name: "half".into(),
        });
        rig.motions.push(motion);
        model
    }

    #[test]
    fn parameters_move_and_reorder_parts() {
        let mut player = Player::new(model()).unwrap();
        // Tree order: part 1 below part 0.
        let order: Vec<u32> = player.draw_list().iter().map(|d| d.part).collect();
        assert_eq!(order, vec![1, 0]);
        assert_eq!(&player.positions(1)[..2], &[5.0, 0.0]);

        let slide = player.parameter_index("Slide").unwrap();
        player.set_parameter(slide, 1.0);
        player.update();
        assert_eq!(&player.positions(1)[..2], &[25.0, 0.0]);
        let order: Vec<u32> = player.draw_list().iter().map(|d| d.part).collect();
        assert_eq!(order, vec![0, 1], "the keyed draw order lifts the slider");
        let slider = player.draw_list().iter().find(|d| d.part == 1).unwrap();
        assert!((slider.opacity - 0.5).abs() < 1e-6);

        player.reset();
        assert_eq!(player.parameter_value(slide), 0.0);
        assert_eq!(&player.positions(1)[..2], &[5.0, 0.0]);
    }

    #[test]
    fn motions_play_and_fire_their_events() {
        let mut player = Player::new(model()).unwrap();
        assert!(player.play_motion(0, false));
        assert!(!player.play_motion(9, false));
        let mut fired = Vec::new();
        for _ in 0..100 {
            player.tick(1.0 / 60.0);
            fired.extend(player.events().iter().map(|e| e.name.clone()));
        }
        // 1.67 s of a looping 1 s clip passes the half-way mark twice.
        assert_eq!(fired, vec!["half", "half"]);
        let slide = player.parameter_index("Slide").unwrap();
        let value = player.parameter_value(slide);
        assert!((value - 2.0 / 3.0).abs() < 0.02, "{value}");
        player.stop_motions();
        for _ in 0..60 {
            player.tick(1.0 / 60.0);
        }
        assert!(!player.is_playing());
    }

    #[test]
    fn hit_testing_finds_the_topmost_part() {
        let mut player = Player::new(model()).unwrap();
        // The overlap (5..10) belongs to part 0, which is drawn on top.
        assert_eq!(player.hit_test(7.0, 5.0), Some(0));
        assert_eq!(player.hit_test(12.0, 5.0), Some(1));
        assert_eq!(player.hit_test(40.0, 5.0), None);
        let slide = player.parameter_index("Slide").unwrap();
        player.set_parameter(slide, 1.0);
        player.update();
        assert_eq!(player.hit_test(28.0, 5.0), Some(1));
        let b = player.bounds();
        assert_eq!((b.min.x, b.max.x), (0.0, 35.0));
    }

    #[test]
    fn clipped_parts_carry_their_base() {
        let mut m = model();
        m.tree = vec![Node::Part {
            index: 0,
            part: 1,
            clipped: vec![0],
        }];
        let player = Player::new(m).unwrap();
        let list = player.draw_list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].mask_part(), Some(1));
        assert_eq!(list[1].mask_opacity, 1.0);
        // Outside the base, a clipped part is not hit.
        assert_eq!(player.hit_test(2.0, 5.0), None);
        assert_eq!(player.hit_test(7.0, 5.0), Some(0));
    }

    #[test]
    fn transforms_apply_after_deformation() {
        let mut m = model();
        m.parts[1].transform = Some(aether_core::math::Transform2D::translation(Vec2::new(0.0, 3.0)));
        let mut player = Player::new(m).unwrap();
        let slide = player.parameter_index("Slide").unwrap();
        player.set_parameter(slide, 1.0);
        player.update();
        assert_eq!(&player.positions(1)[..2], &[25.0, 3.0]);
    }

    #[test]
    fn input_switches_look_at_and_lip_sync_on() {
        let mut m = model();
        m.rig.behaviours.look.enabled = false;
        m.rig.behaviours.lip_sync.enabled = false;
        let mut player = Player::new(m).unwrap();
        player.set_audio(0.0, 0.0);
        assert!(
            !player.model().rig.behaviours.lip_sync.enabled,
            "silence changes nothing"
        );
        player.look_at(Some((0.5, f32::NAN)));
        assert!(
            !player.model().rig.behaviours.look.enabled,
            "a broken target is ignored"
        );
        player.look_at(Some((0.5, -2.0)));
        assert!(player.model().rig.behaviours.look.enabled);
        player.set_audio(0.8, 2.0);
        assert!(player.model().rig.behaviours.lip_sync.enabled);
    }
}
