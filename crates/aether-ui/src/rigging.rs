//! Rigging and animation in the editor.
//!
//! [`RigEditor`] is the UI-side state for working on a document's rig: what
//! is selected, which blend shape an edit targets, the timeline playhead, and
//! the [`RigRuntime`] that previews physics, behaviours and motions live.
//!
//! The operations live on [`EditorState`] in this module so every button in
//! the rig panels is a method a test can call. Rig edits go through
//! [`SetRigCommand`], so they undo like any other edit; posing (slider moves,
//! scrubbing) does not enter the history.
//!
//! Two modes keep editing unambiguous:
//!
//! * **Rig mode** — sliders set the pose, and the deform tools edit the
//!   keyform the pose sits on.
//! * **Animate mode** — a motion drives the pose from the playhead, and
//!   moving a slider writes a key at the playhead instead.

use crate::state::EditorState;
use aether_core::id::IdGenerator;
use aether_core::math::{Rect, Vec2};
use aether_core::{AetherError, LayerId, ParameterId, Result};
use aether_document::layer::LayerContent;
use aether_document::rig::audio::AudioClip;
use aether_document::rig::automesh::{self, AutoMeshOptions};
use aether_document::rig::generate::{self, Anchor, HeadTurnOptions};
use aether_document::rig::motion::Easing;
use aether_document::rig::{
    ArtMesh, Behaviours, Bone, DeformerKind, Driver, Expression, Jiggle, Motion, NodeRef, Parameter, Rig,
    RigNode, RigPose, RigRuntime,
};
use aether_document::SetRigCommand;
use aether_io::animation::{self, AnimationSettings};
use std::path::Path;

/// What the rig tools act on.
#[derive(Clone, Debug, PartialEq)]
pub struct RigToolState {
    /// The selected rig object.
    pub selection: Option<RigNode>,
    /// When set, deform edits change this blend shape of the selection
    /// instead of its base keyforms.
    pub blend_shape: Option<usize>,
    /// Soft-selection radius of the deform tool, in document pixels.
    pub radius: f32,
    /// Automatic mesh density (1 = a few hundred vertices per layer).
    pub mesh_density: f32,
}

impl Default for RigToolState {
    fn default() -> Self {
        Self {
            selection: None,
            blend_shape: None,
            radius: 40.0,
            mesh_density: 1.0,
        }
    }
}

/// Choices remembered by the rig panel's generator and inspector widgets.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratorSettings {
    /// Parameter for "sway".
    pub sway_param: Option<ParameterId>,
    /// Sway distance, pixels.
    pub sway_amount: f32,
    /// Fixed edge for "sway".
    pub sway_anchor: Anchor,
    /// Parameter for "close".
    pub squash_param: Option<ParameterId>,
    /// Where "close" collapses to, 0 (top) to 1 (bottom).
    pub squash_line: f32,
    /// Horizontal parameter for "head turn".
    pub turn_x: Option<ParameterId>,
    /// Vertical parameter for "head turn".
    pub turn_y: Option<ParameterId>,
    /// Mesh to glue the selection to.
    pub glue_other: Option<LayerId>,
    /// Automatic skinning falloff.
    pub skin_falloff: f32,
}

impl Default for GeneratorSettings {
    fn default() -> Self {
        Self {
            sway_param: None,
            sway_amount: 20.0,
            sway_anchor: Anchor::Top,
            squash_param: None,
            squash_line: 0.65,
            turn_x: None,
            turn_y: None,
            glue_other: None,
            skin_falloff: 2.5,
        }
    }
}

/// The kinds of animation export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    /// Animated GIF.
    Gif,
    /// Animated PNG: full colour and soft transparency.
    Apng,
    /// Numbered PNG files.
    PngSequence,
    /// One PNG grid plus a JSON atlas.
    SpriteSheet,
}

/// UI state for rigging and animation.
#[derive(Clone, Debug)]
pub struct RigEditor {
    /// Selection and tool settings.
    pub tool: RigToolState,
    /// Live preview of physics, behaviours, drivers and motions.
    pub runtime: RigRuntime,
    /// Run physics, jiggle and behaviours in the editor.
    pub simulate: bool,
    /// Animate mode: the selected motion drives the pose.
    pub animate: bool,
    /// The playhead is advancing.
    pub playing: bool,
    /// Motion shown in the timeline.
    pub motion: Option<usize>,
    /// Playhead, seconds.
    pub playhead: f32,
    /// In animate mode, slider moves write keys.
    pub auto_key: bool,
    /// Look-at follows the pointer over the canvas.
    pub follow_pointer: bool,
    /// Show rigged layers at rest (the mesh tool edits rest positions).
    pub rest_view: bool,
    /// Draw the selected mesh's wireframe.
    pub show_mesh: bool,
    /// Draw deformer lattices and pivots.
    pub show_deformers: bool,
    /// Draw bones.
    pub show_bones: bool,
    /// Settings for "generate head turn".
    pub head_turn: HeadTurnOptions,
    /// Animation export settings.
    pub export: AnimationSettings,
    /// Loaded audio for lip sync: file name and samples.
    pub audio: Option<(String, AudioClip)>,
    /// Lip-sync level gain.
    pub lip_gain: f32,
    /// Selected keyframe: parameter and key index.
    pub selected_key: Option<(ParameterId, usize)>,
    /// The pose last drawn, for incremental redraws.
    pub last_pose: Option<RigPose>,
    /// A parameter definition being edited in the parameter dialog.
    pub editing_parameter: Option<Parameter>,
    /// A driver expression being edited: target and source text.
    pub editing_driver: Option<(ParameterId, String)>,
    /// Name typed for new expressions and motions.
    pub name_draft: String,
    /// Generator and inspector choices.
    pub generator: GeneratorSettings,
}

impl Default for RigEditor {
    fn default() -> Self {
        Self {
            tool: RigToolState::default(),
            runtime: RigRuntime::new(),
            simulate: false,
            animate: false,
            playing: false,
            motion: None,
            playhead: 0.0,
            auto_key: true,
            follow_pointer: false,
            rest_view: false,
            show_mesh: true,
            show_deformers: true,
            show_bones: true,
            head_turn: HeadTurnOptions::default(),
            export: AnimationSettings::default(),
            audio: None,
            lip_gain: 1.2,
            selected_key: None,
            last_pose: None,
            editing_parameter: None,
            editing_driver: None,
            name_draft: String::new(),
            generator: GeneratorSettings::default(),
        }
    }
}

impl RigEditor {
    /// The runtime is producing values this frame.
    pub fn is_live(&self, rig: &Rig) -> bool {
        self.simulate || self.playing || (self.animate && self.motion.is_some()) || !rig.drivers.is_empty()
    }
}

impl EditorState {
    // ------------------------------------------------------------ plumbing

    /// Apply `edit` to a copy of the rig and record it as one undoable step.
    ///
    /// `coalesce` merges consecutive edits with the same key (slider drags).
    /// The edit is validated first, so a broken rig never enters the history.
    pub fn edit_rig(
        &mut self,
        label: &str,
        coalesce: Option<String>,
        edit: impl FnOnce(&mut Rig, &IdGenerator) -> Result<()>,
    ) -> Result<()> {
        let mut rig = self.doc.rig.clone();
        edit(&mut rig, &self.doc.ids)?;
        rig.validate()?;
        let mut command = SetRigCommand::new(label, rig);
        if let Some(key) = coalesce {
            command = command.coalescing(key);
        }
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Advance the rig preview by `dt` seconds and mark whatever moved for
    /// redraw. Returns true while something is animating, so the window keeps
    /// repainting.
    pub fn tick_rig(&mut self, dt: f32) -> bool {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        self.cache.set_deform(!self.rig.rest_view);
        if self.rig.motion.is_some_and(|m| m >= self.doc.rig.motions.len()) {
            self.rig.motion = None;
            self.rig.playing = false;
        }
        if let Some(node) = self.rig.tool.selection {
            if !self.doc.rig.contains(node) {
                self.rig.tool.selection = None;
                self.rig.tool.blend_shape = None;
            }
        }

        // Playback moves the playhead.
        if self.rig.playing {
            if let Some(motion) = self.rig.motion.and_then(|m| self.doc.rig.motions.get(m)) {
                self.rig.playhead += dt;
                if self.rig.playhead > motion.duration {
                    if motion.looping {
                        self.rig.playhead %= motion.duration.max(1e-3);
                    } else {
                        self.rig.playhead = motion.duration;
                        self.rig.playing = false;
                    }
                }
            }
        }

        let live = self.rig.is_live(&self.doc.rig);
        if live {
            let simulate = self.rig.simulate || self.rig.playing;
            let settings = &mut self.rig.runtime.settings;
            settings.physics = simulate;
            settings.jiggle = simulate;
            settings.behaviours = simulate;
            settings.drivers = true;
            settings.motions = true;
            self.rig.runtime.scrub = if self.rig.animate || self.rig.playing {
                self.rig.motion.map(|m| (m, self.rig.playhead))
            } else {
                None
            };
            let step = if simulate { dt } else { 0.0 };
            self.rig.runtime.tick(&mut self.doc.rig, step);
        } else if self.doc.rig.dynamics != Default::default() {
            self.rig.runtime.stop(&mut self.doc.rig);
        }

        // Redraw only what moved.
        if self.doc.rig.is_inert() {
            if self.rig.last_pose.take().is_some() {
                self.doc.mark_all_dirty();
            }
            return live && (self.rig.simulate || self.rig.playing);
        }
        let pose = self.doc.rig.evaluate();
        match &self.rig.last_pose {
            Some(last) => {
                let change = last.change_to(&pose);
                if !change.toggled.is_empty() || !change.reordered.is_empty() {
                    self.doc.mark_all_dirty();
                } else if !change.area.is_empty() {
                    // Layer effects can reach beyond the mesh (shadows, glow).
                    let spread = self.doc.layers.iter().any(|l| l.has_effects());
                    if spread {
                        self.doc.mark_all_dirty();
                    } else {
                        self.doc.mark_dirty(change.area);
                    }
                }
            }
            None => self.doc.mark_all_dirty(),
        }
        self.rig.last_pose = Some(pose);
        live && (self.rig.simulate || self.rig.playing)
    }

    /// Set a parameter from a slider: posing in rig mode, keying in animate
    /// mode (with auto-key on).
    pub fn set_parameter_value(&mut self, id: ParameterId, value: f32) -> Result<()> {
        if self.rig.animate && self.rig.auto_key {
            if let Some(m) = self.rig.motion {
                let time = self.snapped_playhead();
                let key = format!("key:{m}:{}:{}", id.raw(), (time * 1000.0).round());
                return self.edit_rig("Key parameter", Some(key), |rig, _| {
                    let p = rig
                        .parameter(id)
                        .cloned()
                        .ok_or_else(|| AetherError::rig("no such parameter"))?;
                    let motion = rig
                        .motions
                        .get_mut(m)
                        .ok_or_else(|| AetherError::rig("no such motion"))?;
                    motion.track_mut(id).set_key(time, p.clamp(value));
                    Ok(())
                });
            }
        }
        self.doc.rig.set_value(id, value);
        Ok(())
    }

    /// The playhead snapped to the motion's frame grid.
    pub fn snapped_playhead(&self) -> f32 {
        let fps = self
            .rig
            .motion
            .and_then(|m| self.doc.rig.motions.get(m))
            .map(|m| m.fps)
            .unwrap_or(30.0);
        (self.rig.playhead * fps).round() / fps
    }

    /// Put every parameter back to its default and stop previews.
    pub fn reset_pose(&mut self) {
        self.doc.rig.reset_values();
        self.rig.runtime.reset();
    }

    // --------------------------------------------------------------- meshes

    /// Mesh a raster layer automatically. An existing mesh is re-meshed and
    /// keeps its keyforms, skin and jiggle weights.
    pub fn mesh_layer(&mut self, layer: LayerId) -> Result<()> {
        let l = self.doc.layers.try_get(layer)?;
        let LayerContent::Raster(raster) = &l.content else {
            return Err(AetherError::rig(format!("'{}' holds no pixels to mesh", l.name)));
        };
        let bounds = automesh::opaque_bounds(&raster.pixmap, 8)
            .ok_or_else(|| AetherError::rig(format!("'{}' is empty", l.name)))?;
        let options = AutoMeshOptions::for_bounds(bounds, self.rig.tool.mesh_density);
        let generated = automesh::auto_mesh(&raster.pixmap, &options)
            .ok_or_else(|| AetherError::rig(format!("could not mesh '{}'", l.name)))?;
        let name = l.name.clone();
        self.edit_rig("Mesh layer", None, |rig, _| {
            match rig.mesh_mut(layer) {
                Some(mesh) => mesh.retopologize(generated.vertices, generated.triangles),
                None => {
                    let mut mesh = ArtMesh::new(layer, generated.vertices, generated.triangles);
                    mesh.name = name;
                    rig.set_mesh(mesh);
                }
            }
            Ok(())
        })?;
        self.rig.tool.selection = Some(RigNode::Mesh(layer));
        Ok(())
    }

    /// Mesh every non-empty raster layer inside `root` (or the whole
    /// document for `None`) that has no mesh yet. Returns how many were
    /// meshed.
    pub fn mesh_all_layers(&mut self, root: Option<LayerId>) -> Result<usize> {
        let meshes = self.missing_meshes(root);
        let count = meshes.len();
        if count > 0 {
            self.edit_rig("Mesh layers", None, |rig, _| {
                for mesh in meshes {
                    rig.set_mesh(mesh);
                }
                Ok(())
            })?;
        }
        Ok(count)
    }

    /// Generated meshes for the painted layers under `root` that lack one.
    fn missing_meshes(&self, root: Option<LayerId>) -> Vec<ArtMesh> {
        aether_document::rigging::missing_meshes(&self.doc, root, self.rig.tool.mesh_density)
    }

    /// Rig the whole document from its layer names in one undoable step:
    /// mesh every painted layer, then build deformers, keyforms, physics,
    /// behaviours and an idle motion (see
    /// [`autorig`](aether_document::rig::autorig)).
    pub fn auto_rig(&mut self) -> Result<aether_document::rig::autorig::AutoRigReport> {
        use aether_document::rig::autorig;
        let meshes = self.missing_meshes(None);
        let parts = aether_document::rigging::rig_parts(&self.doc, &meshes);
        let mut report = None;
        self.edit_rig("Auto rig", None, |rig, ids| {
            for mesh in meshes {
                rig.set_mesh(mesh);
            }
            report = Some(autorig::auto_rig(rig, ids, &parts)?);
            Ok(())
        })?;
        let report = report.unwrap_or_default();
        self.rig.tool.selection = self
            .doc
            .rig
            .deformers
            .iter()
            .find(|d| d.name == "Head")
            .map(|d| RigNode::Deformer(d.id));
        let parts: usize = report.roles.iter().map(|(_, n)| n).sum();
        self.status = format!(
            "Auto rig: {parts} parts, {} deformers, {} physics chains, {} unrecognised",
            report.deformers,
            report.physics,
            report.unrecognised.len()
        );
        Ok(report)
    }

    /// Remove a layer's mesh.
    pub fn remove_mesh(&mut self, layer: LayerId) -> Result<()> {
        self.edit_rig("Remove mesh", None, |rig, _| {
            rig.remove_mesh(layer)
                .map(|_| ())
                .ok_or_else(|| AetherError::rig("that layer has no mesh"))
        })
    }

    // ------------------------------------------------------------ hierarchy

    /// Rest-space bounds of a node (and everything inside it).
    pub fn node_bounds(&self, node: RigNode) -> Option<Rect> {
        let rig = &self.doc.rig;
        match node {
            RigNode::Mesh(layer) => rig.mesh(layer).map(|m| m.bounds()),
            RigNode::Bone(id) => rig.bone(id).map(|b| Rect::from_corners(b.head, b.tail)),
            RigNode::Deformer(id) => {
                let d = rig.deformer(id)?;
                let own = match &d.kind {
                    DeformerKind::Warp(w) => Some(w.rect),
                    DeformerKind::Rotation(r) => Some(Rect::from_corners(r.origin, r.origin)),
                };
                let mut bounds = own;
                for child in rig.children_of(Some(NodeRef::Deformer(id))) {
                    if let Some(b) = self.node_bounds(child) {
                        bounds = Some(bounds.map(|x| x.union(&b)).unwrap_or(b));
                    }
                }
                bounds
            }
        }
    }

    /// The nodes a "wrap" should act on: the selection, or every mesh of the
    /// active layer's subtree (so selecting a group wraps its parts).
    fn wrap_targets(&self) -> Vec<RigNode> {
        if let Some(node) = self.rig.tool.selection {
            if !matches!(node, RigNode::Bone(_)) {
                return vec![node];
            }
        }
        let active = self.doc.active_layer;
        self.doc
            .layers
            .subtree_ids(active)
            .into_iter()
            .filter(|id| self.doc.rig.mesh(*id).is_some())
            .map(RigNode::Mesh)
            .collect()
    }

    /// Put the selection (or the active group's meshes) in a new warp
    /// deformer.
    pub fn add_warp_deformer(&mut self, name: &str) -> Result<()> {
        let targets = self.wrap_targets();
        if targets.is_empty() {
            return Err(AetherError::rig(
                "select a mesh, a deformer or a group with meshes first",
            ));
        }
        let mut bounds: Option<Rect> = None;
        for t in &targets {
            if let Some(b) = self.node_bounds(*t) {
                bounds = Some(bounds.map(|x| x.union(&b)).unwrap_or(b));
            }
        }
        let rect = bounds
            .ok_or_else(|| AetherError::rig("nothing to wrap"))?
            .expanded(16.0);
        let parent = self.doc.rig.parent_of(targets[0]);
        let name = name.to_string();
        let mut created = None;
        self.edit_rig("Add warp deformer", None, |rig, ids| {
            let id = ids.deformer();
            let mut deformer = aether_document::rig::Deformer::warp(id, name, rect, 5, 5);
            deformer.parent = parent;
            rig.deformers.push(deformer);
            for t in &targets {
                rig.set_parent(*t, Some(NodeRef::Deformer(id)))?;
            }
            created = Some(id);
            Ok(())
        })?;
        self.rig.tool.selection = created.map(RigNode::Deformer);
        Ok(())
    }

    /// Put the selection (or the active group's meshes) in a new rotation
    /// deformer pivoting at the bottom centre of its bounds.
    pub fn add_rotation_deformer(&mut self, name: &str) -> Result<()> {
        let targets = self.wrap_targets();
        if targets.is_empty() {
            return Err(AetherError::rig(
                "select a mesh, a deformer or a group with meshes first",
            ));
        }
        let mut bounds: Option<Rect> = None;
        for t in &targets {
            if let Some(b) = self.node_bounds(*t) {
                bounds = Some(bounds.map(|x| x.union(&b)).unwrap_or(b));
            }
        }
        let b = bounds.ok_or_else(|| AetherError::rig("nothing to wrap"))?;
        let origin = Vec2::new((b.min.x + b.max.x) * 0.5, b.max.y);
        let name = name.to_string();
        let mut created = None;
        self.edit_rig("Add rotation deformer", None, |rig, ids| {
            let id = generate::wrap_in_rotation(rig, ids, &targets, name, origin)?;
            created = Some(id);
            Ok(())
        })?;
        self.rig.tool.selection = created.map(RigNode::Deformer);
        Ok(())
    }

    /// Add a bone from `head` to `tail`, parented to `parent`.
    pub fn add_bone(&mut self, head: Vec2, tail: Vec2, parent: Option<aether_core::BoneId>) -> Result<()> {
        if head.distance(tail) < 1.0 {
            return Err(AetherError::rig("drag further to make a bone"));
        }
        let mut created = None;
        self.edit_rig("Add bone", None, |rig, ids| {
            let id = ids.bone();
            let mut bone = Bone::new(id, format!("Bone {}", rig.bones.len() + 1), head, tail);
            bone.parent = parent.filter(|p| rig.bone(*p).is_some());
            rig.bones.push(bone);
            created = Some(id);
            Ok(())
        })?;
        self.rig.tool.selection = created.map(RigNode::Bone);
        Ok(())
    }

    /// Delete the selected rig object (a mesh is unbound, not the layer).
    pub fn delete_rig_node(&mut self, node: RigNode) -> Result<()> {
        self.edit_rig("Delete rig object", None, |rig, _| match node {
            RigNode::Mesh(layer) => rig
                .remove_mesh(layer)
                .map(|_| ())
                .ok_or_else(|| AetherError::rig("no such mesh")),
            RigNode::Deformer(id) => rig.remove_deformer(id).map(|_| ()),
            RigNode::Bone(id) => rig.remove_bone(id).map(|_| ()),
        })?;
        self.rig.tool.selection = None;
        self.rig.tool.blend_shape = None;
        Ok(())
    }

    /// Re-parent a node.
    pub fn set_rig_parent(&mut self, node: RigNode, parent: Option<NodeRef>) -> Result<()> {
        self.edit_rig("Change parent", None, |rig, _| rig.set_parent(node, parent))
    }

    // ------------------------------------------------------------ parameters

    /// Add a parameter with a unique name derived from `name`.
    pub fn add_parameter(&mut self, name: &str, min: f32, max: f32, default: f32) -> Result<ParameterId> {
        let mut unique = name.trim().to_string();
        if unique.is_empty() {
            unique = "Param".into();
        }
        let base = unique.clone();
        let mut n = 2;
        while self.doc.rig.parameter_named(&unique).is_some() {
            unique = format!("{base}{n}");
            n += 1;
        }
        let mut created = ParameterId::NONE;
        self.edit_rig("Add parameter", None, |rig, ids| {
            created = rig.add_parameter(Parameter::new(ids.parameter(), unique, min, max, default))?;
            Ok(())
        })?;
        Ok(created)
    }

    /// Add the standard parameter set (skipping names that exist).
    pub fn add_standard_parameters(&mut self) -> Result<usize> {
        let mut added = 0;
        self.edit_rig("Add standard parameters", None, |rig, ids| {
            added = rig.add_standard_parameters(ids).len();
            Ok(())
        })?;
        Ok(added)
    }

    /// Replace a parameter's definition (name, range, group, wrapping).
    pub fn update_parameter(&mut self, parameter: Parameter) -> Result<()> {
        let id = parameter.id;
        let key = format!("param-def:{}", id.raw());
        self.edit_rig("Edit parameter", Some(key), |rig, _| {
            if rig
                .parameters
                .iter()
                .any(|p| p.id != id && p.name == parameter.name)
            {
                return Err(AetherError::rig(format!(
                    "a parameter called '{}' already exists",
                    parameter.name
                )));
            }
            let slot = rig
                .parameter_mut(id)
                .ok_or_else(|| AetherError::rig("no such parameter"))?;
            let mut parameter = parameter;
            parameter.sanitize();
            *slot = parameter;
            Ok(())
        })
    }

    /// Delete a parameter and every use of it.
    pub fn remove_parameter(&mut self, id: ParameterId) -> Result<()> {
        self.edit_rig("Delete parameter", None, |rig, _| rig.remove_parameter(id))
    }

    /// The selection, or an explanation of why there is none.
    fn require_selection(&self) -> Result<RigNode> {
        self.rig
            .tool
            .selection
            .ok_or_else(|| AetherError::rig("select a mesh, deformer or bone first"))
    }

    /// Bind `param` to the selection with keys at its minimum, default and
    /// maximum.
    pub fn bind_three_keys(&mut self, param: ParameterId) -> Result<()> {
        let node = self.require_selection()?;
        let p = self
            .doc
            .rig
            .parameter(param)
            .cloned()
            .ok_or_else(|| AetherError::rig("no such parameter"))?;
        let mut keys = vec![p.min, p.default, p.max];
        keys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        self.edit_rig("Add keys", None, |rig, _| rig.bind_parameter(node, param, &keys))
    }

    /// Add a key for the selection at `param`'s current value.
    pub fn add_key_here(&mut self, param: ParameterId) -> Result<()> {
        let node = self.require_selection()?;
        let blend = self.rig.tool.blend_shape;
        let value = self.doc.rig.value(param);
        self.edit_rig("Add key", None, |rig, _| rig.add_key(node, blend, param, value))
    }

    /// Remove the selection's key at `param`'s current value.
    pub fn remove_key_here(&mut self, param: ParameterId) -> Result<()> {
        let node = self.require_selection()?;
        let blend = self.rig.tool.blend_shape;
        let value = self.doc.rig.value(param);
        self.edit_rig("Remove key", None, |rig, _| {
            rig.remove_key(node, blend, param, value)
        })
    }

    /// Stop `param` from driving the selection.
    pub fn unbind_parameter(&mut self, param: ParameterId) -> Result<()> {
        let node = self.require_selection()?;
        self.edit_rig("Unbind parameter", None, |rig, _| {
            rig.unbind_parameter(node, param)
        })
    }

    /// Copy the selection's keyform at the current value of `param` to the
    /// opposite key, mirrored about the selection's centre.
    pub fn mirror_key(&mut self, param: ParameterId) -> Result<()> {
        let node = self.require_selection()?;
        let axis = self
            .node_bounds(node)
            .map(|b| (b.min.x + b.max.x) * 0.5)
            .unwrap_or(self.doc.width as f32 * 0.5);
        let value = self.doc.rig.value(param);
        self.edit_rig("Mirror key", None, |rig, _| {
            generate::mirror_keyform(rig, node, param, value, axis)
        })
    }

    /// Add a blend shape on `param` to the selection and target it.
    pub fn add_blend_shape(&mut self, param: ParameterId) -> Result<()> {
        let node = self.require_selection()?;
        let p = self
            .doc
            .rig
            .parameter(param)
            .cloned()
            .ok_or_else(|| AetherError::rig("no such parameter"))?;
        let mut keys = vec![p.min, p.default, p.max];
        keys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let axis = aether_document::rig::KeyAxis::new(param, keys)?;
        let name = format!("{} shape", p.name);
        let mut index = 0;
        self.edit_rig("Add blend shape", None, |rig, _| {
            use aether_document::rig::{BlendShape, MeshForm, RotationForm, WarpForm};
            match node {
                RigNode::Mesh(layer) => {
                    let mesh = rig
                        .mesh_mut(layer)
                        .ok_or_else(|| AetherError::rig("no such mesh"))?;
                    let n = mesh.vertex_count();
                    mesh.blend_shapes
                        .push(BlendShape::new(name, axis, MeshForm::zero_delta(n)));
                    index = mesh.blend_shapes.len() - 1;
                }
                RigNode::Deformer(id) => {
                    let d = rig
                        .deformer_mut(id)
                        .ok_or_else(|| AetherError::rig("no such deformer"))?;
                    match &mut d.kind {
                        DeformerKind::Warp(w) => {
                            let zero = WarpForm {
                                offsets: vec![Vec2::ZERO; w.point_count()],
                                opacity: 0.0,
                            };
                            w.blend_shapes.push(BlendShape::new(name, axis, zero));
                            index = w.blend_shapes.len() - 1;
                        }
                        DeformerKind::Rotation(r) => {
                            let zero = RotationForm {
                                offset: Vec2::ZERO,
                                angle: 0.0,
                                scale: 0.0,
                                opacity: 0.0,
                            };
                            r.blend_shapes.push(BlendShape::new(name, axis, zero));
                            index = r.blend_shapes.len() - 1;
                        }
                    }
                }
                RigNode::Bone(_) => return Err(AetherError::rig("bones use keyforms, not blend shapes")),
            }
            Ok(())
        })?;
        self.rig.tool.blend_shape = Some(index);
        Ok(())
    }

    // ------------------------------------------------------------ generators

    /// Generate a head turn on the selected warp deformer.
    pub fn generate_head_turn(&mut self, x: ParameterId, y: Option<ParameterId>) -> Result<()> {
        let Some(RigNode::Deformer(id)) = self.rig.tool.selection else {
            return Err(AetherError::rig("select a warp deformer first"));
        };
        let options = self.rig.head_turn;
        self.edit_rig("Generate head turn", None, |rig, _| {
            generate::head_turn(rig, id, x, y, &options)
        })
    }

    /// Key a sway on the selection.
    pub fn generate_sway(&mut self, param: ParameterId, amount: f32, anchor: Anchor) -> Result<()> {
        let node = self.require_selection()?;
        self.edit_rig("Generate sway", None, |rig, _| {
            generate::sway(rig, node, param, amount, anchor)
        })
    }

    /// Key a close/squash on the selection.
    pub fn generate_squash(&mut self, param: ParameterId, line: f32) -> Result<()> {
        let node = self.require_selection()?;
        self.edit_rig("Generate close", None, |rig, _| {
            generate::squash(rig, node, param, line)
        })
    }

    /// Compute skin weights for the selected mesh from every deforming bone.
    pub fn auto_skin(&mut self, falloff: f32) -> Result<()> {
        let Some(RigNode::Mesh(layer)) = self.rig.tool.selection else {
            return Err(AetherError::rig("select a mesh first"));
        };
        if self.doc.rig.bones.iter().all(|b| !b.deform) {
            return Err(AetherError::rig("add a bone first"));
        }
        self.edit_rig("Skin to bones", None, |rig, _| {
            let bones = rig.bones.clone();
            let mesh = rig
                .mesh_mut(layer)
                .ok_or_else(|| AetherError::rig("no such mesh"))?;
            mesh.skin = Some(aether_document::rig::skeleton::auto_weights(
                &bones,
                &mesh.vertices,
                falloff,
            ));
            Ok(())
        })
    }

    /// Turn jiggle on or off for the selected mesh.
    pub fn set_jiggle(&mut self, layer: LayerId, enabled: bool) -> Result<()> {
        self.edit_rig("Jiggle", None, |rig, _| {
            let mesh = rig
                .mesh_mut(layer)
                .ok_or_else(|| AetherError::rig("no such mesh"))?;
            if enabled {
                if mesh.jiggle.is_none() {
                    mesh.jiggle = Some(Jiggle::new(mesh.vertex_count()));
                }
            } else {
                mesh.jiggle = None;
            }
            Ok(())
        })
    }

    /// Glue the vertices of `layer`'s mesh that lie within `distance` of a
    /// vertex of `other`'s mesh, both ways at half strength, so the two parts
    /// stay joined at their seam. Returns the number of glued pairs.
    pub fn auto_glue(&mut self, layer: LayerId, other: LayerId, distance: f32) -> Result<usize> {
        if layer == other {
            return Err(AetherError::rig("choose a different mesh to glue to"));
        }
        let (a, b) = match (self.doc.rig.mesh(layer), self.doc.rig.mesh(other)) {
            (Some(a), Some(b)) => (a.vertices.clone(), b.vertices.clone()),
            _ => return Err(AetherError::rig("both layers need meshes")),
        };
        let mut pairs = Vec::new();
        for (i, p) in a.iter().enumerate() {
            if let Some(j) = ArtMesh::nearest_vertex(&b, *p, distance) {
                pairs.push((i as u32, j as u32));
            }
        }
        if pairs.is_empty() {
            return Err(AetherError::rig(
                "no vertices of the two meshes are close enough to glue",
            ));
        }
        let count = pairs.len();
        self.edit_rig("Glue meshes", None, |rig, _| {
            use aether_document::rig::mesh::GluePoint;
            if let Some(mesh) = rig.mesh_mut(layer) {
                mesh.glue.retain(|g| g.other != other);
                mesh.glue.extend(pairs.iter().map(|&(i, j)| GluePoint {
                    vertex: i,
                    other,
                    other_vertex: j,
                    strength: 0.5,
                }));
            }
            if let Some(mesh) = rig.mesh_mut(other) {
                mesh.glue.retain(|g| g.other != layer);
                mesh.glue.extend(pairs.iter().map(|&(i, j)| GluePoint {
                    vertex: j,
                    other: layer,
                    other_vertex: i,
                    strength: 0.5,
                }));
            }
            Ok(())
        })?;
        Ok(count)
    }

    /// Give the selected bone a rotation parameter (−180…180 degrees) keyed
    /// so the parameter value *is* the bone's angle — direct FK control that
    /// animates on the timeline like any other parameter.
    pub fn add_bone_rotation_control(&mut self) -> Result<ParameterId> {
        let Some(RigNode::Bone(bone)) = self.rig.tool.selection else {
            return Err(AetherError::rig("select a bone first"));
        };
        let name = self
            .doc
            .rig
            .bone(bone)
            .map(|b| format!("{} Rotation", b.name))
            .unwrap_or_else(|| "Bone Rotation".into());
        let param = self.add_parameter(&name, -180.0, 180.0, 0.0)?;
        self.edit_rig("Bone rotation control", None, |rig, _| {
            rig.bind_parameter(RigNode::Bone(bone), param, &[-180.0, 0.0, 180.0])?;
            let b = rig
                .bone_mut(bone)
                .ok_or_else(|| AetherError::rig("no such bone"))?;
            let axes = b.keyforms.axes.clone();
            let a = axes.iter().position(|x| x.param == param).unwrap_or(0);
            let stride: usize = axes[..a].iter().map(|x| x.keys.len()).product();
            for (i, form) in b.keyforms.forms.iter_mut().enumerate() {
                let k = (i / stride) % 3;
                form.rotation += [-180.0, 0.0, 180.0][k];
            }
            Ok(())
        })?;
        Ok(param)
    }

    /// Hair physics and standard behaviours in one step.
    pub fn add_standard_dynamics(&mut self) -> Result<usize> {
        let mut groups = 0;
        self.edit_rig("Standard physics", None, |rig, _| {
            groups = generate::standard_physics(rig);
            if rig.behaviours == Behaviours::default() {
                rig.behaviours = Behaviours::standard(&rig.parameters);
            }
            Ok(())
        })?;
        Ok(groups)
    }

    /// Add a driver for `target`.
    pub fn add_driver(&mut self, target: ParameterId, expression: &str) -> Result<()> {
        aether_document::rig::driver::validate_expression(&self.doc.rig.parameters, expression)?;
        let expression = expression.to_string();
        self.edit_rig("Add driver", None, |rig, _| {
            rig.drivers.retain(|d| d.target != target);
            rig.drivers.push(Driver::new(target, expression));
            Ok(())
        })
    }

    /// Capture the current pose as an expression preset.
    pub fn capture_expression(&mut self, name: &str) -> Result<()> {
        let expression = Expression::capture(name, &self.doc.rig.parameters, &self.doc.rig.values);
        if expression.entries.is_empty() {
            return Err(AetherError::rig(
                "move some parameters away from their defaults first",
            ));
        }
        self.edit_rig("Capture expression", None, |rig, _| {
            rig.expressions.push(expression);
            Ok(())
        })
    }

    // -------------------------------------------------------------- motions

    /// Create a motion and show it in the timeline.
    pub fn add_motion(&mut self, name: &str) -> Result<usize> {
        let name = if name.trim().is_empty() { "Motion" } else { name };
        let name = name.to_string();
        let mut index = 0;
        self.edit_rig("Add motion", None, |rig, _| {
            rig.motions.push(Motion::new(name, 3.0, 30.0));
            index = rig.motions.len() - 1;
            Ok(())
        })?;
        self.rig.motion = Some(index);
        self.rig.playhead = 0.0;
        self.rig.animate = true;
        Ok(index)
    }

    /// Delete a motion.
    pub fn delete_motion(&mut self, index: usize) -> Result<()> {
        self.edit_rig("Delete motion", None, |rig, _| {
            if index >= rig.motions.len() {
                return Err(AetherError::rig("no such motion"));
            }
            rig.motions.remove(index);
            Ok(())
        })?;
        self.rig.motion = None;
        self.rig.animate = false;
        self.rig.playing = false;
        Ok(())
    }

    /// Key every parameter at the playhead with its current value.
    pub fn key_all_at_playhead(&mut self) -> Result<()> {
        let m = self
            .rig
            .motion
            .ok_or_else(|| AetherError::rig("choose or create a motion first"))?;
        let time = self.snapped_playhead();
        let values = self.doc.rig.effective_values().clone();
        self.edit_rig("Key all", None, |rig, _| {
            let parameters = rig.parameters.clone();
            let motion = rig
                .motions
                .get_mut(m)
                .ok_or_else(|| AetherError::rig("no such motion"))?;
            motion.key_all(&parameters, &values, time);
            Ok(())
        })
    }

    /// Delete one keyframe.
    pub fn delete_keyframe(&mut self, param: ParameterId, index: usize) -> Result<()> {
        let m = self.rig.motion.ok_or_else(|| AetherError::rig("no motion"))?;
        self.edit_rig("Delete key", None, |rig, _| {
            let motion = rig
                .motions
                .get_mut(m)
                .ok_or_else(|| AetherError::rig("no such motion"))?;
            let track = motion.track_mut(param);
            if index >= track.keys.len() {
                return Err(AetherError::rig("no such key"));
            }
            track.keys.remove(index);
            motion.tracks.retain(|t| !t.keys.is_empty());
            Ok(())
        })?;
        self.rig.selected_key = None;
        Ok(())
    }

    /// Change the easing of one keyframe.
    pub fn set_key_easing(&mut self, param: ParameterId, index: usize, easing: Easing) -> Result<()> {
        let m = self.rig.motion.ok_or_else(|| AetherError::rig("no motion"))?;
        self.edit_rig("Key easing", None, |rig, _| {
            let key = rig
                .motions
                .get_mut(m)
                .and_then(|motion| motion.tracks.iter_mut().find(|t| t.param == param))
                .and_then(|t| t.keys.get_mut(index))
                .ok_or_else(|| AetherError::rig("no such key"))?;
            key.easing = easing;
            Ok(())
        })
    }

    /// Move a keyframe in time (coalesced while dragging).
    pub fn move_keyframe(&mut self, param: ParameterId, index: usize, time: f32) -> Result<usize> {
        let m = self.rig.motion.ok_or_else(|| AetherError::rig("no motion"))?;
        let mut new_index = index;
        self.edit_rig(
            "Move key",
            Some(format!("move-key:{m}:{}", param.raw())),
            |rig, _| {
                let motion = rig
                    .motions
                    .get_mut(m)
                    .ok_or_else(|| AetherError::rig("no such motion"))?;
                let duration = motion.duration;
                let track = motion
                    .tracks
                    .iter_mut()
                    .find(|t| t.param == param)
                    .ok_or_else(|| AetherError::rig("no such track"))?;
                new_index = track
                    .move_key(index, time.clamp(0.0, duration))
                    .ok_or_else(|| AetherError::rig("no such key"))?;
                Ok(())
            },
        )?;
        self.rig.selected_key = Some((param, new_index));
        Ok(new_index)
    }

    /// Play or pause the timeline.
    pub fn toggle_playback(&mut self) {
        if self.rig.motion.is_none() && !self.doc.rig.motions.is_empty() {
            self.rig.motion = Some(0);
        }
        self.rig.playing = !self.rig.playing;
        if self.rig.playing {
            self.rig.animate = true;
        }
    }

    /// Step the playhead by whole frames.
    pub fn step_frames(&mut self, frames: i32) {
        let Some(motion) = self.rig.motion.and_then(|m| self.doc.rig.motions.get(m)) else {
            return;
        };
        let fps = motion.fps;
        let duration = motion.duration;
        self.rig.playing = false;
        self.rig.animate = true;
        let frame = (self.rig.playhead * fps).round() + frames as f32;
        self.rig.playhead = (frame / fps).clamp(0.0, duration);
    }

    // ------------------------------------------------------------ lip sync

    /// Load a WAV file for lip sync.
    pub fn load_audio(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let clip = AudioClip::from_wav(&bytes)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.status = format!("Loaded {name} ({:.1} s)", clip.duration());
        self.rig.audio = Some((name, clip));
        Ok(())
    }

    /// Bake the loaded audio into mouth tracks of the current motion,
    /// stretching the motion to fit the audio if needed.
    pub fn bake_lip_sync(&mut self) -> Result<()> {
        let m = self
            .rig
            .motion
            .ok_or_else(|| AetherError::rig("choose or create a motion first"))?;
        let (_, clip) = self
            .rig
            .audio
            .clone()
            .ok_or_else(|| AetherError::rig("load a WAV file first"))?;
        let rig = &self.doc.rig;
        let open = rig
            .behaviours
            .lip_sync
            .mouth_open
            .or_else(|| rig.parameter_named("MouthOpenY").map(|p| p.id))
            .ok_or_else(|| AetherError::rig("add a MouthOpenY parameter (or set one in lip sync) first"))?;
        let form = rig
            .behaviours
            .lip_sync
            .mouth_form
            .or_else(|| rig.parameter_named("MouthForm").map(|p| p.id));
        let gain = self.rig.lip_gain;
        self.edit_rig("Bake lip sync", None, |rig, _| {
            let motion = rig
                .motions
                .get_mut(m)
                .ok_or_else(|| AetherError::rig("no such motion"))?;
            let tracks = clip.bake_lip_sync(motion.fps, open, form, gain, 0.04);
            for track in tracks {
                motion.tracks.retain(|t| t.param != track.param);
                motion.tracks.push(track);
            }
            motion.duration = motion.duration.max(clip.duration());
            Ok(())
        })
    }

    // --------------------------------------------------------------- export

    /// Render the current motion (or an idle loop) and write it out.
    pub fn export_animation(&mut self, kind: ExportKind, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let mut settings = self.rig.export.clone();
        settings.motion = self.rig.motion;
        let frames = animation::render_frames(&self.doc, &self.compositor, &settings)?;
        let background = match self.doc.background {
            aether_document::Background::Solid(c) => c,
            aether_document::Background::Transparent => aether_core::color::Rgba8::WHITE,
        };
        match kind {
            ExportKind::Gif => animation::export_gif(&frames, path, settings.fps, background)?,
            ExportKind::Apng => animation::export_apng(&frames, path, settings.fps)?,
            ExportKind::PngSequence => {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "frame".into());
                let dir = path.parent().unwrap_or_else(|| Path::new("."));
                animation::export_png_sequence(&frames, dir, &stem)?;
            }
            ExportKind::SpriteSheet => {
                animation::export_sprite_sheet(&frames, path, settings.fps)?;
            }
        }
        self.status = format!("Exported {} frames to {}", frames.len(), path.display());
        Ok(())
    }

    /// Ask for a path, then export.
    pub fn export_animation_via_dialog(&mut self, kind: ExportKind) {
        let (label, ext) = match kind {
            ExportKind::Gif => ("GIF", "gif"),
            ExportKind::Apng => ("Animated PNG", "png"),
            ExportKind::PngSequence | ExportKind::SpriteSheet => ("PNG", "png"),
        };
        let picked = rfd::FileDialog::new()
            .add_filter(label, &[ext])
            .set_file_name(format!("{}.{ext}", self.doc.name))
            .save_file();
        if let Some(path) = picked {
            if let Err(error) = self.export_animation(kind, path) {
                self.report_error("Export animation", &error);
            }
        }
    }

    /// Write the runtime model — `model.json` plus texture atlases, for
    /// games and the web player — into `dir`. Returns what the export had to
    /// approximate.
    pub fn export_runtime_model(&mut self, dir: impl AsRef<Path>) -> Result<Vec<String>> {
        let dir = dir.as_ref();
        let export = aether_io::runtime_model::export_model_to_dir(&self.doc, dir, &Default::default())?;
        for warning in &export.warnings {
            tracing::warn!("runtime model: {warning}");
        }
        self.status = match export.warnings.first() {
            None => format!(
                "Exported runtime model to {} ({} parts)",
                dir.display(),
                export.model.parts.len()
            ),
            Some(first) => format!(
                "Exported runtime model to {} — {} note(s): {first}",
                dir.display(),
                export.warnings.len()
            ),
        };
        Ok(export.warnings)
    }

    /// Ask for a folder, then export the runtime model into it.
    pub fn export_runtime_model_via_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            if let Err(error) = self.export_runtime_model(dir) {
                self.report_error("Export runtime model", &error);
            }
        }
    }

    // ------------------------------------------------- Live2D interchange

    /// Import a Live2D motion (`.motion3.json`) as a new motion, in one undo
    /// step, and show it in the timeline. Returns what could not be carried
    /// over (curves for parameters this rig lacks, part opacity).
    pub fn import_live2d_motion(&mut self, path: impl AsRef<Path>) -> Result<Vec<String>> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path)?;
        let name = live2d_name(path);
        let (motion, notes) = aether_io::live2d::import_motion(&json, &self.doc.rig, &name)?;
        let mut index = 0;
        self.edit_rig("Import Live2D motion", None, |rig, _| {
            rig.motions.push(motion);
            index = rig.motions.len() - 1;
            Ok(())
        })?;
        self.rig.motion = Some(index);
        self.rig.playhead = 0.0;
        self.rig.animate = true;
        self.status = with_notes(format!("Imported motion \"{name}\""), &notes);
        Ok(notes)
    }

    /// Write motion `index` as a Live2D `.motion3.json`.
    pub fn export_live2d_motion(&mut self, index: usize, path: impl AsRef<Path>) -> Result<()> {
        let motion = self
            .doc
            .rig
            .motions
            .get(index)
            .ok_or_else(|| AetherError::rig("no such motion"))?;
        let json = aether_io::live2d::export_motion(motion, &self.doc.rig);
        std::fs::write(path.as_ref(), json)?;
        self.status = format!("Exported {}", path.as_ref().display());
        Ok(())
    }

    /// Import a Live2D expression (`.exp3.json`), in one undo step.
    pub fn import_live2d_expression(&mut self, path: impl AsRef<Path>) -> Result<Vec<String>> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path)?;
        let name = live2d_name(path);
        let (expression, notes) = aether_io::live2d::import_expression(&json, &self.doc.rig, &name)?;
        self.edit_rig("Import Live2D expression", None, |rig, _| {
            rig.expressions.push(expression);
            Ok(())
        })?;
        self.status = with_notes(format!("Imported expression \"{name}\""), &notes);
        Ok(notes)
    }

    /// Write expression `index` as a Live2D `.exp3.json`.
    pub fn export_live2d_expression(&mut self, index: usize, path: impl AsRef<Path>) -> Result<()> {
        let expression = self
            .doc
            .rig
            .expressions
            .get(index)
            .ok_or_else(|| AetherError::rig("no such expression"))?;
        let json = aether_io::live2d::export_expression(expression, &self.doc.rig);
        std::fs::write(path.as_ref(), json)?;
        self.status = format!("Exported {}", path.as_ref().display());
        Ok(())
    }

    /// Ask for a `.motion3.json`, then import it.
    pub fn import_live2d_motion_via_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Live2D motion", &["json"])
            .pick_file()
        {
            if let Err(error) = self.import_live2d_motion(path) {
                self.report_error("Import Live2D motion", &error);
            }
        }
    }

    /// Ask where to write motion `index` as `.motion3.json`.
    pub fn export_live2d_motion_via_dialog(&mut self, index: usize) {
        let name = self
            .doc
            .rig
            .motions
            .get(index)
            .map(|m| m.name.clone())
            .unwrap_or_default();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Live2D motion", &["json"])
            .set_file_name(format!("{name}.motion3.json"))
            .save_file()
        {
            if let Err(error) = self.export_live2d_motion(index, path) {
                self.report_error("Export Live2D motion", &error);
            }
        }
    }

    /// Ask for an `.exp3.json`, then import it.
    pub fn import_live2d_expression_via_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Live2D expression", &["json"])
            .pick_file()
        {
            if let Err(error) = self.import_live2d_expression(path) {
                self.report_error("Import Live2D expression", &error);
            }
        }
    }

    /// Ask where to write expression `index` as `.exp3.json`.
    pub fn export_live2d_expression_via_dialog(&mut self, index: usize) {
        let name = self
            .doc
            .rig
            .expressions
            .get(index)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Live2D expression", &["json"])
            .set_file_name(format!("{name}.exp3.json"))
            .save_file()
        {
            if let Err(error) = self.export_live2d_expression(index, path) {
                self.report_error("Export Live2D expression", &error);
            }
        }
    }

    /// Ask for a WAV file, then load it.
    pub fn load_audio_via_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV audio", &["wav"])
            .pick_file()
        {
            if let Err(error) = self.load_audio(path) {
                self.report_error("Load audio", &error);
            }
        }
    }
}

/// A motion or expression name from a Live2D file name
/// (`wave.motion3.json` → `wave`).
fn live2d_name(path: &Path) -> String {
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    [".motion3.json", ".exp3.json", ".json"]
        .iter()
        .find_map(|ext| file.strip_suffix(ext))
        .unwrap_or(&file)
        .to_string()
}

/// A status line, with the first of any notes.
fn with_notes(message: String, notes: &[String]) -> String {
    match notes.first() {
        None => message,
        Some(first) => format!("{message} — {} note(s): {first}", notes.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;
    use aether_core::math::IRect;

    fn painted_state() -> EditorState {
        let mut state = EditorState::new(aether_document::Document::new(96, 96, "rig"));
        let layer = state.doc.active_layer;
        if let Some(pm) = state.doc.layers.get_mut(layer).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(24, 24, 40, 40), Rgba8::rgb(200, 40, 40));
        }
        state
    }

    fn param(state: &EditorState, name: &str) -> ParameterId {
        state.doc.rig.parameter_named(name).expect("param").id
    }

    #[test]
    fn meshing_a_layer_is_undoable_and_selects_it() {
        let mut state = painted_state();
        let layer = state.doc.active_layer;
        state.mesh_layer(layer).expect("mesh");
        assert!(state.doc.rig.mesh(layer).is_some());
        assert_eq!(state.rig.tool.selection, Some(RigNode::Mesh(layer)));
        state.history.undo(&mut state.doc).expect("undo");
        assert!(state.doc.rig.mesh(layer).is_none());
    }

    #[test]
    fn an_empty_layer_cannot_be_meshed() {
        let mut state = EditorState::new(aether_document::Document::new(32, 32, "empty"));
        let layer = state.doc.active_layer;
        let err = state.mesh_layer(layer).expect_err("empty");
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn keys_bind_and_editing_moves_the_layer() {
        let mut state = painted_state();
        let layer = state.doc.active_layer;
        state.mesh_layer(layer).expect("mesh");
        state.add_standard_parameters().expect("params");
        let x = param(&state, "AngleX");
        state.bind_three_keys(x).expect("bind");
        state.set_parameter_value(x, 30.0).expect("pose");
        let form = state.doc.rig.mesh_form_mut(layer, None).expect("on key");
        form.offsets.iter_mut().for_each(|o| *o = Vec2::new(20.0, 0.0));
        state.tick_rig(1.0 / 60.0);
        let composite = state.compositor.render(&state.doc);
        assert_eq!(
            composite.get(70, 40),
            Rgba8::rgb(200, 40, 40),
            "the square moved right"
        );
    }

    #[test]
    fn slider_moves_are_not_undo_steps() {
        let mut state = painted_state();
        state.add_standard_parameters().expect("params");
        let before = state.history.entries().len();
        let x = param(&state, "AngleX");
        state.set_parameter_value(x, 12.0).expect("pose");
        assert_eq!(state.history.entries().len(), before);
        assert_eq!(state.doc.rig.value(x), 12.0);
    }

    #[test]
    fn animate_mode_keys_the_motion_at_the_playhead() {
        let mut state = painted_state();
        state.add_standard_parameters().expect("params");
        let x = param(&state, "AngleX");
        let m = state.add_motion("Wave").expect("motion");
        state.rig.playhead = 1.0;
        state.set_parameter_value(x, 20.0).expect("key");
        state.set_parameter_value(x, 25.0).expect("key again");
        let track = state.doc.rig.motions[m].track(x).expect("track");
        assert_eq!(track.keys.len(), 1, "one key, updated in place");
        assert_eq!(track.keys[0].value, 25.0);
        state.history.undo(&mut state.doc).expect("undo");
        assert!(
            state.doc.rig.motions[m].track(x).is_none(),
            "the drag coalesced into one undo step"
        );
    }

    #[test]
    fn playback_advances_and_drives_the_pose() {
        let mut state = painted_state();
        state.add_standard_parameters().expect("params");
        let x = param(&state, "AngleX");
        let m = state.add_motion("Turn").expect("motion");
        state
            .edit_rig("keys", None, |rig, _| {
                let track = rig.motions[m].track_mut(x);
                track.set_key(0.0, 0.0);
                track.set_key(1.0, 30.0);
                Ok(())
            })
            .expect("keys");
        state.toggle_playback();
        for _ in 0..30 {
            state.tick_rig(1.0 / 60.0);
        }
        let v = state.doc.rig.effective_value(x);
        assert!((v - 15.0).abs() < 1.0, "half a second in: {v}");
        assert_eq!(state.doc.rig.value(x), 0.0, "authored pose untouched");
        state.step_frames(3);
        assert!(!state.rig.playing, "stepping pauses");
    }

    #[test]
    fn warps_wrap_the_selection_and_head_turns_generate() {
        let mut state = painted_state();
        let layer = state.doc.active_layer;
        state.mesh_layer(layer).expect("mesh");
        state.add_standard_parameters().expect("params");
        state.add_warp_deformer("Head").expect("warp");
        let Some(RigNode::Deformer(d)) = state.rig.tool.selection else {
            panic!("the new warp is selected");
        };
        assert_eq!(
            state.doc.rig.mesh(layer).and_then(|m| m.parent),
            Some(NodeRef::Deformer(d))
        );
        let (x, y) = (param(&state, "AngleX"), param(&state, "AngleY"));
        state.generate_head_turn(x, Some(y)).expect("head turn");
        state.set_parameter_value(x, 30.0).expect("pose");
        state.tick_rig(0.0);
        let pose = state.doc.rig.evaluate();
        assert!(!pose.meshes[&layer].rest);
    }

    #[test]
    fn deleting_a_deformer_keeps_its_meshes() {
        let mut state = painted_state();
        let layer = state.doc.active_layer;
        state.mesh_layer(layer).expect("mesh");
        state.add_rotation_deformer("Pivot").expect("rotation");
        let node = state.rig.tool.selection.expect("selected");
        state.delete_rig_node(node).expect("delete");
        assert!(state.doc.rig.mesh(layer).is_some());
        assert_eq!(state.doc.rig.mesh(layer).and_then(|m| m.parent), None);
    }

    #[test]
    fn bones_skin_meshes() {
        let mut state = painted_state();
        let layer = state.doc.active_layer;
        state.mesh_layer(layer).expect("mesh");
        state
            .add_bone(Vec2::new(44.0, 64.0), Vec2::new(44.0, 24.0), None)
            .expect("bone");
        assert!(
            state.add_bone(Vec2::ZERO, Vec2::ZERO, None).is_err(),
            "zero-length bones are refused"
        );
        state.rig.tool.selection = Some(RigNode::Mesh(layer));
        state.auto_skin(2.0).expect("skin");
        assert!(state.doc.rig.mesh(layer).and_then(|m| m.skin.as_ref()).is_some());
    }

    #[test]
    fn drivers_are_validated_and_run_in_the_editor() {
        let mut state = painted_state();
        state.add_standard_parameters().expect("params");
        let (x, body) = (param(&state, "AngleX"), param(&state, "BodyAngleX"));
        assert!(state.add_driver(body, "Nope * 2").is_err());
        state.add_driver(body, "AngleX / 3").expect("driver");
        state.set_parameter_value(x, 30.0).expect("pose");
        state.tick_rig(0.0);
        assert_eq!(state.doc.rig.effective_value(body), 10.0);
    }

    #[test]
    fn expressions_capture_the_pose() {
        let mut state = painted_state();
        state.add_standard_parameters().expect("params");
        assert!(
            state.capture_expression("Neutral").is_err(),
            "nothing to capture at rest"
        );
        let smile = param(&state, "MouthForm");
        state.set_parameter_value(smile, 1.0).expect("pose");
        state.capture_expression("Smile").expect("capture");
        assert_eq!(state.doc.rig.expressions.len(), 1);
    }

    #[test]
    fn lip_sync_bakes_from_a_wav_file() {
        let mut state = painted_state();
        state.add_standard_parameters().expect("params");
        let m = state.add_motion("Talk").expect("motion");
        let rate = 8000;
        let samples: Vec<f32> = (0..rate * 2)
            .map(|i| {
                let t = i as f32 / rate as f32;
                if (t * 3.0).fract() < 0.5 {
                    (t * 200.0 * std::f32::consts::TAU).sin() * 0.5
                } else {
                    0.0
                }
            })
            .collect();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("voice.wav");
        std::fs::write(
            &path,
            AudioClip {
                samples,
                sample_rate: rate,
            }
            .to_wav(),
        )
        .expect("write");
        state.load_audio(&path).expect("load");
        state.bake_lip_sync().expect("bake");
        let mouth = param(&state, "MouthOpenY");
        assert!(state.doc.rig.motions[m].track(mouth).is_some());
    }

    #[test]
    fn animation_exports_write_files() {
        let mut state = painted_state();
        let layer = state.doc.active_layer;
        state.mesh_layer(layer).expect("mesh");
        state.add_standard_parameters().expect("params");
        state.add_motion("Loop").expect("motion");
        state.rig.export.fps = 4.0;
        state.rig.export.warmup = 0.0;
        let dir = tempfile::tempdir().expect("tempdir");
        state
            .export_animation(ExportKind::Gif, dir.path().join("loop.gif"))
            .expect("gif");
        assert!(dir.path().join("loop.gif").exists());
        state
            .export_animation(ExportKind::Apng, dir.path().join("loop.png"))
            .expect("apng");
        let apng = std::fs::read(dir.path().join("loop.png")).expect("read");
        assert!(apng.windows(4).any(|w| w == b"acTL"), "an animated PNG");
        state
            .export_animation(ExportKind::SpriteSheet, dir.path().join("sheet.png"))
            .expect("sheet");
        assert!(dir.path().join("sheet.json").exists());
    }

    #[test]
    fn gluing_joins_nearby_vertices_both_ways() {
        let mut state = painted_state();
        let a = state.doc.active_layer;
        let b = state.add_layer().expect("layer");
        if let Some(pm) = state.doc.layers.get_mut(b).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(60, 24, 20, 40), Rgba8::WHITE);
        }
        state.mesh_layer(a).expect("mesh a");
        state.mesh_layer(b).expect("mesh b");
        let glued = state.auto_glue(a, b, 12.0).expect("glue");
        assert!(glued > 0);
        assert_eq!(state.doc.rig.mesh(b).map(|m| m.glue.len()), Some(glued));
        assert!(state.auto_glue(a, a, 12.0).is_err());
    }

    #[test]
    fn a_bone_rotation_control_turns_the_bone_by_its_value() {
        let mut state = painted_state();
        state
            .add_bone(Vec2::new(10.0, 10.0), Vec2::new(40.0, 10.0), None)
            .expect("bone");
        let Some(RigNode::Bone(bone)) = state.rig.tool.selection else {
            panic!("bone selected");
        };
        let param = state.add_bone_rotation_control().expect("control");
        state.set_parameter_value(param, 90.0).expect("pose");
        let eval = aether_document::rig::Evaluator::new(&state.doc.rig);
        let tail = eval.skeleton().tail(bone).expect("tail");
        assert!(
            tail.distance(Vec2::new(10.0, 40.0)) < 1e-2,
            "turned a quarter: {tail:?}"
        );
    }

    #[test]
    fn auto_rig_builds_a_working_rig_in_one_undo_step() {
        let mut state = EditorState::new(aether_document::Document::new(200, 240, "auto"));
        let face = state.doc.active_layer;
        let paint = |state: &mut EditorState, id: LayerId, name: &str, rect: IRect| {
            if let Some(layer) = state.doc.layers.get_mut(id) {
                layer.name = name.into();
                if let Some(pm) = layer.pixmap_mut() {
                    pm.fill_rect(rect, Rgba8::rgb(230, 200, 180));
                }
            }
        };
        paint(&mut state, face, "顔", IRect::new(50, 40, 100, 120));
        let eye = state.add_layer().expect("layer");
        paint(&mut state, eye, "白目 左", IRect::new(70, 80, 20, 12));
        let hair = state.add_layer().expect("layer");
        paint(&mut state, hair, "前髪", IRect::new(45, 30, 110, 40));
        let before = state.history.entries().len();
        let report = state.auto_rig().expect("auto rig");
        assert_eq!(state.history.entries().len(), before + 1, "one undo step");
        assert!(report.unrecognised.is_empty());
        assert!(state.doc.rig.meshes.len() == 3);
        assert!(matches!(state.rig.tool.selection, Some(RigNode::Deformer(_))));
        let open = param(&state, "EyeLOpen");
        state.set_parameter_value(open, 0.0).expect("blink");
        assert!(!state.doc.rig.evaluate().meshes[&eye].rest);
        state.history.undo(&mut state.doc).expect("undo");
        assert!(
            state.doc.rig.meshes.is_empty(),
            "undo removes meshes and rig together"
        );
    }

    #[test]
    fn mesh_all_meshes_every_painted_layer_once() {
        let mut state = painted_state();
        let second = state.add_layer().expect("layer");
        if let Some(pm) = state.doc.layers.get_mut(second).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(4, 4, 10, 10), Rgba8::WHITE);
        }
        state.add_layer().expect("empty layer");
        assert_eq!(state.mesh_all_layers(None).expect("mesh"), 2);
        assert_eq!(state.mesh_all_layers(None).expect("again"), 0);
    }

    #[test]
    fn the_runtime_model_exports_from_the_editor() {
        let mut state = painted_state();
        state.mesh_all_layers(None).expect("mesh");
        let dir = tempfile::tempdir().expect("temp dir");
        let notes = state.export_runtime_model(dir.path()).expect("export");
        assert!(notes.is_empty(), "{notes:?}");
        assert!(
            state.status.starts_with("Exported runtime model"),
            "{}",
            state.status
        );
        let (model, textures) = aether_io::runtime_model::load_model(dir.path()).expect("load");
        assert_eq!(model.parts.len(), 1);
        assert_eq!(textures.len(), 1);
    }

    #[test]
    fn live2d_motions_and_expressions_round_trip_through_the_editor() {
        let mut state = painted_state();
        state.add_standard_parameters().expect("parameters");
        let dir = tempfile::tempdir().expect("temp dir");
        let motion = dir.path().join("wave.motion3.json");
        std::fs::write(
            &motion,
            r#"{"Version":3,"Meta":{"Duration":2,"Fps":30,"Loop":true},
               "Curves":[{"Target":"Parameter","Id":"ParamAngleX","Segments":[0,0,0,2,30]},
                         {"Target":"Parameter","Id":"ParamUnknown","Segments":[0,0,0,2,1]}]}"#,
        )
        .expect("write");
        let before = state.history.entries().len();
        let notes = state.import_live2d_motion(&motion).expect("import");
        assert_eq!(notes.len(), 1);
        assert_eq!(state.history.entries().len(), before + 1, "one undo step");
        let index = state.rig.motion.expect("shown in the timeline");
        assert_eq!(state.doc.rig.motions[index].name, "wave");
        assert!(state.status.contains("1 note"), "{}", state.status);

        let out = dir.path().join("out.motion3.json");
        state.export_live2d_motion(index, &out).expect("export");
        assert!(std::fs::read_to_string(&out).unwrap().contains("ParamAngleX"));

        let smile = dir.path().join("smile.exp3.json");
        std::fs::write(
            &smile,
            r#"{"Type":"Live2D Expression","Parameters":[{"Id":"ParamMouthForm","Value":1,"Blend":"Overwrite"}]}"#,
        )
        .expect("write");
        assert!(state.import_live2d_expression(&smile).expect("import").is_empty());
        let e = state.doc.rig.expressions.len() - 1;
        assert_eq!(state.doc.rig.expressions[e].name, "smile");
        state
            .export_live2d_expression(e, dir.path().join("x.exp3.json"))
            .expect("export");
        assert!(state.export_live2d_motion(99, dir.path().join("y.json")).is_err());
    }
}
