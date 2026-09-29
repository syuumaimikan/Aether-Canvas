//! Building a [`Moc`] from objects.
//!
//! A `.moc3` file is a web of flat arrays that index into each other: key
//! tables, bindings, keyform ranges, position offsets, colour offsets,
//! draw-order groups. [`MocBuilder`] takes the model the way it is authored —
//! parameters, parts, deformers and art meshes, each with a grid of keyforms
//! over some parameters — and lays those arrays out, sharing identical key
//! tables and bindings the way Cubism Editor does.
//!
//! Conventions follow the file format: positions are in the parent's space
//! (model units, y down, for objects at the root; the unit square for
//! children of a warp; pixels about the pivot for children of a rotation),
//! texture coordinates are v down, and a grid's first axis varies fastest.

use crate::moc3::*;
use std::collections::BTreeMap;
use thiserror::Error;

/// Why a model could not be built.
#[derive(Debug, Error, Clone, PartialEq)]
#[error("cannot build the MOC3 model: {0}")]
pub struct BuildError(pub String);

fn error(message: impl Into<String>) -> BuildError {
    BuildError(message.into())
}

/// One axis of a keyform grid: a parameter (by index) and its keys.
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub parameter: usize,
    /// Strictly increasing; at least two (a one-key axis would disable the
    /// object whenever the parameter leaves that key).
    pub keys: Vec<f32>,
}

/// Keyforms laid out on a grid of parameter keys; the first axis varies
/// fastest. No axes means one form that always applies.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid<T> {
    pub axes: Vec<Axis>,
    pub forms: Vec<T>,
}

impl<T> Grid<T> {
    /// A single form.
    pub fn constant(form: T) -> Self {
        Self {
            axes: Vec::new(),
            forms: vec![form],
        }
    }

    fn expected(&self) -> usize {
        self.axes.iter().map(|a| a.keys.len()).product()
    }
}

/// A parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterDef {
    pub id: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Wraps around instead of clamping.
    pub repeat: bool,
    /// Keys closer than `0.1^decimal_places` to a value snap to it.
    pub decimal_places: i32,
}

/// A part (a folder of the model).
#[derive(Clone, Debug, PartialEq)]
pub struct PartDef {
    pub id: String,
    pub parent: Option<usize>,
    /// Sorts its children in a draw-order group of their own, drawn as one
    /// block at the part's own draw order.
    pub group: bool,
    /// Draw order within the parent's group (500 is Cubism's default).
    pub draw_order: Grid<f32>,
}

/// A multiply and a screen colour (RGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub multiply: [f32; 3],
    pub screen: [f32; 3],
}

impl Default for Tint {
    fn default() -> Self {
        Self {
            multiply: [1.0; 3],
            screen: [0.0; 3],
        }
    }
}

/// A warp deformer's lattice at one keyform.
#[derive(Clone, Debug, PartialEq)]
pub struct WarpForm {
    /// `(rows + 1) × (cols + 1)` points, row-major, x and y interleaved.
    pub positions: Vec<f32>,
    pub opacity: f32,
    pub tint: Tint,
}

/// A rotation deformer at one keyform.
#[derive(Clone, Debug, PartialEq)]
pub struct RotationForm {
    /// The pivot, in the parent's space.
    pub origin: [f32; 2],
    /// Degrees.
    pub angle: f32,
    /// Children are in pixels about the pivot; the scale converts them to
    /// the parent's units (1 / pixels-per-unit at the top of a chain).
    pub scale: f32,
    pub reflect_x: bool,
    pub reflect_y: bool,
    pub opacity: f32,
    pub tint: Tint,
}

/// What kind of deformer.
#[derive(Clone, Debug, PartialEq)]
pub enum DeformerKind {
    Warp {
        rows: usize,
        cols: usize,
        /// Bilinear cells (Cubism 3.3+) instead of two triangles each.
        quad: bool,
        forms: Grid<WarpForm>,
    },
    Rotation {
        base_angle: f32,
        forms: Grid<RotationForm>,
    },
}

/// A deformer. Parents must come before their children.
#[derive(Clone, Debug, PartialEq)]
pub struct DeformerDef {
    pub id: String,
    pub parent_part: Option<usize>,
    pub parent_deformer: Option<usize>,
    pub kind: DeformerKind,
}

/// An art mesh at one keyform.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshForm {
    /// x and y interleaved, in the parent deformer's space.
    pub positions: Vec<f32>,
    pub opacity: f32,
    pub draw_order: f32,
    pub tint: Tint,
}

/// How an art mesh blends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshBlend {
    #[default]
    Normal,
    Add,
    Multiply,
    /// A Cubism 5.3 colour blend code (written only to 5.3 files).
    Code(i32),
}

/// An art mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshDef {
    pub id: String,
    pub parent_part: Option<usize>,
    pub parent_deformer: Option<usize>,
    pub texture: usize,
    /// Texture coordinates, v down, one per vertex.
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u16; 3]>,
    /// Art meshes whose texture alpha clips this one.
    pub masks: Vec<usize>,
    pub inverted_mask: bool,
    pub double_sided: bool,
    pub blend: MeshBlend,
    pub forms: Grid<MeshForm>,
}

/// Glue between two art meshes.
#[derive(Clone, Debug, PartialEq)]
pub struct GlueDef {
    pub id: String,
    pub mesh_a: usize,
    pub mesh_b: usize,
    /// `(vertex on a, vertex on b, pull on a, pull on b)`.
    pub pairs: Vec<(u16, u16, f32, f32)>,
    pub intensity: Grid<f32>,
}

/// The model to build.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MocBuilder {
    pub canvas: Canvas,
    pub parameters: Vec<ParameterDef>,
    pub parts: Vec<PartDef>,
    pub deformers: Vec<DeformerDef>,
    pub meshes: Vec<MeshDef>,
    pub glues: Vec<GlueDef>,
    /// Back-to-front order of the top-level draw group's members; members of
    /// grouped parts are ordered by their position in this list too. Meshes
    /// and parts left out follow in index order.
    pub draw_list: Vec<DrawRef>,
}

/// A member of a draw-order group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DrawRef {
    Mesh(usize),
    Part(usize),
}

/// Key tables and bindings, shared between objects.
#[derive(Default)]
struct Tables {
    /// Per parameter: its distinct key lists.
    per_param: BTreeMap<usize, Vec<Vec<f32>>>,
    bindings: Vec<Vec<(usize, usize)>>,
}

impl Tables {
    fn table(&mut self, parameter: usize, keys: &[f32]) -> (usize, usize) {
        let lists = self.per_param.entry(parameter).or_default();
        let at = match lists.iter().position(|k| k == keys) {
            Some(i) => i,
            None => {
                lists.push(keys.to_vec());
                lists.len() - 1
            }
        };
        (parameter, at)
    }

    fn binding(&mut self, axes: &[Axis]) -> usize {
        let key: Vec<(usize, usize)> = axes.iter().map(|a| self.table(a.parameter, &a.keys)).collect();
        match self.bindings.iter().position(|b| *b == key) {
            Some(i) => i,
            None => {
                self.bindings.push(key);
                self.bindings.len() - 1
            }
        }
    }
}

fn check_grid<T>(what: &str, grid: &Grid<T>, parameters: &[ParameterDef]) -> Result<(), BuildError> {
    for (i, axis) in grid.axes.iter().enumerate() {
        if axis.parameter >= parameters.len() {
            return Err(error(format!("{what} is keyed on a missing parameter")));
        }
        if axis.keys.len() < 2 {
            return Err(error(format!("{what} has an axis with fewer than two keys")));
        }
        let increasing = axis.keys.windows(2).all(|w| w[1] > w[0]);
        if !increasing || axis.keys.iter().any(|k| !k.is_finite()) {
            return Err(error(format!("{what} has keys that do not increase")));
        }
        if grid.axes[..i].iter().any(|b| b.parameter == axis.parameter) {
            return Err(error(format!("{what} uses one parameter twice")));
        }
    }
    if grid.axes.len() > 16 {
        return Err(error(format!("{what} is keyed on more than 16 parameters")));
    }
    if grid.forms.len() != grid.expected() {
        return Err(error(format!(
            "{what} has {} keyforms where its keys call for {}",
            grid.forms.len(),
            grid.expected()
        )));
    }
    Ok(())
}

fn index(i: Option<usize>) -> i32 {
    i.map(|i| i as i32).unwrap_or(-1)
}

/// `to_i32(x + 0.001)` as Core rounds draw orders.
fn order(x: f32) -> i32 {
    (x + 0.001).floor() as i32
}

impl MocBuilder {
    /// Lay the model out as a `.moc3` of `version` (at least
    /// [`VERSION_33`], for bilinear warps).
    pub fn build(&self, version: u8) -> Result<Moc, BuildError> {
        let version = version.clamp(VERSION_33, LATEST_VERSION);
        let colors = version >= VERSION_42;
        let np = self.parameters.len();
        for p in &self.parameters {
            let valid = p.min < p.max && p.min <= p.default && p.default <= p.max;
            if !valid {
                return Err(error(format!("parameter {} has an invalid range", p.id)));
            }
        }
        for (i, part) in self.parts.iter().enumerate() {
            check_grid(&format!("part {}", part.id), &part.draw_order, &self.parameters)?;
            if part.parent.is_some_and(|p| p >= i) {
                return Err(error(format!("part {} must follow its parent", part.id)));
            }
        }
        for (i, d) in self.deformers.iter().enumerate() {
            if d.parent_deformer.is_some_and(|p| p >= i) {
                return Err(error(format!("deformer {} must follow its parent", d.id)));
            }
            if d.parent_part.is_some_and(|p| p >= self.parts.len()) {
                return Err(error(format!("deformer {} is in a missing part", d.id)));
            }
            match &d.kind {
                DeformerKind::Warp {
                    rows, cols, forms, ..
                } => {
                    check_grid(&format!("deformer {}", d.id), forms, &self.parameters)?;
                    let n = (rows + 1) * (cols + 1) * 2;
                    if *rows == 0 || *cols == 0 || forms.forms.iter().any(|f| f.positions.len() != n) {
                        return Err(error(format!("warp {} has a lattice of the wrong size", d.id)));
                    }
                }
                DeformerKind::Rotation { forms, .. } => {
                    check_grid(&format!("deformer {}", d.id), forms, &self.parameters)?;
                }
            }
        }
        for m in &self.meshes {
            check_grid(&format!("art mesh {}", m.id), &m.forms, &self.parameters)?;
            let n = m.uvs.len();
            if n == 0 || n > u16::MAX as usize {
                return Err(error(format!("art mesh {} has {n} vertices", m.id)));
            }
            if m.forms.forms.iter().any(|f| f.positions.len() != n * 2) {
                return Err(error(format!("art mesh {} has keyforms of the wrong size", m.id)));
            }
            if m.triangles.iter().flatten().any(|&v| v as usize >= n) {
                return Err(error(format!(
                    "art mesh {} has a triangle past its vertices",
                    m.id
                )));
            }
            if m.masks.iter().any(|&k| k >= self.meshes.len()) {
                return Err(error(format!("art mesh {} is masked by a missing mesh", m.id)));
            }
            if m.parent_deformer.is_some_and(|d| d >= self.deformers.len())
                || m.parent_part.is_some_and(|p| p >= self.parts.len())
            {
                return Err(error(format!("art mesh {} has a missing parent", m.id)));
            }
        }
        for g in &self.glues {
            check_grid(&format!("glue {}", g.id), &g.intensity, &self.parameters)?;
            let (Some(a), Some(b)) = (self.meshes.get(g.mesh_a), self.meshes.get(g.mesh_b)) else {
                return Err(error(format!("glue {} joins a missing mesh", g.id)));
            };
            if g.pairs
                .iter()
                .any(|&(va, vb, _, _)| va as usize >= a.uvs.len() || vb as usize >= b.uvs.len())
            {
                return Err(error(format!("glue {} joins a missing vertex", g.id)));
            }
        }

        // Key tables and bindings. Binding 0 is the empty one.
        let mut tables = Tables::default();
        tables.bindings.push(Vec::new());
        let part_bindings: Vec<usize> = self
            .parts
            .iter()
            .map(|p| tables.binding(&p.draw_order.axes))
            .collect();
        let deformer_bindings: Vec<usize> = self
            .deformers
            .iter()
            .map(|d| match &d.kind {
                DeformerKind::Warp { forms, .. } => tables.binding(&forms.axes),
                DeformerKind::Rotation { forms, .. } => tables.binding(&forms.axes),
            })
            .collect();
        let mesh_bindings: Vec<usize> = self
            .meshes
            .iter()
            .map(|m| tables.binding(&m.forms.axes))
            .collect();
        let glue_bindings: Vec<usize> = self
            .glues
            .iter()
            .map(|g| tables.binding(&g.intensity.axes))
            .collect();

        let mut m = Moc {
            version,
            canvas: self.canvas,
            ..Default::default()
        };

        // Parameters own contiguous runs of key tables.
        let mut table_index: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for (pi, p) in self.parameters.iter().enumerate() {
            let q = &mut m.parameters;
            q.ids.push(p.id.clone());
            q.max.push(p.max);
            q.min.push(p.min);
            q.default.push(p.default);
            q.repeat.push(p.repeat as i32);
            q.decimal_places.push(p.decimal_places);
            let lists = tables.per_param.get(&pi).cloned().unwrap_or_default();
            q.key_table_off.push(m.key_tables.keys_off.len() as i32);
            q.key_table_len.push(lists.len() as i32);
            for (li, keys) in lists.iter().enumerate() {
                table_index.insert((pi, li), m.key_tables.keys_off.len());
                m.key_tables.keys_off.push(m.keys.len() as i32);
                m.key_tables.keys_len.push(keys.len() as i32);
                m.keys.extend_from_slice(keys);
            }
        }
        if colors {
            for pi in 0..np {
                let mut all: Vec<f32> = tables
                    .per_param
                    .get(&pi)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .copied()
                    .collect();
                all.sort_by(f32::total_cmp);
                all.dedup();
                let q = &mut m.parameters;
                q.keys_off
                    .push(if all.is_empty() { 0 } else { m.keys.len() as i32 });
                q.keys_len.push(all.len() as i32);
                q.kind.push(PARAMETER_NORMAL);
                q.blend_key_table_off.push(0);
                q.blend_key_table_len.push(0);
                m.keys.extend(all);
            }
        }
        for b in &tables.bindings {
            m.bindings.key_table_off.push(m.key_table_indices.len() as i32);
            m.bindings.key_table_len.push(b.len() as i32);
            for t in b {
                m.key_table_indices.push(table_index[t] as i32);
            }
        }

        // Colours: each object's keyforms own a contiguous run.
        let push_tints = |m: &mut Moc, tints: &mut dyn Iterator<Item = Tint>| -> i32 {
            let off = m.multiply_colors.r.len() as i32;
            for t in tints {
                m.multiply_colors.r.push(t.multiply[0]);
                m.multiply_colors.g.push(t.multiply[1]);
                m.multiply_colors.b.push(t.multiply[2]);
                m.screen_colors.r.push(t.screen[0]);
                m.screen_colors.g.push(t.screen[1]);
                m.screen_colors.b.push(t.screen[2]);
            }
            off
        };

        // Parts.
        for (i, part) in self.parts.iter().enumerate() {
            let p = &mut m.parts;
            p.ids.push(part.id.clone());
            p.binding.push(part_bindings[i] as i32);
            p.keyform_off.push(m.part_keyforms.draw_order.len() as i32);
            p.key_len.push(part.draw_order.forms.len() as i32);
            p.visible.push(1);
            p.enable.push(1);
            p.parent_part.push(index(part.parent));
            if version >= VERSION_53 {
                p.offscreen.push(-1);
            }
            for &o in &part.draw_order.forms {
                m.part_keyforms.draw_order.push(o);
                if version >= VERSION_53 {
                    m.part_keyforms.offscreen_keyform.push(-1);
                }
            }
        }

        // Deformers.
        for (i, d) in self.deformers.iter().enumerate() {
            let binding = deformer_bindings[i] as i32;
            m.deformers.ids.push(d.id.clone());
            m.deformers.binding.push(binding);
            m.deformers.visible.push(1);
            m.deformers.enable.push(1);
            m.deformers.parent_part.push(index(d.parent_part));
            m.deformers.parent_deformer.push(index(d.parent_deformer));
            match &d.kind {
                DeformerKind::Warp {
                    rows,
                    cols,
                    quad,
                    forms,
                } => {
                    m.deformers.kind.push(DEFORMER_WARP);
                    m.deformers.local_index.push(m.warps.binding.len() as i32);
                    let w = &mut m.warps;
                    w.binding.push(binding);
                    w.keyform_off.push(m.warp_keyforms.opacity.len() as i32);
                    w.key_len.push(forms.forms.len() as i32);
                    w.vertex_count.push(((rows + 1) * (cols + 1)) as i32);
                    w.rows.push(*rows as i32);
                    w.cols.push(*cols as i32);
                    w.quad.push(*quad as i32);
                    if colors {
                        let off = push_tints(&mut m, &mut forms.forms.iter().map(|f| f.tint));
                        m.warps.color_off.push(off);
                    }
                    for (k, f) in forms.forms.iter().enumerate() {
                        m.warp_keyforms.opacity.push(f.opacity);
                        m.warp_keyforms
                            .position_off
                            .push(m.keyform_positions.len() as i32);
                        m.keyform_positions.extend_from_slice(&f.positions);
                        if version >= VERSION_50 {
                            let c = *m.warps.color_off.last().unwrap_or(&0) + k as i32;
                            m.warp_keyforms.multiply_off.push(c);
                            m.warp_keyforms.screen_off.push(c);
                        }
                    }
                }
                DeformerKind::Rotation { base_angle, forms } => {
                    m.deformers.kind.push(DEFORMER_ROTATION);
                    m.deformers.local_index.push(m.rotations.binding.len() as i32);
                    let r = &mut m.rotations;
                    r.binding.push(binding);
                    r.keyform_off.push(m.rotation_keyforms.opacity.len() as i32);
                    r.key_len.push(forms.forms.len() as i32);
                    r.base_angle.push(*base_angle);
                    if colors {
                        let off = push_tints(&mut m, &mut forms.forms.iter().map(|f| f.tint));
                        m.rotations.color_off.push(off);
                    }
                    for (k, f) in forms.forms.iter().enumerate() {
                        let rk = &mut m.rotation_keyforms;
                        rk.opacity.push(f.opacity);
                        rk.angle.push(f.angle);
                        rk.origin_x.push(f.origin[0]);
                        rk.origin_y.push(f.origin[1]);
                        rk.scale.push(f.scale);
                        rk.reflect_x.push(f.reflect_x as i32);
                        rk.reflect_y.push(f.reflect_y as i32);
                        if version >= VERSION_50 {
                            let c = *m.rotations.color_off.last().unwrap_or(&0) + k as i32;
                            rk.multiply_off.push(c);
                            rk.screen_off.push(c);
                        }
                    }
                }
            }
        }

        // Art meshes.
        for (i, mesh) in self.meshes.iter().enumerate() {
            let binding = mesh_bindings[i] as i32;
            let (blend_flags, code) = match mesh.blend {
                MeshBlend::Normal => (0, 0),
                MeshBlend::Add => (FLAG_ADDITIVE, 2),
                MeshBlend::Multiply => (FLAG_MULTIPLICATIVE, 1),
                MeshBlend::Code(c) => (0, c),
            };
            let mut flags = if version >= VERSION_53 { 0 } else { blend_flags };
            if mesh.double_sided {
                flags |= FLAG_DOUBLE_SIDED;
            }
            if mesh.inverted_mask {
                flags |= FLAG_INVERTED_MASK;
            }
            let a = &mut m.art_meshes;
            a.ids.push(mesh.id.clone());
            a.binding.push(binding);
            a.keyform_off.push(m.art_mesh_keyforms.opacity.len() as i32);
            a.key_len.push(mesh.forms.forms.len() as i32);
            a.visible.push(1);
            a.enable.push(1);
            a.parent_part.push(index(mesh.parent_part));
            a.parent_deformer.push(index(mesh.parent_deformer));
            a.texture.push(mesh.texture as i32);
            a.flags.push(flags);
            a.vertex_count.push(mesh.uvs.len() as i32);
            a.uv_off.push(m.uvs.len() as i32);
            a.index_off.push(m.indices.len() as i32);
            a.index_len.push((mesh.triangles.len() * 3) as i32);
            a.mask_off.push(if mesh.masks.is_empty() {
                0
            } else {
                m.masks.len() as i32
            });
            a.mask_len.push(mesh.masks.len() as i32);
            if version >= VERSION_53 {
                // Cubism 5.3 codes: colour blend in the low byte, alpha
                // blend (over) in the next.
                a.blend_mode.push(code);
            }
            for uv in &mesh.uvs {
                m.uvs.extend_from_slice(uv);
            }
            for t in &mesh.triangles {
                m.indices.extend_from_slice(t);
            }
            m.masks.extend(mesh.masks.iter().map(|&k| k as i32));
            if colors {
                let off = push_tints(&mut m, &mut mesh.forms.forms.iter().map(|f| f.tint));
                m.art_meshes.color_off.push(off);
            }
            for (k, f) in mesh.forms.forms.iter().enumerate() {
                let ak = &mut m.art_mesh_keyforms;
                ak.opacity.push(f.opacity);
                ak.draw_order.push(f.draw_order);
                ak.position_off.push(m.keyform_positions.len() as i32);
                m.keyform_positions.extend_from_slice(&f.positions);
                if version >= VERSION_50 {
                    let c = *m.art_meshes.color_off.last().unwrap_or(&0) + k as i32;
                    m.art_mesh_keyforms.multiply_off.push(c);
                    m.art_mesh_keyforms.screen_off.push(c);
                }
            }
        }

        // Glue.
        for (i, g) in self.glues.iter().enumerate() {
            let gl = &mut m.glues;
            gl.ids.push(g.id.clone());
            gl.binding.push(glue_bindings[i] as i32);
            gl.keyform_off.push(m.glue_intensity.len() as i32);
            gl.key_len.push(g.intensity.forms.len() as i32);
            gl.art_mesh_a.push(g.mesh_a as i32);
            gl.art_mesh_b.push(g.mesh_b as i32);
            gl.info_off.push(m.glue_info.weight.len() as i32);
            gl.info_len.push((g.pairs.len() * 2) as i32);
            for &(va, vb, wa, wb) in &g.pairs {
                m.glue_info.vertex.extend_from_slice(&[va, vb]);
                m.glue_info.weight.extend_from_slice(&[wa, wb]);
            }
            m.glue_intensity.extend_from_slice(&g.intensity.forms);
        }

        self.draw_groups(&mut m);
        m.validate().map_err(|e| error(e.to_string()))?;
        Ok(m)
    }

    /// Draw-order groups: the root, plus one per grouped part. A member of a
    /// part without a group of its own joins the nearest grouped ancestor.
    fn draw_groups(&self, m: &mut Moc) {
        // The group each part's children sort in: its own, or its parent's.
        let mut group_of_part: Vec<Option<usize>> = Vec::with_capacity(self.parts.len());
        let mut own_group: Vec<Option<usize>> = vec![None; self.parts.len()];
        let mut groups: Vec<Vec<DrawRef>> = vec![Vec::new()];
        for (i, p) in self.parts.iter().enumerate() {
            let outer = p.parent.and_then(|q| group_of_part[q]).unwrap_or(0);
            if p.group {
                own_group[i] = Some(groups.len());
                groups.push(Vec::new());
                group_of_part.push(own_group[i]);
            } else {
                group_of_part.push(Some(outer));
            }
        }
        let container = |r: DrawRef| -> usize {
            match r {
                DrawRef::Mesh(i) => self.meshes[i]
                    .parent_part
                    .and_then(|p| group_of_part[p])
                    .unwrap_or(0),
                DrawRef::Part(i) => self.parts[i].parent.and_then(|p| group_of_part[p]).unwrap_or(0),
            }
        };
        // Members in draw-list order, then anything the list left out.
        let mut seen = std::collections::BTreeSet::new();
        let listed = self.draw_list.iter().copied();
        let rest = (0..self.meshes.len())
            .map(DrawRef::Mesh)
            .chain((0..self.parts.len()).map(DrawRef::Part));
        for r in listed.chain(rest) {
            let valid = match r {
                DrawRef::Mesh(i) => i < self.meshes.len(),
                DrawRef::Part(i) => i < self.parts.len() && own_group[i].is_some(),
            };
            if valid && seen.insert(r) {
                groups[container(r)].push(r);
            }
        }

        // Total drawables per group, children first.
        let mut totals = vec![0i32; groups.len()];
        for g in (0..groups.len()).rev() {
            totals[g] = groups[g]
                .iter()
                .map(|r| match *r {
                    DrawRef::Mesh(_) => 1,
                    DrawRef::Part(p) => totals[own_group[p].unwrap_or(0)],
                })
                .sum();
        }
        for (g, members) in groups.iter().enumerate() {
            let mut lo = i32::MAX;
            let mut hi = i32::MIN;
            m.draw_groups.object_off.push(m.draw_objects.kind.len() as i32);
            m.draw_groups.object_len.push(members.len() as i32);
            for r in members {
                let forms: &[f32] = match *r {
                    DrawRef::Mesh(i) => {
                        m.draw_objects.kind.push(DRAW_OBJECT_ART_MESH);
                        m.draw_objects.index.push(i as i32);
                        m.draw_objects.self_group.push(-1);
                        for f in &self.meshes[i].forms.forms {
                            lo = lo.min(order(f.draw_order));
                            hi = hi.max(order(f.draw_order));
                        }
                        &[]
                    }
                    DrawRef::Part(p) => {
                        m.draw_objects.kind.push(DRAW_OBJECT_PART);
                        m.draw_objects.index.push(p as i32);
                        m.draw_objects.self_group.push(own_group[p].unwrap_or(0) as i32);
                        &self.parts[p].draw_order.forms
                    }
                };
                for &o in forms {
                    lo = lo.min(order(o));
                    hi = hi.max(order(o));
                }
            }
            if lo > hi {
                (lo, hi) = (500, 500);
            }
            m.draw_groups.total_count.push(totals[g]);
            m.draw_groups.max_order.push(hi);
            m.draw_groups.min_order.push(lo);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;
    use std::sync::Arc;

    fn canvas() -> Canvas {
        Canvas {
            pixels_per_unit: 100.0,
            origin_x: 50.0,
            origin_y: 50.0,
            width: 100.0,
            height: 100.0,
            flags: 0,
        }
    }

    fn quad_mesh(id: &str, parent: Option<usize>, forms: Grid<MeshForm>) -> MeshDef {
        MeshDef {
            id: id.into(),
            parent_part: Some(0),
            parent_deformer: parent,
            texture: 0,
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            masks: Vec::new(),
            inverted_mask: false,
            double_sided: true,
            blend: MeshBlend::Normal,
            forms,
        }
    }

    fn form(positions: &[f32], draw_order: f32) -> MeshForm {
        MeshForm {
            positions: positions.to_vec(),
            opacity: 1.0,
            draw_order,
            tint: Tint::default(),
        }
    }

    /// A root warp holding a rotation holding a mesh keyed on a parameter,
    /// plus a second mesh at the root drawn behind it.
    fn builder() -> MocBuilder {
        let x = ParameterDef {
            id: "ParamX".into(),
            min: -1.0,
            max: 1.0,
            default: 0.0,
            repeat: false,
            decimal_places: 3,
        };
        let angle = ParameterDef {
            id: "ParamAngle".into(),
            min: -30.0,
            max: 30.0,
            default: 0.0,
            repeat: false,
            decimal_places: 3,
        };
        // A 1×1 lattice over (-0.5, -0.5)..(0.5, 0.5) units, shifted right
        // by 0.1 units at X = 1.
        let lattice = |dx: f32| WarpForm {
            positions: vec![-0.5 + dx, -0.5, 0.5 + dx, -0.5, -0.5 + dx, 0.5, 0.5 + dx, 0.5],
            opacity: 1.0,
            tint: Tint::default(),
        };
        let pivot = |angle: f32| RotationForm {
            origin: [0.5, 0.5],
            angle,
            scale: 1.0 / 100.0,
            reflect_x: false,
            reflect_y: false,
            opacity: 1.0,
            tint: Tint::default(),
        };
        MocBuilder {
            canvas: canvas(),
            parameters: vec![x, angle],
            parts: vec![PartDef {
                id: "Part".into(),
                parent: None,
                group: false,
                draw_order: Grid::constant(500.0),
            }],
            deformers: vec![
                DeformerDef {
                    id: "Warp".into(),
                    parent_part: Some(0),
                    parent_deformer: None,
                    kind: DeformerKind::Warp {
                        rows: 1,
                        cols: 1,
                        quad: true,
                        forms: Grid {
                            axes: vec![Axis {
                                parameter: 0,
                                keys: vec![-1.0, 0.0, 1.0],
                            }],
                            forms: vec![lattice(0.0), lattice(0.0), lattice(0.1)],
                        },
                    },
                },
                DeformerDef {
                    id: "Rotation".into(),
                    parent_part: Some(0),
                    parent_deformer: Some(0),
                    kind: DeformerKind::Rotation {
                        base_angle: 0.0,
                        forms: Grid {
                            axes: vec![Axis {
                                parameter: 1,
                                keys: vec![-30.0, 30.0],
                            }],
                            forms: vec![pivot(-90.0), pivot(90.0)],
                        },
                    },
                },
            ],
            meshes: vec![
                quad_mesh(
                    "Front",
                    Some(1),
                    Grid::constant(form(&[0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0], 600.0)),
                ),
                quad_mesh(
                    "Back",
                    None,
                    Grid {
                        axes: vec![Axis {
                            parameter: 0,
                            keys: vec![-1.0, 1.0],
                        }],
                        forms: vec![
                            form(&[-0.5, -0.5, 0.5, -0.5, 0.5, 0.5, -0.5, 0.5], 500.0),
                            form(&[-0.5, -0.5, 0.5, -0.5, 0.5, 0.5, -0.5, 0.5], 700.0),
                        ],
                    },
                ),
            ],
            glues: Vec::new(),
            draw_list: vec![DrawRef::Mesh(1), DrawRef::Mesh(0)],
        }
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn built_models_evaluate_as_authored() {
        for version in [VERSION_33, VERSION_42, VERSION_50, VERSION_53] {
            let moc = builder().build(version).expect("builds");
            // Survives a write and a read.
            let moc = Moc::read(&moc.write()).expect("reads back");
            let mut model = Model::new(Arc::new(moc));
            model.update();
            // At rest the rotation sits at the warp's centre (canvas pixel
            // 50, 50) unturned: the mesh's first vertex is there, its third
            // 10 pixels right and down.
            let front = model.drawable_positions_px(0);
            assert!(close(front[0], [50.0, 50.0]), "{front:?}");
            assert!(close(front[2], [60.0, 60.0]), "{front:?}");

            // Turned 90° clockwise on screen and carried right by the warp.
            model.parameter_values[1] = 30.0;
            model.parameter_values[0] = 1.0;
            model.update();
            let front = model.drawable_positions_px(0);
            assert!(close(front[0], [60.0, 50.0]), "{front:?}");
            assert!(close(front[1], [60.0, 60.0]), "{front:?}");

            // Draw order: "Back" (500) is behind "Front" (600) until its
            // keyed order passes it.
            model.parameter_values[0] = -1.0;
            model.update();
            assert!(model.drawable_render_order(1) < model.drawable_render_order(0));
            model.parameter_values[0] = 1.0;
            model.update();
            assert!(model.drawable_render_order(1) > model.drawable_render_order(0));
        }
    }

    #[test]
    fn identical_key_tables_and_bindings_are_shared() {
        let mut b = builder();
        let mut copy = b.meshes[1].clone();
        copy.id = "Copy".into();
        b.meshes.push(copy);
        let moc = b.build(VERSION_42).expect("builds");
        // Empty binding, warp's, rotation's, "Back"'s (shared by "Copy").
        assert_eq!(moc.bindings.key_table_off.len(), 4);
        assert_eq!(moc.key_tables.keys_off.len(), 3);
        assert_eq!(moc.art_meshes.binding[1], moc.art_meshes.binding[2]);
    }

    #[test]
    fn grouped_parts_draw_as_one_block() {
        let mut b = builder();
        b.parts.push(PartDef {
            id: "Group".into(),
            parent: Some(0),
            group: true,
            draw_order: Grid::constant(550.0),
        });
        b.meshes[0].parent_part = Some(1);
        b.draw_list = vec![DrawRef::Mesh(1), DrawRef::Part(1), DrawRef::Mesh(0)];
        let moc = b.build(VERSION_42).expect("builds");
        assert_eq!(moc.draw_groups.object_off.len(), 2);
        assert_eq!(moc.draw_groups.total_count, vec![2, 1]);
        let mut model = Model::new(Arc::new(moc));
        model.parameter_values[0] = -1.0;
        model.update();
        // "Back" at 500 is behind the group at 550.
        assert!(model.drawable_render_order(1) < model.drawable_render_order(0));
    }

    #[test]
    fn inconsistent_models_are_refused() {
        let mut b = builder();
        b.meshes[0].forms.forms[0].positions.pop();
        assert!(b.build(VERSION_42).is_err());
        let mut b = builder();
        b.deformers.swap(0, 1);
        assert!(b.build(VERSION_42).is_err());
        let mut b = builder();
        b.meshes[1].forms.axes[0].keys = vec![0.0];
        b.meshes[1].forms.forms.truncate(1);
        assert!(b.build(VERSION_42).is_err());
    }

    /// Everything the builder can express at once: a grouped part, keyed
    /// colours and opacity, a mask (inverted), glue and blend modes.
    fn everything() -> MocBuilder {
        let mut b = builder();
        b.parts.push(PartDef {
            id: "Group".into(),
            parent: Some(0),
            group: true,
            draw_order: Grid {
                axes: vec![Axis {
                    parameter: 0,
                    keys: vec![-1.0, 1.0],
                }],
                forms: vec![450.0, 650.0],
            },
        });
        b.meshes[0].parent_part = Some(1);
        b.meshes[1].forms.forms[1].tint = Tint {
            multiply: [0.2, 0.4, 0.6],
            screen: [0.1, 0.0, 0.3],
        };
        b.meshes[1].forms.forms[0].opacity = 0.25;
        let mut masked = b.meshes[1].clone();
        masked.id = "Masked".into();
        masked.masks = vec![0];
        masked.inverted_mask = true;
        masked.blend = MeshBlend::Multiply;
        masked.double_sided = false;
        b.meshes.push(masked);
        b.glues.push(GlueDef {
            id: "Glue".into(),
            mesh_a: 1,
            mesh_b: 2,
            pairs: vec![(0, 1, 0.5, 0.5), (2, 3, 1.0, 0.0)],
            intensity: Grid {
                axes: vec![Axis {
                    parameter: 1,
                    keys: vec![-30.0, 30.0],
                }],
                forms: vec![0.0, 1.0],
            },
        });
        b.draw_list = vec![
            DrawRef::Mesh(1),
            DrawRef::Part(1),
            DrawRef::Mesh(2),
            DrawRef::Mesh(0),
        ];
        with_nested_rotation(&mut b);
        b
    }

    /// Adds "Nested", a rotation 10 px right of "Rotation"'s pivot turning
    /// 0° to 90° with ParamX, holding mesh "Spun" (index returned).
    fn with_nested_rotation(b: &mut MocBuilder) -> usize {
        let turn = |angle: f32| RotationForm {
            origin: [10.0, 0.0],
            angle,
            scale: 1.0,
            reflect_x: false,
            reflect_y: false,
            opacity: 1.0,
            tint: Tint::default(),
        };
        b.deformers.push(DeformerDef {
            id: "Nested".into(),
            parent_part: Some(0),
            parent_deformer: Some(1),
            kind: DeformerKind::Rotation {
                base_angle: 0.0,
                forms: Grid {
                    axes: vec![Axis {
                        parameter: 0,
                        keys: vec![-1.0, 1.0],
                    }],
                    forms: vec![turn(0.0), turn(90.0)],
                },
            },
        });
        let nested = b.deformers.len() - 1;
        b.meshes.push(quad_mesh(
            "Spun",
            Some(nested),
            Grid::constant(form(&[10.0, 0.0, 20.0, 0.0, 20.0, 10.0, 10.0, 10.0], 800.0)),
        ));
        b.meshes.len() - 1
    }

    #[test]
    fn nested_rotations_add_their_angles() {
        let mut b = builder();
        let spun = with_nested_rotation(&mut b);
        let mut model = Model::new(Arc::new(b.build(VERSION_42).expect("builds")));
        // The outer pivot (canvas 50, 50) turned 90° clockwise puts the inner
        // pivot 10 px below it; the inner one turns 45° more, so a point
        // 10 px along it points 135° from +x.
        model.parameter_values[1] = 30.0;
        model.parameter_values[0] = 0.0;
        model.update();
        let p = model.drawable_positions_px(spun)[0];
        let (s, c) = (135f32.to_radians().sin(), 135f32.to_radians().cos());
        assert!(close(p, [50.0 + 10.0 * c, 60.0 + 10.0 * s]), "{p:?}");
    }

    #[test]
    fn built_models_load_and_deform_in_cubism_core() {
        for version in [VERSION_33, VERSION_40, VERSION_42, VERSION_50, VERSION_53] {
            let moc = everything().build(version).expect("builds");
            let bytes = moc.write();
            let dump = match crate::oracle::dump_with_core(&bytes, 16) {
                Ok(Some(d)) => d,
                Ok(None) => {
                    eprintln!("no Cubism Core (crates/aether-live2d/oracle/run.sh); skipping");
                    return;
                }
                Err(e) => panic!("version {version}: {e}"),
            };
            assert!(
                dump.consistent(),
                "version {version}: Core's consistency check fails"
            );
            let result = crate::oracle::compare(moc, &dump, 1e-3);
            assert!(
                result.failures.is_empty(),
                "version {version}:\n{}",
                result.failures.join("\n")
            );
        }
    }

    #[test]
    fn colours_are_keyed_per_keyform() {
        let mut b = builder();
        b.meshes[1].forms.forms[1].tint.multiply = [0.0, 0.5, 1.0];
        let moc = b.build(VERSION_42).expect("builds");
        let mut model = Model::new(Arc::new(moc));
        model.parameter_values[0] = 0.0;
        model.update();
        let mul = model.drawable_multiply(1);
        assert!(
            (mul[0] - 0.5).abs() < 1e-5 && (mul[1] - 0.75).abs() < 1e-5,
            "{mul:?}"
        );
    }
}
