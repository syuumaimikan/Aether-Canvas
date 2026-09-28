//! Evaluating a model: what Cubism Core's `csmUpdateModel` computes.
//!
//! [`Model::update`] runs the same pipeline as Core, in the same order and
//! with the same floating-point steps, so results match it to rounding:
//!
//! 1. Parameters are clamped (or wrapped, when repeating) and each key
//!    table finds the key segment its parameter falls in. A value outside a
//!    table's keys puts every object bound to it out of range: it is
//!    disabled and keeps its previous state.
//! 2. Each object's keyform grid is blended multilinearly: parts (draw
//!    order), warp and rotation deformers, art meshes (vertices, opacity,
//!    draw order, multiply and screen colour), glue intensity and, in 5.3,
//!    offscreen surfaces.
//! 3. Blend shapes add their deltas.
//! 4. Deformers are resolved parent first: a warp's lattice is carried
//!    through its parent; a rotation's pivot is, and its angle turns with
//!    the parent's local direction. Art meshes are then mapped through their
//!    own deformer. Opacity and colours accumulate down the hierarchy.
//! 5. Part opacity multiplies in, glue pulls vertex pairs together, y is
//!    flipped for Core's y-up output, and draw-order groups are sorted into
//!    a render order.
//!
//! Output positions are in model units (canvas pixels divided by
//! [`Canvas::pixels_per_unit`](crate::moc3::Canvas), origin at the canvas
//! origin, y up), exactly as Core reports them;
//! [`Model::drawable_positions_px`] converts to canvas pixels, y down.

use crate::moc3::*;
use std::sync::Arc;

const PI: f32 = std::f32::consts::PI;

#[derive(Clone, Debug)]
struct ParamState {
    value: f32,
    snap_eps: f32,
    interp_eps: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct KeyTableState {
    key_count: usize,
    index: usize,
    weight: f32,
    out_of_range: bool,
}

#[derive(Clone, Debug, Default)]
struct BindingState {
    tables: Vec<usize>,
    blend_count: usize,
    keyform: Vec<usize>,
    weights: Vec<f32>,
    out_of_range: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct BlendKeyTableState {
    index: i32,
    weight: f32,
    index_dirty: bool,
    weight_dirty: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct BlendBindingState {
    blend_count: usize,
    keyform: [i32; 2],
    weights: [f32; 2],
    weight: f32,
}

/// A model being posed.
#[derive(Clone, Debug)]
pub struct Model {
    moc: Arc<Moc>,
    /// Parameter values to evaluate at. [`Model::update`] clamps them to
    /// their range (or wraps repeating ones), as Core does.
    pub parameter_values: Vec<f32>,
    /// Part opacities (Core's writable part opacity; pose groups drive
    /// these). Start at 1 for visible parts and 0 for hidden ones.
    pub part_opacities: Vec<f32>,

    params: Vec<ParamState>,
    key_tables: Vec<KeyTableState>,
    bindings: Vec<BindingState>,
    blend_key_tables: Vec<BlendKeyTableState>,
    blend_bindings: Vec<BlendBindingState>,
    constraint_weights: Vec<f32>,
    force_update: bool,
    param_dirty: Vec<bool>,

    part_enable: Vec<bool>,
    part_draw_order: Vec<i32>,
    part_opacity: Vec<f32>,

    deformer_enable: Vec<bool>,
    deformer_opacity: Vec<f32>,
    deformer_scale: Vec<f32>,
    deformer_multiply: Vec<[f32; 4]>,
    deformer_screen: Vec<[f32; 4]>,

    warp_opacity: Vec<f32>,
    warp_positions: Vec<Vec<f32>>,
    warp_multiply: Vec<[f32; 4]>,
    warp_screen: Vec<[f32; 4]>,

    rotation_opacity: Vec<f32>,
    rotation_angle: Vec<f32>,
    rotation_origin_x: Vec<f32>,
    rotation_origin_y: Vec<f32>,
    rotation_scale: Vec<f32>,
    rotation_reflect_x: Vec<i32>,
    rotation_reflect_y: Vec<i32>,
    rotation_multiply: Vec<[f32; 4]>,
    rotation_screen: Vec<[f32; 4]>,

    mesh_enable: Vec<bool>,
    mesh_opacity: Vec<f32>,
    mesh_draw_order: Vec<i32>,
    mesh_positions: Vec<Vec<f32>>,
    mesh_multiply: Vec<[f32; 4]>,
    mesh_screen: Vec<[f32; 4]>,

    glue_intensity: Vec<f32>,

    offscreen_enable: Vec<bool>,
    offscreen_opacity: Vec<f32>,
    offscreen_multiply: Vec<[f32; 4]>,
    offscreen_screen: Vec<[f32; 4]>,

    render_order: Vec<i32>,

    masks: Vec<Vec<i32>>,
    uvs: Vec<Vec<f32>>,
    indices: Vec<Vec<u16>>,
}

/// The index range `off..off + len`, empty whenever `len` is (validation
/// allows any offset on an empty range).
fn span(off: i32, len: i32) -> std::ops::Range<usize> {
    if len <= 0 {
        0..0
    } else {
        off as usize..off as usize + len as usize
    }
}

fn clamp01(v: f32) -> f32 {
    v.max(0.0).min(1.0)
}

/// Core's float-to-int conversion (truncation, clamped, NaN to 0).
fn to_i32(v: f32) -> i32 {
    if v.is_nan() {
        0
    } else {
        v.clamp(-2147483648.0, 2147483520.0) as i32
    }
}

/// IEEE remainder, as C's `remainderf`.
fn remainder(x: f32, y: f32) -> f32 {
    let (x, y) = (x as f64, y as f64);
    let n = (x / y).round_ties_even();
    (x - n * y) as f32
}

fn signed_angle(v1: [f32; 2], v2: [f32; 2]) -> f32 {
    let a1 = v1[1].atan2(v1[0]);
    let a2 = v2[1].atan2(v2[0]);
    remainder(a1 - a2, 2.0 * PI)
}

impl Model {
    /// A model at its default parameter values (call [`Model::update`]
    /// before reading results).
    pub fn new(moc: Arc<Moc>) -> Self {
        let m = &*moc;
        let v = m.version;
        let params = (0..m.parameters.ids.len())
            .map(|i| {
                let snap_eps = 0.1f32.powf(m.parameters.decimal_places[i] as f32);
                ParamState {
                    value: m.parameters.default[i],
                    snap_eps,
                    interp_eps: snap_eps * 1.5,
                }
            })
            .collect();
        let key_tables = m
            .key_tables
            .keys_len
            .iter()
            .map(|&n| KeyTableState {
                key_count: n as usize,
                out_of_range: true,
                ..Default::default()
            })
            .collect();
        let bindings = (0..m.bindings.key_table_off.len())
            .map(|b| {
                let tables: Vec<usize> = m.key_table_indices
                    [span(m.bindings.key_table_off[b], m.bindings.key_table_len[b])]
                .iter()
                .map(|&t| t as usize)
                .collect();
                let max = 1usize << tables.len();
                BindingState {
                    tables,
                    blend_count: 0,
                    keyform: vec![0; max],
                    weights: vec![0.0; max],
                    out_of_range: true,
                }
            })
            .collect();
        let blend_bindings = vec![
            BlendBindingState {
                weight: 1.0,
                ..Default::default()
            };
            m.blend_bindings.key_table.len()
        ];
        let blend_key_tables = vec![
            BlendKeyTableState {
                index_dirty: true,
                weight_dirty: true,
                ..Default::default()
            };
            m.blend_key_tables.keys_off.len()
        ];

        let parts = m.parts.ids.len();
        let deformers = m.deformers.ids.len();
        let warps = m.warps.binding.len();
        let rotations = m.rotations.binding.len();
        let meshes = m.art_meshes.ids.len();
        let offscreens = if v >= VERSION_53 {
            m.offscreens.owner.len()
        } else {
            0
        };

        // Masks: Core drops negative entries at load, with its own quirks.
        let masks = (0..meshes)
            .map(|i| clean_masks(&m.masks[span(m.art_meshes.mask_off[i], m.art_meshes.mask_len[i])]))
            .collect();
        let y_reversed = m.canvas.flags & CANVAS_Y_REVERSED != 0;
        let uvs = (0..meshes)
            .map(|i| {
                let mut uv = m.uvs[span(m.art_meshes.uv_off[i], m.art_meshes.vertex_count[i] * 2)].to_vec();
                if !y_reversed {
                    for v in uv.iter_mut().skip(1).step_by(2) {
                        *v = 1.0 - *v;
                    }
                }
                uv
            })
            .collect();
        let indices = (0..meshes)
            .map(|i| {
                let mut idx = m.indices[span(m.art_meshes.index_off[i], m.art_meshes.index_len[i])].to_vec();
                if !y_reversed {
                    for t in idx.chunks_exact_mut(3) {
                        t.swap(0, 2);
                    }
                }
                idx
            })
            .collect();

        let white = [1.0, 1.0, 1.0, 1.0];
        let black = [0.0, 0.0, 0.0, 1.0];
        Model {
            parameter_values: m.parameters.default.clone(),
            part_opacities: m
                .parts
                .visible
                .iter()
                .map(|&v| if v != 0 { 1.0 } else { 0.0 })
                .collect(),
            params,
            key_tables,
            bindings,
            blend_key_tables,
            blend_bindings,
            constraint_weights: vec![1.0; m.blend_constraints.parameter.len()],
            force_update: true,
            param_dirty: vec![true; m.parameters.ids.len()],
            part_enable: vec![false; parts],
            part_draw_order: vec![0; parts],
            part_opacity: vec![0.0; parts],
            deformer_enable: vec![false; deformers],
            deformer_opacity: vec![0.0; deformers],
            deformer_scale: vec![0.0; deformers],
            deformer_multiply: vec![white; deformers],
            deformer_screen: vec![black; deformers],
            warp_opacity: vec![0.0; warps],
            warp_positions: (0..warps)
                .map(|i| vec![0.0; m.warps.vertex_count[i] as usize * 2])
                .collect(),
            warp_multiply: vec![white; warps],
            warp_screen: vec![black; warps],
            rotation_opacity: vec![0.0; rotations],
            rotation_angle: vec![0.0; rotations],
            rotation_origin_x: vec![0.0; rotations],
            rotation_origin_y: vec![0.0; rotations],
            rotation_scale: vec![0.0; rotations],
            rotation_reflect_x: vec![0; rotations],
            rotation_reflect_y: vec![0; rotations],
            rotation_multiply: vec![white; rotations],
            rotation_screen: vec![black; rotations],
            mesh_enable: vec![false; meshes],
            mesh_opacity: vec![0.0; meshes],
            mesh_draw_order: vec![0; meshes],
            mesh_positions: (0..meshes)
                .map(|i| vec![0.0; m.art_meshes.vertex_count[i] as usize * 2])
                .collect(),
            mesh_multiply: vec![white; meshes],
            mesh_screen: vec![black; meshes],
            glue_intensity: vec![0.0; m.glues.ids.len()],
            offscreen_enable: vec![false; offscreens],
            offscreen_opacity: vec![0.0; offscreens],
            offscreen_multiply: vec![white; offscreens],
            offscreen_screen: vec![white; offscreens],
            render_order: vec![0; meshes + offscreens],
            masks,
            uvs,
            indices,
            moc,
        }
    }

    /// The model data.
    pub fn moc(&self) -> &Arc<Moc> {
        &self.moc
    }

    /// A parameter's index by id.
    pub fn parameter_index(&self, id: &str) -> Option<usize> {
        self.moc.parameters.ids.iter().position(|p| p == id)
    }

    /// A part's index by id.
    pub fn part_index(&self, id: &str) -> Option<usize> {
        self.moc.parts.ids.iter().position(|p| p == id)
    }

    /// A drawable's index by id.
    pub fn drawable_index(&self, id: &str) -> Option<usize> {
        self.moc.art_meshes.ids.iter().position(|p| p == id)
    }

    // ------------------------------------------------------------ results

    /// Number of drawables (art meshes).
    pub fn drawable_count(&self) -> usize {
        self.mesh_positions.len()
    }

    /// A drawable's vertices in model units, x and y interleaved, y up.
    pub fn drawable_positions(&self, i: usize) -> &[f32] {
        &self.mesh_positions[i]
    }

    /// A drawable's vertices in canvas pixels, y down.
    pub fn drawable_positions_px(&self, i: usize) -> Vec<[f32; 2]> {
        let c = &self.moc.canvas;
        self.mesh_positions[i]
            .chunks_exact(2)
            .map(|p| {
                [
                    c.origin_x + p[0] * c.pixels_per_unit,
                    c.origin_y - p[1] * c.pixels_per_unit,
                ]
            })
            .collect()
    }

    /// Texture coordinates (v up, as Core reports them).
    pub fn drawable_uvs(&self, i: usize) -> &[f32] {
        &self.uvs[i]
    }

    /// Triangle indices (winding as Core reports it).
    pub fn drawable_indices(&self, i: usize) -> &[u16] {
        &self.indices[i]
    }

    /// Masks of a drawable (as Core reports them; skip negative entries).
    pub fn drawable_masks(&self, i: usize) -> &[i32] {
        &self.masks[i]
    }

    /// Whether a drawable is enabled (its part, deformer and keys allow it).
    pub fn drawable_enabled(&self, i: usize) -> bool {
        self.mesh_enable[i]
    }

    /// Whether Core would report a drawable visible: enabled and not fully
    /// transparent.
    pub fn drawable_visible(&self, i: usize) -> bool {
        self.mesh_enable[i] && self.mesh_opacity[i] != 0.0
    }

    /// Final opacity.
    pub fn drawable_opacity(&self, i: usize) -> f32 {
        self.mesh_opacity[i]
    }

    /// Keyed draw order (0..1000).
    pub fn drawable_draw_order(&self, i: usize) -> i32 {
        self.mesh_draw_order[i]
    }

    /// Position in the final drawing order (0 is drawn first).
    pub fn drawable_render_order(&self, i: usize) -> i32 {
        self.render_order[i]
    }

    /// Render orders of drawables, then (5.3) offscreen surfaces.
    pub fn render_orders(&self) -> &[i32] {
        &self.render_order
    }

    /// Multiply colour (RGBA, A = 1).
    pub fn drawable_multiply(&self, i: usize) -> [f32; 4] {
        self.mesh_multiply[i]
    }

    /// Screen colour (RGBA, A = 1).
    pub fn drawable_screen(&self, i: usize) -> [f32; 4] {
        self.mesh_screen[i]
    }

    /// Final part opacity (after its parents).
    pub fn part_opacity(&self, i: usize) -> f32 {
        self.part_opacity[i]
    }

    /// Offscreen surface opacities (5.3).
    pub fn offscreen_opacities(&self) -> &[f32] {
        &self.offscreen_opacity
    }

    // ------------------------------------------------------------- update

    /// Evaluate the model at [`Model::parameter_values`] and
    /// [`Model::part_opacities`].
    pub fn update(&mut self) {
        let moc = self.moc.clone();
        let m = &*moc;
        self.resolve_params(m);
        self.resolve_key_tables(m);
        self.resolve_blend_key_tables(m);
        self.resolve_bindings();
        self.resolve_blend_bindings(m);

        for o in &mut self.part_opacities {
            *o = clamp01(*o);
        }
        self.parts(m);
        self.deformers(m);
        self.art_meshes(m);
        self.glues(m);
        self.offscreens(m);

        self.blend_shapes(m);

        for d in 0..m.deformers.ids.len() {
            if self.deformer_enable[d] {
                if m.deformers.kind[d] == DEFORMER_WARP {
                    self.apply_warp(m, d);
                } else {
                    self.apply_rotation(m, d);
                }
            }
        }
        self.transform_meshes(m);
        self.apply_part_opacity(m);
        self.apply_parts_to_meshes(m);
        self.apply_glues(m);
        if m.canvas.flags & CANVAS_Y_REVERSED == 0 {
            for (i, pos) in self.mesh_positions.iter_mut().enumerate() {
                if self.mesh_enable[i] {
                    for y in pos.iter_mut().skip(1).step_by(2) {
                        *y = -*y;
                    }
                }
            }
        }
        self.sort_render_order(m);
        if m.version >= VERSION_53 {
            for (o, &e) in self.offscreen_opacity.iter_mut().zip(&self.offscreen_enable) {
                if !e {
                    *o = 0.0;
                }
            }
        }
        self.force_update = false;
    }

    fn resolve_params(&mut self, m: &Moc) {
        for i in 0..self.params.len() {
            let input = self.parameter_values[i];
            let (min, max) = (m.parameters.min[i], m.parameters.max[i]);
            let value = if m.parameters.repeat[i] != 0 {
                let length = max - min;
                let normalized = (input - min) / length;
                let wrapped = normalized - normalized.floor();
                wrapped * length + min
            } else {
                let v = input.max(min).min(max);
                self.parameter_values[i] = v;
                v
            };
            self.param_dirty[i] = self.params[i].value != value;
            self.params[i].value = value;
        }
    }

    fn resolve_key_tables(&mut self, m: &Moc) {
        for i in 0..self.params.len() {
            let kind = if m.version >= VERSION_42 {
                m.parameters.kind[i]
            } else {
                PARAMETER_NORMAL
            };
            if kind != PARAMETER_NORMAL {
                continue;
            }
            let p = &self.params[i];
            for t in span(m.parameters.key_table_off[i], m.parameters.key_table_len[i]) {
                let kl = m.key_tables.keys_len[t].max(0) as usize;
                if kl == 0 {
                    continue;
                }
                let keys = &m.keys[span(m.key_tables.keys_off[t], m.key_tables.keys_len[t])];
                let (index, weight, outside) = find_key_segment(p.value, keys, p.snap_eps, p.interp_eps);
                self.key_tables[t] = KeyTableState {
                    key_count: kl,
                    index,
                    weight,
                    out_of_range: outside,
                };
            }
        }
    }

    fn resolve_blend_key_tables(&mut self, m: &Moc) {
        if m.version < VERSION_42 {
            return;
        }
        for i in 0..self.params.len() {
            if m.parameters.kind[i] != PARAMETER_BLEND_SHAPE {
                continue;
            }
            let value = self.params[i].value;
            let dirty = self.force_update || self.param_dirty[i];
            for t in span(
                m.parameters.blend_key_table_off[i],
                m.parameters.blend_key_table_len[i],
            ) {
                let state = &mut self.blend_key_tables[t];
                if !dirty {
                    state.index_dirty = false;
                    state.weight_dirty = false;
                    continue;
                }
                let kc = m.blend_key_tables.keys_len[t].max(0) as usize;
                let mut index = 0usize;
                let mut weight = 0.0f32;
                if kc >= 2 {
                    let keys = &m.keys[span(m.blend_key_tables.keys_off[t], m.blend_key_tables.keys_len[t])];
                    if value > keys[0] {
                        index = 1;
                        while index < kc && value >= keys[index] {
                            index += 1;
                        }
                        index -= 1;
                        if index < kc - 1 {
                            weight = (value - keys[index]) / (keys[index + 1] - keys[index]);
                        }
                    }
                }
                let index = index as i32;
                let mut index_dirty = state.index != index;
                let weight_dirty = state.weight != weight;
                if weight_dirty {
                    index_dirty = weight == 0.0 || state.weight == 0.0 || state.index != index;
                }
                *state = BlendKeyTableState {
                    index,
                    weight,
                    index_dirty,
                    weight_dirty,
                };
            }
        }
    }

    fn resolve_bindings(&mut self) {
        let tables = &self.key_tables;
        for b in &mut self.bindings {
            if b.tables.iter().any(|&t| tables[t].out_of_range) {
                b.out_of_range = true;
                continue;
            }
            let active = b.tables.iter().filter(|&&t| tables[t].weight != 0.0).count();
            let count = 1usize << active;
            b.blend_count = count;
            for j in 0..count {
                b.keyform[j] = 0;
                b.weights[j] = 1.0;
            }
            let (mut index_stride, mut combo_stride) = (1usize, 1usize);
            for &t in &b.tables {
                let state = tables[t];
                let key_count = state.key_count;
                let index_offset = state.index * index_stride;
                if state.weight != 0.0 {
                    let next_offset = (state.index + 1) * index_stride;
                    let inv = 1.0 - state.weight;
                    for j in 0..count {
                        if j & combo_stride == 0 {
                            b.keyform[j] += index_offset;
                            b.weights[j] *= inv;
                        } else {
                            b.keyform[j] += next_offset;
                            b.weights[j] *= state.weight;
                        }
                    }
                    combo_stride *= 2;
                } else {
                    for j in 0..count {
                        b.keyform[j] += index_offset;
                    }
                }
                index_stride *= key_count;
            }
            b.out_of_range = false;
        }
    }

    fn resolve_blend_bindings(&mut self, m: &Moc) {
        if m.version < VERSION_42 {
            return;
        }
        let force = self.force_update;
        for b in 0..self.blend_bindings.len() {
            let table = m.blend_bindings.key_table[b] as usize;
            let t = self.blend_key_tables[table];
            let base = m.blend_key_tables.base_key[table];
            let state = &mut self.blend_bindings[b];
            let weight_dirty = force || t.weight_dirty;
            let index_dirty = force || t.index_dirty;
            if weight_dirty || index_dirty {
                let weight = t.weight;
                let mut next = t.index;
                if weight != 0.0 && next == base {
                    state.blend_count = 1;
                    state.weights = [weight, 1.0 - weight];
                    next += 1;
                    state.keyform = [next, next + 1];
                } else {
                    state.blend_count = if weight == 0.0 {
                        usize::from(next != base)
                    } else if next + 1 == base {
                        1
                    } else {
                        2
                    };
                    if weight_dirty {
                        state.weights = [1.0 - weight, weight];
                    }
                    if index_dirty {
                        state.keyform = [next, next + 1];
                    }
                }
            }

            let mut weight = 1.0f32;
            for k in span(
                m.blend_bindings.constraint_off[b],
                m.blend_bindings.constraint_len[b],
            ) {
                let c = m.blend_constraint_indices[k] as usize;
                let param = m.blend_constraints.parameter[c] as usize;
                let cw = if force || self.param_dirty[param] {
                    let values = span(m.blend_constraints.value_off[c], m.blend_constraints.value_len[c]);
                    let vl = values.len();
                    let keys = &m.blend_constraint_values.key[values.clone()];
                    let weights = &m.blend_constraint_values.weight[values];
                    let value = self.params[param].value;
                    let cw = if vl >= 2 {
                        if value > keys[0] {
                            let mut idx = 1;
                            while idx < vl && value >= keys[idx] {
                                idx += 1;
                            }
                            idx -= 1;
                            if idx < vl - 1 {
                                let t = (value - keys[idx]) / (keys[idx + 1] - keys[idx]);
                                weights[idx] * (1.0 - t) + weights[idx + 1] * t
                            } else {
                                weights[vl - 1]
                            }
                        } else {
                            weights[0]
                        }
                    } else if vl == 1 {
                        weights[0]
                    } else {
                        1.0
                    };
                    self.constraint_weights[c] = cw;
                    cw
                } else {
                    self.constraint_weights[c]
                };
                weight = weight.min(cw);
            }
            self.blend_bindings[b].weight = weight;
        }
    }

    // ------------------------------------------------------ interpolation

    /// Blend a scalar keyform array for an object.
    fn blend_scalar(&self, binding: usize, first: usize, values: &[f32]) -> f32 {
        let b = &self.bindings[binding];
        let mut sum = 0.0f32;
        for j in 0..b.blend_count {
            sum += values[b.keyform[j] + first] * b.weights[j];
        }
        sum
    }

    fn blend_positions(&self, binding: usize, first: usize, offsets: &[i32], m: &Moc, out: &mut [f32]) {
        let b = &self.bindings[binding];
        out.iter_mut().for_each(|v| *v = 0.0);
        for j in 0..b.blend_count {
            let w = b.weights[j];
            let at = offsets[b.keyform[j] + first] as usize;
            let src = &m.keyform_positions[at..at + out.len()];
            for (d, s) in out.iter_mut().zip(src) {
                *d += s * w;
            }
        }
    }

    fn blend_colors(&self, binding: usize, color_off: i32, m: &Moc) -> ([f32; 4], [f32; 4]) {
        let b = &self.bindings[binding];
        let mut mul = [0.0f32, 0.0, 0.0, 1.0];
        let mut scr = [0.0f32, 0.0, 0.0, 1.0];
        for c in 0..3 {
            let (ms, ss) = match c {
                0 => (&m.multiply_colors.r, &m.screen_colors.r),
                1 => (&m.multiply_colors.g, &m.screen_colors.g),
                _ => (&m.multiply_colors.b, &m.screen_colors.b),
            };
            let mut msum = 0.0f32;
            let mut ssum = 0.0f32;
            for j in 0..b.blend_count {
                let k = (b.keyform[j] as i64 + color_off as i64) as usize;
                msum += ms.get(k).copied().unwrap_or(1.0) * b.weights[j];
                ssum += ss.get(k).copied().unwrap_or(0.0) * b.weights[j];
            }
            mul[c] = msum;
            scr[c] = ssum;
        }
        (mul, scr)
    }

    fn parts(&mut self, m: &Moc) {
        for i in 0..m.parts.ids.len() {
            let parent = m.parts.parent_part[i];
            let mut e = m.parts.enable[i] != 0;
            if e && parent != -1 {
                e = self.part_enable[parent as usize];
            }
            let binding = m.parts.binding[i] as usize;
            if e {
                e = !self.bindings[binding].out_of_range;
            }
            self.part_enable[i] = e;
            if e {
                let sum = self.blend_scalar(
                    binding,
                    m.parts.keyform_off[i] as usize,
                    &m.part_keyforms.draw_order,
                );
                self.part_draw_order[i] = to_i32(sum + 0.001);
            }
        }
    }

    fn deformers(&mut self, m: &Moc) {
        for i in 0..m.deformers.ids.len() {
            let mut e = m.deformers.enable[i] != 0;
            let pp = m.deformers.parent_part[i];
            let pd = m.deformers.parent_deformer[i];
            if e && pp != -1 {
                e = self.part_enable[pp as usize];
            }
            if e && pd != -1 {
                e = self.deformer_enable[pd as usize];
            }
            if e {
                e = !self.bindings[m.deformers.binding[i] as usize].out_of_range;
            }
            self.deformer_enable[i] = e;
        }
        let colors = m.version >= VERSION_42;
        for d in 0..m.deformers.ids.len() {
            if !self.deformer_enable[d] {
                continue;
            }
            let local = m.deformers.local_index[d] as usize;
            if m.deformers.kind[d] == DEFORMER_WARP {
                let binding = m.warps.binding[local] as usize;
                let first = m.warps.keyform_off[local] as usize;
                self.warp_opacity[local] = self.blend_scalar(binding, first, &m.warp_keyforms.opacity);
                let mut pos = std::mem::take(&mut self.warp_positions[local]);
                self.blend_positions(binding, first, &m.warp_keyforms.position_off, m, &mut pos);
                self.warp_positions[local] = pos;
                if colors {
                    let (mul, scr) = self.blend_colors(binding, m.warps.color_off[local], m);
                    self.warp_multiply[local] = mul;
                    self.warp_screen[local] = scr;
                }
            } else {
                let binding = m.rotations.binding[local] as usize;
                let first = m.rotations.keyform_off[local] as usize;
                let k = &m.rotation_keyforms;
                self.rotation_opacity[local] = self.blend_scalar(binding, first, &k.opacity);
                self.rotation_angle[local] = self.blend_scalar(binding, first, &k.angle);
                self.rotation_origin_x[local] = self.blend_scalar(binding, first, &k.origin_x);
                self.rotation_origin_y[local] = self.blend_scalar(binding, first, &k.origin_y);
                self.rotation_scale[local] = self.blend_scalar(binding, first, &k.scale);
                let b = &self.bindings[binding];
                if b.blend_count > 0 {
                    let kf = b.keyform[0] + first;
                    self.rotation_reflect_x[local] = k.reflect_x[kf];
                    self.rotation_reflect_y[local] = k.reflect_y[kf];
                }
                if colors {
                    let (mul, scr) = self.blend_colors(binding, m.rotations.color_off[local], m);
                    self.rotation_multiply[local] = mul;
                    self.rotation_screen[local] = scr;
                }
            }
        }
    }

    fn art_meshes(&mut self, m: &Moc) {
        let a = &m.art_meshes;
        for i in 0..a.ids.len() {
            let mut e = a.enable[i] != 0;
            let pp = a.parent_part[i];
            let pd = a.parent_deformer[i];
            if e && pp != -1 {
                e = self.part_enable[pp as usize];
            }
            if e && pd != -1 {
                e = self.deformer_enable[pd as usize];
            }
            let binding = a.binding[i] as usize;
            if e {
                e = !self.bindings[binding].out_of_range;
            }
            self.mesh_enable[i] = e;
            if !e {
                continue;
            }
            let first = a.keyform_off[i] as usize;
            let k = &m.art_mesh_keyforms;
            self.mesh_opacity[i] = self.blend_scalar(binding, first, &k.opacity);
            self.mesh_draw_order[i] = to_i32(self.blend_scalar(binding, first, &k.draw_order) + 0.001);
            let mut pos = std::mem::take(&mut self.mesh_positions[i]);
            self.blend_positions(binding, first, &k.position_off, m, &mut pos);
            self.mesh_positions[i] = pos;
            if m.version >= VERSION_42 {
                let (mul, scr) = self.blend_colors(binding, a.color_off[i], m);
                self.mesh_multiply[i] = mul;
                self.mesh_screen[i] = scr;
            }
        }
    }

    fn glues(&mut self, m: &Moc) {
        for i in 0..m.glues.ids.len() {
            let binding = m.glues.binding[i] as usize;
            self.glue_intensity[i] =
                self.blend_scalar(binding, m.glues.keyform_off[i] as usize, &m.glue_intensity);
        }
    }

    fn offscreens(&mut self, m: &Moc) {
        if m.version < VERSION_53 {
            return;
        }
        for i in 0..m.offscreens.owner.len() {
            let owner = m.offscreens.owner[i] as usize;
            let e = self.part_enable[owner];
            self.offscreen_enable[i] = e;
            if !e {
                continue;
            }
            let binding = m.parts.binding[owner] as usize;
            let first_part_keyform = m.parts.keyform_off[owner] as usize;
            let base = m.part_keyforms.offscreen_keyform[first_part_keyform];
            if base < 0 {
                continue;
            }
            let b = &self.bindings[binding];
            let k = &m.offscreen_keyforms;
            let mut sum = 0.0f32;
            for j in 0..b.blend_count {
                sum += k
                    .opacity
                    .get(b.keyform[j] + base as usize)
                    .copied()
                    .unwrap_or(0.0)
                    * b.weights[j];
            }
            self.offscreen_opacity[i] = sum;
            let color_base = k.multiply_off.get(base as usize).copied().unwrap_or(-1);
            if color_base >= 0 {
                let mut mul = [0.0f32, 0.0, 0.0, 1.0];
                let mut scr = [0.0f32, 0.0, 0.0, 1.0];
                for j in 0..b.blend_count {
                    let idx = b.keyform[j] + color_base as usize;
                    let w = b.weights[j];
                    mul[0] += m.multiply_colors.r.get(idx).copied().unwrap_or(1.0) * w;
                    mul[1] += m.multiply_colors.g.get(idx).copied().unwrap_or(1.0) * w;
                    mul[2] += m.multiply_colors.b.get(idx).copied().unwrap_or(1.0) * w;
                    scr[0] += m.screen_colors.r.get(idx).copied().unwrap_or(0.0) * w;
                    scr[1] += m.screen_colors.g.get(idx).copied().unwrap_or(0.0) * w;
                    scr[2] += m.screen_colors.b.get(idx).copied().unwrap_or(0.0) * w;
                }
                self.offscreen_multiply[i] = mul;
                self.offscreen_screen[i] = scr;
            }
        }
    }

    // ------------------------------------------------------- blend shapes

    fn shape_value(&self, binding: usize, first_keyform: i32, src: &[f32]) -> f32 {
        let b = &self.blend_bindings[binding];
        let off = first_keyform as i64;
        let at = |k: i32| src.get((k as i64 + off) as usize).copied().unwrap_or(0.0);
        let value = match b.blend_count {
            1 => at(b.keyform[0]) * b.weights[0],
            2 => at(b.keyform[0]) * b.weights[0] + at(b.keyform[1]) * b.weights[1],
            _ => return 0.0,
        };
        b.weight * value
    }

    fn shape_bindings(shapes: &BlendShapes, i: usize) -> std::ops::Range<usize> {
        span(shapes.binding_off[i], shapes.binding_len[i])
    }

    fn blend_shapes(&mut self, m: &Moc) {
        if m.version < VERSION_42 {
            return;
        }
        let v50 = m.version >= VERSION_50;
        if v50 {
            // Part draw order.
            for i in 0..m.blend_parts.target.len() {
                let t = m.blend_parts.target[i] as usize;
                let mut value = self.part_draw_order[t] as f32;
                for b in Self::shape_bindings(&m.blend_parts, i) {
                    value +=
                        self.shape_value(b, m.blend_bindings.keyform_off[b], &m.part_keyforms.draw_order);
                }
                self.part_draw_order[t] = (value + 0.001).clamp(0.0, 1000.0) as i32;
            }
        }
        // Warps: positions, then (5.0) opacity and colours.
        for i in 0..m.blend_warps.target.len() {
            let t = m.blend_warps.target[i] as usize;
            let mut pos = std::mem::take(&mut self.warp_positions[t]);
            self.shape_positions(m, &m.blend_warps, i, &m.warp_keyforms.position_off, &mut pos);
            self.warp_positions[t] = pos;
        }
        if v50 {
            for i in 0..m.blend_warps.target.len() {
                let t = m.blend_warps.target[i] as usize;
                let mut value = self.warp_opacity[t];
                for b in Self::shape_bindings(&m.blend_warps, i) {
                    value += self.shape_value(b, m.blend_bindings.keyform_off[b], &m.warp_keyforms.opacity);
                }
                self.warp_opacity[t] = value.max(0.0).min(1.0);
            }
            for i in 0..m.blend_warps.target.len() {
                let t = m.blend_warps.target[i] as usize;
                let mut mul = self.warp_multiply[t];
                self.shape_colors(
                    m,
                    &m.blend_warps,
                    i,
                    &m.warp_keyforms.multiply_off,
                    &m.multiply_colors,
                    &mut mul,
                );
                self.warp_multiply[t] = mul;
            }
            for i in 0..m.blend_warps.target.len() {
                let t = m.blend_warps.target[i] as usize;
                let mut scr = self.warp_screen[t];
                self.shape_colors(
                    m,
                    &m.blend_warps,
                    i,
                    &m.warp_keyforms.screen_off,
                    &m.screen_colors,
                    &mut scr,
                );
                self.warp_screen[t] = scr;
            }

            // Rotations.
            let r = &m.rotation_keyforms;
            for i in 0..m.blend_rotations.target.len() {
                let t = m.blend_rotations.target[i] as usize;
                let x = self.shape_scalar(m, &m.blend_rotations, i, &r.origin_x, self.rotation_origin_x[t]);
                self.rotation_origin_x[t] = x;
            }
            for i in 0..m.blend_rotations.target.len() {
                let t = m.blend_rotations.target[i] as usize;
                let y = self.shape_scalar(m, &m.blend_rotations, i, &r.origin_y, self.rotation_origin_y[t]);
                self.rotation_origin_y[t] = y;
            }
            for i in 0..m.blend_rotations.target.len() {
                let t = m.blend_rotations.target[i] as usize;
                let o = self.shape_scalar(m, &m.blend_rotations, i, &r.opacity, self.rotation_opacity[t]);
                self.rotation_opacity[t] = o.max(0.0).min(1.0);
            }
            for i in 0..m.blend_rotations.target.len() {
                let t = m.blend_rotations.target[i] as usize;
                let mut mul = self.rotation_multiply[t];
                self.shape_colors(
                    m,
                    &m.blend_rotations,
                    i,
                    &r.multiply_off,
                    &m.multiply_colors,
                    &mut mul,
                );
                self.rotation_multiply[t] = mul;
            }
            for i in 0..m.blend_rotations.target.len() {
                let t = m.blend_rotations.target[i] as usize;
                let mut scr = self.rotation_screen[t];
                self.shape_colors(
                    m,
                    &m.blend_rotations,
                    i,
                    &r.screen_off,
                    &m.screen_colors,
                    &mut scr,
                );
                self.rotation_screen[t] = scr;
            }
            for i in 0..m.blend_rotations.target.len() {
                let t = m.blend_rotations.target[i] as usize;
                let a = self.shape_scalar(m, &m.blend_rotations, i, &r.angle, self.rotation_angle[t]);
                self.rotation_angle[t] = a.max(-3600.0).min(3600.0);
            }
            for i in 0..m.blend_rotations.target.len() {
                let t = m.blend_rotations.target[i] as usize;
                let s = self.shape_scalar(m, &m.blend_rotations, i, &r.scale, self.rotation_scale[t]);
                self.rotation_scale[t] = s.max(0.0001).min(100.0);
            }
        }

        // Art meshes: positions, then (5.0) draw order, opacity, colours.
        for i in 0..m.blend_art_meshes.target.len() {
            let t = m.blend_art_meshes.target[i] as usize;
            let mut pos = std::mem::take(&mut self.mesh_positions[t]);
            self.shape_positions(
                m,
                &m.blend_art_meshes,
                i,
                &m.art_mesh_keyforms.position_off,
                &mut pos,
            );
            self.mesh_positions[t] = pos;
        }
        if v50 {
            let k = &m.art_mesh_keyforms;
            for i in 0..m.blend_art_meshes.target.len() {
                let t = m.blend_art_meshes.target[i] as usize;
                let mut value = self.mesh_draw_order[t] as f32;
                for b in Self::shape_bindings(&m.blend_art_meshes, i) {
                    value += self.shape_value(b, m.blend_bindings.keyform_off[b], &k.draw_order);
                }
                self.mesh_draw_order[t] = (value + 0.001).clamp(0.0, 1000.0) as i32;
            }
            for i in 0..m.blend_art_meshes.target.len() {
                let t = m.blend_art_meshes.target[i] as usize;
                let o = self.shape_scalar(m, &m.blend_art_meshes, i, &k.opacity, self.mesh_opacity[t]);
                self.mesh_opacity[t] = o.max(0.0).min(1.0);
            }
            for i in 0..m.blend_art_meshes.target.len() {
                let t = m.blend_art_meshes.target[i] as usize;
                let mut mul = self.mesh_multiply[t];
                self.shape_colors(
                    m,
                    &m.blend_art_meshes,
                    i,
                    &k.multiply_off,
                    &m.multiply_colors,
                    &mut mul,
                );
                self.mesh_multiply[t] = mul;
            }
            for i in 0..m.blend_art_meshes.target.len() {
                let t = m.blend_art_meshes.target[i] as usize;
                let mut scr = self.mesh_screen[t];
                self.shape_colors(
                    m,
                    &m.blend_art_meshes,
                    i,
                    &k.screen_off,
                    &m.screen_colors,
                    &mut scr,
                );
                self.mesh_screen[t] = scr;
            }

            // Glue.
            for i in 0..m.blend_glues.target.len() {
                let t = m.blend_glues.target[i] as usize;
                let v = self.shape_scalar(m, &m.blend_glues, i, &m.glue_intensity, self.glue_intensity[t]);
                self.glue_intensity[t] = v.max(0.0).min(1.0);
            }
        }

        if m.version >= VERSION_53 {
            let k = &m.offscreen_keyforms;
            for i in 0..m.blend_offscreens.target.len() {
                let t = m.blend_offscreens.target[i] as usize;
                let o = self.shape_scalar(m, &m.blend_offscreens, i, &k.opacity, self.offscreen_opacity[t]);
                self.offscreen_opacity[t] = o.max(0.0).min(1.0);
            }
            for i in 0..m.blend_offscreens.target.len() {
                let t = m.blend_offscreens.target[i] as usize;
                let mut mul = self.offscreen_multiply[t];
                self.shape_colors(
                    m,
                    &m.blend_offscreens,
                    i,
                    &k.multiply_off,
                    &m.multiply_colors,
                    &mut mul,
                );
                self.offscreen_multiply[t] = mul;
            }
            for i in 0..m.blend_offscreens.target.len() {
                let t = m.blend_offscreens.target[i] as usize;
                let mut scr = self.offscreen_screen[t];
                self.shape_colors(
                    m,
                    &m.blend_offscreens,
                    i,
                    &k.multiply_off,
                    &m.screen_colors,
                    &mut scr,
                );
                self.offscreen_screen[t] = scr;
            }
        }
    }

    fn shape_scalar(&self, m: &Moc, shapes: &BlendShapes, i: usize, src: &[f32], start: f32) -> f32 {
        let mut value = start;
        for b in Self::shape_bindings(shapes, i) {
            value += self.shape_value(b, m.blend_bindings.keyform_off[b], src);
        }
        value
    }

    fn shape_positions(&self, m: &Moc, shapes: &BlendShapes, i: usize, pos_off: &[i32], out: &mut [f32]) {
        for b in Self::shape_bindings(shapes, i) {
            let s = &self.blend_bindings[b];
            let off = m.blend_bindings.keyform_off[b] as i64;
            let cw = s.weight;
            let at = |k: i32| pos_off.get((k as i64 + off) as usize).map(|&p| p as usize);
            match s.blend_count {
                1 => {
                    let Some(p0) = at(s.keyform[0]) else { continue };
                    let w0 = s.weights[0];
                    for (k, o) in out.iter_mut().enumerate() {
                        *o += m.keyform_positions.get(p0 + k).copied().unwrap_or(0.0) * w0 * cw;
                    }
                }
                2 => {
                    let (Some(p0), Some(p1)) = (at(s.keyform[0]), at(s.keyform[1])) else {
                        continue;
                    };
                    let (w0, w1) = (s.weights[0], s.weights[1]);
                    for (k, o) in out.iter_mut().enumerate() {
                        let a = m.keyform_positions.get(p0 + k).copied().unwrap_or(0.0);
                        let b = m.keyform_positions.get(p1 + k).copied().unwrap_or(0.0);
                        *o += (w0 * a + b * w1) * cw;
                    }
                }
                _ => {}
            }
        }
    }

    fn shape_colors(
        &self,
        m: &Moc,
        shapes: &BlendShapes,
        i: usize,
        color_off: &[i32],
        src: &Colors,
        out: &mut [f32; 4],
    ) {
        for b in Self::shape_bindings(shapes, i) {
            let s = &self.blend_bindings[b];
            let off = m.blend_bindings.keyform_off[b] as i64;
            let at = |k: i32| color_off.get((k as i64 + off) as usize).copied().unwrap_or(-1);
            let get = |c: &Vec<f32>, i: i32| c.get(i as usize).copied().unwrap_or(0.0);
            let (r, g, bl) = match s.blend_count {
                1 => {
                    let ci = at(s.keyform[0]);
                    if ci < 0 {
                        continue;
                    }
                    let w0 = s.weights[0];
                    (get(&src.r, ci) * w0, get(&src.g, ci) * w0, get(&src.b, ci) * w0)
                }
                2 => {
                    let (c0, c1) = (at(s.keyform[0]), at(s.keyform[1]));
                    if c0 < 0 || c1 < 0 {
                        continue;
                    }
                    let (w0, w1) = (s.weights[0], s.weights[1]);
                    (
                        w0 * get(&src.r, c0) + get(&src.r, c1) * w1,
                        w0 * get(&src.g, c0) + get(&src.g, c1) * w1,
                        w0 * get(&src.b, c0) + get(&src.b, c1) * w1,
                    )
                }
                _ => continue,
            };
            out[0] += r * s.weight;
            out[1] += g * s.weight;
            out[2] += bl * s.weight;
        }
        out[0] = clamp01(out[0]);
        out[1] = clamp01(out[1]);
        out[2] = clamp01(out[2]);
    }

    // --------------------------------------------------------- transforms

    fn warp_transform(&self, m: &Moc, d: usize, points: &mut [f32]) {
        let local = m.deformers.local_index[d] as usize;
        let pos = &self.warp_positions[local];
        let rows = m.warps.rows[local];
        let cols = m.warps.cols[local];
        let quad = m.version >= VERSION_33 && m.warps.quad[local] != 0;
        warp_transform(pos, rows, cols, quad, points);
    }

    fn rotation_transform(&self, m: &Moc, d: usize, points: &mut [f32]) {
        let r = m.deformers.local_index[d] as usize;
        let angle = (m.rotations.base_angle[r] + self.rotation_angle[r]) * PI / 180.0;
        let (sin, cos) = (angle.sin(), angle.cos());
        let scale = self.rotation_scale[r];
        let rx = if self.rotation_reflect_x[r] != 0 {
            -1.0
        } else {
            1.0
        };
        let ry = if self.rotation_reflect_y[r] != 0 {
            -1.0
        } else {
            1.0
        };
        let m00 = scale * cos * rx;
        let m01 = scale * (-sin) * ry;
        let m10 = scale * sin * rx;
        let m11 = scale * cos * ry;
        let (ox, oy) = (self.rotation_origin_x[r], self.rotation_origin_y[r]);
        for p in points.chunks_exact_mut(2) {
            let (x, y) = (p[0], p[1]);
            p[0] = ox + m00 * x + m01 * y;
            p[1] = oy + m10 * x + m11 * y;
        }
    }

    fn transform_point(&self, m: &Moc, d: usize, p: [f32; 2]) -> [f32; 2] {
        let mut pt = [p[0], p[1]];
        if m.deformers.kind[d] == DEFORMER_WARP {
            self.warp_transform(m, d, &mut pt);
        } else {
            self.rotation_transform(m, d, &mut pt);
        }
        pt
    }

    fn apply_warp(&mut self, m: &Moc, d: usize) {
        let parent = m.deformers.parent_deformer[d];
        let local = m.deformers.local_index[d] as usize;
        if parent == -1 {
            self.deformer_opacity[d] = self.warp_opacity[local];
            self.deformer_scale[d] = 1.0;
        } else {
            let p = parent as usize;
            let mut pos = std::mem::take(&mut self.warp_positions[local]);
            if m.deformers.kind[p] == DEFORMER_WARP {
                self.warp_transform(m, p, &mut pos);
            } else {
                self.rotation_transform(m, p, &mut pos);
            }
            self.warp_positions[local] = pos;
            self.deformer_opacity[d] = self.warp_opacity[local] * self.deformer_opacity[p];
            self.deformer_scale[d] = self.deformer_scale[p];
        }
        if m.version >= VERSION_42 {
            let (mul, scr) = (self.warp_multiply[local], self.warp_screen[local]);
            self.propagate_colors(d, parent, mul, scr);
        }
    }

    fn apply_rotation(&mut self, m: &Moc, d: usize) {
        let parent = m.deformers.parent_deformer[d];
        let r = m.deformers.local_index[d] as usize;
        if parent == -1 {
            self.deformer_opacity[d] = self.rotation_opacity[r];
            self.deformer_scale[d] = self.rotation_scale[r];
        } else {
            let p = parent as usize;
            let origin = [self.rotation_origin_x[r], self.rotation_origin_y[r]];
            let delta = if m.deformers.kind[p] == DEFORMER_ROTATION {
                -10.0f32
            } else {
                -0.1f32
            };
            let t_origin = self.transform_point(m, p, origin);
            let mut direction = [0.0f32, 0.0];
            let mut scale = 1.0f32;
            for _ in 0..16 {
                let tt = self.transform_point(m, p, [origin[0], origin[1] + scale * delta]);
                let dv = [tt[0] - t_origin[0], tt[1] - t_origin[1]];
                if dv[0] != 0.0 || dv[1] != 0.0 {
                    direction = dv;
                    break;
                }
                let tt = self.transform_point(m, p, [origin[0], origin[1] - scale * delta]);
                let dv = [tt[0] - t_origin[0], tt[1] - t_origin[1]];
                if dv[0] != 0.0 || dv[1] != 0.0 {
                    direction = [-dv[0], -dv[1]];
                    break;
                }
                scale *= 0.1;
            }
            let adjust = (signed_angle([0.0, delta], direction) * -180.0) / PI;
            let origin = self.transform_point(m, p, origin);
            self.rotation_origin_x[r] = origin[0];
            self.rotation_origin_y[r] = origin[1];
            self.rotation_angle[r] += adjust;
            self.deformer_opacity[d] = self.rotation_opacity[r] * self.deformer_opacity[p];
            let cs = self.rotation_scale[r] * self.deformer_scale[p];
            self.deformer_scale[d] = cs;
            self.rotation_scale[r] = cs;
        }
        if m.version >= VERSION_42 {
            let (mul, scr) = (self.rotation_multiply[r], self.rotation_screen[r]);
            self.propagate_colors(d, parent, mul, scr);
        }
    }

    fn propagate_colors(&mut self, d: usize, parent: i32, mul: [f32; 4], scr: [f32; 4]) {
        if parent == -1 {
            self.deformer_multiply[d] = [mul[0], mul[1], mul[2], 1.0];
            self.deformer_screen[d] = [scr[0], scr[1], scr[2], 1.0];
        } else {
            let pm = self.deformer_multiply[parent as usize];
            let ps = self.deformer_screen[parent as usize];
            self.deformer_multiply[d] = [mul[0] * pm[0], mul[1] * pm[1], mul[2] * pm[2], 1.0];
            self.deformer_screen[d] = [
                scr[0] + ps[0] - scr[0] * ps[0],
                scr[1] + ps[1] - scr[1] * ps[1],
                scr[2] + ps[2] - scr[2] * ps[2],
                1.0,
            ];
        }
    }

    fn transform_meshes(&mut self, m: &Moc) {
        for i in 0..m.art_meshes.ids.len() {
            if !self.mesh_enable[i] {
                continue;
            }
            let pd = m.art_meshes.parent_deformer[i];
            if pd == -1 {
                continue;
            }
            let d = pd as usize;
            self.mesh_opacity[i] *= self.deformer_opacity[d];
            let mut pos = std::mem::take(&mut self.mesh_positions[i]);
            if m.deformers.kind[d] == DEFORMER_WARP {
                self.warp_transform(m, d, &mut pos);
            } else {
                self.rotation_transform(m, d, &mut pos);
            }
            self.mesh_positions[i] = pos;
        }
    }

    fn apply_part_opacity(&mut self, m: &Moc) {
        let offscreen = |i: usize| -> i32 {
            if m.version >= VERSION_53 {
                m.parts.offscreen[i]
            } else {
                -1
            }
        };
        for i in 0..m.parts.ids.len() {
            if !self.part_enable[i] {
                continue;
            }
            let mut opacity = self.part_opacities[i];
            self.part_opacity[i] = opacity;
            let parent = m.parts.parent_part[i];
            if parent != -1 && offscreen(parent as usize) == -1 {
                opacity *= self.part_opacity[parent as usize];
                self.part_opacity[i] = opacity;
            }
            let o = offscreen(i);
            if o != -1 {
                self.offscreen_opacity[o as usize] *= opacity;
            }
        }
    }

    fn apply_parts_to_meshes(&mut self, m: &Moc) {
        let a = &m.art_meshes;
        for i in 0..a.ids.len() {
            if !self.mesh_enable[i] {
                continue;
            }
            let pp = a.parent_part[i];
            if pp != -1 {
                let has_offscreen = m.version >= VERSION_53 && m.parts.offscreen[pp as usize] != -1;
                if !has_offscreen {
                    self.mesh_opacity[i] *= self.part_opacity[pp as usize];
                }
            }
        }
        if m.version < VERSION_42 {
            return;
        }
        for i in 0..a.ids.len() {
            if !self.mesh_enable[i] {
                continue;
            }
            let pd = a.parent_deformer[i];
            if pd == -1 {
                continue;
            }
            let pm = self.deformer_multiply[pd as usize];
            let ps = self.deformer_screen[pd as usize];
            let mm = self.mesh_multiply[i];
            let ms = self.mesh_screen[i];
            self.mesh_multiply[i] = [
                clamp01(mm[0] * pm[0]),
                clamp01(mm[1] * pm[1]),
                clamp01(mm[2] * pm[2]),
                1.0,
            ];
            self.mesh_screen[i] = [
                clamp01(ms[0] + ps[0] - ms[0] * ps[0]),
                clamp01(ms[1] + ps[1] - ms[1] * ps[1]),
                clamp01(ms[2] + ps[2] - ms[2] * ps[2]),
                1.0,
            ];
        }
    }

    fn apply_glues(&mut self, m: &Moc) {
        for g in 0..m.glues.ids.len() {
            let info = span(m.glues.info_off[g], m.glues.info_len[g]);
            let (off, len) = (info.start, info.len());
            if len == 0 {
                continue;
            }
            let (m0, m1) = (m.glues.art_mesh_a[g] as usize, m.glues.art_mesh_b[g] as usize);
            let intensity = self.glue_intensity[g];
            let mut i = 0;
            while i + 1 < len {
                let i0 = m.glue_info.vertex[off + i] as usize;
                let i1 = m.glue_info.vertex[off + i + 1] as usize;
                let w0 = m.glue_info.weight[off + i];
                let w1 = m.glue_info.weight[off + i + 1];
                let a = [
                    self.mesh_positions[m0][2 * i0],
                    self.mesh_positions[m0][2 * i0 + 1],
                ];
                let b = [
                    self.mesh_positions[m1][2 * i1],
                    self.mesh_positions[m1][2 * i1 + 1],
                ];
                let dv = [b[0] - a[0], b[1] - a[1]];
                self.mesh_positions[m0][2 * i0] = a[0] + dv[0] * (intensity * w0);
                self.mesh_positions[m0][2 * i0 + 1] = a[1] + dv[1] * (intensity * w0);
                self.mesh_positions[m1][2 * i1] = b[0] - dv[0] * (intensity * w1);
                self.mesh_positions[m1][2 * i1 + 1] = b[1] - dv[1] * (intensity * w1);
                i += 2;
            }
        }
    }

    fn sort_render_order(&mut self, m: &Moc) {
        let groups = m.draw_groups.object_off.len();
        if groups == 0 {
            return;
        }
        let meshes = m.art_meshes.ids.len();
        // Draw order of each object in each group.
        let mut orders: Vec<Vec<i32>> = Vec::with_capacity(groups);
        for g in 0..groups {
            let off = m.draw_groups.object_off[g] as usize;
            let len = m.draw_groups.object_len[g] as usize;
            let min = m.draw_groups.min_order[g];
            orders.push(
                (off..off + len)
                    .map(|k| {
                        let o = m.draw_objects.index[k] as usize;
                        if m.draw_objects.kind[k] == DRAW_OBJECT_PART {
                            if self.part_enable[o] {
                                self.part_draw_order[o]
                            } else {
                                min
                            }
                        } else if self.mesh_enable[o] {
                            self.mesh_draw_order[o]
                        } else {
                            min
                        }
                    })
                    .collect(),
            );
        }
        let mut cursor = vec![0i32; groups];
        for g in 0..groups {
            let off = m.draw_groups.object_off[g] as usize;
            let len = m.draw_groups.object_len[g] as usize;
            let min = m.draw_groups.min_order[g];
            let levels = (m.draw_groups.max_order[g] - min + 1).max(0) as usize;
            if levels == 0 || len == 0 {
                continue;
            }
            // Bucket by order (stable within a bucket), then walk.
            let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); levels];
            for (j, &order) in orders[g].iter().enumerate() {
                let rel = (order as i64 - min as i64).clamp(0, levels as i64 - 1) as usize;
                buckets[rel].push(j);
            }
            let mut pos = cursor[g];
            for bucket in &buckets {
                for &j in bucket {
                    let k = off + j;
                    let o = m.draw_objects.index[k] as usize;
                    if m.draw_objects.kind[k] == DRAW_OBJECT_PART {
                        if m.version >= VERSION_53 {
                            let oi = m.parts.offscreen[o];
                            if oi >= 0 {
                                self.render_order[meshes + oi as usize] = pos;
                                pos += 1;
                            }
                        }
                        let child = m.draw_objects.self_group[k] as usize;
                        cursor[child] = pos;
                        pos += m.draw_groups.total_count[child];
                    } else {
                        self.render_order[o] = pos;
                        pos += 1;
                    }
                }
            }
        }
    }
}

/// Which key segment `value` falls in: (lower key index, weight toward the
/// next key, outside the keys).
fn find_key_segment(value: f32, keys: &[f32], snap: f32, interp: f32) -> (usize, f32, bool) {
    let n = keys.len();
    if n == 1 {
        let k0 = keys[0];
        let outside = value <= k0 - snap || value >= k0 + snap;
        return (0, 0.0, outside);
    }
    let mut k0 = keys[0];
    if value < k0 - snap {
        return (0, 0.0, true);
    }
    if value < k0 + snap {
        return (0, 0.0, false);
    }
    let mut k1 = keys[1];
    if value < k1 - snap {
        let diff = k1 - k0;
        let weight = if diff >= interp { (value - k0) / diff } else { 0.0 };
        return (0, weight, false);
    }
    if value < k1 + snap {
        return (1, 0.0, false);
    }
    for (k, &key) in keys.iter().enumerate().skip(2) {
        k0 = k1;
        k1 = key;
        if value < k1 - snap {
            let diff = k1 - k0;
            let weight = if diff >= interp { (value - k0) / diff } else { 0.0 };
            return (k - 1, weight, false);
        }
        if value < k1 + snap {
            return (k, 0.0, false);
        }
    }
    (n - 1, 0.0, true)
}

/// Core's clean-up of a drawable's mask list at load.
fn clean_masks(masks: &[i32]) -> Vec<i32> {
    let mut m = masks.to_vec();
    let mut valid = m.len();
    if valid > 1 {
        let mut w = 0;
        let mut j = 0;
        while j + 1 < valid {
            while w + 1 < valid && m[w] >= 0 {
                w += 1;
            }
            if w + 1 < valid {
                m.remove(w);
                valid -= 1;
            }
            j += 1;
        }
    }
    if valid > 0 && m[valid - 1] < 0 {
        valid -= 1;
    }
    m.truncate(valid);
    m
}

#[derive(Clone, Copy)]
struct V2 {
    x: f32,
    y: f32,
}

fn v2(x: f32, y: f32) -> V2 {
    V2 { x, y }
}

fn load(pos: &[f32], i: usize) -> V2 {
    v2(pos[i * 2], pos[i * 2 + 1])
}

fn add(a: V2, b: V2) -> V2 {
    v2(a.x + b.x, a.y + b.y)
}

fn sub(a: V2, b: V2) -> V2 {
    v2(a.x - b.x, a.y - b.y)
}

fn scale(a: V2, s: f32) -> V2 {
    v2(a.x * s, a.y * s)
}

fn bary3(a: V2, b: V2, c: V2, wa: f32, wb: f32, wc: f32) -> V2 {
    v2(wc * c.x + (wb * b.x + wa * a.x), wc * c.y + (wb * b.y + wa * a.y))
}

fn bilinear(p00: V2, p10: V2, p01: V2, p11: V2, u: f32, v: f32) -> V2 {
    let iu = 1.0 - u;
    let x0 = u * p10.x + iu * p00.x;
    let y0 = u * p10.y + iu * p00.y;
    let x1 = u * p11.x + iu * p01.x;
    let y1 = u * p11.y + iu * p01.y;
    let iv = 1.0 - v;
    v2(v * x1 + iv * x0, v * y1 + iv * y0)
}

struct Cell {
    fu: f32,
    fv: f32,
    p00: V2,
    p10: V2,
    p01: V2,
    p11: V2,
}

fn triangle(c: &Cell) -> V2 {
    if c.fu + c.fv <= 1.0 {
        let w00 = 1.0 - c.fu - c.fv;
        bary3(c.p00, c.p10, c.p01, w00, c.fu, c.fv)
    } else {
        let w10 = 1.0 - c.fv;
        let w11 = c.fu + c.fv - 1.0;
        let w01 = 1.0 - c.fu;
        bary3(c.p10, c.p11, c.p01, w10, w11, w01)
    }
}

/// Map points from a warp's (u, v) space through its lattice `pos`
/// ((cols + 1) × (rows + 1) points, row-major), extrapolating outside the
/// unit square as Core does. Points are updated in place.
pub fn warp_transform(pos: &[f32], rows: i32, cols: i32, quad: bool, points: &mut [f32]) {
    let stride = (cols + 1) as usize;
    let (fr, fc) = (rows as f32, cols as f32);
    let (row, col) = (rows as usize, cols as usize);
    let mut basis: Option<(V2, V2, V2)> = None;
    for p in points.chunks_exact_mut(2) {
        let (u, v) = (p[0], p[1]);
        let gu = u * fc;
        let gv = v * fr;
        let r = if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
            // Rounding can land exactly on the far edge; stay in the last cell.
            let cu = (gu as usize).min(col - 1);
            let cv = (gv as usize).min(row - 1);
            let fu = gu - cu as f32;
            let fv = gv - cv as f32;
            let bi = cv * stride + cu;
            let p00 = load(pos, bi);
            let p10 = load(pos, bi + 1);
            let p01 = load(pos, bi + stride);
            let p11 = load(pos, bi + stride + 1);
            if quad {
                bilinear(p00, p10, p01, p11, fu, fv)
            } else {
                triangle(&Cell {
                    fu,
                    fv,
                    p00,
                    p10,
                    p01,
                    p11,
                })
            }
        } else {
            let (center, dv, du) = *basis.get_or_insert_with(|| {
                let c00 = v2(pos[0], pos[1]);
                let c10 = load(pos, col);
                let c01 = load(pos, row * stride);
                let c11 = load(pos, row * stride + col);
                let d11_00 = sub(c11, c00);
                let d10_01 = sub(c10, c01);
                let dpdv = scale(sub(d11_00, d10_01), 0.5);
                let dpdu = scale(add(d10_01, d11_00), 0.5);
                let sum = add(add(c00, c10), add(c01, c11));
                let center = sub(scale(sum, 0.25), scale(d11_00, 0.5));
                (center, dpdv, dpdu)
            });
            if u > -2.0 && u < 3.0 && v > -2.0 && v < 3.0 {
                triangle(&extrapolated_cell(
                    u, v, gu, gv, row, col, stride, pos, center, dv, du,
                ))
            } else {
                v2(du.x * u + center.x + dv.x * v, du.y * u + center.y + dv.y * v)
            }
        };
        p[0] = r.x;
        p[1] = r.y;
    }
}

#[allow(clippy::too_many_arguments)]
fn extrapolated_cell(
    u: f32,
    v: f32,
    gu: f32,
    gv: f32,
    row: usize,
    col: usize,
    stride: usize,
    pos: &[f32],
    cen: V2,
    dv: V2,
    du: V2,
) -> Cell {
    let (fr, fc) = (row as f32, col as f32);
    let (mut cu, mut cv) = (0usize, 0usize);
    let (mut uc, mut un, mut vc, mut vn) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let fu = if u <= 0.0 {
        (u + 2.0) * 0.5
    } else if u >= 1.0 {
        (u - 1.0) * 0.5
    } else {
        cu = gu as usize;
        if cu == col {
            cu = col - 1;
        }
        uc = cu as f32 / fc;
        un = (cu + 1) as f32 / fc;
        gu - cu as f32
    };
    let fv = if v <= 0.0 {
        (v + 2.0) * 0.5
    } else if v >= 1.0 {
        (v - 1.0) * 0.5
    } else {
        cv = gv as usize;
        if cv == row {
            cv = row - 1;
        }
        vc = cv as f32 / fr;
        vn = (cv + 1) as f32 / fr;
        gv - cv as f32
    };
    let (p00, p10, p01, p11);
    if u <= 0.0 {
        if v <= 0.0 {
            p00 = sub(cen, add(scale(dv, 2.0), scale(du, 2.0)));
            p10 = sub(cen, scale(dv, 2.0));
            p01 = sub(cen, scale(du, 2.0));
            p11 = v2(pos[0], pos[1]);
        } else if v < 1.0 {
            p00 = add(sub(cen, scale(du, 2.0)), scale(dv, vc));
            p10 = load(pos, cv * stride);
            p01 = add(sub(cen, scale(du, 2.0)), scale(dv, vn));
            p11 = load(pos, (cv + 1) * stride);
        } else {
            p00 = add(sub(cen, scale(du, 2.0)), dv);
            p10 = load(pos, row * stride);
            p01 = add(sub(cen, scale(du, 2.0)), scale(dv, 3.0));
            p11 = add(cen, scale(dv, 3.0));
        }
    } else if u < 1.0 {
        if v <= 0.0 {
            p00 = add(scale(du, uc), sub(cen, scale(dv, 2.0)));
            p10 = add(scale(du, un), sub(cen, scale(dv, 2.0)));
            p01 = load(pos, cu);
            p11 = load(pos, cu + 1);
        } else {
            p00 = load(pos, row * stride + cu);
            p10 = load(pos, row * stride + cu + 1);
            p01 = add(add(cen, scale(du, uc)), scale(dv, 3.0));
            p11 = add(add(cen, scale(du, un)), scale(dv, 3.0));
        }
    } else if v <= 0.0 {
        p00 = add(sub(cen, scale(dv, 2.0)), du);
        p10 = add(sub(cen, scale(dv, 2.0)), scale(du, 3.0));
        p01 = load(pos, col);
        p11 = add(cen, scale(du, 3.0));
    } else if v < 1.0 {
        p00 = load(pos, col + cv * stride);
        p10 = add(add(cen, scale(du, 3.0)), scale(dv, vc));
        p01 = load(pos, col + (cv + 1) * stride);
        p11 = add(add(cen, scale(du, 3.0)), scale(dv, vn));
    } else {
        p00 = load(pos, row * stride + col);
        p10 = add(add(cen, scale(du, 3.0)), dv);
        p01 = add(add(cen, scale(dv, 3.0)), du);
        p11 = add(cen, add(scale(du, 3.0), scale(dv, 3.0)));
    }
    Cell {
        fu,
        fv,
        p00,
        p10,
        p01,
        p11,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warp_interpolation_is_exact_on_a_regular_grid() {
        // A 2×2 grid over (10, 20)-(30, 40): (u, v) maps affinely, inside
        // and (by extrapolation) outside.
        let mut pos = Vec::new();
        for j in 0..3 {
            for i in 0..3 {
                pos.extend([10.0 + 10.0 * i as f32, 20.0 + 10.0 * j as f32]);
            }
        }
        for quad in [false, true] {
            let mut pts = vec![0.25, 0.75, 0.5, 0.5, -0.5, 0.2, 1.5, 2.5, -3.0, 4.0];
            warp_transform(&pos, 2, 2, quad, &mut pts);
            let expected = [15.0, 35.0, 20.0, 30.0, 0.0, 24.0, 40.0, 70.0, -50.0, 100.0];
            for (a, b) in pts.iter().zip(expected) {
                assert!((a - b).abs() < 1e-4, "{pts:?}");
            }
        }
    }

    #[test]
    fn keys_segment_and_snap() {
        let keys = [-30.0, 0.0, 30.0];
        assert_eq!(find_key_segment(-15.0, &keys, 0.1, 0.15), (0, 0.5, false));
        assert_eq!(find_key_segment(0.05, &keys, 0.1, 0.15), (1, 0.0, false));
        assert_eq!(find_key_segment(30.0, &keys, 0.1, 0.15), (2, 0.0, false));
        assert!(find_key_segment(31.0, &keys, 0.1, 0.15).2);
        // A single key only covers its own value.
        assert!(!find_key_segment(1.0, &[1.0], 0.1, 0.15).2);
        assert!(find_key_segment(0.5, &[1.0], 0.1, 0.15).2);
    }

    #[test]
    fn masks_are_cleaned_like_core() {
        assert_eq!(clean_masks(&[3, -1]), vec![3]);
        assert_eq!(clean_masks(&[-1, 3, 4]), vec![3, 4]);
        assert_eq!(clean_masks(&[]), Vec::<i32>::new());
    }

    #[test]
    fn corrupted_models_that_validate_never_panic() {
        let bytes = crate::moc3::tests_support::tiny_bytes();
        let mut read = 0;
        for i in 64..bytes.len() {
            for flip in [0x01u8, 0x80, 0xff] {
                let mut b = bytes.clone();
                b[i] ^= flip;
                if let Ok(moc) = Moc::read(&b) {
                    read += 1;
                    let mut model = Model::new(Arc::new(moc));
                    model.parameter_values.iter_mut().for_each(|v| *v = 0.7);
                    model.update();
                    model.update();
                }
            }
        }
        assert!(read > 0);
    }
}
