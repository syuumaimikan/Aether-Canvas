//! Physics must move parameters as the Cubism Framework does.
//!
//! `oracle/run.sh` runs the Framework's own CubismPhysics (JavaScript, under
//! Node) on each sample model's physics3.json: it settles at the defaults,
//! then sweeps every input parameter for 240 frames of mixed lengths
//! (1/144 s to 1/20 s) and records every parameter after each frame. Without
//! that data the test says so and passes.

use aether_live2d::physics::{Parameters, Physics};
use serde_json::Value;
use std::path::PathBuf;

fn oracle_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("AETHER_LIVE2D_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/live2d-oracle"));
    dir.join("Hiyori.physics.json").exists().then_some(dir)
}

fn floats(v: &Value) -> Vec<f32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap() as f32)
        .collect()
}

#[test]
fn physics_matches_the_cubism_framework() {
    let Some(dir) = oracle_dir() else {
        eprintln!("no Cubism Framework reference data (run crates/aether-live2d/oracle/run.sh); skipping");
        return;
    };
    let mut failures = Vec::new();
    let mut checked = 0;
    for name in ["Haru", "Hiyori", "Mark", "Natori", "Rice", "Mao", "Wanko", "Ren"] {
        let Ok(reference) = std::fs::read_to_string(dir.join(format!("{name}.physics.json"))) else {
            continue;
        };
        let reference: Value = serde_json::from_str(&reference).unwrap();
        let params: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{name}.params.json"))).unwrap())
                .unwrap();
        let ids: Vec<String> = params
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect();
        let min: Vec<f32> = params
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["min"].as_f64().unwrap() as f32)
            .collect();
        let max: Vec<f32> = params
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["max"].as_f64().unwrap() as f32)
            .collect();
        let default: Vec<f32> = params
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["default"].as_f64().unwrap() as f32)
            .collect();
        let physics_file = std::fs::read_dir(dir.join(format!("models/{name}")))
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.to_string_lossy().ends_with(".physics3.json"))
            .unwrap();
        let json: Value = serde_json::from_str(&std::fs::read_to_string(physics_file).unwrap()).unwrap();
        let mut physics = Physics::from_json(&json).expect("physics3.json parses");
        physics.bind(&ids);

        let mut values = default.clone();
        {
            let mut p = Parameters {
                values: &mut values,
                min: &min,
                max: &max,
                default: &default,
            };
            physics.stabilize(&mut p);
        }
        let mut worst = 0.0f32;
        let mut worst_at = String::new();
        let mut compare = |ours: &[f32], theirs: &[f32], at: &str| {
            for (i, (a, b)) in ours.iter().zip(theirs).enumerate() {
                let range = (max[i] - min[i]).abs().max(1e-6);
                let e = (a - b).abs() / range;
                if e > worst {
                    worst = e;
                    worst_at = format!("{at} {}", ids[i]);
                }
            }
        };
        compare(&values, &floats(&reference["stabilized"]), "settling");
        for (f, frame) in reference["frames"].as_array().unwrap().iter().enumerate() {
            for (k, v) in frame["set"].as_object().unwrap() {
                values[k.parse::<usize>().unwrap()] = v.as_f64().unwrap() as f32;
            }
            let dt = frame["dt"].as_f64().unwrap();
            let mut p = Parameters {
                values: &mut values,
                min: &min,
                max: &max,
                default: &default,
            };
            physics.evaluate(&mut p, dt);
            compare(&values, &floats(&frame["values"]), &format!("frame {f}"));
        }
        eprintln!(
            "{name}: worst difference {:.2e} of a parameter's range ({worst_at})",
            worst
        );
        if worst > 1e-3 {
            failures.push(format!(
                "{name}: parameters differ by up to {:.4} of their range ({worst_at})",
                worst
            ));
        }
        checked += 1;
    }
    assert!(checked > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
