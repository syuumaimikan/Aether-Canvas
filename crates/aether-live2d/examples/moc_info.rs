//! Print a summary of a `.moc3` file.
//!
//!   cargo run -p aether-live2d --example moc_info -- MODEL.moc3 [--full]

use aether_live2d::Moc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("usage: moc_info MODEL.moc3 [--full]")?;
    let full = args.iter().any(|a| a == "--full");
    let moc = Moc::read(&std::fs::read(path)?)?;
    println!("version {}", moc.version);
    println!("canvas {:?}", moc.canvas);
    println!(
        "{} parameters, {} parts, {} deformers ({} warps, {} rotations), {} art meshes, {} glues",
        moc.parameters.ids.len(),
        moc.parts.ids.len(),
        moc.deformers.ids.len(),
        moc.warps.binding.len(),
        moc.rotations.binding.len(),
        moc.art_meshes.ids.len(),
        moc.glues.ids.len()
    );
    let p = &moc.parameters;
    for i in 0..p.ids.len() {
        println!(
            "param {:3} {:24} [{}, {}] default {} repeat {} dp {} tables {}+{}",
            i,
            p.ids[i],
            p.min[i],
            p.max[i],
            p.default[i],
            p.repeat[i],
            p.decimal_places[i],
            p.key_table_off[i],
            p.key_table_len[i]
        );
        if !full && i > 8 {
            println!("...");
            break;
        }
    }
    if full {
        println!("{moc:#?}");
    }
    Ok(())
}
