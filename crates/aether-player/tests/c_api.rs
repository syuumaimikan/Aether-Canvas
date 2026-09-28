//! Compile `runtime/c/play.c` against `include/aether_player.h`, link it to
//! the shared library Cargo built for this test run, and play a model.
//!
//! This proves the header, the exported symbols and the struct layout agree.
//! Skipped only when no C compiler is installed.

use aether_core::math::Vec2;
use aether_core::LayerId;
use aether_player::model::{BlendKind, Model, Node, Part, Texture};
use std::path::{Path, PathBuf};
use std::process::Command;

fn compiler() -> Option<String> {
    let candidates = [
        std::env::var("CC").ok(),
        Some("cc".into()),
        Some("clang".into()),
        Some("gcc".into()),
    ];
    candidates.into_iter().flatten().find(|cc| {
        Command::new(cc)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Build the shared library and return the directory it is in.
///
/// `cargo test` does not produce the `cdylib`, so build it with a nested
/// Cargo into a target directory of its own (the outer build holds the lock
/// on the usual one). It is incremental, so only the first run pays.
fn shared_library() -> PathBuf {
    let exe = std::env::current_exe().expect("test executable");
    // target/<profile>/deps/c_api-<hash> -> target/c-api
    let target = exe.ancestors().nth(3).expect("target dir").join("c-api");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "--offline", "--lib", "-p", "aether-player"])
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .expect("run cargo");
    assert!(status.success(), "building the shared library failed");
    target.join("debug")
}

fn model_json() -> String {
    let mut model = Model::new("c", 64, 48);
    model.textures.push(Texture {
        file: "texture_0.png".into(),
        width: 4,
        height: 4,
    });
    for (i, (name, x)) in [("Back", 0.0), ("Front", 24.0)].into_iter().enumerate() {
        model.parts.push(Part {
            layer: LayerId(i as u64 + 1),
            name: name.into(),
            texture: 0,
            vertices: vec![
                Vec2::new(x, 0.0),
                Vec2::new(x + 40.0, 0.0),
                Vec2::new(x + 40.0, 48.0),
                Vec2::new(x, 48.0),
            ],
            uvs: vec![Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::ONE, Vec2::new(0.0, 1.0)],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            opacity: 0.75,
            blend: if i == 1 {
                BlendKind::Screen
            } else {
                BlendKind::Normal
            },
            transform: None,
        });
    }
    model.tree = vec![
        Node::Part {
            index: 0,
            part: 0,
            clipped: vec![],
        },
        Node::Part {
            index: 1,
            part: 1,
            clipped: vec![],
        },
    ];
    model.to_json()
}

#[test]
fn a_c_program_plays_a_model() {
    let Some(cc) = compiler() else {
        eprintln!("no C compiler; skipping");
        return;
    };
    let lib_dir = shared_library();
    let has_library = [
        "libaether_player.so",
        "libaether_player.dylib",
        "aether_player.dll",
    ]
    .iter()
    .any(|f| lib_dir.join(f).exists());
    assert!(has_library, "no shared library in {}", lib_dir.display());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let work = std::env::temp_dir().join(format!("aether-c-api-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let exe = work.join("play");
    let build = Command::new(&cc)
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror"])
        .arg(root.join("../../runtime/c/play.c"))
        .arg("-I")
        .arg(root.join("include"))
        .arg("-L")
        .arg(&lib_dir)
        .arg("-laether_player")
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run the C compiler");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let model = work.join("model.json");
    std::fs::write(&model, model_json()).unwrap();
    let run = Command::new(&exe)
        .arg(&model)
        .env("LD_LIBRARY_PATH", &lib_dir)
        .env("DYLD_LIBRARY_PATH", &lib_dir)
        .env(
            "PATH",
            std::env::join_paths(std::iter::once(lib_dir.clone()).chain(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            )))
            .expect("PATH"),
        )
        .output()
        .expect("run the C program");
    let _ = std::fs::remove_dir_all(&work);
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(stdout.contains("api 1, canvas 64x48, 2 parts"), "{stdout}");
    assert!(
        stdout.contains("draw Back           blend 0 opacity 0.75 mask -1 first vertex (0.0, 0.0)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("draw Front          blend 2 opacity 0.75"),
        "{stdout}"
    );
    assert!(stdout.contains("2 draw calls, 4 triangles"), "{stdout}");
    assert!(stdout.contains("hit at centre: Front"), "{stdout}");
}
