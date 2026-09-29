//! Opening Live2D Cubism models (`.model3.json` with its `.moc3`, textures,
//! motions, expressions, pose and display names) as Aether documents.
//!
//! The model becomes one Live2D model layer, drawn exactly as Cubism
//! deforms it (see `aether_live2d`), and its parameters become rig
//! parameters: motions, expressions, physics, the timeline, look-at, lip
//! sync, blinking and face tracking all drive it. Parameter names lose the
//! `Param` prefix, as Aether's standard parameters do (`ParamAngleX` becomes
//! `AngleX`), so everything wired to standard names finds them; display
//! names and groups from `.cdi3.json` show in the parameter panel. Parts
//! that motions or pose groups switch get a 0..1 parameter named after the
//! part.

use crate::image_io::decode_image;
use crate::live2d::{import_expression, import_motion};
use aether_core::math::Vec2;
use aether_core::{AetherError, Result};
use aether_document::layer::{Layer, LayerContent, Live2DContent};
use aether_document::rig::behaviour::Behaviours;
use aether_document::rig::cubism::{CubismRig, HitArea, PoseGroup, PosePart};
use aether_document::rig::Parameter;
use aether_document::Document;
use aether_live2d::moc3::Moc;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// How to open a model.
#[derive(Clone, Debug)]
pub struct Live2DImportOptions {
    /// Longest document side in pixels; the model's canvas is scaled down
    /// to fit (never up).
    pub max_size: u32,
}

impl Default for Live2DImportOptions {
    fn default() -> Self {
        Self { max_size: 2048 }
    }
}

/// An opened model.
pub struct ImportedLive2D {
    /// The document.
    pub document: Document,
    /// What was left out or approximated.
    pub notes: Vec<String>,
}

fn bad(message: impl std::fmt::Display) -> AetherError {
    AetherError::serialization(format!("not a Live2D model: {message}"))
}

fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key)
}

fn text(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// A rig parameter name for a Cubism parameter id.
fn parameter_name(id: &str) -> String {
    match id.get(..5) {
        Some(p) if p.eq_ignore_ascii_case("param") && id.len() > 5 => id[5..].to_string(),
        _ => id.to_string(),
    }
}

/// Open `path` (a `.model3.json`, or a folder holding one).
pub fn import_live2d(path: impl AsRef<Path>, options: &Live2DImportOptions) -> Result<ImportedLive2D> {
    let mut path = path.as_ref().to_path_buf();
    if path.is_dir() {
        let found = std::fs::read_dir(&path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.to_string_lossy().ends_with(".model3.json"))
            .ok_or_else(|| bad("the folder holds no .model3.json"))?;
        path = found;
    }
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.trim_end_matches(".model3.json").to_string())
        .unwrap_or_else(|| "Live2D model".into());
    let json = std::fs::read_to_string(&path)?;
    let read = |file: &str| -> Result<Vec<u8>> {
        let full: PathBuf = dir.join(file);
        std::fs::read(&full).map_err(|e| AetherError::serialization(format!("{}: {e}", full.display())))
    };
    import_live2d_with(&json, &name, &read, options)
}

/// Open a model whose `model3.json` text is `json`, reading the files it
/// refers to through `read` (paths relative to the model3.json).
pub fn import_live2d_with(
    json: &str,
    name: &str,
    read: &dyn Fn(&str) -> Result<Vec<u8>>,
    options: &Live2DImportOptions,
) -> Result<ImportedLive2D> {
    let settings: Value = serde_json::from_str(json).map_err(bad)?;
    let refs = field(&settings, "FileReferences").ok_or_else(|| bad("no FileReferences"))?;
    let moc_file = text(field(refs, "Moc")).ok_or_else(|| bad("no Moc file"))?;
    let moc = Moc::read(&read(moc_file)?).map_err(|e| bad(format!("{moc_file}: {e}")))?;
    let mut notes = Vec::new();

    // Texture pages.
    let mut textures = Vec::new();
    let mut texture_files = Vec::new();
    for t in field(refs, "Textures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(file) = t.as_str() else { continue };
        textures.push(decode_image(&read(file)?)?);
        texture_files.push(file.to_string());
    }
    let pages_needed = moc
        .art_meshes
        .texture
        .iter()
        .map(|&t| t + 1)
        .max()
        .unwrap_or(0)
        .max(0) as usize;
    if textures.len() < pages_needed {
        return Err(bad(format!(
            "the model uses {pages_needed} textures but lists {}",
            textures.len()
        )));
    }

    // The document: the canvas, scaled down to fit.
    let (cw, ch) = (moc.canvas.width.max(1.0), moc.canvas.height.max(1.0));
    let scale = (options.max_size.max(16) as f32 / cw.max(ch)).min(1.0);
    let width = (cw * scale).ceil() as u32;
    let height = (ch * scale).ceil() as u32;
    let mut doc = Document::empty(width, height, name);
    if scale < 1.0 {
        notes.push(format!(
            "the {}×{} canvas is shown at {:.0}% ({width}×{height}); export keeps the original",
            cw as u32,
            ch as u32,
            scale * 100.0
        ));
    }

    // Display names and groups.
    let display = text(field(refs, "DisplayInfo"))
        .and_then(|f| read(f).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let display_name = |section: &str, id: &str| -> Option<String> {
        display
            .as_ref()?
            .get(section)?
            .as_array()?
            .iter()
            .find(|e| e.get("Id").and_then(Value::as_str) == Some(id))?
            .get("Name")?
            .as_str()
            .map(str::to_string)
    };
    let group_of = |id: &str| -> String {
        let Some(d) = display.as_ref() else {
            return String::new();
        };
        let group_id = d
            .get("Parameters")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find(|e| e.get("Id").and_then(Value::as_str) == Some(id)))
            .and_then(|e| e.get("GroupId"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if group_id.is_empty() {
            return String::new();
        }
        display_name("ParameterGroups", group_id).unwrap_or_else(|| group_id.to_string())
    };

    // Parameters.
    let mut rig_model = CubismRig::new(aether_core::LayerId(0), moc.clone());
    for (i, id) in moc.parameters.ids.iter().enumerate() {
        let mut pname = parameter_name(id);
        if doc.rig.parameter_named(&pname).is_some() {
            pname = id.clone();
        }
        let mut p = Parameter::new(
            doc.ids.parameter(),
            pname,
            moc.parameters.min[i],
            moc.parameters.max[i],
            moc.parameters.default[i],
        );
        p.cyclic = moc.parameters.repeat[i] != 0;
        p.group = group_of(id);
        if let Some(label) = display_name("Parameters", id) {
            p.label = label;
        }
        rig_model.parameters[i] = Some(doc.rig.add_parameter(p)?);
    }

    // Pose groups, and the parts motions switch, get part parameters.
    let pose = text(field(refs, "Pose"))
        .and_then(|f| read(f).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let mut switched: BTreeSet<usize> = BTreeSet::new();
    let part_index = |id: &str| moc.parts.ids.iter().position(|p| p == id);
    if let Some(pose) = &pose {
        rig_model.pose_fade = pose.get("FadeInTime").and_then(Value::as_f64).unwrap_or(0.5) as f32;
        for group in pose.get("Groups").and_then(Value::as_array).into_iter().flatten() {
            let mut g = PoseGroup::default();
            for member in group.as_array().into_iter().flatten() {
                let Some(part) = member.get("Id").and_then(Value::as_str).and_then(part_index) else {
                    continue;
                };
                let links = member
                    .get("Link")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|l| l.as_str().and_then(part_index))
                    .collect();
                switched.insert(part);
                g.parts.push(PosePart { part, links });
            }
            if !g.parts.is_empty() {
                rig_model.pose.push(g);
            }
        }
    }

    // Motion files, read once: part opacity curves need parameters first.
    let mut motion_files = Vec::new();
    if let Some(groups) = field(refs, "Motions").and_then(Value::as_object) {
        for (group, entries) in groups {
            let entries = entries.as_array().cloned().unwrap_or_default();
            let many = entries.len() > 1;
            for (k, entry) in entries.iter().enumerate() {
                let Some(file) = entry.get("File").and_then(Value::as_str) else {
                    continue;
                };
                let label = if many {
                    format!("{group} {}", k + 1)
                } else {
                    group.clone()
                };
                match read(file).map(|b| String::from_utf8_lossy(&b).into_owned()) {
                    Ok(text) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&text) {
                            for c in v.get("Curves").and_then(Value::as_array).into_iter().flatten() {
                                if c.get("Target").and_then(Value::as_str) == Some("PartOpacity") {
                                    if let Some(p) = c.get("Id").and_then(Value::as_str).and_then(part_index)
                                    {
                                        switched.insert(p);
                                    }
                                }
                            }
                        }
                        if entry.get("Sound").is_some() {
                            notes.push(format!("motion {label}: its sound is not imported"));
                        }
                        motion_files.push((label, text, entry.clone()));
                    }
                    Err(e) => notes.push(format!("motion {label} could not be read: {e}")),
                }
            }
        }
    }

    for &part in &switched {
        let id = &moc.parts.ids[part];
        let in_pose = rig_model
            .pose
            .iter()
            .any(|g| g.parts.iter().any(|m| m.part == part));
        let default = if in_pose {
            // The first member of each pose group shows at first.
            let first = rig_model
                .pose
                .iter()
                .any(|g| g.parts.first().is_some_and(|m| m.part == part));
            if first {
                1.0
            } else {
                0.0
            }
        } else if moc.parts.visible[part] != 0 {
            1.0
        } else {
            0.0
        };
        let mut pname = id.clone();
        if doc.rig.parameter_named(&pname).is_some() {
            pname = format!("Part {id}");
        }
        let mut p = Parameter::new(doc.ids.parameter(), pname, 0.0, 1.0, default);
        p.group = "Parts".into();
        if let Some(label) = display_name("Parts", id) {
            p.label = label;
        }
        rig_model.parts[part] = Some(doc.rig.add_parameter(p)?);
    }

    // The layer.
    let layer_id = doc.ids.layer();
    rig_model.layer = layer_id;
    rig_model.scale = scale;
    rig_model.offset = Vec2::ZERO;
    rig_model.texture_files = texture_files;
    let layer = Layer::with_content(layer_id, name, LayerContent::Live2D(Live2DContent { textures }));
    doc.layers.insert(layer, None, 0)?;
    doc.active_layer = layer_id;

    // Hit areas.
    for h in field(&settings, "HitAreas")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(id), name) = (
            h.get("Id").and_then(Value::as_str),
            h.get("Name").and_then(Value::as_str),
        ) {
            rig_model.hit_areas.push(HitArea {
                drawable: id.to_string(),
                name: name.unwrap_or(id).to_string(),
            });
        }
    }

    // Keep what the editor does not interpret, for export.
    let userdata = text(field(refs, "UserData"))
        .and_then(|f| read(f).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let physics = text(field(refs, "Physics"))
        .and_then(|f| read(f).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    rig_model.extra = serde_json::json!({
        "model3": settings,
        "cdi3": display,
        "userdata3": userdata,
    });
    rig_model.physics = physics;
    doc.rig.cubism.push(rig_model);

    // Motions and expressions.
    for (label, text, entry) in motion_files {
        match import_motion(&text, &doc.rig, &label) {
            Ok((mut motion, motion_notes)) => {
                if let Some(f) = entry.get("FadeInTime").and_then(Value::as_f64) {
                    motion.fade_in = f.max(0.0) as f32;
                }
                if let Some(f) = entry.get("FadeOutTime").and_then(Value::as_f64) {
                    motion.fade_out = f.max(0.0) as f32;
                }
                notes.extend(motion_notes.into_iter().map(|n| format!("motion {label}: {n}")));
                doc.rig.motions.push(motion);
            }
            Err(e) => notes.push(format!("motion {label} could not be read: {e}")),
        }
    }
    for e in field(refs, "Expressions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(ename), Some(file)) = (
            e.get("Name").and_then(Value::as_str),
            e.get("File").and_then(Value::as_str),
        ) else {
            continue;
        };
        match read(file).map(|b| String::from_utf8_lossy(&b).into_owned()) {
            Ok(text) => match import_expression(&text, &doc.rig, ename) {
                Ok((expression, expr_notes)) => {
                    notes.extend(expr_notes.into_iter().map(|n| format!("expression {ename}: {n}")));
                    doc.rig.expressions.push(expression);
                }
                Err(err) => notes.push(format!("expression {ename} could not be read: {err}")),
            },
            Err(err) => notes.push(format!("expression {ename} could not be read: {err}")),
        }
    }

    // Blinking and lip sync from the model's groups, then the standard
    // wiring for anything else (breathing, look-at).
    let mut behaviours = Behaviours::standard(&doc.rig.parameters);
    for g in field(&settings, "Groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ids: Vec<_> = g
            .get("Ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|i| i.as_str())
            .filter_map(|id| moc.parameters.ids.iter().position(|p| p == id))
            .filter_map(|i| doc.rig.cubism[0].parameters[i])
            .collect();
        match g.get("Name").and_then(Value::as_str) {
            Some("EyeBlink") => {
                behaviours.blink.params = ids;
                behaviours.blink.enabled = !behaviours.blink.params.is_empty();
            }
            Some("LipSync") => {
                behaviours.lip_sync.mouth_open = ids.first().copied();
                behaviours.lip_sync.enabled = behaviours.lip_sync.mouth_open.is_some();
            }
            _ => {}
        }
    }
    doc.rig.behaviours = behaviours;
    Ok(ImportedLive2D { document: doc, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/live2d-oracle/models")
            .join(name)
            .join(format!("{name}.model3.json"));
        p.exists().then_some(p)
    }

    #[test]
    fn live2d_samples_open_as_documents() {
        let Some(path) = sample("Hiyori") else {
            eprintln!("no Live2D samples (crates/aether-live2d/oracle/run.sh); skipping");
            return;
        };
        let imported = import_live2d(&path, &Live2DImportOptions::default()).unwrap();
        let doc = &imported.document;
        assert_eq!(doc.rig.cubism.len(), 1);
        assert!(doc.rig.parameter_named("AngleX").is_some());
        assert!(
            doc.rig.parameter_named("PartArmA").is_some(),
            "pose parts get parameters"
        );
        assert!(doc.rig.motions.len() >= 10);
        assert!(doc.rig.behaviours.blink.enabled);
        assert!(doc.width <= 2048 && doc.height <= 2048);
        let label = &doc.rig.parameter_named("AngleX").unwrap().label;
        assert_eq!(label, "角度 X");
    }

    #[test]
    fn live2d_physics_and_pose_run_in_the_rig_runtime() {
        let Some(path) = sample("Hiyori") else {
            eprintln!("no Live2D samples (crates/aether-live2d/oracle/run.sh); skipping");
            return;
        };
        let mut doc = import_live2d(&path, &Live2DImportOptions::default())
            .unwrap()
            .document;
        assert!(doc.rig.cubism[0].physics.is_some());
        let hair = doc.rig.parameter_named("HairFront").unwrap().id;
        let angle = doc.rig.parameter_named("AngleX").unwrap().id;
        let arm_a = doc.rig.parameter_named("PartArmA").unwrap().id;
        let arm_b = doc.rig.parameter_named("PartArmB").unwrap().id;
        let mut runtime = aether_document::rig::RigRuntime::new();
        runtime.settings.behaviours = false;
        runtime.tick(&mut doc.rig, 1.0 / 60.0);
        let rest = doc
            .rig
            .dynamics
            .values
            .as_ref()
            .unwrap()
            .get(&hair)
            .copied()
            .unwrap_or(0.0);

        // Turning the head swings the hair.
        doc.rig.set_value(angle, 30.0);
        let mut moved: f32 = 0.0;
        for _ in 0..30 {
            runtime.tick(&mut doc.rig, 1.0 / 60.0);
            let v = doc
                .rig
                .dynamics
                .values
                .as_ref()
                .unwrap()
                .get(&hair)
                .copied()
                .unwrap_or(0.0);
            moved = moved.max((v - rest).abs());
        }
        assert!(moved > 0.05, "hair moved {moved}");

        // Switching the arm pose cross-fades the parts.
        doc.rig.set_value(arm_a, 0.0);
        doc.rig.set_value(arm_b, 1.0);
        let parts =
            |doc: &aether_document::Document| doc.rig.dynamics.cubism_parts.values().next().unwrap().clone();
        let before = parts(&doc);
        runtime.tick(&mut doc.rig, 0.1);
        let during = parts(&doc);
        for _ in 0..30 {
            runtime.tick(&mut doc.rig, 1.0 / 60.0);
        }
        let after = parts(&doc);
        assert_ne!(before, during);
        assert_ne!(during, after, "the fade takes more than one step");
    }

    #[test]
    fn live2d_models_round_trip_through_a_project_file() {
        let Some(path) = sample("Hiyori") else {
            eprintln!("no Live2D samples (crates/aether-live2d/oracle/run.sh); skipping");
            return;
        };
        let doc = import_live2d(&path, &Live2DImportOptions::default())
            .unwrap()
            .document;
        let bytes = crate::project::serialize_project(&doc).unwrap();
        let back = crate::project::deserialize_project(&bytes).unwrap();
        let (a, b) = (&doc.rig.cubism[0], &back.rig.cubism[0]);
        assert_eq!(a.moc.write(), b.moc.write());
        assert_eq!(a.parameters, b.parameters);
        assert_eq!(a.physics, b.physics);
        assert_eq!(doc.rig.motions.len(), back.rig.motions.len());
        let render = |d: &aether_document::Document| {
            aether_render::Compositor::new().render_with(d, &aether_render::RenderOptions::default())
        };
        assert!(render(&doc).data() == render(&back).data());
    }
}
