//! Bones, forward and inverse kinematics, and skinning.
//!
//! Warp and rotation deformers are excellent for faces, where the motion is
//! small and hand-shaped. Limbs are different: an arm is a chain of rigid
//! segments, and posing it by hand-warping lattices is slow and fragile. Bones
//! give limbs, tails and ribbons a proper skeleton:
//!
//! * **Forward kinematics** — each bone has a keyform grid of local rotation,
//!   translation and length scale, so bones are posed by parameters exactly
//!   like everything else in the rig and animate on the same timeline.
//! * **Inverse kinematics** — a constraint makes the tip of a chain reach for
//!   a target bone: an exact analytic solve for two-bone limbs, and cyclic
//!   coordinate descent for longer chains, blended with FK by a weight.
//! * **Skinning** — meshes carry per-vertex bone weights and are deformed by
//!   linear blend skinning, so an elbow bends smoothly instead of splitting.
//!
//! A bone's rest pose is given in document space by its head and tail. Its
//! local frame has the origin at the head and the x axis pointing along the
//! bone.

use crate::keyform::{Blend, KeyformGrid, ParamSource};
use aether_core::math::{Transform2D, Vec2};
use aether_core::{AetherError, BoneId, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

/// A bone's local pose at one keyform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoneForm {
    /// Rotation about the head, degrees, clockwise on screen.
    pub rotation: f32,
    /// Translation of the head, in the parent's rest frame (document space for
    /// a root bone).
    pub translation: Vec2,
    /// Stretch along the bone.
    #[serde(default = "one")]
    pub scale: f32,
}

impl BoneForm {
    /// No change from rest.
    pub fn rest() -> Self {
        Self {
            rotation: 0.0,
            translation: Vec2::ZERO,
            scale: 1.0,
        }
    }
}

impl Blend for BoneForm {
    fn zeroed(&self) -> Self {
        Self {
            rotation: 0.0,
            translation: Vec2::ZERO,
            scale: 0.0,
        }
    }

    fn add_scaled(&mut self, other: &Self, weight: f32) {
        self.rotation += other.rotation * weight;
        self.translation += other.translation * weight;
        self.scale += other.scale * weight;
    }
}

/// Make the tip of a chain reach for a target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IkConstraint {
    /// The bone whose head is the goal.
    pub target: BoneId,
    /// How many bones, counting up from the constrained bone, may rotate.
    pub chain: usize,
    /// Which way a two-bone chain bends.
    #[serde(default = "yes")]
    pub bend_positive: bool,
    /// Mix between FK (0) and IK (1).
    #[serde(default = "one")]
    pub weight: f32,
}

/// One bone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    /// Stable identity.
    pub id: BoneId,
    /// Display name.
    pub name: String,
    /// Parent bone.
    #[serde(default)]
    pub parent: Option<BoneId>,
    /// Rest position of the head, document space.
    pub head: Vec2,
    /// Rest position of the tail, document space.
    pub tail: Vec2,
    /// Whether the bone deforms meshes. Control bones (IK targets, handles)
    /// set this to false so automatic weighting ignores them.
    #[serde(default = "yes")]
    pub deform: bool,
    /// Local pose keyforms.
    pub keyforms: KeyformGrid<BoneForm>,
    /// Optional IK constraint ending at this bone.
    #[serde(default)]
    pub ik: Option<IkConstraint>,
}

impl Bone {
    /// A bone from `head` to `tail` with no keyforms.
    pub fn new(id: BoneId, name: impl Into<String>, head: Vec2, tail: Vec2) -> Self {
        Self {
            id,
            name: name.into(),
            parent: None,
            head,
            tail,
            deform: true,
            keyforms: KeyformGrid::constant(BoneForm::rest()),
            ik: None,
        }
    }

    /// Rest length (never zero).
    pub fn length(&self) -> f32 {
        self.head.distance(self.tail).max(1e-3)
    }

    /// Rest direction, radians.
    pub fn rest_angle(&self) -> f32 {
        (self.tail - self.head).angle()
    }

    /// Rest world transform: bone-local → document.
    pub fn rest_world(&self) -> Transform2D {
        Transform2D::rotation(self.rest_angle()).then(&Transform2D::translation(self.head))
    }
}

/// The posed skeleton.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkeletonPose {
    /// Posed world transform per bone.
    pub world: BTreeMap<BoneId, Transform2D>,
    /// Rest-to-posed transform per bone, the skinning matrix.
    pub skinning: BTreeMap<BoneId, Transform2D>,
    /// Bone lengths as posed (rest length × scale).
    pub lengths: BTreeMap<BoneId, f32>,
}

impl SkeletonPose {
    /// Posed head of a bone.
    pub fn head(&self, id: BoneId) -> Option<Vec2> {
        self.world.get(&id).map(|w| w.apply(Vec2::ZERO))
    }

    /// Posed tail of a bone.
    pub fn tail(&self, id: BoneId) -> Option<Vec2> {
        let length = *self.lengths.get(&id)?;
        self.world.get(&id).map(|w| w.apply(Vec2::new(length, 0.0)))
    }

    /// Map a rest-space point rigidly with one bone.
    pub fn transform(&self, id: BoneId, p: Vec2) -> Vec2 {
        self.skinning.get(&id).map(|m| m.apply(p)).unwrap_or(p)
    }
}

/// Bones ordered so every parent precedes its children. Bones whose parent
/// chain loops or points at a missing bone are treated as roots, so a damaged
/// file still poses.
pub fn topological_order(bones: &[Bone]) -> Vec<usize> {
    let index: BTreeMap<BoneId, usize> = bones.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
    let mut depth = vec![0usize; bones.len()];
    for (i, bone) in bones.iter().enumerate() {
        let mut d = 0;
        let mut current = bone.parent;
        while let Some(p) = current {
            let Some(&pi) = index.get(&p) else { break };
            d += 1;
            if d > bones.len() {
                d = 0; // cycle: treat as a root
                break;
            }
            current = bones[pi].parent;
        }
        depth[i] = d;
    }
    let mut order: Vec<usize> = (0..bones.len()).collect();
    order.sort_by_key(|&i| depth[i]);
    order
}

/// Check the skeleton for missing parents and cycles.
pub fn validate(bones: &[Bone]) -> Result<()> {
    let ids: BTreeMap<BoneId, usize> = bones.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
    if ids.len() != bones.len() {
        return Err(AetherError::rig("two bones share an id"));
    }
    for bone in bones {
        bone.keyforms.validate()?;
        let mut steps = 0;
        let mut current = bone.parent;
        while let Some(p) = current {
            let Some(&pi) = ids.get(&p) else {
                return Err(AetherError::rig(format!(
                    "bone '{}' has a missing parent",
                    bone.name
                )));
            };
            steps += 1;
            if steps > bones.len() {
                return Err(AetherError::rig(format!(
                    "bone '{}' is its own ancestor",
                    bone.name
                )));
            }
            current = bones[pi].parent;
        }
        if let Some(ik) = &bone.ik {
            if !ids.contains_key(&ik.target) {
                return Err(AetherError::rig(format!(
                    "the IK target of '{}' is missing",
                    bone.name
                )));
            }
        }
    }
    Ok(())
}

/// Pose the skeleton: forward kinematics from keyforms, then IK.
pub fn solve(bones: &[Bone], source: &dyn ParamSource) -> SkeletonPose {
    let order = topological_order(bones);
    let by_id: BTreeMap<BoneId, usize> = bones.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
    let rest_world: Vec<Transform2D> = bones.iter().map(|b| b.rest_world()).collect();

    // Local transforms (posed) relative to the parent's posed frame.
    let mut local: Vec<Transform2D> = vec![Transform2D::IDENTITY; bones.len()];
    let mut lengths: Vec<f32> = vec![1.0; bones.len()];
    for &i in &order {
        let bone = &bones[i];
        let form = bone.keyforms.evaluate(source);
        let parent_rest = bone
            .parent
            .and_then(|p| by_id.get(&p))
            .map(|&pi| rest_world[pi])
            .unwrap_or(Transform2D::IDENTITY);
        let rest_local = parent_rest
            .inverse()
            .map(|inv| rest_world[i].then(&inv))
            .unwrap_or(rest_world[i]);
        let scale = if form.scale.is_finite() { form.scale } else { 1.0 };
        // Stretch along the bone, rotate about the head, place relative to
        // the parent, then translate in the parent's frame.
        local[i] = Transform2D::scale(Vec2::new(scale, 1.0))
            .then(&Transform2D::rotation(form.rotation.to_radians()))
            .then(&rest_local)
            .then(&Transform2D::translation(form.translation));
        lengths[i] = bone.length() * scale;
    }

    let mut world = compute_world(bones, &order, &by_id, &local);

    // Inverse kinematics, in declaration order.
    for (end, bone) in bones.iter().enumerate() {
        let Some(ik) = &bone.ik else { continue };
        let Some(&target_index) = by_id.get(&ik.target) else {
            continue;
        };
        let weight = ik.weight.clamp(0.0, 1.0);
        if weight <= 0.0 {
            continue;
        }
        let target = world[target_index].apply(Vec2::ZERO);
        // The chain, from the root-most bone down to `end`.
        let mut chain = vec![end];
        let mut current = bone.parent;
        while chain.len() < ik.chain.max(1) {
            let Some(p) = current else { break };
            let Some(&pi) = by_id.get(&p) else { break };
            if chain.contains(&pi) {
                break;
            }
            chain.push(pi);
            current = bones[pi].parent;
        }
        chain.reverse();
        // A target inside the chain would chase itself.
        if chain
            .iter()
            .any(|&c| is_ancestor_or_self(bones, &by_id, bones[c].id, ik.target))
        {
            continue;
        }

        if chain.len() == 2 {
            solve_two_bone(
                bones,
                &order,
                &by_id,
                &mut local,
                &mut world,
                &lengths,
                (chain[0], chain[1]),
                target,
                ik.bend_positive,
                weight,
            );
        } else {
            for _ in 0..16 {
                for &link in chain.iter().rev() {
                    let head = world[link].apply(Vec2::ZERO);
                    let tip = world[end].apply(Vec2::new(lengths[end], 0.0));
                    let to_tip = tip - head;
                    let to_target = target - head;
                    if to_tip.length_squared() < 1e-8 || to_target.length_squared() < 1e-8 {
                        continue;
                    }
                    let delta = wrap_angle(to_target.angle() - to_tip.angle()) * weight;
                    rotate_world(bones, &order, &by_id, &mut local, &mut world, link, delta);
                }
            }
        }
    }

    let mut pose = SkeletonPose::default();
    for (i, bone) in bones.iter().enumerate() {
        let skin = rest_world[i]
            .inverse()
            .map(|inv| inv.then(&world[i]))
            .unwrap_or(Transform2D::IDENTITY);
        pose.world.insert(bone.id, world[i]);
        pose.skinning.insert(bone.id, skin);
        pose.lengths.insert(bone.id, lengths[i]);
    }
    pose
}

fn is_ancestor_or_self(
    bones: &[Bone],
    by_id: &BTreeMap<BoneId, usize>,
    ancestor: BoneId,
    of: BoneId,
) -> bool {
    let mut current = Some(of);
    let mut steps = 0;
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        steps += 1;
        if steps > bones.len() {
            return false;
        }
        current = by_id.get(&id).and_then(|&i| bones[i].parent);
    }
    false
}

fn compute_world(
    bones: &[Bone],
    order: &[usize],
    by_id: &BTreeMap<BoneId, usize>,
    local: &[Transform2D],
) -> Vec<Transform2D> {
    let mut world = vec![Transform2D::IDENTITY; bones.len()];
    for &i in order {
        let parent = bones[i].parent.and_then(|p| by_id.get(&p)).copied();
        world[i] = match parent {
            Some(pi) if pi != i => local[i].then(&world[pi]),
            _ => local[i],
        };
    }
    world
}

/// Rotate bone `index` by `delta` radians about its head in world space,
/// updating its local transform and every descendant's world transform.
fn rotate_world(
    bones: &[Bone],
    order: &[usize],
    by_id: &BTreeMap<BoneId, usize>,
    local: &mut [Transform2D],
    world: &mut Vec<Transform2D>,
    index: usize,
    delta: f32,
) {
    if delta.abs() < 1e-9 {
        return;
    }
    let head = world[index].apply(Vec2::ZERO);
    let spin = Transform2D::translation(-head)
        .then(&Transform2D::rotation(delta))
        .then(&Transform2D::translation(head));
    let new_world = world[index].then(&spin);
    let parent_world = bones[index]
        .parent
        .and_then(|p| by_id.get(&p))
        .map(|&pi| world[pi])
        .unwrap_or(Transform2D::IDENTITY);
    local[index] = parent_world
        .inverse()
        .map(|inv| new_world.then(&inv))
        .unwrap_or(new_world);
    *world = compute_world(bones, order, by_id, local);
}

#[allow(clippy::too_many_arguments)]
fn solve_two_bone(
    bones: &[Bone],
    order: &[usize],
    by_id: &BTreeMap<BoneId, usize>,
    local: &mut [Transform2D],
    world: &mut Vec<Transform2D>,
    lengths: &[f32],
    (upper, lower): (usize, usize),
    target: Vec2,
    bend_positive: bool,
    weight: f32,
) {
    let a = world[upper].apply(Vec2::ZERO);
    let la = lengths[upper].abs().max(1e-4);
    let lb = lengths[lower].abs().max(1e-4);
    let to_target = target - a;
    let d = to_target.length().clamp((la - lb).abs() + 1e-4, la + lb - 1e-4);
    let cos_a = ((la * la + d * d - lb * lb) / (2.0 * la * d)).clamp(-1.0, 1.0);
    let bend = if bend_positive { 1.0 } else { -1.0 };
    let desired_upper = to_target.angle() - bend * cos_a.acos();
    let current_upper = world_angle(&world[upper]);
    rotate_world(
        bones,
        order,
        by_id,
        local,
        world,
        upper,
        wrap_angle(desired_upper - current_upper) * weight,
    );
    let b = world[lower].apply(Vec2::ZERO);
    let desired_lower = (target - b).angle();
    let current_lower = world_angle(&world[lower]);
    rotate_world(
        bones,
        order,
        by_id,
        local,
        world,
        lower,
        wrap_angle(desired_lower - current_lower) * weight,
    );
}

fn world_angle(t: &Transform2D) -> f32 {
    t.b.atan2(t.a)
}

/// Wrap an angle into `-π..=π`.
pub fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut a = a % tau;
    if a > std::f32::consts::PI {
        a -= tau;
    } else if a < -std::f32::consts::PI {
        a += tau;
    }
    a
}

/// Automatic skin weights for `vertices` from the deforming bones.
///
/// Each vertex is weighted by inverse distance to each bone segment raised to
/// `falloff` (higher values give sharper joints), keeping the four strongest
/// influences. The result is a starting point meant to be refined by painting.
pub fn auto_weights(bones: &[Bone], vertices: &[Vec2], falloff: f32) -> crate::mesh::Skin {
    let deforming: Vec<&Bone> = bones.iter().filter(|b| b.deform).collect();
    let mut skin = crate::mesh::Skin::new(deforming.iter().map(|b| b.id).collect(), vertices.len());
    if deforming.is_empty() {
        return skin;
    }
    let falloff = falloff.clamp(0.5, 8.0);
    for (v, p) in vertices.iter().enumerate() {
        let mut weights: Vec<(usize, f32)> = deforming
            .iter()
            .enumerate()
            .map(|(slot, bone)| {
                let (q, _) = crate::geom::closest_on_segment(*p, bone.head, bone.tail);
                let d = q.distance(*p).max(0.5);
                (slot, 1.0 / d.powf(falloff))
            })
            .collect();
        weights.sort_by(|a, b| b.1.total_cmp(&a.1));
        weights.truncate(4);
        for (slot, w) in weights {
            skin.set_weight(v, slot, w);
        }
    }
    skin.normalize();
    skin
}

/// Linear blend skinning of one point.
pub fn skin_point(pose: &SkeletonPose, skin: &crate::mesh::Skin, vertex: usize, p: Vec2) -> Vec2 {
    let weights = skin.vertex(vertex);
    let total: f32 = weights.iter().sum();
    if total <= 1e-6 {
        return p;
    }
    let mut acc = Vec2::ZERO;
    for (slot, &w) in weights.iter().enumerate() {
        if w <= 0.0 {
            continue;
        }
        let q = skin
            .bones
            .get(slot)
            .and_then(|id| pose.skinning.get(id))
            .map(|m| m.apply(p))
            .unwrap_or(p);
        acc += q * (w / total);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyform::{AxisInput, KeyAxis};
    use aether_core::math::vec2;
    use aether_core::ParameterId;

    const P: ParameterId = ParameterId(100);

    fn source(value: f32) -> impl ParamSource {
        move |id: ParameterId| (id == P).then_some(AxisInput { value, cycle: None })
    }

    fn arm() -> Vec<Bone> {
        let mut upper = Bone::new(BoneId(1), "upper", vec2(0.0, 0.0), vec2(10.0, 0.0));
        upper
            .keyforms
            .add_axis(KeyAxis::new(P, [0.0, 1.0]).expect("axis"))
            .expect("axis");
        upper.keyforms.forms[1].rotation = 90.0;
        let mut lower = Bone::new(BoneId(2), "lower", vec2(10.0, 0.0), vec2(20.0, 0.0));
        lower.parent = Some(BoneId(1));
        vec![upper, lower]
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn the_rest_pose_is_the_identity() {
        let bones = arm();
        let pose = solve(&bones, &source(0.0));
        for bone in &bones {
            assert!(
                pose.skinning[&bone.id].is_identity(),
                "{} moved at rest",
                bone.name
            );
        }
        assert!(close(pose.tail(BoneId(2)).expect("tail"), vec2(20.0, 0.0)));
    }

    #[test]
    fn rotating_a_parent_carries_its_children() {
        let bones = arm();
        let pose = solve(&bones, &source(1.0));
        // Upper arm points down (clockwise on screen with y down).
        assert!(close(pose.tail(BoneId(1)).expect("tail"), vec2(0.0, 10.0)));
        assert!(close(pose.tail(BoneId(2)).expect("tail"), vec2(0.0, 20.0)));
        // A point on the forearm moves with it.
        assert!(close(pose.transform(BoneId(2), vec2(15.0, 0.0)), vec2(0.0, 15.0)));
    }

    #[test]
    fn two_bone_ik_reaches_a_reachable_target() {
        let mut bones = arm();
        let target = Bone::new(BoneId(3), "target", vec2(12.0, 8.0), vec2(13.0, 8.0));
        bones.push(target);
        bones[1].ik = Some(IkConstraint {
            target: BoneId(3),
            chain: 2,
            bend_positive: true,
            weight: 1.0,
        });
        validate(&bones).expect("valid");
        let pose = solve(&bones, &source(0.0));
        let tip = pose.tail(BoneId(2)).expect("tip");
        assert!(tip.distance(vec2(12.0, 8.0)) < 1e-2, "tip at {tip:?}");
        // Bone lengths are preserved.
        let elbow = pose.head(BoneId(2)).expect("elbow");
        assert!((elbow.length() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn ik_bends_the_requested_way() {
        let mut bones = arm();
        bones.push(Bone::new(BoneId(3), "target", vec2(14.0, 0.0), vec2(15.0, 0.0)));
        for bend in [true, false] {
            bones[1].ik = Some(IkConstraint {
                target: BoneId(3),
                chain: 2,
                bend_positive: bend,
                weight: 1.0,
            });
            let pose = solve(&bones, &source(0.0));
            let elbow = pose.head(BoneId(2)).expect("elbow");
            assert!(elbow.y.abs() > 1.0, "the elbow must leave the line");
            assert_eq!(elbow.y < 0.0, bend, "bend {bend} put the elbow at {elbow:?}");
        }
    }

    #[test]
    fn ccd_solves_longer_chains() {
        let mut bones: Vec<Bone> = (0..4)
            .map(|i| {
                let mut b = Bone::new(
                    BoneId(i + 1),
                    format!("b{i}"),
                    vec2(i as f32 * 10.0, 0.0),
                    vec2((i + 1) as f32 * 10.0, 0.0),
                );
                if i > 0 {
                    b.parent = Some(BoneId(i));
                }
                b
            })
            .collect();
        bones.push(Bone::new(BoneId(10), "goal", vec2(20.0, 20.0), vec2(21.0, 20.0)));
        bones[3].ik = Some(IkConstraint {
            target: BoneId(10),
            chain: 4,
            bend_positive: true,
            weight: 1.0,
        });
        let pose = solve(&bones, &source(0.0));
        let tip = pose.tail(BoneId(4)).expect("tip");
        assert!(tip.distance(vec2(20.0, 20.0)) < 0.5, "tip at {tip:?}");
    }

    #[test]
    fn unreachable_targets_stretch_toward_them_without_breaking() {
        let mut bones = arm();
        bones.push(Bone::new(BoneId(3), "target", vec2(0.0, 100.0), vec2(1.0, 100.0)));
        bones[1].ik = Some(IkConstraint {
            target: BoneId(3),
            chain: 2,
            bend_positive: true,
            weight: 1.0,
        });
        let pose = solve(&bones, &source(0.0));
        let tip = pose.tail(BoneId(2)).expect("tip");
        assert!(tip.is_finite());
        assert!(tip.y > 19.0, "the arm points at the target: {tip:?}");
    }

    #[test]
    fn cycles_and_missing_parents_are_reported_but_still_pose() {
        let mut bones = arm();
        bones[0].parent = Some(BoneId(2));
        assert!(validate(&bones).is_err());
        let pose = solve(&bones, &source(0.0));
        assert_eq!(pose.world.len(), 2);
    }

    #[test]
    fn auto_weights_favour_the_nearest_bone_and_normalise() {
        let bones = arm();
        let vertices = vec![vec2(2.0, 1.0), vec2(18.0, 1.0), vec2(10.0, 1.0)];
        let skin = auto_weights(&bones, &vertices, 2.0);
        assert!(skin.weight(0, 0) > skin.weight(0, 1));
        assert!(skin.weight(1, 1) > skin.weight(1, 0));
        for v in 0..3 {
            assert!((skin.vertex(v).iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        let pose = solve(&bones, &source(1.0));
        let moved = skin_point(&pose, &skin, 1, vertices[1]);
        assert!(moved.y > 10.0, "the forearm vertex swings down: {moved:?}");
    }

    #[test]
    fn angles_wrap_into_range() {
        assert!((wrap_angle(3.0 * std::f32::consts::PI) - std::f32::consts::PI).abs() < 1e-4);
        assert!((wrap_angle(-0.5) + 0.5).abs() < 1e-6);
    }
}
