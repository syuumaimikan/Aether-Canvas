//! The rig: everything that makes a document move.
//!
//! [`Rig`] owns parameter definitions and their current values, the deformer
//! hierarchy, the skeleton, the art meshes bound to layers, physics, drivers,
//! motions, expressions and procedural behaviours. It is plain data — it
//! serialises into the project manifest and is edited through undoable
//! commands like the rest of the document.
//!
//! Evaluation is a pure function of the rig and a set of parameter values
//! ([`Evaluator`]): the same values always give the same pose, which is what
//! makes export, the canvas and a runtime player agree. Time-dependent
//! systems (physics, jiggle, blinking, motion playback) live in
//! [`RigRuntime`](crate::runtime::RigRuntime) and only ever communicate by
//! producing parameter values and per-vertex offsets.

use crate::behaviour::Behaviours;
use crate::deformer::{Deformer, DeformerKind, DeformerMap, DeformerState, RotationForm, WarpForm};
use crate::driver::Driver;
use crate::keyform::{AxisInput, GridOps, KeyAxis, ParamSource};
use crate::mesh::{ArtMesh, MeshForm};
use crate::motion::{Expression, Motion};
use crate::param::{ParamValues, Parameter, StandardParam};
use crate::physics::PhysicsGroup;
use crate::pose::{MeshPose, RigPose};
use crate::skeleton::{self, Bone, BoneForm, SkeletonPose};
use aether_core::id::IdGenerator;
use aether_core::math::Vec2;
use aether_core::{AetherError, BoneId, DeformerId, LayerId, ParameterId, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a mesh or deformer is attached to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NodeRef {
    /// A warp or rotation deformer.
    Deformer(DeformerId),
    /// A bone, followed rigidly.
    Bone(BoneId),
}

/// Any object in the rig hierarchy, for selection and editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RigNode {
    /// The mesh bound to a layer.
    Mesh(LayerId),
    /// A deformer.
    Deformer(DeformerId),
    /// A bone.
    Bone(BoneId),
}

impl From<NodeRef> for RigNode {
    fn from(n: NodeRef) -> Self {
        match n {
            NodeRef::Deformer(id) => RigNode::Deformer(id),
            NodeRef::Bone(id) => RigNode::Bone(id),
        }
    }
}

/// Transient simulation output. Never saved: it is recomputed by the runtime.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dynamics {
    /// Parameter values after motions, behaviours, drivers and physics, when a
    /// runtime is active. `None` means "use the rig's own values".
    pub values: Option<ParamValues>,
    /// Per-vertex offsets from jiggle, added after all deformation.
    pub offsets: BTreeMap<LayerId, Vec<Vec2>>,
}

/// A complete rig.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rig {
    /// Parameter definitions, in panel order.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// Current (authored) parameter values; missing entries are at default.
    #[serde(default)]
    pub values: ParamValues,
    /// Deformers, parents before children is not required.
    #[serde(default)]
    pub deformers: Vec<Deformer>,
    /// The skeleton.
    #[serde(default)]
    pub bones: Vec<Bone>,
    /// Meshes bound to raster layers.
    #[serde(default)]
    pub meshes: Vec<ArtMesh>,
    /// Pendulum physics groups.
    #[serde(default)]
    pub physics: Vec<PhysicsGroup>,
    /// Parameter drivers.
    #[serde(default)]
    pub drivers: Vec<Driver>,
    /// Animation clips.
    #[serde(default)]
    pub motions: Vec<Motion>,
    /// Facial expression presets.
    #[serde(default)]
    pub expressions: Vec<Expression>,
    /// Procedural behaviours (blinking, breathing, look-at, lip sync).
    #[serde(default)]
    pub behaviours: Behaviours,
    /// Simulation output; not saved.
    #[serde(skip)]
    pub dynamics: Dynamics,
}

/// Parameter values resolved against their definitions.
#[derive(Clone, Debug, Default)]
pub struct ResolvedParams {
    map: BTreeMap<ParameterId, AxisInput>,
}

impl ResolvedParams {
    /// Resolve `values` (missing = default, clamped or wrapped into range).
    pub fn new(parameters: &[Parameter], values: &ParamValues) -> Self {
        let map = parameters
            .iter()
            .map(|p| {
                let value = values.get(&p.id).copied().unwrap_or(p.default);
                (
                    p.id,
                    AxisInput {
                        value: p.clamp(value),
                        cycle: p.cycle(),
                    },
                )
            })
            .collect();
        Self { map }
    }

    /// Current value of `id`.
    pub fn value(&self, id: ParameterId) -> Option<f32> {
        self.map.get(&id).map(|s| s.value)
    }
}

impl ParamSource for ResolvedParams {
    fn sample(&self, id: ParameterId) -> Option<AxisInput> {
        self.map.get(&id).copied()
    }
}

impl Rig {
    /// An empty rig.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the rig deforms nothing, so rendering can skip it entirely.
    pub fn is_inert(&self) -> bool {
        self.meshes.is_empty()
    }

    // ------------------------------------------------------------ parameters

    /// A parameter by id.
    pub fn parameter(&self, id: ParameterId) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.id == id)
    }

    /// A parameter by id, mutably.
    pub fn parameter_mut(&mut self, id: ParameterId) -> Option<&mut Parameter> {
        self.parameters.iter_mut().find(|p| p.id == id)
    }

    /// A parameter by name.
    pub fn parameter_named(&self, name: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.name == name)
    }

    /// Add a parameter; names must be unique because expressions use them.
    pub fn add_parameter(&mut self, parameter: Parameter) -> Result<ParameterId> {
        if parameter.name.trim().is_empty() {
            return Err(AetherError::rig("a parameter needs a name"));
        }
        if self.parameter_named(&parameter.name).is_some() {
            return Err(AetherError::rig(format!(
                "a parameter called '{}' already exists",
                parameter.name
            )));
        }
        if self.parameter(parameter.id).is_some() {
            return Err(AetherError::rig("a parameter with that id already exists"));
        }
        let id = parameter.id;
        self.parameters.push(parameter);
        Ok(id)
    }

    /// Add every standard parameter that is not already present.
    pub fn add_standard_parameters(&mut self, ids: &IdGenerator) -> Vec<ParameterId> {
        let mut added = Vec::new();
        for standard in StandardParam::ALL {
            if self.parameter_named(standard.name).is_none() {
                let id = ids.parameter();
                if self.add_parameter(standard.instantiate(id)).is_ok() {
                    added.push(id);
                }
            }
        }
        added
    }

    /// Delete a parameter and every use of it. Keyforms keep the slice at the
    /// parameter's default, so the rest pose is unchanged.
    pub fn remove_parameter(&mut self, id: ParameterId) -> Result<()> {
        let parameter = self
            .parameter(id)
            .cloned()
            .ok_or_else(|| AetherError::rig("no such parameter"))?;
        let keep = parameter.default;
        for mesh in &mut self.meshes {
            mesh.keyforms.detach_param(id, keep);
            mesh.blend_shapes.retain(|s| !s.grid.uses(id));
        }
        for deformer in &mut self.deformers {
            match &mut deformer.kind {
                DeformerKind::Warp(w) => {
                    w.keyforms.detach_param(id, keep);
                    w.blend_shapes.retain(|s| !s.grid.uses(id));
                }
                DeformerKind::Rotation(r) => {
                    r.keyforms.detach_param(id, keep);
                    r.blend_shapes.retain(|s| !s.grid.uses(id));
                }
            }
        }
        for bone in &mut self.bones {
            bone.keyforms.detach_param(id, keep);
        }
        self.drivers.retain(|d| d.target != id);
        for group in &mut self.physics {
            group.inputs.retain(|i| i.param != id);
            group.outputs.retain(|o| o.param != id);
        }
        for motion in &mut self.motions {
            motion.tracks.retain(|t| t.param != id);
        }
        for expression in &mut self.expressions {
            expression.entries.retain(|e| e.param != id);
        }
        self.behaviours.forget_parameter(id);
        self.values.remove(&id);
        self.parameters.retain(|p| p.id != id);
        Ok(())
    }

    /// Current authored value of a parameter (default when unset).
    pub fn value(&self, id: ParameterId) -> f32 {
        match self.parameter(id) {
            Some(p) => p.clamp(self.values.get(&id).copied().unwrap_or(p.default)),
            None => 0.0,
        }
    }

    /// The value actually used for rendering: the runtime's output when a
    /// simulation is running, otherwise the authored value.
    pub fn effective_value(&self, id: ParameterId) -> f32 {
        match (&self.dynamics.values, self.parameter(id)) {
            (Some(values), Some(p)) => p.clamp(values.get(&id).copied().unwrap_or(p.default)),
            _ => self.value(id),
        }
    }

    /// Set an authored value (clamped into range). Returns true if it changed.
    pub fn set_value(&mut self, id: ParameterId, value: f32) -> bool {
        let Some(p) = self.parameter(id) else {
            return false;
        };
        let value = p.clamp(value);
        let old = self.value(id);
        if (old - value).abs() < 1e-7 {
            return false;
        }
        if (value - p.default).abs() < 1e-7 {
            self.values.remove(&id);
        } else {
            self.values.insert(id, value);
        }
        true
    }

    /// Put every parameter back at its default.
    pub fn reset_values(&mut self) {
        self.values.clear();
    }

    /// The values rendering uses.
    pub fn effective_values(&self) -> &ParamValues {
        self.dynamics.values.as_ref().unwrap_or(&self.values)
    }

    /// Resolve the values rendering uses.
    pub fn resolved(&self) -> ResolvedParams {
        ResolvedParams::new(&self.parameters, self.effective_values())
    }

    /// Resolve the authored values (what editing refers to).
    pub fn resolved_authored(&self) -> ResolvedParams {
        ResolvedParams::new(&self.parameters, &self.values)
    }

    // --------------------------------------------------------------- objects

    /// The mesh on `layer`.
    pub fn mesh(&self, layer: LayerId) -> Option<&ArtMesh> {
        self.meshes.iter().find(|m| m.layer == layer)
    }

    /// The mesh on `layer`, mutably.
    pub fn mesh_mut(&mut self, layer: LayerId) -> Option<&mut ArtMesh> {
        self.meshes.iter_mut().find(|m| m.layer == layer)
    }

    /// Bind a mesh to its layer, replacing any existing mesh there.
    pub fn set_mesh(&mut self, mesh: ArtMesh) {
        match self.meshes.iter_mut().find(|m| m.layer == mesh.layer) {
            Some(existing) => *existing = mesh,
            None => self.meshes.push(mesh),
        }
    }

    /// Unbind the mesh from `layer`.
    pub fn remove_mesh(&mut self, layer: LayerId) -> Option<ArtMesh> {
        let index = self.meshes.iter().position(|m| m.layer == layer)?;
        let mesh = self.meshes.remove(index);
        for other in &mut self.meshes {
            other.glue.retain(|g| g.other != layer);
        }
        Some(mesh)
    }

    /// A deformer by id.
    pub fn deformer(&self, id: DeformerId) -> Option<&Deformer> {
        self.deformers.iter().find(|d| d.id == id)
    }

    /// A deformer by id, mutably.
    pub fn deformer_mut(&mut self, id: DeformerId) -> Option<&mut Deformer> {
        self.deformers.iter_mut().find(|d| d.id == id)
    }

    /// Delete a deformer. Its children are re-attached to its parent, and
    /// keep their rest geometry.
    pub fn remove_deformer(&mut self, id: DeformerId) -> Result<Deformer> {
        let index = self
            .deformers
            .iter()
            .position(|d| d.id == id)
            .ok_or_else(|| AetherError::rig("no such deformer"))?;
        let removed = self.deformers.remove(index);
        let target = NodeRef::Deformer(id);
        for mesh in &mut self.meshes {
            if mesh.parent == Some(target) {
                mesh.parent = removed.parent;
            }
        }
        for deformer in &mut self.deformers {
            if deformer.parent == Some(target) {
                deformer.parent = removed.parent;
            }
        }
        Ok(removed)
    }

    /// A bone by id.
    pub fn bone(&self, id: BoneId) -> Option<&Bone> {
        self.bones.iter().find(|b| b.id == id)
    }

    /// A bone by id, mutably.
    pub fn bone_mut(&mut self, id: BoneId) -> Option<&mut Bone> {
        self.bones.iter_mut().find(|b| b.id == id)
    }

    /// Delete a bone: children move to its parent, skin weights on it are
    /// dropped (and renormalised), constraints targeting it are removed.
    pub fn remove_bone(&mut self, id: BoneId) -> Result<Bone> {
        let index = self
            .bones
            .iter()
            .position(|b| b.id == id)
            .ok_or_else(|| AetherError::rig("no such bone"))?;
        let removed = self.bones.remove(index);
        for bone in &mut self.bones {
            if bone.parent == Some(id) {
                bone.parent = removed.parent;
            }
            if bone.ik.as_ref().map(|ik| ik.target == id).unwrap_or(false) {
                bone.ik = None;
            }
        }
        let fallback = removed.parent.map(NodeRef::Bone);
        for mesh in &mut self.meshes {
            if mesh.parent == Some(NodeRef::Bone(id)) {
                mesh.parent = fallback;
            }
            if let Some(skin) = &mut mesh.skin {
                if let Some(slot) = skin.bones.iter().position(|b| *b == id) {
                    let n = skin.bones.len();
                    let vertices = skin.weights.len() / n.max(1);
                    let mut weights = Vec::with_capacity(vertices * (n - 1));
                    for v in 0..vertices {
                        for b in 0..n {
                            if b != slot {
                                weights.push(skin.weights[v * n + b]);
                            }
                        }
                    }
                    skin.bones.remove(slot);
                    skin.weights = weights;
                    skin.normalize();
                }
            }
        }
        for deformer in &mut self.deformers {
            if deformer.parent == Some(NodeRef::Bone(id)) {
                deformer.parent = fallback;
            }
        }
        Ok(removed)
    }

    /// True when `node` exists.
    pub fn contains(&self, node: RigNode) -> bool {
        match node {
            RigNode::Mesh(layer) => self.mesh(layer).is_some(),
            RigNode::Deformer(id) => self.deformer(id).is_some(),
            RigNode::Bone(id) => self.bone(id).is_some(),
        }
    }

    /// The parent of a mesh or deformer (bones report their parent bone).
    pub fn parent_of(&self, node: RigNode) -> Option<NodeRef> {
        match node {
            RigNode::Mesh(layer) => self.mesh(layer).and_then(|m| m.parent),
            RigNode::Deformer(id) => self.deformer(id).and_then(|d| d.parent),
            RigNode::Bone(id) => self.bone(id).and_then(|b| b.parent).map(NodeRef::Bone),
        }
    }

    /// Re-parent a mesh or deformer, refusing cycles and missing parents.
    pub fn set_parent(&mut self, node: RigNode, parent: Option<NodeRef>) -> Result<()> {
        if let Some(p) = parent {
            if !self.contains(p.into()) {
                return Err(AetherError::rig("the new parent does not exist"));
            }
            if RigNode::from(p) == node {
                return Err(AetherError::rig("an object cannot be its own parent"));
            }
            // Walk up from the new parent; meeting `node` would close a loop.
            let mut current = Some(p);
            let mut steps = 0;
            while let Some(c) = current {
                if RigNode::from(c) == node {
                    return Err(AetherError::rig("that would put an object inside itself"));
                }
                steps += 1;
                if steps > self.deformers.len() + self.bones.len() + 1 {
                    break;
                }
                current = self.parent_of(c.into());
            }
        }
        match node {
            RigNode::Mesh(layer) => {
                self.mesh_mut(layer)
                    .ok_or_else(|| AetherError::rig("no such mesh"))?
                    .parent = parent;
            }
            RigNode::Deformer(id) => {
                self.deformer_mut(id)
                    .ok_or_else(|| AetherError::rig("no such deformer"))?
                    .parent = parent;
            }
            RigNode::Bone(id) => {
                let parent_bone = match parent {
                    None => None,
                    Some(NodeRef::Bone(b)) => Some(b),
                    Some(NodeRef::Deformer(_)) => {
                        return Err(AetherError::rig("bones can only be parented to bones"));
                    }
                };
                self.bone_mut(id)
                    .ok_or_else(|| AetherError::rig("no such bone"))?
                    .parent = parent_bone;
            }
        }
        Ok(())
    }

    /// Children of a hierarchy node (or the top level for `None`), meshes last.
    pub fn children_of(&self, parent: Option<NodeRef>) -> Vec<RigNode> {
        let mut out: Vec<RigNode> = Vec::new();
        if let Some(NodeRef::Bone(id)) = parent {
            out.extend(
                self.bones
                    .iter()
                    .filter(|b| b.parent == Some(id))
                    .map(|b| RigNode::Bone(b.id)),
            );
        } else if parent.is_none() {
            out.extend(
                self.bones
                    .iter()
                    .filter(|b| b.parent.is_none() || self.bone(b.parent.unwrap_or(BoneId::NONE)).is_none())
                    .map(|b| RigNode::Bone(b.id)),
            );
        }
        out.extend(
            self.deformers
                .iter()
                .filter(|d| d.parent == parent || (parent.is_none() && self.dangling(d.parent)))
                .map(|d| RigNode::Deformer(d.id)),
        );
        out.extend(
            self.meshes
                .iter()
                .filter(|m| m.parent == parent || (parent.is_none() && self.dangling(m.parent)))
                .map(|m| RigNode::Mesh(m.layer)),
        );
        out
    }

    fn dangling(&self, parent: Option<NodeRef>) -> bool {
        match parent {
            Some(p) => !self.contains(p.into()),
            None => false,
        }
    }

    /// Display name of a node.
    pub fn node_name(&self, node: RigNode) -> String {
        match node {
            RigNode::Mesh(layer) => self
                .mesh(layer)
                .map(|m| {
                    if m.name.is_empty() {
                        format!("Mesh {layer}")
                    } else {
                        m.name.clone()
                    }
                })
                .unwrap_or_default(),
            RigNode::Deformer(id) => self.deformer(id).map(|d| d.name.clone()).unwrap_or_default(),
            RigNode::Bone(id) => self.bone(id).map(|b| b.name.clone()).unwrap_or_default(),
        }
    }

    // --------------------------------------------------------------- keyforms

    /// The keyform grid of `node` (or of one of its blend shapes).
    pub fn grid_mut(&mut self, node: RigNode, blend_shape: Option<usize>) -> Option<&mut dyn GridOps> {
        match node {
            RigNode::Mesh(layer) => {
                let mesh = self.mesh_mut(layer)?;
                match blend_shape {
                    None => Some(&mut mesh.keyforms),
                    Some(i) => mesh
                        .blend_shapes
                        .get_mut(i)
                        .map(|s| &mut s.grid as &mut dyn GridOps),
                }
            }
            RigNode::Deformer(id) => match &mut self.deformer_mut(id)?.kind {
                DeformerKind::Warp(w) => match blend_shape {
                    None => Some(&mut w.keyforms),
                    Some(i) => w.blend_shapes.get_mut(i).map(|s| &mut s.grid as &mut dyn GridOps),
                },
                DeformerKind::Rotation(r) => match blend_shape {
                    None => Some(&mut r.keyforms),
                    Some(i) => r.blend_shapes.get_mut(i).map(|s| &mut s.grid as &mut dyn GridOps),
                },
            },
            RigNode::Bone(id) => {
                if blend_shape.is_some() {
                    return None;
                }
                Some(&mut self.bone_mut(id)?.keyforms)
            }
        }
    }

    /// Read-only access to a node's grid.
    pub fn grid(&self, node: RigNode, blend_shape: Option<usize>) -> Option<&dyn GridOps> {
        match node {
            RigNode::Mesh(layer) => {
                let mesh = self.mesh(layer)?;
                match blend_shape {
                    None => Some(&mesh.keyforms),
                    Some(i) => mesh.blend_shapes.get(i).map(|s| &s.grid as &dyn GridOps),
                }
            }
            RigNode::Deformer(id) => match &self.deformer(id)?.kind {
                DeformerKind::Warp(w) => match blend_shape {
                    None => Some(&w.keyforms),
                    Some(i) => w.blend_shapes.get(i).map(|s| &s.grid as &dyn GridOps),
                },
                DeformerKind::Rotation(r) => match blend_shape {
                    None => Some(&r.keyforms),
                    Some(i) => r.blend_shapes.get(i).map(|s| &s.grid as &dyn GridOps),
                },
            },
            RigNode::Bone(id) => {
                if blend_shape.is_some() {
                    return None;
                }
                Some(&self.bone(id)?.keyforms)
            }
        }
    }

    /// Bind `param` to a node with keys at `keys` (e.g. min, default, max).
    pub fn bind_parameter(&mut self, node: RigNode, param: ParameterId, keys: &[f32]) -> Result<()> {
        let parameter = self
            .parameter(param)
            .cloned()
            .ok_or_else(|| AetherError::rig("no such parameter"))?;
        let keys: Vec<f32> = keys.iter().map(|k| parameter.clamp(*k)).collect();
        let axis = KeyAxis::new(param, keys)?;
        let grid = self
            .grid_mut(node, None)
            .ok_or_else(|| AetherError::rig("no such object"))?;
        grid.add_axis(axis)
    }

    /// Add a key at `value` on the node's axis for `param`, binding the
    /// parameter first if needed.
    pub fn add_key(
        &mut self,
        node: RigNode,
        blend_shape: Option<usize>,
        param: ParameterId,
        value: f32,
    ) -> Result<()> {
        let parameter = self
            .parameter(param)
            .cloned()
            .ok_or_else(|| AetherError::rig("no such parameter"))?;
        let value = parameter.clamp(value);
        let grid = self
            .grid_mut(node, blend_shape)
            .ok_or_else(|| AetherError::rig("no such object"))?;
        match grid.axes().iter().position(|a| a.param == param) {
            Some(axis) => grid.insert_key(axis, value).map(|_| ()),
            None => grid.add_axis(KeyAxis::new(param, [value])?),
        }
    }

    /// Remove the key at `value` on `param`'s axis.
    pub fn remove_key(
        &mut self,
        node: RigNode,
        blend_shape: Option<usize>,
        param: ParameterId,
        value: f32,
    ) -> Result<()> {
        let grid = self
            .grid_mut(node, blend_shape)
            .ok_or_else(|| AetherError::rig("no such object"))?;
        let axis = grid
            .axes()
            .iter()
            .position(|a| a.param == param)
            .ok_or_else(|| AetherError::rig("that parameter does not drive this object"))?;
        let key = grid.axes()[axis]
            .key_at(value)
            .ok_or_else(|| AetherError::rig("the parameter is not on a key"))?;
        grid.remove_key(axis, key)
    }

    /// Stop `param` from driving a node.
    pub fn unbind_parameter(&mut self, node: RigNode, param: ParameterId) -> Result<()> {
        let keep = self.parameter(param).map(|p| p.default).unwrap_or(0.0);
        let grid = self
            .grid_mut(node, None)
            .ok_or_else(|| AetherError::rig("no such object"))?;
        grid.detach_param(param, keep);
        Ok(())
    }

    /// The form index an edit at the current authored pose would change,
    /// with an explanation when the pose is between keys.
    pub fn editable_form(&self, node: RigNode, blend_shape: Option<usize>) -> Result<usize> {
        let grid = self
            .grid(node, blend_shape)
            .ok_or_else(|| AetherError::rig("no such object"))?;
        let source = self.resolved_authored();
        grid.form_at_keys(&source).ok_or_else(|| {
            let names: Vec<String> = grid
                .axes_off_key(&source)
                .iter()
                .map(|id| {
                    self.parameter(*id)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| id.to_string())
                })
                .collect();
            AetherError::rig(format!(
                "move {} onto a key before editing this shape",
                names.join(", ")
            ))
        })
    }

    /// The mesh form to edit at the current pose.
    pub fn mesh_form_mut(&mut self, layer: LayerId, blend_shape: Option<usize>) -> Result<&mut MeshForm> {
        let index = self.editable_form(RigNode::Mesh(layer), blend_shape)?;
        let mesh = self
            .mesh_mut(layer)
            .ok_or_else(|| AetherError::rig("no such mesh"))?;
        let grid = match blend_shape {
            None => &mut mesh.keyforms,
            Some(i) => {
                &mut mesh
                    .blend_shapes
                    .get_mut(i)
                    .ok_or_else(|| AetherError::rig("no such blend shape"))?
                    .grid
            }
        };
        grid.forms
            .get_mut(index)
            .ok_or_else(|| AetherError::rig("no such keyform"))
    }

    /// The warp form to edit at the current pose.
    pub fn warp_form_mut(&mut self, id: DeformerId, blend_shape: Option<usize>) -> Result<&mut WarpForm> {
        let index = self.editable_form(RigNode::Deformer(id), blend_shape)?;
        match &mut self
            .deformer_mut(id)
            .ok_or_else(|| AetherError::rig("no such deformer"))?
            .kind
        {
            DeformerKind::Warp(w) => {
                let grid = match blend_shape {
                    None => &mut w.keyforms,
                    Some(i) => {
                        &mut w
                            .blend_shapes
                            .get_mut(i)
                            .ok_or_else(|| AetherError::rig("no such blend shape"))?
                            .grid
                    }
                };
                grid.forms
                    .get_mut(index)
                    .ok_or_else(|| AetherError::rig("no such keyform"))
            }
            DeformerKind::Rotation(_) => Err(AetherError::rig("that deformer is not a warp")),
        }
    }

    /// The rotation form to edit at the current pose.
    pub fn rotation_form_mut(
        &mut self,
        id: DeformerId,
        blend_shape: Option<usize>,
    ) -> Result<&mut RotationForm> {
        let index = self.editable_form(RigNode::Deformer(id), blend_shape)?;
        match &mut self
            .deformer_mut(id)
            .ok_or_else(|| AetherError::rig("no such deformer"))?
            .kind
        {
            DeformerKind::Rotation(r) => {
                let grid = match blend_shape {
                    None => &mut r.keyforms,
                    Some(i) => {
                        &mut r
                            .blend_shapes
                            .get_mut(i)
                            .ok_or_else(|| AetherError::rig("no such blend shape"))?
                            .grid
                    }
                };
                grid.forms
                    .get_mut(index)
                    .ok_or_else(|| AetherError::rig("no such keyform"))
            }
            DeformerKind::Warp(_) => Err(AetherError::rig("that deformer is not a rotation")),
        }
    }

    /// The bone form to edit at the current pose.
    pub fn bone_form_mut(&mut self, id: BoneId) -> Result<&mut BoneForm> {
        let index = self.editable_form(RigNode::Bone(id), None)?;
        self.bone_mut(id)
            .ok_or_else(|| AetherError::rig("no such bone"))?
            .keyforms
            .forms
            .get_mut(index)
            .ok_or_else(|| AetherError::rig("no such keyform"))
    }

    /// Every node whose keyforms depend on `param`.
    pub fn nodes_using(&self, param: ParameterId) -> Vec<RigNode> {
        let mut out = Vec::new();
        for m in &self.meshes {
            if m.keyforms.uses(param) || m.blend_shapes.iter().any(|s| s.grid.uses(param)) {
                out.push(RigNode::Mesh(m.layer));
            }
        }
        for d in &self.deformers {
            if d.params().contains(&param) {
                out.push(RigNode::Deformer(d.id));
            }
        }
        for b in &self.bones {
            if b.keyforms.uses(param) {
                out.push(RigNode::Bone(b.id));
            }
        }
        out
    }

    // ------------------------------------------------------------- integrity

    /// Check every invariant; returns the first problem found.
    pub fn validate(&self) -> Result<()> {
        for (i, p) in self.parameters.iter().enumerate() {
            if self.parameters[i + 1..]
                .iter()
                .any(|q| q.id == p.id || q.name == p.name)
            {
                return Err(AetherError::rig(format!(
                    "parameter '{}' is defined twice",
                    p.name
                )));
            }
        }
        for mesh in &self.meshes {
            mesh.validate()?;
        }
        for deformer in &self.deformers {
            deformer.validate()?;
        }
        skeleton::validate(&self.bones)?;
        for d in &self.deformers {
            let mut current = d.parent;
            let mut steps = 0;
            while let Some(p) = current {
                if p == NodeRef::Deformer(d.id) || steps > self.deformers.len() {
                    return Err(AetherError::rig(format!(
                        "deformer '{}' is inside itself",
                        d.name
                    )));
                }
                steps += 1;
                current = self.parent_of(p.into());
            }
        }
        Ok(())
    }

    /// Repair what can be repaired after loading: drop broken meshes' bad
    /// triangles, pad arrays, break parent cycles, forget values for deleted
    /// parameters.
    pub fn repair(&mut self) {
        for p in &mut self.parameters {
            p.sanitize();
        }
        let ids: Vec<ParameterId> = self.parameters.iter().map(|p| p.id).collect();
        self.values.retain(|id, v| ids.contains(id) && v.is_finite());
        for mesh in &mut self.meshes {
            mesh.repair();
        }
        let deformer_count = self.deformers.len();
        for i in 0..deformer_count {
            let id = self.deformers[i].id;
            let mut current = self.deformers[i].parent;
            let mut steps = 0;
            while let Some(p) = current {
                if p == NodeRef::Deformer(id) || steps > deformer_count {
                    self.deformers[i].parent = None;
                    break;
                }
                steps += 1;
                current = self.parent_of(p.into());
            }
        }
        self.deformers.retain(|d| d.validate().is_ok());
        for bone in &mut self.bones {
            if bone.keyforms.validate().is_err() {
                bone.keyforms = crate::keyform::KeyformGrid::constant(BoneForm::rest());
            }
        }
        self.dynamics = Dynamics::default();
    }

    /// Raise `ids` past every id in the rig so new objects never collide.
    pub fn reserve_ids(&self, ids: &IdGenerator) {
        let max = self
            .parameters
            .iter()
            .map(|p| p.id.raw())
            .chain(self.deformers.iter().map(|d| d.id.raw()))
            .chain(self.bones.iter().map(|b| b.id.raw()))
            .max()
            .unwrap_or(0);
        ids.reserve_at_least(max);
    }

    // ------------------------------------------------------------ evaluation

    /// Pose the rig at the values rendering uses.
    pub fn evaluate(&self) -> RigPose {
        Evaluator::new(self).pose()
    }
}

/// Evaluates a rig at one set of parameter values.
///
/// Construction solves the skeleton and every deformer once; afterwards
/// points can be mapped through any part of the hierarchy cheaply, which is
/// what both the full pose and the editing tools (converting a drag on screen
/// into a rest-space edit) need.
pub struct Evaluator<'a> {
    rig: &'a Rig,
    params: ResolvedParams,
    skeleton: SkeletonPose,
    deformers: BTreeMap<DeformerId, (usize, DeformerState)>,
    include_dynamics: bool,
}

impl<'a> Evaluator<'a> {
    /// Evaluate at the values rendering uses, including simulation output.
    pub fn new(rig: &'a Rig) -> Self {
        Self::with_params(rig, rig.resolved(), true)
    }

    /// Evaluate at explicit values, ignoring simulation offsets.
    pub fn with_values(rig: &'a Rig, values: &ParamValues) -> Self {
        Self::with_params(rig, ResolvedParams::new(&rig.parameters, values), false)
    }

    fn with_params(rig: &'a Rig, params: ResolvedParams, include_dynamics: bool) -> Self {
        let skeleton = skeleton::solve(&rig.bones, &params);
        let deformers = rig
            .deformers
            .iter()
            .enumerate()
            .map(|(i, d)| (d.id, (i, d.evaluate(&params))))
            .collect();
        Self {
            rig,
            params,
            skeleton,
            deformers,
            include_dynamics,
        }
    }

    /// The resolved parameter values.
    pub fn params(&self) -> &ResolvedParams {
        &self.params
    }

    /// The posed skeleton.
    pub fn skeleton(&self) -> &SkeletonPose {
        &self.skeleton
    }

    /// A deformer's own evaluated state.
    pub fn deformer_state(&self, id: DeformerId) -> Option<&DeformerState> {
        self.deformers.get(&id).map(|(_, s)| s)
    }

    /// Map a rest-space point through `parent` and everything above it.
    pub fn map(&self, parent: Option<NodeRef>, p: Vec2) -> Vec2 {
        let mut p = p;
        let mut node = parent;
        let mut depth = 0;
        while let Some(n) = node {
            depth += 1;
            if depth > 64 {
                break;
            }
            match n {
                NodeRef::Deformer(id) => {
                    let Some((index, state)) = self.deformers.get(&id) else {
                        break;
                    };
                    p = state.map.apply(p);
                    node = self.rig.deformers[*index].parent;
                }
                NodeRef::Bone(id) => {
                    p = self.skeleton.transform(id, p);
                    node = None;
                }
            }
        }
        p
    }

    /// Combined opacity of `parent` and its ancestors.
    pub fn chain_opacity(&self, parent: Option<NodeRef>) -> f32 {
        let mut opacity = 1.0;
        let mut node = parent;
        let mut depth = 0;
        while let Some(NodeRef::Deformer(id)) = node {
            depth += 1;
            if depth > 64 {
                break;
            }
            let Some((index, state)) = self.deformers.get(&id) else {
                break;
            };
            opacity *= state.opacity;
            node = self.rig.deformers[*index].parent;
        }
        opacity
    }

    /// The 2×2 Jacobian of [`Evaluator::map`] at `p`, by central differences:
    /// `[dx/dx, dy/dx, dx/dy, dy/dy]`.
    pub fn jacobian(&self, parent: Option<NodeRef>, p: Vec2) -> [f32; 4] {
        let h = 0.5;
        let dx =
            (self.map(parent, p + Vec2::new(h, 0.0)) - self.map(parent, p - Vec2::new(h, 0.0))) / (2.0 * h);
        let dy =
            (self.map(parent, p + Vec2::new(0.0, h)) - self.map(parent, p - Vec2::new(0.0, h))) / (2.0 * h);
        [dx.x, dx.y, dy.x, dy.y]
    }

    /// Convert a movement seen on screen at rest-space point `p` into the
    /// rest-space movement that produces it, so dragging a vertex inside a
    /// turned head moves it where the cursor goes.
    pub fn unmap_delta(&self, parent: Option<NodeRef>, p: Vec2, delta: Vec2) -> Vec2 {
        let [a, b, c, d] = self.jacobian(parent, p);
        let det = a * d - b * c;
        if det.abs() < 1e-6 {
            return delta;
        }
        Vec2::new(
            (d * delta.x - c * delta.y) / det,
            (-b * delta.x + a * delta.y) / det,
        )
    }

    /// Mesh vertices with keyforms and skinning applied, before parents.
    pub fn mesh_local(&self, mesh: &ArtMesh) -> (MeshForm, Vec<Vec2>) {
        let form = mesh.evaluate_form(&self.params);
        let mut points: Vec<Vec2> = mesh
            .vertices
            .iter()
            .zip(&form.offsets)
            .map(|(v, o)| *v + *o)
            .collect();
        if let Some(skin) = &mesh.skin {
            if !skin.bones.is_empty() {
                for (i, p) in points.iter_mut().enumerate() {
                    *p = skeleton::skin_point(&self.skeleton, skin, i, *p);
                }
            }
        }
        (form, points)
    }

    /// One mesh, fully posed (without glue, which needs every mesh).
    pub fn mesh_pose(&self, mesh: &ArtMesh) -> MeshPose {
        let (form, local) = self.mesh_local(mesh);
        let mut positions: Vec<Vec2> = local.iter().map(|p| self.map(mesh.parent, *p)).collect();
        if self.include_dynamics {
            if let Some(offsets) = self.rig.dynamics.offsets.get(&mesh.layer) {
                for (p, o) in positions.iter_mut().zip(offsets) {
                    *p += *o;
                }
            }
        }
        let opacity = (form.opacity * self.chain_opacity(mesh.parent)).clamp(0.0, 1.0);
        let moved = positions
            .iter()
            .zip(&mesh.vertices)
            .any(|(p, v)| p.distance(*v) > 1e-3);
        let rest = !moved
            && (opacity - 1.0).abs() < 1e-6
            && form.multiply.iter().all(|c| (c - 1.0).abs() < 1e-6)
            && form.screen.iter().all(|c| c.abs() < 1e-6);
        MeshPose {
            layer: mesh.layer,
            positions,
            opacity,
            multiply: form.multiply,
            screen: form.screen,
            draw_order: form.draw_order,
            rest,
        }
    }

    /// Positions of a deformer's handles in final space: the lattice for a
    /// warp (row-major), or `[pivot, arm tip]` for a rotation.
    pub fn deformer_handles(&self, id: DeformerId) -> Vec<Vec2> {
        let Some((index, state)) = self.deformers.get(&id) else {
            return Vec::new();
        };
        let parent = self.rig.deformers[*index].parent;
        match &state.map {
            DeformerMap::Warp(w) => w.points().into_iter().map(|p| self.map(parent, p)).collect(),
            DeformerMap::Rotation(r) => {
                let arm = r.pivot() + Vec2::new(40.0 * r.scale, 0.0).rotated(r.angle);
                vec![self.map(parent, r.pivot()), self.map(parent, arm)]
            }
        }
    }

    /// Pose everything.
    pub fn pose(&self) -> RigPose {
        let mut meshes: BTreeMap<LayerId, MeshPose> = self
            .rig
            .meshes
            .iter()
            .map(|m| (m.layer, self.mesh_pose(m)))
            .collect();
        // Glue pulls vertex pairs together using the unglued positions of
        // both sides, so the result does not depend on mesh order.
        let mut moves: Vec<(LayerId, usize, Vec2)> = Vec::new();
        for mesh in &self.rig.meshes {
            for glue in &mesh.glue {
                let (Some(this), Some(other)) = (meshes.get(&mesh.layer), meshes.get(&glue.other)) else {
                    continue;
                };
                let (Some(a), Some(b)) = (
                    this.positions.get(glue.vertex as usize),
                    other.positions.get(glue.other_vertex as usize),
                ) else {
                    continue;
                };
                moves.push((
                    mesh.layer,
                    glue.vertex as usize,
                    (*b - *a) * glue.strength.clamp(0.0, 1.0),
                ));
            }
        }
        for (layer, vertex, delta) in moves {
            if let Some(pose) = meshes.get_mut(&layer) {
                if let Some(p) = pose.positions.get_mut(vertex) {
                    *p += delta;
                    if delta.length_squared() > 1e-8 {
                        pose.rest = false;
                    }
                }
            }
        }
        RigPose { meshes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deformer::Deformer;
    use aether_core::math::{vec2, Rect};

    fn rig_with_mesh() -> (Rig, IdGenerator) {
        let ids = IdGenerator::new();
        let mut rig = Rig::new();
        rig.add_standard_parameters(&ids);
        let mesh = ArtMesh::quad(
            LayerId(500),
            Rect::from_corners(vec2(0.0, 0.0), vec2(100.0, 100.0)),
        );
        rig.set_mesh(mesh);
        (rig, ids)
    }

    fn param(rig: &Rig, name: &str) -> ParameterId {
        rig.parameter_named(name).expect("parameter").id
    }

    #[test]
    fn standard_parameters_are_added_once() {
        let (mut rig, ids) = rig_with_mesh();
        let count = rig.parameters.len();
        assert_eq!(count, StandardParam::ALL.len());
        assert!(rig.add_standard_parameters(&ids).is_empty());
        let dup = Parameter::new(ids.parameter(), "AngleX", 0.0, 1.0, 0.0);
        assert!(rig.add_parameter(dup).is_err(), "names must be unique");
    }

    #[test]
    fn values_clamp_and_default() {
        let (mut rig, _) = rig_with_mesh();
        let x = param(&rig, "AngleX");
        assert_eq!(rig.value(x), 0.0);
        assert!(rig.set_value(x, 100.0));
        assert_eq!(rig.value(x), 30.0);
        assert!(rig.set_value(x, 0.0));
        assert!(rig.values.is_empty(), "defaults are not stored");
    }

    #[test]
    fn an_unkeyed_mesh_poses_at_rest() {
        let (rig, _) = rig_with_mesh();
        let pose = rig.evaluate();
        let mesh = &pose.meshes[&LayerId(500)];
        assert!(mesh.rest);
    }

    #[test]
    fn editing_requires_the_pose_to_be_on_a_key() {
        let (mut rig, _) = rig_with_mesh();
        let x = param(&rig, "AngleX");
        let node = RigNode::Mesh(LayerId(500));
        rig.bind_parameter(node, x, &[-30.0, 0.0, 30.0]).expect("bind");
        rig.set_value(x, 30.0);
        rig.mesh_form_mut(LayerId(500), None)
            .expect("on a key")
            .offsets
            .iter_mut()
            .for_each(|o| *o = vec2(10.0, 0.0));
        rig.set_value(x, 15.0);
        let err = rig.mesh_form_mut(LayerId(500), None).expect_err("between keys");
        assert!(err.to_string().contains("AngleX"), "unhelpful: {err}");
        let pose = rig.evaluate();
        let p = pose.meshes[&LayerId(500)].positions[0];
        assert!((p.x - 5.0).abs() < 1e-4, "halfway to the key: {p:?}");
    }

    #[test]
    fn meshes_inside_deformers_follow_them() {
        let (mut rig, ids) = rig_with_mesh();
        let x = param(&rig, "AngleX");
        let d = ids.deformer();
        rig.deformers
            .push(Deformer::rotation(d, "Head", vec2(50.0, 50.0)));
        rig.set_parent(RigNode::Mesh(LayerId(500)), Some(NodeRef::Deformer(d)))
            .expect("parent");
        rig.bind_parameter(RigNode::Deformer(d), x, &[0.0, 30.0])
            .expect("bind");
        rig.set_value(x, 30.0);
        rig.rotation_form_mut(d, None).expect("on key").angle = 180.0;
        let pose = rig.evaluate();
        let p = pose.meshes[&LayerId(500)].positions[0];
        assert!(
            p.distance(vec2(100.0, 100.0)) < 1e-3,
            "rotated half a turn: {p:?}"
        );
    }

    #[test]
    fn deformer_opacity_multiplies_into_children() {
        let (mut rig, ids) = rig_with_mesh();
        let d = ids.deformer();
        rig.deformers.push(Deformer::warp(
            d,
            "Fade",
            Rect::from_corners(vec2(0.0, 0.0), vec2(100.0, 100.0)),
            2,
            2,
        ));
        rig.set_parent(RigNode::Mesh(LayerId(500)), Some(NodeRef::Deformer(d)))
            .expect("parent");
        rig.warp_form_mut(d, None).expect("form").opacity = 0.25;
        let pose = rig.evaluate();
        assert!((pose.meshes[&LayerId(500)].opacity - 0.25).abs() < 1e-6);
        assert!(!pose.meshes[&LayerId(500)].rest);
    }

    #[test]
    fn parent_cycles_are_refused() {
        let (mut rig, ids) = rig_with_mesh();
        let a = ids.deformer();
        let b = ids.deformer();
        rig.deformers.push(Deformer::rotation(a, "a", Vec2::ZERO));
        rig.deformers.push(Deformer::rotation(b, "b", Vec2::ZERO));
        rig.set_parent(RigNode::Deformer(b), Some(NodeRef::Deformer(a)))
            .expect("parent");
        assert!(rig
            .set_parent(RigNode::Deformer(a), Some(NodeRef::Deformer(b)))
            .is_err());
        assert!(rig
            .set_parent(RigNode::Deformer(a), Some(NodeRef::Deformer(a)))
            .is_err());
        rig.validate().expect("still valid");
    }

    #[test]
    fn removing_a_parameter_keeps_the_rest_pose_and_cleans_up() {
        let (mut rig, _) = rig_with_mesh();
        let x = param(&rig, "AngleX");
        let node = RigNode::Mesh(LayerId(500));
        rig.bind_parameter(node, x, &[0.0, 30.0]).expect("bind");
        rig.set_value(x, 30.0);
        rig.mesh_form_mut(LayerId(500), None).expect("form").opacity = 0.0;
        rig.remove_parameter(x).expect("remove");
        rig.validate().expect("valid");
        let pose = rig.evaluate();
        assert_eq!(
            pose.meshes[&LayerId(500)].opacity,
            1.0,
            "the default slice was kept"
        );
        assert!(rig.parameter(x).is_none());
    }

    #[test]
    fn removing_a_deformer_reattaches_children() {
        let (mut rig, ids) = rig_with_mesh();
        let outer = ids.deformer();
        let inner = ids.deformer();
        rig.deformers.push(Deformer::rotation(outer, "outer", Vec2::ZERO));
        rig.deformers.push(Deformer::rotation(inner, "inner", Vec2::ZERO));
        rig.set_parent(RigNode::Deformer(inner), Some(NodeRef::Deformer(outer)))
            .expect("parent");
        rig.set_parent(RigNode::Mesh(LayerId(500)), Some(NodeRef::Deformer(inner)))
            .expect("parent");
        rig.remove_deformer(inner).expect("remove");
        assert_eq!(
            rig.mesh(LayerId(500)).and_then(|m| m.parent),
            Some(NodeRef::Deformer(outer))
        );
    }

    #[test]
    fn unmapping_a_drag_undoes_the_parent_transform() {
        let (mut rig, ids) = rig_with_mesh();
        let d = ids.deformer();
        rig.deformers.push(Deformer::rotation(d, "r", vec2(50.0, 50.0)));
        rig.rotation_form_mut(d, None).expect("form").angle = 90.0;
        rig.rotation_form_mut(d, None).expect("form").scale = 2.0;
        let eval = Evaluator::new(&rig);
        let parent = Some(NodeRef::Deformer(d));
        let rest = vec2(60.0, 50.0);
        let screen_delta = vec2(0.0, 4.0);
        let local = eval.unmap_delta(parent, rest, screen_delta);
        let moved = eval.map(parent, rest + local) - eval.map(parent, rest);
        assert!(moved.distance(screen_delta) < 1e-3, "got {moved:?}");
    }

    #[test]
    fn glue_pulls_seams_together() {
        let (mut rig, _) = rig_with_mesh();
        let mut other = ArtMesh::quad(
            LayerId(501),
            Rect::from_corners(vec2(100.0, 0.0), vec2(200.0, 100.0)),
        );
        // Move the other mesh 10px down permanently.
        other.keyforms.forms[0]
            .offsets
            .iter_mut()
            .for_each(|o| *o = vec2(0.0, 10.0));
        rig.set_mesh(other);
        // Glue vertex 1 (top-right of the first) to vertex 0 (top-left of the second).
        if let Some(mesh) = rig.mesh_mut(LayerId(500)) {
            mesh.glue.push(crate::mesh::GluePoint {
                vertex: 1,
                other: LayerId(501),
                other_vertex: 0,
                strength: 0.5,
            });
        }
        let pose = rig.evaluate();
        let p = pose.meshes[&LayerId(500)].positions[1];
        assert!((p.y - 5.0).abs() < 1e-4, "halfway to the other side: {p:?}");
    }

    #[test]
    fn rigs_round_trip_through_json() {
        let (mut rig, ids) = rig_with_mesh();
        let x = param(&rig, "AngleX");
        let d = ids.deformer();
        rig.deformers.push(Deformer::warp(
            d,
            "Face",
            Rect::from_corners(vec2(0.0, 0.0), vec2(100.0, 100.0)),
            3,
            3,
        ));
        rig.bind_parameter(RigNode::Deformer(d), x, &[-30.0, 0.0, 30.0])
            .expect("bind");
        rig.set_value(x, 12.0);
        let text = serde_json::to_string(&rig).expect("serialize");
        let back: Rig = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back, rig);
        back.validate().expect("valid");
    }
}
