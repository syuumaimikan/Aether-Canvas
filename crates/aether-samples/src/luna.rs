//! Luna: a finished illustration, rigged.
//!
//! Luna was drawn the way most characters are: one full-body illustration
//! (`assets/luna/character.png`) and a sheet of separate parts
//! (`assets/luna/parts-sheet.png`). `tools/prepare_luna.py` cuts what has
//! to change shape from the sheet, fits it onto the illustration and writes
//! it as layers the illustration's size (`assets/luna/layers/`): skin to
//! cover the drawn eyes and mouth, the sheet's eyes split into whites,
//! irises and lashes, its mouth shapes, a blush, and the strands of the
//! illustration's hair that fall over the eyes. Closed eyes are painted
//! here, in the lashes' colour.
//!
//! Everything else moves by deforming the illustration itself through a
//! stack of warp deformers, each confined to its region (its lattice edges
//! stay put, so what lies outside is untouched):
//!
//! ```text
//! Body            lean (BodyAngleZ), turn (BodyAngleX), bow (BodyAngleY), breath
//! └ Hem           drapes and hair ends (HairBack)
//!   └ Hair R      the long hair on the viewer's left (HairSide)
//!     └ Hair L    and on the right, around the arms
//!       └ Head tilt   AngleZ, bending the neck
//!         └ Head turn AngleX and AngleY, a skull shaped from her face
//!           ├ Bangs   fringe and ahoge (HairFront), tassels (HairSide)
//!           │ ├ Illustration
//!           │ └ Hair over eyes
//!           └ the face: cover, eyes, mouths, blush
//! ```
//!
//! On top: blinking and smiling eyes, eyes that look around, five mouth
//! shapes, a blush, hair physics, breathing, a body-follows-head driver,
//! expressions and motions.

use crate::paint::{Canvas, Color, Mode, Path, Stroke};
use crate::rig::{expression, motion, param};
use aether_core::id::IdGenerator;
use aether_core::math::{Rect, Vec2};
use aether_core::{AetherError, DeformerId, LayerId, ParameterId, Result};
use aether_document::layer::Layer;
use aether_document::rig::generate::{self, HeadShape};
use aether_document::rig::{
    ArtMesh, Behaviours, Deformer, DeformerKind, Driver, KeyAxis, KeyInterpolation, KeyformGrid, MeshForm,
    NodeRef, Rig, RigNode, WarpForm,
};
use aether_document::Document;
use aether_raster::Pixmap;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Canvas size: the illustration's.
pub const WIDTH: u32 = 1024;
pub const HEIGHT: u32 = 1536;

macro_rules! layer_png {
    ($name:literal) => {
        include_bytes!(concat!("../assets/luna/layers/", $name, ".png")).as_slice()
    };
}

const ILLUSTRATION: &[u8] = include_bytes!("../assets/luna/character.png");
const LAYOUT: &[u8] = include_bytes!("../assets/luna/layers/layout.json");

// Landmarks of the illustration, in pixels.
/// Where the feet stand: the body leans about it.
const FEET: Vec2 = Vec2::new(512.0, 1490.0);
/// The base of the neck: the head tilts about it.
const NECK: Vec2 = Vec2::new(527.0, 300.0);
/// The face, cheek to cheek and hairline to chin.
const FACE: Rect = Rect {
    min: Vec2::new(455.0, 122.0),
    max: Vec2::new(595.0, 264.0),
};
/// The middle of the face.
const FACE_X: f32 = 525.0;
/// Shoulder and hand on the viewer's left (her right) and right (her left).
const ARM_R: (Vec2, Vec2) = (Vec2::new(330.0, 430.0), Vec2::new(157.0, 667.0));
const ARM_L: (Vec2, Vec2) = (Vec2::new(694.0, 430.0), Vec2::new(888.0, 685.0));
/// The legs' vertical axis.
const LEGS_X: f32 = 515.0;
/// The tassels hanging from her hair ornaments.
const TASSELS: [f32; 2] = [376.0, 653.0];

/// Where things are, written by `prepare_luna.py`.
#[derive(Clone, Debug, Deserialize)]
struct Layout {
    eyes: BTreeMap<String, EyeLayout>,
    mouth: MouthLayout,
}

#[derive(Clone, Debug, Deserialize)]
struct EyeLayout {
    lash_colour: [f32; 3],
    /// `[x0, y0, x1, y1]` of the eye.
    bounds: [f32; 4],
    /// Corners of the opening.
    outer: [f32; 2],
    inner: [f32; 2],
    /// The upper lid (the bottom edge of the upper lashes), `[x, y]` left
    /// to right.
    upper: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, Deserialize)]
struct MouthLayout {
    centre: [f32; 2],
}

/// An eye's lids. They meet on a line from a little past the outer corner
/// to the inner one, sagging in the middle; the upper lid comes down to it.
#[derive(Clone, Debug)]
struct Lids {
    from: Vec2,
    to: Vec2,
    sag: f32,
    height: f32,
    upper: Vec<[f32; 2]>,
}

impl Lids {
    fn new(eye: &EyeLayout) -> Self {
        let outer = Vec2::new(eye.outer[0], eye.outer[1]);
        let inner = Vec2::new(eye.inner[0], eye.inner[1]);
        let length = outer.distance(inner).max(1.0);
        let out = (outer - inner).normalized();
        Self {
            from: outer + out * (0.22 * length),
            to: inner - out * (0.04 * length),
            sag: 0.1 * length,
            height: (eye.bounds[3] - eye.bounds[1]).max(1.0),
            upper: eye.upper.clone(),
        }
    }

    /// Height of the upper lid at `x` (the meeting line where there is no
    /// lid, so nothing moves there).
    fn upper(&self, x: f32) -> f32 {
        let meet = self.y(x);
        let u = &self.upper;
        let (Some(first), Some(last)) = (u.first(), u.last()) else {
            return meet;
        };
        let y = if x <= first[0] {
            first[1]
        } else if x >= last[0] {
            last[1]
        } else {
            let i = u.partition_point(|p| p[0] <= x).max(1);
            let (a, b) = (u[i - 1], u[i]);
            let t = if b[0] > a[0] {
                (x - a[0]) / (b[0] - a[0])
            } else {
                0.0
            };
            a[1] + (b[1] - a[1]) * t
        };
        y.min(meet)
    }

    fn length(&self) -> f32 {
        self.from.distance(self.to)
    }

    /// Position along the line (0 at the outer end) of `x`.
    fn t(&self, x: f32) -> f32 {
        let span = self.to.x - self.from.x;
        if span.abs() < 1e-3 {
            0.5
        } else {
            ((x - self.from.x) / span).clamp(0.0, 1.0)
        }
    }

    /// `1 − u²` across the line: 1 in the middle, 0 at the ends.
    fn bulge(&self, x: f32) -> f32 {
        let u = 2.0 * self.t(x) - 1.0;
        1.0 - u * u
    }

    /// Height of the line at `x`.
    fn y(&self, x: f32) -> f32 {
        let t = self.t(x);
        self.from.y + (self.to.y - self.from.y) * t + self.sag * self.bulge(x)
    }

    /// Where an open eye's point `p` goes: `open` 1 leaves it, 0 shuts the
    /// eye onto the line; `smile` lifts the lower lid.
    ///
    /// Closing slides the upper lashes down whole (squashing them would
    /// thin them to a grey line) and folds the opening below them into what
    /// is left; the lower lid rises a little.
    fn close(&self, p: Vec2, open: f32, smile: f32) -> Vec2 {
        let mut q = p;
        let meet = self.y(q.x);
        let pivot = meet - 0.2 * self.height;
        if q.y > pivot {
            q.y = pivot + (q.y - pivot) * (1.0 - 0.6 * smile);
        }
        if q.y < meet {
            let lid = self.upper(q.x);
            let gap = meet - lid;
            let left = piecewise(open, &[(0.0, 0.0), (0.25, 0.2), (0.5, 0.45), (1.0, 1.0)]);
            q.y = if q.y <= lid {
                q.y + (1.0 - left) * gap
            } else {
                meet - (meet - q.y) * left
            };
        } else {
            q.y = meet + (q.y - meet) * piecewise(open, &[(0.0, 0.5), (0.25, 0.7), (0.5, 0.85), (1.0, 1.0)]);
        }
        q
    }
}

/// Piecewise-linear through `(x, y)` points in increasing `x`, flat beyond
/// the ends.
fn piecewise(t: f32, points: &[(f32, f32)]) -> f32 {
    let (first, last) = (points[0], points[points.len() - 1]);
    if t <= first.0 {
        return first.1;
    }
    for w in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if t <= x1 {
            return y0 + (y1 - y0) * (t - x0) / (x1 - x0).max(1e-6);
        }
    }
    last.1
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 0 below `a`, rising to 1 at `b`, falling from `c` back to 0 at `d`.
fn band(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    smoothstep(a, b, x) * (1.0 - smoothstep(c, d, x))
}

/// Near 1 along the segment `a`–`b`, fading over `radius`.
fn capsule(p: Vec2, (a, b): (Vec2, Vec2), radius: f32) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    let d = p.distance(a + ab * t) / radius;
    (-d * d).exp()
}

fn rotate_about(p: Vec2, pivot: Vec2, degrees: f32) -> Vec2 {
    pivot + (p - pivot).rotated(degrees.to_radians())
}

/// Luna's layers.
#[derive(Clone, Debug)]
pub struct Parts {
    /// The illustration, which everything but the face is.
    pub illustration: LayerId,
    /// Skin over the illustration's own eyes and mouth.
    pub face_cover: LayerId,
    pub cheek: LayerId,
    /// The illustration's mouth, then the sheet's: line, open, wide, "o".
    pub mouths: [LayerId; 5],
    /// Eye layers, `[L, R]` as she sees them: her left eye is on the
    /// viewer's right.
    pub eye_white: [LayerId; 2],
    pub iris: [LayerId; 2],
    pub lash: [LayerId; 2],
    pub eye_closed: [LayerId; 2],
    /// Strands of hair in front of the eyes.
    pub hair_over_eyes: LayerId,
}

fn decode(bytes: &[u8]) -> Result<Pixmap> {
    aether_io::image_io::decode_image(bytes)
}

fn layout() -> Result<Layout> {
    serde_json::from_slice(LAYOUT).map_err(|e| AetherError::Asset(format!("Luna's layout: {e}")))
}

/// A closed eye: a lash line where the lids meet, heavy at the outer
/// corner, with two short lashes there.
fn paint_closed_eye(eye: &EyeLayout) -> Canvas {
    let lids = Lids::new(eye);
    let [r, g, b] = eye.lash_colour;
    let colour = Color { r, g, b, a: 1.0 };
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let along = |t: f32| {
        let x = lids.from.x + (lids.to.x - lids.from.x) * t;
        (x, lids.y(x))
    };
    let points: Vec<(f32, f32)> = (1..=10).map(|k| along(k as f32 / 10.0)).collect();
    let line = Path::new()
        .move_to(lids.from.x, lids.from.y)
        .smooth_through(&points);
    let width = (lids.length() * 0.075).clamp(2.2, 4.0);
    c.stroke(
        &line,
        Stroke {
            width,
            taper_in: 0.06,
            taper_out: 0.6,
        },
        colour,
        Mode::Over,
    );
    let out = (lids.from - lids.to).normalized();
    let down = Vec2::new(-out.y, out.x) * if out.x < 0.0 { -1.0 } else { 1.0 };
    for (start, reach, drop, w) in [(0.02, 0.16, 0.10, width * 0.7), (0.12, 0.12, 0.2, width * 0.55)] {
        let (sx, sy) = along(start);
        let s = Vec2::new(sx, sy);
        let len = lids.length();
        let e = s + out * (reach * len) + down * (drop * len);
        let lash = Path::new().move_to(s.x, s.y).quad_to(
            (s.x + e.x) * 0.5 + down.x * 1.5,
            (s.y + e.y) * 0.5 + down.y * 1.5,
            e.x,
            e.y,
        );
        c.stroke(
            &lash,
            Stroke {
                width: w,
                taper_in: 0.0,
                taper_out: 1.0,
            },
            colour,
            Mode::Over,
        );
    }
    c
}

/// A mesh of square cells, `spacing` pixels apart, over every cell the
/// layer's visible pixels touch.
fn grid_mesh(layer: LayerId, name: &str, pixmap: &Pixmap, spacing: f32) -> Option<ArtMesh> {
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let data = pixmap.data();
    let visible = |x: usize, y: usize| data[(y * w + x) * 4 + 3] > 0;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if visible(x, y) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 {
        return None;
    }
    let origin = Vec2::new(x0 as f32 - 2.0, y0 as f32 - 2.0);
    let nx = (((x1 - x0) as f32 + 5.0) / spacing).ceil() as usize;
    let ny = (((y1 - y0) as f32 + 5.0) / spacing).ceil() as usize;
    let mut used = vec![false; nx * ny];
    let cell = |v: f32, o: f32, n: usize| (((v - o) / spacing).floor().max(0.0) as usize).min(n - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            if !visible(x, y) {
                continue;
            }
            // The pixel and the neighbourhood filtering reads around it.
            for (dx, dy) in [(-0.5, -0.5), (1.5, -0.5), (-0.5, 1.5), (1.5, 1.5)] {
                let i = cell(x as f32 + dx, origin.x, nx);
                let j = cell(y as f32 + dy, origin.y, ny);
                used[j * nx + i] = true;
            }
        }
    }
    let mut index = vec![u32::MAX; (nx + 1) * (ny + 1)];
    let mut vertices = Vec::new();
    let mut corner = |i: usize, j: usize, vertices: &mut Vec<Vec2>| -> u32 {
        let k = j * (nx + 1) + i;
        if index[k] == u32::MAX {
            index[k] = vertices.len() as u32;
            vertices.push(origin + Vec2::new(i as f32 * spacing, j as f32 * spacing));
        }
        index[k]
    };
    let mut triangles = Vec::new();
    for j in 0..ny {
        for i in 0..nx {
            if !used[j * nx + i] {
                continue;
            }
            let a = corner(i, j, &mut vertices);
            let b = corner(i + 1, j, &mut vertices);
            let c = corner(i, j + 1, &mut vertices);
            let d = corner(i + 1, j + 1, &mut vertices);
            triangles.push([a, b, d]);
            triangles.push([a, d, c]);
        }
    }
    let mut mesh = ArtMesh::new(layer, vertices, triangles);
    mesh.name = name.to_string();
    Some(mesh)
}

/// A keyform grid over `axes`, axis 0 varying fastest, each form computed
/// from its key values.
fn grid<T>(
    axes: &[(ParameterId, &[f32])],
    interpolation: KeyInterpolation,
    mut form: impl FnMut(&[f32]) -> T,
) -> Result<KeyformGrid<T>> {
    let axes: Vec<KeyAxis> = axes
        .iter()
        .map(|(p, keys)| KeyAxis::new(*p, keys.iter().copied()))
        .collect::<Result<_>>()?;
    let total: usize = axes.iter().map(|a| a.keys.len()).product();
    let mut values = vec![0.0; axes.len()];
    let mut forms = Vec::with_capacity(total);
    for i in 0..total {
        let mut rest = i;
        for (a, axis) in axes.iter().enumerate() {
            values[a] = axis.keys[rest % axis.keys.len()];
            rest /= axis.keys.len();
        }
        forms.push(form(&values));
    }
    Ok(KeyformGrid {
        axes,
        forms,
        interpolation,
    })
}

/// Add a warp over `rect` whose keyforms move each rest lattice point by
/// `shift(point, key values)`.
#[allow(clippy::too_many_arguments)]
fn add_warp(
    rig: &mut Rig,
    ids: &IdGenerator,
    name: &str,
    rect: Rect,
    (cols, rows): (usize, usize),
    parent: Option<DeformerId>,
    axes: &[(ParameterId, &[f32])],
    interpolation: KeyInterpolation,
    shift: impl Fn(Vec2, &[f32]) -> Vec2,
) -> Result<DeformerId> {
    let id = ids.deformer();
    let mut deformer = Deformer::warp(id, name, rect, cols, rows);
    deformer.parent = parent.map(NodeRef::Deformer);
    if let DeformerKind::Warp(warp) = &mut deformer.kind {
        let rest = warp.rest_points();
        warp.keyforms = grid(axes, interpolation, |values| WarpForm {
            offsets: rest.iter().map(|p| shift(*p, values)).collect(),
            opacity: 1.0,
        })?;
    }
    rig.deformers.push(deformer);
    Ok(id)
}

/// Key a mesh: `form(key values, rest vertex)` gives where the vertex goes,
/// `opacity(key values)` how visible the mesh is.
fn key_mesh(
    rig: &mut Rig,
    layer: LayerId,
    axes: &[(ParameterId, &[f32])],
    form: impl Fn(&[f32], Vec2) -> Vec2,
    opacity: impl Fn(&[f32]) -> f32,
) -> Result<()> {
    let mesh = rig
        .mesh_mut(layer)
        .ok_or_else(|| AetherError::rig("the part has no mesh"))?;
    let rest = mesh.vertices.clone();
    mesh.keyforms = grid(axes, KeyInterpolation::Linear, |values| MeshForm {
        offsets: rest.iter().map(|v| form(values, *v) - *v).collect(),
        opacity: opacity(values),
        ..MeshForm::rest(rest.len())
    })?;
    Ok(())
}

/// Paint Luna's layers into a new document, unrigged.
pub fn paint() -> Result<(Document, Parts)> {
    let layout = layout()?;
    let eye = |side: &str| {
        layout
            .eyes
            .get(side)
            .cloned()
            .ok_or_else(|| AetherError::Asset(format!("Luna's layout has no eye {side}")))
    };
    let eyes = [eye("L")?, eye("R")?];

    let mut doc = Document::empty(WIDTH, HEIGHT, "Luna");
    let mut add = |name: &str, pixmap: Pixmap, clipping: bool| -> LayerId {
        let id = doc.next_layer_id();
        let mut layer = Layer::raster(id, name, WIDTH, HEIGHT);
        if let Some(pm) = layer.pixmap_mut() {
            *pm = pixmap;
        }
        layer.clipping = clipping;
        let _ = doc.layers.push_top(layer);
        id
    };
    let illustration = add("Illustration", decode(ILLUSTRATION)?, false);
    let face_cover = add("Face cover", decode(layer_png!("face_cover"))?, false);
    let cheek = add("Cheek", decode(layer_png!("cheek"))?, false);
    let mouths = [
        add("Mouth", decode(layer_png!("mouth"))?, false),
        add("Mouth line", decode(layer_png!("mouth_line"))?, false),
        add("Mouth open", decode(layer_png!("mouth_open"))?, false),
        add("Mouth wide", decode(layer_png!("mouth_wide"))?, false),
        add("Mouth o", decode(layer_png!("mouth_o"))?, false),
    ];
    let files = [
        (
            "L",
            layer_png!("eye_white_L"),
            layer_png!("iris_L"),
            layer_png!("lash_L"),
        ),
        (
            "R",
            layer_png!("eye_white_R"),
            layer_png!("iris_R"),
            layer_png!("lash_R"),
        ),
    ];
    let mut eye_white = [LayerId(0); 2];
    let mut iris = [LayerId(0); 2];
    let mut lash = [LayerId(0); 2];
    let mut eye_closed = [LayerId(0); 2];
    // Her right eye (the viewer's left) first, as the parts sheet has them.
    for i in [1, 0] {
        let (side, white_png, iris_png, lash_png) = files[i];
        eye_white[i] = add(&format!("Eye white {side}"), decode(white_png)?, false);
        iris[i] = add(&format!("Iris {side}"), decode(iris_png)?, true);
        lash[i] = add(&format!("Eyelash {side}"), decode(lash_png)?, false);
        eye_closed[i] = add(
            &format!("Eye closed {side}"),
            paint_closed_eye(&eyes[i]).to_pixmap(),
            false,
        );
    }
    let hair_over_eyes = add("Hair over eyes", decode(layer_png!("hair_over_eyes"))?, false);
    doc.active_layer = illustration;
    Ok((
        doc,
        Parts {
            illustration,
            face_cover,
            cheek,
            mouths,
            eye_white,
            iris,
            lash,
            eye_closed,
            hair_over_eyes,
        },
    ))
}

/// Paint and rig Luna.
pub fn luna() -> Result<Document> {
    Ok(build()?.0)
}

/// Luna rigged, with her layers.
pub fn build() -> Result<(Document, Parts)> {
    let (mut doc, parts) = paint()?;
    rig(&mut doc, &parts)?;
    Ok((doc, parts))
}

/// Mesh every layer, at a spacing suited to how it moves.
fn mesh_parts(doc: &mut Document, parts: &Parts) -> Result<()> {
    let spacing = |layer: LayerId| {
        if layer == parts.illustration {
            16.0
        } else if layer == parts.face_cover || layer == parts.cheek {
            8.0
        } else if parts.mouths.contains(&layer) || layer == parts.hair_over_eyes {
            4.0
        } else {
            3.0
        }
    };
    let layers: Vec<(LayerId, String)> = doc.layers.iter().map(|l| (l.id, l.name.clone())).collect();
    for (id, name) in layers {
        let Some(pixmap) = doc.layers.get(id).and_then(|l| l.pixmap()) else {
            continue;
        };
        let mesh = grid_mesh(id, &name, pixmap, spacing(id))
            .ok_or_else(|| AetherError::Asset(format!("Luna's layer {name} is empty")))?;
        doc.rig.set_mesh(mesh);
    }
    Ok(())
}

/// Rig Luna's layers (see the module documentation).
pub fn rig(doc: &mut Document, parts: &Parts) -> Result<()> {
    let layout = layout()?;
    mesh_parts(doc, parts)?;
    let ids = &doc.ids;
    let rig = &mut doc.rig;
    rig.add_standard_parameters(ids);
    let (angle_x, angle_y, angle_z) = (
        param(rig, "AngleX")?,
        param(rig, "AngleY")?,
        param(rig, "AngleZ")?,
    );
    let (body_x, body_y, body_z) = (
        param(rig, "BodyAngleX")?,
        param(rig, "BodyAngleY")?,
        param(rig, "BodyAngleZ")?,
    );
    let breath = param(rig, "Breath")?;
    let (hair_front, hair_side, hair_back) = (
        param(rig, "HairFront")?,
        param(rig, "HairSide")?,
        param(rig, "HairBack")?,
    );
    const ANGLE: &[f32] = &[-30.0, 0.0, 30.0];
    const BODY: &[f32] = &[-10.0, 0.0, 10.0];
    const SWAY: &[f32] = &[-1.0, 0.0, 1.0];
    const LINEAR: KeyInterpolation = KeyInterpolation::Linear;

    // The whole figure: leaning from the feet, turning and bowing from the
    // waist up, breathing with the chest and shoulders.
    let body = add_warp(
        rig,
        ids,
        "Body",
        Rect::from_corners(Vec2::new(-40.0, -40.0), Vec2::new(1064.0, 1576.0)),
        (10, 14),
        None,
        &[
            (body_x, BODY),
            (body_y, BODY),
            (body_z, BODY),
            (breath, &[0.0, 1.0]),
        ],
        LINEAR,
        |p, v| {
            let (nx, ny, nz, b) = (v[0] / 10.0, v[1] / 10.0, v[2] / 10.0, v[3]);
            let upper = 1.0 - smoothstep(560.0, 980.0, p.y);
            let bulge = (1.0 - ((p.x - 512.0) / 300.0).powi(2)).max(0.0);
            let side = band((p.x - 512.0).abs(), 90.0, 200.0, 260.0, 360.0);
            let lift = b
                * (3.0 * (1.0 - smoothstep(430.0, 720.0, p.y))
                    + 1.5 * (-((p.y - 390.0) / 50.0).powi(2)).exp() * side);
            let moved = p + Vec2::new(nx * upper * (12.0 + 8.0 * bulge), -ny * upper * 7.0 - lift);
            rotate_about(moved, FEET, nz * (1.0 + 2.6 * upper)) - p
        },
    )?;

    // The ends of the drapes and of the hair, around the legs.
    let hem = add_warp(
        rig,
        ids,
        "Hem",
        Rect::from_corners(Vec2::new(-40.0, 820.0), Vec2::new(1064.0, 1560.0)),
        (14, 10),
        Some(body),
        &[(hair_back, SWAY)],
        LINEAR,
        |p, v| {
            let legs = 1.0 - smoothstep(95.0, 165.0, (p.x - LEGS_X).abs());
            let w = smoothstep(860.0, 1380.0, p.y) * (1.0 - legs);
            Vec2::new(v[0] * 26.0 * w, -v[0].abs() * 5.0 * w)
        },
    )?;

    // The long hair either side, keeping still around the arms and hands.
    let hair = |p: Vec2, s: f32, wx: f32, arm: (Vec2, Vec2)| {
        let g = band(p.y, 290.0, 900.0, 1180.0, 1320.0);
        let w = g * wx * (1.0 - capsule(p, arm, 70.0));
        Vec2::new(s * 30.0 * w, -s.abs() * 4.0 * w)
    };
    let hair_r = add_warp(
        rig,
        ids,
        "Hair R",
        Rect::from_corners(Vec2::new(-40.0, 240.0), Vec2::new(440.0, 1330.0)),
        (12, 22),
        Some(hem),
        &[(hair_side, SWAY)],
        LINEAR,
        |p, v| hair(p, v[0], 1.0 - smoothstep(300.0, 420.0, p.x), ARM_R),
    )?;
    let hair_l = add_warp(
        rig,
        ids,
        "Hair L",
        Rect::from_corners(Vec2::new(584.0, 240.0), Vec2::new(1064.0, 1330.0)),
        (12, 22),
        Some(hair_r),
        &[(hair_side, SWAY)],
        LINEAR,
        |p, v| hair(p, v[0], smoothstep(604.0, 724.0, p.x), ARM_L),
    )?;

    // The head tilts about the neck, which bends.
    let tilt = add_warp(
        rig,
        ids,
        "Head tilt",
        Rect::from_corners(Vec2::new(270.0, -60.0), Vec2::new(790.0, 400.0)),
        (10, 10),
        Some(hair_l),
        &[(angle_z, ANGLE)],
        LINEAR,
        |p, v| {
            let w = (1.0 - smoothstep(262.0, 345.0, p.y))
                * (1.0 - smoothstep(170.0, 250.0, (p.x - FACE_X).abs()));
            rotate_about(p, NECK, v[0] / 30.0 * 12.0 * w) - p
        },
    )?;

    // The head turns on a skull shaped from her face, fading out below the
    // chin and past the hair at the sides so the neck and shoulders stay.
    let head = HeadShape::around(FACE);
    let turn = add_warp(
        rig,
        ids,
        "Head turn",
        Rect::from_corners(Vec2::new(300.0, -40.0), Vec2::new(760.0, 360.0)),
        (16, 14),
        Some(tilt),
        &[(angle_x, ANGLE), (angle_y, ANGLE)],
        KeyInterpolation::Smooth,
        |p, v| {
            let w = (1.0 - smoothstep(262.0, 335.0, p.y))
                * (1.0 - smoothstep(165.0, 225.0, (p.x - FACE.center().x).abs()));
            let yaw = (v[0] / 30.0 * 30.0).to_radians();
            let pitch = (v[1] / 30.0 * 20.0).to_radians();
            (head.turn_surface(p, yaw, pitch) - p) * w
        },
    )?;

    // The fringe and the ahoge swing from the top of the head; the tassels
    // swing with the side hair.
    let bangs = add_warp(
        rig,
        ids,
        "Bangs",
        Rect::from_corners(Vec2::new(330.0, 0.0), Vec2::new(720.0, 330.0)),
        (13, 11),
        Some(turn),
        &[(hair_front, SWAY), (hair_side, SWAY)],
        LINEAR,
        |p, v| {
            let fringe = band(p.x, 415.0, 455.0, 600.0, 640.0)
                * band(p.y, 45.0, 110.0, 165.0, 215.0)
                * ((p.y - 40.0) / 140.0).clamp(0.0, 1.0);
            let ahoge =
                (1.0 - smoothstep(10.0, 30.0, (p.x - 482.0).abs())) * (1.0 - smoothstep(55.0, 90.0, p.y));
            let tassel = |x: f32| {
                (1.0 - smoothstep(12.0, 34.0, (p.x - x).abs())) * band(p.y, 175.0, 285.0, 295.0, 325.0)
            };
            let tassels: f32 = TASSELS.iter().map(|x| tassel(*x)).sum();
            Vec2::new(v[0] * 7.0 * (fringe + ahoge) + v[1] * 9.0 * tassels, 0.0)
        },
    )?;

    for layer in [parts.illustration, parts.hair_over_eyes] {
        rig.set_parent(RigNode::Mesh(layer), Some(NodeRef::Deformer(bangs)))?;
    }
    let face: Vec<LayerId> = rig
        .meshes
        .iter()
        .map(|m| m.layer)
        .filter(|l| *l != parts.illustration && *l != parts.hair_over_eyes)
        .collect();
    for layer in face {
        rig.set_parent(RigNode::Mesh(layer), Some(NodeRef::Deformer(turn)))?;
    }

    key_face(rig, parts, &layout)?;

    // Hair physics, blinking, breathing, lip sync, and a body that follows
    // the head.
    generate::standard_physics(rig);
    rig.behaviours = Behaviours::standard(&rig.parameters);
    if !rig.drivers.iter().any(|d| d.target == body_x) {
        rig.drivers.push(Driver::new(body_x, "self + AngleX * 0.3"));
    }
    acting(rig)?;
    rig.validate()?;
    Ok(())
}

/// Eyes, mouth and blush.
fn key_face(rig: &mut Rig, parts: &Parts, layout: &Layout) -> Result<()> {
    // Eyes swap to the painted closed line in the last quarter of a blink.
    const EYE_OPEN: &[f32] = &[0.0, 0.25, 0.5, 1.0];
    const OPEN: &[f32] = &[0.0, 0.5, 1.0];
    const ON: &[f32] = &[0.0, 1.0];
    const SWAY: &[f32] = &[-1.0, 0.0, 1.0];
    let ball_x = param(rig, "EyeBallX")?;
    let ball_y = param(rig, "EyeBallY")?;
    for (i, side) in ["L", "R"].into_iter().enumerate() {
        let eye = layout
            .eyes
            .get(side)
            .ok_or_else(|| AetherError::Asset(format!("Luna's layout has no eye {side}")))?;
        let lids = Lids::new(eye);
        let open = param(rig, &format!("Eye{side}Open"))?;
        let smile = param(rig, &format!("Eye{side}Smile"))?;
        let shown = |v: &[f32]| piecewise(v[0], &[(0.0, 0.0), (0.25, 1.0)]);
        for layer in [parts.eye_white[i], parts.lash[i]] {
            key_mesh(
                rig,
                layer,
                &[(open, EYE_OPEN), (smile, ON)],
                |v, p| lids.close(p, v[0], v[1]),
                shown,
            )?;
        }
        let look = Vec2::new(0.1 * lids.length(), 0.09 * lids.height);
        key_mesh(
            rig,
            parts.iris[i],
            &[(open, EYE_OPEN), (smile, ON), (ball_x, SWAY), (ball_y, SWAY)],
            |v, p| lids.close(p + Vec2::new(v[2] * look.x, -v[3] * look.y), v[0], v[1]),
            shown,
        )?;
        // The closed line bends into a happy arc when smiling.
        let arch = lids.sag + 0.2 * lids.length();
        key_mesh(
            rig,
            parts.eye_closed[i],
            &[(open, EYE_OPEN), (smile, ON)],
            |v, p| p - Vec2::new(0.0, v[1] * arch * lids.bulge(p.x)),
            |v| piecewise(v[0], &[(0.0, 1.0), (0.25, 0.0)]),
        )?;
    }

    // Mouth shapes cross-fade over (MouthForm, MouthOpenY); each stretches
    // a little toward its neighbours and curls with the form.
    let form = param(rig, "MouthForm")?;
    let open = param(rig, "MouthOpenY")?;
    let centre = Vec2::new(layout.mouth.centre[0], layout.mouth.centre[1] + 2.0);
    // (natural openness, opacity at [form −1, 0, 1] × [open 0, ½, 1]).
    type Shown = [[f32; 3]; 3];
    let shapes: [(f32, Shown); 5] = [
        (0.0, [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]),
        (0.0, [[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]),
        (0.5, [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]]),
        (1.0, [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]]),
        (0.75, [[0.0, 1.0, 1.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]),
    ];
    let key = |v: f32, keys: &[f32]| keys.iter().position(|k| (k - v).abs() < 1e-4).unwrap_or(0);
    for (layer, (natural, table)) in parts.mouths.into_iter().zip(shapes) {
        key_mesh(
            rig,
            layer,
            &[(form, SWAY), (open, OPEN)],
            |v, p| {
                let (f, o) = (v[0], v[1]);
                let stretch = (1.0 + (o - natural) * 0.7).clamp(0.55, 1.5);
                let u = ((p.x - centre.x) / 20.0).clamp(-1.0, 1.0);
                Vec2::new(
                    centre.x + (p.x - centre.x) * (1.0 + 0.06 * f),
                    centre.y + (p.y - centre.y) * stretch - f * 2.2 * u * u,
                )
            },
            |v| table[key(v[0], SWAY)][key(v[1], OPEN)],
        )?;
    }

    let cheek = param(rig, "Cheek")?;
    key_mesh(rig, parts.cheek, &[(cheek, ON)], |_, p| p, |v| v[0])?;
    Ok(())
}

/// Expressions and motions.
fn acting(rig: &mut Rig) -> Result<()> {
    let expressions = vec![
        expression(
            rig,
            "Smile",
            &[
                ("EyeLOpen", 0.0),
                ("EyeROpen", 0.0),
                ("EyeLSmile", 1.0),
                ("EyeRSmile", 1.0),
                ("MouthForm", 1.0),
                ("MouthOpenY", 0.5),
                ("Cheek", 0.4),
            ],
        )?,
        expression(
            rig,
            "Surprised",
            &[("MouthForm", -1.0), ("MouthOpenY", 0.8), ("EyeBallY", 0.2)],
        )?,
        expression(
            rig,
            "Calm",
            &[("EyeLOpen", 0.0), ("EyeROpen", 0.0), ("MouthForm", 0.0)],
        )?,
        expression(
            rig,
            "Wink",
            &[
                ("EyeLOpen", 0.0),
                ("EyeLSmile", 1.0),
                ("MouthForm", 1.0),
                ("MouthOpenY", 0.2),
            ],
        )?,
        expression(
            rig,
            "Shy",
            &[
                ("Cheek", 1.0),
                ("EyeLSmile", 0.5),
                ("EyeRSmile", 0.5),
                ("EyeBallX", 0.4),
                ("EyeBallY", -0.5),
                ("MouthForm", 0.3),
            ],
        )?,
        expression(
            rig,
            "Troubled",
            &[
                ("EyeLOpen", 0.7),
                ("EyeROpen", 0.7),
                ("EyeBallY", -0.3),
                ("MouthForm", -1.0),
            ],
        )?,
    ];
    rig.expressions.extend(expressions);

    let motions = vec![
        motion(
            rig,
            "Idle",
            6.0,
            true,
            &[
                ("AngleX", &[(0.0, 0.0), (1.8, 6.0), (4.0, -5.0), (6.0, 0.0)]),
                ("AngleY", &[(0.0, 0.0), (1.5, -3.0), (3.6, 2.0), (6.0, 0.0)]),
                ("AngleZ", &[(0.0, 0.0), (2.0, 4.0), (4.4, -3.0), (6.0, 0.0)]),
                ("BodyAngleZ", &[(0.0, 0.0), (2.4, 2.0), (4.8, -2.0), (6.0, 0.0)]),
                ("EyeBallX", &[(0.0, 0.0), (1.8, 0.3), (4.0, -0.3), (6.0, 0.0)]),
            ],
        )?,
        motion(
            rig,
            "Greeting",
            3.2,
            false,
            &[
                ("AngleY", &[(0.0, 0.0), (0.45, -12.0), (0.9, 4.0), (1.4, 0.0)]),
                ("AngleZ", &[(0.0, 0.0), (0.8, 10.0), (2.4, 10.0), (3.0, 0.0)]),
                ("BodyAngleZ", &[(0.0, 0.0), (0.9, 3.0), (2.4, 3.0), (3.1, 0.0)]),
                ("EyeLOpen", &[(0.0, 1.0), (0.5, 0.0), (2.3, 0.0), (2.7, 1.0)]),
                ("EyeROpen", &[(0.0, 1.0), (0.5, 0.0), (2.3, 0.0), (2.7, 1.0)]),
                ("EyeLSmile", &[(0.0, 0.0), (0.5, 1.0), (2.4, 1.0), (2.8, 0.0)]),
                ("EyeRSmile", &[(0.0, 0.0), (0.5, 1.0), (2.4, 1.0), (2.8, 0.0)]),
                ("MouthForm", &[(0.0, 0.0), (0.5, 1.0), (2.6, 1.0), (3.0, 0.2)]),
                (
                    "MouthOpenY",
                    &[
                        (0.0, 0.0),
                        (0.7, 0.7),
                        (1.0, 0.2),
                        (1.3, 0.6),
                        (1.7, 0.1),
                        (2.0, 0.0),
                    ],
                ),
                ("Cheek", &[(0.0, 0.0), (0.6, 0.5), (2.6, 0.5), (3.2, 0.0)]),
            ],
        )?,
        motion(
            rig,
            "Happy",
            2.0,
            true,
            &[
                (
                    "AngleZ",
                    &[(0.0, -6.0), (0.5, 6.0), (1.0, -6.0), (1.5, 6.0), (2.0, -6.0)],
                ),
                (
                    "BodyAngleZ",
                    &[(0.0, -3.0), (0.5, 3.0), (1.0, -3.0), (1.5, 3.0), (2.0, -3.0)],
                ),
                ("EyeLOpen", &[(0.0, 0.0), (2.0, 0.0)]),
                ("EyeROpen", &[(0.0, 0.0), (2.0, 0.0)]),
                ("EyeLSmile", &[(0.0, 1.0), (2.0, 1.0)]),
                ("EyeRSmile", &[(0.0, 1.0), (2.0, 1.0)]),
                ("MouthForm", &[(0.0, 1.0), (2.0, 1.0)]),
                (
                    "MouthOpenY",
                    &[(0.0, 0.5), (0.5, 0.8), (1.0, 0.5), (1.5, 0.8), (2.0, 0.5)],
                ),
                ("Cheek", &[(0.0, 0.5), (2.0, 0.5)]),
            ],
        )?,
        motion(
            rig,
            "Surprised",
            2.6,
            false,
            &[
                ("AngleY", &[(0.0, 0.0), (0.2, 10.0), (1.6, 8.0), (2.4, 0.0)]),
                ("AngleX", &[(0.0, 0.0), (0.2, -4.0), (1.6, -3.0), (2.4, 0.0)]),
                ("BodyAngleY", &[(0.0, 0.0), (0.25, 4.0), (2.4, 0.0)]),
                ("MouthForm", &[(0.0, 0.0), (0.2, -1.0), (1.8, -1.0), (2.4, 0.0)]),
                ("MouthOpenY", &[(0.0, 0.0), (0.2, 0.9), (1.6, 0.7), (2.4, 0.0)]),
                ("EyeBallY", &[(0.0, 0.0), (0.2, 0.3), (2.0, 0.0)]),
            ],
        )?,
        motion(
            rig,
            "Shy",
            3.0,
            false,
            &[
                ("AngleX", &[(0.0, 0.0), (0.8, -14.0), (2.3, -12.0), (3.0, 0.0)]),
                ("AngleY", &[(0.0, 0.0), (0.8, -10.0), (2.3, -9.0), (3.0, 0.0)]),
                ("AngleZ", &[(0.0, 0.0), (0.8, -6.0), (2.3, -5.0), (3.0, 0.0)]),
                ("EyeBallX", &[(0.0, 0.0), (0.6, 0.7), (2.4, 0.6), (3.0, 0.0)]),
                ("EyeBallY", &[(0.0, 0.0), (0.6, -0.5), (2.4, -0.5), (3.0, 0.0)]),
                ("Cheek", &[(0.0, 0.0), (0.7, 1.0), (2.4, 1.0), (3.0, 0.0)]),
                ("MouthForm", &[(0.0, 0.0), (0.7, 0.3), (2.4, 0.3), (3.0, 0.0)]),
            ],
        )?,
        motion(
            rig,
            "Wink",
            1.6,
            false,
            &[
                ("AngleZ", &[(0.0, 0.0), (0.3, -7.0), (1.2, -7.0), (1.6, 0.0)]),
                ("EyeLOpen", &[(0.0, 1.0), (0.25, 0.0), (1.1, 0.0), (1.4, 1.0)]),
                ("EyeLSmile", &[(0.0, 0.0), (0.25, 1.0), (1.1, 1.0), (1.4, 0.0)]),
                ("MouthForm", &[(0.0, 0.0), (0.3, 1.0), (1.2, 1.0), (1.6, 0.0)]),
            ],
        )?,
    ];
    rig.motions.extend(motions);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_document::rig::Evaluator;

    fn pose_with(doc: &Document, values: &[(&str, f32)]) -> aether_document::rig::RigPose {
        let mut rig = doc.rig.clone();
        rig.drivers.clear();
        for (name, v) in values {
            let id = param(&rig, name).unwrap();
            rig.set_value(id, *v);
        }
        Evaluator::new(&rig).pose()
    }

    /// The posed vertex of `layer` nearest to rest point `at`, and how far it
    /// moved.
    fn moved(doc: &Document, pose: &aether_document::rig::RigPose, layer: LayerId, at: Vec2) -> Vec2 {
        let mesh = doc.rig.mesh(layer).unwrap();
        let (i, _) = mesh
            .vertices
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)))
            .unwrap();
        pose.meshes[&layer].positions[i] - mesh.vertices[i]
    }

    #[test]
    fn luna_is_rigged_and_rests_as_drawn() {
        let (doc, parts) = build().expect("rigs");
        let rig = &doc.rig;
        assert_eq!(rig.meshes.len(), 17, "every layer is meshed");
        for name in [
            "AngleX",
            "AngleY",
            "AngleZ",
            "EyeLOpen",
            "EyeROpen",
            "EyeLSmile",
            "EyeRSmile",
            "EyeBallX",
            "EyeBallY",
            "MouthForm",
            "MouthOpenY",
            "Cheek",
            "BodyAngleX",
            "BodyAngleY",
            "BodyAngleZ",
            "Breath",
            "HairFront",
            "HairSide",
            "HairBack",
        ] {
            let id = param(rig, name).unwrap();
            assert!(!rig.nodes_using(id).is_empty(), "{name} moves something");
        }
        assert_eq!(rig.physics.len(), 3);
        assert_eq!(rig.expressions.len(), 6);
        assert_eq!(rig.motions.len(), 6);

        // At rest every layer that shows is exactly as drawn, and only the
        // rest mouth and the open eyes show. (Hidden mouth shapes wait
        // squashed or stretched toward where they fade in.)
        let pose = pose_with(&doc, &[]);
        for mesh in &rig.meshes {
            let posed = &pose.meshes[&mesh.layer];
            if posed.opacity == 0.0 {
                continue;
            }
            let worst = posed
                .positions
                .iter()
                .zip(&mesh.vertices)
                .map(|(a, b)| a.distance(*b))
                .fold(0.0f32, f32::max);
            assert!(worst < 1e-3, "{} moved {worst} px at rest", mesh.name);
        }
        let opacity = |l: LayerId| pose.meshes[&l].opacity;
        assert_eq!(opacity(parts.mouths[0]), 1.0);
        assert!(parts.mouths[1..].iter().all(|m| opacity(*m) == 0.0));
        assert!(parts.eye_closed.iter().all(|e| opacity(*e) == 0.0));
        assert_eq!(opacity(parts.cheek), 0.0);
    }

    #[test]
    fn luna_moves_where_she_should_and_nowhere_else() {
        let (doc, parts) = build().expect("rigs");
        let body = parts.illustration;
        let shoe = Vec2::new(512.0, 1500.0);
        let eye = Vec2::new(484.0, 197.0);
        let chest = Vec2::new(512.0, 460.0);
        let hair_end = Vec2::new(90.0, 900.0);
        let hand = Vec2::new(157.0, 667.0);

        // Turning the head moves the face, not the chest or the feet.
        let pose = pose_with(&doc, &[("AngleX", 30.0)]);
        assert!(moved(&doc, &pose, body, eye).x > 10.0);
        assert!(
            moved(&doc, &pose, parts.iris[1], eye).x > 10.0,
            "the face layers go with it"
        );
        assert!(moved(&doc, &pose, body, chest).length() < 0.5);
        assert!(moved(&doc, &pose, body, shoe).length() < 0.5);

        // Hair sways around a still hand.
        let pose = pose_with(&doc, &[("HairSide", 1.0)]);
        assert!(moved(&doc, &pose, body, hair_end).x > 15.0);
        assert!(moved(&doc, &pose, body, hand).length() < 6.0);
        assert!(moved(&doc, &pose, body, chest).length() < 0.5);

        // Leaning moves the head but the feet stay planted.
        let pose = pose_with(&doc, &[("BodyAngleZ", 10.0)]);
        assert!(moved(&doc, &pose, body, eye).length() > 30.0);
        assert!(moved(&doc, &pose, body, shoe).length() < 1.0);

        // Blinking hides the open eye and shows the closed line; smiling
        // bends that line upward.
        let pose = pose_with(&doc, &[("EyeLOpen", 0.0)]);
        assert_eq!(pose.meshes[&parts.eye_white[0]].opacity, 0.0);
        assert_eq!(pose.meshes[&parts.eye_closed[0]].opacity, 1.0);
        let top = |pose: &aether_document::rig::RigPose| {
            pose.meshes[&parts.eye_closed[0]]
                .positions
                .iter()
                .map(|p| p.y)
                .fold(f32::INFINITY, f32::min)
        };
        let happy = pose_with(&doc, &[("EyeLOpen", 0.0), ("EyeLSmile", 1.0)]);
        assert!(top(&happy) < top(&pose) - 3.0);

        // Every mouth shape takes its turn.
        for (values, shape) in [
            (vec![("MouthForm", -1.0)], 1),
            (vec![("MouthOpenY", 0.5)], 2),
            (vec![("MouthOpenY", 1.0)], 3),
            (vec![("MouthForm", -1.0), ("MouthOpenY", 1.0)], 4),
        ] {
            let pose = pose_with(&doc, &values);
            for (i, m) in parts.mouths.iter().enumerate() {
                let expected = if i == shape { 1.0 } else { 0.0 };
                assert_eq!(pose.meshes[m].opacity, expected, "{values:?}: mouth {i}");
            }
        }
    }
}
