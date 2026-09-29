//! Rig generators: the one-click steps that save hours of manual keying.
//!
//! * [`head_turn`] fills a warp deformer's keyforms for horizontal and
//!   vertical head angles from a [`HeadShape`]: a skull whose front is the
//!   face. Each lattice point is given the depth of the head there, turned
//!   in 3D and put back on the picture, so the middle of the face travels
//!   further than the cheeks, the far side compresses, the chin comes along
//!   with the features and nodding pivots on the neck — for a 3×3 grid of
//!   keys that would otherwise be nine hand-shaped keyforms. [`key_turn`]
//!   does the same for any rule, which is how parts that stand in front of
//!   the face or behind the head get their parallax.
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
#[serde(default)]
pub struct HeadTurnOptions {
    /// Visual yaw at the ends of the horizontal parameter, degrees.
    pub yaw: f32,
    /// Visual pitch at the ends of the vertical parameter, degrees.
    pub pitch: f32,
    /// How far the face stands out of the head's outline, as a fraction of
    /// its half-width: 0 is a flat card.
    pub depth: f32,
    /// Foreshortening strength (0 = orthographic).
    pub perspective: f32,
    /// Where the eyes are, as a fraction of the lattice rectangle, when no
    /// [`face`](Self::face) is given (`(0.5, 0.5)` is the middle).
    pub center: Vec2,
    /// The face, cheek to cheek and hairline to chin, when it is known. The
    /// lattice may reach well beyond it (hair, ears).
    pub face: Option<Rect>,
}

impl Default for HeadTurnOptions {
    fn default() -> Self {
        Self {
            yaw: 30.0,
            pitch: 20.0,
            depth: 0.75,
            perspective: 0.12,
            center: Vec2::new(0.5, 0.55),
            face: None,
        }
    }
}

impl HeadTurnOptions {
    /// The head these options describe, for a lattice over `rect`: the
    /// given face, or one guessed from the rectangle and
    /// [`center`](Self::center).
    pub fn shape(&self, rect: Rect) -> HeadShape {
        let face = self.face.unwrap_or_else(|| {
            let size = rect.size();
            let eyes = Vec2::new(
                rect.min.x + size.x * self.center.x.clamp(0.0, 1.0),
                rect.min.y + size.y * self.center.y.clamp(0.0, 1.0),
            );
            let half = Vec2::new(size.x * 0.4, size.y * 0.31);
            Rect::from_corners(eyes - half, eyes + half)
        });
        HeadShape {
            face,
            crown: self
                .face
                .map(|f| f.min.y - 0.3 * f.height())
                .unwrap_or(rect.min.y + 0.05 * rect.height()),
            depth: self.depth,
            perspective: self.perspective,
        }
    }
}

/// A head, as [`HeadShape::turn`] moves it: a skull whose front is the face.
///
/// The face stands out of the plane of the head's outline by
/// [`depth`](Self::depth) × its half-width at the middle, curving back to
/// nothing at the cheeks' edges. It keeps most of that depth down to the
/// chin, which juts forward of the neck, and above the hairline the skull
/// curves back to the crown. The head yaws about the vertical line through
/// the middle of the face and nods about the top of the neck, below and
/// behind the eyes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadShape {
    /// The face: cheek to cheek, hairline to chin.
    pub face: Rect,
    /// Height of the top of the skull.
    pub crown: f32,
    /// How far the middle of the face stands out of the head's outline, as
    /// a fraction of the face's half-width.
    pub depth: f32,
    /// Foreshortening strength (0 = orthographic).
    pub perspective: f32,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl HeadShape {
    /// A head around `face` (cheek to cheek, hairline to chin), with the
    /// crown a little above the hairline.
    pub fn around(face: Rect) -> Self {
        let options = HeadTurnOptions {
            face: Some(face),
            ..Default::default()
        };
        options.shape(face)
    }

    fn half_width(&self) -> f32 {
        (self.face.width() * 0.5).max(1.0)
    }

    fn height(&self) -> f32 {
        self.face.height().max(1.0)
    }

    /// Where the eyes are: the centre of the face.
    pub fn centre(&self) -> Vec2 {
        Vec2::new(self.face.center().x, self.face.min.y + 0.5 * self.height())
    }

    /// How far the head's surface at `p` stands toward the viewer, in
    /// pixels, from the plane of its outline.
    pub fn surface(&self, p: Vec2) -> f32 {
        let rx = self.half_width();
        let h = self.height();
        let u = (p.x - self.face.center().x) / (rx * 1.08);
        // (1 − u²)^1.5 across: round in the middle, with zero slope at the
        // edge, which keeps the lattice from folding up to about 50° of yaw.
        let across = (1.0 - u * u).max(0.0).powf(1.5);
        let top = self.crown.min(self.face.min.y - 1.0);
        let down = smoothstep(top, self.face.min.y + 0.35 * h, p.y)
            * (1.0 - smoothstep(self.face.max.y - 0.15 * h, self.face.max.y + 0.3 * h, p.y));
        self.depth.max(0.0) * rx * across * down
    }

    /// How firmly `p` goes with the head: 1 on it, fading to 0 a little
    /// over a face's width away (the ends of long hair stay where they are).
    pub fn attachment(&self, p: Vec2) -> f32 {
        let rx = self.half_width();
        let h = self.height();
        let c = self.face.center().x;
        let outside = Vec2::new(
            ((p.x - c).abs() - rx * 1.1).max(0.0),
            (self.crown - p.y).max(0.0).max(p.y - (self.face.max.y + 0.3 * h)),
        )
        .length();
        1.0 - smoothstep(0.0, 2.4 * rx, outside)
    }

    /// Where the point at rest position `p`, standing `z` pixels toward the
    /// viewer from the plane of the head's outline, goes when the head turns
    /// by `yaw` (positive: the face moves right) and `pitch` (positive:
    /// looking up), in radians.
    pub fn turn(&self, p: Vec2, z: f32, yaw: f32, pitch: f32) -> Vec2 {
        let rx = self.half_width();
        let h = self.height();
        let cx = self.face.center().x;
        // The top of the neck, which the head nods on.
        let pivot_y = self.face.min.y + 0.7 * h;
        let pivot_z = -0.15 * rx;
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let x = p.x - cx;
        let y = p.y - pivot_y;
        // Yaw shears by depth, so the outline stays where it is drawn: its
        // far side cannot show what was hidden behind it anyway.
        let x1 = x + z * sy;
        let z1 = z * cy - x * sy;
        let zn = z1 - pivot_z;
        let y2 = y * cp - zn * sp;
        let z2 = y * sp + zn * cp + pivot_z;
        let moved = Vec2::new(cx + x1, pivot_y + y2);
        // What comes nearer grows a little and what goes away shrinks,
        // about the middle of the face, in proportion to how far it stands
        // out: the outline of a round head looks the same from any side.
        let centre = self.centre();
        let from = moved - centre;
        let reach = (1.3 * rx / from.length().max(1e-3)).min(1.0);
        let standing = (z / (self.depth.max(1e-3) * rx)).clamp(0.0, 1.0);
        let grow = self.perspective * standing * (z2 - z) / rx;
        let moved = moved + from * (grow * reach);
        p + (moved - p) * self.attachment(p)
    }

    /// Where the head's own surface at `p` goes.
    pub fn turn_surface(&self, p: Vec2, yaw: f32, pitch: f32) -> Vec2 {
        self.turn(p, self.surface(p), yaw, pitch)
    }

    /// How much further than the surface beneath it a part standing
    /// `lift` pixels in front of the head at `p` moves — the extra keyed on
    /// a warp nested inside the head's.
    pub fn parallax(&self, p: Vec2, lift: f32, yaw: f32, pitch: f32) -> Vec2 {
        let z = self.surface(p);
        self.turn(p, z + lift, yaw, pitch) - self.turn(p, z, yaw, pitch)
    }
}

/// Replace a warp deformer's keyforms with a generated head turn over
/// `x_param` (yaw) and `y_param` (pitch), keyed at each parameter's minimum,
/// default and maximum: the surface of [`HeadTurnOptions::shape`].
pub fn head_turn(
    rig: &mut Rig,
    deformer: DeformerId,
    x_param: ParameterId,
    y_param: Option<ParameterId>,
    options: &HeadTurnOptions,
) -> Result<()> {
    let rect = match rig.deformer(deformer).map(|d| &d.kind) {
        Some(DeformerKind::Warp(w)) => w.rect,
        Some(_) => return Err(AetherError::rig("a head turn needs a warp deformer")),
        None => return Err(AetherError::rig("no such deformer")),
    };
    let head = options.shape(rect);
    key_turn(rig, deformer, x_param, y_param, options, |p, yaw, pitch| {
        head.turn_surface(p, yaw, pitch)
    })
}

/// Key a warp deformer over `x_param` (yaw) and `y_param` (pitch) at each
/// parameter's minimum, default and maximum, moving each rest lattice point
/// `p` to `place(p, yaw, pitch)` (radians, reaching the options' angles at
/// the parameters' ends).
pub fn key_turn(
    rig: &mut Rig,
    deformer: DeformerId,
    x_param: ParameterId,
    y_param: Option<ParameterId>,
    options: &HeadTurnOptions,
    place: impl Fn(Vec2, f32, f32) -> Vec2,
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
            let offsets = if nx == 0.0 && ny == 0.0 {
                vec![Vec2::ZERO; rest.len()]
            } else {
                rest.iter().map(|p| place(*p, yaw, pitch) - *p).collect()
            };
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

    /// A face 200 px wide and 260 tall, eyes at y = 230, chin at 360.
    fn head() -> HeadShape {
        HeadShape::around(Rect::from_corners(vec2(100.0, 100.0), vec2(300.0, 360.0)))
    }

    #[test]
    fn a_head_rests_where_it_is_drawn() {
        let head = head();
        for p in [
            vec2(10.0, 10.0),
            vec2(200.0, 230.0),
            vec2(290.0, 340.0),
            vec2(200.0, 900.0),
        ] {
            assert!(head.turn_surface(p, 0.0, 0.0).distance(p) < 1e-3);
        }
    }

    #[test]
    fn turning_moves_the_face_and_leaves_its_outline() {
        let head = head();
        let yaw = 30f32.to_radians();
        let moved = |p: Vec2| head.turn_surface(p, yaw, 0.0) - p;
        let nose = moved(vec2(200.0, 270.0));
        let chin = moved(vec2(200.0, 358.0));
        let cheek = moved(vec2(299.0, 280.0));
        let crown = moved(vec2(200.0, head.crown));
        assert!(nose.x > 30.0, "the face swings toward the turn: {nose:?}");
        assert!(
            chin.x > 0.6 * nose.x,
            "the chin comes with the features ({chin:?} against {nose:?}) rather than shearing the face"
        );
        assert!(cheek.x.abs() < 0.2 * nose.x, "the outline stays: {cheek:?}");
        assert!(crown.x.abs() < 0.2 * nose.x, "and so does the crown: {crown:?}");
        assert!(nose.y.abs() < 3.0, "a turn does not lift the face: {nose:?}");
    }

    #[test]
    fn nodding_moves_the_chin_with_the_eyes() {
        let head = head();
        let eyes = vec2(200.0, 230.0);
        let chin = vec2(200.0, 358.0);
        let crown = vec2(200.0, head.crown + 1.0);
        for pitch in [20f32, -20.0] {
            let pitch = pitch.to_radians();
            let e = head.turn_surface(eyes, 0.0, pitch);
            let c = head.turn_surface(chin, 0.0, pitch);
            let t = head.turn_surface(crown, 0.0, pitch);
            let up = pitch > 0.0;
            assert_eq!(e.y < eyes.y, up, "the eyes follow the nod: {e:?}");
            assert_eq!(c.y < chin.y, up, "and so does the chin: {c:?}");
            let length = c.y - e.y;
            assert!(
                length <= (chin.y - eyes.y) + 0.5,
                "the lower face never stretches: {length} against {}",
                chin.y - eyes.y
            );
            // Looking down shows more of the top of the head.
            let top = e.y - t.y;
            if up {
                assert!(top < eyes.y - crown.y, "looking up flattens the top: {top}");
            } else {
                assert!(top > eyes.y - crown.y, "looking down shows the top: {top}");
            }
        }
    }

    #[test]
    fn long_hair_stays_put_at_its_ends() {
        let head = head();
        let tip = vec2(200.0, 1000.0);
        let root = vec2(320.0, 150.0);
        let (yaw, pitch) = (30f32.to_radians(), 20f32.to_radians());
        assert!(head.turn_surface(tip, yaw, pitch).distance(tip) < 1e-3);
        assert!(
            head.turn_surface(root, yaw, pitch).distance(root) > 1.0,
            "the root goes with the head"
        );
    }

    #[test]
    fn parts_in_front_of_the_face_move_further() {
        let head = head();
        let p = vec2(200.0, 230.0);
        let extra = head.parallax(p, 20.0, 30f32.to_radians(), 0.0);
        assert!(extra.x > 5.0, "{extra:?}");
        let behind = head.parallax(p, -20.0, 30f32.to_radians(), 0.0);
        assert!(behind.x < -5.0, "{behind:?}");
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
        // Sample dense rows and columns and require them to keep their
        // order: a fold would show as a coordinate going backwards.
        let head = head();
        for yaw_deg in [-45.0f32, -30.0, 30.0, 45.0] {
            for pitch_deg in [-25.0f32, 0.0, 25.0] {
                let (yaw, pitch) = (yaw_deg.to_radians(), pitch_deg.to_radians());
                for row in [120.0, 230.0, 300.0, 360.0, 420.0] {
                    let mut last = f32::NEG_INFINITY;
                    for i in 0..=800 {
                        let x = i as f32 * 0.5;
                        let p = head.turn_surface(vec2(x, row), yaw, pitch);
                        assert!(
                            p.x > last,
                            "fold at x = {x}, y = {row}, yaw {yaw_deg}, pitch {pitch_deg}"
                        );
                        last = p.x;
                    }
                }
                for column in [120.0, 170.0, 200.0, 250.0, 290.0] {
                    let mut last = f32::NEG_INFINITY;
                    for i in 0..=1000 {
                        let y = i as f32 * 0.5;
                        let p = head.turn_surface(vec2(column, y), yaw, pitch);
                        assert!(
                            p.y > last,
                            "fold at y = {y}, x = {column}, yaw {yaw_deg}, pitch {pitch_deg}"
                        );
                        last = p.y;
                    }
                }
            }
        }
    }
}
