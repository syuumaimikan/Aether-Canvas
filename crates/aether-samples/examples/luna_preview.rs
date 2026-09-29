//! Render Luna in a few poses (whole figure) and expressions (face).
//!
//!   cargo run -p aether-samples --example luna_preview -- OUT_DIR

use aether_core::color::Rgba8;
use aether_core::math::IRect;
use aether_document::Document;

fn posed(doc: &Document, values: &[(&str, f32)], expression: Option<usize>) -> Document {
    let mut d = doc.clone();
    // Show the pose as keyed: no driver adds to the parameters.
    d.rig.drivers.clear();
    if let Some(e) = expression {
        let mut values = d.rig.values.clone();
        d.rig.expressions[e].apply(&d.rig.parameters, 1.0, &mut values);
        d.rig.values = values;
    }
    for (name, v) in values {
        let id = d.rig.parameter_named(name).expect(name).id;
        d.rig.set_value(id, *v);
    }
    d
}

fn main() -> aether_core::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&out)?;
    let started = std::time::Instant::now();
    let doc = aether_samples::luna()?;
    eprintln!("built in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
    let compositor = aether_render::Compositor::new();
    let bg = Rgba8::rgb(236, 232, 244);
    let render = |d: &Document| aether_io::animation::flatten(&compositor.render(d), bg);

    let poses: Vec<(&str, Vec<(&str, f32)>)> = vec![
        ("rest", vec![]),
        (
            "turn",
            vec![("AngleX", 30.0), ("AngleY", 10.0), ("BodyAngleX", 8.0)],
        ),
        ("tilt", vec![("AngleZ", -30.0), ("BodyAngleZ", -10.0)]),
        (
            "lean",
            vec![("AngleZ", 20.0), ("BodyAngleZ", 10.0), ("BodyAngleY", -10.0)],
        ),
        (
            "sway",
            vec![("HairSide", 1.0), ("HairBack", 1.0), ("HairFront", 1.0)],
        ),
        (
            "sway back",
            vec![("HairSide", -1.0), ("HairBack", -1.0), ("HairFront", -1.0)],
        ),
    ];
    let mut tiles = Vec::new();
    for (name, values) in &poses {
        let t = std::time::Instant::now();
        let image = render(&posed(&doc, values, None));
        eprintln!("{name}: {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        tiles.push(image.scaled(image.width() * 2 / 5, image.height() * 2 / 5));
    }
    let (sheet, _) = aether_io::animation::pack_sprite_sheet(&tiles, Some(3))?;
    aether_io::save_png(&sheet, out.join("luna-poses.png"))?;

    let face = IRect::new(392, 90, 272, 200);
    let mut faces: Vec<(String, Vec<(&str, f32)>, Option<usize>)> = vec![
        ("rest".into(), vec![], None),
        (
            "blink half".into(),
            vec![("EyeLOpen", 0.4), ("EyeROpen", 0.4)],
            None,
        ),
        ("closed".into(), vec![("EyeLOpen", 0.0), ("EyeROpen", 0.0)], None),
        ("look".into(), vec![("EyeBallX", 1.0), ("EyeBallY", 1.0)], None),
        ("turn".into(), vec![("AngleX", 30.0), ("AngleY", -20.0)], None),
        (
            "turn left".into(),
            vec![("AngleX", -30.0), ("AngleY", 20.0)],
            None,
        ),
    ];
    for (e, expression) in doc.rig.expressions.iter().enumerate() {
        faces.push((expression.name.clone(), vec![], Some(e)));
    }
    let mut tiles = Vec::new();
    for (_, values, e) in &faces {
        let image = render(&posed(&doc, values, *e)).copy_rect(face);
        tiles.push(image.scaled(face.width as u32 * 3 / 2, face.height as u32 * 3 / 2));
    }
    let (sheet, _) = aether_io::animation::pack_sprite_sheet(&tiles, Some(4))?;
    aether_io::save_png(&sheet, out.join("luna-faces.png"))?;

    // The eyes up close.
    let eyes = IRect::new(440, 160, 170, 70);
    let mut tiles = Vec::new();
    for values in [
        vec![],
        vec![("EyeBallX", -1.0), ("EyeBallY", -1.0)],
        vec![("EyeBallX", 1.0), ("EyeBallY", 1.0)],
        vec![("EyeLOpen", 0.7), ("EyeROpen", 0.7), ("EyeBallY", -0.5)],
        vec![("EyeLSmile", 1.0), ("EyeRSmile", 1.0)],
        vec![("EyeLOpen", 0.3), ("EyeROpen", 0.3)],
        vec![("EyeLOpen", 0.12), ("EyeROpen", 0.12)],
    ] {
        let image = render(&posed(&doc, &values, None)).copy_rect(eyes);
        tiles.push(image.scaled(eyes.width as u32 * 3, eyes.height as u32 * 3));
    }
    let (sheet, _) = aether_io::animation::pack_sprite_sheet(&tiles, Some(2))?;
    aether_io::save_png(&sheet, out.join("luna-eyes.png"))?;
    eprintln!(
        "faces: {}",
        faces.iter().map(|f| f.0.as_str()).collect::<Vec<_>>().join(", ")
    );
    Ok(())
}
