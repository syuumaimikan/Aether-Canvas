//! Open a Live2D model and render it to a PNG, optionally posed.
//!
//!   cargo run --release -p aether-desktop --example live2d_render -- \
//!       MODEL.model3.json OUT.png [PARAM=VALUE ...] [--motion NAME@SECONDS] [--no-physics]
//!
//! Parameters are rig names (`AngleX`, `EyeLOpen`, `PartArmA`, ...).

use aether_document::rig::RigRuntime;
use aether_io::live2d_model::{import_live2d, Live2DImportOptions};
use aether_render::{Compositor, RenderOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(model), Some(out)) = (args.first(), args.get(1)) else {
        return Err(
            "usage: live2d_render MODEL.model3.json OUT.png [PARAM=VALUE ...] [--motion NAME@SECONDS]".into(),
        );
    };
    let started = std::time::Instant::now();
    let imported = import_live2d(model, &Live2DImportOptions::default())?;
    for note in &imported.notes {
        eprintln!("note: {note}");
    }
    let mut doc = imported.document;
    eprintln!(
        "opened {} ({}×{}, {} parameters, {} motions) in {:.0} ms",
        doc.name,
        doc.width,
        doc.height,
        doc.rig.parameters.len(),
        doc.rig.motions.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    let mut motion = None;
    let mut physics = true;
    let mut rest = args[2..].iter();
    while let Some(arg) = rest.next() {
        if arg == "--no-physics" {
            physics = false;
            continue;
        }
        if arg == "--motion" {
            let spec = rest.next().ok_or("--motion needs NAME@SECONDS")?;
            let (name, time) = spec.split_once('@').ok_or("--motion needs NAME@SECONDS")?;
            motion = Some((name.to_string(), time.parse::<f32>()?));
            continue;
        }
        let (name, value) = arg.split_once('=').ok_or("parameters are NAME=VALUE")?;
        let id = doc
            .rig
            .parameter_named(name)
            .ok_or(format!("no parameter {name}"))?
            .id;
        doc.rig.set_value(id, value.parse()?);
    }
    if let Some((name, time)) = motion {
        let index = doc
            .rig
            .motions
            .iter()
            .position(|m| m.name == name)
            .ok_or(format!("no motion {name}"))?;
        let mut runtime = RigRuntime::new();
        runtime.settings.behaviours = false;
        runtime.settings.physics = physics;
        runtime.play(&doc.rig, index);
        let steps = (time * 60.0).round() as usize;
        for _ in 0..steps {
            runtime.tick(&mut doc.rig, 1.0 / 60.0);
        }
    }
    let started = std::time::Instant::now();
    let image = Compositor::new().render_with(
        &doc,
        &RenderOptions {
            include_background: false,
            ..Default::default()
        },
    );
    eprintln!("rendered in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
    aether_io::save_png(&image, out)?;
    Ok(())
}
