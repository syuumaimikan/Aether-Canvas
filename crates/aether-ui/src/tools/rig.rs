//! Rig tools: editing meshes, deforming keyforms and drawing bones.
//!
//! All three follow the live-edit pattern of the painting tools: the rig is
//! changed directly while the pointer is down (the canvas redraws whatever the
//! pose change touched), and one [`SetRigCommand`] holding the before and
//! after rig is recorded when the gesture ends.
//!
//! * [`MeshTool`] edits the active layer's mesh in its rest pose: click to add
//!   a vertex, drag a vertex to move it, Alt-click to delete one.
//! * [`DeformTool`] shapes the keyform the current pose sits on. Meshes and
//!   warp lattices are pushed with a soft brush whose falloff the artist
//!   sets; rotation deformers turn (drag), scale (Shift) or move their pivot
//!   (drag the pivot); bones rotate (drag) or translate (Ctrl). Drags inside a
//!   deformed parent are mapped back through the parent, so points follow the
//!   cursor even inside a turned head. A click without a drag selects what is
//!   under the pointer.
//! * [`BoneTool`] drags out new bones; starting on a bone's tail chains the
//!   new bone to it.

use super::{Tool, ToolContext, ToolEvent, ToolId, ToolPreview};
use aether_core::math::Vec2;
use aether_core::{BoneId, LayerId};
use aether_document::rig::{ArtMesh, Bone, DeformerKind, Evaluator, Rig, RigNode};
use aether_document::SetRigCommand;

/// Screen-space pick radius, pixels.
const PICK_RADIUS: f32 = 9.0;

fn pick_radius(ctx: &ToolContext) -> f32 {
    PICK_RADIUS / ctx.viewport.zoom.max(1e-3)
}

/// Record a finished live edit.
fn record(ctx: &mut ToolContext, label: &str, before: Rig) {
    let command = SetRigCommand::applied(label, before, ctx.doc.rig.clone());
    if !command.is_noop() {
        ctx.history.push_applied(Box::new(command));
        ctx.doc.mark_all_dirty();
    }
}

/// Put the rig back to `before` without recording anything.
fn restore(ctx: &mut ToolContext, before: Rig) {
    let values = std::mem::take(&mut ctx.doc.rig.values);
    let dynamics = std::mem::take(&mut ctx.doc.rig.dynamics);
    ctx.doc.rig = before;
    ctx.doc.rig.values = values;
    ctx.doc.rig.dynamics = dynamics;
    ctx.doc.mark_all_dirty();
}

/// What is under `p`: bones first (they are thin), then the topmost mesh.
pub fn pick(doc: &aether_document::Document, p: Vec2, radius: f32) -> Option<RigNode> {
    let rig = &doc.rig;
    let eval = Evaluator::new(rig);
    let skeleton = eval.skeleton();
    let mut best: Option<(f32, BoneId)> = None;
    for bone in &rig.bones {
        let (Some(head), Some(tail)) = (skeleton.head(bone.id), skeleton.tail(bone.id)) else {
            continue;
        };
        let (q, _) = aether_document::rig::geom::closest_on_segment(p, head, tail);
        let d = q.distance(p);
        if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, bone.id));
        }
    }
    if let Some((_, id)) = best {
        return Some(RigNode::Bone(id));
    }
    for (id, _) in doc.layers.iter_ui_order() {
        let Some(layer) = doc.layers.get(id) else { continue };
        if !layer.visible {
            continue;
        }
        let Some(mesh) = rig.mesh(id) else { continue };
        let pose = eval.mesh_pose(mesh);
        if let Some(aether_document::rig::geom::Location::Inside { .. }) =
            aether_document::rig::geom::locate(p, &pose.positions, &mesh.triangles)
        {
            return Some(RigNode::Mesh(id));
        }
    }
    None
}

// ------------------------------------------------------------------ mesh

/// Edit the active layer's mesh in its rest pose.
#[derive(Default)]
pub struct MeshTool {
    dragging: Option<(LayerId, usize, Rig)>,
}

impl MeshTool {
    /// A mesh tool with no gesture in progress.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Tool for MeshTool {
    fn id(&self) -> ToolId {
        ToolId::Mesh
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let layer = ctx.doc.active_layer;
        let Some(mesh) = ctx.doc.rig.mesh(layer) else {
            ctx.report("This layer has no mesh yet: use Auto mesh in the tool options");
            return;
        };
        ctx.rig.selection = Some(RigNode::Mesh(layer));
        let p = event.sample.position;
        let near = ArtMesh::nearest_vertex(&mesh.vertices, p, pick_radius(ctx));
        let before = ctx.doc.rig.clone();
        match (near, event.modifiers.alt) {
            (Some(index), true) => {
                let result = ctx
                    .doc
                    .rig
                    .mesh_mut(layer)
                    .map(|m| m.remove_vertex(index))
                    .unwrap_or(Ok(()));
                match result {
                    Ok(()) => record(ctx, "Delete vertex", before),
                    Err(e) => ctx.report(e.to_string()),
                }
            }
            (Some(index), false) => self.dragging = Some((layer, index, before)),
            (None, _) => {
                let result = ctx.doc.rig.mesh_mut(layer).map(|m| m.add_vertex(p));
                match result {
                    Some(Ok(index)) => {
                        record(ctx, "Add vertex", before);
                        // Keep dragging the new vertex if the pointer moves.
                        self.dragging = Some((layer, index, ctx.doc.rig.clone()));
                    }
                    Some(Err(e)) => ctx.report(e.to_string()),
                    None => {}
                }
            }
        }
    }

    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let Some((layer, index, _)) = &self.dragging else {
            return;
        };
        let p = event.sample.position;
        if let Some(mesh) = ctx.doc.rig.mesh_mut(*layer) {
            let _ = mesh.move_rest_vertex(*index, p);
        }
    }

    fn pointer_up(&mut self, ctx: &mut ToolContext, _event: &ToolEvent) {
        if let Some((_, _, before)) = self.dragging.take() {
            record(ctx, "Move vertex", before);
        }
    }

    fn cancel(&mut self, ctx: &mut ToolContext) {
        if let Some((_, _, before)) = self.dragging.take() {
            restore(ctx, before);
        }
    }

    fn is_active(&self) -> bool {
        self.dragging.is_some()
    }
}

// ---------------------------------------------------------------- deform

#[derive(Clone, Copy, Debug, PartialEq)]
enum RotationGrip {
    Pivot,
    Turn,
    Scale,
}

#[derive(Clone, Debug)]
enum Gesture {
    /// Soft-brush edit of mesh vertices or lattice points: index and weight.
    Points(Vec<(usize, f32)>),
    Rotation(RotationGrip),
    BoneRotate,
    BoneMove,
}

/// Shape the keyform at the current pose.
#[derive(Default)]
pub struct DeformTool {
    target: Option<RigNode>,
    gesture: Option<Gesture>,
    before: Option<Rig>,
    start: Vec2,
    last: Vec2,
    moved: bool,
}

impl DeformTool {
    /// A deform tool with no gesture in progress.
    pub fn new() -> Self {
        Self::default()
    }

    fn falloff(d: f32, radius: f32) -> f32 {
        if radius <= 0.0 {
            return 0.0;
        }
        let t = (1.0 - (d / radius).powi(2)).clamp(0.0, 1.0);
        t * t
    }

    /// Weights for points near `p`: a soft brush, or only the nearest point
    /// with Alt.
    fn brush(points: &[Vec2], p: Vec2, radius: f32, pick: f32, single: bool) -> Vec<(usize, f32)> {
        let nearest = ArtMesh::nearest_vertex(points, p, pick.max(radius * 0.25));
        if single {
            return nearest.map(|i| vec![(i, 1.0)]).unwrap_or_default();
        }
        let mut weights: Vec<(usize, f32)> = points
            .iter()
            .enumerate()
            .filter_map(|(i, q)| {
                let w = Self::falloff(q.distance(p), radius);
                (w > 1e-3).then_some((i, w))
            })
            .collect();
        if weights.is_empty() {
            if let Some(i) = nearest {
                weights.push((i, 1.0));
            }
        }
        weights
    }

    fn apply_delta(&mut self, ctx: &mut ToolContext, p: Vec2, delta: Vec2) -> aether_core::Result<()> {
        let (Some(node), Some(gesture)) = (self.target, self.gesture.clone()) else {
            return Ok(());
        };
        let blend = ctx.rig.blend_shape;
        match (node, gesture) {
            (RigNode::Mesh(layer), Gesture::Points(weights)) => {
                let local = {
                    let rig = &ctx.doc.rig;
                    let eval = Evaluator::new(rig);
                    let mesh = rig
                        .mesh(layer)
                        .ok_or_else(|| aether_core::AetherError::rig("no such mesh"))?;
                    let (_, local) = eval.mesh_local(mesh);
                    weights
                        .iter()
                        .filter_map(|&(i, w)| {
                            Some((i, eval.unmap_delta(mesh.parent, *local.get(i)?, delta * w)))
                        })
                        .collect::<Vec<(usize, Vec2)>>()
                };
                let form = ctx.doc.rig.mesh_form_mut(layer, blend)?;
                for (i, d) in local {
                    if let Some(o) = form.offsets.get_mut(i) {
                        *o += d;
                    }
                }
            }
            (RigNode::Deformer(id), Gesture::Points(weights)) => {
                let moves: Vec<(usize, Vec2)> = {
                    let rig = &ctx.doc.rig;
                    let eval = Evaluator::new(rig);
                    let d = rig
                        .deformer(id)
                        .ok_or_else(|| aether_core::AetherError::rig("no such deformer"))?;
                    let local = match eval.deformer_state(id).map(|s| &s.map) {
                        Some(aether_document::rig::deformer::DeformerMap::Warp(w)) => w.points(),
                        _ => Vec::new(),
                    };
                    weights
                        .iter()
                        .filter_map(|&(i, w)| {
                            Some((i, eval.unmap_delta(d.parent, *local.get(i)?, delta * w)))
                        })
                        .collect()
                };
                let form = ctx.doc.rig.warp_form_mut(id, blend)?;
                for (i, d) in moves {
                    if let Some(o) = form.offsets.get_mut(i) {
                        *o += d;
                    }
                }
            }
            (RigNode::Deformer(id), Gesture::Rotation(grip)) => {
                let (pivot_screen, parent_delta) = {
                    let rig = &ctx.doc.rig;
                    let eval = Evaluator::new(rig);
                    let handles = eval.deformer_handles(id);
                    let pivot = handles.first().copied().unwrap_or(p);
                    let parent = rig.deformer(id).and_then(|d| d.parent);
                    let state_pivot = match eval.deformer_state(id).map(|s| &s.map) {
                        Some(aether_document::rig::deformer::DeformerMap::Rotation(r)) => r.pivot(),
                        _ => pivot,
                    };
                    (pivot, eval.unmap_delta(parent, state_pivot, delta))
                };
                let form = ctx.doc.rig.rotation_form_mut(id, blend)?;
                match grip {
                    RotationGrip::Pivot => form.offset += parent_delta,
                    RotationGrip::Turn => {
                        let a0 = (self.last - pivot_screen).angle();
                        let a1 = (p - pivot_screen).angle();
                        form.angle += aether_document::rig::skeleton::wrap_angle(a1 - a0).to_degrees();
                    }
                    RotationGrip::Scale => {
                        let d0 = self.last.distance(pivot_screen).max(1.0);
                        let d1 = p.distance(pivot_screen).max(1.0);
                        form.scale = (form.scale * d1 / d0).clamp(0.01, 100.0);
                    }
                }
            }
            (RigNode::Bone(id), Gesture::BoneRotate) => {
                let head = Evaluator::new(&ctx.doc.rig)
                    .skeleton()
                    .head(id)
                    .unwrap_or(self.start);
                let a0 = (self.last - head).angle();
                let a1 = (p - head).angle();
                let form = ctx.doc.rig.bone_form_mut(id)?;
                form.rotation += aether_document::rig::skeleton::wrap_angle(a1 - a0).to_degrees();
            }
            (RigNode::Bone(id), Gesture::BoneMove) => {
                // Translation is expressed in the parent's rest frame; undo the
                // parent's current rotation so the head follows the cursor.
                let local = {
                    let rig = &ctx.doc.rig;
                    let eval = Evaluator::new(rig);
                    let parent = rig.bone(id).and_then(|b| b.parent);
                    match parent.and_then(|pid| eval.skeleton().skinning.get(&pid)) {
                        Some(m) => {
                            let inv = m.inverse().unwrap_or(aether_core::math::Transform2D::IDENTITY);
                            inv.apply_vector(delta)
                        }
                        None => delta,
                    }
                };
                let form = ctx.doc.rig.bone_form_mut(id)?;
                form.translation += local;
            }
            _ => {}
        }
        Ok(())
    }
}

impl Tool for DeformTool {
    fn id(&self) -> ToolId {
        ToolId::Deform
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let p = event.sample.position;
        self.start = p;
        self.last = p;
        self.moved = false;
        self.gesture = None;
        let node = ctx.rig.selection.or_else(|| {
            ctx.doc
                .rig
                .mesh(ctx.doc.active_layer)
                .map(|m| RigNode::Mesh(m.layer))
        });
        let Some(node) = node.filter(|n| ctx.doc.rig.contains(*n)) else {
            // Nothing selected: this press can only select.
            self.target = None;
            return;
        };
        self.target = Some(node);
        if let Err(e) = ctx.doc.rig.editable_form(node, ctx.rig.blend_shape) {
            ctx.report(e.to_string());
            self.target = None;
            return;
        }
        let radius = ctx.rig.radius;
        let pick = pick_radius(ctx);
        let single = event.modifiers.alt;
        let gesture = {
            let rig = &ctx.doc.rig;
            let eval = Evaluator::new(rig);
            match node {
                RigNode::Mesh(layer) => rig.mesh(layer).map(|mesh| {
                    Gesture::Points(Self::brush(
                        &eval.mesh_pose(mesh).positions,
                        p,
                        radius,
                        pick,
                        single,
                    ))
                }),
                RigNode::Deformer(id) => match rig.deformer(id).map(|d| &d.kind) {
                    Some(DeformerKind::Warp(_)) => {
                        let handles = eval.deformer_handles(id);
                        Some(Gesture::Points(Self::brush(&handles, p, radius, pick, single)))
                    }
                    Some(DeformerKind::Rotation(_)) => {
                        let pivot = eval.deformer_handles(id).first().copied().unwrap_or(p);
                        let grip = if pivot.distance(p) <= pick * 1.5 {
                            RotationGrip::Pivot
                        } else if event.modifiers.shift {
                            RotationGrip::Scale
                        } else {
                            RotationGrip::Turn
                        };
                        Some(Gesture::Rotation(grip))
                    }
                    None => None,
                },
                RigNode::Bone(_) => Some(if event.modifiers.ctrl {
                    Gesture::BoneMove
                } else {
                    Gesture::BoneRotate
                }),
            }
        };
        if matches!(&gesture, Some(Gesture::Points(w)) if w.is_empty()) {
            self.gesture = None;
            return;
        }
        self.gesture = gesture;
        self.before = Some(ctx.doc.rig.clone());
    }

    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let p = event.sample.position;
        if p.distance(self.start) > pick_radius(ctx) * 0.3 {
            self.moved = true;
        }
        if self.gesture.is_none() || !self.moved {
            return;
        }
        let delta = p - self.last;
        if let Err(e) = self.apply_delta(ctx, p, delta) {
            ctx.report(e.to_string());
        }
        self.last = p;
    }

    fn pointer_up(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let before = self.before.take();
        if self.moved {
            if let Some(before) = before {
                record(ctx, "Deform", before);
            }
        } else {
            // A click selects whatever is under the pointer.
            let picked = pick(ctx.doc, event.sample.position, pick_radius(ctx));
            if picked != ctx.rig.selection {
                ctx.rig.selection = picked;
                ctx.rig.blend_shape = None;
            }
            if let Some(RigNode::Mesh(layer)) = picked {
                ctx.doc.set_active_layer(layer);
            }
        }
        self.gesture = None;
        self.moved = false;
    }

    fn cancel(&mut self, ctx: &mut ToolContext) {
        if let Some(before) = self.before.take() {
            restore(ctx, before);
        }
        self.gesture = None;
        self.moved = false;
    }

    fn is_active(&self) -> bool {
        self.gesture.is_some()
    }
}

// ------------------------------------------------------------------ bone

/// Drag out bones.
#[derive(Default)]
pub struct BoneTool {
    start: Option<Vec2>,
    parent: Option<BoneId>,
    current: Vec2,
}

impl BoneTool {
    /// A bone tool with no gesture in progress.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Tool for BoneTool {
    fn id(&self) -> ToolId {
        ToolId::Bone
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let p = event.sample.position;
        let radius = pick_radius(ctx);
        let eval = Evaluator::new(&ctx.doc.rig);
        // Starting on a tail chains the new bone to that bone.
        let mut best: Option<(f32, BoneId, Vec2)> = None;
        for bone in &ctx.doc.rig.bones {
            if let Some(tail) = eval.skeleton().tail(bone.id) {
                let d = tail.distance(p);
                if d <= radius && best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, bone.id, tail));
                }
            }
        }
        let (start, parent) = match best {
            Some((_, id, tail)) => (tail, Some(id)),
            None => (p, None),
        };
        self.start = Some(start);
        self.parent = parent;
        self.current = p;
    }

    fn pointer_move(&mut self, _ctx: &mut ToolContext, event: &ToolEvent) {
        self.current = event.sample.position;
    }

    fn pointer_up(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let Some(start) = self.start.take() else { return };
        let end = event.sample.position;
        if start.distance(end) < pick_radius(ctx) {
            // A click selects a bone instead.
            if let Some(node @ RigNode::Bone(_)) = pick(ctx.doc, end, pick_radius(ctx)) {
                ctx.rig.selection = Some(node);
            }
            return;
        }
        // New bones are drawn at the rest pose; a posed parent would put the
        // child somewhere else once the pose resets.
        let id = ctx.doc.ids.bone();
        let mut rig = ctx.doc.rig.clone();
        let mut bone = Bone::new(id, format!("Bone {}", rig.bones.len() + 1), start, end);
        bone.parent = self.parent;
        rig.bones.push(bone);
        let command = SetRigCommand::new("Add bone", rig);
        match ctx.history.execute(ctx.doc, Box::new(command)) {
            Ok(()) => ctx.rig.selection = Some(RigNode::Bone(id)),
            Err(e) => ctx.report(e.to_string()),
        }
    }

    fn cancel(&mut self, _ctx: &mut ToolContext) {
        self.start = None;
    }

    fn preview(&self) -> Option<ToolPreview> {
        self.start.map(|s| ToolPreview::Line(s, self.current))
    }

    fn is_active(&self) -> bool {
        self.start.is_some()
    }
}
