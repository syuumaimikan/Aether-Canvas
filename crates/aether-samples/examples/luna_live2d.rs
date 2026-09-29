//! Export Luna as a Live2D Cubism model.
//!
//!   cargo run --release -p aether-samples --example luna_live2d -- OUT_DIR

fn main() -> aether_core::Result<()> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "luna-live2d".into());
    let doc = aether_samples::luna()?;
    let started = std::time::Instant::now();
    let export = aether_io::live2d_export::export_live2d(&doc, &Default::default())?;
    eprintln!("exported in {:.1} s", started.elapsed().as_secs_f64());
    for (path, bytes) in &export.files {
        eprintln!("  {path}: {} KB", bytes.len() / 1024);
    }
    for note in &export.notes {
        eprintln!("  note: {note}");
    }
    eprintln!("  largest difference: {:?} px", export.max_error);
    let model = aether_io::live2d_export::save_live2d(&export, &out)?;
    eprintln!("wrote {}", model.display());
    Ok(())
}
