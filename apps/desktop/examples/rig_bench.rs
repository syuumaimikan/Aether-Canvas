//! How long a rigged document takes to composite while it is being posed.
//!
//! ```text
//! cargo run --release -p aether-desktop --example rig_demo -- demo
//! cargo run --release -p aether-desktop --example rig_bench -- demo/aether-chan.aether
//! ```
//!
//! Reports, per frame: compositing with deformation off, compositing a
//! posed rig, the mesh-drawing share of that, and rig evaluation alone.
//! Set `RAYON_NUM_THREADS=1` to see single-core numbers.

use aether_raster::mesh::{draw_textured_mesh, MeshDrawOptions};
use aether_raster::Pixmap;
use aether_render::{Compositor, RenderOptions};
use std::time::Instant;

const FRAMES: u32 = 30;

fn per_frame(mut f: impl FnMut()) -> f64 {
    f(); // warm up
    let start = Instant::now();
    for _ in 0..FRAMES {
        f();
    }
    start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64
}

fn main() -> aether_core::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "demo/aether-chan.aether".to_string());
    let doc = aether_io::load_project(&path)?;
    let compositor = Compositor::new();
    let mut target = Pixmap::new(doc.width, doc.height);

    let flat_options = RenderOptions {
        deform: false,
        ..Default::default()
    };
    let flat = per_frame(|| compositor.render_into(&doc, &mut target, &flat_options));

    let mut posed = doc.clone();
    for (name, value) in [
        ("AngleX", 25.0),
        ("AngleY", 10.0),
        ("AngleZ", 12.0),
        ("BodyAngleX", 6.0),
    ] {
        if let Some(id) = posed.rig.parameter_named(name).map(|p| p.id) {
            posed.rig.set_value(id, value);
        }
    }
    let options = RenderOptions::default();
    let composite = per_frame(|| compositor.render_into(&posed, &mut target, &options));
    let evaluate = per_frame(|| {
        let _ = posed.rig.evaluate();
    });

    let pose = posed.rig.evaluate();
    let deformed: Vec<_> = pose.meshes.values().filter(|m| !m.rest).collect();
    let meshes = per_frame(|| {
        for mesh_pose in &deformed {
            let (Some(texture), Some(mesh)) = (
                posed.layers.get(mesh_pose.layer).and_then(|l| l.pixmap()),
                posed.rig.mesh(mesh_pose.layer),
            ) else {
                continue;
            };
            let region = mesh_pose
                .bounds()
                .to_irect_outer()
                .expanded(1)
                .intersect(&posed.bounds());
            let mut dst = Pixmap::new(posed.width, posed.height);
            draw_textured_mesh(
                &mut dst,
                texture,
                &mesh_pose.positions,
                &mesh.vertices,
                &mesh.triangles,
                &MeshDrawOptions::new(region),
            );
        }
    });

    println!(
        "{}×{}, {} layers, {} deformed meshes, {} threads",
        doc.width,
        doc.height,
        doc.layers.len(),
        deformed.len(),
        rayon::current_num_threads()
    );
    println!("  composite, deformation off  {flat:7.2} ms");
    println!("  composite, posed            {composite:7.2} ms");
    println!("    of which mesh drawing     {meshes:7.2} ms");
    println!("  rig evaluation              {evaluate:7.2} ms");
    Ok(())
}
