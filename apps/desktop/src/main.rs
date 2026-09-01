//! Aether Canvas desktop entry point.
//!
//! Everything of substance lives in the library crates; this binary only sets
//! up logging, parses the command line and opens a GPU-backed window.

mod cli;

use aether_ui::AetherApp;
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

/// Logging goes to stderr, filtered by `AETHER_LOG` (or `RUST_LOG`).
fn init_logging() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("AETHER_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}
