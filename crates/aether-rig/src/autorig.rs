//! One-click rigging from layer names.
//!
//! Artwork arrives split into named parts — "face", "eye white L", "瞳",
//! "前髪" — and those names already say what each part should do. The
//! auto-rigger reads them (English or Japanese, with left/right from the name
//! or, failing that, from position) and builds a complete, working rig:
//!
//! * a **Body** warp and a **Body tilt** pivot, a **Neck** pivot and a
//!   **Head** warp, with the parts sorted into them;
//! * a generated **3D head turn** (AngleX/AngleY) and body turn;
//! * **eyes** that close (whites and irises squash, lashes drop into a
//!   closed-eye line) and **irises** that look around;
//! * **brows** that raise and lower, a **mouth** that opens and smiles,
//!   **blush** that fades in;
//! * **hair and accessories** that sway, driven by pendulum **physics**;
//! * **breathing**, **auto-blink**, **look-at**, a body-follows-head
//!   **driver**, and an **Idle** motion to play straight away.
//!
//! Everything it makes is ordinary rig data, so the result is a starting
//! point to refine by hand, not a black box.

use crate::behaviour::Behaviours;
use crate::deformer::DeformerKind;
use crate::driver::Driver;
use crate::generate::{self, Anchor, HeadTurnOptions};
use crate::keyform::KeyformGrid;
use crate::mesh::MeshForm;
use crate::motion::{Easing, Motion};
use crate::rig::{NodeRef, Rig, RigNode};
use aether_core::id::IdGenerator;
use aether_core::math::{Rect, Vec2};
use aether_core::{AetherError, LayerId, ParameterId, Result};

/// What a part is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// Face skin / head outline.
    Face,
    /// The white of an eye.
    EyeWhite,
    /// Iris, pupil or eye highlight.
    Iris,
    /// Eyelash or eyelid line.
    Lash,
    /// Eyebrow.
    Brow,
    /// The inside of an open mouth (tongue, teeth, cavity).
    MouthOpen,
    /// The mouth line or lips.
    Mouth,
    /// Blush.
    Cheek,
    /// Nose.
    Nose,
    /// Ear.
    Ear,
    /// Bangs.
    HairFront,
    /// Side locks.
    HairSide,
    /// Back hair, ponytails.
    HairBack,
    /// Other hair.
    Hair,
    /// Ribbons, earrings, tails: things that dangle.
    Accessory,
    /// Neck.
    Neck,
    /// Torso and clothes.
    Body,
    /// Arms and hands.
    Arm,
    /// Not recognised.
    Unknown,
}

impl Role {
    /// True for parts that belong to the head.
    pub fn is_head(self) -> bool {
        matches!(
            self,
            Role::Face
                | Role::EyeWhite
                | Role::Iris
                | Role::Lash
                | Role::Brow
                | Role::MouthOpen
                | Role::Mouth
                | Role::Cheek
                | Role::Nose
                | Role::Ear
                | Role::HairFront
                | Role::HairSide
                | Role::HairBack
                | Role::Hair
        )
    }
}

/// Which side of the face a part is on (the character's left is `Left`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Left.
    Left,
    /// Right.
    Right,
    /// Neither, or not said.
    Unknown,
}

/// One meshed layer offered to the auto-rigger.
#[derive(Clone, Debug, PartialEq)]
pub struct PartInfo {
    /// The layer.
    pub layer: LayerId,
    /// Its name.
    pub name: String,
    /// Names of the folders it sits in, outermost first.
    pub groups: Vec<String>,
    /// Rest bounds of its mesh.
    pub bounds: Rect,
}

fn has(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

/// Words split on anything that is not a letter or digit, lowercased.
fn tokens(name: &str) -> Vec<String> {
    // Split camel case too: "EyeL" → "eye", "l".
    let mut spaced = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_uppercase() && previous_lower {
            spaced.push(' ');
        }
        previous_lower = c.is_lowercase();
        spaced.push(c);
    }
    spaced
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// Left/right as the name states it.
pub fn side_of(name: &str) -> Side {
    if name.contains('左') {
        return Side::Left;
    }
    if name.contains('右') {
        return Side::Right;
    }
    let tokens = tokens(name);
    if tokens.iter().any(|t| t == "l" || t == "left" || t == "lt") {
        return Side::Left;
    }
    if tokens.iter().any(|t| t == "r" || t == "right" || t == "rt") {
        return Side::Right;
    }
    Side::Unknown
}

/// Decide a part's role from its name and the folders around it.
pub fn classify(name: &str, groups: &[String]) -> Role {
    let n = name.to_lowercase();
    let context = format!("{} {}", groups.join(" ").to_lowercase(), n);
    let eye_context = has(&context, &["eye", "目", "め"]);
    let mouth_context = has(&context, &["mouth", "口", "くち", "lip", "唇"]);
    let hair_context = has(&context, &["hair", "髪", "かみ"]);

    if has(
        &n,
        &[
            "lash",
            "eyelid",
            "まつ",
            "睫",
            "まぶた",
            "瞼",
            "アイライン",
            "eyeline",
            "eye line",
            "lid",
        ],
    ) {
        return Role::Lash;
    }
    if has(&n, &["iris", "pupil", "瞳", "黒目", "虹彩"])
        || (eye_context && has(&n, &["highlight", "ハイライト", "hilight"]))
    {
        return Role::Iris;
    }
    if has(&n, &["sclera", "白目", "しろめ"]) || (eye_context && has(&n, &["white", "白", "ball", "base"]))
    {
        return Role::EyeWhite;
    }
    if has(&n, &["brow", "眉", "まゆ"]) {
        return Role::Brow;
    }
    if mouth_context
        && has(
            &n,
            &[
                "open", "開", "inside", "inner", "中", "舌", "tongue", "teeth", "歯", "cavity",
            ],
        )
    {
        return Role::MouthOpen;
    }
    if has(&n, &["mouth", "lip", "口", "唇", "くち"]) {
        return Role::Mouth;
    }
    if has(&n, &["cheek", "blush", "頬", "ほお", "チーク", "赤面"]) {
        return Role::Cheek;
    }
    if has(&n, &["nose", "鼻"]) {
        return Role::Nose;
    }
    // "tail" as a whole word only: a ponytail is hair, not an accessory.
    let tail = tokens(name).iter().any(|t| t == "tail" || t == "tails");
    if tail
        || has(
            &n,
            &[
                "ribbon",
                "リボン",
                "earring",
                "イヤリング",
                "ピアス",
                "しっぽ",
                "尻尾",
                "tie",
                "ネクタイ",
                "charm",
                "accessory",
                "アクセ",
            ],
        )
    {
        return Role::Accessory;
    }
    let ear_token = tokens(name).iter().any(|t| t == "ear" || t == "ears");
    if ear_token || n.contains('耳') {
        return Role::Ear;
    }
    if has(&n, &["bang", "fringe", "前髪"]) || (hair_context && has(&n, &["front", "前"])) {
        return Role::HairFront;
    }
    if has(&n, &["横髪", "サイド", "もみあげ", "sidelock", "side lock"])
        || (hair_context && has(&n, &["side", "横"]))
    {
        return Role::HairSide;
    }
    if has(
        &n,
        &[
            "後ろ髪",
            "後髪",
            "うしろ髪",
            "ponytail",
            "ポニー",
            "ツイン",
            "twintail",
            "twin tail",
        ],
    ) || (hair_context && has(&n, &["back", "後", "うしろ"]))
    {
        return Role::HairBack;
    }
    if has(&n, &["hair", "髪", "アホ毛", "ahoge", "かみ"]) {
        return Role::Hair;
    }
    if has(&n, &["neck", "首"]) {
        return Role::Neck;
    }
    if has(&n, &["arm", "腕", "hand", "手"]) {
        return Role::Arm;
    }
    if has(&n, &["face", "顔", "skin", "肌", "輪郭", "head", "頭"]) {
        return Role::Face;
    }
    if has(
        &n,
        &[
            "body",
            "体",
            "胴",
            "torso",
            "cloth",
            "服",
            "shirt",
            "シャツ",
            "dress",
            "制服",
            "uniform",
            "chest",
            "胸",
            "shoulder",
            "肩",
            "coat",
            "jacket",
        ],
    ) {
        return Role::Body;
    }
    Role::Unknown
}

/// What the auto-rigger did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoRigReport {
    /// Parts per role.
    pub roles: Vec<(Role, usize)>,
    /// Names of parts that were not recognised (placed in the body or head
    /// by folder, with no motion of their own).
    pub unrecognised: Vec<String>,
    /// Deformers created.
    pub deformers: usize,
    /// Physics groups added.
    pub physics: usize,
}

fn union(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    rects.into_iter().reduce(|a, b| a.union(&b))
}

fn param(rig: &Rig, name: &str) -> Result<ParameterId> {
    rig.parameter_named(name)
        .map(|p| p.id)
        .ok_or_else(|| AetherError::rig(format!("the standard parameter {name} is missing")))
}

/// Set offsets of every keyform at key index `key` of `param`'s axis on a
/// mesh (adding to what is there).
fn add_at_key(rig: &mut Rig, layer: LayerId, param: ParameterId, key: usize, offsets: impl Fn(Vec2) -> Vec2) {
    let Some(mesh) = rig.mesh_mut(layer) else { return };
    let axes = mesh.keyforms.axes.clone();
    let Some(a) = axes.iter().position(|x| x.param == param) else {
        return;
    };
    let stride: usize = axes[..a].iter().map(|x| x.keys.len()).product();
    let count = axes[a].keys.len();
    let rest = mesh.vertices.clone();
    for (i, form) in mesh.keyforms.forms.iter_mut().enumerate() {
        if (i / stride) % count == key {
            for (o, v) in form.offsets.iter_mut().zip(&rest) {
                *o += offsets(*v);
            }
        }
    }
}

/// Build a complete rig from `parts`. The rig's existing deformers, bones,
/// physics and mesh keyforms are replaced; meshes and parameters are kept.
pub fn auto_rig(rig: &mut Rig, ids: &IdGenerator, parts: &[PartInfo]) -> Result<AutoRigReport> {
    let mut report = AutoRigReport::default();
    rig.add_standard_parameters(ids);

    // Start clean: the result should not depend on what was there.
    rig.deformers.clear();
    rig.bones.clear();
    rig.physics.clear();
    for part in parts {
        if let Some(mesh) = rig.mesh_mut(part.layer) {
            let n = mesh.vertex_count();
            mesh.parent = None;
            mesh.keyforms = KeyformGrid::constant(MeshForm::rest(n));
            mesh.blend_shapes.clear();
            mesh.skin = None;
        }
    }
    let parts: Vec<&PartInfo> = parts.iter().filter(|p| rig.mesh(p.layer).is_some()).collect();

    // Classify, letting folders decide for unrecognised parts.
    let mut roles: Vec<(Role, &PartInfo)> = Vec::new();
    for part in &parts {
        let mut role = classify(&part.name, &part.groups);
        if role == Role::Unknown {
            let folder = part.groups.join(" ").to_lowercase();
            if has(&folder, &["head", "face", "頭", "顔"]) {
                role = Role::Face;
            }
            report.unrecognised.push(part.name.clone());
        }
        roles.push((role, part));
    }
    let head_parts: Vec<&PartInfo> = roles
        .iter()
        .filter(|(r, _)| r.is_head())
        .map(|(_, p)| *p)
        .collect();
    if head_parts.is_empty() {
        return Err(AetherError::rig(
            "no face parts were recognised: name layers like face, eye, mouth, hair (or 顔, 目, 口, 髪)",
        ));
    }
    let face_bounds = union(
        roles
            .iter()
            .filter(|(r, _)| *r == Role::Face)
            .map(|(_, p)| p.bounds),
    )
    .or_else(|| union(head_parts.iter().map(|p| p.bounds)))
    .ok_or_else(|| AetherError::rig("the face has no size"))?;
    let face_center = face_bounds.center();
    let face_w = face_bounds.width().max(1.0);
    let face_h = face_bounds.height().max(1.0);

    // Accessories near the head swing with it; others hang off the body.
    let head_region = union(head_parts.iter().map(|p| p.bounds)).unwrap_or(face_bounds);
    let in_head = |role: Role, part: &PartInfo| {
        role.is_head() || (role == Role::Accessory && head_region.contains(part.bounds.center()))
    };
    let head_layers: Vec<LayerId> = roles
        .iter()
        .filter(|(r, p)| in_head(*r, p))
        .map(|(_, p)| p.layer)
        .collect();
    let body_layers: Vec<LayerId> = roles
        .iter()
        .filter(|(r, p)| !in_head(*r, p))
        .map(|(_, p)| p.layer)
        .collect();

    // Hierarchy: [Body tilt ⊃ Body ⊃] Neck ⊃ Head ⊃ head parts.
    let head = generate::wrap_in_warp(rig, ids, &head_layers, "Head", (8, 8), 16.0)?;
    let neck_pivot = Vec2::new(face_center.x, face_bounds.max.y);
    let neck = generate::wrap_in_rotation(rig, ids, &[RigNode::Deformer(head)], "Neck", neck_pivot)?;
    report.deformers += 2;
    let head_rect = match rig.deformer(head).map(|d| &d.kind) {
        Some(DeformerKind::Warp(w)) => w.rect,
        _ => face_bounds,
    };
    let turn = HeadTurnOptions {
        center: Vec2::new(
            ((face_center.x - head_rect.min.x) / head_rect.width().max(1.0)).clamp(0.2, 0.8),
            ((face_center.y - head_rect.min.y) / head_rect.height().max(1.0)).clamp(0.2, 0.8),
        ),
        ..Default::default()
    };
    generate::head_turn(
        rig,
        head,
        param(rig, "AngleX")?,
        Some(param(rig, "AngleY")?),
        &turn,
    )?;
    let angle_z = param(rig, "AngleZ")?;
    rig.bind_parameter(RigNode::Deformer(neck), angle_z, &[-30.0, 0.0, 30.0])?;
    set_rotation_keys(rig, neck, [-12.0, 0.0, 12.0]);

    if !body_layers.is_empty() {
        let body = generate::wrap_in_warp(rig, ids, &body_layers, "Body", (6, 6), 16.0)?;
        rig.set_parent(RigNode::Deformer(neck), Some(NodeRef::Deformer(body)))?;
        let body_bounds = union(
            roles
                .iter()
                .filter(|(r, p)| !in_head(*r, p))
                .map(|(_, p)| p.bounds),
        )
        .unwrap_or(face_bounds);
        let tilt_pivot = Vec2::new(body_bounds.center().x, body_bounds.max.y);
        let tilt = generate::wrap_in_rotation(rig, ids, &[RigNode::Deformer(body)], "Body tilt", tilt_pivot)?;
        report.deformers += 2;
        generate::head_turn(
            rig,
            body,
            param(rig, "BodyAngleX")?,
            None,
            &HeadTurnOptions {
                yaw: 14.0,
                depth: 0.45,
                center: Vec2::new(0.5, 0.35),
                ..Default::default()
            },
        )?;
        let body_z = param(rig, "BodyAngleZ")?;
        rig.bind_parameter(RigNode::Deformer(tilt), body_z, &[-10.0, 0.0, 10.0])?;
        set_rotation_keys(rig, tilt, [-4.0, 0.0, 4.0]);

        // Breathing lifts the shoulders of the torso parts.
        let breath = param(rig, "Breath")?;
        for (role, part) in &roles {
            if *role != Role::Body {
                continue;
            }
            rig.bind_parameter(RigNode::Mesh(part.layer), breath, &[0.0, 1.0])?;
            let top = body_bounds.min.y;
            let height = body_bounds.height().max(1.0);
            let lift = (face_h * 0.02).max(2.0);
            add_at_key(rig, part.layer, breath, 1, |v| {
                let t = ((v.y - top) / height).clamp(0.0, 1.0);
                Vec2::new(0.0, -lift * (1.0 - t).powi(2))
            });
        }
    }

    // Eyes, per side.
    let side_for = |part: &PartInfo| match side_of(&part.name) {
        Side::Unknown => {
            let group_side = part
                .groups
                .iter()
                .rev()
                .map(|g| side_of(g))
                .find(|s| *s != Side::Unknown);
            group_side.unwrap_or(if part.bounds.center().x < face_center.x {
                Side::Left
            } else {
                Side::Right
            })
        }
        s => s,
    };
    for side in [Side::Left, Side::Right] {
        let (open_name, brow_name) = match side {
            Side::Left => ("EyeLOpen", "BrowLY"),
            _ => ("EyeROpen", "BrowRY"),
        };
        let open = param(rig, open_name)?;
        let of = |role: Role| -> Vec<&PartInfo> {
            roles
                .iter()
                .filter(|(r, p)| *r == role && side_for(p) == side)
                .map(|(_, p)| *p)
                .collect()
        };
        let whites = of(Role::EyeWhite);
        let irises = of(Role::Iris);
        let lashes = of(Role::Lash);
        let eye = union(whites.iter().map(|p| p.bounds))
            .or_else(|| union(irises.iter().map(|p| p.bounds)))
            .or_else(|| union(lashes.iter().map(|p| p.bounds)));
        if let Some(eye) = eye {
            let line = eye.min.y + eye.height() * 0.66;
            for part in &whites {
                generate::squash(rig, RigNode::Mesh(part.layer), open, 0.66)?;
            }
            for part in &irises {
                let local = ((line - part.bounds.min.y) / part.bounds.height().max(1.0)).clamp(0.0, 1.0);
                generate::squash(rig, RigNode::Mesh(part.layer), open, local)?;
                // Look around.
                for (name, shift) in [
                    ("EyeBallX", Vec2::new(eye.width() * 0.16, 0.0)),
                    ("EyeBallY", Vec2::new(0.0, -eye.height() * 0.18)),
                ] {
                    let id = param(rig, name)?;
                    rig.bind_parameter(RigNode::Mesh(part.layer), id, &[-1.0, 0.0, 1.0])?;
                    add_at_key(rig, part.layer, id, 0, |_| shift * -1.0);
                    add_at_key(rig, part.layer, id, 2, |_| shift);
                }
            }
            for part in &lashes {
                rig.bind_parameter(RigNode::Mesh(part.layer), open, &[0.0, 1.0])?;
                let bottom = part.bounds.max.y;
                let drop = line - bottom + part.bounds.height() * 0.5;
                add_at_key(rig, part.layer, open, 0, |v| {
                    Vec2::new(0.0, drop + (bottom - v.y) * 0.4)
                });
            }
        }
        let brow = param(rig, brow_name)?;
        for (_, part) in roles
            .iter()
            .filter(|(r, p)| *r == Role::Brow && side_for(p) == side)
        {
            rig.bind_parameter(RigNode::Mesh(part.layer), brow, &[-1.0, 0.0, 1.0])?;
            let travel = (face_h * 0.03).max(2.0);
            add_at_key(rig, part.layer, brow, 0, |_| Vec2::new(0.0, travel * 0.7));
            add_at_key(rig, part.layer, brow, 2, |_| Vec2::new(0.0, -travel));
        }
    }

    // Mouth.
    let mouth_open = param(rig, "MouthOpenY")?;
    let mouth_form = param(rig, "MouthForm")?;
    let inner: Vec<&PartInfo> = roles
        .iter()
        .filter(|(r, _)| *r == Role::MouthOpen)
        .map(|(_, p)| *p)
        .collect();
    let lines: Vec<&PartInfo> = roles
        .iter()
        .filter(|(r, _)| *r == Role::Mouth)
        .map(|(_, p)| *p)
        .collect();
    for part in &inner {
        generate::squash(rig, RigNode::Mesh(part.layer), mouth_open, 0.3)?;
    }
    for part in &lines {
        rig.bind_parameter(RigNode::Mesh(part.layer), mouth_open, &[0.0, 1.0])?;
        let top = part.bounds.min.y;
        if inner.is_empty() {
            // A single closed mouth drawing opens by stretching downwards.
            add_at_key(rig, part.layer, mouth_open, 1, |v| {
                Vec2::new(0.0, (v.y - top) * 1.4)
            });
        } else {
            add_at_key(rig, part.layer, mouth_open, 1, |_| {
                Vec2::new(0.0, -face_h * 0.008)
            });
        }
        rig.bind_parameter(RigNode::Mesh(part.layer), mouth_form, &[-1.0, 0.0, 1.0])?;
        let (cx, hw) = (part.bounds.center().x, (part.bounds.width() * 0.5).max(1.0));
        let lift = (part.bounds.width() * 0.12).max(1.5);
        let curve = move |v: Vec2| ((v.x - cx) / hw).clamp(-1.0, 1.0).powi(2);
        add_at_key(rig, part.layer, mouth_form, 2, move |v| {
            Vec2::new(0.0, -lift * curve(v))
        });
        add_at_key(rig, part.layer, mouth_form, 0, move |v| {
            Vec2::new(0.0, lift * 0.8 * curve(v))
        });
    }

    // Blush.
    let cheek = param(rig, "Cheek")?;
    for (_, part) in roles.iter().filter(|(r, _)| *r == Role::Cheek) {
        rig.bind_parameter(RigNode::Mesh(part.layer), cheek, &[0.0, 1.0])?;
        if let Some(mesh) = rig.mesh_mut(part.layer) {
            if let Some(a) = mesh.keyforms.axes.iter().position(|x| x.param == cheek) {
                let stride: usize = mesh.keyforms.axes[..a].iter().map(|x| x.keys.len()).product();
                for (i, form) in mesh.keyforms.forms.iter_mut().enumerate() {
                    if (i / stride) % 2 == 0 {
                        form.opacity = 0.0;
                    }
                }
            }
        }
    }

    // Hair and accessories sway.
    for (role, part) in &roles {
        let (name, amount) = match role {
            Role::HairFront | Role::Hair => ("HairFront", 0.07),
            Role::HairSide => ("HairSide", 0.12),
            Role::HairBack => ("HairBack", 0.09),
            Role::Accessory => ("HairSide", 0.08),
            _ => continue,
        };
        let id = param(rig, name)?;
        generate::sway(rig, RigNode::Mesh(part.layer), id, face_w * amount, Anchor::Top)?;
    }
    generate::standard_physics(rig);
    // Keep only the chains whose parameter actually moves something.
    let used: Vec<ParameterId> = rig
        .physics
        .iter()
        .flat_map(|g| g.outputs.iter().map(|o| o.param))
        .filter(|p| !rig.nodes_using(*p).is_empty())
        .collect();
    rig.physics
        .retain(|g| g.outputs.iter().any(|o| used.contains(&o.param)));
    report.physics = rig.physics.len();

    // Behaviours, a body-follows-head driver and an idle loop.
    rig.behaviours = Behaviours::standard(&rig.parameters);
    let body_x = param(rig, "BodyAngleX")?;
    if !rig.drivers.iter().any(|d| d.target == body_x) {
        rig.drivers.push(Driver::new(body_x, "self + AngleX * 0.3"));
    }
    if !rig.motions.iter().any(|m| m.name == "Idle") {
        let mut idle = Motion::new("Idle", 4.0, 30.0);
        for (name, values) in [
            ("AngleX", [0.0, 6.0, -4.0, 0.0]),
            ("AngleY", [0.0, -3.0, 2.0, 0.0]),
            ("AngleZ", [0.0, 3.0, -2.0, 0.0]),
        ] {
            let id = param(rig, name)?;
            let track = idle.track_mut(id);
            for (t, v) in [0.0, 1.2, 2.6, 4.0].into_iter().zip(values) {
                let i = track.set_key(t, v);
                track.keys[i].easing = Easing::EaseInOut;
            }
        }
        rig.motions.push(idle);
    }

    let mut counts: std::collections::BTreeMap<Role, usize> = Default::default();
    for (role, _) in &roles {
        *counts.entry(*role).or_default() += 1;
    }
    report.roles = counts.into_iter().collect();
    rig.validate()?;
    Ok(report)
}

fn set_rotation_keys(rig: &mut Rig, id: aether_core::DeformerId, angles: [f32; 3]) {
    if let Some(DeformerKind::Rotation(r)) = rig.deformer_mut(id).map(|d| &mut d.kind) {
        for (form, angle) in r.keyforms.forms.iter_mut().zip(angles) {
            form.angle = angle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::ArtMesh;
    use aether_core::math::vec2;

    #[test]
    fn english_and_japanese_names_classify() {
        let none: Vec<String> = Vec::new();
        let eyes = vec!["Eyes".to_string()];
        for (name, groups, role) in [
            ("face", &none, Role::Face),
            ("顔", &none, Role::Face),
            ("白目 左", &none, Role::EyeWhite),
            ("white", &eyes, Role::EyeWhite),
            ("瞳R", &none, Role::Iris),
            ("Iris L", &none, Role::Iris),
            ("まつ毛", &none, Role::Lash),
            ("Eyelash_R", &none, Role::Lash),
            ("眉毛", &none, Role::Brow),
            ("mouth", &none, Role::Mouth),
            ("口 開き", &none, Role::MouthOpen),
            ("頬", &none, Role::Cheek),
            ("前髪", &none, Role::HairFront),
            ("hair front", &none, Role::HairFront),
            ("横髪 右", &none, Role::HairSide),
            ("後ろ髪", &none, Role::HairBack),
            ("ponytail", &none, Role::HairBack),
            ("アホ毛", &none, Role::Hair),
            ("リボン", &none, Role::Accessory),
            ("earring", &none, Role::Accessory),
            ("ear L", &none, Role::Ear),
            ("首", &none, Role::Neck),
            ("体", &none, Role::Body),
            ("shirt", &none, Role::Body),
            ("レイヤー 12", &none, Role::Unknown),
        ] {
            assert_eq!(classify(name, groups), role, "{name}");
        }
    }

    #[test]
    fn sides_come_from_the_name() {
        assert_eq!(side_of("白目 左"), Side::Left);
        assert_eq!(side_of("Eye_R"), Side::Right);
        assert_eq!(side_of("EyeL"), Side::Left);
        assert_eq!(side_of("eye left"), Side::Left);
        assert_eq!(
            side_of("Lash"),
            Side::Unknown,
            "a word starting with L is not a side"
        );
        assert_eq!(side_of("mouth"), Side::Unknown);
    }

    fn part(rig: &mut Rig, id: u64, name: &str, rect: Rect) -> PartInfo {
        let layer = LayerId(id);
        rig.set_mesh(ArtMesh::quad(layer, rect));
        PartInfo {
            layer,
            name: name.into(),
            groups: Vec::new(),
            bounds: rect,
        }
    }

    fn r(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_corners(vec2(x0, y0), vec2(x1, y1))
    }

    fn character() -> (Rig, IdGenerator, Vec<PartInfo>) {
        let ids = IdGenerator::new();
        ids.reserve_at_least(1000);
        let mut rig = Rig::new();
        let parts = vec![
            part(&mut rig, 1, "体", r(100.0, 400.0, 400.0, 640.0)),
            part(&mut rig, 2, "顔", r(150.0, 130.0, 360.0, 390.0)),
            part(&mut rig, 3, "白目 左", r(180.0, 240.0, 240.0, 285.0)),
            part(&mut rig, 4, "瞳 左", r(195.0, 245.0, 225.0, 285.0)),
            part(&mut rig, 5, "まつ毛 左", r(175.0, 230.0, 245.0, 255.0)),
            part(&mut rig, 6, "白目 右", r(272.0, 240.0, 332.0, 285.0)),
            part(&mut rig, 7, "瞳 右", r(287.0, 245.0, 317.0, 285.0)),
            part(&mut rig, 8, "まつ毛 右", r(267.0, 230.0, 337.0, 255.0)),
            part(&mut rig, 9, "眉 左", r(180.0, 210.0, 235.0, 225.0)),
            part(&mut rig, 10, "眉 右", r(277.0, 210.0, 332.0, 225.0)),
            part(&mut rig, 11, "口", r(236.0, 320.0, 276.0, 340.0)),
            part(&mut rig, 12, "前髪", r(130.0, 100.0, 380.0, 260.0)),
            part(&mut rig, 13, "後ろ髪", r(110.0, 110.0, 400.0, 480.0)),
            part(&mut rig, 14, "頬", r(180.0, 295.0, 330.0, 315.0)),
            part(&mut rig, 15, "謎の模様", r(10.0, 10.0, 20.0, 20.0)),
        ];
        (rig, ids, parts)
    }

    fn set(rig: &mut Rig, name: &str, value: f32) {
        let id = rig.parameter_named(name).expect("param").id;
        rig.set_value(id, value);
    }

    #[test]
    fn a_named_character_is_rigged_in_one_call() {
        let (mut rig, ids, parts) = character();
        let report = auto_rig(&mut rig, &ids, &parts).expect("auto rig");
        rig.validate().expect("valid");
        assert_eq!(report.unrecognised, vec!["謎の模様".to_string()]);
        assert_eq!(report.deformers, 4);
        assert_eq!(report.physics, 2, "front and back hair chains");
        assert!(rig.motions.iter().any(|m| m.name == "Idle"));
        assert!(rig.behaviours.blink.enabled);

        let rest = rig.evaluate();
        // Everything draws as painted at rest — except blush, which is keyed
        // to be invisible until Cheek rises.
        assert!(
            rest.meshes
                .iter()
                .all(|(layer, m)| m.rest || *layer == LayerId(14)),
            "the rest pose is untouched"
        );

        // Closing the left eye moves only the left eye parts.
        set(&mut rig, "EyeLOpen", 0.0);
        let closed = rig.evaluate();
        assert!(!closed.meshes[&LayerId(3)].rest);
        assert!(closed.meshes[&LayerId(6)].rest, "the right eye stays open");
        let lash = &closed.meshes[&LayerId(5)].positions;
        assert!(
            lash.iter().all(|p| p.y > 255.0),
            "the lash drops to the closed line"
        );
        set(&mut rig, "EyeLOpen", 1.0);

        // Turning the head moves face parts but not the body.
        set(&mut rig, "AngleX", 30.0);
        let turned = rig.evaluate();
        assert!(!turned.meshes[&LayerId(2)].rest);
        assert!(
            turned.meshes[&LayerId(1)].rest,
            "the body does not turn with the head"
        );
        set(&mut rig, "AngleX", 0.0);

        // Mouth, brows, blush and hair all respond.
        for (name, value, layer) in [
            ("MouthOpenY", 1.0, 11),
            ("MouthForm", 1.0, 11),
            ("BrowLY", 1.0, 9),
            ("HairFront", 1.0, 12),
            ("HairBack", -1.0, 13),
            ("Breath", 1.0, 1),
        ] {
            set(&mut rig, name, value);
            assert!(
                !rig.evaluate().meshes[&LayerId(layer)].rest,
                "{name} moved nothing"
            );
            rig.reset_values();
        }
        set(&mut rig, "Cheek", 0.0);
        assert_eq!(
            rig.evaluate().meshes[&LayerId(14)].opacity,
            0.0,
            "blush is hidden at rest"
        );
    }

    #[test]
    fn running_it_twice_rebuilds_rather_than_stacks() {
        let (mut rig, ids, parts) = character();
        auto_rig(&mut rig, &ids, &parts).expect("first");
        let first = rig.deformers.len();
        auto_rig(&mut rig, &ids, &parts).expect("second");
        assert_eq!(rig.deformers.len(), first);
        assert_eq!(rig.physics.len(), 2);
        assert_eq!(rig.motions.len(), 1);
    }

    #[test]
    fn a_rig_without_face_parts_is_refused_helpfully() {
        let ids = IdGenerator::new();
        let mut rig = Rig::new();
        let parts = vec![part(&mut rig, 1, "Layer 1", r(0.0, 0.0, 10.0, 10.0))];
        let err = auto_rig(&mut rig, &ids, &parts).expect_err("nothing to rig");
        assert!(err.to_string().contains("face"), "{err}");
    }
}
