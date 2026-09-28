//! Art meshes.
//!
//! An [`ArtMesh`] lays a triangle mesh over one raster layer. The mesh's rest
//! vertices are in document coordinates and double as texture coordinates:
//! wherever a vertex sits at rest is where it reads the layer's pixels from.
//! Deformation moves vertices away from rest, and the layer is redrawn through
//! the moved triangles.
//!
//! Keeping the artwork in the layer rather than copying it into the rig is the
//! point. Repainting a rigged layer — even mid-animation — shows up
//! immediately through the deformation, with no re-import or texture-atlas
//! rebuild step.
//!
//! What a mesh does at a pose is a [`MeshForm`]: per-vertex offsets from rest
//! plus opacity, multiply/screen tint and a draw-order offset, all keyable.
//! Topology edits (adding, deleting or re-meshing vertices) carry every
//! keyform along by barycentric interpolation, so re-meshing never throws away
//! deformation work.

use crate::flat;
use crate::geom::{self, Location};
use crate::keyform::{Blend, BlendShape, KeyformGrid, ParamSource};
use crate::NodeRef;
use aether_core::math::{Rect, Vec2};
use aether_core::{AetherError, BoneId, LayerId, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn one() -> f32 {
    1.0
}

fn white() -> [f32; 3] {
    [1.0; 3]
}

fn yes() -> bool {
    true
}

/// A mesh's shape and appearance at one keyform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshForm {
    /// Offset of each vertex from its rest position.
    #[serde(with = "flat")]
    pub offsets: Vec<Vec2>,
    /// Opacity multiplier.
    #[serde(default = "one")]
    pub opacity: f32,
    /// Multiply tint per channel (white = none).
    #[serde(default = "white")]
    pub multiply: [f32; 3],
    /// Screen tint per channel (black = none).
    #[serde(default)]
    pub screen: [f32; 3],
    /// Moves the layer up (positive) or down among its siblings.
    #[serde(default)]
    pub draw_order: f32,
}

impl MeshForm {
    /// The undeformed, untinted form for `n` vertices.
    pub fn rest(n: usize) -> Self {
        Self {
            offsets: vec![Vec2::ZERO; n],
            opacity: 1.0,
            multiply: [1.0; 3],
            screen: [0.0; 3],
            draw_order: 0.0,
        }
    }

    /// A blend-shape delta that changes nothing.
    pub fn zero_delta(n: usize) -> Self {
        Self {
            offsets: vec![Vec2::ZERO; n],
            opacity: 0.0,
            multiply: [0.0; 3],
            screen: [0.0; 3],
            draw_order: 0.0,
        }
    }

    /// True when the form leaves the layer exactly as painted.
    pub fn is_rest(&self) -> bool {
        self.offsets.iter().all(|o| o.length_squared() < 1e-10)
            && (self.opacity - 1.0).abs() < 1e-6
            && self.multiply.iter().all(|c| (c - 1.0).abs() < 1e-6)
            && self.screen.iter().all(|c| c.abs() < 1e-6)
            && self.draw_order.abs() < 1e-6
    }
}

impl Blend for MeshForm {
    fn zeroed(&self) -> Self {
        Self::zero_delta(self.offsets.len())
    }

    fn add_scaled(&mut self, other: &Self, weight: f32) {
        for (a, b) in self.offsets.iter_mut().zip(&other.offsets) {
            *a += *b * weight;
        }
        self.opacity += other.opacity * weight;
        for c in 0..3 {
            self.multiply[c] += other.multiply[c] * weight;
            self.screen[c] += other.screen[c] * weight;
        }
        self.draw_order += other.draw_order * weight;
    }
}

/// Bone weights for linear blend skinning.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Skin {
    /// Bones that influence this mesh.
    pub bones: Vec<BoneId>,
    /// Vertex-major weights: `weights[v * bones.len() + b]`.
    pub weights: Vec<f32>,
}

impl Skin {
    /// An empty skin for `vertices` vertices over `bones`.
    pub fn new(bones: Vec<BoneId>, vertices: usize) -> Self {
        let weights = vec![0.0; bones.len() * vertices];
        Self { bones, weights }
    }

    /// Weight of `bone_slot` on vertex `v`.
    pub fn weight(&self, v: usize, bone_slot: usize) -> f32 {
        self.weights
            .get(v * self.bones.len() + bone_slot)
            .copied()
            .unwrap_or(0.0)
    }

    /// Set a weight (unnormalised).
    pub fn set_weight(&mut self, v: usize, bone_slot: usize, w: f32) {
        let n = self.bones.len();
        if bone_slot < n {
            if let Some(slot) = self.weights.get_mut(v * n + bone_slot) {
                *slot = w.max(0.0);
            }
        }
    }

    /// The weights of one vertex.
    pub fn vertex(&self, v: usize) -> &[f32] {
        let n = self.bones.len();
        self.weights.get(v * n..(v + 1) * n).unwrap_or(&[])
    }

    /// Make every vertex's weights sum to one (vertices with no weight stay
    /// unskinned).
    pub fn normalize(&mut self) {
        let n = self.bones.len();
        if n == 0 {
            return;
        }
        for chunk in self.weights.chunks_mut(n) {
            let sum: f32 = chunk.iter().sum();
            if sum > 1e-6 {
                for w in chunk.iter_mut() {
                    *w /= sum;
                }
            }
        }
    }

    /// Slot index of `bone`, adding it if needed.
    pub fn slot_for(&mut self, bone: BoneId) -> usize {
        if let Some(i) = self.bones.iter().position(|b| *b == bone) {
            return i;
        }
        let old = self.bones.len();
        let vertices = if old == 0 { 0 } else { self.weights.len() / old };
        let mut weights = Vec::with_capacity((old + 1) * vertices);
        for v in 0..vertices {
            weights.extend_from_slice(&self.weights[v * old..(v + 1) * old]);
            weights.push(0.0);
        }
        self.weights = weights;
        self.bones.push(bone);
        old
    }

    fn resize_vertices(&mut self, vertices: usize) {
        self.weights.resize(vertices * self.bones.len(), 0.0);
    }
}

/// Spring-driven secondary motion on a mesh's vertices.
///
/// Each vertex with a non-zero weight lags behind where the rig puts it and
/// springs back, so soft parts — cheeks, a ribbon, the tip of a ponytail —
/// wobble on their own when the rig moves them. Weights are painted per
/// vertex: 0 is pinned, 1 is fully free.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Jiggle {
    /// Whether the simulation runs.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Spring strength pulling a vertex back to its target (per second²).
    pub stiffness: f32,
    /// Fraction of velocity lost per second.
    pub damping: f32,
    /// Constant pull, in document pixels per second², e.g. `(0, 400)` to sag.
    #[serde(default)]
    pub gravity: Vec2,
    /// Largest distance a vertex may lag behind, in pixels.
    pub max_offset: f32,
    /// Per-vertex freedom in `0..=1`.
    pub weights: Vec<f32>,
}

impl Jiggle {
    /// A lively default for `vertices` vertices, all free.
    pub fn new(vertices: usize) -> Self {
        Self {
            enabled: true,
            stiffness: 180.0,
            damping: 6.0,
            gravity: Vec2::ZERO,
            max_offset: 40.0,
            weights: vec![1.0; vertices],
        }
    }
}

/// A triangle mesh bound to a raster layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtMesh {
    /// The layer whose pixels the mesh carries.
    pub layer: LayerId,
    /// Display name (empty means "use the layer's name").
    #[serde(default)]
    pub name: String,
    /// The deformer or bone this mesh follows.
    #[serde(default)]
    pub parent: Option<NodeRef>,
    /// Rest positions in document space; also the texture coordinates.
    #[serde(with = "flat")]
    pub vertices: Vec<Vec2>,
    /// Triangles as vertex index triples.
    pub triangles: Vec<[u32; 3]>,
    /// Keyforms over the mesh's parameters.
    pub keyforms: KeyformGrid<MeshForm>,
    /// Additive corrections.
    #[serde(default)]
    pub blend_shapes: Vec<BlendShape<MeshForm>>,
    /// Bone weights, when the mesh is skinned.
    #[serde(default)]
    pub skin: Option<Skin>,
    /// Secondary motion.
    #[serde(default)]
    pub jiggle: Option<Jiggle>,
    /// Glue: vertex pairs to hold together, `(this vertex, other layer, its
    /// vertex, strength)`, so separately meshed parts do not tear apart at
    /// their seams.
    #[serde(default)]
    pub glue: Vec<GluePoint>,
}

/// One glued vertex pair.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GluePoint {
    /// Vertex on this mesh.
    pub vertex: u32,
    /// The other mesh's layer.
    pub other: LayerId,
    /// Vertex on the other mesh.
    pub other_vertex: u32,
    /// How strongly this vertex is pulled to the other, `0..=1`. Two glued
    /// meshes at 0.5 each meet halfway.
    pub strength: f32,
}

impl ArtMesh {
    /// A mesh with the given topology and a single rest keyform.
    pub fn new(layer: LayerId, vertices: Vec<Vec2>, triangles: Vec<[u32; 3]>) -> Self {
        let n = vertices.len();
        Self {
            layer,
            name: String::new(),
            parent: None,
            vertices,
            triangles,
            keyforms: KeyformGrid::constant(MeshForm::rest(n)),
            blend_shapes: Vec::new(),
            skin: None,
            jiggle: None,
            glue: Vec::new(),
        }
    }

    /// A quad of two triangles covering `rect` — the simplest useful mesh.
    pub fn quad(layer: LayerId, rect: Rect) -> Self {
        let vertices = vec![
            rect.min,
            Vec2::new(rect.max.x, rect.min.y),
            rect.max,
            Vec2::new(rect.min.x, rect.max.y),
        ];
        Self::new(layer, vertices, vec![[0, 1, 2], [0, 2, 3]])
    }

    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Rest bounding box.
    pub fn bounds(&self) -> Rect {
        bounds_of(&self.vertices)
    }

    /// Check indices and per-vertex array lengths.
    pub fn validate(&self) -> Result<()> {
        let n = self.vertices.len();
        if self.triangles.iter().flatten().any(|&i| i as usize >= n) {
            return Err(AetherError::rig(format!(
                "a triangle of the mesh on {} refers to a missing vertex",
                self.layer
            )));
        }
        self.keyforms.validate()?;
        for form in &self.keyforms.forms {
            if form.offsets.len() != n {
                return Err(AetherError::rig(format!(
                    "a keyform of the mesh on {} has {} offsets for {} vertices",
                    self.layer,
                    form.offsets.len(),
                    n
                )));
            }
        }
        for shape in &self.blend_shapes {
            shape.grid.validate()?;
            if shape.grid.forms.iter().any(|f| f.offsets.len() != n) {
                return Err(AetherError::rig(format!(
                    "blend shape '{}' does not match its mesh's vertex count",
                    shape.name
                )));
            }
        }
        if let Some(skin) = &self.skin {
            if skin.weights.len() != skin.bones.len() * n {
                return Err(AetherError::rig("skin weights do not match the vertex count"));
            }
        }
        if let Some(jiggle) = &self.jiggle {
            if jiggle.weights.len() != n {
                return Err(AetherError::rig("jiggle weights do not match the vertex count"));
            }
        }
        Ok(())
    }

    /// Repair per-vertex arrays whose lengths drifted (from an older file or a
    /// plugin), padding with neutral values rather than refusing to load.
    pub fn repair(&mut self) {
        let n = self.vertices.len();
        self.triangles.retain(|t| t.iter().all(|&i| (i as usize) < n));
        if self.keyforms.validate().is_err() {
            self.keyforms = KeyformGrid::constant(MeshForm::rest(n));
        }
        self.keyforms.for_each_form(|f| f.offsets.resize(n, Vec2::ZERO));
        self.blend_shapes.retain(|s| s.grid.validate().is_ok());
        for shape in &mut self.blend_shapes {
            shape.grid.for_each_form(|f| f.offsets.resize(n, Vec2::ZERO));
        }
        if let Some(skin) = &mut self.skin {
            skin.resize_vertices(n);
        }
        if let Some(jiggle) = &mut self.jiggle {
            jiggle.weights.resize(n, 1.0);
        }
        self.glue.retain(|g| (g.vertex as usize) < n);
    }

    /// The form for the current parameters: base grid plus blend shapes.
    pub fn evaluate_form(&self, source: &dyn ParamSource) -> MeshForm {
        let mut form = self.keyforms.evaluate(source);
        for shape in &self.blend_shapes {
            shape.accumulate(&mut form, source);
        }
        form.opacity = form.opacity.clamp(0.0, 1.0);
        for c in 0..3 {
            form.multiply[c] = form.multiply[c].clamp(0.0, 2.0);
            form.screen[c] = form.screen[c].clamp(0.0, 1.0);
        }
        form
    }

    /// Index of the vertex nearest `p` among `positions`, within `radius`.
    pub fn nearest_vertex(positions: &[Vec2], p: Vec2, radius: f32) -> Option<usize> {
        let mut best = None;
        let mut best_d = radius;
        for (i, v) in positions.iter().enumerate() {
            let d = v.distance(p);
            if d <= best_d {
                best_d = d;
                best = Some(i);
            }
        }
        best
    }

    /// Boundary edges: edges used by exactly one triangle, as `(a, b)` in the
    /// winding of that triangle.
    pub fn boundary_edges(&self) -> Vec<(u32, u32)> {
        let mut count: BTreeMap<(u32, u32), (usize, (u32, u32))> = BTreeMap::new();
        for tri in &self.triangles {
            for e in 0..3 {
                let (a, b) = (tri[e], tri[(e + 1) % 3]);
                count
                    .entry((a.min(b), a.max(b)))
                    .and_modify(|c| c.0 += 1)
                    .or_insert((1, (a, b)));
            }
        }
        count
            .into_values()
            .filter(|(c, _)| *c == 1)
            .map(|(_, e)| e)
            .collect()
    }

    /// Interpolation weights over existing vertices for a rest position `p`.
    fn weights_at(&self, p: Vec2) -> Vec<(usize, f32)> {
        match geom::locate(p, &self.vertices, &self.triangles) {
            Some(location) => {
                let (t, w) = location.parts();
                let tri = self.triangles[t];
                (0..3).map(|i| (tri[i] as usize, w[i])).collect()
            }
            None => {
                // No triangles yet: copy the nearest vertex's data.
                Self::nearest_vertex(&self.vertices, p, f32::INFINITY)
                    .map(|i| vec![(i, 1.0)])
                    .unwrap_or_default()
            }
        }
    }

    /// Append a vertex whose per-vertex data is interpolated from `weights`.
    fn push_interpolated_vertex(&mut self, p: Vec2, weights: &[(usize, f32)]) -> usize {
        let blend = |values: &[Vec2]| -> Vec2 {
            weights.iter().fold(Vec2::ZERO, |acc, &(i, w)| {
                acc + values.get(i).copied().unwrap_or(Vec2::ZERO) * w
            })
        };
        self.keyforms.for_each_form(|f| {
            let v = blend(&f.offsets);
            f.offsets.push(v);
        });
        for shape in &mut self.blend_shapes {
            shape.grid.for_each_form(|f| {
                let v = blend(&f.offsets);
                f.offsets.push(v);
            });
        }
        let n_before = self.vertices.len();
        if let Some(skin) = &mut self.skin {
            let bones = skin.bones.len();
            for b in 0..bones {
                let w: f32 = weights.iter().map(|&(i, w)| skin.weight(i, b) * w).sum();
                skin.weights.push(w);
            }
        }
        if let Some(jiggle) = &mut self.jiggle {
            let w: f32 = weights
                .iter()
                .map(|&(i, w)| jiggle.weights.get(i).copied().unwrap_or(1.0) * w)
                .sum();
            jiggle.weights.push(w.clamp(0.0, 1.0));
        }
        self.vertices.push(p);
        n_before
    }

    /// Add a vertex at rest position `p`, keeping every keyform consistent.
    ///
    /// Inside the mesh the containing triangle is split in three; outside it,
    /// the vertex is joined to the nearest boundary edge.
    pub fn add_vertex(&mut self, p: Vec2) -> Result<usize> {
        if !p.is_finite() {
            return Err(AetherError::rig("vertex position is not a finite number"));
        }
        if let Some(existing) = Self::nearest_vertex(&self.vertices, p, 0.5) {
            return Ok(existing);
        }
        let location = geom::locate(p, &self.vertices, &self.triangles);
        let weights = self.weights_at(p);
        let index = self.push_interpolated_vertex(p, &weights) as u32;
        match location {
            Some(Location::Inside { triangle, weights }) => {
                let [a, b, c] = self.triangles[triangle];
                let on_edge = weights.iter().position(|w| w.abs() < 1e-4).map(|k| match k {
                    0 => (b, c),
                    1 => (c, a),
                    _ => (a, b),
                });
                match on_edge {
                    // On a shared edge: split every triangle using that edge,
                    // or the neighbour would keep a T-junction and crack.
                    Some((u, v)) => {
                        let mut split = Vec::new();
                        for t in &mut self.triangles {
                            let Some(k) = (0..3).find(|&k| {
                                let (p, q) = (t[k], t[(k + 1) % 3]);
                                (p == u && q == v) || (p == v && q == u)
                            }) else {
                                continue;
                            };
                            let (p, q, r) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                            *t = [p, index, r];
                            split.push([index, q, r]);
                        }
                        self.triangles.extend(split);
                    }
                    None => {
                        self.triangles[triangle] = [a, b, index];
                        self.triangles.push([b, c, index]);
                        self.triangles.push([c, a, index]);
                    }
                }
            }
            _ => {
                let edges = self.boundary_edges();
                let nearest = edges.iter().min_by(|x, y| {
                    let dx =
                        geom::closest_on_segment(p, self.vertices[x.0 as usize], self.vertices[x.1 as usize])
                            .0
                            .distance(p);
                    let dy =
                        geom::closest_on_segment(p, self.vertices[y.0 as usize], self.vertices[y.1 as usize])
                            .0
                            .distance(p);
                    dx.total_cmp(&dy)
                });
                if let Some(&(a, b)) = nearest {
                    if geom::orient(self.vertices[a as usize], self.vertices[b as usize], p).abs() > 1e-9 {
                        // Reverse the edge so the new triangle winds like its neighbour.
                        self.triangles.push([b, a, index]);
                    }
                } else if self.vertices.len() >= 3 && self.triangles.is_empty() {
                    self.triangles = geom::delaunay(&self.vertices);
                }
            }
        }
        Ok(index as usize)
    }

    /// Delete a vertex and re-fill the hole it leaves.
    pub fn remove_vertex(&mut self, index: usize) -> Result<()> {
        let n = self.vertices.len();
        if index >= n {
            return Err(AetherError::rig("no such vertex"));
        }
        if n <= 3 {
            return Err(AetherError::rig("a mesh needs at least three vertices"));
        }
        let idx = index as u32;
        // The ring of neighbours, walked in order around the vertex.
        let fan: Vec<[u32; 3]> = self
            .triangles
            .iter()
            .copied()
            .filter(|t| t.contains(&idx))
            .collect();
        let mut next: BTreeMap<u32, u32> = BTreeMap::new();
        for t in &fan {
            let k = t.iter().position(|&v| v == idx).unwrap_or(0);
            next.insert(t[(k + 1) % 3], t[(k + 2) % 3]);
        }
        let mut ring: Vec<u32> = Vec::new();
        // An open fan (boundary vertex) starts at a neighbour nobody points to.
        let pointed: Vec<u32> = next.values().copied().collect();
        let start = next
            .keys()
            .copied()
            .find(|k| !pointed.contains(k))
            .or_else(|| next.keys().next().copied());
        if let Some(mut current) = start {
            for _ in 0..=next.len() {
                ring.push(current);
                match next.get(&current) {
                    Some(&n) if !ring.contains(&n) => current = n,
                    Some(_) => break,
                    None => break,
                }
            }
        }
        self.triangles.retain(|t| !t.contains(&idx));
        let patch = geom::ear_clip(&self.vertices, &ring);
        // Keep the patch's winding consistent with the rest of the mesh.
        let reference = fan.first().map(|t| {
            geom::orient(
                self.vertices[t[0] as usize],
                self.vertices[t[1] as usize],
                self.vertices[t[2] as usize],
            ) > 0.0
        });
        for mut t in patch {
            let ccw = geom::orient(
                self.vertices[t[0] as usize],
                self.vertices[t[1] as usize],
                self.vertices[t[2] as usize],
            ) > 0.0;
            if Some(ccw) != reference && reference.is_some() {
                t.swap(1, 2);
            }
            self.triangles.push(t);
        }

        // Drop the vertex from every per-vertex array and renumber.
        self.vertices.remove(index);
        self.keyforms.for_each_form(|f| {
            f.offsets.remove(index);
        });
        for shape in &mut self.blend_shapes {
            shape.grid.for_each_form(|f| {
                f.offsets.remove(index);
            });
        }
        if let Some(skin) = &mut self.skin {
            let b = skin.bones.len();
            skin.weights.drain(index * b..(index + 1) * b);
        }
        if let Some(jiggle) = &mut self.jiggle {
            jiggle.weights.remove(index);
        }
        self.glue.retain(|g| g.vertex != idx);
        for g in &mut self.glue {
            if g.vertex > idx {
                g.vertex -= 1;
            }
        }
        for t in &mut self.triangles {
            for v in t.iter_mut() {
                if *v > idx {
                    *v -= 1;
                }
            }
        }
        Ok(())
    }

    /// Move a vertex's rest position. Because rest positions are texture
    /// coordinates, this changes which pixels the vertex carries.
    pub fn move_rest_vertex(&mut self, index: usize, p: Vec2) -> Result<()> {
        let v = self
            .vertices
            .get_mut(index)
            .ok_or_else(|| AetherError::rig("no such vertex"))?;
        if !p.is_finite() {
            return Err(AetherError::rig("vertex position is not a finite number"));
        }
        *v = p;
        Ok(())
    }

    /// Replace the topology, carrying every keyform, blend shape, skin weight
    /// and jiggle weight across by interpolating the old mesh at each new
    /// vertex.
    pub fn retopologize(&mut self, vertices: Vec<Vec2>, triangles: Vec<[u32; 3]>) {
        let weights: Vec<Vec<(usize, f32)>> = vertices.iter().map(|p| self.weights_at(*p)).collect();
        let resample = |values: &[Vec2]| -> Vec<Vec2> {
            weights
                .iter()
                .map(|ws| {
                    ws.iter().fold(Vec2::ZERO, |acc, &(i, w)| {
                        acc + values.get(i).copied().unwrap_or(Vec2::ZERO) * w
                    })
                })
                .collect()
        };
        self.keyforms.for_each_form(|f| f.offsets = resample(&f.offsets));
        for shape in &mut self.blend_shapes {
            shape.grid.for_each_form(|f| f.offsets = resample(&f.offsets));
        }
        if let Some(skin) = &mut self.skin {
            let b = skin.bones.len();
            let mut new_weights = Vec::with_capacity(vertices.len() * b);
            for ws in &weights {
                for slot in 0..b {
                    new_weights.push(ws.iter().map(|&(i, w)| skin.weight(i, slot) * w).sum());
                }
            }
            skin.weights = new_weights;
            skin.normalize();
        }
        if let Some(jiggle) = &mut self.jiggle {
            jiggle.weights = weights
                .iter()
                .map(|ws| {
                    ws.iter()
                        .map(|&(i, w)| jiggle.weights.get(i).copied().unwrap_or(1.0) * w)
                        .sum::<f32>()
                        .clamp(0.0, 1.0)
                })
                .collect();
        }
        // Glue refers to vertex indices that no longer exist.
        self.glue.clear();
        self.vertices = vertices;
        self.triangles = triangles;
    }

    /// Unique undirected edges, for wireframe drawing.
    pub fn edges(&self) -> Vec<(u32, u32)> {
        let mut edges: Vec<(u32, u32)> = self
            .triangles
            .iter()
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        edges.sort_unstable();
        edges.dedup();
        edges
    }
}

/// Bounding box of a point list.
pub fn bounds_of(points: &[Vec2]) -> Rect {
    let mut iter = points.iter().filter(|p| p.is_finite());
    let Some(first) = iter.next() else {
        return Rect::ZERO;
    };
    let mut min = *first;
    let mut max = *first;
    for p in iter {
        min = min.min(*p);
        max = max.max(*p);
    }
    Rect { min, max }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyform::{AxisInput, KeyAxis};
    use aether_core::math::vec2;
    use aether_core::ParameterId;

    fn square() -> ArtMesh {
        ArtMesh::quad(LayerId(1), Rect::from_corners(vec2(0.0, 0.0), vec2(10.0, 10.0)))
    }

    fn keyed_square() -> ArtMesh {
        let mut mesh = square();
        mesh.keyforms
            .add_axis(KeyAxis::new(ParameterId(7), [0.0, 1.0]).expect("axis"))
            .expect("add axis");
        // At key 1 the whole square moves right by 4.
        for o in &mut mesh.keyforms.forms[1].offsets {
            *o = vec2(4.0, 0.0);
        }
        mesh
    }

    fn at(value: f32) -> impl ParamSource {
        move |_id: ParameterId| Some(AxisInput { value, cycle: None })
    }

    #[test]
    fn adding_a_vertex_inside_splits_a_triangle_and_interpolates_forms() {
        let mut mesh = keyed_square();
        let i = mesh.add_vertex(vec2(6.0, 3.0)).expect("add");
        mesh.validate().expect("valid");
        assert_eq!(mesh.triangles.len(), 4);
        assert_eq!(mesh.keyforms.forms[1].offsets[i], vec2(4.0, 0.0));
        let form = mesh.evaluate_form(&at(0.5));
        assert!((form.offsets[i].x - 2.0).abs() < 1e-5);
    }

    #[test]
    fn adding_a_vertex_outside_attaches_it_to_the_boundary() {
        let mut mesh = square();
        let i = mesh.add_vertex(vec2(15.0, 5.0)).expect("add");
        mesh.validate().expect("valid");
        assert_eq!(mesh.triangles.len(), 3);
        assert!(mesh.triangles.iter().any(|t| t.contains(&(i as u32))));
    }

    #[test]
    fn removing_a_vertex_refills_the_hole() {
        let mut mesh = keyed_square();
        let centre = mesh.add_vertex(vec2(5.0, 5.0)).expect("add");
        assert_eq!(mesh.triangles.len(), 4);
        mesh.remove_vertex(centre).expect("remove");
        mesh.validate().expect("valid");
        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.triangles.len(), 2, "the hole is re-filled");
        let area: f64 = mesh
            .triangles
            .iter()
            .map(|t| {
                geom::orient(
                    mesh.vertices[t[0] as usize],
                    mesh.vertices[t[1] as usize],
                    mesh.vertices[t[2] as usize],
                )
                .abs()
                    * 0.5
            })
            .sum();
        assert!((area - 100.0).abs() < 1e-3);
        assert!(mesh.remove_vertex(0).is_ok());
        assert!(mesh.remove_vertex(0).is_err(), "three vertices is the minimum");
    }

    #[test]
    fn retopologizing_keeps_the_deformation() {
        let mut mesh = keyed_square();
        let vertices = vec![
            vec2(0.0, 0.0),
            vec2(5.0, 0.0),
            vec2(10.0, 0.0),
            vec2(0.0, 10.0),
            vec2(5.0, 10.0),
            vec2(10.0, 10.0),
        ];
        let triangles = geom::delaunay(&vertices);
        mesh.retopologize(vertices, triangles);
        mesh.validate().expect("valid");
        assert!(mesh.keyforms.forms[1]
            .offsets
            .iter()
            .all(|o| (o.x - 4.0).abs() < 1e-5));
    }

    #[test]
    fn skin_slots_grow_without_losing_weights() {
        let mut skin = Skin::new(vec![BoneId(1)], 2);
        skin.set_weight(1, 0, 0.5);
        let slot = skin.slot_for(BoneId(2));
        assert_eq!(slot, 1);
        skin.set_weight(1, 1, 1.5);
        assert_eq!(skin.weight(1, 0), 0.5);
        skin.normalize();
        assert!((skin.vertex(1).iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert_eq!(skin.vertex(0), &[0.0, 0.0], "unweighted vertices stay unskinned");
    }

    #[test]
    fn repair_pads_mismatched_arrays() {
        let mut mesh = keyed_square();
        mesh.keyforms.forms[0].offsets.pop();
        mesh.triangles.push([0, 1, 42]);
        assert!(mesh.validate().is_err());
        mesh.repair();
        mesh.validate().expect("repaired");
    }

    #[test]
    fn forms_blend_every_channel() {
        let mut a = MeshForm::rest(1);
        a.multiply = [0.0; 3];
        let b = MeshForm::rest(1);
        let mut out = a.zeroed();
        out.add_scaled(&a, 0.5);
        out.add_scaled(&b, 0.5);
        assert_eq!(out.multiply, [0.5; 3]);
        assert_eq!(out.opacity, 1.0);
        assert!(MeshForm::rest(3).is_rest());
    }
}
