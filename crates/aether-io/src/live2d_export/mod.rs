//! Exporting Live2D Cubism models.
//!
//! [`export_live2d`] writes what Cubism's runtimes load — VTube Studio,
//! nizima LIVE, the Cubism SDKs for Unity, native and the web: a `.moc3`,
//! its textures, `.model3.json`, `.physics3.json`, `.cdi3.json` (display
//! names), motions (`.motion3.json`) and expressions (`.exp3.json`), with
//! blinking and lip sync declared for the SDK's eye-blink and lip-sync
//! helpers.
//!
//! Two kinds of document export:
//!
//! * **An opened Live2D model** is written back as it came, with the edits
//!   made in Aether: its motions and expressions (new and changed), pose
//!   groups, hit areas, parameter ranges and display names. The `.moc3`
//!   and textures are unchanged.
//! * **A document rigged in Aether** becomes a new Cubism model: see
//!   [`native`] for how the rig is translated and checked, and [`physics`]
//!   for how physics is fitted.
//!
//! The `.moc3` is for runtimes; it is not a Cubism Editor project (`.cmo3`),
//! which keeps editing data the runtime format does not have.

mod native;
mod physics;
pub mod sample;

use crate::image_io::encode_pixmap_png;
use crate::live2d::{export_expression_with, export_motion_with, Live2DTarget};
use aether_core::{ParameterId, Result};
use aether_document::layer::LayerContent;
use aether_document::rig::cubism::CubismRig;
use aether_document::rig::motion::{Easing, Keyframe, Track};
use aether_document::rig::{driver, Motion, ParamValues, Rig};
use aether_document::Document;
use aether_live2d::moc3::{self, Moc};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Knobs for [`export_live2d`].
#[derive(Clone, Debug, PartialEq)]
pub struct Live2DExportOptions {
    /// `.moc3` version for new models ([`moc3::VERSION_42`], Cubism 4.2,
    /// by default: multiply and screen colours, read by every current
    /// runtime). [`moc3::VERSION_53`] adds screen and other blend modes but
    /// needs a Cubism 5.3 runtime.
    pub version: u8,
    /// Largest texture page, pixels.
    pub max_texture_size: u32,
    /// How far, in pixels, a new model's keyforms may stray from the rig
    /// before more are added; `None` for 0.1% of the canvas's longer side
    /// (at least a quarter pixel).
    pub tolerance: Option<f32>,
    /// Fold drivers into the model's keyforms, so a driven parameter moves
    /// with what drives it as in the editor. Off by default: Cubism models
    /// have no drivers, apps such as VTube Studio drive each parameter
    /// themselves, and baking makes keyform grids much larger. Motions carry
    /// driven curves either way.
    pub bake_drivers: bool,
}

impl Default for Live2DExportOptions {
    fn default() -> Self {
        Self {
            version: moc3::VERSION_42,
            max_texture_size: 4096,
            tolerance: None,
            bake_drivers: false,
        }
    }
}

/// An exported model: its files, ready to write.
#[derive(Clone, Debug)]
pub struct Live2DExport {
    /// Base name of the files (`NAME.model3.json` and so on).
    pub name: String,
    /// Files by path relative to the model folder.
    pub files: Vec<(String, Vec<u8>)>,
    /// What was approximated or left out.
    pub notes: Vec<String>,
    /// For a new model: the largest difference, in pixels, between a vertex
    /// in Cubism and in Aether at the poses checked.
    pub max_error: Option<f32>,
}

impl Live2DExport {
    /// Path of the `.model3.json`, relative to the model folder.
    pub fn model_file(&self) -> String {
        format!("{}.model3.json", self.name)
    }

    /// A file's contents.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| b.as_slice())
    }
}

/// A Cubism id from a name: letters, digits and underscores, starting
/// with a letter, at most 60 bytes, and not in `used` (which it joins).
pub(crate) fn unique_id(name: &str, prefix: &str, used: &mut BTreeSet<String>) -> String {
    let mut id: String = name
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => Some(c),
            ' ' | '-' | '.' | '/' => Some('_'),
            _ => None,
        })
        .collect();
    let trimmed = id.trim_matches('_');
    id = if trimmed.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        trimmed.to_string()
    } else if trimmed.is_empty() || trimmed == prefix.trim_matches('_') {
        prefix.to_string()
    } else {
        format!("{prefix}_{trimmed}")
    };
    // Names that were all non-ASCII leave only the prefix (and maybe an
    // underscore): number them.
    if id == prefix || id.ends_with('_') {
        id = format!("{}{}", id.trim_end_matches('_'), used.len() + 1);
    }
    id.truncate(56);
    let mut candidate = id.clone();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{id}_{n}");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// A file name from a display name (letters, digits, `_`, `-`; others
/// become `_`), unique in `used`.
fn file_name(name: &str, fallback: &str, used: &mut BTreeSet<String>) -> String {
    let mut base: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    base = base.trim_matches('_').to_string();
    if base.is_empty() {
        base = fallback.to_string();
    }
    let mut candidate = base.clone();
    let mut n = 2;
    while !used.insert(candidate.to_lowercase()) {
        candidate = format!("{base}_{n}");
        n += 1;
    }
    candidate
}

fn json_bytes(v: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(v).expect("JSON serialises");
    text.push('\n');
    text.into_bytes()
}

/// Export `doc` as a Live2D model.
pub fn export_live2d(doc: &Document, options: &Live2DExportOptions) -> Result<Live2DExport> {
    let mut used = BTreeSet::new();
    let name = file_name(&doc.name, "model", &mut used);
    let cubism = doc.rig.cubism.iter().find(|c| {
        matches!(
            doc.layers.get(c.layer).map(|l| &l.content),
            Some(LayerContent::Live2D(_))
        )
    });
    match cubism {
        Some(model) => pass_through(doc, model, name),
        None => new_model(doc, options, name),
    }
}

/// Export `doc` into `dir` (created if needed). Returns the export; the
/// model is at `dir.join(export.model_file())`.
pub fn export_live2d_to_dir(
    doc: &Document,
    dir: impl AsRef<Path>,
    options: &Live2DExportOptions,
) -> Result<Live2DExport> {
    let export = export_live2d(doc, options)?;
    save_live2d(&export, dir)?;
    Ok(export)
}

/// Write an export's files into `dir`. Returns the `.model3.json` path.
pub fn save_live2d(export: &Live2DExport, dir: impl AsRef<Path>) -> Result<PathBuf> {
    let dir = dir.as_ref();
    for (path, bytes) in &export.files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full, bytes)?;
    }
    Ok(dir.join(export.model_file()))
}

/// Motions, expressions and the model3.json sections that refer to them.
struct Companions {
    files: Vec<(String, Vec<u8>)>,
    motions: Map<String, Value>,
    expressions: Vec<Value>,
}

fn companions(
    motion_list: &[Motion],
    rig: &Rig,
    target: &dyn Fn(ParameterId) -> Option<Live2DTarget>,
) -> Companions {
    let mut files = Vec::new();
    let mut motions = Map::new();
    let mut used = BTreeSet::new();
    for motion in motion_list {
        let file = format!(
            "motions/{}.motion3.json",
            file_name(&motion.name, "motion", &mut used)
        );
        files.push((file.clone(), export_motion_with(motion, target).into_bytes()));
        // Cubism apps play the "Idle" group when nothing else is playing.
        let group = if motion.name.to_lowercase().starts_with("idle") {
            "Idle".to_string()
        } else {
            motion.name.clone()
        };
        let entry = json!({
            "File": file,
            "FadeInTime": motion.fade_in,
            "FadeOutTime": motion.fade_out,
        });
        match motions.get_mut(&group).and_then(Value::as_array_mut) {
            Some(list) => list.push(entry),
            None => {
                motions.insert(group, json!([entry]));
            }
        }
    }
    let mut expressions = Vec::new();
    let mut used = BTreeSet::new();
    let id = |p: ParameterId| match target(p) {
        Some(Live2DTarget::Parameter(id)) => Some(id),
        _ => None,
    };
    for expression in &rig.expressions {
        let file = format!(
            "expressions/{}.exp3.json",
            file_name(&expression.name, "expression", &mut used)
        );
        files.push((file.clone(), export_expression_with(expression, &id).into_bytes()));
        expressions.push(json!({ "Name": expression.name, "File": file }));
    }
    Companions {
        files,
        motions,
        expressions,
    }
}

/// `Groups` for eye blinking and lip sync.
fn groups(rig: &Rig, id: &dyn Fn(ParameterId) -> Option<String>) -> Vec<Value> {
    let b = &rig.behaviours;
    let blink: Vec<String> = b.blink.params.iter().filter_map(|&p| id(p)).collect();
    let lip: Vec<String> = b.lip_sync.mouth_open.iter().filter_map(|&p| id(p)).collect();
    vec![
        json!({ "Target": "Parameter", "Name": "EyeBlink", "Ids": blink }),
        json!({ "Target": "Parameter", "Name": "LipSync", "Ids": lip }),
    ]
}

fn texture_files(
    name: &str,
    pages: &[aether_raster::Pixmap],
    names: Option<&[String]>,
) -> Result<Vec<(String, Vec<u8>)>> {
    let size = pages.iter().map(|p| p.width().max(p.height())).max().unwrap_or(0);
    pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let path = names
                .and_then(|n| n.get(i).cloned())
                .unwrap_or_else(|| format!("{name}.{size}/texture_{i:02}.png"));
            Ok((path, encode_pixmap_png(page)?))
        })
        .collect()
}

/// A document rigged in Aether, as a new Cubism model.
fn new_model(doc: &Document, options: &Live2DExportOptions, name: String) -> Result<Live2DExport> {
    let mut notes = Vec::new();
    let built = native::export_native(doc, options, &mut notes)?;
    let rig = &doc.rig;
    let ids: BTreeMap<ParameterId, String> = built.parameter_ids.iter().cloned().collect();
    let id = |p: ParameterId| ids.get(&p).cloned();
    let target = |p: ParameterId| ids.get(&p).cloned().map(Live2DTarget::Parameter);

    let mut files = vec![(format!("{name}.moc3"), built.moc.write())];
    let textures = texture_files(&name, &built.textures, None)?;
    let texture_paths: Vec<String> = textures.iter().map(|(p, _)| p.clone()).collect();
    files.extend(textures);

    let mut refs = Map::new();
    refs.insert("Moc".into(), json!(format!("{name}.moc3")));
    refs.insert("Textures".into(), json!(texture_paths));
    if let Some(physics) = physics::export_physics(rig, &ids, &mut notes) {
        let path = format!("{name}.physics3.json");
        files.push((path.clone(), json_bytes(&physics)));
        refs.insert("Physics".into(), json!(path));
    }

    // Display names and groups.
    let mut group_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut used = BTreeSet::new();
    for p in &rig.parameters {
        if !p.group.is_empty() && !group_ids.contains_key(&p.group) {
            group_ids.insert(
                p.group.clone(),
                unique_id(&format!("ParamGroup_{}", p.group), "ParamGroup", &mut used),
            );
        }
    }
    let cdi = json!({
        "Version": 3,
        "Parameters": rig.parameters.iter().filter_map(|p| Some(json!({
            "Id": ids.get(&p.id)?,
            "GroupId": group_ids.get(&p.group).cloned().unwrap_or_default(),
            "Name": if p.label.is_empty() { &p.name } else { &p.label },
        }))).collect::<Vec<_>>(),
        "ParameterGroups": group_ids.iter().map(|(name, id)| json!({
            "Id": id, "GroupId": "", "Name": name,
        })).collect::<Vec<_>>(),
        "Parts": built.parts.iter().map(|(id, name)| json!({ "Id": id, "Name": name })).collect::<Vec<_>>(),
    });
    let cdi_path = format!("{name}.cdi3.json");
    files.push((cdi_path.clone(), json_bytes(&cdi)));
    refs.insert("DisplayInfo".into(), json!(cdi_path));

    let motions: Vec<Motion> = if options.bake_drivers {
        rig.motions.clone()
    } else {
        rig.motions.iter().map(|m| with_driven_curves(rig, m)).collect()
    };
    let c = companions(&motions, rig, &target);
    files.extend(c.files);
    if !c.expressions.is_empty() {
        refs.insert("Expressions".into(), Value::Array(c.expressions));
    }
    if !c.motions.is_empty() {
        refs.insert("Motions".into(), Value::Object(c.motions));
    }
    let settings = json!({
        "Version": 3,
        "FileReferences": refs,
        "Groups": groups(rig, &id),
        "HitAreas": [],
    });
    files.push((format!("{name}.model3.json"), json_bytes(&settings)));

    if !rig.behaviours.breath.is_empty() || rig.behaviours.look.enabled {
        notes.push(
            "breathing and look-at are left to the app (VTube Studio and the Cubism SDK have their own)"
                .into(),
        );
    }
    Ok(Live2DExport {
        name,
        files,
        notes,
        max_error: Some(built.max_error),
    })
}

/// An opened Live2D model, written back with the edits made in Aether.
fn pass_through(doc: &Document, model: &CubismRig, name: String) -> Result<Live2DExport> {
    let rig = &doc.rig;
    let mut notes = Vec::new();
    let others = doc
        .layers
        .iter()
        .filter(|l| l.visible && matches!(l.content, LayerContent::Raster(_) | LayerContent::Fill(_)))
        .filter(|l| l.id != model.layer)
        .count();
    if others > 0 {
        notes.push(format!(
            "{others} layer(s) painted in Aether are left out: they cannot be added to an opened Live2D model yet"
        ));
    }
    if rig.cubism.len() > 1 {
        notes.push("only the first Live2D model in the document is exported".into());
    }
    if rig.drivers.iter().any(|d| d.enabled) {
        notes.push("drivers are left out (Cubism has none)".into());
    }

    // Parameter ranges and defaults as edited.
    let mut moc: Moc = (*model.moc).clone();
    let mut ids: BTreeMap<ParameterId, Live2DTarget> = BTreeMap::new();
    for (i, linked) in model.parameters.iter().enumerate() {
        let Some(pid) = linked else { continue };
        ids.insert(*pid, Live2DTarget::Parameter(moc.parameters.ids[i].clone()));
        if let Some(p) = rig.parameter(*pid) {
            if p.min < p.max && p.min <= p.default && p.default <= p.max {
                moc.parameters.min[i] = p.min;
                moc.parameters.max[i] = p.max;
                moc.parameters.default[i] = p.default;
            }
        }
    }
    for (i, linked) in model.parts.iter().enumerate() {
        if let Some(pid) = linked {
            ids.insert(*pid, Live2DTarget::PartOpacity(moc.parts.ids[i].clone()));
        }
    }
    let target = |p: ParameterId| ids.get(&p).cloned();
    let id = |p: ParameterId| match ids.get(&p) {
        Some(Live2DTarget::Parameter(id)) => Some(id.clone()),
        _ => None,
    };
    let unmapped: Vec<&str> = rig
        .motions
        .iter()
        .flat_map(|m| m.tracks.iter())
        .filter(|t| t.enabled && !ids.contains_key(&t.param))
        .filter_map(|t| rig.parameter(t.param).map(|p| p.name.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !unmapped.is_empty() {
        notes.push(format!(
            "motion curves for parameters the model does not have are left out: {}",
            unmapped.join(", ")
        ));
    }

    let original = model.extra.get("model3").cloned().unwrap_or(Value::Null);
    let moc_path = original
        .pointer("/FileReferences/Moc")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{name}.moc3"));
    let mut files = vec![(moc_path.clone(), moc.write())];
    let pages = match doc.layers.get(model.layer).map(|l| &l.content) {
        Some(LayerContent::Live2D(content)) => content.textures.clone(),
        _ => Vec::new(),
    };
    let names = (model.texture_files.len() == pages.len()).then_some(model.texture_files.as_slice());
    let textures = texture_files(&name, &pages, names)?;
    let texture_paths: Vec<String> = textures.iter().map(|(p, _)| p.clone()).collect();
    files.extend(textures);

    let mut refs = Map::new();
    refs.insert("Moc".into(), json!(moc_path));
    refs.insert("Textures".into(), json!(texture_paths));
    let original_ref = |key: &str, default: String| -> String {
        original
            .pointer(&format!("/FileReferences/{key}"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or(default)
    };
    if let Some(physics) = &model.physics {
        let path = original_ref("Physics", format!("{name}.physics3.json"));
        files.push((path.clone(), json_bytes(physics)));
        refs.insert("Physics".into(), json!(path));
    }
    if !model.pose.is_empty() {
        let groups: Vec<Value> = model
            .pose
            .iter()
            .map(|g| {
                Value::Array(
                    g.parts
                        .iter()
                        .map(|m| {
                            json!({
                                "Id": moc.parts.ids[m.part],
                                "Link": m.links.iter().map(|&l| moc.parts.ids[l].clone()).collect::<Vec<_>>(),
                            })
                        })
                        .collect(),
                )
            })
            .collect();
        let pose = json!({ "Type": "Live2D Pose", "FadeInTime": model.pose_fade, "Groups": groups });
        let path = original_ref("Pose", format!("{name}.pose3.json"));
        files.push((path.clone(), json_bytes(&pose)));
        refs.insert("Pose".into(), json!(path));
    }
    // Display names: the original, with names edited in Aether.
    let mut cdi = model
        .extra
        .get("cdi3")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "Version": 3, "Parameters": [], "ParameterGroups": [], "Parts": [] }));
    let labels: BTreeMap<String, String> = ids
        .iter()
        .filter_map(|(pid, t)| {
            let p = rig.parameter(*pid)?;
            let id = match t {
                Live2DTarget::Parameter(id) | Live2DTarget::PartOpacity(id) => id.clone(),
            };
            (!p.label.is_empty()).then(|| (id, p.label.clone()))
        })
        .collect();
    for section in ["Parameters", "Parts"] {
        if let Some(list) = cdi.get_mut(section).and_then(Value::as_array_mut) {
            for entry in list {
                let label = entry
                    .get("Id")
                    .and_then(Value::as_str)
                    .and_then(|i| labels.get(i));
                if let Some(label) = label {
                    entry["Name"] = json!(label);
                }
            }
        }
    }
    let cdi_path = original_ref("DisplayInfo", format!("{name}.cdi3.json"));
    files.push((cdi_path.clone(), json_bytes(&cdi)));
    refs.insert("DisplayInfo".into(), json!(cdi_path));
    if let Some(userdata) = model.extra.get("userdata3").filter(|v| v.is_object()) {
        let path = original_ref("UserData", format!("{name}.userdata3.json"));
        files.push((path.clone(), json_bytes(userdata)));
        refs.insert("UserData".into(), json!(path));
    }

    let c = companions(&rig.motions, rig, &target);
    files.extend(c.files);
    if !c.expressions.is_empty() {
        refs.insert("Expressions".into(), Value::Array(c.expressions));
    }
    if !c.motions.is_empty() {
        refs.insert("Motions".into(), Value::Object(c.motions));
    }
    let hit_areas: Vec<Value> = model
        .hit_areas
        .iter()
        .map(|h| json!({ "Id": h.drawable, "Name": h.name }))
        .collect();
    let mut settings = json!({
        "Version": 3,
        "FileReferences": refs,
        "Groups": groups(rig, &id),
        "HitAreas": hit_areas,
    });
    if let Some(layout) = original.get("Layout") {
        settings["Layout"] = layout.clone();
    }
    files.push((format!("{name}.model3.json"), json_bytes(&settings)));
    Ok(Live2DExport {
        name,
        files,
        notes,
        max_error: None,
    })
}

/// `motion` with the curves its drivers produce: each parameter driven
/// from something the motion animates gets a track of the driven values,
/// so the motion plays in Cubism as it does in the editor.
pub(crate) fn with_driven_curves(rig: &Rig, motion: &Motion) -> Motion {
    let mut out = motion.clone();
    let animated: BTreeSet<ParameterId> = motion
        .tracks
        .iter()
        .filter(|t| t.enabled && !t.keys.is_empty())
        .map(|t| t.param)
        .collect();
    let mut targets = Vec::new();
    for d in rig.drivers.iter().filter(|d| d.enabled && d.mix > 0.0) {
        let Ok(program) = d.compile(&rig.parameters) else {
            continue;
        };
        let reads_animated = program
            .variables
            .iter()
            .filter_map(|&slot| rig.parameters.get(slot))
            .any(|p| p.id != d.target && animated.contains(&p.id));
        if reads_animated && !targets.contains(&d.target) {
            targets.push(d.target);
        }
    }
    if targets.is_empty() {
        return out;
    }
    let fps = motion.fps.clamp(10.0, 60.0);
    let frames = (motion.duration * fps).ceil().max(1.0) as usize;
    let mut samples: BTreeMap<ParameterId, Vec<(f32, f32)>> = BTreeMap::new();
    for f in 0..=frames {
        let time = (f as f32 / fps).min(motion.duration);
        let mut values: ParamValues = motion
            .tracks
            .iter()
            .filter(|t| t.enabled)
            .filter_map(|t| Some((t.param, t.sample(time)?)))
            .collect();
        driver::apply(&rig.parameters, &rig.drivers, &mut values, time);
        for &target in &targets {
            let p = rig.parameter(target);
            let v = values
                .get(&target)
                .copied()
                .or(p.map(|p| p.default))
                .unwrap_or(0.0);
            samples.entry(target).or_default().push((time, v));
        }
    }
    for (target, points) in samples {
        let span = rig.parameter(target).map(|p| p.span()).unwrap_or(1.0);
        let kept = simplify(&points, span * 0.002);
        let track = out.track_mut(target);
        track.enabled = true;
        track.keys = kept
            .into_iter()
            .map(|(time, value)| Keyframe {
                time,
                value,
                easing: Easing::Linear,
            })
            .collect();
    }
    out.tracks.retain(|t: &Track| !t.keys.is_empty());
    out
}

/// Drop points a straight line between their neighbours already passes
/// within `epsilon` of (Douglas-Peucker on value).
fn simplify(points: &[(f32, f32)], epsilon: f32) -> Vec<(f32, f32)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (t0, v0) = points[a];
        let (t1, v1) = points[b];
        let mut worst = (0.0f32, a);
        for (i, &(t, v)) in points.iter().enumerate().take(b).skip(a + 1) {
            let line = if t1 > t0 {
                v0 + (v1 - v0) * (t - t0) / (t1 - t0)
            } else {
                v0
            };
            let e = (v - line).abs();
            if e > worst.0 {
                worst = (e, i);
            }
        }
        if worst.0 > epsilon {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(p, _)| *p)
        .collect()
}

#[cfg(test)]
mod tests;
