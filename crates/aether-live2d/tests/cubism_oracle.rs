//! The evaluator must compute what Live2D Cubism Core computes.
//!
//! Reference data comes from `oracle/run.sh`, which runs Cubism Core itself
//! (for JavaScript, under Node) on Live2D's sample models at their defaults
//! and at random parameter values, and dumps every output. Without that
//! data (it is downloaded, never committed) the test says so and passes.
//!
//! Checked per model: every static array Core reports (ids, ranges,
//! flags, textures, UVs, indices, masks, parents) and, at each parameter
//! set, every vertex position, opacity, draw order, render order,
//! visibility, multiply and screen colour and part opacity. It also checks
//! that the file round-trips through the writer byte for byte.

use aether_live2d::moc3::Moc;
use aether_live2d::Model;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

fn oracle_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("AETHER_LIVE2D_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/live2d-oracle"));
    dir.join("Haru.json").exists().then_some(dir)
}

fn f32s(v: &Value) -> Vec<f32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap() as f32)
        .collect()
}

fn i64s(v: &Value) -> Vec<i64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_evaluator_matches_cubism_core_on_the_sample_models() {
    let Some(dir) = oracle_dir() else {
        eprintln!("no Cubism Core reference data (run crates/aether-live2d/oracle/run.sh); skipping");
        return;
    };
    let mut failures = Vec::new();
    for name in ["Haru", "Hiyori", "Mark", "Natori", "Rice", "Mao", "Wanko", "Ren"] {
        let bytes = std::fs::read(dir.join(format!("models/{name}/{name}.moc3"))).unwrap();
        let moc = match Moc::read(&bytes) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let written = moc.write();
        if written != bytes {
            let first = written.iter().zip(&bytes).position(|(a, b)| a != b);
            failures.push(format!(
                "{name}: writing does not reproduce the file (lengths {} vs {}, first difference at {first:?})",
                written.len(),
                bytes.len()
            ));
        }
        let info: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap())
                .unwrap();
        let bin = std::fs::read(dir.join(format!("{name}.bin"))).unwrap();
        let floats: Vec<f32> = bin
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();

        let mut model = Model::new(Arc::new(moc));
        let m = model.moc().clone();

        // Static data.
        let p = &info["parameters"];
        let mut check = |ok: bool, what: String| {
            if !ok {
                failures.push(format!("{name}: {what}"));
            }
        };
        check(strings(&p["ids"]) == m.parameters.ids, "parameter ids".into());
        check(f32s(&p["min"]) == m.parameters.min, "parameter minima".into());
        check(f32s(&p["max"]) == m.parameters.max, "parameter maxima".into());
        check(
            f32s(&p["defaults"]) == m.parameters.default,
            "parameter defaults".into(),
        );
        check(strings(&info["parts"]["ids"]) == m.parts.ids, "part ids".into());
        check(
            i64s(&info["parts"]["parents"])
                == m.parts.parent_part.iter().map(|&x| x as i64).collect::<Vec<_>>(),
            "part parents".into(),
        );
        let d = &info["drawables"];
        check(strings(&d["ids"]) == m.art_meshes.ids, "drawable ids".into());
        check(
            i64s(&d["textureIndices"]) == m.art_meshes.texture.iter().map(|&x| x as i64).collect::<Vec<_>>(),
            "texture indices".into(),
        );
        check(
            i64s(&d["vertexCounts"])
                == m.art_meshes
                    .vertex_count
                    .iter()
                    .map(|&x| x as i64)
                    .collect::<Vec<_>>(),
            "vertex counts".into(),
        );
        let flags = i64s(&d["constantFlags"]);
        let n = model.drawable_count();
        for i in 0..n {
            if flags[i] & 0x0c != (m.art_meshes.flags[i] & 0x0c) as i64 {
                check(
                    false,
                    format!(
                        "drawable {i} constant flags {} vs {}",
                        flags[i], m.art_meshes.flags[i]
                    ),
                );
            }
            let masks = i64s(&d["masks"][i]);
            let ours: Vec<i64> = model.drawable_masks(i).iter().map(|&x| x as i64).collect();
            if masks[..(d["maskCounts"][i].as_i64().unwrap() as usize).min(masks.len())] != ours[..] {
                check(false, format!("drawable {i} masks {masks:?} vs {ours:?}"));
            }
            if f32s(&d["uvs"][i]) != model.drawable_uvs(i) {
                check(false, format!("drawable {i} uvs"));
            }
            let idx: Vec<u16> = i64s(&d["indices"][i]).iter().map(|&x| x as u16).collect();
            if idx != model.drawable_indices(i) {
                check(false, format!("drawable {i} indices"));
            }
        }

        // Every parameter set.
        let mut offset = 0usize;
        let mut worst = 0.0f32;
        let mut worst_at = String::new();
        for (s, set) in info["sets"].as_array().unwrap().iter().enumerate() {
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
                let core = &floats[offset..offset + vc];
                offset += vc;
                let visible_here = visible[i] != 0;
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
                    if e > worst {
                        worst = e;
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
            if render_orders
                != model
                    .render_orders()
                    .iter()
                    .map(|&x| x as i64)
                    .collect::<Vec<_>>()
            {
                bad.push("render orders differ".into());
            }
            for (i, &o) in part_opacities.iter().enumerate() {
                let ours = model.part_opacities[i];
                if (o - ours).abs() > 1e-6 {
                    bad.push(format!("part {i} opacity {o} vs {ours}"));
                }
            }
            for b in bad.into_iter().take(8) {
                failures.push(format!("{name} set {s}: {b}"));
            }
        }
        let ppu = m.canvas.pixels_per_unit;
        eprintln!(
            "{name}: worst vertex difference {worst:e} units ({:.5} px) at {worst_at}",
            worst * ppu
        );
        if worst * ppu > 0.1 {
            failures.push(format!(
                "{name}: vertices differ by up to {:.4} px ({worst_at})",
                worst * ppu
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
