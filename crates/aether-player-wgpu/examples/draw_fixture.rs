//! Export the drawing-path fixture (the GPU parity test's scene: a rigged,
//! tinted, fading mesh with a part clipped to it, multiply, screen and add
//! parts, translucency) as a runtime model, with the software player's
//! renders of a few poses as references. Other runtimes' tests (the Godot
//! package's, the web player's) draw the same poses and compare.
//!
//!   cargo run -p aether-player-wgpu --example draw_fixture -- OUT_DIR
//!
//! writes OUT_DIR/model/ and OUT_DIR/reference/ (PNGs and poses.json, in the
//! format the demo character's references use).

use aether_io::runtime_model::export_model_to_dir;
use aether_player::{cpu, Player};

#[path = "../tests/fixture/mod.rs"]
mod fixture;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).ok_or("usage: draw_fixture OUT_DIR")?);
    let doc = fixture::scene();
    let export = export_model_to_dir(&doc, out.join("model"), &Default::default())?;
    if !export.warnings.is_empty() {
        return Err(format!("the fixture should export exactly: {:?}", export.warnings).into());
    }
    let reference = out.join("reference");
    std::fs::create_dir_all(&reference)?;
    let mut player = Player::new(export.model.clone())?;
    let swing = player.parameter_index("Swing").ok_or("no Swing parameter")?;
    let mut listing = Vec::new();
    for value in [0.0f64, 0.4, 1.0] {
        player.reset();
        player.set_parameter(swing, value as f32);
        player.update();
        let name = format!("swing-{value}");
        let file = format!("{name}.png");
        aether_io::save_png(&cpu::render(&player, &export.textures), reference.join(&file))?;
        listing.push(serde_json::json!({
            "name": name,
            "image": file,
            "values": { "Swing": value },
        }));
    }
    std::fs::write(reference.join("poses.json"), serde_json::to_string(&listing)?)?;
    println!("wrote {}", out.display());
    Ok(())
}
