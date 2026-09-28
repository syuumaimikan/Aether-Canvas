//! The `.aether` project container.
//!
//! A project file is an ordinary ZIP archive:
//!
//! ```text
//! project.aether
//! ├── project.json      manifest: schema version, document settings, layer tree
//! ├── layers/<id>.png   one PNG per raster layer
//! ├── masks/<id>.png    one grayscale PNG per layer mask
//! ├── selection.png     the saved selection, when there is one
//! ├── rig.json          rigging and animation, when the document has any
//! └── thumbnail.png     512px preview for file browsers
//! ```
//!
//! Two decisions are deliberate. **The manifest is JSON**, so a corrupted or
//! future-version file can be inspected and repaired by hand rather than being
//! lost. **Pixels are PNG**, so layers can be recovered with any image tool
//! even if this application is gone.
//!
//! The manifest carries [`SCHEMA_VERSION`]; loading runs the JSON through
//! [`migrate`] before deserialising, which is where upgrades from older
//! versions live.

use crate::image_io::{decode_image, decode_mask_png, encode_mask_png, encode_pixmap_png, ExportSettings};
use aether_core::blend::BlendMode;
use aether_core::color::{ColorModel, Rgba8};
use aether_core::id::IdGenerator;
use aether_core::math::Transform2D;
use aether_core::{AetherError, LayerId, Result};
use aether_document::layer::{
    AdjustmentContent, ColorLabel, FillContent, GroupContent, Layer, LayerContent, Live2DContent,
    RasterContent,
};
use aether_document::selection::Selection;
use aether_document::tree::LayerTree;
use aether_document::{Background, Document, DocumentMetadata};
use aether_raster::adjust::Adjustment;
use aether_raster::{LayerEffect, Mask, Pixmap};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

/// Schema version written by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// Conventional file extension.
pub const EXTENSION: &str = "aether";

const MANIFEST: &str = "project.json";
const THUMBNAIL: &str = "thumbnail.png";
const SELECTION: &str = "selection.png";
const RIG: &str = "rig.json";

/// Top level of `project.json`.
#[derive(Debug, Serialize, Deserialize)]
struct ProjectFile {
    schema_version: u32,
    #[serde(default)]
    app_version: String,
    document: DocumentDto,
}

/// The document, with pixel data replaced by archive paths.
#[derive(Debug, Serialize, Deserialize)]
struct DocumentDto {
    name: String,
    width: u32,
    height: u32,
    background: Background,
    #[serde(default)]
    color_model: ColorModel,
    active_layer: LayerId,
    id_watermark: u64,
    metadata: DocumentMetadata,
    roots: Vec<LayerId>,
    layers: Vec<LayerDto>,
    #[serde(default)]
    selection: Option<String>,
    /// Archive path of the rig, when the document has one. Added after schema
    /// version 1 shipped; older files have none.
    #[serde(default)]
    rig: Option<String>,
}

/// One layer's properties plus references to its data entries.
#[derive(Debug, Serialize, Deserialize)]
struct LayerDto {
    id: LayerId,
    name: String,
    visible: bool,
    locked: bool,
    alpha_lock: bool,
    opacity: f32,
    blend_mode: BlendMode,
    #[serde(default)]
    transform: Transform2D,
    clipping: bool,
    #[serde(default = "default_true")]
    mask_enabled: bool,
    #[serde(default)]
    color_label: ColorLabel,
    /// Added after schema version 1 shipped; older files simply have none.
    #[serde(default)]
    effects: Vec<LayerEffect>,
    #[serde(default)]
    mask: Option<String>,
    content: LayerContentDto,
}

fn default_true() -> bool {
    true
}

/// Layer payloads as stored on disk.
#[derive(Debug, Serialize, Deserialize)]
enum LayerContentDto {
    /// Pixels live at the given archive path.
    Raster { data: String },
    /// A folder.
    Group {
        children: Vec<LayerId>,
        #[serde(default)]
        isolate: bool,
        #[serde(default)]
        collapsed: bool,
    },
    /// A colour adjustment.
    Adjustment { adjustment: Adjustment },
    /// A flat fill.
    Fill { color: Rgba8 },
    /// A Live2D model's texture pages at the given archive paths (the model
    /// itself is in the rig).
    Live2D { textures: Vec<String> },
    /// Plugin-owned content, preserved verbatim.
    Custom {
        kind: String,
        payload: serde_json::Value,
    },
}

/// Bring an older manifest up to [`SCHEMA_VERSION`].
///
/// Each step transforms the JSON in place, so a version 1 file loaded by a
/// future build walks 1 → 2 → 3 rather than needing a separate reader per
/// version.
pub fn migrate(value: &mut serde_json::Value, from: u32) -> Result<()> {
    if from > SCHEMA_VERSION {
        return Err(AetherError::UnsupportedVersion {
            found: from,
            supported: SCHEMA_VERSION,
        });
    }
    if from < SCHEMA_VERSION {
        // Version 1 is the first published schema, so anything lower is a file
        // this build cannot upgrade. Each future release adds a step here that
        // rewrites `value` in place and falls through to the next one.
        return Err(AetherError::serialization(format!(
            "no migration path from schema version {from}"
        )));
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert("schema_version".into(), serde_json::json!(SCHEMA_VERSION));
    }
    Ok(())
}

fn layer_data_path(id: LayerId) -> String {
    format!("layers/{}.png", id.raw())
}

fn mask_data_path(id: LayerId) -> String {
    format!("masks/{}.png", id.raw())
}

/// Write `doc` to `path` as a `.aether` project.
///
/// The file is written to a temporary sibling first and renamed into place, so
/// a crash or a full disk cannot destroy the previous save.
pub fn save_project(doc: &Document, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let bytes = serialize_project(doc)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let temp = path.with_extension("aether.tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

/// Serialise a document into project-file bytes.
pub fn serialize_project(doc: &Document) -> Result<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let mut layers = Vec::with_capacity(doc.layers.len());
        let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();

        for layer in doc.layers.iter() {
            let content = match &layer.content {
                LayerContent::Raster(raster) => {
                    let path = layer_data_path(layer.id);
                    blobs.insert(path.clone(), encode_pixmap_png(&raster.pixmap)?);
                    LayerContentDto::Raster { data: path }
                }
                LayerContent::Group(group) => LayerContentDto::Group {
                    children: group.children.clone(),
                    isolate: group.isolate,
                    collapsed: group.collapsed,
                },
                LayerContent::Adjustment(a) => LayerContentDto::Adjustment {
                    adjustment: a.adjustment.clone(),
                },
                LayerContent::Fill(f) => LayerContentDto::Fill { color: f.color },
                LayerContent::Live2D(model) => {
                    let mut textures = Vec::new();
                    for (i, page) in model.textures.iter().enumerate() {
                        let path = format!("live2d/{}/texture_{i}.png", layer.id.raw());
                        blobs.insert(path.clone(), encode_pixmap_png(page)?);
                        textures.push(path);
                    }
                    LayerContentDto::Live2D { textures }
                }
                LayerContent::Custom { kind, payload } => LayerContentDto::Custom {
                    kind: kind.clone(),
                    payload: payload.clone(),
                },
            };
            let mask = match &layer.mask {
                Some(mask) => {
                    let path = mask_data_path(layer.id);
                    blobs.insert(path.clone(), encode_mask_png(mask)?);
                    Some(path)
                }
                None => None,
            };
            layers.push(LayerDto {
                id: layer.id,
                name: layer.name.clone(),
                visible: layer.visible,
                locked: layer.locked,
                alpha_lock: layer.alpha_lock,
                opacity: layer.opacity,
                blend_mode: layer.blend_mode,
                transform: layer.transform,
                clipping: layer.clipping,
                mask_enabled: layer.mask_enabled,
                color_label: layer.color_label,
                effects: layer.effects.clone(),
                mask,
                content,
            });
        }

        let selection = match doc.selection.mask() {
            Some(mask) => {
                blobs.insert(SELECTION.to_string(), encode_mask_png(mask)?);
                Some(SELECTION.to_string())
            }
            None => None,
        };

        // The rig is kept out of the manifest: it can be large, and a
        // separate entry keeps project.json readable.
        let rig = if doc.rig == aether_document::rig::Rig::default() {
            None
        } else {
            let json = serde_json::to_vec(&doc.rig)
                .map_err(|e| AetherError::serialization(format!("could not write the rig: {e}")))?;
            blobs.insert(RIG.to_string(), json);
            Some(RIG.to_string())
        };

        let manifest = ProjectFile {
            schema_version: SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            document: DocumentDto {
                name: doc.name.clone(),
                width: doc.width,
                height: doc.height,
                background: doc.background,
                color_model: doc.color_model,
                active_layer: doc.active_layer,
                id_watermark: doc.ids.watermark(),
                metadata: doc.metadata.clone(),
                roots: doc.layers.roots().to_vec(),
                layers,
                selection,
                rig,
            },
        };

        let json = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| AetherError::serialization(format!("could not write manifest: {e}")))?;
        zip.start_file(MANIFEST, options)
            .map_err(|e| AetherError::serialization(e.to_string()))?;
        zip.write_all(&json)?;

        for (path, data) in blobs {
            zip.start_file(&path, options)
                .map_err(|e| AetherError::serialization(e.to_string()))?;
            zip.write_all(&data)?;
        }

        // A thumbnail makes the file useful to browsers and to our own recent-files list.
        if let Ok(thumb) = render_thumbnail(doc, 512) {
            zip.start_file(THUMBNAIL, options)
                .map_err(|e| AetherError::serialization(e.to_string()))?;
            zip.write_all(&thumb)?;
        }

        zip.finish()
            .map_err(|e| AetherError::serialization(e.to_string()))?;
    }
    Ok(buffer.into_inner())
}

fn render_thumbnail(doc: &Document, max_edge: u32) -> Result<Vec<u8>> {
    let composite = aether_render::Compositor::new().render(doc);
    let scale = (max_edge as f32 / doc.width.max(doc.height) as f32).min(1.0);
    let w = ((doc.width as f32 * scale).round() as u32).max(1);
    let h = ((doc.height as f32 * scale).round() as u32).max(1);
    crate::image_io::encode_image(&composite.scaled(w, h), &ExportSettings::default())
}

/// Read a `.aether` project from disk.
pub fn load_project(path: impl AsRef<Path>) -> Result<Document> {
    let bytes = std::fs::read(path.as_ref())?;
    deserialize_project(&bytes)
}

/// Parse project-file bytes into a document.
pub fn deserialize_project(bytes: &[u8]) -> Result<Document> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AetherError::UnsupportedFormat(format!("not an Aether project: {e}")))?;

    let mut json = String::new();
    archive
        .by_name(MANIFEST)
        .map_err(|_| AetherError::UnsupportedFormat("project.json is missing".into()))?
        .read_to_string(&mut json)?;

    let mut value: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| AetherError::serialization(format!("manifest is not valid JSON: {e}")))?;
    let version = value.get("schema_version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version != SCHEMA_VERSION {
        migrate(&mut value, version)?;
    }

    let manifest: ProjectFile = serde_json::from_value(value)
        .map_err(|e| AetherError::serialization(format!("manifest does not match the schema: {e}")))?;
    let dto = manifest.document;

    // Read every blob up front; the archive borrows mutably per entry.
    let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| AetherError::serialization(e.to_string()))?;
        let name = entry.name().to_string();
        if name == MANIFEST || name == THUMBNAIL || name.ends_with('/') {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        blobs.insert(name, data);
    }

    let mut doc = Document::empty(dto.width, dto.height, dto.name);
    doc.background = dto.background;
    doc.color_model = dto.color_model;
    doc.metadata = dto.metadata;
    doc.ids = IdGenerator::resume_from(dto.id_watermark);

    let mut tree = LayerTree::new();
    let mut ordered: BTreeMap<LayerId, Layer> = BTreeMap::new();
    for dto_layer in dto.layers {
        let content = match dto_layer.content {
            LayerContentDto::Raster { data } => {
                let pixmap = match blobs.get(&data) {
                    Some(bytes) => decode_image(bytes)?,
                    // A missing blob must not lose the rest of the document.
                    None => Pixmap::new(dto.width, dto.height),
                };
                LayerContent::Raster(RasterContent { pixmap })
            }
            LayerContentDto::Group {
                children,
                isolate,
                collapsed,
            } => LayerContent::Group(GroupContent {
                children,
                isolate,
                collapsed,
            }),
            LayerContentDto::Adjustment { adjustment } => {
                LayerContent::Adjustment(AdjustmentContent { adjustment })
            }
            LayerContentDto::Fill { color } => LayerContent::Fill(FillContent { color }),
            LayerContentDto::Live2D { textures } => {
                let mut pages = Vec::new();
                for path in textures {
                    pages.push(match blobs.get(&path) {
                        Some(bytes) => decode_image(bytes)?,
                        None => Pixmap::new(1, 1),
                    });
                }
                LayerContent::Live2D(Live2DContent { textures: pages })
            }
            LayerContentDto::Custom { kind, payload } => LayerContent::Custom { kind, payload },
        };
        let mask = dto_layer
            .mask
            .as_ref()
            .and_then(|path| blobs.get(path))
            .map(|bytes| decode_mask_png(bytes))
            .transpose()?;

        ordered.insert(
            dto_layer.id,
            Layer {
                id: dto_layer.id,
                name: dto_layer.name,
                visible: dto_layer.visible,
                locked: dto_layer.locked,
                alpha_lock: dto_layer.alpha_lock,
                opacity: dto_layer.opacity.clamp(0.0, 1.0),
                blend_mode: dto_layer.blend_mode,
                transform: dto_layer.transform,
                clipping: dto_layer.clipping,
                mask,
                mask_enabled: dto_layer.mask_enabled,
                color_label: dto_layer.color_label,
                effects: dto_layer.effects,
                content,
            },
        );
    }

    // Rebuild the tree by walking the saved root order, then any orphans, so a
    // manifest with a broken link still opens with every layer present.
    let mut placed = Vec::new();
    for root in &dto.roots {
        place_subtree(&mut tree, &mut ordered, *root, None, &mut placed)?;
    }
    let leftovers: Vec<LayerId> = ordered.keys().copied().collect();
    for id in leftovers {
        place_subtree(&mut tree, &mut ordered, id, None, &mut placed)?;
    }

    doc.layers = tree;
    doc.active_layer = dto.active_layer;
    if let Some(path) = dto.selection.as_ref().and_then(|p| blobs.get(p)) {
        let mask: Mask = decode_mask_png(path)?;
        let mut selection = Selection::none();
        selection.set_mask(Some(mask));
        doc.selection = selection;
    }
    if let Some(bytes) = dto.rig.as_ref().and_then(|p| blobs.get(p)) {
        // A damaged rig is reported rather than silently dropped: saving over
        // the file afterwards would otherwise destroy the rigging work.
        doc.rig = serde_json::from_slice(bytes)
            .map_err(|e| AetherError::serialization(format!("rig.json does not match the schema: {e}")))?;
    }
    doc.repair();
    Ok(doc)
}

/// Move `id` and its descendants from the pending map into the tree.
fn place_subtree(
    tree: &mut LayerTree,
    pending: &mut BTreeMap<LayerId, Layer>,
    id: LayerId,
    parent: Option<LayerId>,
    placed: &mut Vec<LayerId>,
) -> Result<()> {
    let Some(layer) = pending.remove(&id) else {
        // Already placed, or referenced but absent: skip rather than fail.
        return Ok(());
    };
    let children = layer.children().to_vec();
    let index = tree.children_of(parent).len();
    // Insert with an empty child list; children are linked as they are placed.
    let mut layer = layer;
    if let LayerContent::Group(group) = &mut layer.content {
        group.children.clear();
    }
    tree.insert(layer, parent, index)?;
    placed.push(id);
    for child in children {
        place_subtree(tree, pending, child, Some(id), placed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::math::IRect;
    use aether_document::selection::SelectionMode;

    fn sample_document() -> Document {
        let mut doc = Document::new(32, 24, "Sample");
        doc.background = Background::Solid(Rgba8::rgb(12, 34, 56));
        let base = doc.active_layer;
        if let Some(pm) = doc.layers.get_mut(base).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(2, 2, 10, 10), Rgba8::new(255, 128, 0, 200));
        }
        if let Some(layer) = doc.layers.get_mut(base) {
            layer.opacity = 0.75;
            layer.blend_mode = BlendMode::Multiply;
            layer.color_label = ColorLabel::Blue;
            let mut mask = Mask::new(32, 24);
            mask.fill_rect(IRect::new(0, 0, 16, 24), 255);
            layer.mask = Some(mask);
        }

        // A group holding a nested raster layer.
        let group_id = doc.next_layer_id();
        doc.layers
            .push_top(Layer::group(group_id, "Folder"))
            .expect("group");
        let child = doc.next_layer_id();
        doc.layers
            .insert(Layer::raster(child, "Nested", 32, 24), Some(group_id), 0)
            .expect("child");

        // An adjustment layer and a plugin layer.
        let adj = doc.next_layer_id();
        doc.layers
            .push_top(Layer::adjustment(adj, "Levels", Adjustment::default_levels()))
            .expect("adjustment");
        let custom = doc.next_layer_id();
        doc.layers
            .push_top(Layer::with_content(
                custom,
                "Plugin",
                LayerContent::Custom {
                    kind: "studio.example.thing".into(),
                    payload: serde_json::json!({ "value": 7 }),
                },
            ))
            .expect("custom");

        doc.selection
            .select_rect(32, 24, IRect::new(4, 4, 8, 8), SelectionMode::Replace);
        doc.active_layer = base;
        doc
    }

    #[test]
    fn round_trip_preserves_the_document() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("serialize");
        let back = deserialize_project(&bytes).expect("deserialize");

        assert_eq!((back.width, back.height), (doc.width, doc.height));
        assert_eq!(back.name, doc.name);
        assert_eq!(back.background, doc.background);
        assert_eq!(back.layer_count(), doc.layer_count());
        assert_eq!(back.active_layer, doc.active_layer);
        assert_eq!(back.layers.iter_render_order(), doc.layers.iter_render_order());
    }

    #[test]
    fn round_trip_preserves_pixels_exactly() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("serialize");
        let back = deserialize_project(&bytes).expect("deserialize");
        let before = aether_render::Compositor::new().render(&doc);
        let after = aether_render::Compositor::new().render(&back);
        assert_eq!(before, after, "the composite changed across save/load");
    }

    #[test]
    fn round_trip_preserves_layer_properties() {
        let doc = sample_document();
        let base = doc.active_layer;
        let bytes = serialize_project(&doc).expect("serialize");
        let back = deserialize_project(&bytes).expect("deserialize");
        let original = doc.layers.get(base).expect("original");
        let restored = back.layers.get(base).expect("restored");
        assert_eq!(restored.opacity, original.opacity);
        assert_eq!(restored.blend_mode, original.blend_mode);
        assert_eq!(restored.color_label, original.color_label);
        assert!(restored.mask.is_some());
        assert_eq!(restored.mask, original.mask);
    }

    #[test]
    fn round_trip_preserves_groups_and_nesting() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("serialize");
        let back = deserialize_project(&bytes).expect("deserialize");
        for (id, depth) in doc.layers.iter_render_order() {
            let restored_depth = back
                .layers
                .iter_render_order()
                .into_iter()
                .find(|(rid, _)| *rid == id)
                .map(|(_, d)| d);
            assert_eq!(restored_depth, Some(depth), "nesting changed for {id}");
        }
    }

    #[test]
    fn round_trip_preserves_plugin_payloads() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("serialize");
        let back = deserialize_project(&bytes).expect("deserialize");
        let custom = back
            .layers
            .iter()
            .find(|l| matches!(l.content, LayerContent::Custom { .. }))
            .expect("plugin layer survived");
        match &custom.content {
            LayerContent::Custom { kind, payload } => {
                assert_eq!(kind, "studio.example.thing");
                assert_eq!(payload["value"], 7);
            }
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[test]
    fn round_trip_preserves_the_selection() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("serialize");
        let back = deserialize_project(&bytes).expect("deserialize");
        assert!(back.selection.is_active());
        assert_eq!(back.selection.bounds(), doc.selection.bounds());
    }

    #[test]
    fn ids_do_not_collide_after_a_reload() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("serialize");
        let back = deserialize_project(&bytes).expect("deserialize");
        let fresh = back.next_layer_id();
        assert!(!back.layers.contains(fresh));
        assert!(fresh.raw() > back.layers.max_id());
    }

    #[test]
    fn saving_to_disk_and_reading_back_works() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/dir/test.aether");
        let doc = sample_document();
        save_project(&doc, &path).expect("save");
        assert!(path.exists());
        let back = load_project(&path).expect("load");
        assert_eq!(back.layer_count(), doc.layer_count());
        // The temporary file must not be left behind.
        assert!(!path.with_extension("aether.tmp").exists());
    }

    #[test]
    fn the_archive_contains_the_documented_entries() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("serialize");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        assert!(names.contains(&MANIFEST.to_string()));
        assert!(names.contains(&THUMBNAIL.to_string()));
        assert!(names.iter().any(|n| n.starts_with("layers/")));
        assert!(names.iter().any(|n| n.starts_with("masks/")));
        assert!(names.contains(&SELECTION.to_string()));
    }

    #[test]
    fn a_future_schema_version_is_rejected_clearly() {
        let doc = Document::new(4, 4, "d");
        let bytes = serialize_project(&doc).expect("serialize");
        // Rewrite the manifest with an impossible version.
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut json = String::new();
        archive
            .by_name(MANIFEST)
            .expect("manifest")
            .read_to_string(&mut json)
            .expect("read");
        let mut value: serde_json::Value = serde_json::from_str(&json).expect("json");
        value["schema_version"] = serde_json::json!(SCHEMA_VERSION + 5);

        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            zip.start_file(MANIFEST, options).expect("start");
            zip.write_all(serde_json::to_string(&value).expect("json").as_bytes())
                .expect("write");
            zip.finish().expect("finish");
        }
        let err = deserialize_project(&out.into_inner()).expect_err("must refuse");
        assert!(
            matches!(err, AetherError::UnsupportedVersion { .. }),
            "expected a version error, got {err:?}"
        );
    }

    #[test]
    fn a_non_project_file_fails_with_a_clear_error() {
        let err = deserialize_project(b"definitely not a zip").expect_err("must fail");
        assert!(matches!(err, AetherError::UnsupportedFormat(_)), "got {err:?}");
    }

    #[test]
    fn a_missing_layer_blob_does_not_lose_the_document() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("serialize");
        // Rebuild the archive without the layer pixel data.
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).expect("entry");
                let name = entry.name().to_string();
                if name.starts_with("layers/") {
                    continue;
                }
                let mut data = Vec::new();
                entry.read_to_end(&mut data).expect("read");
                zip.start_file(&name, options).expect("start");
                zip.write_all(&data).expect("write");
            }
            zip.finish().expect("finish");
        }
        let back = deserialize_project(&out.into_inner()).expect("should still open");
        assert_eq!(
            back.layer_count(),
            doc.layer_count(),
            "structure must survive missing pixels"
        );
    }
}

#[cfg(test)]
mod effect_round_trip_tests {
    use super::*;
    use aether_raster::{EffectKind, LayerEffect};

    #[test]
    fn effect_stacks_survive_save_and_load() {
        let mut doc = Document::new(16, 16, "effects");
        let id = doc.active_layer;
        if let Some(layer) = doc.layers.get_mut(id) {
            layer.effects = vec![
                LayerEffect::new(EffectKind::DropShadow {
                    dx: 4.0,
                    dy: 5.0,
                    radius: 2.0,
                    color: Rgba8::BLACK,
                    opacity: 0.7,
                }),
                LayerEffect {
                    kind: EffectKind::Blur { sigma: 1.5 },
                    enabled: false,
                },
            ];
        }
        let bytes = serialize_project(&doc).expect("serialize");
        let back = deserialize_project(&bytes).expect("deserialize");
        let restored = back.layers.get(id).expect("layer");
        assert_eq!(restored.effects.len(), 2);
        assert!(!restored.effects[1].enabled, "the disabled flag must survive");
        assert_eq!(restored.effects, doc.layers.get(id).expect("layer").effects);
    }

    #[test]
    fn projects_written_before_effects_existed_still_open() {
        // Round-trip a document, then strip the field from the manifest the way
        // an older build would have written it.
        let doc = Document::new(8, 8, "legacy");
        let bytes = serialize_project(&doc).expect("serialize");
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut json = String::new();
        archive
            .by_name(MANIFEST)
            .expect("manifest")
            .read_to_string(&mut json)
            .expect("read");
        let mut value: serde_json::Value = serde_json::from_str(&json).expect("json");
        if let Some(layers) = value["document"]["layers"].as_array_mut() {
            for layer in layers {
                layer.as_object_mut().expect("object").remove("effects");
            }
        }

        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            zip.start_file(MANIFEST, options).expect("start");
            zip.write_all(serde_json::to_string(&value).expect("json").as_bytes())
                .expect("write");
            zip.finish().expect("finish");
        }
        let back = deserialize_project(&out.into_inner()).expect("older files must still open");
        assert_eq!(back.layer_count(), 1);
        assert!(back.layers.iter().all(|l| l.effects.is_empty()));
    }
}

#[cfg(test)]
mod rig_round_trip_tests {
    use super::*;

    fn sample_document() -> Document {
        let mut doc = Document::new(32, 32, "rigged");
        let id = doc.active_layer;
        if let Some(pm) = doc.layers.get_mut(id).and_then(|l| l.pixmap_mut()) {
            pm.fill(Rgba8::rgb(10, 20, 30));
        }
        doc
    }

    #[test]
    fn rigs_round_trip_through_a_project_file() {
        use aether_core::math::{vec2, Rect};
        use aether_document::rig::{ArtMesh, KeyAxis, Motion, Parameter};
        let mut doc = sample_document();
        let layer = doc.active_layer;
        let param = doc.ids.parameter();
        doc.rig
            .add_parameter(Parameter::new(param, "AngleX", -30.0, 30.0, 0.0))
            .expect("param");
        let mut mesh = ArtMesh::quad(layer, Rect::from_corners(vec2(0.0, 0.0), vec2(16.0, 16.0)));
        mesh.keyforms
            .add_axis(KeyAxis::new(param, [-30.0, 30.0]).expect("axis"))
            .expect("axis");
        mesh.keyforms.forms[1].offsets[0] = vec2(3.0, 4.0);
        doc.rig.set_mesh(mesh);
        let mut motion = Motion::new("idle", 2.0, 30.0);
        motion.track_mut(param).set_key(1.0, 12.0);
        doc.rig.motions.push(motion);
        doc.rig.set_value(param, 7.5);

        let bytes = serialize_project(&doc).expect("save");
        let back = deserialize_project(&bytes).expect("load");
        assert_eq!(back.rig, doc.rig);
        assert!(back.next_layer_id().raw() > param.raw());
    }

    #[test]
    fn documents_without_a_rig_write_no_rig_entry() {
        let doc = sample_document();
        let bytes = serialize_project(&doc).expect("save");
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        assert!(archive.file_names().all(|n| n != RIG));
    }

    #[test]
    fn a_damaged_rig_is_reported_not_dropped() {
        use aether_document::rig::Parameter;
        let mut doc = sample_document();
        let id = doc.ids.parameter();
        doc.rig
            .add_parameter(Parameter::new(id, "AngleX", -30.0, 30.0, 0.0))
            .expect("param");
        let bytes = serialize_project(&doc).expect("save");
        // Rewrite the archive with a corrupted rig.json.
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut out = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut out);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).expect("entry");
                let name = entry.name().to_string();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).expect("read");
                if name == RIG {
                    data = b"{ not json".to_vec();
                }
                writer.start_file(&name, options).expect("start");
                writer.write_all(&data).expect("write");
            }
            writer.finish().expect("finish");
        }
        let err = deserialize_project(&out.into_inner()).expect_err("corrupt rig");
        assert!(err.to_string().contains("rig.json"), "{err}");
    }
}
