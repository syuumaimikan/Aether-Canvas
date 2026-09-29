//! Aether rigs as Cubism models.
//!
//! The hierarchy carries over: warp deformers become warp deformers,
//! rotation deformers become rotation deformers, and bones become nested
//! rotation deformers (a bone's children turn about its head, exactly as a
//! rotation's children turn about its pivot). Each object's keyforms are
//! *sampled* from Aether's own evaluation (see [`super::sample`]) on a grid
//! over the parameters it depends on, so Catmull-Rom interpolation, blend
//! shapes, drivers and inverse kinematics are all reproduced by keyforms
//! Cubism can interpolate. What Cubism cannot represent is approximated:
//!
//! * smooth (bicubic) warps become bilinear warps on a finer lattice;
//! * a rotation inside a warp stays rigid in Cubism (it turns with the
//!   warp but is not stretched by it);
//! * skinned meshes are baked: their vertices are keyed on the parameters
//!   that move their bones.
//!
//! After building, the model is evaluated as Cubism Core does (see
//! `aether_live2d::Model`) at the defaults, at each parameter's extremes and
//! at random poses, and compared vertex by vertex with the editor; the
//! largest differences are reported.

use super::sample::{sample, SampleAxis, SampleLimits, Sampled};
use super::{unique_id, Live2DExportOptions};
use crate::runtime_model::{export_model, ModelExportOptions};
use aether_core::math::{Rect, Transform2D, Vec2};
use aether_core::{AetherError, BoneId, DeformerId, LayerId, ParameterId, Result};
use aether_document::rig::deformer::DeformerMap;
use aether_document::rig::rig::ResolvedParams;
use aether_document::rig::{driver, skeleton, ArtMesh, DeformerKind, Evaluator, NodeRef, ParamValues, Rig};
use aether_document::Document;
use aether_live2d::builder::{self as b, DrawRef, MocBuilder};
use aether_live2d::moc3::{Canvas, Moc};
use aether_player::model::{BlendKind, Node, Part};
use aether_raster::Pixmap;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Opacity and colour differences, in tolerance units per unit: 0.01 of
/// opacity weighs as much as half a pixel.
const COLOR_WEIGHT: f32 = 50.0;
/// Radius at which a rotation's angle error is measured, pixels.
const ARM: f32 = 150.0;
/// Cubism's default draw order.
const DRAW_ORDER: f32 = 500.0;

/// Glued vertex pairs of two meshes: `(vertex on the first, vertex on
/// the second)` → the pull on each.
type GluePairs = BTreeMap<(u16, u16), (f32, f32)>;

/// A built model.
pub(super) struct NativeModel {
    pub moc: Moc,
    pub textures: Vec<Pixmap>,
    /// Rig parameter → Cubism id, in rig order.
    pub parameter_ids: Vec<(ParameterId, String)>,
    /// Part ids and display names.
    pub parts: Vec<(String, String)>,
    /// Largest vertex difference from the editor found by the check.
    pub max_error: f32,
}

/// How a deformer's children give their coordinates.
#[derive(Clone, Copy, Debug)]
enum Frame {
    /// Model units (the canvas).
    Root,
    /// The unit square over a warp's rest rectangle.
    Warp(Rect),
    /// Pixels from a pivot's rest position.
    Pivot(Vec2),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Bone(BoneId),
    Deformer(DeformerId),
}

struct Exported {
    source: Source,
    /// A rotation (or bone) above it in Cubism's chain: rotation scales
    /// then stay relative.
    under_rotation: bool,
    frame: Frame,
}

/// Parameters that drivers compute from others.
struct Drivers {
    /// Driven parameter → the free parameters it is computed from.
    inputs: BTreeMap<ParameterId, BTreeSet<ParameterId>>,
    active: bool,
}

impl Drivers {
    /// Drivers left out of the keyforms: named in a note.
    fn left_out(rig: &Rig, notes: &mut Vec<String>) -> Self {
        let driven: Vec<&str> = rig
            .drivers
            .iter()
            .filter(|d| d.enabled && d.mix > 0.0)
            .filter_map(|d| rig.parameter(d.target).map(|p| p.name.as_str()))
            .collect();
        if !driven.is_empty() {
            notes.push(format!(
                "{}: driven in Aether, ordinary parameters in Cubism (apps such as VTube Studio \
                 drive them from tracking; exported motions include the driven curves)",
                driven.join(", ")
            ));
        }
        Self {
            inputs: BTreeMap::new(),
            active: false,
        }
    }

    fn new(rig: &Rig, notes: &mut Vec<String>) -> Self {
        let mut direct: BTreeMap<ParameterId, Vec<ParameterId>> = BTreeMap::new();
        for d in rig.drivers.iter().filter(|d| d.enabled && d.mix > 0.0) {
            let Ok(program) = d.compile(&rig.parameters) else {
                continue;
            };
            if program.uses_time {
                let name = rig.parameter(d.target).map(|p| p.name.as_str()).unwrap_or("?");
                notes.push(format!(
                    "the driver on \"{name}\" changes over time; it is exported as it is at time 0"
                ));
            }
            let mut reads: Vec<ParameterId> = program
                .variables
                .iter()
                .map(|&slot| rig.parameters.get(slot).map(|p| p.id).unwrap_or(d.target))
                .collect();
            if d.mix < 1.0 {
                // Part of the undriven value shows through.
                reads.push(d.target);
            }
            direct.entry(d.target).or_insert(reads);
        }
        // Close over chains of drivers.
        let mut inputs = BTreeMap::new();
        for &target in direct.keys() {
            let mut free = BTreeSet::new();
            let mut stack = vec![target];
            let mut seen = BTreeSet::new();
            while let Some(p) = stack.pop() {
                if !seen.insert(p) {
                    continue;
                }
                match direct.get(&p) {
                    Some(reads) => {
                        for &r in reads {
                            if r == p {
                                free.insert(p);
                            } else {
                                stack.push(r);
                            }
                        }
                    }
                    None => {
                        free.insert(p);
                    }
                }
            }
            inputs.insert(target, free);
        }
        let active = !inputs.is_empty();
        Self { inputs, active }
    }

    fn free_inputs(&self, p: ParameterId) -> BTreeSet<ParameterId> {
        self.inputs
            .get(&p)
            .cloned()
            .unwrap_or_else(|| BTreeSet::from([p]))
    }
}

struct Exporter<'a> {
    rig: &'a Rig,
    options: &'a Live2DExportOptions,
    drivers: Drivers,
    ppu: f32,
    origin: Vec2,
    param_index: BTreeMap<ParameterId, usize>,
    limits: SampleLimits,
    capped: BTreeSet<String>,
    notes: Vec<String>,
}

fn grid_params<T>(grid: &aether_document::rig::KeyformGrid<T>, out: &mut BTreeMap<ParameterId, Vec<f32>>) {
    for axis in &grid.axes {
        out.entry(axis.param).or_default().extend(&axis.keys);
    }
}

fn warp_bilinear(points: &[Vec2], cols: usize, rows: usize, u: f32, v: f32) -> Vec2 {
    let gx = (u * cols as f32).clamp(0.0, cols as f32);
    let gy = (v * rows as f32).clamp(0.0, rows as f32);
    let i = (gx.floor() as usize).min(cols - 1);
    let j = (gy.floor() as usize).min(rows - 1);
    let (tx, ty) = (gx - i as f32, gy - j as f32);
    let at = |i: usize, j: usize| points[j * (cols + 1) + i];
    let top = at(i, j) * (1.0 - tx) + at(i + 1, j) * tx;
    let bottom = at(i, j + 1) * (1.0 - tx) + at(i + 1, j + 1) * tx;
    top * (1.0 - ty) + bottom * ty
}

impl<'a> Exporter<'a> {
    /// The parameter values rendering sees for these free values.
    fn effective(&self, free: &ParamValues) -> ParamValues {
        let mut values = free.clone();
        if self.drivers.active {
            driver::apply(&self.rig.parameters, &self.rig.drivers, &mut values, 0.0);
        }
        values
    }

    fn resolved(&self, axes: &[SampleAxis], at: &[f32]) -> (ParamValues, ResolvedParams) {
        let free: ParamValues = axes.iter().zip(at).map(|(a, &v)| (a.param, v)).collect();
        let values = self.effective(&free);
        let resolved = ResolvedParams::new(&self.rig.parameters, &values);
        (values, resolved)
    }

    /// Grid axes for an object keyed directly on `direct`: driven
    /// parameters are replaced by what drives them, and every axis spans
    /// its parameter's whole range (Cubism hides an object whose parameter
    /// leaves its keys).
    fn axes(&self, direct: &BTreeMap<ParameterId, Vec<f32>>) -> Vec<SampleAxis> {
        let mut keys: BTreeMap<ParameterId, Vec<f32>> = BTreeMap::new();
        for (&p, ks) in direct {
            for q in self.drivers.free_inputs(p) {
                let Some(param) = self.rig.parameter(q) else {
                    continue;
                };
                let entry = keys.entry(q).or_default();
                if q == p {
                    entry.extend(ks);
                } else {
                    entry.push(param.default);
                }
                entry.extend([param.min, param.max]);
            }
        }
        let mut axes: Vec<SampleAxis> = keys
            .into_iter()
            .filter_map(|(p, mut ks)| {
                let param = self.rig.parameter(p)?;
                for k in &mut ks {
                    *k = k.clamp(param.min, param.max);
                }
                ks.sort_by(f32::total_cmp);
                ks.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
                (ks.len() >= 2).then_some(SampleAxis { param: p, keys: ks })
            })
            .collect();
        axes.sort_by_key(|a| self.param_index.get(&a.param).copied().unwrap_or(usize::MAX));
        axes
    }

    fn grid<T>(&self, s: &Sampled, form: impl Fn(&[f32]) -> T) -> b::Grid<T> {
        b::Grid {
            axes: s
                .axes
                .iter()
                .map(|a| b::Axis {
                    parameter: self.param_index[&a.param],
                    keys: a.keys.clone(),
                })
                .collect(),
            forms: s.forms.iter().map(|f| form(f)).collect(),
        }
    }

    fn to_local(&self, frame: Frame, p: Vec2) -> [f32; 2] {
        match frame {
            Frame::Root => [(p.x - self.origin.x) / self.ppu, (p.y - self.origin.y) / self.ppu],
            Frame::Warp(r) => [
                (p.x - r.min.x) / r.width().max(1e-6),
                (p.y - r.min.y) / r.height().max(1e-6),
            ],
            Frame::Pivot(o) => [p.x - o.x, p.y - o.y],
        }
    }

    fn positions(&self, frame: Frame, flat: &[f32]) -> Vec<f32> {
        flat.chunks_exact(2)
            .flat_map(|c| self.to_local(frame, Vec2::new(c[0], c[1])))
            .collect()
    }

    /// Parameters that move bone `id`'s own pose relative to its parent.
    fn bone_local_params(&self, id: BoneId, out: &mut BTreeMap<ParameterId, Vec<f32>>) {
        let Some(bone) = self.rig.bone(id) else { return };
        grid_params(&bone.keyforms, out);
        // Inverse kinematics turns the chain toward a target that moves
        // with its own parameters.
        for end in &self.rig.bones {
            let Some(ik) = &end.ik else { continue };
            let mut chain = vec![end.id];
            let mut current = end.parent;
            while chain.len() < ik.chain.max(1) {
                let Some(p) = current else { break };
                chain.push(p);
                current = self.rig.bone(p).and_then(|b| b.parent);
            }
            if chain.contains(&id) {
                // The target's own keyforms and its ancestors' (IK on those
                // is not followed further).
                let mut current = Some(ik.target);
                let mut steps = 0;
                while let Some(t) = current {
                    steps += 1;
                    if steps > self.rig.bones.len() {
                        break;
                    }
                    let Some(tb) = self.rig.bone(t) else { break };
                    grid_params(&tb.keyforms, out);
                    current = tb.parent;
                }
                for &c in &chain {
                    if let Some(cb) = self.rig.bone(c) {
                        grid_params(&cb.keyforms, out);
                    }
                }
            }
        }
    }

    /// Parameters that move bone `id` in the document: its own and its
    /// ancestors' local parameters.
    fn bone_world_params(&self, id: BoneId, out: &mut BTreeMap<ParameterId, Vec<f32>>) {
        let mut current = Some(id);
        let mut steps = 0;
        while let Some(b) = current {
            steps += 1;
            if steps > self.rig.bones.len() + 1 {
                break;
            }
            self.bone_local_params(b, out);
            current = self.rig.bone(b).and_then(|x| x.parent);
        }
    }
}

/// World angle of a posed bone's x axis, radians.
fn bone_angle(world: &Transform2D) -> f32 {
    let x = world.apply_vector(Vec2::new(1.0, 0.0));
    x.y.atan2(x.x)
}

fn next_pow2(n: u32) -> u32 {
    n.max(1).next_power_of_two()
}

/// Deepest common ancestor (or self) of `bones`.
fn common_ancestor(rig: &Rig, bones: &[BoneId]) -> Option<BoneId> {
    let chain = |b: BoneId| -> Vec<BoneId> {
        let mut out = Vec::new();
        let mut current = Some(b);
        while let Some(c) = current {
            if out.contains(&c) || out.len() > rig.bones.len() {
                break;
            }
            out.push(c);
            current = rig.bone(c).and_then(|x| x.parent);
        }
        out
    };
    let first = chain(*bones.first()?);
    first
        .into_iter()
        .find(|candidate| bones.iter().all(|&b| chain(b).contains(candidate)))
}

pub(super) fn export_native(
    doc: &Document,
    options: &Live2DExportOptions,
    notes: &mut Vec<String>,
) -> Result<NativeModel> {
    let rig = &doc.rig;
    let runtime = export_model(
        doc,
        &ModelExportOptions {
            max_texture_size: options.max_texture_size,
            padding: 2,
        },
    );
    notes.extend(runtime.warnings.iter().cloned());
    let model = &runtime.model;
    if model.parts.is_empty() {
        return Err(AetherError::serialization("the document draws nothing to export"));
    }

    // Texture pages, padded to power-of-two squares as Cubism tools expect.
    let mut textures = Vec::with_capacity(runtime.textures.len());
    let mut uv_scale = Vec::with_capacity(runtime.textures.len());
    for page in &runtime.textures {
        let size = next_pow2(page.width().max(page.height()));
        let mut square = Pixmap::new(size, size);
        square.paste_rect(page, 0, 0);
        uv_scale.push(Vec2::new(
            page.width() as f32 / size as f32,
            page.height() as f32 / size as f32,
        ));
        textures.push(square);
    }

    let ppu = doc.width.max(doc.height).max(1) as f32;
    let canvas = Canvas {
        pixels_per_unit: ppu,
        origin_x: doc.width as f32 / 2.0,
        origin_y: doc.height as f32 / 2.0,
        width: doc.width as f32,
        height: doc.height as f32,
        flags: 0,
    };

    // Parameters.
    let mut used = BTreeSet::new();
    let mut builder = MocBuilder {
        canvas,
        ..Default::default()
    };
    let mut parameter_ids = Vec::new();
    let mut param_index = BTreeMap::new();
    for p in &rig.parameters {
        let id = unique_id(&crate::live2d::live2d_id(&p.name), "Param", &mut used);
        param_index.insert(p.id, builder.parameters.len());
        builder.parameters.push(b::ParameterDef {
            id: id.clone(),
            min: p.min,
            max: p.max,
            default: p.default,
            repeat: p.cyclic,
            decimal_places: 3,
        });
        parameter_ids.push((p.id, id));
    }

    let mut ex = Exporter {
        rig,
        options,
        drivers: if options.bake_drivers {
            Drivers::new(rig, notes)
        } else {
            Drivers::left_out(rig, notes)
        },
        ppu,
        origin: Vec2::new(canvas.origin_x, canvas.origin_y),
        param_index,
        limits: SampleLimits {
            tolerance: tolerance(doc, options),
            ..Default::default()
        },
        capped: BTreeSet::new(),
        notes: Vec::new(),
    };

    // Which bones and deformers the exported meshes need, and where each
    // skinned mesh goes.
    let mut need_bones: BTreeSet<BoneId> = BTreeSet::new();
    let mut need_deformers: BTreeSet<DeformerId> = BTreeSet::new();
    let mut skin_home: BTreeMap<LayerId, BoneId> = BTreeMap::new();
    let mut require = |node: Option<NodeRef>, bones: &mut BTreeSet<BoneId>| {
        let mut current = node;
        let mut steps = 0;
        while let Some(n) = current {
            steps += 1;
            if steps > 256 {
                break;
            }
            match n {
                NodeRef::Deformer(id) => {
                    need_deformers.insert(id);
                    current = rig.deformer(id).and_then(|d| d.parent);
                }
                NodeRef::Bone(id) => {
                    bones.insert(id);
                    current = None;
                }
            }
        }
    };
    for part in &model.parts {
        let Some(mesh) = rig.mesh(part.layer) else {
            continue;
        };
        require(mesh.parent, &mut need_bones);
        if let Some(skin) = mesh.skin.as_ref().filter(|s| !s.bones.is_empty()) {
            if mesh.parent.is_none() {
                let used: Vec<BoneId> = skin
                    .bones
                    .iter()
                    .enumerate()
                    .filter(|(slot, _)| (0..mesh.vertices.len()).any(|v| skin.weight(v, *slot) > 0.0))
                    .map(|(_, &b)| b)
                    .collect();
                if let Some(home) = common_ancestor(rig, &used) {
                    skin_home.insert(part.layer, home);
                    need_bones.insert(home);
                }
            }
        }
        if mesh.jiggle.as_ref().is_some_and(|j| j.enabled) {
            notes.push(format!(
                "jiggle on \"{}\" is left out (Cubism has no per-vertex springs; use physics)",
                mesh_name(doc, mesh)
            ));
        }
    }
    // Bones carry their ancestors along.
    for id in need_bones.clone() {
        let mut current = rig.bone(id).and_then(|b| b.parent);
        let mut steps = 0;
        while let Some(p) = current {
            steps += 1;
            if steps > rig.bones.len() || !need_bones.insert(p) {
                break;
            }
            current = rig.bone(p).and_then(|b| b.parent);
        }
    }

    // Deformers in Cubism order: bones (parents first), then deformers by
    // depth.
    let mut order: Vec<Source> = skeleton::topological_order(&rig.bones)
        .into_iter()
        .map(|i| rig.bones[i].id)
        .filter(|id| need_bones.contains(id))
        .map(Source::Bone)
        .collect();
    let depth = |id: DeformerId| -> usize {
        let mut d = 0;
        let mut current = rig.deformer(id).and_then(|x| x.parent);
        while let Some(NodeRef::Deformer(p)) = current {
            d += 1;
            if d > 256 {
                break;
            }
            current = rig.deformer(p).and_then(|x| x.parent);
        }
        d
    };
    let mut deformers: Vec<DeformerId> = rig
        .deformers
        .iter()
        .map(|d| d.id)
        .filter(|id| need_deformers.contains(id))
        .collect();
    deformers.sort_by_key(|&id| depth(id));
    order.extend(deformers.into_iter().map(Source::Deformer));

    let mut exported: Vec<Exported> = Vec::new();
    let mut index_of: BTreeMap<Source, usize> = BTreeMap::new();
    let mut scaled_bones = BTreeSet::new();
    for source in order {
        let parent_node = match source {
            Source::Bone(id) => rig.bone(id).and_then(|b| b.parent).map(Source::Bone),
            Source::Deformer(id) => rig.deformer(id).and_then(|d| d.parent).map(|p| match p {
                NodeRef::Deformer(d) => Source::Deformer(d),
                NodeRef::Bone(b) => Source::Bone(b),
            }),
        };
        let parent = parent_node.and_then(|p| index_of.get(&p).copied());
        let parent_frame = parent.map(|i| exported[i].frame).unwrap_or(Frame::Root);
        let under_rotation = parent
            .is_some_and(|i| exported[i].under_rotation || !matches!(exported[i].frame, Frame::Warp(_)));
        let (def, frame) = match source {
            Source::Bone(id) => {
                let bone = rig.bone(id).expect("needed bones exist");
                if bone.keyforms.forms.iter().any(|f| (f.scale - 1.0).abs() > 1e-4) {
                    scaled_bones.insert(bone.name.clone());
                }
                let parent_bone = parent.and_then(|i| match exported[i].source {
                    Source::Bone(p) => Some(p),
                    Source::Deformer(_) => None,
                });
                (
                    ex.bone_deformer(id, parent_bone, parent_frame, under_rotation, &mut used),
                    Frame::Pivot(bone.head),
                )
            }
            Source::Deformer(id) => {
                let d = rig.deformer(id).expect("needed deformers exist");
                match &d.kind {
                    DeformerKind::Warp(w) => (
                        ex.warp_deformer(d, w, parent_frame, &mut used),
                        Frame::Warp(w.rect),
                    ),
                    DeformerKind::Rotation(r) => (
                        ex.rotation_deformer(d, parent_frame, under_rotation, &mut used),
                        Frame::Pivot(r.origin),
                    ),
                }
            }
        };
        let mut def = def;
        def.parent_deformer = parent;
        index_of.insert(source, exported.len());
        builder.deformers.push(def);
        exported.push(Exported {
            source,
            under_rotation,
            frame,
        });
    }
    for name in &scaled_bones {
        notes.push(format!(
            "bone \"{name}\" stretches; Cubism rotations cannot, so what it carries keeps its length"
        ));
    }

    // Parts, meshes and the draw order, from the runtime draw tree.
    let mut walk = Walk {
        doc,
        model,
        uv_scale: &uv_scale,
        exported: &exported,
        index_of: &index_of,
        skin_home: &skin_home,
        mesh_of_layer: BTreeMap::new(),
        parts: Vec::new(),
        transformed: Vec::new(),
        baked: BTreeMap::new(),
    };
    walk.nodes(&mut ex, &mut builder, &model.tree, None, &mut used)?;
    for name in walk.transformed.drain(..) {
        notes.push(format!(
            "\"{name}\" has a layer transform inside a deformer; Cubism leaves it out"
        ));
    }
    let mesh_of_layer = walk.mesh_of_layer.clone();
    let parts = walk.parts.clone();
    let baked = walk.baked.clone();

    // Glue.
    let mut glues: BTreeMap<(usize, usize), GluePairs> = BTreeMap::new();
    for mesh in &rig.meshes {
        let Some(&a) = mesh_of_layer.get(&mesh.layer) else {
            continue;
        };
        for g in &mesh.glue {
            let Some(&other) = mesh_of_layer.get(&g.other) else {
                continue;
            };
            if other == a {
                continue;
            }
            let (Ok(va), Ok(vb)) = (u16::try_from(g.vertex), u16::try_from(g.other_vertex)) else {
                continue;
            };
            let s = g.strength.clamp(0.0, 1.0);
            if a < other {
                glues
                    .entry((a, other))
                    .or_default()
                    .entry((va, vb))
                    .or_default()
                    .0 = s;
            } else {
                glues
                    .entry((other, a))
                    .or_default()
                    .entry((vb, va))
                    .or_default()
                    .1 = s;
            }
        }
    }
    for ((a, bm), pairs) in glues {
        let id = unique_id(
            &format!("Glue_{}_{}", builder.meshes[a].id, builder.meshes[bm].id),
            "Glue",
            &mut used,
        );
        builder.glues.push(b::GlueDef {
            id,
            mesh_a: a,
            mesh_b: bm,
            pairs: pairs
                .into_iter()
                .map(|((va, vb), (wa, wb))| (va, vb, wa, wb))
                .collect(),
            intensity: b::Grid::constant(1.0),
        });
    }

    notes.append(&mut ex.notes);
    for what in &ex.capped {
        notes.push(format!(
            "{what} moves in ways that need very many keyforms; it is approximated more coarsely"
        ));
    }

    let moc = builder
        .build(options.version)
        .map_err(|e| AetherError::serialization(e.to_string()))?;

    // How close Cubism comes to the editor.
    let max_error = check(doc, &ex, &moc, &mesh_of_layer, &baked, notes);
    Ok(NativeModel {
        moc,
        textures,
        parameter_ids,
        parts,
        max_error,
    })
}

/// The sampling tolerance in pixels.
fn tolerance(doc: &Document, options: &Live2DExportOptions) -> f32 {
    options
        .tolerance
        .unwrap_or(doc.width.max(doc.height) as f32 * 0.001)
        .max(0.25)
}

fn mesh_name(doc: &Document, mesh: &ArtMesh) -> String {
    if !mesh.name.is_empty() {
        return mesh.name.clone();
    }
    doc.layers
        .get(mesh.layer)
        .map(|l| l.name.clone())
        .unwrap_or_else(|| mesh.layer.to_string())
}

impl Exporter<'_> {
    fn warp_deformer(
        &mut self,
        d: &aether_document::rig::Deformer,
        w: &aether_document::rig::WarpDeformer,
        parent_frame: Frame,
        used: &mut BTreeSet<String>,
    ) -> b::DeformerDef {
        let mut direct = BTreeMap::new();
        grid_params(&w.keyforms, &mut direct);
        for s in &w.blend_shapes {
            grid_params(&s.grid, &mut direct);
        }
        // Rotations inside turn with the warp's local direction at their
        // pivot, measured over a tenth of its height; an error there grows
        // with the reach of what they carry.
        let reach = self.rotation_reach(d.id);
        let probe = w.rect.height() * 0.1;
        let tolerance = self.limits.tolerance * (probe / reach.max(probe)).clamp(0.05, 1.0);
        // The error of carrying adds to the warps' own, so it gets a share.
        let factor = self
            .fine_factor(w, tolerance)
            .max(self.carried_factor(d, w, tolerance * 0.35));
        let (cols, rows) = (w.cols * factor, w.rows * factor);
        let size = w.rect.size();
        let rest: Vec<Vec2> = (0..=rows)
            .flat_map(|j| {
                (0..=cols).map(move |i| {
                    Vec2::new(
                        w.rect.min.x + size.x * i as f32 / cols as f32,
                        w.rect.min.y + size.y * j as f32 / rows as f32,
                    )
                })
            })
            .collect();
        let axes = self.axes(&direct);
        let n = rest.len() * 2;
        let mut weights = vec![1.0; n];
        weights.push(COLOR_WEIGHT);
        let s = {
            let this = &*self;
            let mut eval = |at: &[f32]| -> Vec<f32> {
                let (_, resolved) = this.resolved(&axes, at);
                let state = d.evaluate(&resolved);
                let mut out = Vec::with_capacity(n + 1);
                if let DeformerMap::Warp(ws) = &state.map {
                    for p in &rest {
                        let q = *p + ws.displacement(*p);
                        out.extend([q.x, q.y]);
                    }
                }
                out.push(state.opacity);
                out
            };
            let limits = SampleLimits {
                tolerance,
                ..this.limits
            };
            sample(axes.clone(), &weights, &limits, &mut eval)
        };
        if s.capped {
            self.capped.insert(format!("warp \"{}\"", d.name));
        }
        let forms = self.grid(&s, |f| b::WarpForm {
            positions: self.positions(parent_frame, &f[..n]),
            opacity: f[n],
            tint: b::Tint::default(),
        });
        b::DeformerDef {
            id: unique_id(&format!("Warp_{}", d.name), "Warp", used),
            parent_part: None,
            parent_deformer: None,
            kind: b::DeformerKind::Warp {
                rows,
                cols,
                quad: true,
                forms,
            },
        }
    }

    /// How many times finer a bilinear lattice must be to follow a smooth
    /// warp within the tolerance.
    /// How far the artwork carried by rotations directly inside warp `id`
    /// reaches from their pivots (0 when there are none).
    fn rotation_reach(&self, id: DeformerId) -> f32 {
        let rig = self.rig;
        let pivots: Vec<(DeformerId, Vec2)> = rig
            .deformers
            .iter()
            .filter(|d| d.parent == Some(NodeRef::Deformer(id)))
            .filter_map(|d| match &d.kind {
                DeformerKind::Rotation(r) => Some((d.id, r.origin)),
                DeformerKind::Warp(_) => None,
            })
            .collect();
        let mut reach = 0.0f32;
        for mesh in &rig.meshes {
            let mut node = mesh.parent;
            let mut steps = 0;
            while let Some(NodeRef::Deformer(p)) = node {
                steps += 1;
                if steps > 64 {
                    break;
                }
                if let Some((_, pivot)) = pivots.iter().find(|(r, _)| *r == p) {
                    for v in &mesh.vertices {
                        reach = reach.max(v.distance(*pivot));
                    }
                    break;
                }
                node = rig.deformer(p).and_then(|d| d.parent);
            }
        }
        reach
    }

    /// How many times finer warp `d`'s lattice must be for the warps around
    /// it to bend it as they bend its content. Cubism carries a nested
    /// warp's lattice points through the warps around it and interpolates
    /// its content between them, where the editor carries every point.
    fn carried_factor(
        &self,
        d: &aether_document::rig::Deformer,
        w: &aether_document::rig::WarpDeformer,
        tolerance: f32,
    ) -> usize {
        let rig = self.rig;
        let mut around = Vec::new();
        let mut node = d.parent;
        let mut steps = 0;
        while let Some(NodeRef::Deformer(p)) = node {
            steps += 1;
            if steps > 64 {
                break;
            }
            let Some(parent) = rig.deformer(p) else { break };
            if let DeformerKind::Warp(pw) = &parent.kind {
                for form in &pw.keyforms.forms {
                    around.push(aether_document::rig::deformer::WarpState {
                        rect: pw.rect,
                        cols: pw.cols,
                        rows: pw.rows,
                        smooth: pw.smooth,
                        offsets: form.offsets.clone(),
                    });
                }
            }
            node = parent.parent;
        }
        if around.is_empty() {
            return 1;
        }
        let size = w.rect.size();
        for factor in 1..=8usize {
            let (cols, rows) = (w.cols * factor, w.rows * factor);
            if (cols + 1) * (rows + 1) > 4225 {
                return factor.saturating_sub(1).max(1);
            }
            let at = |u: f32, v: f32| Vec2::new(w.rect.min.x + size.x * u, w.rect.min.y + size.y * v);
            let mut worst = 0.0f32;
            for ws in &around {
                let carried = |p: Vec2| p + ws.displacement(p);
                let lattice: Vec<Vec2> = (0..=rows)
                    .flat_map(|j| (0..=cols).map(move |i| (i, j)))
                    .map(|(i, j)| carried(at(i as f32 / cols as f32, j as f32 / rows as f32)))
                    .collect();
                for j in 0..=rows * 2 {
                    for i in 0..=cols * 2 {
                        if i % 2 == 0 && j % 2 == 0 {
                            continue;
                        }
                        let (u, v) = (i as f32 / (cols * 2) as f32, j as f32 / (rows * 2) as f32);
                        let e = carried(at(u, v)).distance(warp_bilinear(&lattice, cols, rows, u, v));
                        worst = worst.max(e);
                    }
                }
            }
            if worst <= tolerance {
                return factor;
            }
        }
        8
    }

    fn fine_factor(&self, w: &aether_document::rig::WarpDeformer, tolerance: f32) -> usize {
        if !w.smooth {
            return 1;
        }
        let states: Vec<aether_document::rig::deformer::WarpState> = w
            .keyforms
            .forms
            .iter()
            .map(|f| aether_document::rig::deformer::WarpState {
                rect: w.rect,
                cols: w.cols,
                rows: w.rows,
                smooth: true,
                offsets: f.offsets.clone(),
            })
            .collect();
        let size = w.rect.size();
        for factor in 1..=8usize {
            let (cols, rows) = (w.cols * factor, w.rows * factor);
            if (cols + 1) * (rows + 1) > 4225 {
                return factor.saturating_sub(1).max(1);
            }
            let mut worst = 0.0f32;
            for ws in &states {
                let lattice: Vec<Vec2> = (0..=rows)
                    .flat_map(|j| (0..=cols).map(move |i| (i, j)))
                    .map(|(i, j)| {
                        let p = Vec2::new(
                            w.rect.min.x + size.x * i as f32 / cols as f32,
                            w.rect.min.y + size.y * j as f32 / rows as f32,
                        );
                        ws.displacement(p)
                    })
                    .collect();
                // Cell centres and edge midpoints, where bilinear
                // interpolation strays furthest.
                for j in 0..=rows * 2 {
                    for i in 0..=cols * 2 {
                        if i % 2 == 0 && j % 2 == 0 {
                            continue;
                        }
                        let (u, v) = (i as f32 / (cols * 2) as f32, j as f32 / (rows * 2) as f32);
                        let p = Vec2::new(w.rect.min.x + size.x * u, w.rect.min.y + size.y * v);
                        let e = ws
                            .displacement(p)
                            .distance(warp_bilinear(&lattice, cols, rows, u, v));
                        worst = worst.max(e);
                    }
                }
            }
            if worst <= tolerance {
                return factor;
            }
        }
        8
    }

    fn rotation_deformer(
        &mut self,
        d: &aether_document::rig::Deformer,
        parent_frame: Frame,
        under_rotation: bool,
        used: &mut BTreeSet<String>,
    ) -> b::DeformerDef {
        let DeformerKind::Rotation(r) = &d.kind else {
            unreachable!("a rotation deformer")
        };
        let mut direct = BTreeMap::new();
        grid_params(&r.keyforms, &mut direct);
        for s in &r.blend_shapes {
            grid_params(&s.grid, &mut direct);
        }
        let axes = self.axes(&direct);
        let weights = [1.0, 1.0, ARM * std::f32::consts::PI / 180.0, ARM, COLOR_WEIGHT];
        let s = {
            let this = &*self;
            let mut eval = |at: &[f32]| -> Vec<f32> {
                let (_, resolved) = this.resolved(&axes, at);
                let state = d.evaluate(&resolved);
                match &state.map {
                    DeformerMap::Rotation(rs) => {
                        let pivot = rs.pivot();
                        vec![pivot.x, pivot.y, rs.angle.to_degrees(), rs.scale, state.opacity]
                    }
                    DeformerMap::Warp(_) => vec![0.0; 5],
                }
            };
            sample(axes.clone(), &weights, &this.limits, &mut eval)
        };
        if s.capped {
            self.capped.insert(format!("rotation \"{}\"", d.name));
        }
        let unit = if under_rotation { 1.0 } else { 1.0 / self.ppu };
        let forms = self.grid(&s, |f| b::RotationForm {
            origin: self.to_local(parent_frame, Vec2::new(f[0], f[1])),
            angle: f[2],
            scale: f[3] * unit,
            reflect_x: false,
            reflect_y: false,
            opacity: f[4],
            tint: b::Tint::default(),
        });
        b::DeformerDef {
            id: unique_id(&format!("Rotation_{}", d.name), "Rotation", used),
            parent_part: None,
            parent_deformer: None,
            kind: b::DeformerKind::Rotation {
                base_angle: 0.0,
                forms,
            },
        }
    }

    fn bone_deformer(
        &mut self,
        id: BoneId,
        parent: Option<BoneId>,
        parent_frame: Frame,
        under_rotation: bool,
        used: &mut BTreeSet<String>,
    ) -> b::DeformerDef {
        let rig = self.rig;
        let bone = rig.bone(id).expect("bone exists");
        let mut direct = BTreeMap::new();
        if parent.is_some() {
            self.bone_local_params(id, &mut direct);
        } else {
            self.bone_world_params(id, &mut direct);
        }
        let axes = self.axes(&direct);
        let rest_angle = bone.rest_angle();
        let parent_bone = parent.and_then(|p| rig.bone(p));
        // Angles are unwrapped around the rest pose, so keys never jump a
        // turn.
        let reference = {
            let resolved = ResolvedParams::new(&rig.parameters, &Default::default());
            let pose = skeleton::solve(&rig.bones, &resolved);
            let own = pose.world.get(&id).map(bone_angle).unwrap_or(rest_angle) - rest_angle;
            let up = parent_bone
                .and_then(|p| pose.world.get(&p.id).map(|w| bone_angle(w) - p.rest_angle()))
                .unwrap_or(0.0);
            own - up
        };
        let weights = [1.0, 1.0, ARM * std::f32::consts::PI / 180.0];
        let s = {
            let this = &*self;
            let mut eval = |at: &[f32]| -> Vec<f32> {
                let (_, resolved) = this.resolved(&axes, at);
                let pose = skeleton::solve(&rig.bones, &resolved);
                let Some(world) = pose.world.get(&id) else {
                    return vec![bone.head.x, bone.head.y, 0.0];
                };
                let head = world.apply(Vec2::ZERO);
                let angle = bone_angle(world) - rest_angle;
                let (origin, local) = match parent_bone.and_then(|p| pose.world.get(&p.id).map(|w| (p, w))) {
                    Some((p, pw)) => {
                        let up = bone_angle(pw) - p.rest_angle();
                        let o = pw.apply(Vec2::ZERO);
                        (p.head + (head - o).rotated(-up), angle - up)
                    }
                    None => (head, angle),
                };
                let local = reference + skeleton::wrap_angle(local - reference);
                vec![origin.x, origin.y, local.to_degrees()]
            };
            sample(axes.clone(), &weights, &this.limits, &mut eval)
        };
        if s.capped {
            self.capped.insert(format!("bone \"{}\"", bone.name));
        }
        let unit = if under_rotation { 1.0 } else { 1.0 / self.ppu };
        let forms = self.grid(&s, |f| b::RotationForm {
            origin: self.to_local(parent_frame, Vec2::new(f[0], f[1])),
            angle: f[2],
            scale: unit,
            reflect_x: false,
            reflect_y: false,
            opacity: 1.0,
            tint: b::Tint::default(),
        });
        b::DeformerDef {
            id: unique_id(&format!("Bone_{}", bone.name), "Bone", used),
            parent_part: None,
            parent_deformer: None,
            kind: b::DeformerKind::Rotation {
                base_angle: 0.0,
                forms,
            },
        }
    }
}

/// Walking the draw tree into parts and art meshes.
struct Walk<'a> {
    doc: &'a Document,
    model: &'a aether_player::model::Model,
    uv_scale: &'a [Vec2],
    exported: &'a [Exported],
    index_of: &'a BTreeMap<Source, usize>,
    skin_home: &'a BTreeMap<LayerId, BoneId>,
    mesh_of_layer: BTreeMap<LayerId, usize>,
    parts: Vec<(String, String)>,
    transformed: Vec<String>,
    /// Layer transforms folded into root-level meshes.
    baked: BTreeMap<LayerId, Transform2D>,
}

/// Where a mesh sits among its siblings.
#[derive(Clone, Copy)]
struct Slot {
    part: Option<usize>,
    /// Sibling index (the compositor's sort key before offsets).
    index: u32,
    /// Siblings sort by keyed draw order.
    keyed: bool,
}

impl Slot {
    fn order(&self, offset: f32) -> f32 {
        if self.keyed {
            DRAW_ORDER + 10.0 * (self.index as f32 + offset)
        } else {
            DRAW_ORDER
        }
    }
}

fn keyed_draw_order(rig: &Rig, layer: LayerId) -> bool {
    rig.mesh(layer).is_some_and(|m| {
        m.keyforms.forms.iter().any(|f| f.draw_order.abs() > 1e-6)
            || m.blend_shapes
                .iter()
                .any(|s| s.grid.forms.iter().any(|f| f.draw_order.abs() > 1e-6))
    })
}

impl Walk<'_> {
    fn nodes(
        &mut self,
        ex: &mut Exporter,
        builder: &mut MocBuilder,
        nodes: &[Node],
        part: Option<usize>,
        used: &mut BTreeSet<String>,
    ) -> Result<()> {
        let rig = ex.rig;
        let keyed = nodes.iter().any(|n| match n {
            Node::Part { part, .. } => keyed_draw_order(rig, self.model.parts[*part as usize].layer),
            Node::Group { .. } => false,
        });
        for node in nodes {
            match node {
                Node::Group {
                    index,
                    name,
                    children,
                } => {
                    let slot = Slot {
                        part,
                        index: *index,
                        keyed,
                    };
                    let id = unique_id(&format!("Part_{name}"), "Part", used);
                    self.parts.push((id.clone(), name.clone()));
                    builder.parts.push(b::PartDef {
                        id,
                        parent: part,
                        group: true,
                        draw_order: b::Grid::constant(slot.order(0.0)),
                    });
                    let p = builder.parts.len() - 1;
                    builder.draw_list.push(DrawRef::Part(p));
                    self.nodes(ex, builder, children, Some(p), used)?;
                }
                Node::Part {
                    index,
                    part: base,
                    clipped,
                } => {
                    let slot = Slot {
                        part,
                        index: *index,
                        keyed,
                    };
                    let base_part = &self.model.parts[*base as usize];
                    let moves = keyed && keyed_draw_order(rig, base_part.layer);
                    let (mesh, order) = self.mesh(ex, base_part, slot, Vec::new(), used)?;
                    if moves && !clipped.is_empty() {
                        // The clipping group moves as one: a part of its own
                        // takes the base's draw order.
                        let name = self.layer_name(base_part);
                        let id = unique_id(&format!("Part_{name}_clipping"), "Part", used);
                        self.parts.push((id.clone(), format!("{name} (clipping)")));
                        builder.parts.push(b::PartDef {
                            id,
                            parent: part,
                            group: true,
                            draw_order: order.clone(),
                        });
                        let p = builder.parts.len() - 1;
                        builder.draw_list.push(DrawRef::Part(p));
                        let mut mesh = mesh;
                        mesh.parent_part = Some(p);
                        for f in &mut mesh.forms.forms {
                            f.draw_order = DRAW_ORDER;
                        }
                        let base_index = self.push(builder, base_part.layer, mesh);
                        let inner = Slot {
                            part: Some(p),
                            index: 0,
                            keyed: false,
                        };
                        for c in clipped {
                            let cp = &self.model.parts[*c as usize];
                            let (m, _) = self.mesh(ex, cp, inner, vec![base_index], used)?;
                            self.push(builder, cp.layer, m);
                        }
                    } else {
                        let base_index = self.push(builder, base_part.layer, mesh);
                        for c in clipped {
                            let cp = &self.model.parts[*c as usize];
                            let (mut m, _) = self.mesh(ex, cp, slot, vec![base_index], used)?;
                            // Clipped layers ride on their base.
                            let o = slot.order(0.0);
                            for f in &mut m.forms.forms {
                                f.draw_order = o;
                            }
                            self.push(builder, cp.layer, m);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn push(&mut self, builder: &mut MocBuilder, layer: LayerId, mesh: b::MeshDef) -> usize {
        builder.meshes.push(mesh);
        let i = builder.meshes.len() - 1;
        builder.draw_list.push(DrawRef::Mesh(i));
        self.mesh_of_layer.insert(layer, i);
        i
    }

    fn layer_name(&self, part: &Part) -> String {
        if !part.name.is_empty() {
            return part.name.clone();
        }
        self.doc
            .layers
            .get(part.layer)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| part.layer.to_string())
    }

    /// An art mesh for a runtime part, and its draw order grid.
    fn mesh(
        &mut self,
        ex: &mut Exporter,
        part: &Part,
        slot: Slot,
        masks: Vec<usize>,
        used: &mut BTreeSet<String>,
    ) -> Result<(b::MeshDef, b::Grid<f32>)> {
        let rig = ex.rig;
        let name = self.layer_name(part);
        let n = part.vertices.len();
        if n > u16::MAX as usize {
            return Err(AetherError::serialization(format!(
                "\"{name}\" has {n} vertices; Cubism allows at most 65535 per mesh"
            )));
        }
        let scale = self
            .uv_scale
            .get(part.texture as usize)
            .copied()
            .unwrap_or(Vec2::ONE);
        let uvs = part
            .uvs
            .iter()
            .map(|uv| [uv.x * scale.x, uv.y * scale.y])
            .collect();
        let triangles = part
            .triangles
            .iter()
            .map(|t| [t[0] as u16, t[1] as u16, t[2] as u16])
            .collect();
        let transform = part.transform.unwrap_or(Transform2D::IDENTITY);
        let blend = match part.blend {
            BlendKind::Normal => b::MeshBlend::Normal,
            BlendKind::Multiply => b::MeshBlend::Multiply,
            BlendKind::Add => b::MeshBlend::Add,
            BlendKind::Screen => b::MeshBlend::Code(aether_live2d::moc3::BLEND_SCREEN),
        };
        if part.blend == BlendKind::Screen && ex.options.version < aether_live2d::moc3::VERSION_53 {
            ex.notes.push(format!(
                "\"{name}\" uses screen blending, which Cubism has only from 5.3; it is drawn as add"
            ));
        }
        let static_opacity = part.opacity;

        let (forms, parent_deformer, order) = match rig.mesh(part.layer) {
            None => {
                let positions: Vec<f32> = part
                    .vertices
                    .iter()
                    .flat_map(|v| ex.to_local(Frame::Root, transform.apply(*v)))
                    .collect();
                let o = slot.order(0.0);
                (
                    b::Grid::constant(b::MeshForm {
                        positions,
                        opacity: static_opacity,
                        draw_order: o,
                        tint: b::Tint::default(),
                    }),
                    None,
                    b::Grid::constant(o),
                )
            }
            Some(mesh) => {
                let home = self.skin_home.get(&part.layer).copied();
                let parent = match (mesh.parent, home) {
                    (Some(NodeRef::Deformer(d)), _) => self.index_of.get(&Source::Deformer(d)).copied(),
                    (Some(NodeRef::Bone(b)), _) => self.index_of.get(&Source::Bone(b)).copied(),
                    (None, Some(b)) => self.index_of.get(&Source::Bone(b)).copied(),
                    (None, None) => None,
                };
                let frame = parent.map(|i| self.exported[i].frame).unwrap_or(Frame::Root);
                let apply_transform = !transform.is_identity();
                if apply_transform && parent.is_some() {
                    self.transformed.push(name.clone());
                } else if apply_transform {
                    self.baked.insert(part.layer, transform);
                }
                let mut direct = BTreeMap::new();
                grid_params(&mesh.keyforms, &mut direct);
                for s in &mesh.blend_shapes {
                    grid_params(&s.grid, &mut direct);
                }
                let skinned = mesh.skin.as_ref().is_some_and(|s| !s.bones.is_empty());
                if let Some(skin) = mesh.skin.as_ref().filter(|_| skinned) {
                    for &bone in &skin.bones {
                        match home {
                            // Relative to the home bone: only the bones
                            // below it matter.
                            Some(h) => {
                                let mut current = Some(bone);
                                let mut steps = 0;
                                while let Some(c) = current {
                                    steps += 1;
                                    if c == h || steps > rig.bones.len() {
                                        break;
                                    }
                                    ex.bone_local_params(c, &mut direct);
                                    current = rig.bone(c).and_then(|x| x.parent);
                                }
                            }
                            None => ex.bone_world_params(bone, &mut direct),
                        }
                    }
                }
                let axes = ex.axes(&direct);
                let mut weights = vec![1.0; 2 * n];
                weights.extend([COLOR_WEIGHT; 7]);
                weights.push(0.0);
                let home_bone = home.and_then(|h| rig.bone(h));
                let s = {
                    let this = &*ex;
                    let mut eval = |at: &[f32]| -> Vec<f32> {
                        let (values, resolved) = this.resolved(&axes, at);
                        let (form, local) = if skinned {
                            let ev = Evaluator::with_values(rig, &values);
                            let (form, mut local) = ev.mesh_local(mesh);
                            if let Some(h) = home_bone {
                                if let Some(w) = ev.skeleton().world.get(&h.id) {
                                    let turn = bone_angle(w) - h.rest_angle();
                                    let o = w.apply(Vec2::ZERO);
                                    for p in &mut local {
                                        *p = h.head + (*p - o).rotated(-turn);
                                    }
                                }
                            }
                            (form, local)
                        } else {
                            let form = mesh.evaluate_form(&resolved);
                            let local = mesh
                                .vertices
                                .iter()
                                .zip(&form.offsets)
                                .map(|(v, o)| *v + *o)
                                .collect();
                            (form, local)
                        };
                        let mut out = Vec::with_capacity(2 * n + 8);
                        for p in &local {
                            let p = if apply_transform && parent.is_none() {
                                transform.apply(*p)
                            } else {
                                *p
                            };
                            out.extend([p.x, p.y]);
                        }
                        out.push(form.opacity);
                        out.extend(form.multiply);
                        out.extend(form.screen);
                        out.push(slot.order(form.draw_order));
                        out
                    };
                    sample(axes.clone(), &weights, &this.limits, &mut eval)
                };
                if s.capped {
                    ex.capped.insert(format!("\"{name}\""));
                }
                let forms = ex.grid(&s, |f| b::MeshForm {
                    positions: ex.positions(frame, &f[..2 * n]),
                    opacity: (f[2 * n] * static_opacity).clamp(0.0, 1.0),
                    draw_order: f[2 * n + 7],
                    tint: b::Tint {
                        multiply: [f[2 * n + 1], f[2 * n + 2], f[2 * n + 3]],
                        screen: [f[2 * n + 4], f[2 * n + 5], f[2 * n + 6]],
                    },
                });
                let order = ex.grid(&s, |f| f[2 * n + 7]);
                (forms, parent, order)
            }
        };
        let mesh = b::MeshDef {
            id: unique_id(&format!("ArtMesh_{name}"), "ArtMesh", used),
            parent_part: slot.part,
            parent_deformer,
            texture: part.texture as usize,
            uvs,
            triangles,
            masks,
            inverted_mask: false,
            double_sided: true,
            blend,
            forms,
        };
        Ok((mesh, order))
    }
}

/// Evaluate the built model as Cubism does and compare it with the editor
/// at the defaults, at each parameter's extremes and at random poses.
/// Returns the largest vertex difference, in pixels.
fn check(
    doc: &Document,
    ex: &Exporter,
    moc: &Moc,
    mesh_of_layer: &BTreeMap<LayerId, usize>,
    transforms: &BTreeMap<LayerId, Transform2D>,
    notes: &mut Vec<String>,
) -> f32 {
    let rig = ex.rig;
    let mut model = aether_live2d::Model::new(Arc::new(moc.clone()));
    let mut poses: Vec<ParamValues> = vec![ParamValues::new()];
    for p in &rig.parameters {
        for v in [p.min, p.max] {
            poses.push(ParamValues::from([(p.id, v)]));
        }
    }
    let mut seed = 0x2545_f491_u32;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    for _ in 0..24 {
        poses.push(
            rig.parameters
                .iter()
                .map(|p| (p.id, p.min + random() * (p.max - p.min)))
                .collect(),
        );
    }

    let mut worst: BTreeMap<LayerId, (f32, f32)> = BTreeMap::new();
    for free in &poses {
        for (i, p) in rig.parameters.iter().enumerate() {
            model.parameter_values[i] = free.get(&p.id).copied().unwrap_or(p.default);
        }
        model.update();
        let values = ex.effective(free);
        let pose = Evaluator::with_values(rig, &values).pose();
        for (&layer, &d) in mesh_of_layer {
            let Some(ours) = pose.meshes.get(&layer) else {
                continue;
            };
            if ours.opacity < 0.01 && model.drawable_opacity(d) < 0.01 {
                continue;
            }
            let theirs = model.drawable_positions_px(d);
            let t = transforms.get(&layer);
            let mut e = 0.0f32;
            for (a, b) in ours.positions.iter().zip(&theirs) {
                let a = t.map(|t| t.apply(*a)).unwrap_or(*a);
                e = e.max(a.distance(Vec2::new(b[0], b[1])));
            }
            let entry = worst.entry(layer).or_insert((0.0, 0.0));
            entry.0 = entry.0.max(e);
            entry.1 = entry.1.max((ours.opacity - model.drawable_opacity(d)).abs());
        }
    }
    let limit = (ex.limits.tolerance * 4.0).max(1.0);
    let mut reported = 0;
    let mut max_error = 0.0f32;
    let mut by_error: Vec<(LayerId, (f32, f32))> = worst.into_iter().collect();
    by_error.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
    for (layer, (e, o)) in by_error {
        max_error = max_error.max(e);
        if (e > limit || o > 0.05) && reported < 12 {
            reported += 1;
            let name = doc
                .layers
                .get(layer)
                .map(|l| l.name.clone())
                .unwrap_or_else(|| layer.to_string());
            if e > limit {
                notes.push(format!(
                    "\"{name}\" deforms up to {e:.1} px differently in Cubism than in Aether"
                ));
            } else {
                notes.push(format!(
                    "\"{name}\" fades differently in Cubism (opacity off by up to {o:.2})"
                ));
            }
        }
    }
    max_error
}
