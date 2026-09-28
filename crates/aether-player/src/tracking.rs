//! Face tracking: a tracker's view of a face, turned into parameter values.
//!
//! Any face tracker that reports head angles and the usual 52 blend shapes
//! (ARKit on iPhones, MediaPipe in browsers, most VTuber tracking apps)
//! drives a model through one [`FaceFrame`]. The mapping onto the standard
//! parameters lives here, once, so the web player, native hosts and the
//! editor all move a model the same way.
//!
//! ## Conventions
//!
//! Frames describe the **tracked person**, the way trackers report them:
//! `yaw > 0` turns toward the person's own left, `pitch > 0` looks up,
//! `roll > 0` tips the top of the head toward their left shoulder, and a
//! shape ending in `Left` belongs to their left side.
//!
//! Models use **screen** directions: `AngleX > 0` turns toward the right of
//! the screen, `AngleY > 0` looks up, `AngleZ > 0` tilts clockwise, and `L`
//! parameters belong to the part on the left of the screen.
//!
//! With [`TrackingSettings::mirror`] on (the default), the model behaves like
//! a mirror image of the person, which is what people expect when they see
//! themselves: turn left and the model turns to the left of the screen.

use aether_core::ParameterId;
use aether_rig::{Parameter, Rig};

/// Blend shape names in the order [`FaceFrame::shapes`] stores them: ARKit's
/// 52, which MediaPipe also reports (all but `tongueOut`).
pub const BLENDSHAPES: [&str; 52] = [
    "browDownLeft",
    "browDownRight",
    "browInnerUp",
    "browOuterUpLeft",
    "browOuterUpRight",
    "cheekPuff",
    "cheekSquintLeft",
    "cheekSquintRight",
    "eyeBlinkLeft",
    "eyeBlinkRight",
    "eyeLookDownLeft",
    "eyeLookDownRight",
    "eyeLookInLeft",
    "eyeLookInRight",
    "eyeLookOutLeft",
    "eyeLookOutRight",
    "eyeLookUpLeft",
    "eyeLookUpRight",
    "eyeSquintLeft",
    "eyeSquintRight",
    "eyeWideLeft",
    "eyeWideRight",
    "jawForward",
    "jawLeft",
    "jawOpen",
    "jawRight",
    "mouthClose",
    "mouthDimpleLeft",
    "mouthDimpleRight",
    "mouthFrownLeft",
    "mouthFrownRight",
    "mouthFunnel",
    "mouthLeft",
    "mouthLowerDownLeft",
    "mouthLowerDownRight",
    "mouthPressLeft",
    "mouthPressRight",
    "mouthPucker",
    "mouthRight",
    "mouthRollLower",
    "mouthRollUpper",
    "mouthShrugLower",
    "mouthShrugUpper",
    "mouthSmileLeft",
    "mouthSmileRight",
    "mouthStretchLeft",
    "mouthStretchRight",
    "mouthUpperUpLeft",
    "mouthUpperUpRight",
    "noseSneerLeft",
    "noseSneerRight",
    "tongueOut",
];

/// Index of a blend shape in [`BLENDSHAPES`].
pub fn blendshape_index(name: &str) -> Option<usize> {
    BLENDSHAPES.iter().position(|n| *n == name)
}

/// One tracker sample, in the tracked person's frame of reference (see the
/// module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct FaceFrame {
    /// Degrees; positive turns toward the person's left.
    pub yaw: f32,
    /// Degrees; positive looks up.
    pub pitch: f32,
    /// Degrees; positive tips the head toward the person's left shoulder.
    pub roll: f32,
    /// Blend shape weights `0..=1`, in [`BLENDSHAPES`] order.
    pub shapes: [f32; 52],
}

impl Default for FaceFrame {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            shapes: [0.0; 52],
        }
    }
}

impl FaceFrame {
    /// A blend shape's weight by name (0 for unknown names).
    pub fn shape(&self, name: &str) -> f32 {
        blendshape_index(name).map(|i| self.shapes[i]).unwrap_or(0.0)
    }

    /// Set a blend shape by name; unknown names are ignored.
    pub fn set_shape(&mut self, name: &str, weight: f32) {
        if let Some(i) = blendshape_index(name) {
            self.shapes[i] = weight;
        }
    }

    /// The frame with non-finite values replaced by zero and weights clamped.
    fn sanitized(&self) -> Self {
        let f = |v: f32| if v.is_finite() { v } else { 0.0 };
        let mut out = self.clone();
        out.yaw = f(self.yaw);
        out.pitch = f(self.pitch);
        out.roll = f(self.roll);
        for s in &mut out.shapes {
            *s = f(*s).clamp(0.0, 1.0);
        }
        out
    }
}

/// How tracking maps onto the model.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackingSettings {
    /// Move like a mirror image of the person (see the module docs).
    pub mirror: bool,
    /// Time constant of the smoothing, seconds. Hides tracker jitter.
    pub smoothing: f32,
    /// Multiplies head angles.
    pub head_gain: f32,
    /// How much of the head's turn the body follows, when no driver in the
    /// rig already makes it follow.
    pub body_follow: f32,
    /// Multiplies mouth opening.
    pub mouth_gain: f32,
}

impl Default for TrackingSettings {
    fn default() -> Self {
        Self {
            mirror: true,
            smoothing: 0.08,
            head_gain: 1.0,
            body_follow: 0.3,
            mouth_gain: 1.0,
        }
    }
}

/// Target values for the standard parameters the rig has, from one frame
/// measured against a `neutral` (calibration) frame.
pub fn face_targets(
    rig: &Rig,
    frame: &FaceFrame,
    neutral: &FaceFrame,
    settings: &TrackingSettings,
) -> Vec<(ParameterId, f32)> {
    let frame = frame.sanitized();
    let neutral = neutral.sanitized();
    let s = |name: &str| frame.shape(name);
    // Relative to the resting face, so a person whose eyes read half shut at
    // rest still opens the model's eyes fully.
    let rel = |name: &str| (frame.shape(name) - neutral.shape(name)).max(0.0);
    // Which way the person's left is on screen: left in a mirror, right when
    // the model faces them like another person would.
    let person_left: f32 = if settings.mirror { -1.0 } else { 1.0 };
    // So the shapes of the part on the screen's left are the person's left
    // ones in a mirror, and their right ones otherwise.
    let (screen_left, screen_right) = if settings.mirror {
        ("Left", "Right")
    } else {
        ("Right", "Left")
    };
    let side = |base: &str, screen: &str| format!("{base}{screen}");

    let yaw = (frame.yaw - neutral.yaw) * settings.head_gain;
    let pitch = (frame.pitch - neutral.pitch) * settings.head_gain;
    let roll = (frame.roll - neutral.roll) * settings.head_gain;

    let eye_open = |screen: &str| {
        let blink = rel(&side("eyeBlink", screen));
        let wide = rel(&side("eyeWide", screen));
        // Blink weights rarely reach 1 even with the eye shut.
        (1.0 - blink / 0.6).clamp(0.0, 1.0) + wide * 0.5
    };
    let eye_smile =
        |screen: &str| ((s(&side("cheekSquint", screen)) + s(&side("eyeSquint", screen))) * 0.6).min(1.0);
    let brow = |screen: &str| {
        let up = rel(&side("browOuterUp", screen)) + rel("browInnerUp") * 0.5;
        let down = rel(&side("browDown", screen));
        ((up - down) * 1.5).clamp(-1.0, 1.0)
    };
    // Gaze toward the person's left: their left eye looks out, their right
    // eye looks in.
    let gaze_left =
        (s("eyeLookOutLeft") + s("eyeLookInRight") - s("eyeLookInLeft") - s("eyeLookOutRight")) * 0.5;
    let gaze_up =
        (s("eyeLookUpLeft") + s("eyeLookUpRight") - s("eyeLookDownLeft") - s("eyeLookDownRight")) * 0.5;
    let mouth_open = ((rel("jawOpen") - s("mouthClose") * 0.5) * 2.0 * settings.mouth_gain).clamp(0.0, 1.0);
    let smile = (s("mouthSmileLeft") + s("mouthSmileRight")) * 0.5;
    let frown = (s("mouthFrownLeft") + s("mouthFrownRight")) * 0.5;
    let mouth_form = ((smile - frown - s("mouthPucker") * 0.5) * 1.5).clamp(-1.0, 1.0);

    // AngleX turns toward screen-right: a turn to the person's left heads
    // toward `person_left`. AngleZ tilts clockwise, which moves the top of
    // the head toward screen-right: a tip toward the person's left shoulder
    // moves it toward `person_left`.
    let angle_x = yaw * person_left;
    let angle_z = roll * person_left;

    let mut out = Vec::new();
    let mut put = |name: &str, value: f32| {
        if let Some(p) = rig.parameter_named(name) {
            out.push((p.id, p.clamp(value)));
        }
    };
    put("AngleX", angle_x);
    put("AngleY", pitch);
    put("AngleZ", angle_z);
    put("EyeBallX", gaze_left * person_left * 1.5);
    put("EyeBallY", gaze_up * 1.5);
    put("EyeLOpen", eye_open(screen_left));
    put("EyeROpen", eye_open(screen_right));
    put("EyeLSmile", eye_smile(screen_left));
    put("EyeRSmile", eye_smile(screen_right));
    put("BrowLY", brow(screen_left));
    put("BrowRY", brow(screen_right));
    put("MouthOpenY", mouth_open);
    put("MouthForm", mouth_form);

    // The body follows the head a little, unless the rig already does that
    // with a driver.
    let driven = |name: &str| {
        rig.parameter_named(name)
            .is_some_and(|p| rig.drivers.iter().any(|d| d.enabled && d.target == p.id))
    };
    let follow = settings.body_follow;
    for (body, head) in [
        ("BodyAngleX", angle_x),
        ("BodyAngleY", pitch),
        ("BodyAngleZ", angle_z),
    ] {
        if follow > 0.0 && !driven(body) {
            if let Some(p) = rig.parameter_named(body) {
                // Scale by the ranges, so ±30° of head gives ±10 of body.
                let head_range = rig
                    .parameter_named("AngleX")
                    .map(|h| h.max)
                    .unwrap_or(30.0)
                    .max(1e-3);
                out.push((p.id, p.clamp(head / head_range * p.max * follow * 3.0)));
            }
        }
    }
    out
}

/// Tracking state kept by a player.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tracking {
    pub settings: TrackingSettings,
    pub neutral: FaceFrame,
    pub latest: Option<FaceFrame>,
    pub targets: Vec<(ParameterId, f32)>,
    pub current: Vec<(ParameterId, f32)>,
    /// Whether auto-blink was on before tracking took over the eyes.
    pub blink_was_enabled: Option<bool>,
}

impl Tracking {
    /// Ease the current values toward the targets over `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        let k = if self.settings.smoothing > 0.0 {
            1.0 - (-dt / self.settings.smoothing).exp()
        } else {
            1.0
        };
        for (id, target) in &self.targets {
            match self.current.iter_mut().find(|(c, _)| c == id) {
                Some((_, value)) => *value += (*target - *value) * k,
                // A parameter seen for the first time starts on target.
                None => self.current.push((*id, *target)),
            }
        }
        self.current
            .retain(|(id, _)| self.targets.iter().any(|(t, _)| t == id));
    }
}

/// The standard parameters tracking can drive, for hosts that want to show
/// which of a model's parameters are live.
pub fn tracked_parameters(parameters: &[Parameter]) -> Vec<ParameterId> {
    const NAMES: [&str; 16] = [
        "AngleX",
        "AngleY",
        "AngleZ",
        "EyeBallX",
        "EyeBallY",
        "EyeLOpen",
        "EyeROpen",
        "EyeLSmile",
        "EyeRSmile",
        "BrowLY",
        "BrowRY",
        "MouthOpenY",
        "MouthForm",
        "BodyAngleX",
        "BodyAngleY",
        "BodyAngleZ",
    ];
    parameters
        .iter()
        .filter(|p| NAMES.contains(&p.name.as_str()))
        .map(|p| p.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlendKind, Model, Node, Part, Texture};
    use crate::Player;
    use aether_core::color::Rgba8;
    use aether_core::id::IdGenerator;
    use aether_core::math::{IRect, Vec2};
    use aether_core::LayerId;
    use aether_raster::Pixmap;
    use aether_rig::automesh::{auto_mesh, AutoMeshOptions};
    use aether_rig::autorig::{auto_rig, PartInfo};
    use aether_rig::ArtMesh;

    /// A face painted with the usual layer names, auto-rigged, as a player.
    /// "L" parts are on the left of the screen, as in the editor.
    fn face() -> Player {
        let (w, h) = (200u32, 240u32);
        let layers: [(&str, IRect); 8] = [
            ("Face", IRect::new(50, 40, 100, 130)),
            ("Eye L", IRect::new(70, 95, 22, 12)),
            ("Eye R", IRect::new(108, 95, 22, 12)),
            ("Brow L", IRect::new(68, 80, 26, 4)),
            ("Brow R", IRect::new(106, 80, 26, 4)),
            ("Mouth", IRect::new(86, 140, 28, 6)),
            ("Front hair", IRect::new(46, 30, 108, 34)),
            ("Body", IRect::new(40, 170, 120, 70)),
        ];
        let ids = IdGenerator::new();
        let mut model = Model::new("face", w, h);
        model.textures.push(Texture {
            file: "t.png".into(),
            width: 1,
            height: 1,
        });
        let mut parts = Vec::new();
        for (i, (name, rect)) in layers.iter().enumerate() {
            let layer = LayerId(i as u64 + 1);
            let mut pixels = Pixmap::new(w, h);
            pixels.fill_rect(*rect, Rgba8::new(200, 150, 120, 255));
            let generated = auto_mesh(&pixels, &AutoMeshOptions::for_bounds(*rect, 1.0)).expect("mesh");
            let mesh = ArtMesh::new(layer, generated.vertices.clone(), generated.triangles.clone());
            parts.push(PartInfo {
                layer,
                name: name.to_string(),
                groups: Vec::new(),
                bounds: mesh.bounds(),
            });
            model.rig.set_mesh(mesh);
            model.parts.push(Part {
                layer,
                name: name.to_string(),
                texture: 0,
                uvs: vec![Vec2::ZERO; generated.vertices.len()],
                vertices: generated.vertices,
                triangles: generated.triangles,
                opacity: 1.0,
                blend: BlendKind::Normal,
                transform: None,
            });
            model.tree.push(Node::Part {
                index: i as u32,
                part: i as u32,
                clipped: vec![],
            });
        }
        auto_rig(&mut model.rig, &ids, &parts).expect("auto rig");
        // Start from a still model: no idle motion, no breathing.
        model.rig.behaviours.breath.clear();
        Player::new(model).expect("player")
    }

    fn centroid(player: &Player, name: &str) -> Vec2 {
        let part = player.model().parts.iter().position(|p| p.name == name).unwrap();
        let xy = player.positions(part);
        let n = xy.len() as f32 / 2.0;
        let sum = xy
            .chunks_exact(2)
            .fold(Vec2::ZERO, |a, c| a + Vec2::new(c[0], c[1]));
        Vec2::new(sum.x / n, sum.y / n)
    }

    fn height(player: &Player, name: &str) -> f32 {
        let part = player.model().parts.iter().position(|p| p.name == name).unwrap();
        let ys: Vec<f32> = player.positions(part).chunks_exact(2).map(|c| c[1]).collect();
        ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min)
    }

    fn value(player: &Player, name: &str) -> f32 {
        player.parameter_value(player.parameter_index(name).unwrap())
    }

    /// Track `frame` long enough for the smoothing to settle.
    fn settle(player: &mut Player, frame: &FaceFrame) {
        for _ in 0..60 {
            player.track_face(frame);
            player.tick(1.0 / 60.0);
        }
    }

    #[test]
    fn turning_left_turns_the_mirror_image_left() {
        let mut player = face();
        let rest = centroid(&player, "Eye L");
        settle(
            &mut player,
            &FaceFrame {
                yaw: 20.0,
                ..Default::default()
            },
        );
        assert!(
            (value(&player, "AngleX") + 20.0).abs() < 0.5,
            "{}",
            value(&player, "AngleX")
        );
        assert!(
            centroid(&player, "Eye L").x < rest.x - 2.0,
            "features move to the left of the screen"
        );

        player.tracking_settings_mut().mirror = false;
        settle(
            &mut player,
            &FaceFrame {
                yaw: 20.0,
                ..Default::default()
            },
        );
        assert!(
            centroid(&player, "Eye L").x > rest.x + 2.0,
            "without a mirror they move right"
        );
    }

    #[test]
    fn tipping_toward_the_left_shoulder_tilts_the_mirror_image_counter_clockwise() {
        let mut player = face();
        let (left, right) = (centroid(&player, "Eye L"), centroid(&player, "Eye R"));
        settle(
            &mut player,
            &FaceFrame {
                roll: 15.0,
                ..Default::default()
            },
        );
        // Counter-clockwise on screen (y down): the left eye drops, the right
        // eye rises.
        let (l, r) = (centroid(&player, "Eye L"), centroid(&player, "Eye R"));
        assert!(
            l.y - left.y > 1.0 && r.y - right.y < -1.0,
            "{left:?}->{l:?}, {right:?}->{r:?}"
        );
        assert!(value(&player, "AngleZ") < -10.0);
    }

    #[test]
    fn looking_up_raises_the_face() {
        let mut player = face();
        let rest = centroid(&player, "Eye L");
        settle(
            &mut player,
            &FaceFrame {
                pitch: 20.0,
                ..Default::default()
            },
        );
        assert!(centroid(&player, "Eye L").y < rest.y - 1.0);
    }

    #[test]
    fn a_wink_closes_the_matching_eye_on_screen() {
        let mut player = face();
        let open = height(&player, "Eye L");
        let mut frame = FaceFrame::default();
        frame.set_shape("eyeBlinkLeft", 0.9);
        settle(&mut player, &frame);
        // In a mirror the person's left eye is the model's screen-left eye.
        assert!(value(&player, "EyeLOpen") < 0.05);
        assert!((value(&player, "EyeROpen") - 1.0).abs() < 1e-3);
        assert!(
            height(&player, "Eye L") < open * 0.5,
            "{} vs {open}",
            height(&player, "Eye L")
        );

        player.tracking_settings_mut().mirror = false;
        settle(&mut player, &frame);
        assert!(value(&player, "EyeROpen") < 0.05);
        assert!(value(&player, "EyeLOpen") > 0.95);
    }

    #[test]
    fn mouth_brows_and_gaze_follow_their_shapes() {
        let mut player = face();
        let mut frame = FaceFrame::default();
        frame.set_shape("jawOpen", 0.45);
        frame.set_shape("mouthSmileLeft", 0.6);
        frame.set_shape("mouthSmileRight", 0.6);
        frame.set_shape("browInnerUp", 0.4);
        frame.set_shape("browOuterUpLeft", 0.5);
        frame.set_shape("browOuterUpRight", 0.5);
        // Looking toward the person's left: their left eye looks out, the
        // right one in. A mirror image looks toward the screen's left.
        frame.set_shape("eyeLookOutLeft", 0.6);
        frame.set_shape("eyeLookInRight", 0.6);
        settle(&mut player, &frame);
        assert!(value(&player, "MouthOpenY") > 0.8);
        assert!(value(&player, "MouthForm") > 0.5);
        assert!(value(&player, "BrowLY") > 0.5 && value(&player, "BrowRY") > 0.5);
        assert!(
            value(&player, "EyeBallX") < -0.5,
            "{}",
            value(&player, "EyeBallX")
        );
    }

    #[test]
    fn calibration_makes_the_current_face_neutral() {
        let mut player = face();
        let mut resting = FaceFrame {
            yaw: 8.0,
            pitch: -12.0,
            roll: 3.0,
            ..Default::default()
        };
        resting.set_shape("eyeBlinkLeft", 0.3);
        resting.set_shape("eyeBlinkRight", 0.3);
        resting.set_shape("jawOpen", 0.1);
        player.track_face(&resting);
        player.calibrate_tracking();
        settle(&mut player, &resting);
        for name in ["AngleX", "AngleY", "AngleZ", "MouthOpenY"] {
            assert!(
                value(&player, name).abs() < 1e-3,
                "{name} = {}",
                value(&player, name)
            );
        }
        assert!((value(&player, "EyeLOpen") - 1.0).abs() < 1e-3);
    }

    #[test]
    fn tracking_is_smoothed_and_hands_back_cleanly() {
        let mut player = face();
        let blink_on = player.model().rig.behaviours.blink.enabled;
        assert!(blink_on, "auto rig switches blinking on");
        player.track_face(&FaceFrame::default());
        player.tick(1.0 / 60.0);
        assert!(
            !player.model().rig.behaviours.blink.enabled,
            "auto-blink pauses while tracking"
        );

        // A sudden turn is eased in, not jumped to.
        player.track_face(&FaceFrame {
            yaw: -30.0,
            ..Default::default()
        });
        player.tick(1.0 / 60.0);
        let first = value(&player, "AngleX");
        assert!(first > 1.0 && first < 29.0, "{first}");

        player.stop_tracking();
        player.tick(1.0 / 60.0);
        assert!(!player.is_tracking());
        assert!(player.model().rig.behaviours.blink.enabled, "auto-blink resumes");
        assert!(value(&player, "AngleX").abs() < 1e-3);
    }

    #[test]
    fn the_body_follows_the_head_unless_a_driver_already_does() {
        let mut player = face();
        let driven = player.model().rig.parameter_named("BodyAngleX").is_some_and(|p| {
            player
                .model()
                .rig
                .drivers
                .iter()
                .any(|d| d.enabled && d.target == p.id)
        });
        settle(
            &mut player,
            &FaceFrame {
                yaw: -30.0,
                ..Default::default()
            },
        );
        let body = value(&player, "BodyAngleX");
        // AngleX is +30 in a mirror; the body follows by a third of its range
        // either way, through the rig's driver or through tracking.
        assert!(body > 2.0, "body {body}, driven {driven}");
    }

    #[test]
    fn broken_samples_are_harmless() {
        let mut player = face();
        let mut frame = FaceFrame {
            yaw: f32::NAN,
            pitch: f32::INFINITY,
            roll: 1e9,
            ..Default::default()
        };
        frame.set_shape("jawOpen", f32::NAN);
        frame.set_shape("notAShape", 1.0);
        settle(&mut player, &frame);
        for p in player.parameters() {
            let v = player.parameter_value(player.parameter_index(&p.name).unwrap());
            assert!(v.is_finite() && v >= p.min && v <= p.max, "{} = {v}", p.name);
        }
        assert_eq!(blendshape_index("tongueOut"), Some(51));
        assert_eq!(BLENDSHAPES.len(), 52);
    }
}
