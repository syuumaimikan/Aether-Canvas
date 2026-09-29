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

/// Load a project, PSD, Live2D model or image as a document.
fn load(file: &Path) -> aether_core::Result<aether_document::Document> {
    let extension = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    Ok(match extension.as_str() {
        "aether" => aether_io::load_project(file)?,
        "psd" => aether_io::psd::load_psd_file(file)?,
        _ if name.ends_with(".model3.json") || file.is_dir() => {
            let opened = aether_io::live2d_model::import_live2d(file, &Default::default())?;
            for note in &opened.notes {
                println!("note: {note}");
            }
            opened.document
        }
        _ => {
            let pixmap = aether_io::load_image(file)?;
            let name = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut doc = aether_document::Document::new(pixmap.width(), pixmap.height(), name);
            if let Some(target) = doc.layers.get_mut(doc.active_layer).and_then(|l| l.pixmap_mut()) {
                *target = pixmap;
            }
            doc
        }
    })
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
