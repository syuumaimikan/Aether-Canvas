//! Aether Canvas desktop entry point.
//!
//! Everything of substance lives in the library crates; this binary only sets
//! up logging, parses the command line and opens a GPU-backed window.

mod cli;

use aether_ui::AetherApp;
use std::path::Path;
use std::process::ExitCode;

/// Initial window size, in logical points.
const DEFAULT_WINDOW: [f32; 2] = [1600.0, 1000.0];
/// Smallest window that still fits the docked layout.
const MIN_WINDOW: [f32; 2] = [900.0, 600.0];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = cli::parse(&args);

    if cli.help {
        print!("{}", cli::USAGE);
        return ExitCode::SUCCESS;
    }
    if cli.version {
        println!("aether-canvas {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if !cli.unknown.is_empty() {
        eprintln!("unrecognised arguments: {}", cli.unknown.join(", "));
        eprint!("{}", cli::USAGE);
        return ExitCode::FAILURE;
    }

    init_logging();

    if cli.list {
        return list(cli.open.as_deref());
    }
    if cli.export_model.is_some() && cli.export_live2d.is_some() {
        eprintln!("choose one of --export-model and --export-live2d");
        return ExitCode::FAILURE;
    }
    if let Some(dir) = &cli.export_model {
        return export(cli.open.as_deref(), dir, cli.auto_rig, Target::Runtime);
    }
    if let Some(dir) = &cli.export_live2d {
        return export(cli.open.as_deref(), dir, cli.auto_rig, Target::Live2D);
    }
    if cli.auto_rig {
        eprintln!("--auto-rig only applies together with --export-model or --export-live2d");
        return ExitCode::FAILURE;
    }

    // wgpu keeps the door open to Vulkan, Metal and DX12 from one code path,
    // which is what the compositor's GPU backend will target.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(DEFAULT_WINDOW)
            .with_min_inner_size(MIN_WINDOW)
            .with_title("Aether Canvas"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    let open = cli.open.clone();
    let result = eframe::run_native(
        "Aether Canvas",
        options,
        Box::new(move |cc| Ok(Box::new(AetherApp::new(cc, open.clone())))),
    );

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // A missing display or GPU is the usual cause; say so plainly
            // instead of printing a backtrace at the user.
            eprintln!("Aether Canvas could not start: {error}");
            tracing::error!("startup failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What an export writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    /// `--export-model`: model.json and texture atlases.
    Runtime,
    /// `--export-live2d`: a Live2D Cubism model.
    Live2D,
}

/// Load a project, PSD, Live2D model, image, or an entry of a ZIP archive
/// (`pack.zip#path/in/it`) as a document.
fn load(file: &Path) -> aether_core::Result<aether_document::Document> {
    let opened = aether_io::library::open_spec(&file.to_string_lossy(), &Default::default())?;
    for note in &opened.notes {
        println!("note: {note}");
    }
    Ok(opened.document)
}

/// `--list`: what a folder or archive holds that can be opened.
fn list(file: Option<&Path>) -> ExitCode {
    let root = file.map(Path::to_path_buf).unwrap_or_else(|| {
        aether_io::library::default_sample_folders()
            .into_iter()
            .next()
            .unwrap_or_else(|| Path::new("assets_sample").to_path_buf())
    });
    let items = if aether_io::archive::is_archive(&root) {
        match aether_io::archive::Archive::open(&root) {
            Ok(archive) => archive
                .entries()
                .into_iter()
                .map(|e| aether_io::library::LibraryItem {
                    file: root.clone(),
                    entry: Some(e.path),
                    kind: e.kind,
                    size: e.size,
                })
                .collect(),
            Err(error) => {
                eprintln!("could not read {}: {error}", root.display());
                return ExitCode::FAILURE;
            }
        }
    } else if root.is_dir() {
        aether_io::library::scan_folder(&root)
    } else {
        eprintln!("{} is neither a folder nor a ZIP archive", root.display());
        return ExitCode::FAILURE;
    };
    if items.is_empty() {
        println!("nothing to open in {}", root.display());
    }
    for item in &items {
        println!(
            "{:<14} {:>9}  {}",
            item.kind.label(),
            format!("{:.1} MB", item.size as f64 / 1e6),
            item.spec()
        );
    }
    ExitCode::SUCCESS
}

/// `--export-model` / `--export-live2d`: load a file, optionally auto-rig
/// it, and write the model — no window, for build pipelines.
fn export(file: Option<&Path>, dir: &Path, auto_rig: bool, target: Target) -> ExitCode {
    let Some(file) = file else {
        eprintln!("exporting needs a FILE to export");
        return ExitCode::FAILURE;
    };
    let run = || -> aether_core::Result<()> {
        let mut doc = load(file)?;
        if auto_rig {
            let report = aether_document::rigging::auto_rig_document(&mut doc, 1.0)?;
            let parts: usize = report.roles.iter().map(|(_, n)| n).sum();
            println!(
                "auto rig: {parts} parts recognised, {} deformers, {} physics chains",
                report.deformers, report.physics
            );
            if !report.unrecognised.is_empty() {
                println!("auto rig: left static: {}", report.unrecognised.join(", "));
            }
            if report.variant_parts > 0 {
                println!(
                    "auto rig: {} parts drawn as A/B alternatives switch with the Variant parameter",
                    report.variant_parts
                );
            }
            if !report.ignored.is_empty() {
                println!(
                    "auto rig: reference art and backgrounds left out: {}",
                    report.ignored.join(", ")
                );
            }
        }
        match target {
            Target::Runtime => {
                let export = aether_io::runtime_model::export_model_to_dir(&doc, dir, &Default::default())?;
                for warning in &export.warnings {
                    println!("note: {warning}");
                }
                println!(
                    "wrote {} ({} parts, {} parameters, {} motions, {} texture pages)",
                    dir.join(aether_io::runtime_model::MODEL_FILE).display(),
                    export.model.parts.len(),
                    export.model.rig.parameters.len(),
                    export.model.rig.motions.len(),
                    export.textures.len()
                );
            }
            Target::Live2D => {
                let export = aether_io::live2d_export::export_live2d_to_dir(&doc, dir, &Default::default())?;
                for note in &export.notes {
                    println!("note: {note}");
                }
                print!(
                    "wrote {} ({} files",
                    dir.join(export.model_file()).display(),
                    export.files.len()
                );
                if let Some(error) = export.max_error {
                    print!("; Cubism matches the editor within {error:.1} px");
                }
                println!(")");
            }
        }
        Ok(())
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("could not export {}: {error}", file.display());
            ExitCode::FAILURE
        }
    }
}

/// Logging goes to stderr, filtered by `AETHER_LOG` (or `RUST_LOG`).
fn init_logging() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("AETHER_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}
