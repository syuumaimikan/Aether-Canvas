//! Checking models against Live2D Cubism Core itself.
//!
//! For tests and tooling only. `oracle/run.sh` downloads Cubism Core for
//! JavaScript (it is never committed or shipped); `oracle/dump.mjs` runs it
//! under Node on a `.moc3` and dumps everything it computes at the model's
//! defaults and at random parameter values. [`compare`] holds this crate's
//! evaluator to that dump, and [`dump_with_core`] produces one for any file
//! — which is how files written by [`builder`](crate::builder) and by the
//! editor's exporter are shown to load and deform in Cubism exactly as
//! intended.

use crate::moc3::Moc;
use crate::model::Model;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Where `oracle/run.sh` put Cubism Core and the reference data:
/// `$AETHER_LIVE2D_ORACLE`, or `target/live2d-oracle` in the workspace.
/// `None` until it has been run.
pub fn oracle_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("AETHER_LIVE2D_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/live2d-oracle"));
    dir.join("core.js").exists().then_some(dir)
}

/// What Cubism Core computed for one file.
#[derive(Clone, Debug)]
pub struct CoreDump {
    /// Static data and per-set outputs (see `oracle/dump.mjs`).
    pub info: Value,
    /// Every drawable's vertex positions for each set, in order.
    pub positions: Vec<f32>,
}

impl CoreDump {
    /// Read `PREFIX.json` and `PREFIX.bin`.
    pub fn load(prefix: &Path) -> Result<Self, String> {
        let json = prefix.with_extension("json");
        let bin = prefix.with_extension("bin");
        let text = std::fs::read_to_string(&json).map_err(|e| format!("{}: {e}", json.display()))?;
        let info = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", json.display()))?;
        let bytes = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
        let positions = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Ok(Self { info, positions })
    }

    /// Whether Core's consistency check (`csmHasMocConsistency`) passed.
    pub fn consistent(&self) -> bool {
        self.info["consistent"].as_bool().unwrap_or(false)
    }
}

/// Run Cubism Core on `moc` (a `.moc3` file's bytes) at the defaults and at
/// `sets - 1` random parameter sets. `Ok(None)` when Core or Node is not
/// available; `Err` when Core refuses the file.
pub fn dump_with_core(moc: &[u8], sets: usize) -> Result<Option<CoreDump>, String> {
    let Some(dir) = oracle_dir() else {
        return Ok(None);
    };
    if Command::new("node").arg("--version").output().is_err() {
        return Ok(None);
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("oracle/dump.mjs");
    let work = std::env::temp_dir().join(format!(
        "aether-live2d-oracle-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let file = work.join("model.moc3");
    std::fs::write(&file, moc).map_err(|e| e.to_string())?;
    let prefix = work.join("dump");
    let output = Command::new("node")
        .arg(&script)
        .arg(dir.join("core.js"))
        .arg(&file)
        .arg(&prefix)
        .arg(sets.max(1).to_string())
        .output()
        .map_err(|e| e.to_string())?;
    let result = if output.status.success() {
        CoreDump::load(&prefix)
    } else {
        Err(format!(
            "Cubism Core refused the file: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
        ))
    };
    let _ = std::fs::remove_dir_all(&work);
    result.map(Some)
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn f32s(v: &Value) -> Vec<f32> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN) as f32).collect())
        .unwrap_or_default()
}

fn i64s(v: &Value) -> Vec<i64> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_i64().unwrap_or(i64::MIN)).collect())
        .unwrap_or_default()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default()
}

/// The result of [`compare`].
#[derive(Clone, Debug, Default)]
pub struct Comparison {
    /// Everything that differs, at most eight per parameter set.
    pub failures: Vec<String>,
    /// Largest vertex difference, in canvas pixels.
    pub worst_px: f32,
    /// Where it was.
    pub worst_at: String,
}

/// Compare this crate's evaluation of `moc` with Core's `dump` of it: every
/// static array Core reports (ids, ranges, flags, textures, UVs, indices,
/// masks, parents) and, at each parameter set, every vertex position,
/// opacity, draw order, render order, visibility, multiply and screen
/// colour and part opacity. Vertices may differ by `tolerance_px` canvas
/// pixels.
pub fn compare(moc: Moc, dump: &CoreDump, tolerance_px: f32) -> Comparison {
    let info = &dump.info;
    let floats = &dump.positions;
    let mut failures = Vec::new();
    let mut model = Model::new(Arc::new(moc));
    let m = model.moc().clone();

    // Static data.
    let p = &info["parameters"];
    let mut check = |ok: bool, what: &str| {
        if !ok {
            failures.push(what.to_string());
        }
    };
    check(strings(&p["ids"]) == m.parameters.ids, "parameter ids");
    check(f32s(&p["min"]) == m.parameters.min, "parameter minima");
    check(f32s(&p["max"]) == m.parameters.max, "parameter maxima");
    check(f32s(&p["defaults"]) == m.parameters.default, "parameter defaults");
    check(strings(&info["parts"]["ids"]) == m.parts.ids, "part ids");
    let parents: Vec<i64> = m.parts.parent_part.iter().map(|&x| x as i64).collect();
    check(i64s(&info["parts"]["parents"]) == parents, "part parents");
    let d = &info["drawables"];
    check(strings(&d["ids"]) == m.art_meshes.ids, "drawable ids");
    let textures: Vec<i64> = m.art_meshes.texture.iter().map(|&x| x as i64).collect();
    check(i64s(&d["textureIndices"]) == textures, "texture indices");
    let counts: Vec<i64> = m.art_meshes.vertex_count.iter().map(|&x| x as i64).collect();
    check(i64s(&d["vertexCounts"]) == counts, "vertex counts");
    let flags = i64s(&d["constantFlags"]);
    let n = model.drawable_count();
    for i in 0..n.min(flags.len()) {
        if flags[i] & 0x0c != (m.art_meshes.flags[i] & 0x0c) as i64 {
            failures.push(format!(
                "drawable {i} constant flags {} vs {}",
                flags[i], m.art_meshes.flags[i]
            ));
        }
        let masks = i64s(&d["masks"][i]);
        let ours: Vec<i64> = model.drawable_masks(i).iter().map(|&x| x as i64).collect();
        let count = (d["maskCounts"][i].as_i64().unwrap_or(0) as usize).min(masks.len());
        if masks[..count] != ours[..] {
            failures.push(format!("drawable {i} masks {masks:?} vs {ours:?}"));
        }
        if f32s(&d["uvs"][i]) != model.drawable_uvs(i) {
            failures.push(format!("drawable {i} uvs"));
        }
        let idx: Vec<u16> = i64s(&d["indices"][i]).iter().map(|&x| x as u16).collect();
        if idx != model.drawable_indices(i) {
            failures.push(format!("drawable {i} indices"));
        }
    }
    if !failures.is_empty() {
        return Comparison {
            failures,
            ..Default::default()
        };
    }

    // Every parameter set.
    let mut offset = 0usize;
    let mut worst = 0.0f32;
    let mut worst_at = String::new();
    let sets = info["sets"].as_array().cloned().unwrap_or_default();
    for (s, set) in sets.iter().enumerate() {
        model.parameter_values = f32s(&set["values"]);
        model.update();
        let opacities = f32s(&set["opacities"]);
        let draw_orders = i64s(&set["drawOrders"]);
        let render_orders = i64s(&set["renderOrders"]);
        let visible = i64s(&set["visible"]);
        let part_opacities = f32s(&set["partOpacities"]);
        let multiply = set["multiply"].as_array().map(|_| f32s(&set["multiply"]));
        let screen = set["screen"].as_array().map(|_| f32s(&set["screen"]));
        let mut bad = Vec::new();
        for i in 0..n {
            let vc = m.art_meshes.vertex_count[i] as usize * 2;
            let Some(core) = floats.get(offset..offset + vc) else {
                bad.push("Core's positions are cut short".to_string());
                break;
            };
            offset += vc;
            let visible_here = visible.get(i).copied().unwrap_or(0) != 0;
            if visible_here != model.drawable_visible(i) {
                bad.push(format!(
                    "drawable {i} visible {visible_here} vs {}",
                    model.drawable_visible(i)
                ));
                continue;
            }
            if !visible_here {
                continue;
            }
            for (a, b) in core.iter().zip(model.drawable_positions(i)) {
                let e = (a - b).abs();
                if e.is_nan() || e > worst {
                    worst = if e.is_nan() { f32::INFINITY } else { e };
                    worst_at = format!("set {s} drawable {i} ({})", m.art_meshes.ids[i]);
                }
            }
            if (opacities[i] - model.drawable_opacity(i)).abs() > 1e-5 {
                bad.push(format!(
                    "drawable {i} opacity {} vs {}",
                    opacities[i],
                    model.drawable_opacity(i)
                ));
            }
            if draw_orders[i] != model.drawable_draw_order(i) as i64 {
                bad.push(format!(
                    "drawable {i} draw order {} vs {}",
                    draw_orders[i],
                    model.drawable_draw_order(i)
                ));
            }
            if let (Some(mul), Some(scr)) = (&multiply, &screen) {
                let (om, os) = (model.drawable_multiply(i), model.drawable_screen(i));
                for c in 0..4 {
                    if (mul[i * 4 + c] - om[c]).abs() > 1e-5 || (scr[i * 4 + c] - os[c]).abs() > 1e-5 {
                        bad.push(format!(
                            "drawable {i} colours {:?}/{:?} vs {om:?}/{os:?}",
                            &mul[i * 4..i * 4 + 4],
                            &scr[i * 4..i * 4 + 4]
                        ));
                        break;
                    }
                }
            }
        }
        let ours: Vec<i64> = model.render_orders().iter().map(|&x| x as i64).collect();
        if render_orders != ours {
            bad.push(format!("render orders {render_orders:?} vs {ours:?}"));
        }
        for (i, &o) in part_opacities.iter().enumerate() {
            let ours = model.part_opacities[i];
            if (o - ours).abs() > 1e-6 {
                bad.push(format!("part {i} opacity {o} vs {ours}"));
            }
        }
        failures.extend(bad.into_iter().take(8).map(|b| format!("set {s}: {b}")));
    }
    let worst_px = worst * m.canvas.pixels_per_unit;
    if worst_px > tolerance_px {
        failures.push(format!("vertices differ by up to {worst_px:.4} px ({worst_at})"));
    }
    Comparison {
        failures,
        worst_px,
        worst_at,
    }
}
