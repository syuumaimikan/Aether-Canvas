//! Rig generators: the one-click steps that save hours of manual keying.
//!
//! * [`head_turn`] fills a warp deformer's keyforms for horizontal and
//!   vertical head angles by projecting its lattice onto an ellipsoid,
//!   rotating it in 3D and projecting back. Features near the middle of the
//!   face travel further than the silhouette, and the far side compresses —
//!   the parallax that sells a 2D head turn — for a 3×3 grid of keys that
//!   would otherwise be nine hand-shaped keyforms.
//! * [`wrap_in_warp`] puts meshes into a new warp deformer sized to them.
//! * [`standard_physics`] adds hair-sway groups wired to the standard
//!   parameters.
//! * [`mirror_keyform`] copies a keyform to the opposite key, mirrored — the
//!   other half of every symmetric pose.
//! * [`sway`] keys a pendulum bend (hair, ribbons, tails) and [`squash`] keys
//!   a collapse onto a line (eyelids, a closing mouth) — the two shapes most
//!   rigs need dozens of times.

use crate::deformer::{Deformer, DeformerKind, WarpForm};
use crate::keyform::{KeyAxis, KeyInterpolation, KeyformGrid};
use crate::physics::{InputKind, PhysicsGroup};
use crate::rig::{NodeRef, Rig, RigNode};
use aether_core::id::IdGenerator;
use aether_core::math::{Rect, Vec2};
use aether_core::{AetherError, DeformerId, LayerId, ParameterId, Result};
use serde::{Deserialize, Serialize};

/// Knobs for [`head_turn`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadTurnOptions {
    /// Visual yaw at the ends of the horizontal parameter, degrees.
    pub yaw: f32,
    /// Visual pitch at the ends of the vertical parameter, degrees.
    pub pitch: f32,
    /// How rounded the head is: 0 is a flat card, 1 a full dome.
    pub depth: f32,
    /// Foreshortening strength (0 = orthographic).
    pub perspective: f32,
    /// Centre of the head as a fraction of the lattice rectangle
    /// (`(0.5, 0.5)` is the middle).
    pub center: Vec2,
}

impl Default for HeadTurnOptions {
    fn default() -> Self {
        Self {
            yaw: 28.0,
            pitch: 18.0,
            depth: 0.75,
            perspective: 0.12,
            center: Vec2::new(0.5, 0.55),
        }
    }
}

/// Where a rest point of `rect` lands when the head is turned by `yaw` and
/// `pitch` (radians).
pub fn project_turn(p: Vec2, rect: Rect, yaw: f32, pitch: f32, options: &HeadTurnOptions) -> Vec2 {
    let size = rect.size();
    let center = Vec2::new(
        rect.min.x + size.x * options.center.x,
        rect.min.y + size.y * options.center.y,
    );
    let radius = Vec2::new((size.x * 0.5).max(1e-3), (size.y * 0.5).max(1e-3));
    let u = (p.x - center.x) / radius.x;
    let v = (p.y - center.y) / radius.y;
    // Depth profile (1 − r²)^1.5: rounded like a head in the middle, but with
    // zero slope at the rim. A true hemisphere (√(1 − r²)) has infinite slope
    // there, which stretches a thin band of the lattice so violently that
    // triangles cannot follow it and silhouettes turn jagged; this profile
    // keeps the warp monotonic (fold-free) up to about 40° of yaw.
    let z = options.depth.clamp(0.0, 2.0) * (1.0 - u * u - v * v).max(0.0).powf(1.5);
    let (sa, ca) = yaw.sin_cos();
    let (sb, cb) = pitch.sin_cos();
    let x1 = u * ca + z * sa;
    let z1 = -u * sa + z * ca;
    let y2 = v * cb - z1 * sb;
    let z2 = v * sb + z1 * cb;
    let scale = 1.0 + options.perspective * (z2 - z);
    Vec2::new(center.x + x1 * scale * radius.x, center.y + y2 * scale * radius.y)
}

/// Replace a warp deformer's keyforms with a generated head turn over
/// `x_param` (yaw) and `y_param` (pitch), keyed at each parameter's minimum,
/// default and maximum.
pub fn head_turn(
    rig: &mut Rig,
    deformer: DeformerId,
    x_param: ParameterId,
    y_param: Option<ParameterId>,
    options: &HeadTurnOptions,
) -> Result<()> {
    let px = rig
        .parameter(x_param)
        .cloned()
        .ok_or_else(|| AetherError::rig("the horizontal parameter does not exist"))?;
    let py = match y_param {
        Some(id) => Some(
            rig.parameter(id)
                .cloned()
                .ok_or_else(|| AetherError::rig("the vertical parameter does not exist"))?,
        ),
        None => None,
    };
    let d = rig
        .deformer_mut(deformer)
        .ok_or_else(|| AetherError::rig("no such deformer"))?;
    let DeformerKind::Warp(warp) = &mut d.kind else {
        return Err(AetherError::rig("a head turn needs a warp deformer"));
    };
    let keys = |p: &crate::param::Parameter| -> Vec<f32> {
        let mut k = vec![p.min, p.default, p.max];
        k.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        k
    };
    let x_keys = keys(&px);
    let y_keys = py.as_ref().map(keys).unwrap_or_else(|| vec![0.0]);
    let rest = warp.rest_points();
    let mut forms = Vec::with_capacity(x_keys.len() * y_keys.len());
    for &ky in &y_keys {
        let ny = py.as_ref().map(|p| p.normalized(ky)).unwrap_or(0.0);
        for &kx in &x_keys {
            let nx = px.normalized(kx);
            let yaw = (nx * options.yaw).to_radians();
            // Positive vertical values look up.
            let pitch = (ny * options.pitch).to_radians();
            let offsets = rest
                .iter()
                .map(|p| project_turn(*p, warp.rect, yaw, pitch, options) - *p)
                .collect();
            forms.push(WarpForm {
                offsets,
                opacity: 1.0,
            });
        }
    }
    let mut axes = vec![KeyAxis::new(x_param, x_keys)?];
    if let (Some(id), Some(_)) = (y_param, &py) {
        axes.push(KeyAxis::new(id, y_keys)?);
    }
    warp.keyforms = KeyformGrid {
        axes,
        forms,
        interpolation: KeyInterpolation::Smooth,
    };
    warp.smooth = true;
    Ok(())
}

/// Put `layers`' meshes into a new warp deformer that encloses them with
/// `padding` pixels to spare. The deformer takes the meshes' common parent.
pub fn wrap_in_warp(
    rig: &mut Rig,
    ids: &IdGenerator,
    layers: &[LayerId],
    name: impl Into<String>,
    cells: (usize, usize),
    padding: f32,
) -> Result<DeformerId> {
    let meshes: Vec<_> = layers.iter().filter_map(|l| rig.mesh(*l)).collect();
    if meshes.is_empty() {
        return Err(AetherError::rig("select at least one meshed layer"));
    }
    let mut bounds = meshes[0].bounds();
    for m in &meshes[1..] {
        bounds = bounds.union(&m.bounds());
    }
    let parent = meshes[0].parent;
    let common = meshes.iter().all(|m| m.parent == parent);
    let rect = bounds.expanded(padding.max(0.0));
    let id = ids.deformer();
    let mut deformer = Deformer::warp(id, name, rect, cells.0, cells.1);
    deformer.parent = if common { parent } else { None };
    rig.deformers.push(deformer);
    for layer in layers {
        if rig.mesh(*layer).is_some() {
            rig.set_parent(RigNode::Mesh(*layer), Some(NodeRef::Deformer(id)))?;
        }
    }
    Ok(id)
}

/// Add a rotation deformer at `origin` containing `children`.
pub fn wrap_in_rotation(
    rig: &mut Rig,
    ids: &IdGenerator,
    children: &[RigNode],
    name: impl Into<String>,
    origin: Vec2,
) -> Result<DeformerId> {
    let id = ids.deformer();
    let parent = children.first().and_then(|c| rig.parent_of(*c));
    let mut deformer = Deformer::rotation(id, name, origin);
    deformer.parent = parent.filter(|p| matches!(p, NodeRef::Deformer(_)));
    rig.deformers.push(deformer);
    for child in children {
        if matches!(child, RigNode::Bone(_)) {
            continue;
        }
        rig.set_parent(*child, Some(NodeRef::Deformer(id)))?;
    }
    Ok(id)
}

/// Add hair-sway physics for each of `HairFront`, `HairSide` and `HairBack`
/// that exists, driven by the head and body angles. Returns how many groups
/// were added.
pub fn standard_physics(rig: &mut Rig) -> usize {
    let find = |name: &str| rig.parameter_named(name).map(|p| p.id);
    let inputs: Vec<(ParameterId, InputKind, f32)> = [
        ("AngleX", InputKind::X, 0.6),
        ("AngleZ", InputKind::Angle, 0.6),
        ("BodyAngleX", InputKind::X, 0.4),
        ("BodyAngleZ", InputKind::Angle, 0.4),
    ]
    .iter()
    .filter_map(|(name, kind, w)| find(name).map(|id| (id, *kind, *w)))
    .collect();
    let outputs: Vec<(&str, Option<ParameterId>)> = ["HairFront", "HairSide", "HairBack"]
        .iter()
        .map(|n| (*n, find(n)))
        .collect();
    let mut added = 0;
    for ((name, length, stiffness), (_, output)) in [
        ("HairFront", 10.0, 30.0),
        ("HairSide", 14.0, 18.0),
        ("HairBack", 18.0, 10.0),
    ]
    .into_iter()
    .zip(outputs)
    {
        let Some(output) = output else { continue };
        if rig
            .physics
            .iter()
            .any(|g| g.outputs.iter().any(|o| o.param == output))
        {
            continue;
        }
        let mut group = PhysicsGroup::sway(name, &inputs, output);
        for p in &mut group.particles {
            p.length = length;
            p.stiffness = stiffness;
        }
        rig.physics.push(group);
        added += 1;
    }
    added
}

/// Copy the keyform at `from` (a key value of `param`) to the opposite key
/// (`2·default − from`), mirrored horizontally about `axis_x`.
///
/// Every vertex or lattice point takes the mirrored offset of the rest point
/// nearest its mirror image, so meshes need not be perfectly symmetric.
pub fn mirror_keyform(
    rig: &mut Rig,
    node: RigNode,
    param: ParameterId,
    from: f32,
    axis_x: f32,
) -> Result<()> {
    let p = rig
        .parameter(param)
        .cloned()
        .ok_or_else(|| AetherError::rig("no such parameter"))?;
    let to = p.clamp(2.0 * p.default - from);
    // Snapshot values and pose each key in turn to find both forms.
    let saved = rig.values.clone();
    rig.set_value(param, from);
    let from_index = rig.editable_form(node, None);
    rig.set_value(param, to);
    let to_index = rig.editable_form(node, None);
    rig.values = saved;
    let (from_index, to_index) = (from_index?, to_index?);
    if from_index == to_index {
        return Err(AetherError::rig("the key is its own mirror image"));
    }
    let mirror = |rest: &[Vec2], offsets: &[Vec2]| -> Vec<Vec2> {
        rest.iter()
            .map(|r| {
                let image = Vec2::new(2.0 * axis_x - r.x, r.y);
                let (j, _) = rest
                    .iter()
                    .enumerate()
                    .map(|(j, q)| (j, q.distance(image)))
                    .fold((0, f32::INFINITY), |best, c| if c.1 < best.1 { c } else { best });
                let o = offsets.get(j).copied().unwrap_or(Vec2::ZERO);
                Vec2::new(-o.x, o.y)
            })
            .collect()
    };
    match node {
        RigNode::Mesh(layer) => {
            let mesh = rig
                .mesh_mut(layer)
                .ok_or_else(|| AetherError::rig("no such mesh"))?;
            let mut form = mesh.keyforms.forms[from_index].clone();
            form.offsets = mirror(&mesh.vertices, &form.offsets);
            mesh.keyforms.forms[to_index] = form;
        }
        RigNode::Deformer(id) => {
            let d = rig
                .deformer_mut(id)
                .ok_or_else(|| AetherError::rig("no such deformer"))?;
            match &mut d.kind {
                DeformerKind::Warp(w) => {
                    let rest = w.rest_points();
                    let mut form = w.keyforms.forms[from_index].clone();
                    form.offsets = mirror(&rest, &form.offsets);
                    w.keyforms.forms[to_index] = form;
                }
                DeformerKind::Rotation(r) => {
                    let mut form = r.keyforms.forms[from_index].clone();
                    form.angle = -form.angle;
                    form.offset.x = -form.offset.x;
                    r.keyforms.forms[to_index] = form;
                }
            }
        }
        RigNode::Bone(id) => {
            let b = rig.bone_mut(id).ok_or_else(|| AetherError::rig("no such bone"))?;
            let mut form = b.keyforms.forms[from_index].clone();
            form.rotation = -form.rotation;
            form.translation.x = -form.translation.x;
            b.keyforms.forms[to_index] = form;
        }
    }
    Ok(())
}

/// Which edge of an object stays put when it sways.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anchor {
    /// Hangs from the top (hair, earrings).
    Top,
    /// Grows from the bottom (grass, a raised hand).
    Bottom,
    /// Attached on the left.
    Left,
    /// Attached on the right.
    Right,
}

/// Rest positions of the points a node's keyforms move.
fn rest_points_of(rig: &Rig, node: RigNode) -> Result<Vec<Vec2>> {
    match node {
        RigNode::Mesh(layer) => Ok(rig
            .mesh(layer)
            .ok_or_else(|| AetherError::rig("no such mesh"))?
            .vertices
            .clone()),
        RigNode::Deformer(id) => match &rig
            .deformer(id)
            .ok_or_else(|| AetherError::rig("no such deformer"))?
            .kind
        {
            DeformerKind::Warp(w) => Ok(w.rest_points()),
            DeformerKind::Rotation(_) => Err(AetherError::rig("choose a mesh or a warp deformer")),
        },
        RigNode::Bone(_) => Err(AetherError::rig("choose a mesh or a warp deformer")),
    }
}

/// Bind `param` (at `keys`, if not yet bound) and add `delta(key value,
/// rest points)` to the offsets of every keyform along that axis. Other axes
/// are untouched, so the new motion layers on top of existing poses.
fn add_along_axis(
    rig: &mut Rig,
    node: RigNode,
    param: ParameterId,
    keys: &[f32],
    delta: impl Fn(f32, &[Vec2]) -> Vec<Vec2>,
) -> Result<()> {
    let rest = rest_points_of(rig, node)?;
    let bound = rig
        .grid(node, None)
        .map(|g| g.axes().iter().any(|a| a.param == param));
    if bound != Some(true) {
        rig.bind_parameter(node, param, keys)?;
    }
    // The key value of `param`'s axis for form `index`.
    let adjust = |axes: &[KeyAxis], index: usize| -> Option<f32> {
        let axis_index = axes.iter().position(|a| a.param == param)?;
        let mut stride = 1;
        for a in &axes[..axis_index] {
            stride *= a.keys.len();
        }
        let k = (index / stride) % axes[axis_index].keys.len();
        Some(axes[axis_index].keys[k])
    };
    match node {
        RigNode::Mesh(layer) => {
            let mesh = rig
                .mesh_mut(layer)
                .ok_or_else(|| AetherError::rig("no such mesh"))?;
            let axes = mesh.keyforms.axes.clone();
            for (i, form) in mesh.keyforms.forms.iter_mut().enumerate() {
                if let Some(v) = adjust(&axes, i) {
                    for (o, d) in form.offsets.iter_mut().zip(delta(v, &rest)) {
                        *o += d;
                    }
                }
            }
        }
        RigNode::Deformer(id) => {
            let d = rig
                .deformer_mut(id)
                .ok_or_else(|| AetherError::rig("no such deformer"))?;
            if let DeformerKind::Warp(w) = &mut d.kind {
                let axes = w.keyforms.axes.clone();
                for (i, form) in w.keyforms.forms.iter_mut().enumerate() {
                    if let Some(v) = adjust(&axes, i) {
                        for (o, d) in form.offsets.iter_mut().zip(delta(v, &rest)) {
                            *o += d;
                        }
                    }
                }
            }
        }
        RigNode::Bone(_) => return Err(AetherError::rig("choose a mesh or a warp deformer")),
    }
    Ok(())
}

/// Key a pendulum sway on `param`: at the parameter's maximum, points far from
/// the `anchor` edge move `amount` pixels sideways (positive x for a top
/// anchor), easing in quadratically from the anchor; the minimum mirrors it.
pub fn sway(rig: &mut Rig, node: RigNode, param: ParameterId, amount: f32, anchor: Anchor) -> Result<()> {
    let p = rig
        .parameter(param)
        .cloned()
        .ok_or_else(|| AetherError::rig("no such parameter"))?;
    let keys = [p.min, p.default, p.max];
    add_along_axis(rig, node, param, &keys, |value, rest| {
        let n = p.normalized(value);
        let bounds = crate::mesh::bounds_of(rest);
        let (w, h) = (bounds.width().max(1e-3), bounds.height().max(1e-3));
        rest.iter()
            .map(|q| {
                let t = match anchor {
                    Anchor::Top => (q.y - bounds.min.y) / h,
                    Anchor::Bottom => (bounds.max.y - q.y) / h,
                    Anchor::Left => (q.x - bounds.min.x) / w,
                    Anchor::Right => (bounds.max.x - q.x) / w,
                };
                let t = t.clamp(0.0, 1.0);
                let swing = n * amount * t * t;
                // A pendulum rises a little as it swings.
                let lift = -n.abs() * amount.abs() * 0.12 * t * t * t;
                match anchor {
                    Anchor::Top => Vec2::new(swing, lift),
                    Anchor::Bottom => Vec2::new(swing, -lift),
                    Anchor::Left => Vec2::new(-lift, swing),
                    Anchor::Right => Vec2::new(lift, swing),
                }
            })
            .collect()
    })
}

/// Key a collapse on `param`: at the parameter's minimum every point moves
/// vertically onto the line at `line` (0 = top of the object, 1 = bottom);
/// at the maximum nothing moves. Eyelids and mouths close this way.
pub fn squash(rig: &mut Rig, node: RigNode, param: ParameterId, line: f32) -> Result<()> {
    let p = rig
        .parameter(param)
        .cloned()
        .ok_or_else(|| AetherError::rig("no such parameter"))?;
    let mut keys = vec![p.min, p.max];
    if p.default > p.min && p.default < p.max {
        keys.insert(1, p.default);
    }
    add_along_axis(rig, node, param, &keys, |value, rest| {
        let closure = ((p.max - value) / p.span()).clamp(0.0, 1.0);
        let bounds = crate::mesh::bounds_of(rest);
        let line_y = bounds.min.y + bounds.height() * line.clamp(0.0, 1.0);
        rest.iter()
            .map(|q| Vec2::new(0.0, (line_y - q.y) * closure))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::ArtMesh;
    use aether_core::math::vec2;

    fn face_rig() -> (Rig, IdGenerator, DeformerId) {
        let ids = IdGenerator::new();
        let mut rig = Rig::new();
        rig.add_standard_parameters(&ids);
        rig.set_mesh(ArtMesh::quad(
            LayerId(900),
            Rect::from_corners(vec2(20.0, 20.0), vec2(180.0, 180.0)),
        ));
        let d = wrap_in_warp(&mut rig, &ids, &[LayerId(900)], "Face", (6, 6), 10.0).expect("warp");
        (rig, ids, d)
    }

    fn param(rig: &Rig, name: &str) -> ParameterId {
        rig.parameter_named(name).expect("param").id
    }

    #[test]
    fn projection_is_the_identity_at_rest() {
        let rect = Rect::from_corners(vec2(0.0, 0.0), vec2(100.0, 100.0));
        let options = HeadTurnOptions::default();
        for p in [vec2(10.0, 10.0), vec2(50.0, 55.0), vec2(90.0, 40.0)] {
            assert!(project_turn(p, rect, 0.0, 0.0, &options).distance(p) < 1e-4);
        }
    }

    #[test]
    fn turning_moves_the_middle_more_than_the_edge() {
        let rect = Rect::from_corners(vec2(0.0, 0.0), vec2(100.0, 100.0));
        let options = HeadTurnOptions {
            center: vec2(0.5, 0.5),
            ..Default::default()
        };
        let yaw = 0.4;
        let middle = project_turn(vec2(50.0, 50.0), rect, yaw, 0.0, &options) - vec2(50.0, 50.0);
        let edge = project_turn(vec2(99.0, 50.0), rect, yaw, 0.0, &options) - vec2(99.0, 50.0);
        assert!(
            middle.x > 10.0,
            "the face centre swings toward the turn: {middle:?}"
        );
        assert!(
            middle.x > edge.x.abs() * 2.0,
            "the silhouette barely moves: {edge:?}"
        );
        let up = project_turn(vec2(50.0, 50.0), rect, 0.0, 0.3, &options);
        assert!(up.y < 45.0, "looking up raises the centre: {up:?}");
    }

    #[test]
    fn head_turn_fills_a_three_by_three_grid() {
        let (mut rig, _, d) = face_rig();
        let x = param(&rig, "AngleX");
        let y = param(&rig, "AngleY");
        head_turn(&mut rig, d, x, Some(y), &HeadTurnOptions::default()).expect("generate");
        rig.validate().expect("valid");
        let Some(Deformer {
            kind: DeformerKind::Warp(w),
            ..
        }) = rig.deformer(d)
        else {
            panic!("warp expected");
        };
        assert_eq!(w.keyforms.forms.len(), 9);
        let centre_form = &w.keyforms.forms[4];
        assert!(
            centre_form.offsets.iter().all(|o| o.length() < 1e-4),
            "rest stays rest"
        );
        // The whole mesh follows: turning right moves the mesh centre right.
        rig.set_value(x, 30.0);
        let pose = rig.evaluate();
        let positions = &pose.meshes[&LayerId(900)].positions;
        let before = rig.mesh(LayerId(900)).expect("mesh").vertices.clone();
        let moved: f32 = positions.iter().zip(&before).map(|(a, b)| a.x - b.x).sum::<f32>();
        assert!(moved.abs() > 0.1);
    }

    #[test]
    fn wrapping_reparents_and_encloses() {
        let (rig, _, d) = face_rig();
        assert_eq!(
            rig.mesh(LayerId(900)).and_then(|m| m.parent),
            Some(NodeRef::Deformer(d))
        );
        let Some(Deformer {
            kind: DeformerKind::Warp(w),
            ..
        }) = rig.deformer(d)
        else {
            panic!("warp expected");
        };
        assert!(w.rect.min.x <= 10.0 && w.rect.max.x >= 190.0);
    }

    #[test]
    fn standard_physics_uses_the_standard_parameters() {
        let (mut rig, _, _) = face_rig();
        assert_eq!(standard_physics(&mut rig), 3);
        assert_eq!(standard_physics(&mut rig), 0, "not added twice");
        assert!(rig.physics.iter().all(|g| g.inputs.len() == 4));
    }

    #[test]
    fn mirroring_copies_the_opposite_pose() {
        let (mut rig, _, _) = face_rig();
        let x = param(&rig, "AngleX");
        let node = RigNode::Mesh(LayerId(900));
        rig.bind_parameter(node, x, &[-30.0, 0.0, 30.0]).expect("bind");
        rig.set_value(x, 30.0);
        {
            let form = rig.mesh_form_mut(LayerId(900), None).expect("form");
            // Push the right-hand vertices (1 and 2) further right.
            form.offsets[1] = vec2(8.0, 1.0);
            form.offsets[2] = vec2(8.0, -1.0);
        }
        mirror_keyform(&mut rig, node, x, 30.0, 100.0).expect("mirror");
        rig.set_value(x, -30.0);
        let form = rig.mesh_form_mut(LayerId(900), None).expect("form").clone();
        // Left-hand vertices (0 and 3) now move left.
        assert_eq!(form.offsets[0], vec2(-8.0, 1.0));
        assert_eq!(form.offsets[3], vec2(-8.0, -1.0));
    }

    #[test]
    fn sway_bends_the_far_end_and_leaves_the_anchor() {
        let (mut rig, _, _) = face_rig();
        let hair = param(&rig, "HairFront");
        let node = RigNode::Mesh(LayerId(900));
        sway(&mut rig, node, hair, 20.0, Anchor::Top).expect("sway");
        rig.validate().expect("valid");
        rig.set_value(hair, 1.0);
        let pose = rig.evaluate();
        let p = &pose.meshes[&LayerId(900)].positions;
        let rest = rig.mesh(LayerId(900)).expect("mesh").vertices.clone();
        // Quad vertices: 0,1 on top; 2,3 on the bottom.
        assert!((p[0].x - rest[0].x).abs() < 1e-3, "the anchored top stays");
        assert!(
            (p[2].x - rest[2].x - 20.0).abs() < 0.5,
            "the bottom swings: {:?}",
            p[2]
        );
        rig.set_value(hair, -1.0);
        let pose = rig.evaluate();
        assert!(pose.meshes[&LayerId(900)].positions[2].x < rest[2].x - 19.0);
    }

    #[test]
    fn squash_closes_onto_a_line_and_layers_on_existing_keys() {
        let (mut rig, _, _) = face_rig();
        let eye = param(&rig, "EyeLOpen");
        let x = param(&rig, "AngleX");
        let node = RigNode::Mesh(LayerId(900));
        rig.bind_parameter(node, x, &[-30.0, 0.0, 30.0]).expect("bind");
        squash(&mut rig, node, eye, 0.5).expect("squash");
        rig.validate().expect("valid");
        rig.set_value(eye, 0.0);
        let pose = rig.evaluate();
        let p = &pose.meshes[&LayerId(900)].positions;
        assert!(
            p.iter().all(|v| (v.y - 100.0).abs() < 1e-3),
            "closed onto y = 100: {p:?}"
        );
        rig.set_value(eye, 1.0);
        assert!(rig.evaluate().meshes[&LayerId(900)].rest, "open is untouched");
    }

    #[test]
    fn generated_turns_never_fold_the_lattice() {
        // Sample a dense row through the middle and require x to keep
        // increasing: a fold would show as x going backwards.
        let rect = Rect::from_corners(vec2(0.0, 0.0), vec2(200.0, 200.0));
        let options = HeadTurnOptions {
            center: vec2(0.5, 0.5),
            depth: 0.75,
            ..Default::default()
        };
        for yaw_deg in [-38.0f32, -20.0, 20.0, 38.0] {
            for row in [60.0, 100.0, 140.0] {
                let mut last = f32::NEG_INFINITY;
                for i in 0..=400 {
                    let x = i as f32 * 0.5;
                    let p = project_turn(vec2(x, row), rect, yaw_deg.to_radians(), 0.0, &options);
                    assert!(p.x > last, "fold at x = {x}, yaw {yaw_deg}");
                    last = p.x;
                }
            }
        }
    }
}
