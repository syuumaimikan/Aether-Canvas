//! The evaluator must compute what Live2D Cubism Core computes.
//!
//! Reference data comes from `oracle/run.sh`, which runs Cubism Core itself
//! (for JavaScript, under Node) on Live2D's sample models at their defaults
//! and at random parameter values, and dumps every output. Without that
//! data (it is downloaded, never committed) the test says so and passes.
//!
//! Checked per model: everything [`oracle::compare`] checks, and that the
//! file round-trips through the writer byte for byte.

use aether_live2d::moc3::Moc;
use aether_live2d::oracle::{self, CoreDump};

#[test]
fn the_evaluator_matches_cubism_core_on_the_sample_models() {
    let Some(dir) = oracle::oracle_dir().filter(|d| d.join("Haru.json").exists()) else {
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
        let dump = CoreDump::load(&dir.join(name)).unwrap();
        let result = oracle::compare(moc, &dump, 0.1);
        eprintln!(
            "{name}: worst vertex difference {:.5} px at {}",
            result.worst_px, result.worst_at
        );
        failures.extend(result.failures.into_iter().map(|f| format!("{name}: {f}")));
    }
    assert!(
        failures.is_empty(),
        "{} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
