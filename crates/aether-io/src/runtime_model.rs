//! Exporting runtime models for `aether-player`.
//!
//! The export turns a document into what a game or web page needs and
//! nothing more: one textured mesh ("part") per drawn layer, its pixels
//! packed into texture atlases, the draw order, and the rig. Layers without
//! a mesh become rectangles, so the player draws everything the same way.
//!
//! Editor-only features are resolved during export:
//!
//! * layer masks and effects are baked into the part's pixels;
//! * group opacity, blend mode and transform are folded into the parts they
//!   contain, as Live2D does for part opacity — exact unless children of a
//!   translucent group overlap each other;
//! * blend modes map onto the four every GPU does natively (normal,
//!   multiply, screen, add), and the nearest match is reported;
//! * adjustment and plugin layers are left out, with a warning.
//!
//! Anything approximated is listed in [`ModelExport::warnings`].

use aether_core::blend::BlendMode;
use aether_core::color::Rgba8;
use aether_core::math::{Rect, Transform2D, Vec2};
use aether_core::{AetherError, LayerId, Result};
use aether_document::layer::{Layer, LayerContent};
use aether_document::Document;
use aether_player::model::{BlendKind, Model, Node, Part, Texture};
use aether_raster::mesh::mesh_coverage;
use aether_raster::{Mask, Pixmap};
use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

/// Knobs for [`export_model`].
#[derive(Clone, Debug, PartialEq)]
pub struct ModelExportOptions {
    /// Largest texture page, pixels. Parts bigger than this get a page of
    /// their own.
    pub max_texture_size: u32,
    /// Transparent gutter around every part in the atlas, so filtering never
    /// bleeds a neighbour in.
    pub padding: u32,
}

impl Default for ModelExportOptions {
    fn default() -> Self {
        Self {
            max_texture_size: 4096,
            padding: 2,
        }
    }
}

/// An exported model and its texture pages.
#[derive(Clone, Debug)]
pub struct ModelExport {
    /// `model.json` contents.
    pub model: Model,
    /// Texture pages, in `model.textures` order.
    pub textures: Vec<Pixmap>,
    /// Everything that could not be carried over exactly.
    pub warnings: Vec<String>,
}

/// File name of `model.json` inside an export directory.
pub const MODEL_FILE: &str = "model.json";

/// Build a runtime model from a document.
pub fn export_model(doc: &Document, options: &ModelExportOptions) -> ModelExport {
    let mut walker = Walker {
        doc,
        parts: Vec::new(),
        pixels: Vec::new(),
        warnings: Vec::new(),
    };
    let tree = walker.walk(None, &Inherited::root());

    let mut model = Model::new(doc.name.clone(), doc.width, doc.height);
    model.rig = doc.rig.clone();
    model.rig.reset_values();
    model.rig.dynamics = Default::default();

    let sizes: Vec<(u32, u32)> = walker
        .pixels
        .iter()
        .map(|p| (p.pixels.width(), p.pixels.height()))
        .collect();
    let (placements, pages) = pack(&sizes, options.max_texture_size.max(64), options.padding);
    let mut textures: Vec<Pixmap> = pages.iter().map(|&(w, h)| Pixmap::new(w, h)).collect();
    for (i, (part, cut)) in walker.parts.iter_mut().zip(&walker.pixels).enumerate() {
        let place = placements[i];
        textures[place.page].paste_rect(&cut.pixels, place.x as i32, place.y as i32);
        let (w, h) = pages[place.page];
        part.texture = place.page as u32;
        // Texture pixel = rest position - crop origin + atlas position.
        let offset = Vec2::new(place.x as f32 - cut.origin.x, place.y as f32 - cut.origin.y);
        part.uvs = match cut.sample {
            Some(texel) => vec![
                Vec2::new(
                    (place.x as f32 + texel.x) / w as f32,
                    (place.y as f32 + texel.y) / h as f32
                );
                part.vertices.len()
            ],
            None => part
                .vertices
                .iter()
                .map(|&v| Vec2::new((v.x + offset.x) / w as f32, (v.y + offset.y) / h as f32))
                .collect(),
        };
    }
    model.textures = (0..pages.len())
        .map(|i| Texture {
            file: format!("texture_{i}.png"),
            width: pages[i].0,
            height: pages[i].1,
        })
        .collect();
    model.parts = walker.parts;
    model.tree = tree;
    ModelExport {
        model,
        textures,
        warnings: walker.warnings,
    }
}

/// Write `model.json` and the texture pages into `dir` (created if
/// needed). Returns the path of `model.json`.
pub fn save_model(export: &ModelExport, dir: impl AsRef<Path>) -> Result<PathBuf> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir)?;
    for (texture, pixels) in export.model.textures.iter().zip(&export.textures) {
        crate::image_io::save_png(pixels, dir.join(&texture.file))?;
    }
    let path = dir.join(MODEL_FILE);
    fs::write(&path, export.model.to_json())?;
    Ok(path)
}

/// Export a document straight into a directory.
pub fn export_model_to_dir(
    doc: &Document,
    dir: impl AsRef<Path>,
    options: &ModelExportOptions,
) -> Result<ModelExport> {
    let export = export_model(doc, options);
    save_model(&export, dir)?;
    Ok(export)
}

/// Read an exported model and decode its texture pages.
pub fn load_model(dir: impl AsRef<Path>) -> Result<(Model, Vec<Pixmap>)> {
    let dir = dir.as_ref();
    let text = fs::read_to_string(dir.join(MODEL_FILE))?;
    let model = Model::from_json(&text).map_err(AetherError::serialization)?;
    let textures = model
        .textures
        .iter()
        .map(|t| crate::image_io::load_image(dir.join(&t.file)))
        .collect::<Result<Vec<_>>>()?;
    Ok((model, textures))
}

/// Pixels cut out for one part, before packing.
struct Cut {
    pixels: Pixmap,
    /// Document position of `pixels`' top-left corner.
    origin: Vec2,
    /// For solid fills: every vertex samples this texel of the cut.
    sample: Option<Vec2>,
}

/// What a layer inherits from the groups around it.
#[derive(Clone)]
struct Inherited<'a> {
    opacity: f32,
    blend: BlendMode,
    transform: Transform2D,
    masks: Vec<&'a Mask>,
}

impl Inherited<'_> {
    fn root() -> Self {
        Self {
            opacity: 1.0,
            blend: BlendMode::Normal,
            transform: Transform2D::IDENTITY,
            masks: Vec::new(),
        }
    }
}

struct Walker<'a> {
    doc: &'a Document,
    parts: Vec<Part>,
    pixels: Vec<Cut>,
    warnings: Vec<String>,
}

fn hidden(layer: &Layer) -> bool {
    !layer.visible || layer.opacity <= 0.0
}

impl<'a> Walker<'a> {
    /// Draw nodes for the children of `parent`, mirroring the compositor's
    /// grouping of base layers with the clipping layers above them.
    fn walk(&mut self, parent: Option<LayerId>, inherited: &Inherited<'a>) -> Vec<Node> {
        let doc = self.doc;
        let mut units: Vec<(u32, &'a Layer, Vec<&'a Layer>)> = Vec::new();
        for (index, id) in doc.layers.children_of(parent).iter().enumerate() {
            let Some(layer) = doc.layers.get(*id) else {
                continue;
            };
            if layer.clipping {
                if let Some(unit) = units.last_mut() {
                    unit.2.push(layer);
                }
                continue;
            }
            units.push((index as u32, layer, Vec::new()));
        }

        let mut nodes = Vec::new();
        for (index, base, clipped) in units {
            if hidden(base) {
                continue;
            }
            match &base.content {
                LayerContent::Group(group) => {
                    if !clipped.is_empty() {
                        self.warn(format!(
                            "layers clipped to group \"{}\" are left out (clipping to a group is editor-only)",
                            base.name
                        ));
                    }
                    if base.has_effects() {
                        self.warn(format!("effects on group \"{}\" are left out", base.name));
                    }
                    let mut inner = inherited.clone();
                    inner.opacity *= base.opacity;
                    if base.blend_mode != BlendMode::Normal {
                        inner.blend = base.blend_mode;
                    }
                    inner.transform = base.transform.then(&inherited.transform);
                    if let Some(mask) = base.active_mask() {
                        inner.masks.push(mask);
                    }
                    let isolated =
                        group.isolate || base.opacity < 1.0 || base.blend_mode != BlendMode::Normal;
                    let children = self.walk(Some(base.id), &inner);
                    if isolated && children.len() > 1 {
                        self.warn(format!(
                            "group \"{}\" is flattened: its opacity and blend apply to each layer inside",
                            base.name
                        ));
                    }
                    if !children.is_empty() {
                        nodes.push(Node::Group {
                            index,
                            name: base.name.clone(),
                            children,
                        });
                    }
                }
                LayerContent::Raster(_) | LayerContent::Fill(_) => {
                    let Some(part) = self.part(base, inherited, 1.0) else {
                        continue;
                    };
                    let clipped = clipped
                        .into_iter()
                        .filter(|c| !hidden(c))
                        .filter_map(|c| match c.content {
                            LayerContent::Raster(_) | LayerContent::Fill(_) => {
                                // The clipping group takes the base's opacity.
                                self.part(c, inherited, base.opacity)
                            }
                            _ => {
                                self.warn(format!(
                                    "clipping layer \"{}\" is left out (unsupported kind)",
                                    c.name
                                ));
                                None
                            }
                        })
                        .collect();
                    nodes.push(Node::Part { index, part, clipped });
                }
                LayerContent::Adjustment(_) => {
                    self.warn(format!("adjustment layer \"{}\" is left out", base.name));
                }
                LayerContent::Live2D(_) => {
                    self.warn(format!(
                        "Live2D model \"{}\" is left out (export it as a Live2D model instead)",
                        base.name
                    ));
                }
                LayerContent::Custom { kind, .. } => {
                    self.warn(format!("plugin layer \"{}\" ({kind}) is left out", base.name));
                }
            }
        }
        nodes
    }

    fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }

    /// Make a part for a raster or fill layer. `None` when it draws nothing.
    fn part(&mut self, layer: &Layer, inherited: &Inherited<'a>, extra_opacity: f32) -> Option<u32> {
        let doc = self.doc;
        let mesh = doc.rig.mesh(layer.id);
        let mut masks: Vec<&Mask> = inherited.masks.clone();
        masks.extend(layer.active_mask());

        let solid = match &layer.content {
            LayerContent::Fill(fill) if masks.is_empty() && !layer.has_effects() => Some(fill.color),
            _ => None,
        };
        let (vertices, triangles, cut) = if let Some(color) = solid {
            // A solid fill needs one colour, not a canvas-sized texture.
            let block = Pixmap::filled(4, 4, color);
            let rect = doc.bounds().to_rect();
            let cut = Cut {
                pixels: block,
                origin: Vec2::ZERO,
                sample: Some(Vec2::new(2.0, 2.0)),
            };
            (corners(rect), quad_triangles(), cut)
        } else {
            let pixels = self.layer_pixels(layer, &masks)?;
            match mesh {
                Some(mesh) => {
                    self.check_coverage(layer, &pixels, &mesh.vertices, &mesh.triangles);
                    let bounds = aether_document::rig::mesh::bounds_of(&mesh.vertices)
                        .to_irect_outer()
                        .union(&pixels.opaque_bounds())
                        .expanded(1);
                    let cut = Cut {
                        pixels: pixels.copy_rect(bounds),
                        origin: Vec2::new(bounds.x as f32, bounds.y as f32),
                        sample: None,
                    };
                    (mesh.vertices.clone(), mesh.triangles.clone(), cut)
                }
                None => {
                    let bounds = pixels.opaque_bounds();
                    if bounds.is_empty() {
                        return None;
                    }
                    let cut = Cut {
                        pixels: pixels.copy_rect(bounds),
                        origin: Vec2::new(bounds.x as f32, bounds.y as f32),
                        sample: None,
                    };
                    (corners(bounds.to_rect()), quad_triangles(), cut)
                }
            }
        };

        let blend = if layer.blend_mode != BlendMode::Normal {
            layer.blend_mode
        } else {
            inherited.blend
        };
        let transform = layer.transform.then(&inherited.transform);
        let blend = self.blend_kind(blend, &layer.name);
        let index = self.parts.len() as u32;
        self.parts.push(Part {
            layer: layer.id,
            name: layer.name.clone(),
            texture: 0,
            vertices,
            uvs: Vec::new(),
            triangles,
            opacity: (layer.opacity * inherited.opacity * extra_opacity).clamp(0.0, 1.0),
            blend,
            transform: (!transform.is_identity()).then_some(transform),
        });
        self.pixels.push(cut);
        Some(index)
    }

    /// The layer's pixels with effects and masks baked in.
    fn layer_pixels(&mut self, layer: &Layer, masks: &[&Mask]) -> Option<Pixmap> {
        let doc = self.doc;
        let mut pixels: Cow<Pixmap> = match &layer.content {
            LayerContent::Raster(raster) => Cow::Borrowed(&raster.pixmap),
            LayerContent::Fill(fill) => Cow::Owned(Pixmap::filled(doc.width, doc.height, fill.color)),
            _ => return None,
        };
        if layer.has_effects() {
            pixels = Cow::Owned(aether_raster::effect::apply_stack(&pixels, &layer.effects));
        }
        if !masks.is_empty() {
            let mut owned = pixels.into_owned();
            for y in 0..owned.height() as i32 {
                for x in 0..owned.width() as i32 {
                    let px = owned.get(x, y);
                    if px.a == 0 {
                        continue;
                    }
                    let coverage = masks.iter().fold(1.0, |acc, m| acc * m.get(x, y) as f32 / 255.0);
                    let a = (px.a as f32 * coverage).round() as u8;
                    owned.set(
                        x,
                        y,
                        if a == 0 {
                            Rgba8::TRANSPARENT
                        } else {
                            Rgba8::new(px.r, px.g, px.b, a)
                        },
                    );
                }
            }
            pixels = Cow::Owned(owned);
        }
        Some(pixels.into_owned())
    }

    /// Warn when painted pixels fall outside a layer's mesh: the editor shows
    /// them at rest, the player never does.
    fn check_coverage(&mut self, layer: &Layer, pixels: &Pixmap, vertices: &[Vec2], triangles: &[[u32; 3]]) {
        let coverage = mesh_coverage(pixels.width(), pixels.height(), vertices, triangles);
        let bounds = pixels.opaque_bounds();
        let mut missed = 0usize;
        for y in bounds.y..bounds.bottom() {
            for x in bounds.x..bounds.right() {
                if pixels.get(x, y).a > 0 && coverage.get(x, y) == 0 {
                    missed += 1;
                }
            }
        }
        if missed > 0 {
            self.warn(format!(
                "{missed} painted pixels of \"{}\" lie outside its mesh and are not drawn (re-mesh the layer)",
                layer.name
            ));
        }
    }

    fn blend_kind(&mut self, mode: BlendMode, name: &str) -> BlendKind {
        let (kind, exact) = match mode {
            BlendMode::Normal => (BlendKind::Normal, true),
            BlendMode::Multiply => (BlendKind::Multiply, true),
            BlendMode::Screen => (BlendKind::Screen, true),
            BlendMode::Add => (BlendKind::Add, true),
            BlendMode::Darken | BlendMode::ColorBurn | BlendMode::LinearBurn => (BlendKind::Multiply, false),
            BlendMode::Lighten | BlendMode::ColorDodge => (BlendKind::Screen, false),
            _ => (BlendKind::Normal, false),
        };
        if !exact {
            self.warn(format!(
                "\"{name}\" uses {mode:?}, drawn as {kind:?} by the player"
            ));
        }
        kind
    }
}

fn corners(r: Rect) -> Vec<Vec2> {
    vec![
        r.min,
        Vec2::new(r.max.x, r.min.y),
        r.max,
        Vec2::new(r.min.x, r.max.y),
    ]
}

fn quad_triangles() -> Vec<[u32; 3]> {
    vec![[0, 1, 2], [0, 2, 3]]
}

/// Where one item landed in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placement {
    page: usize,
    x: u32,
    y: u32,
}

/// Shelf-pack `sizes` into pages no larger than `max` (oversized items get a
/// page of their own). Returns each item's placement and each page's size.
fn pack(sizes: &[(u32, u32)], max: u32, padding: u32) -> (Vec<Placement>, Vec<(u32, u32)>) {
    struct Shelf {
        y: u32,
        height: u32,
        x: u32,
    }
    struct Page {
        shelves: Vec<Shelf>,
        width: u32,
        height: u32,
    }
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(sizes[i].1), std::cmp::Reverse(sizes[i].0), i));

    // Aim for square pages: shelves as wide as the square that would hold
    // everything, but never narrower than the widest item.
    let padded = |i: usize| (sizes[i].0 + padding * 2, sizes[i].1 + padding * 2);
    let fits = |i: usize| padded(i).0 <= max && padded(i).1 <= max;
    let area: f64 = (0..sizes.len())
        .filter(|&i| fits(i))
        .map(|i| padded(i).0 as f64 * padded(i).1 as f64)
        .sum();
    let widest = (0..sizes.len())
        .filter(|&i| fits(i))
        .map(|i| padded(i).0)
        .max()
        .unwrap_or(0);
    let shelf_limit = ((area * 1.1).sqrt().ceil() as u32).max(widest).min(max);

    let mut pages: Vec<Page> = Vec::new();
    let mut placements = vec![Placement { page: 0, x: 0, y: 0 }; sizes.len()];
    for i in order {
        let (w, h) = padded(i);
        let mut placed = None;
        if fits(i) {
            'pages: for (p, page) in pages.iter_mut().enumerate() {
                for shelf in &mut page.shelves {
                    if h <= shelf.height && shelf.x + w <= shelf_limit {
                        placed = Some((p, shelf.x, shelf.y));
                        shelf.x += w;
                        break 'pages;
                    }
                }
                let top = page.shelves.last().map(|s| s.y + s.height).unwrap_or(0);
                if top + h <= max {
                    page.shelves.push(Shelf {
                        y: top,
                        height: h,
                        x: w,
                    });
                    placed = Some((p, 0, top));
                    break;
                }
            }
        }
        let (p, x, y) = placed.unwrap_or_else(|| {
            pages.push(Page {
                shelves: vec![Shelf {
                    y: 0,
                    height: h,
                    x: w,
                }],
                width: 0,
                height: 0,
            });
            (pages.len() - 1, 0, 0)
        });
        let page = &mut pages[p];
        page.width = page.width.max(x + w);
        page.height = page.height.max(y + h);
        placements[i] = Placement {
            page: p,
            x: x + padding,
            y: y + padding,
        };
    }
    // Round page sizes up to a multiple of four, which some GPUs prefer.
    let sizes = pages
        .iter()
        .map(|p| {
            (
                p.width.max(1).next_multiple_of(4),
                p.height.max(1).next_multiple_of(4),
            )
        })
        .collect();
    (placements, sizes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::math::IRect;
    use aether_document::layer::{GroupContent, Layer, LayerContent};
    use aether_document::rig::{ArtMesh, Parameter, RigNode};
    use aether_player::{cpu, Player};
    use aether_render::{Compositor, RenderOptions};

    #[test]
    fn packing_never_overlaps_and_respects_the_limit() {
        let sizes = [(30, 10), (5, 40), (64, 64), (100, 3), (7, 7), (200, 20)];
        let (placements, pages) = pack(&sizes, 128, 2);
        for (i, a) in placements.iter().enumerate() {
            let (w, h) = sizes[i];
            let (pw, ph) = pages[a.page];
            if w + 4 <= 128 {
                assert!(a.x + w + 2 <= pw && a.y + h + 2 <= ph && pw <= 128 && ph <= 128);
            }
            for (j, b) in placements.iter().enumerate().skip(i + 1) {
                if a.page != b.page {
                    continue;
                }
                let (bw, bh) = sizes[j];
                let apart =
                    a.x + w + 2 <= b.x || b.x + bw + 2 <= a.x || a.y + h + 2 <= b.y || b.y + bh + 2 <= a.y;
                assert!(apart, "{i} and {j} overlap");
            }
        }
        // The 200-wide strip cannot fit and gets its own page.
        assert!(pages.iter().any(|&(w, _)| w >= 204));
    }

    #[test]
    fn pages_come_out_roughly_square() {
        let (_, pages) = pack(&[(50, 50); 16], 4096, 2);
        assert_eq!(pages.len(), 1);
        let (w, h) = pages[0];
        assert!(w <= 2 * h && h <= 2 * w, "{w}x{h}");
    }

    fn paint(doc: &mut Document, id: LayerId, rect: IRect, color: Rgba8) {
        if let Some(LayerContent::Raster(r)) = doc.layers.get_mut(id).map(|l| &mut l.content) {
            r.pixmap.fill_rect(rect, color);
        }
    }

    fn add_raster(doc: &mut Document, parent: Option<LayerId>, name: &str) -> LayerId {
        let id = doc.ids.layer();
        let layer = Layer::raster(id, name, doc.width, doc.height);
        doc.layers.insert(layer, parent, usize::MAX).expect("insert")
    }

    /// A document exercising every export path: a background, a rigged arm
    /// with keyed movement, opacity, tint and draw order, a layer clipped to
    /// it, a translucent group, a multiply layer, a masked layer and a hidden
    /// layer.
    fn scene() -> (Document, ParameterIdFor) {
        let mut doc = Document::empty(96, 64, "scene");
        let background = add_raster(&mut doc, None, "Background");
        paint(
            &mut doc,
            background,
            IRect::new(0, 0, 96, 64),
            Rgba8::new(240, 236, 220, 255),
        );
        let arm = add_raster(&mut doc, None, "Arm");
        paint(
            &mut doc,
            arm,
            IRect::new(10, 20, 30, 12),
            Rgba8::new(220, 60, 60, 255),
        );
        let stripe = add_raster(&mut doc, None, "Stripe");
        paint(
            &mut doc,
            stripe,
            IRect::new(0, 24, 96, 4),
            Rgba8::new(30, 30, 200, 255),
        );
        doc.layers.get_mut(stripe).unwrap().clipping = true;

        let group_id = doc.ids.layer();
        let mut group = Layer::group(group_id, "Group");
        group.opacity = 0.5;
        if let LayerContent::Group(GroupContent { isolate, .. }) = &mut group.content {
            *isolate = false;
        }
        doc.layers.push_top(group).unwrap();
        let inner = add_raster(&mut doc, Some(group_id), "Inner");
        paint(
            &mut doc,
            inner,
            IRect::new(60, 8, 20, 20),
            Rgba8::new(20, 160, 60, 255),
        );

        let shade = add_raster(&mut doc, None, "Shade");
        paint(
            &mut doc,
            shade,
            IRect::new(50, 30, 40, 30),
            Rgba8::new(128, 128, 255, 255),
        );
        doc.layers.get_mut(shade).unwrap().blend_mode = BlendMode::Multiply;

        let masked = add_raster(&mut doc, None, "Masked");
        paint(
            &mut doc,
            masked,
            IRect::new(4, 40, 30, 20),
            Rgba8::new(250, 200, 0, 255),
        );
        let mut mask = Mask::new(96, 64);
        for y in 40..60 {
            for x in 4..19 {
                mask.set(x, y, 255);
            }
        }
        let l = doc.layers.get_mut(masked).unwrap();
        l.mask = Some(mask);
        l.mask_enabled = true;

        let hidden = add_raster(&mut doc, None, "Hidden");
        paint(
            &mut doc,
            hidden,
            IRect::new(0, 0, 96, 64),
            Rgba8::new(0, 0, 0, 255),
        );
        doc.layers.get_mut(hidden).unwrap().visible = false;

        // Rig the arm: a mesh that slides right, fades, tints and rises
        // above the multiply layer at the top of its parameter.
        let ids = &doc.ids;
        let param = doc
            .rig
            .add_parameter(Parameter::new(ids.parameter(), "Swing", 0.0, 1.0, 0.0))
            .unwrap();
        let rect = Rect::from_min_size(Vec2::new(8.0, 18.0), Vec2::new(34.0, 16.0));
        let mut vertices = Vec::new();
        for j in 0..=2 {
            for i in 0..=4 {
                vertices.push(Vec2::new(
                    rect.min.x + rect.width() * i as f32 / 4.0,
                    rect.min.y + rect.height() * j as f32 / 2.0,
                ));
            }
        }
        let mut triangles = Vec::new();
        for j in 0..2u32 {
            for i in 0..4u32 {
                let a = j * 5 + i;
                triangles.push([a, a + 1, a + 6]);
                triangles.push([a, a + 6, a + 5]);
            }
        }
        doc.rig.set_mesh(ArtMesh::new(arm, vertices, triangles));
        doc.rig
            .bind_parameter(RigNode::Mesh(arm), param, &[0.0, 1.0])
            .unwrap();
        let form = &mut doc.rig.mesh_mut(arm).unwrap().keyforms.forms[1];
        for (k, o) in form.offsets.iter_mut().enumerate() {
            *o = Vec2::new(24.0, if k % 5 == 4 { 10.0 } else { 3.0 });
        }
        form.opacity = 0.8;
        form.multiply = [1.0, 0.7, 0.7];
        form.draw_order = 4.0;
        (doc, param)
    }

    type ParameterIdFor = aether_core::ParameterId;

    fn compare(a: &Pixmap, b: &Pixmap) -> (u8, f64) {
        let mut worst = 0u8;
        let mut total = 0u64;
        for (x, y) in a.data().iter().zip(b.data()) {
            let d = x.abs_diff(*y);
            worst = worst.max(d);
            total += d as u64;
        }
        (worst, total as f64 / a.data().len() as f64)
    }

    #[test]
    fn exported_models_draw_like_the_editor() {
        let (mut doc, param) = scene();
        let export = export_model(&doc, &ModelExportOptions::default());
        // Hidden layer gone, stripe clipped to the arm, group kept.
        let names: Vec<&str> = export.model.parts.iter().map(|p| p.name.as_str()).collect();
        assert!(!names.contains(&"Hidden"), "{names:?}");
        assert_eq!(names.len(), 6, "{names:?}");
        assert!(export
            .model
            .tree
            .iter()
            .any(|n| matches!(n, Node::Part { clipped, .. } if clipped.len() == 1)));
        assert!(export.warnings.is_empty(), "{:?}", export.warnings);

        let mut player = Player::new(export.model.clone()).expect("valid model");
        let options = RenderOptions {
            include_background: false,
            ..Default::default()
        };
        for value in [0.0, 0.35, 1.0] {
            doc.rig.set_value(param, value);
            let editor = Compositor::new().render_with(&doc, &options);
            player.set_parameter(player.parameter_index("Swing").unwrap(), value);
            player.update();
            let runtime = cpu::render(&player, &export.textures);
            let (worst, mean) = compare(&editor, &runtime);
            assert!(
                worst <= 2 && mean < 0.05,
                "at {value}: worst {worst}, mean {mean}"
            );
        }
    }

    #[test]
    fn models_survive_a_trip_through_files() {
        let (doc, _) = scene();
        let dir = tempfile::tempdir().unwrap();
        let export = export_model_to_dir(&doc, dir.path(), &ModelExportOptions::default()).unwrap();
        assert!(dir.path().join("model.json").exists());
        assert!(dir.path().join("texture_0.png").exists());
        let (model, textures) = load_model(dir.path()).unwrap();
        assert_eq!(model.parts, export.model.parts);
        assert_eq!(textures.len(), export.textures.len());
        assert_eq!(textures[0].data(), export.textures[0].data());
    }

    #[test]
    fn approximations_are_reported() {
        let (mut doc, _) = scene();
        let id = doc.layers.iter().find(|l| l.name == "Shade").unwrap().id;
        doc.layers.get_mut(id).unwrap().blend_mode = BlendMode::Overlay;
        let adjustment = doc.ids.layer();
        doc.layers
            .push_top(Layer::adjustment(
                adjustment,
                "Levels",
                aether_raster::adjust::Adjustment::Invert,
            ))
            .unwrap();
        let export = export_model(&doc, &ModelExportOptions::default());
        let text = export.warnings.join("\n");
        assert!(text.contains("Overlay"), "{text}");
        assert!(text.contains("Levels"), "{text}");
    }

    #[test]
    fn painted_pixels_outside_a_mesh_are_reported() {
        let (mut doc, _) = scene();
        let arm = doc.layers.iter().find(|l| l.name == "Arm").unwrap().id;
        paint(&mut doc, arm, IRect::new(80, 2, 4, 4), Rgba8::new(0, 0, 0, 255));
        let export = export_model(&doc, &ModelExportOptions::default());
        assert!(
            export.warnings.iter().any(|w| w.contains("16 painted pixels")),
            "{:?}",
            export.warnings
        );
    }
}
