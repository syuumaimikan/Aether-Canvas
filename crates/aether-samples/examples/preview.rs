//! Render the rigged sample character in a few poses and expressions.
//!
//!   cargo run -p aether-samples --example preview -- OUT.png

use aether_core::color::Rgba8;

fn main() -> aether_core::Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "aether-chan.png".into());
    let started = std::time::Instant::now();
    let doc = aether_samples::aether_chan()?;
    eprintln!(
        "painted and rigged in {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let compositor = aether_render::Compositor::new();
    let bg = Rgba8::rgb(244, 241, 236);
    let mut tiles = Vec::new();
    let poses: Vec<(&str, Vec<(&str, f32)>)> = vec![
        ("rest", vec![]),
        (
            "turn",
            vec![
                ("AngleX", 30.0),
                ("AngleY", 10.0),
                ("BodyAngleX", 6.0),
                ("EyeBallX", 0.7),
            ],
        ),
        (
            "tilt",
            vec![
                ("AngleZ", -24.0),
                ("BodyAngleZ", -8.0),
                ("AngleX", -20.0),
                ("HairSide", 1.0),
                ("HairFront", 0.8),
                ("HairBack", 1.0),
            ],
        ),
    ];
    for (_, values) in &poses {
        let mut d = doc.clone();
        for (name, v) in values {
            let id = d.rig.parameter_named(name).expect(name).id;
            d.rig.set_value(id, *v);
        }
        tiles.push(aether_io::animation::flatten(&compositor.render(&d), bg));
    }
    for e in 0..doc.rig.expressions.len() {
        let mut d = doc.clone();
        let mut values = d.rig.values.clone();
        d.rig.expressions[e].apply(&d.rig.parameters, 1.0, &mut values);
        d.rig.values = values;
        tiles.push(aether_io::animation::flatten(&compositor.render(&d), bg));
    }
    let small: Vec<_> = tiles
        .iter()
        .map(|t| t.scaled(t.width() / 2, t.height() / 2))
        .collect();
    let (sheet, _) = aether_io::animation::pack_sprite_sheet(&small, Some(4))?;
    aether_io::save_png(&sheet, &out)?;
    eprintln!("wrote {out}");
    Ok(())
}
