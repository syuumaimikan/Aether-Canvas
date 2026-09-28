//! The MOC3 file format.
//!
//! A `.moc3` file is a 64-byte header (`MOC3`, a version byte, an endianness
//! flag), a table of section offsets (160 entries, or 480 from version 6),
//! space Cubism Core reuses for its own pointers, and then the sections: flat
//! arrays of 32-bit integers and floats, 16-bit indices, flag bytes and
//! 64-byte identifiers, in a fixed order that each version extends. The
//! counts of every array are in the first section.
//!
//! [`Moc`] holds those arrays by name. [`Moc::read`] parses and validates a
//! file (every index is checked, so a malformed file is an error, never a
//! panic later), and [`Moc::write`] lays the arrays out again the way Cubism
//! Editor does, so an unmodified model round-trips byte for byte.

use thiserror::Error;

/// Cubism 3.0 to 3.2.
pub const VERSION_30: u8 = 1;
/// Cubism 3.3: quad-interpolated warp deformers.
pub const VERSION_33: u8 = 2;
/// Cubism 4.0 and 4.1: the same layout as 3.3.
pub const VERSION_40: u8 = 3;
/// Cubism 4.2: multiply and screen colours, blend shapes, parameter types.
pub const VERSION_42: u8 = 4;
/// Cubism 5.0 to 5.2: blend shapes on everything, keyed colours.
pub const VERSION_50: u8 = 5;
/// Cubism 5.3: offscreen part rendering and more blend modes.
pub const VERSION_53: u8 = 6;
/// The newest version read and written.
pub const LATEST_VERSION: u8 = VERSION_53;

/// Canvas flag: y already points down (otherwise Core flips it).
pub const CANVAS_Y_REVERSED: u8 = 1;
/// Art mesh flag: additive blending (before 5.3).
pub const FLAG_ADDITIVE: u8 = 1 << 0;
/// Art mesh flag: multiplicative blending (before 5.3).
pub const FLAG_MULTIPLICATIVE: u8 = 1 << 1;
/// Art mesh flag: both faces drawn (otherwise back faces are culled).
pub const FLAG_DOUBLE_SIDED: u8 = 1 << 2;
/// Art mesh flag: drawn only outside its masks.
pub const FLAG_INVERTED_MASK: u8 = 1 << 3;

/// Deformer type: warp.
pub const DEFORMER_WARP: i32 = 0;
/// Deformer type: rotation.
pub const DEFORMER_ROTATION: i32 = 1;

/// Draw-order object type: an art mesh.
pub const DRAW_OBJECT_ART_MESH: i32 = 0;
/// Draw-order object type: a part (with its own group).
pub const DRAW_OBJECT_PART: i32 = 1;

/// Parameter type: keyform parameter.
pub const PARAMETER_NORMAL: i32 = 0;
/// Parameter type: blend-shape parameter (additive).
pub const PARAMETER_BLEND_SHAPE: i32 = 1;

const MAGIC: &[u8; 4] = b"MOC3";
const HEADER_SIZE: usize = 64;
const ID_SIZE: usize = 64;
/// Bytes of pointer space per entry in a runtime section.
const POINTER_SIZE: usize = 8;
const SECTION_ALIGN: usize = 64;
/// Longest key-table product Core accepts per binding (2^N blend weights).
const MAX_KEY_TABLES: usize = 16;

/// Why a file could not be read.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum MocError {
    /// Not a MOC3 file.
    #[error("not a MOC3 file")]
    NotMoc3,
    /// A version this reader does not know.
    #[error("unsupported MOC3 version {0} (this reader knows 1 to {LATEST_VERSION})")]
    UnsupportedVersion(u8),
    /// The file is cut short or a section lies outside it.
    #[error("the MOC3 file is truncated or malformed: {0}")]
    Truncated(String),
    /// An index or range inside the model points outside its array.
    #[error("the MOC3 model is inconsistent: {0}")]
    Invalid(String),
}

type Result<T> = std::result::Result<T, MocError>;

fn invalid(message: impl Into<String>) -> MocError {
    MocError::Invalid(message.into())
}

/// The canvas the model was drawn on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Canvas {
    /// Canvas pixels per model unit.
    pub pixels_per_unit: f32,
    /// Where the model origin sits on the canvas, in pixels.
    pub origin_x: f32,
    /// Where the model origin sits on the canvas, in pixels.
    pub origin_y: f32,
    /// Canvas width in pixels.
    pub width: f32,
    /// Canvas height in pixels.
    pub height: f32,
    /// [`CANVAS_Y_REVERSED`] and reserved bits.
    pub flags: u8,
}

/// Parts: the folders of a model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parts {
    pub ids: Vec<String>,
    pub binding: Vec<i32>,
    pub keyform_off: Vec<i32>,
    pub key_len: Vec<i32>,
    pub visible: Vec<i32>,
    pub enable: Vec<i32>,
    pub parent_part: Vec<i32>,
    /// Version 6: offscreen surface owned by the part, or -1.
    pub offscreen: Vec<i32>,
}

/// Deformers of both kinds, in parent-before-child order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deformers {
    pub ids: Vec<String>,
    pub binding: Vec<i32>,
    pub visible: Vec<i32>,
    pub enable: Vec<i32>,
    pub parent_part: Vec<i32>,
    pub parent_deformer: Vec<i32>,
    /// [`DEFORMER_WARP`] or [`DEFORMER_ROTATION`].
    pub kind: Vec<i32>,
    /// Index into [`Moc::warps`] or [`Moc::rotations`].
    pub local_index: Vec<i32>,
}

/// Warp deformers' own data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Warps {
    pub binding: Vec<i32>,
    pub keyform_off: Vec<i32>,
    pub key_len: Vec<i32>,
    pub vertex_count: Vec<i32>,
    pub rows: Vec<i32>,
    pub cols: Vec<i32>,
    /// Version 2+: bilinear instead of two-triangle interpolation per cell.
    pub quad: Vec<i32>,
    /// Version 4+: first keyform colour (see [`Moc::warp_keyforms`]).
    pub color_off: Vec<i32>,
}

/// Rotation deformers' own data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rotations {
    pub binding: Vec<i32>,
    pub keyform_off: Vec<i32>,
    pub key_len: Vec<i32>,
    pub base_angle: Vec<f32>,
    /// Version 4+.
    pub color_off: Vec<i32>,
}

/// Art meshes: the drawables.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtMeshes {
    pub ids: Vec<String>,
    pub binding: Vec<i32>,
    pub keyform_off: Vec<i32>,
    pub key_len: Vec<i32>,
    pub visible: Vec<i32>,
    pub enable: Vec<i32>,
    pub parent_part: Vec<i32>,
    pub parent_deformer: Vec<i32>,
    pub texture: Vec<i32>,
    /// `FLAG_*` bits.
    pub flags: Vec<u8>,
    pub vertex_count: Vec<i32>,
    /// Into [`Moc::uvs`], in floats.
    pub uv_off: Vec<i32>,
    /// Into [`Moc::indices`].
    pub index_off: Vec<i32>,
    pub index_len: Vec<i32>,
    /// Into [`Moc::masks`].
    pub mask_off: Vec<i32>,
    pub mask_len: Vec<i32>,
    /// Version 4+.
    pub color_off: Vec<i32>,
    /// Version 6: colour and alpha blend mode.
    pub blend_mode: Vec<i32>,
}

/// Parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parameters {
    pub ids: Vec<String>,
    pub max: Vec<f32>,
    pub min: Vec<f32>,
    pub default: Vec<f32>,
    pub repeat: Vec<i32>,
    pub decimal_places: Vec<i32>,
    /// Into [`Moc::key_tables`].
    pub key_table_off: Vec<i32>,
    pub key_table_len: Vec<i32>,
    /// Version 4+: the keys the editor shows (into [`Moc::keys`]).
    pub keys_off: Vec<i32>,
    pub keys_len: Vec<i32>,
    /// Version 4+: [`PARAMETER_NORMAL`] or [`PARAMETER_BLEND_SHAPE`].
    pub kind: Vec<i32>,
    /// Version 4+: into [`Moc::blend_key_tables`].
    pub blend_key_table_off: Vec<i32>,
    pub blend_key_table_len: Vec<i32>,
}

/// Part keyforms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartKeyforms {
    pub draw_order: Vec<f32>,
    /// Version 6: offscreen keyform index.
    pub offscreen_keyform: Vec<i32>,
}

/// Warp keyforms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WarpKeyforms {
    pub opacity: Vec<f32>,
    /// Into [`Moc::keyform_positions`], in floats.
    pub position_off: Vec<i32>,
    /// Version 5+: into the colour arrays, or -1.
    pub multiply_off: Vec<i32>,
    pub screen_off: Vec<i32>,
}

/// Rotation keyforms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RotationKeyforms {
    pub opacity: Vec<f32>,
    pub angle: Vec<f32>,
    pub origin_x: Vec<f32>,
    pub origin_y: Vec<f32>,
    pub scale: Vec<f32>,
    pub reflect_x: Vec<i32>,
    pub reflect_y: Vec<i32>,
    pub multiply_off: Vec<i32>,
    pub screen_off: Vec<i32>,
}

/// Art mesh keyforms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtMeshKeyforms {
    pub opacity: Vec<f32>,
    pub draw_order: Vec<f32>,
    pub position_off: Vec<i32>,
    pub multiply_off: Vec<i32>,
    pub screen_off: Vec<i32>,
}

/// Keyform bindings: which key tables span an object's keyform grid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bindings {
    /// Into [`Moc::key_table_indices`].
    pub key_table_off: Vec<i32>,
    pub key_table_len: Vec<i32>,
}

/// Key tables: the keys of one parameter used by one grid axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyTables {
    /// Into [`Moc::keys`].
    pub keys_off: Vec<i32>,
    pub keys_len: Vec<i32>,
}

/// Draw-order groups (one per part that orders its children, plus the root).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawGroups {
    pub object_off: Vec<i32>,
    pub object_len: Vec<i32>,
    pub total_count: Vec<i32>,
    pub max_order: Vec<i32>,
    pub min_order: Vec<i32>,
}

/// Draw-order group members.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawObjects {
    /// [`DRAW_OBJECT_ART_MESH`] or [`DRAW_OBJECT_PART`].
    pub kind: Vec<i32>,
    pub index: Vec<i32>,
    /// The part's own group, for parts.
    pub self_group: Vec<i32>,
}

/// Glue: vertex pairs of two meshes pulled together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glues {
    pub ids: Vec<String>,
    pub binding: Vec<i32>,
    pub keyform_off: Vec<i32>,
    pub key_len: Vec<i32>,
    pub art_mesh_a: Vec<i32>,
    pub art_mesh_b: Vec<i32>,
    /// Into [`Moc::glue_info`].
    pub info_off: Vec<i32>,
    pub info_len: Vec<i32>,
}

/// Glue vertex pairs: weights and vertex indices, alternating a/b.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlueInfo {
    pub weight: Vec<f32>,
    pub vertex: Vec<u16>,
}

/// Keyed colours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Colors {
    pub r: Vec<f32>,
    pub g: Vec<f32>,
    pub b: Vec<f32>,
}

/// Blend-shape key tables.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendKeyTables {
    pub keys_off: Vec<i32>,
    pub keys_len: Vec<i32>,
    pub base_key: Vec<i32>,
}

/// Blend-shape bindings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendBindings {
    pub key_table: Vec<i32>,
    /// First keyform of the shape (into the target type's keyforms).
    pub keyform_off: Vec<i32>,
    pub keyform_len: Vec<i32>,
    /// Into [`Moc::blend_constraint_indices`].
    pub constraint_off: Vec<i32>,
    pub constraint_len: Vec<i32>,
}

/// Blend shapes applied to one kind of object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendShapes {
    pub target: Vec<i32>,
    /// Into [`Moc::blend_bindings`].
    pub binding_off: Vec<i32>,
    pub binding_len: Vec<i32>,
}

/// Blend-shape constraints: weight curves over another parameter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendConstraints {
    pub parameter: Vec<i32>,
    /// Into [`Moc::blend_constraint_values`].
    pub value_off: Vec<i32>,
    pub value_len: Vec<i32>,
}

/// Blend-shape constraint curve points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendConstraintValues {
    pub key: Vec<f32>,
    pub weight: Vec<f32>,
}

/// Version 6 offscreen surfaces (parts rendered separately, then blended).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Offscreens {
    pub owner: Vec<i32>,
    pub flags: Vec<u8>,
    pub blend_mode: Vec<i32>,
    pub mask_off: Vec<i32>,
    pub mask_len: Vec<i32>,
}

/// Version 6 offscreen keyforms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OffscreenKeyforms {
    pub opacity: Vec<f32>,
    pub multiply_off: Vec<i32>,
    pub screen_off: Vec<i32>,
}

/// A whole `.moc3` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Moc {
    /// `VERSION_*`.
    pub version: u8,
    pub canvas: Canvas,
    pub parts: Parts,
    pub deformers: Deformers,
    pub warps: Warps,
    pub rotations: Rotations,
    pub art_meshes: ArtMeshes,
    pub parameters: Parameters,
    pub part_keyforms: PartKeyforms,
    pub warp_keyforms: WarpKeyforms,
    pub rotation_keyforms: RotationKeyforms,
    pub art_mesh_keyforms: ArtMeshKeyforms,
    /// Keyform vertex positions, x and y interleaved.
    pub keyform_positions: Vec<f32>,
    /// Key tables of each binding, flattened.
    pub key_table_indices: Vec<i32>,
    pub bindings: Bindings,
    pub key_tables: KeyTables,
    /// Parameter key values, flattened.
    pub keys: Vec<f32>,
    /// Texture coordinates, u and v interleaved, v up unless the canvas is
    /// y-reversed.
    pub uvs: Vec<f32>,
    /// Triangle vertex indices.
    pub indices: Vec<u16>,
    /// Mask art mesh indices (-1 entries are padding).
    pub masks: Vec<i32>,
    pub draw_groups: DrawGroups,
    pub draw_objects: DrawObjects,
    pub glues: Glues,
    pub glue_info: GlueInfo,
    /// Glue keyform intensities.
    pub glue_intensity: Vec<f32>,
    pub multiply_colors: Colors,
    pub screen_colors: Colors,
    pub blend_key_tables: BlendKeyTables,
    pub blend_bindings: BlendBindings,
    pub blend_warps: BlendShapes,
    pub blend_art_meshes: BlendShapes,
    pub blend_parts: BlendShapes,
    pub blend_rotations: BlendShapes,
    pub blend_glues: BlendShapes,
    pub blend_offscreens: BlendShapes,
    pub blend_constraint_indices: Vec<i32>,
    pub blend_constraints: BlendConstraints,
    pub blend_constraint_values: BlendConstraintValues,
    pub offscreens: Offscreens,
    pub offscreen_keyforms: OffscreenKeyforms,
}

/// The element counts stored in the first section, in file order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Counts {
    parts: usize,
    deformers: usize,
    warps: usize,
    rotations: usize,
    art_meshes: usize,
    parameters: usize,
    part_keyforms: usize,
    warp_keyforms: usize,
    rotation_keyforms: usize,
    art_mesh_keyforms: usize,
    keyform_positions: usize,
    key_table_indices: usize,
    bindings: usize,
    key_tables: usize,
    keys: usize,
    uvs: usize,
    indices: usize,
    masks: usize,
    draw_groups: usize,
    draw_objects: usize,
    glues: usize,
    glue_info: usize,
    glue_keyforms: usize,
    multiply_colors: usize,
    screen_colors: usize,
    blend_key_tables: usize,
    blend_bindings: usize,
    blend_warps: usize,
    blend_art_meshes: usize,
    blend_constraint_indices: usize,
    blend_constraints: usize,
    blend_constraint_values: usize,
    blend_parts: usize,
    blend_rotations: usize,
    blend_glues: usize,
    offscreens: usize,
    offscreen_keyforms: usize,
    blend_offscreens: usize,
}

impl Counts {
    fn to_array(self) -> [usize; 38] {
        [
            self.parts,
            self.deformers,
            self.warps,
            self.rotations,
            self.art_meshes,
            self.parameters,
            self.part_keyforms,
            self.warp_keyforms,
            self.rotation_keyforms,
            self.art_mesh_keyforms,
            self.keyform_positions,
            self.key_table_indices,
            self.bindings,
            self.key_tables,
            self.keys,
            self.uvs,
            self.indices,
            self.masks,
            self.draw_groups,
            self.draw_objects,
            self.glues,
            self.glue_info,
            self.glue_keyforms,
            self.multiply_colors,
            self.screen_colors,
            self.blend_key_tables,
            self.blend_bindings,
            self.blend_warps,
            self.blend_art_meshes,
            self.blend_constraint_indices,
            self.blend_constraints,
            self.blend_constraint_values,
            self.blend_parts,
            self.blend_rotations,
            self.blend_glues,
            self.offscreens,
            self.offscreen_keyforms,
            self.blend_offscreens,
        ]
    }

    fn from_array(a: &[usize]) -> Self {
        let g = |i: usize| a.get(i).copied().unwrap_or(0);
        Self {
            parts: g(0),
            deformers: g(1),
            warps: g(2),
            rotations: g(3),
            art_meshes: g(4),
            parameters: g(5),
            part_keyforms: g(6),
            warp_keyforms: g(7),
            rotation_keyforms: g(8),
            art_mesh_keyforms: g(9),
            keyform_positions: g(10),
            key_table_indices: g(11),
            bindings: g(12),
            key_tables: g(13),
            keys: g(14),
            uvs: g(15),
            indices: g(16),
            masks: g(17),
            draw_groups: g(18),
            draw_objects: g(19),
            glues: g(20),
            glue_info: g(21),
            glue_keyforms: g(22),
            multiply_colors: g(23),
            screen_colors: g(24),
            blend_key_tables: g(25),
            blend_bindings: g(26),
            blend_warps: g(27),
            blend_art_meshes: g(28),
            blend_constraint_indices: g(29),
            blend_constraints: g(30),
            blend_constraint_values: g(31),
            blend_parts: g(32),
            blend_rotations: g(33),
            blend_glues: g(34),
            offscreens: g(35),
            offscreen_keyforms: g(36),
            blend_offscreens: g(37),
        }
    }
}

fn count_ints(version: u8) -> usize {
    if version >= VERSION_50 {
        64
    } else {
        32
    }
}

fn offset_entries(version: u8) -> usize {
    if version >= VERSION_53 {
        480
    } else {
        160
    }
}

/// Where the sections start: after the header, the offset table and the
/// space Cubism Core overlays with its section pointers.
fn data_start(version: u8) -> usize {
    if version >= VERSION_53 {
        5824
    } else {
        1984
    }
}

/// Walks every dynamic section in file order. Reading and writing share it,
/// so the two can never disagree about the layout.
trait Visitor {
    fn runtime(&mut self, count: usize) -> Result<()>;
    fn ids(&mut self, v: &mut Vec<String>, count: usize) -> Result<()>;
    fn i32s(&mut self, v: &mut Vec<i32>, count: usize) -> Result<()>;
    fn f32s(&mut self, v: &mut Vec<f32>, count: usize) -> Result<()>;
    fn u16s(&mut self, v: &mut Vec<u16>, count: usize) -> Result<()>;
    fn u8s(&mut self, v: &mut Vec<u8>, count: usize) -> Result<()>;
}

fn visit(moc: &mut Moc, c: &Counts, v: &mut impl Visitor) -> Result<()> {
    let version = moc.version;
    let m = moc;

    // Version 1 (Cubism 3.0).
    v.runtime(c.parts)?;
    v.ids(&mut m.parts.ids, c.parts)?;
    v.i32s(&mut m.parts.binding, c.parts)?;
    v.i32s(&mut m.parts.keyform_off, c.parts)?;
    v.i32s(&mut m.parts.key_len, c.parts)?;
    v.i32s(&mut m.parts.visible, c.parts)?;
    v.i32s(&mut m.parts.enable, c.parts)?;
    v.i32s(&mut m.parts.parent_part, c.parts)?;

    v.runtime(c.deformers)?;
    v.ids(&mut m.deformers.ids, c.deformers)?;
    v.i32s(&mut m.deformers.binding, c.deformers)?;
    v.i32s(&mut m.deformers.visible, c.deformers)?;
    v.i32s(&mut m.deformers.enable, c.deformers)?;
    v.i32s(&mut m.deformers.parent_part, c.deformers)?;
    v.i32s(&mut m.deformers.parent_deformer, c.deformers)?;
    v.i32s(&mut m.deformers.kind, c.deformers)?;
    v.i32s(&mut m.deformers.local_index, c.deformers)?;

    v.i32s(&mut m.warps.binding, c.warps)?;
    v.i32s(&mut m.warps.keyform_off, c.warps)?;
    v.i32s(&mut m.warps.key_len, c.warps)?;
    v.i32s(&mut m.warps.vertex_count, c.warps)?;
    v.i32s(&mut m.warps.rows, c.warps)?;
    v.i32s(&mut m.warps.cols, c.warps)?;

    v.i32s(&mut m.rotations.binding, c.rotations)?;
    v.i32s(&mut m.rotations.keyform_off, c.rotations)?;
    v.i32s(&mut m.rotations.key_len, c.rotations)?;
    v.f32s(&mut m.rotations.base_angle, c.rotations)?;

    v.runtime(c.art_meshes)?;
    v.runtime(c.art_meshes)?;
    v.runtime(c.art_meshes)?;
    v.runtime(c.art_meshes)?;
    v.ids(&mut m.art_meshes.ids, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.binding, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.keyform_off, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.key_len, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.visible, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.enable, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.parent_part, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.parent_deformer, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.texture, c.art_meshes)?;
    v.u8s(&mut m.art_meshes.flags, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.vertex_count, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.uv_off, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.index_off, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.index_len, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.mask_off, c.art_meshes)?;
    v.i32s(&mut m.art_meshes.mask_len, c.art_meshes)?;

    v.runtime(c.parameters)?;
    v.ids(&mut m.parameters.ids, c.parameters)?;
    v.f32s(&mut m.parameters.max, c.parameters)?;
    v.f32s(&mut m.parameters.min, c.parameters)?;
    v.f32s(&mut m.parameters.default, c.parameters)?;
    v.i32s(&mut m.parameters.repeat, c.parameters)?;
    v.i32s(&mut m.parameters.decimal_places, c.parameters)?;
    v.i32s(&mut m.parameters.key_table_off, c.parameters)?;
    v.i32s(&mut m.parameters.key_table_len, c.parameters)?;

    v.f32s(&mut m.part_keyforms.draw_order, c.part_keyforms)?;
    v.f32s(&mut m.warp_keyforms.opacity, c.warp_keyforms)?;
    v.i32s(&mut m.warp_keyforms.position_off, c.warp_keyforms)?;
    v.f32s(&mut m.rotation_keyforms.opacity, c.rotation_keyforms)?;
    v.f32s(&mut m.rotation_keyforms.angle, c.rotation_keyforms)?;
    v.f32s(&mut m.rotation_keyforms.origin_x, c.rotation_keyforms)?;
    v.f32s(&mut m.rotation_keyforms.origin_y, c.rotation_keyforms)?;
    v.f32s(&mut m.rotation_keyforms.scale, c.rotation_keyforms)?;
    v.i32s(&mut m.rotation_keyforms.reflect_x, c.rotation_keyforms)?;
    v.i32s(&mut m.rotation_keyforms.reflect_y, c.rotation_keyforms)?;
    v.f32s(&mut m.art_mesh_keyforms.opacity, c.art_mesh_keyforms)?;
    v.f32s(&mut m.art_mesh_keyforms.draw_order, c.art_mesh_keyforms)?;
    v.i32s(&mut m.art_mesh_keyforms.position_off, c.art_mesh_keyforms)?;
    v.f32s(&mut m.keyform_positions, c.keyform_positions)?;
    v.i32s(&mut m.key_table_indices, c.key_table_indices)?;
    v.i32s(&mut m.bindings.key_table_off, c.bindings)?;
    v.i32s(&mut m.bindings.key_table_len, c.bindings)?;
    v.i32s(&mut m.key_tables.keys_off, c.key_tables)?;
    v.i32s(&mut m.key_tables.keys_len, c.key_tables)?;
    v.f32s(&mut m.keys, c.keys)?;
    v.f32s(&mut m.uvs, c.uvs)?;
    v.u16s(&mut m.indices, c.indices)?;
    v.i32s(&mut m.masks, c.masks)?;
    v.i32s(&mut m.draw_groups.object_off, c.draw_groups)?;
    v.i32s(&mut m.draw_groups.object_len, c.draw_groups)?;
    v.i32s(&mut m.draw_groups.total_count, c.draw_groups)?;
    v.i32s(&mut m.draw_groups.max_order, c.draw_groups)?;
    v.i32s(&mut m.draw_groups.min_order, c.draw_groups)?;
    v.i32s(&mut m.draw_objects.kind, c.draw_objects)?;
    v.i32s(&mut m.draw_objects.index, c.draw_objects)?;
    v.i32s(&mut m.draw_objects.self_group, c.draw_objects)?;
    v.runtime(c.glues)?;
    v.ids(&mut m.glues.ids, c.glues)?;
    v.i32s(&mut m.glues.binding, c.glues)?;
    v.i32s(&mut m.glues.keyform_off, c.glues)?;
    v.i32s(&mut m.glues.key_len, c.glues)?;
    v.i32s(&mut m.glues.art_mesh_a, c.glues)?;
    v.i32s(&mut m.glues.art_mesh_b, c.glues)?;
    v.i32s(&mut m.glues.info_off, c.glues)?;
    v.i32s(&mut m.glues.info_len, c.glues)?;
    v.f32s(&mut m.glue_info.weight, c.glue_info)?;
    v.u16s(&mut m.glue_info.vertex, c.glue_info)?;
    v.f32s(&mut m.glue_intensity, c.glue_keyforms)?;
    if version < VERSION_33 {
        return Ok(());
    }

    // Version 2 (Cubism 3.3).
    v.i32s(&mut m.warps.quad, c.warps)?;
    if version < VERSION_42 {
        return Ok(());
    }

    // Version 4 (Cubism 4.2).
    v.runtime(c.parameters)?;
    v.i32s(&mut m.parameters.keys_off, c.parameters)?;
    v.i32s(&mut m.parameters.keys_len, c.parameters)?;
    v.i32s(&mut m.warps.color_off, c.warps)?;
    v.i32s(&mut m.rotations.color_off, c.rotations)?;
    v.i32s(&mut m.art_meshes.color_off, c.art_meshes)?;
    v.f32s(&mut m.multiply_colors.r, c.multiply_colors)?;
    v.f32s(&mut m.multiply_colors.g, c.multiply_colors)?;
    v.f32s(&mut m.multiply_colors.b, c.multiply_colors)?;
    v.f32s(&mut m.screen_colors.r, c.screen_colors)?;
    v.f32s(&mut m.screen_colors.g, c.screen_colors)?;
    v.f32s(&mut m.screen_colors.b, c.screen_colors)?;
    v.i32s(&mut m.parameters.kind, c.parameters)?;
    v.i32s(&mut m.parameters.blend_key_table_off, c.parameters)?;
    v.i32s(&mut m.parameters.blend_key_table_len, c.parameters)?;
    v.i32s(&mut m.blend_key_tables.keys_off, c.blend_key_tables)?;
    v.i32s(&mut m.blend_key_tables.keys_len, c.blend_key_tables)?;
    v.i32s(&mut m.blend_key_tables.base_key, c.blend_key_tables)?;
    v.i32s(&mut m.blend_bindings.key_table, c.blend_bindings)?;
    v.i32s(&mut m.blend_bindings.keyform_off, c.blend_bindings)?;
    v.i32s(&mut m.blend_bindings.keyform_len, c.blend_bindings)?;
    v.i32s(&mut m.blend_bindings.constraint_off, c.blend_bindings)?;
    v.i32s(&mut m.blend_bindings.constraint_len, c.blend_bindings)?;
    v.i32s(&mut m.blend_warps.target, c.blend_warps)?;
    v.i32s(&mut m.blend_warps.binding_off, c.blend_warps)?;
    v.i32s(&mut m.blend_warps.binding_len, c.blend_warps)?;
    v.i32s(&mut m.blend_art_meshes.target, c.blend_art_meshes)?;
    v.i32s(&mut m.blend_art_meshes.binding_off, c.blend_art_meshes)?;
    v.i32s(&mut m.blend_art_meshes.binding_len, c.blend_art_meshes)?;
    v.i32s(&mut m.blend_constraint_indices, c.blend_constraint_indices)?;
    v.i32s(&mut m.blend_constraints.parameter, c.blend_constraints)?;
    v.i32s(&mut m.blend_constraints.value_off, c.blend_constraints)?;
    v.i32s(&mut m.blend_constraints.value_len, c.blend_constraints)?;
    v.f32s(&mut m.blend_constraint_values.key, c.blend_constraint_values)?;
    v.f32s(&mut m.blend_constraint_values.weight, c.blend_constraint_values)?;
    if version < VERSION_50 {
        return Ok(());
    }

    // Version 5 (Cubism 5.0).
    v.i32s(&mut m.warp_keyforms.multiply_off, c.warp_keyforms)?;
    v.i32s(&mut m.warp_keyforms.screen_off, c.warp_keyforms)?;
    v.i32s(&mut m.rotation_keyforms.multiply_off, c.rotation_keyforms)?;
    v.i32s(&mut m.rotation_keyforms.screen_off, c.rotation_keyforms)?;
    v.i32s(&mut m.art_mesh_keyforms.multiply_off, c.art_mesh_keyforms)?;
    v.i32s(&mut m.art_mesh_keyforms.screen_off, c.art_mesh_keyforms)?;
    v.i32s(&mut m.blend_parts.target, c.blend_parts)?;
    v.i32s(&mut m.blend_parts.binding_off, c.blend_parts)?;
    v.i32s(&mut m.blend_parts.binding_len, c.blend_parts)?;
    v.i32s(&mut m.blend_rotations.target, c.blend_rotations)?;
    v.i32s(&mut m.blend_rotations.binding_off, c.blend_rotations)?;
    v.i32s(&mut m.blend_rotations.binding_len, c.blend_rotations)?;
    v.i32s(&mut m.blend_glues.target, c.blend_glues)?;
    v.i32s(&mut m.blend_glues.binding_off, c.blend_glues)?;
    v.i32s(&mut m.blend_glues.binding_len, c.blend_glues)?;
    if version < VERSION_53 {
        return Ok(());
    }

    // Version 6 (Cubism 5.3).
    v.i32s(&mut m.parts.offscreen, c.parts)?;
    v.i32s(&mut m.art_meshes.blend_mode, c.art_meshes)?;
    v.runtime(c.offscreens)?;
    v.i32s(&mut m.offscreens.owner, c.offscreens)?;
    v.u8s(&mut m.offscreens.flags, c.offscreens)?;
    v.i32s(&mut m.offscreens.blend_mode, c.offscreens)?;
    v.i32s(&mut m.offscreens.mask_off, c.offscreens)?;
    v.i32s(&mut m.offscreens.mask_len, c.offscreens)?;
    v.i32s(&mut m.part_keyforms.offscreen_keyform, c.part_keyforms)?;
    v.f32s(&mut m.offscreen_keyforms.opacity, c.offscreen_keyforms)?;
    v.i32s(&mut m.offscreen_keyforms.multiply_off, c.offscreen_keyforms)?;
    v.i32s(&mut m.offscreen_keyforms.screen_off, c.offscreen_keyforms)?;
    v.i32s(&mut m.blend_offscreens.target, c.blend_offscreens)?;
    v.i32s(&mut m.blend_offscreens.binding_off, c.blend_offscreens)?;
    v.i32s(&mut m.blend_offscreens.binding_len, c.blend_offscreens)?;
    Ok(())
}

// ------------------------------------------------------------------ reading

struct Reader<'a> {
    data: &'a [u8],
    offsets: Vec<usize>,
    next: usize,
    big_endian: bool,
}

impl Reader<'_> {
    /// The next section's bytes.
    fn section(&mut self, count: usize, size: usize) -> Result<&[u8]> {
        let index = self.next;
        self.next += 1;
        let offset = *self
            .offsets
            .get(index)
            .ok_or_else(|| MocError::Truncated(format!("no offset for section {index}")))?;
        let len = count
            .checked_mul(size)
            .ok_or_else(|| MocError::Truncated(format!("section {index} is too large")))?;
        let end = offset
            .checked_add(len)
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| MocError::Truncated(format!("section {index} lies outside the file")))?;
        Ok(&self.data[offset..end])
    }

    fn word(&self, b: &[u8]) -> [u8; 4] {
        let w = [b[0], b[1], b[2], b[3]];
        if self.big_endian {
            [w[3], w[2], w[1], w[0]]
        } else {
            w
        }
    }
}

impl Visitor for Reader<'_> {
    fn runtime(&mut self, count: usize) -> Result<()> {
        self.section(count, POINTER_SIZE).map(|_| ())
    }

    fn ids(&mut self, v: &mut Vec<String>, count: usize) -> Result<()> {
        let bytes = self.section(count, ID_SIZE)?;
        *v = bytes
            .chunks_exact(ID_SIZE)
            .map(|id| {
                let len = id.iter().position(|&b| b == 0).unwrap_or(ID_SIZE);
                String::from_utf8_lossy(&id[..len]).into_owned()
            })
            .collect();
        Ok(())
    }

    fn i32s(&mut self, v: &mut Vec<i32>, count: usize) -> Result<()> {
        let big = self.big_endian;
        let bytes = self.section(count, 4)?;
        *v = bytes
            .chunks_exact(4)
            .map(|b| {
                let w = [b[0], b[1], b[2], b[3]];
                if big {
                    i32::from_be_bytes(w)
                } else {
                    i32::from_le_bytes(w)
                }
            })
            .collect();
        Ok(())
    }

    fn f32s(&mut self, v: &mut Vec<f32>, count: usize) -> Result<()> {
        let big = self.big_endian;
        let bytes = self.section(count, 4)?;
        *v = bytes
            .chunks_exact(4)
            .map(|b| {
                let w = [b[0], b[1], b[2], b[3]];
                if big {
                    f32::from_be_bytes(w)
                } else {
                    f32::from_le_bytes(w)
                }
            })
            .collect();
        Ok(())
    }

    fn u16s(&mut self, v: &mut Vec<u16>, count: usize) -> Result<()> {
        let big = self.big_endian;
        let bytes = self.section(count, 2)?;
        *v = bytes
            .chunks_exact(2)
            .map(|b| {
                if big {
                    u16::from_be_bytes([b[0], b[1]])
                } else {
                    u16::from_le_bytes([b[0], b[1]])
                }
            })
            .collect();
        Ok(())
    }

    fn u8s(&mut self, v: &mut Vec<u8>, count: usize) -> Result<()> {
        *v = self.section(count, 1)?.to_vec();
        Ok(())
    }
}

// ------------------------------------------------------------------ writing

struct Writer {
    out: Vec<u8>,
    offsets: Vec<u32>,
    /// The previous section held runtime pointers (identifiers follow those
    /// without padding, as Cubism Editor writes them).
    after_runtime: bool,
}

impl Writer {
    fn begin(&mut self, packed: bool) {
        if !packed {
            let aligned = self.out.len().div_ceil(SECTION_ALIGN) * SECTION_ALIGN;
            self.out.resize(aligned, 0);
        }
        self.offsets.push(self.out.len() as u32);
    }
}

impl Visitor for Writer {
    fn runtime(&mut self, count: usize) -> Result<()> {
        self.begin(false);
        self.out.resize(self.out.len() + count * POINTER_SIZE, 0);
        self.after_runtime = true;
        Ok(())
    }

    fn ids(&mut self, v: &mut Vec<String>, count: usize) -> Result<()> {
        self.begin(self.after_runtime);
        self.after_runtime = false;
        for i in 0..count {
            let mut id = [0u8; ID_SIZE];
            if let Some(s) = v.get(i) {
                let bytes = s.as_bytes();
                let n = bytes.len().min(ID_SIZE - 1);
                id[..n].copy_from_slice(&bytes[..n]);
            }
            self.out.extend_from_slice(&id);
        }
        Ok(())
    }

    fn i32s(&mut self, v: &mut Vec<i32>, count: usize) -> Result<()> {
        self.begin(false);
        self.after_runtime = false;
        for i in 0..count {
            self.out
                .extend_from_slice(&v.get(i).copied().unwrap_or(0).to_le_bytes());
        }
        Ok(())
    }

    fn f32s(&mut self, v: &mut Vec<f32>, count: usize) -> Result<()> {
        self.begin(false);
        self.after_runtime = false;
        for i in 0..count {
            self.out
                .extend_from_slice(&v.get(i).copied().unwrap_or(0.0).to_le_bytes());
        }
        Ok(())
    }

    fn u16s(&mut self, v: &mut Vec<u16>, count: usize) -> Result<()> {
        self.begin(false);
        self.after_runtime = false;
        for i in 0..count {
            self.out
                .extend_from_slice(&v.get(i).copied().unwrap_or(0).to_le_bytes());
        }
        Ok(())
    }

    fn u8s(&mut self, v: &mut Vec<u8>, count: usize) -> Result<()> {
        self.begin(false);
        self.after_runtime = false;
        for i in 0..count {
            self.out.push(v.get(i).copied().unwrap_or(0));
        }
        Ok(())
    }
}

impl Moc {
    /// Parse and validate a `.moc3` file.
    pub fn read(data: &[u8]) -> Result<Moc> {
        if data.len() < HEADER_SIZE || &data[..4] != MAGIC {
            return Err(MocError::NotMoc3);
        }
        let version = data[4];
        if version == 0 || version > LATEST_VERSION {
            return Err(MocError::UnsupportedVersion(version));
        }
        let big_endian = match data[5] {
            0 => false,
            1 => true,
            other => return Err(MocError::Truncated(format!("unknown endianness flag {other}"))),
        };
        let entries = offset_entries(version);
        let table_end = HEADER_SIZE + entries * 4;
        if data.len() < table_end {
            return Err(MocError::Truncated("the section table is cut short".into()));
        }
        let mut reader = Reader {
            data,
            offsets: Vec::with_capacity(entries),
            next: 0,
            big_endian,
        };
        for i in 0..entries {
            let at = HEADER_SIZE + i * 4;
            let word = reader.word(&data[at..at + 4]);
            reader.offsets.push(u32::from_le_bytes(word) as usize);
        }

        // Section 0: the counts.
        let ints = count_ints(version);
        let raw = reader.section(ints, 4)?.to_vec();
        let mut counts = Vec::with_capacity(ints);
        for chunk in raw.chunks_exact(4) {
            let n = i32::from_le_bytes(reader.word(chunk));
            if n < 0 {
                return Err(invalid(format!("negative count {n}")));
            }
            counts.push(n as usize);
        }
        let counts = Counts::from_array(&counts);
        // No count can exceed what the file could hold.
        if counts.to_array().iter().any(|&n| n > data.len()) {
            return Err(MocError::Truncated("a count exceeds the file size".into()));
        }

        // Section 1: the canvas.
        let canvas_bytes = reader.section(1, 21)?.to_vec();
        let float = |i: usize| f32::from_le_bytes(reader.word(&canvas_bytes[i * 4..i * 4 + 4]));
        let canvas = Canvas {
            pixels_per_unit: float(0),
            origin_x: float(1),
            origin_y: float(2),
            width: float(3),
            height: float(4),
            flags: canvas_bytes[20],
        };

        let mut moc = Moc {
            version,
            canvas,
            ..Default::default()
        };
        visit(&mut moc, &counts, &mut reader)?;
        moc.validate()?;
        Ok(moc)
    }

    /// The element counts implied by the arrays.
    fn counts(&self) -> Counts {
        Counts {
            parts: self.parts.ids.len(),
            deformers: self.deformers.ids.len(),
            warps: self.warps.binding.len(),
            rotations: self.rotations.binding.len(),
            art_meshes: self.art_meshes.ids.len(),
            parameters: self.parameters.ids.len(),
            part_keyforms: self.part_keyforms.draw_order.len(),
            warp_keyforms: self.warp_keyforms.opacity.len(),
            rotation_keyforms: self.rotation_keyforms.opacity.len(),
            art_mesh_keyforms: self.art_mesh_keyforms.opacity.len(),
            keyform_positions: self.keyform_positions.len(),
            key_table_indices: self.key_table_indices.len(),
            bindings: self.bindings.key_table_off.len(),
            key_tables: self.key_tables.keys_off.len(),
            keys: self.keys.len(),
            uvs: self.uvs.len(),
            indices: self.indices.len(),
            masks: self.masks.len(),
            draw_groups: self.draw_groups.object_off.len(),
            draw_objects: self.draw_objects.kind.len(),
            glues: self.glues.ids.len(),
            glue_info: self.glue_info.weight.len(),
            glue_keyforms: self.glue_intensity.len(),
            multiply_colors: self.multiply_colors.r.len(),
            screen_colors: self.screen_colors.r.len(),
            blend_key_tables: self.blend_key_tables.keys_off.len(),
            blend_bindings: self.blend_bindings.key_table.len(),
            blend_warps: self.blend_warps.target.len(),
            blend_art_meshes: self.blend_art_meshes.target.len(),
            blend_constraint_indices: self.blend_constraint_indices.len(),
            blend_constraints: self.blend_constraints.parameter.len(),
            blend_constraint_values: self.blend_constraint_values.key.len(),
            blend_parts: self.blend_parts.target.len(),
            blend_rotations: self.blend_rotations.target.len(),
            blend_glues: self.blend_glues.target.len(),
            offscreens: self.offscreens.owner.len(),
            offscreen_keyforms: self.offscreen_keyforms.opacity.len(),
            blend_offscreens: self.blend_offscreens.target.len(),
        }
    }

    /// Serialise as a little-endian `.moc3` file of [`Moc::version`].
    ///
    /// Arrays a version does not have are left out; counts come from the
    /// arrays themselves. Validate first ([`Moc::validate`]) when the model
    /// was built or edited by hand.
    pub fn write(&self) -> Vec<u8> {
        let version = self.version.clamp(VERSION_30, LATEST_VERSION);
        let counts = self.counts();
        let entries = offset_entries(version);
        let mut writer = Writer {
            out: vec![0; data_start(version)],
            offsets: Vec::with_capacity(entries),
            after_runtime: false,
        };
        writer.out[..4].copy_from_slice(MAGIC);
        writer.out[4] = version;

        // Counts and canvas.
        writer.begin(false);
        let mut ints = vec![0i32; count_ints(version)];
        for (slot, n) in ints.iter_mut().zip(counts.to_array()) {
            *slot = n as i32;
        }
        for n in ints {
            writer.out.extend_from_slice(&n.to_le_bytes());
        }
        writer.begin(false);
        for f in [
            self.canvas.pixels_per_unit,
            self.canvas.origin_x,
            self.canvas.origin_y,
            self.canvas.width,
            self.canvas.height,
        ] {
            writer.out.extend_from_slice(&f.to_le_bytes());
        }
        writer.out.extend_from_slice(&[self.canvas.flags, 0, 0, 0]);

        let mut copy = self.clone();
        copy.version = version;
        visit(&mut copy, &counts, &mut writer).expect("writing cannot fail");

        let aligned = writer.out.len().div_ceil(SECTION_ALIGN) * SECTION_ALIGN;
        writer.out.resize(aligned, 0);
        let mut out = writer.out;
        for (i, offset) in writer.offsets.iter().enumerate().take(entries) {
            let at = HEADER_SIZE + i * 4;
            out[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        }
        out
    }

    // --------------------------------------------------------- validation

    /// Check every array length, index and range, so evaluation can index
    /// freely. [`Moc::read`] calls this.
    pub fn validate(&self) -> Result<()> {
        let c = self.counts();
        let v = self.version;
        let same = |what: &str, lens: &[usize], n: usize| -> Result<()> {
            if lens.iter().any(|&l| l != n) {
                return Err(invalid(format!("{what} arrays differ in length")));
            }
            Ok(())
        };
        let p = &self.parts;
        same(
            "part",
            &[
                p.binding.len(),
                p.keyform_off.len(),
                p.key_len.len(),
                p.visible.len(),
                p.enable.len(),
                p.parent_part.len(),
            ],
            c.parts,
        )?;
        let d = &self.deformers;
        same(
            "deformer",
            &[
                d.binding.len(),
                d.visible.len(),
                d.enable.len(),
                d.parent_part.len(),
                d.parent_deformer.len(),
                d.kind.len(),
                d.local_index.len(),
            ],
            c.deformers,
        )?;
        let w = &self.warps;
        same(
            "warp",
            &[
                w.keyform_off.len(),
                w.key_len.len(),
                w.vertex_count.len(),
                w.rows.len(),
                w.cols.len(),
            ],
            c.warps,
        )?;
        let r = &self.rotations;
        same(
            "rotation",
            &[r.keyform_off.len(), r.key_len.len(), r.base_angle.len()],
            c.rotations,
        )?;
        let a = &self.art_meshes;
        same(
            "art mesh",
            &[
                a.binding.len(),
                a.keyform_off.len(),
                a.key_len.len(),
                a.visible.len(),
                a.enable.len(),
                a.parent_part.len(),
                a.parent_deformer.len(),
                a.texture.len(),
                a.flags.len(),
                a.vertex_count.len(),
                a.uv_off.len(),
                a.index_off.len(),
                a.index_len.len(),
                a.mask_off.len(),
                a.mask_len.len(),
            ],
            c.art_meshes,
        )?;
        let q = &self.parameters;
        same(
            "parameter",
            &[
                q.max.len(),
                q.min.len(),
                q.default.len(),
                q.repeat.len(),
                q.decimal_places.len(),
                q.key_table_off.len(),
                q.key_table_len.len(),
            ],
            c.parameters,
        )?;
        same(
            "part keyform",
            &[self.part_keyforms.draw_order.len()],
            c.part_keyforms,
        )?;
        same(
            "warp keyform",
            &[self.warp_keyforms.position_off.len()],
            c.warp_keyforms,
        )?;
        let rk = &self.rotation_keyforms;
        same(
            "rotation keyform",
            &[
                rk.angle.len(),
                rk.origin_x.len(),
                rk.origin_y.len(),
                rk.scale.len(),
                rk.reflect_x.len(),
                rk.reflect_y.len(),
            ],
            c.rotation_keyforms,
        )?;
        let ak = &self.art_mesh_keyforms;
        same(
            "art mesh keyform",
            &[ak.draw_order.len(), ak.position_off.len()],
            c.art_mesh_keyforms,
        )?;
        same("binding", &[self.bindings.key_table_len.len()], c.bindings)?;
        same("key table", &[self.key_tables.keys_len.len()], c.key_tables)?;
        let g = &self.draw_groups;
        same(
            "draw group",
            &[
                g.object_len.len(),
                g.total_count.len(),
                g.max_order.len(),
                g.min_order.len(),
            ],
            c.draw_groups,
        )?;
        let o = &self.draw_objects;
        same(
            "draw object",
            &[o.index.len(), o.self_group.len()],
            c.draw_objects,
        )?;
        let gl = &self.glues;
        same(
            "glue",
            &[
                gl.binding.len(),
                gl.keyform_off.len(),
                gl.key_len.len(),
                gl.art_mesh_a.len(),
                gl.art_mesh_b.len(),
                gl.info_off.len(),
                gl.info_len.len(),
            ],
            c.glues,
        )?;
        same("glue info", &[self.glue_info.vertex.len()], c.glue_info)?;
        if v >= VERSION_33 {
            same("warp quad", &[w.quad.len()], c.warps)?;
        }
        if v >= VERSION_42 {
            same(
                "parameter keys",
                &[q.keys_off.len(), q.keys_len.len(), q.kind.len()],
                c.parameters,
            )?;
            same(
                "parameter blend",
                &[q.blend_key_table_off.len(), q.blend_key_table_len.len()],
                c.parameters,
            )?;
            same("warp colour", &[w.color_off.len()], c.warps)?;
            same("rotation colour", &[r.color_off.len()], c.rotations)?;
            same("art mesh colour", &[a.color_off.len()], c.art_meshes)?;
            let mc = &self.multiply_colors;
            same("multiply colour", &[mc.g.len(), mc.b.len()], c.multiply_colors)?;
            let sc = &self.screen_colors;
            same("screen colour", &[sc.g.len(), sc.b.len()], c.screen_colors)?;
            let bk = &self.blend_key_tables;
            same(
                "blend key table",
                &[bk.keys_len.len(), bk.base_key.len()],
                c.blend_key_tables,
            )?;
            let bb = &self.blend_bindings;
            same(
                "blend binding",
                &[
                    bb.keyform_off.len(),
                    bb.keyform_len.len(),
                    bb.constraint_off.len(),
                    bb.constraint_len.len(),
                ],
                c.blend_bindings,
            )?;
            for (what, s) in [("warp", &self.blend_warps), ("art mesh", &self.blend_art_meshes)] {
                same(what, &[s.binding_off.len(), s.binding_len.len()], s.target.len())?;
            }
            let bc = &self.blend_constraints;
            same(
                "blend constraint",
                &[bc.value_off.len(), bc.value_len.len()],
                c.blend_constraints,
            )?;
            same(
                "blend constraint value",
                &[self.blend_constraint_values.weight.len()],
                c.blend_constraint_values,
            )?;
        }
        if v >= VERSION_50 {
            let wk = &self.warp_keyforms;
            same(
                "warp keyform colour",
                &[wk.multiply_off.len(), wk.screen_off.len()],
                c.warp_keyforms,
            )?;
            same(
                "rotation keyform colour",
                &[rk.multiply_off.len(), rk.screen_off.len()],
                c.rotation_keyforms,
            )?;
            same(
                "art mesh keyform colour",
                &[ak.multiply_off.len(), ak.screen_off.len()],
                c.art_mesh_keyforms,
            )?;
            for (what, s) in [
                ("part", &self.blend_parts),
                ("rotation", &self.blend_rotations),
                ("glue", &self.blend_glues),
            ] {
                same(what, &[s.binding_off.len(), s.binding_len.len()], s.target.len())?;
            }
        }
        if v >= VERSION_53 {
            same("part offscreen", &[p.offscreen.len()], c.parts)?;
            same("art mesh blend mode", &[a.blend_mode.len()], c.art_meshes)?;
            let os = &self.offscreens;
            same(
                "offscreen",
                &[
                    os.flags.len(),
                    os.blend_mode.len(),
                    os.mask_off.len(),
                    os.mask_len.len(),
                ],
                c.offscreens,
            )?;
            same(
                "part keyform offscreen",
                &[self.part_keyforms.offscreen_keyform.len()],
                c.part_keyforms,
            )?;
            let ok = &self.offscreen_keyforms;
            same(
                "offscreen keyform",
                &[ok.multiply_off.len(), ok.screen_off.len()],
                c.offscreen_keyforms,
            )?;
            same(
                "offscreen blend",
                &[
                    self.blend_offscreens.binding_off.len(),
                    self.blend_offscreens.binding_len.len(),
                ],
                self.blend_offscreens.target.len(),
            )?;
        }

        self.validate_indices(&c)
    }

    fn validate_indices(&self, c: &Counts) -> Result<()> {
        let idx = |what: &str, i: i32, n: usize| -> Result<usize> {
            if i < 0 || i as usize >= n {
                return Err(invalid(format!("{what} index {i} out of range 0..{n}")));
            }
            Ok(i as usize)
        };
        let opt = |what: &str, i: i32, n: usize| -> Result<()> {
            if i != -1 {
                idx(what, i, n)?;
            }
            Ok(())
        };
        let range = |what: &str, off: i32, len: i32, n: usize| -> Result<(usize, usize)> {
            // An empty range may carry any offset (Cubism Editor writes -1).
            if len == 0 {
                return Ok((0, 0));
            }
            if off < 0 || len < 0 || off as usize + len as usize > n {
                return Err(invalid(format!("{what} range {off}+{len} out of 0..{n}")));
            }
            Ok((off as usize, len as usize))
        };

        // Key tables and bindings.
        for i in 0..c.key_tables {
            range(
                "key table keys",
                self.key_tables.keys_off[i],
                self.key_tables.keys_len[i],
                c.keys,
            )?;
        }
        let mut grid = vec![0usize; c.bindings];
        for (b, size) in grid.iter_mut().enumerate() {
            let (off, len) = range(
                "binding key tables",
                self.bindings.key_table_off[b],
                self.bindings.key_table_len[b],
                c.key_table_indices,
            )?;
            if len > MAX_KEY_TABLES {
                return Err(invalid("a binding uses too many parameters"));
            }
            let mut product = 1usize;
            for k in off..off + len {
                let table = idx("key table", self.key_table_indices[k], c.key_tables)?;
                product = product.saturating_mul(self.key_tables.keys_len[table].max(1) as usize);
            }
            *size = product;
        }
        let keyforms = |what: &str, binding: i32, off: i32, n: usize| -> Result<()> {
            let b = idx(what, binding, c.bindings)?;
            range(what, off, grid[b] as i32, n).map(|_| ())
        };

        // Parameters.
        for i in 0..c.parameters {
            range(
                "parameter key tables",
                self.parameters.key_table_off[i],
                self.parameters.key_table_len[i],
                c.key_tables,
            )?;
            if self.version >= VERSION_42 {
                range(
                    "parameter keys",
                    self.parameters.keys_off[i],
                    self.parameters.keys_len[i],
                    c.keys,
                )?;
                range(
                    "parameter blend key tables",
                    self.parameters.blend_key_table_off[i],
                    self.parameters.blend_key_table_len[i],
                    c.blend_key_tables,
                )?;
            }
        }

        // Parts, in parent-before-child order.
        for i in 0..c.parts {
            keyforms(
                "part keyforms",
                self.parts.binding[i],
                self.parts.keyform_off[i],
                c.part_keyforms,
            )?;
            let parent = self.parts.parent_part[i];
            if parent != -1 && (parent < 0 || parent as usize >= i) {
                return Err(invalid(format!("part {i} has parent {parent}")));
            }
            if self.version >= VERSION_53 {
                opt("part offscreen", self.parts.offscreen[i], c.offscreens)?;
            }
        }

        // Deformers.
        for i in 0..c.deformers {
            idx("deformer binding", self.deformers.binding[i], c.bindings)?;
            opt("deformer part", self.deformers.parent_part[i], c.parts)?;
            let parent = self.deformers.parent_deformer[i];
            if parent != -1 && (parent < 0 || parent as usize >= i) {
                return Err(invalid(format!("deformer {i} has parent {parent}")));
            }
            let local = self.deformers.local_index[i];
            match self.deformers.kind[i] {
                DEFORMER_WARP => {
                    idx("warp", local, c.warps)?;
                }
                DEFORMER_ROTATION => {
                    idx("rotation", local, c.rotations)?;
                }
                other => return Err(invalid(format!("unknown deformer type {other}"))),
            }
        }
        for i in 0..c.warps {
            keyforms(
                "warp keyforms",
                self.warps.binding[i],
                self.warps.keyform_off[i],
                c.warp_keyforms,
            )?;
            let (rows, cols) = (self.warps.rows[i], self.warps.cols[i]);
            if rows < 1 || cols < 1 || rows > 1024 || cols > 1024 {
                return Err(invalid(format!("warp {i} has a {cols}×{rows} grid")));
            }
            if self.warps.vertex_count[i] != (rows + 1) * (cols + 1) {
                return Err(invalid(format!("warp {i} has the wrong number of points")));
            }
        }
        for i in 0..c.rotations {
            keyforms(
                "rotation keyforms",
                self.rotations.binding[i],
                self.rotations.keyform_off[i],
                c.rotation_keyforms,
            )?;
        }

        // Keyform positions and colours.
        let positions = |what: &str, off: i32, vertices: i32| -> Result<()> {
            range(what, off, vertices.saturating_mul(2), c.keyform_positions).map(|_| ())
        };
        for i in 0..c.warps {
            let off = self.warps.keyform_off[i] as usize;
            let n = grid[self.warps.binding[i] as usize];
            for k in off..off + n {
                positions(
                    "warp keyform positions",
                    self.warp_keyforms.position_off[k],
                    self.warps.vertex_count[i],
                )?;
            }
            if self.version >= VERSION_42 {
                let base = self.warps.color_off[i];
                if base >= 0 && base as usize + n > c.multiply_colors.min(c.screen_colors) {
                    return Err(invalid("warp colours out of range"));
                }
            }
        }
        for i in 0..c.art_meshes {
            let a = &self.art_meshes;
            keyforms(
                "art mesh keyforms",
                a.binding[i],
                a.keyform_off[i],
                c.art_mesh_keyforms,
            )?;
            let vertices = a.vertex_count[i];
            if !(0..=65536).contains(&vertices) {
                return Err(invalid(format!("art mesh {i} has {vertices} vertices")));
            }
            let off = a.keyform_off[i] as usize;
            let n = grid[a.binding[i] as usize];
            for k in off..off + n {
                positions(
                    "art mesh keyform positions",
                    self.art_mesh_keyforms.position_off[k],
                    vertices,
                )?;
            }
            if self.version >= VERSION_42 {
                let base = a.color_off[i];
                if base >= 0 && base as usize + n > c.multiply_colors.min(c.screen_colors) {
                    return Err(invalid("art mesh colours out of range"));
                }
            }
            opt("art mesh part", a.parent_part[i], c.parts)?;
            opt("art mesh deformer", a.parent_deformer[i], c.deformers)?;
            range("art mesh uvs", a.uv_off[i], vertices.saturating_mul(2), c.uvs)?;
            let (io, il) = range("art mesh indices", a.index_off[i], a.index_len[i], c.indices)?;
            if self.indices[io..io + il].iter().any(|&x| x as i32 >= vertices) {
                return Err(invalid(format!("art mesh {i} indexes a missing vertex")));
            }
            let (mo, ml) = range("art mesh masks", a.mask_off[i], a.mask_len[i], c.masks)?;
            for &m in &self.masks[mo..mo + ml] {
                opt("mask", m, c.art_meshes)?;
            }
        }
        for i in 0..c.rotations {
            if self.version >= VERSION_42 {
                let n = grid[self.rotations.binding[i] as usize];
                let base = self.rotations.color_off[i];
                if base >= 0 && base as usize + n > c.multiply_colors.min(c.screen_colors) {
                    return Err(invalid("rotation colours out of range"));
                }
            }
        }
        if self.version >= VERSION_50 {
            let n = c.multiply_colors.min(c.screen_colors);
            for &k in self
                .warp_keyforms
                .multiply_off
                .iter()
                .chain(&self.warp_keyforms.screen_off)
                .chain(&self.rotation_keyforms.multiply_off)
                .chain(&self.rotation_keyforms.screen_off)
                .chain(&self.art_mesh_keyforms.multiply_off)
                .chain(&self.art_mesh_keyforms.screen_off)
            {
                if k >= 0 && k as usize >= n {
                    return Err(invalid("keyform colour out of range"));
                }
            }
        }

        // Draw order.
        for gi in 0..c.draw_groups {
            let g = &self.draw_groups;
            let (off, len) = range(
                "draw group objects",
                g.object_off[gi],
                g.object_len[gi],
                c.draw_objects,
            )?;
            if g.max_order[gi] < g.min_order[gi] || g.max_order[gi] as i64 - g.min_order[gi] as i64 > 100_000
            {
                return Err(invalid("draw group order range"));
            }
            for k in off..off + len {
                let o = &self.draw_objects;
                match o.kind[k] {
                    DRAW_OBJECT_ART_MESH => {
                        idx("draw object mesh", o.index[k], c.art_meshes)?;
                    }
                    DRAW_OBJECT_PART => {
                        idx("draw object part", o.index[k], c.parts)?;
                        let child = idx("draw object group", o.self_group[k], c.draw_groups)?;
                        if child <= gi {
                            return Err(invalid("draw groups are not in parent-first order"));
                        }
                    }
                    other => return Err(invalid(format!("unknown draw object type {other}"))),
                }
            }
        }

        // Glue.
        for i in 0..c.glues {
            let gl = &self.glues;
            keyforms("glue keyforms", gl.binding[i], gl.keyform_off[i], c.glue_keyforms)?;
            let ma = idx("glue mesh", gl.art_mesh_a[i], c.art_meshes)?;
            let mb = idx("glue mesh", gl.art_mesh_b[i], c.art_meshes)?;
            let (off, len) = range("glue info", gl.info_off[i], gl.info_len[i], c.glue_info)?;
            for (k, &vtx) in self.glue_info.vertex[off..off + len].iter().enumerate() {
                let mesh = if k % 2 == 0 { ma } else { mb };
                if vtx as i32 >= self.art_meshes.vertex_count[mesh] {
                    return Err(invalid("glue indexes a missing vertex"));
                }
            }
        }

        // Blend shapes.
        if self.version >= VERSION_42 {
            for i in 0..c.blend_key_tables {
                let bk = &self.blend_key_tables;
                let (_, len) = range("blend key table keys", bk.keys_off[i], bk.keys_len[i], c.keys)?;
                let base = bk.base_key[i];
                if len > 0 && (base < 0 || base as usize >= len) {
                    return Err(invalid("blend key table base key"));
                }
            }
            for i in 0..c.blend_constraint_indices {
                idx(
                    "blend constraint",
                    self.blend_constraint_indices[i],
                    c.blend_constraints,
                )?;
            }
            for i in 0..c.blend_constraints {
                let bc = &self.blend_constraints;
                idx("blend constraint parameter", bc.parameter[i], c.parameters)?;
                range(
                    "blend constraint values",
                    bc.value_off[i],
                    bc.value_len[i],
                    c.blend_constraint_values,
                )?;
            }
            for i in 0..c.blend_bindings {
                let bb = &self.blend_bindings;
                let table = idx("blend binding key table", bb.key_table[i], c.blend_key_tables)?;
                range(
                    "blend binding constraints",
                    bb.constraint_off[i],
                    bb.constraint_len[i],
                    c.blend_constraint_indices,
                )?;
                if bb.keyform_off[i] < 0 || bb.keyform_len[i] < 0 || table >= c.blend_key_tables {
                    return Err(invalid("blend binding keyforms"));
                }
            }
            let shapes = |what: &str,
                          s: &BlendShapes,
                          targets: usize,
                          keyforms: usize,
                          vertices: Option<&dyn Fn(usize) -> i32>,
                          pos: Option<&Vec<i32>>|
             -> Result<()> {
                for i in 0..s.target.len() {
                    let t = idx(what, s.target[i], targets)?;
                    let (off, len) = range(what, s.binding_off[i], s.binding_len[i], c.blend_bindings)?;
                    for b in off..off + len {
                        let table = self.blend_bindings.key_table[b] as usize;
                        let keys = self.blend_key_tables.keys_len[table].max(0) as usize;
                        let first = self.blend_bindings.keyform_off[b] as usize;
                        // Keyforms `first..first + keys` may be read.
                        if first + keys > keyforms {
                            return Err(invalid(format!("{what} keyforms out of range")));
                        }
                        if let (Some(vc), Some(pos)) = (vertices, pos) {
                            for &at in &pos[first..first + keys] {
                                positions(what, at, vc(t))?;
                            }
                        }
                    }
                }
                Ok(())
            };
            let warp_vc = |t: usize| self.warps.vertex_count[t];
            let mesh_vc = |t: usize| self.art_meshes.vertex_count[t];
            shapes(
                "blend shape warp",
                &self.blend_warps,
                c.warps,
                c.warp_keyforms,
                Some(&warp_vc),
                Some(&self.warp_keyforms.position_off),
            )?;
            shapes(
                "blend shape art mesh",
                &self.blend_art_meshes,
                c.art_meshes,
                c.art_mesh_keyforms,
                Some(&mesh_vc),
                Some(&self.art_mesh_keyforms.position_off),
            )?;
            if self.version >= VERSION_50 {
                shapes(
                    "blend shape part",
                    &self.blend_parts,
                    c.parts,
                    c.part_keyforms,
                    None,
                    None,
                )?;
                shapes(
                    "blend shape rotation",
                    &self.blend_rotations,
                    c.rotations,
                    c.rotation_keyforms,
                    None,
                    None,
                )?;
                shapes(
                    "blend shape glue",
                    &self.blend_glues,
                    c.glues,
                    c.glue_keyforms,
                    None,
                    None,
                )?;
            }
            if self.version >= VERSION_53 {
                shapes(
                    "blend shape offscreen",
                    &self.blend_offscreens,
                    c.offscreens,
                    c.offscreen_keyforms,
                    None,
                    None,
                )?;
            }
        }

        // Offscreens.
        if self.version >= VERSION_53 {
            for i in 0..c.offscreens {
                let os = &self.offscreens;
                idx("offscreen owner", os.owner[i], c.parts)?;
                let (mo, ml) = range("offscreen masks", os.mask_off[i], os.mask_len[i], c.masks)?;
                for &m in &self.masks[mo..mo + ml] {
                    opt("offscreen mask", m, c.art_meshes)?;
                }
            }
            for i in 0..c.part_keyforms {
                let k = self.part_keyforms.offscreen_keyform[i];
                if k >= 0 && k as usize >= c.offscreen_keyforms.max(1) {
                    return Err(invalid("part offscreen keyform out of range"));
                }
            }
            for (i, &owner) in self.offscreens.owner.iter().enumerate() {
                // The owner's keyforms, offset by its first offscreen keyform.
                let part = owner as usize;
                let n = grid[self.parts.binding[part] as usize];
                let first = self.parts.keyform_off[part] as usize;
                let base = self.part_keyforms.offscreen_keyform[first];
                if base >= 0 && base as usize + n > c.offscreen_keyforms {
                    return Err(invalid(format!("offscreen {i} keyforms out of range")));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// A small valid file for robustness tests elsewhere in the crate.
    pub fn tiny_bytes() -> Vec<u8> {
        super::tests::tiny().write()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn tiny() -> Moc {
        // One parameter with keys 0 and 1, one mesh with three vertices
        // keyed at both, no deformers.
        let mut m = Moc {
            version: VERSION_40,
            canvas: Canvas {
                pixels_per_unit: 100.0,
                origin_x: 50.0,
                origin_y: 50.0,
                width: 100.0,
                height: 100.0,
                flags: 0,
            },
            ..Default::default()
        };
        m.parameters = Parameters {
            ids: vec!["ParamX".into()],
            max: vec![1.0],
            min: vec![0.0],
            default: vec![0.0],
            repeat: vec![0],
            decimal_places: vec![1],
            key_table_off: vec![0],
            key_table_len: vec![1],
            ..Default::default()
        };
        m.keys = vec![0.0, 1.0];
        m.key_tables = KeyTables {
            keys_off: vec![0],
            keys_len: vec![2],
        };
        m.key_table_indices = vec![0];
        m.bindings = Bindings {
            key_table_off: vec![0, 0],
            key_table_len: vec![1, 0],
        };
        m.parts = Parts {
            ids: vec!["Part".into()],
            binding: vec![1],
            keyform_off: vec![0],
            key_len: vec![1],
            visible: vec![1],
            enable: vec![1],
            parent_part: vec![-1],
            offscreen: vec![],
        };
        m.part_keyforms.draw_order = vec![500.0];
        m.art_meshes = ArtMeshes {
            ids: vec!["Mesh".into()],
            binding: vec![0],
            keyform_off: vec![0],
            key_len: vec![2],
            visible: vec![1],
            enable: vec![1],
            parent_part: vec![0],
            parent_deformer: vec![-1],
            texture: vec![0],
            flags: vec![0],
            vertex_count: vec![3],
            uv_off: vec![0],
            index_off: vec![0],
            index_len: vec![3],
            mask_off: vec![0],
            mask_len: vec![0],
            ..Default::default()
        };
        m.art_mesh_keyforms = ArtMeshKeyforms {
            opacity: vec![1.0, 0.5],
            draw_order: vec![500.0, 500.0],
            position_off: vec![0, 6],
            ..Default::default()
        };
        m.keyform_positions = vec![0.0, 0.0, 0.2, 0.0, 0.0, 0.2, 0.1, 0.0, 0.3, 0.0, 0.1, 0.2];
        m.uvs = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        m.indices = vec![0, 1, 2];
        m.draw_groups = DrawGroups {
            object_off: vec![0],
            object_len: vec![1],
            total_count: vec![1],
            max_order: vec![1000],
            min_order: vec![0],
        };
        m.draw_objects = DrawObjects {
            kind: vec![DRAW_OBJECT_ART_MESH],
            index: vec![0],
            self_group: vec![-1],
        };
        m.warps.quad = vec![];
        m
    }

    #[test]
    fn a_written_model_reads_back_identically() {
        let moc = tiny();
        moc.validate().unwrap();
        let bytes = moc.write();
        assert_eq!(&bytes[..4], b"MOC3");
        assert_eq!(bytes.len() % 64, 0);
        let back = Moc::read(&bytes).unwrap();
        assert_eq!(back, moc);
        assert_eq!(back.write(), bytes);
    }

    #[test]
    fn broken_files_are_errors_not_panics() {
        let bytes = tiny().write();
        assert_eq!(Moc::read(b"nope"), Err(MocError::NotMoc3));
        let mut v9 = bytes.clone();
        v9[4] = 9;
        assert_eq!(Moc::read(&v9), Err(MocError::UnsupportedVersion(9)));
        assert!(Moc::read(&bytes[..bytes.len() / 2]).is_err());
        // Flip every byte in turn: reading must never panic.
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0xa5;
            let _ = Moc::read(&b);
        }
    }

    #[test]
    fn out_of_range_indices_are_rejected() {
        let mut moc = tiny();
        moc.indices[2] = 7;
        assert!(matches!(moc.validate(), Err(MocError::Invalid(_))));
        let mut moc = tiny();
        moc.art_mesh_keyforms.position_off[1] = 100;
        assert!(moc.validate().is_err());
    }
}
