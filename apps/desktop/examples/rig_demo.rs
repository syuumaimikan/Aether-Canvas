//! Paint a character, rig it, animate it and export the animation — headless.
//!
//! This is the whole rigging pipeline driven through the public API, the same
//! calls the editor's panels and tools make:
//!
//! 1. paint layers (face, eyes, hair, body…) with anti-aliased shapes;
//! 2. auto-mesh every layer from its alpha;
//! 3. build a deformer hierarchy — a body warp, a neck pivot, a head warp;
//! 4. *generate* a 3D head turn, hair sway, blinking eyes and an opening
//!    mouth instead of shaping dozens of keyforms by hand;
//! 5. add pendulum physics for the hair, drivers, auto-blink and breathing;
//! 6. key a greeting motion and bake lip sync from a (synthesised) WAV;
//! 7. render frames with physics running and export a GIF, a PNG contact
//!    sheet and the `.aether` project.
//!
//! Run with:
//!
//! ```text
//! cargo run --release --example rig_demo -- out-dir
//! ```

use aether_core::color::Rgba8;
use aether_core::math::{Rect, Vec2};
use aether_core::{LayerId, ParameterId};
use aether_document::layer::Layer;
use aether_document::rig::audio::AudioClip;
use aether_document::rig::automesh::{auto_mesh, AutoMeshOptions};
use aether_document::rig::generate::{self, Anchor, HeadTurnOptions};
use aether_document::rig::motion::Easing;
use aether_document::rig::{ArtMesh, Behaviours, Driver, Motion, NodeRef, RigNode};
use aether_document::Document;
use aether_io::animation::{self, AnimationSettings};
use aether_raster::Pixmap;
use aether_render::Compositor;
use std::path::PathBuf;

const W: u32 = 512;
const H: u32 = 640;

fn main() -> aether_core::Result<()> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "rig-demo".to_string()));
    std::fs::create_dir_all(&out)?;

    let (mut doc, layers) = paint_character();
    rig_character(&mut doc, &layers)?;

    let project = out.join("aether-chan.aether");
    aether_io::save_project(&doc, &project)?;
    println!("wrote {}", project.display());

    let compositor = Compositor::new();
    let settings = AnimationSettings {
        motion: Some(0),
        fps: 24.0,
        duration: None,
        simulate: true,
        warmup: 1.0,
        scale: 0.5,
    };
    let frames = animation::render_frames(&doc, &compositor, &settings)?;
    let background = Rgba8::rgb(246, 243, 238);
    let gif = out.join("aether-chan.gif");
    animation::export_gif(&frames, &gif, settings.fps, background)?;
    println!("wrote {} ({} frames)", gif.display(), frames.len());

    // A contact sheet of eight evenly spaced frames, for a quick look.
    let picks: Vec<Pixmap> = (0..8)
        .map(|i| animation::flatten(&frames[i * frames.len() / 8], background))
        .collect();
    let (sheet, _) = animation::pack_sprite_sheet(&picks, Some(4))?;
    let sheet_path = out.join("aether-chan-contact.png");
    aether_io::save_png(&sheet, &sheet_path)?;
    println!("wrote {}", sheet_path.display());

    // The same artwork rigged automatically, from nothing but its layer
    // names, and played with its generated Idle motion plus physics.
    let (mut auto, _) = paint_character();
    let report = auto_rig_document(&mut auto)?;
    println!(
        "auto rig: {} parts, {} deformers, {} physics chains, unrecognised {:?}",
        report.roles.iter().map(|(_, n)| n).sum::<usize>(),
        report.deformers,
        report.physics,
        report.unrecognised
    );
    let auto_settings = AnimationSettings {
        motion: auto.rig.motions.iter().position(|m| m.name == "Idle"),
        ..settings.clone()
    };
    let auto_frames = animation::render_frames(&auto, &compositor, &auto_settings)?;
    animation::export_gif(
        &auto_frames,
        out.join("aether-chan-auto.gif"),
        auto_settings.fps,
        background,
    )?;
    let mut auto_poses = Vec::new();
    for values in [
        vec![],
        vec![("AngleX", 30.0), ("AngleY", 12.0), ("EyeBallX", 0.8)],
        vec![
            ("AngleX", -30.0),
            ("EyeLOpen", 0.0),
            ("EyeROpen", 0.0),
            ("MouthForm", 1.0),
        ],
        vec![
            ("AngleZ", -25.0),
            ("MouthOpenY", 1.0),
            ("Cheek", 1.0),
            ("BrowLY", 1.0),
            ("BrowRY", 1.0),
            ("HairSide", 1.0),
        ],
    ] {
        let mut posed = auto.clone();
        for (param, value) in values {
            if let Some(id) = posed.rig.parameter_named(param).map(|p| p.id) {
                posed.rig.set_value(id, value);
            }
        }
        auto_poses.push(animation::flatten(&compositor.render(&posed), background));
    }
    let (auto_sheet, _) = animation::pack_sprite_sheet(&auto_poses, Some(4))?;
    aether_io::save_png(&auto_sheet, out.join("aether-chan-auto-poses.png"))?;
    aether_io::save_project(&auto, out.join("aether-chan-auto.aether"))?;
    println!("wrote the auto-rigged variant");

    // Full-size stills of a few distinct poses.
    for (name, values) in [
        ("rest", vec![]),
        (
            "turn-right",
            vec![("AngleX", 30.0), ("AngleY", 10.0), ("EyeBallX", 0.6)],
        ),
        (
            "turn-left-blink",
            vec![("AngleX", -30.0), ("EyeLOpen", 0.0), ("EyeROpen", 0.0)],
        ),
        (
            "talk-tilt",
            vec![
                ("AngleZ", 25.0),
                ("MouthOpenY", 1.0),
                ("Cheek", 1.0),
                ("BrowLY", 1.0),
                ("BrowRY", 1.0),
            ],
        ),
    ] {
        let mut posed = doc.clone();
        for (param, value) in values {
            if let Some(id) = posed.rig.parameter_named(param).map(|p| p.id) {
                posed.rig.set_value(id, value);
            }
        }
        let still = animation::flatten(&compositor.render(&posed), background);
        let path = out.join(format!("pose-{name}.png"));
        aether_io::save_png(&still, &path)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

// ------------------------------------------------------------------ painting

/// Layer ids by role.
struct Layers {
    back_hair: LayerId,
    body: LayerId,
    face: LayerId,
    cheeks: LayerId,
    eye_l: LayerId,
    iris_l: LayerId,
    eye_r: LayerId,
    iris_r: LayerId,
    lash_l: LayerId,
    lash_r: LayerId,
    brow_l: LayerId,
    brow_r: LayerId,
    mouth_open: LayerId,
    mouth_line: LayerId,
    side_l: LayerId,
    side_r: LayerId,
    front_hair: LayerId,
}

fn paint_character() -> (Document, Layers) {
    let mut doc = Document::empty(W, H, "Aether-chan");
    let add = |doc: &mut Document, name: &str, clip: bool, paint: &dyn Fn(&mut Pixmap)| -> LayerId {
        let id = doc.next_layer_id();
        let mut layer = Layer::raster(id, name, W, H);
        if let Some(pm) = layer.pixmap_mut() {
            paint(pm);
        }
        layer.clipping = clip;
        let _ = doc.layers.push_top(layer);
        id
    };

    let hair = Rgba8::rgb(92, 64, 150);
    let hair_dark = Rgba8::rgb(64, 42, 112);
    let skin = Rgba8::rgb(255, 226, 206);
    let skin_shade = Rgba8::rgb(240, 196, 176);

    let back_hair = add(&mut doc, "Back hair", false, &|pm| {
        fill_ellipse(pm, 256.0, 250.0, 142.0, 150.0, hair_dark);
        fill_poly(
            pm,
            &[
                (118.0, 250.0),
                (394.0, 250.0),
                (404.0, 470.0),
                (256.0, 500.0),
                (108.0, 470.0),
            ],
            hair_dark,
        );
    });
    let body = add(&mut doc, "Body", false, &|pm| {
        fill_poly(
            pm,
            &[(228.0, 360.0), (284.0, 360.0), (288.0, 440.0), (224.0, 440.0)],
            skin_shade,
        );
        let shirt = Rgba8::rgb(66, 92, 168);
        fill_ellipse(pm, 170.0, 470.0, 52.0, 44.0, shirt);
        fill_ellipse(pm, 342.0, 470.0, 52.0, 44.0, shirt);
        fill_poly(
            pm,
            &[
                (170.0, 428.0),
                (342.0, 428.0),
                (396.0, 470.0),
                (412.0, 640.0),
                (100.0, 640.0),
                (116.0, 470.0),
            ],
            shirt,
        );
        fill_poly(
            pm,
            &[(226.0, 428.0), (286.0, 428.0), (256.0, 486.0)],
            Rgba8::rgb(250, 250, 255),
        );
        stroke(
            pm,
            &[(226.0, 428.0), (256.0, 486.0), (286.0, 428.0)],
            3.0,
            Rgba8::rgb(40, 56, 110),
        );
    });
    let face = add(&mut doc, "Face", false, &|pm| {
        fill_ellipse(pm, 256.0, 252.0, 112.0, 124.0, skin);
        fill_poly(pm, &[(170.0, 300.0), (342.0, 300.0), (256.0, 392.0)], skin);
    });
    let cheeks = add(&mut doc, "Cheeks", false, &|pm| {
        let blush = Rgba8::new(255, 128, 150, 150);
        fill_ellipse(pm, 190.0, 304.0, 22.0, 10.0, blush);
        fill_ellipse(pm, 322.0, 304.0, 22.0, 10.0, blush);
    });
    let eye_white =
        |cx: f32| move |pm: &mut Pixmap| fill_ellipse(pm, cx, 264.0, 30.0, 22.0, Rgba8::rgb(255, 255, 255));
    let iris = |cx: f32| {
        move |pm: &mut Pixmap| {
            fill_ellipse(pm, cx, 268.0, 16.0, 18.0, Rgba8::rgb(58, 126, 204));
            fill_ellipse(pm, cx, 272.0, 11.0, 12.0, Rgba8::rgb(34, 74, 150));
            fill_ellipse(pm, cx, 270.0, 6.0, 7.0, Rgba8::rgb(20, 24, 44));
            fill_ellipse(pm, cx - 5.0, 262.0, 4.0, 4.0, Rgba8::rgb(255, 255, 255));
        }
    };
    let eye_l = add(&mut doc, "Eye white L", false, &eye_white(208.0));
    let iris_l = add(&mut doc, "Iris L", true, &iris(210.0));
    let eye_r = add(&mut doc, "Eye white R", false, &eye_white(304.0));
    let iris_r = add(&mut doc, "Iris R", true, &iris(302.0));
    let lash = |cx: f32, flip: f32| {
        move |pm: &mut Pixmap| {
            let pts: Vec<(f32, f32)> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let a = std::f32::consts::PI * (1.0 - t);
                    (cx + 32.0 * a.cos(), 262.0 - 22.0 * a.sin())
                })
                .collect();
            stroke(pm, &pts, 5.0, Rgba8::rgb(44, 30, 54));
            stroke(
                pm,
                &[(cx + 31.0 * flip, 262.0), (cx + 38.0 * flip, 256.0)],
                4.0,
                Rgba8::rgb(44, 30, 54),
            );
        }
    };
    let lash_l = add(&mut doc, "Lash L", false, &lash(208.0, -1.0));
    let lash_r = add(&mut doc, "Lash R", false, &lash(304.0, 1.0));
    let brow_color = Rgba8::rgb(70, 44, 90);
    let brow_l = add(&mut doc, "Brow L", false, &|pm| {
        stroke(
            pm,
            &[(180.0, 226.0), (206.0, 216.0), (234.0, 220.0)],
            5.0,
            brow_color,
        )
    });
    let brow_r = add(&mut doc, "Brow R", false, &|pm| {
        stroke(
            pm,
            &[(278.0, 220.0), (306.0, 216.0), (332.0, 226.0)],
            5.0,
            brow_color,
        )
    });
    let mouth_open = add(&mut doc, "Mouth open", false, &|pm| {
        fill_ellipse(pm, 256.0, 338.0, 18.0, 14.0, Rgba8::rgb(126, 30, 46));
        fill_ellipse(pm, 256.0, 346.0, 11.0, 5.0, Rgba8::rgb(236, 120, 130));
    });
    let mouth_line = add(&mut doc, "Mouth line", false, &|pm| {
        stroke(
            pm,
            &[(236.0, 330.0), (256.0, 338.0), (276.0, 330.0)],
            3.0,
            Rgba8::rgb(150, 62, 70),
        )
    });
    let side = |flip: f32| {
        move |pm: &mut Pixmap| {
            let x = |v: f32| 256.0 + (v - 256.0) * flip;
            fill_poly(
                pm,
                &[
                    (x(128.0), 186.0),
                    (x(172.0), 196.0),
                    (x(166.0), 320.0),
                    (x(152.0), 430.0),
                    (x(128.0), 404.0),
                    (x(116.0), 300.0),
                ],
                hair,
            );
        }
    };
    let side_l = add(&mut doc, "Side hair L", false, &side(1.0));
    let side_r = add(&mut doc, "Side hair R", false, &side(-1.0));
    let front_hair = add(&mut doc, "Front hair", false, &|pm| {
        fill_poly(
            pm,
            &[
                (134.0, 214.0),
                (146.0, 140.0),
                (200.0, 108.0),
                (256.0, 100.0),
                (312.0, 108.0),
                (366.0, 140.0),
                (378.0, 214.0),
                (358.0, 250.0),
                (338.0, 206.0),
                (314.0, 256.0),
                (290.0, 204.0),
                (262.0, 250.0),
                (238.0, 204.0),
                (210.0, 256.0),
                (184.0, 206.0),
                (156.0, 250.0),
            ],
            hair,
        );
        stroke(
            pm,
            &[(200.0, 124.0), (232.0, 116.0)],
            4.0,
            Rgba8::new(190, 170, 240, 200),
        );
    });
    doc.active_layer = face;
    doc.mark_all_dirty();
    (
        doc,
        Layers {
            back_hair,
            body,
            face,
            cheeks,
            eye_l,
            iris_l,
            eye_r,
            iris_r,
            lash_l,
            lash_r,
            brow_l,
            brow_r,
            mouth_open,
            mouth_line,
            side_l,
            side_r,
            front_hair,
        },
    )
}

// -------------------------------------------------------------------- rigging

fn rig_character(doc: &mut Document, l: &Layers) -> aether_core::Result<()> {
    let ids = &doc.ids;
    let rig = &mut doc.rig;
    rig.add_standard_parameters(ids);
    let p = |rig: &aether_document::rig::Rig, name: &str| -> ParameterId {
        rig.parameter_named(name)
            .map(|p| p.id)
            .unwrap_or(ParameterId::NONE)
    };

    // 1. Mesh every layer from its alpha.
    for layer in doc.layers.iter() {
        let Some(pixmap) = layer.pixmap() else { continue };
        let Some(bounds) = aether_document::rig::automesh::opaque_bounds(pixmap, 8) else {
            continue;
        };
        let options = AutoMeshOptions::for_bounds(bounds, 1.2);
        if let Some(generated) = auto_mesh(pixmap, &options) {
            let mut mesh = ArtMesh::new(layer.id, generated.vertices, generated.triangles);
            mesh.name = layer.name.clone();
            doc.rig.set_mesh(mesh);
        }
    }
    let rig = &mut doc.rig;

    // 2. Hierarchy: Body warp ⊃ Neck pivot ⊃ Head warp ⊃ face parts.
    let head_parts = [
        l.back_hair,
        l.face,
        l.cheeks,
        l.eye_l,
        l.iris_l,
        l.eye_r,
        l.iris_r,
        l.lash_l,
        l.lash_r,
        l.brow_l,
        l.brow_r,
        l.mouth_open,
        l.mouth_line,
        l.side_l,
        l.side_r,
        l.front_hair,
    ];
    let head = generate::wrap_in_warp(rig, ids, &head_parts, "Head", (8, 8), 16.0)?;
    let neck = generate::wrap_in_rotation(
        rig,
        ids,
        &[RigNode::Deformer(head)],
        "Neck",
        Vec2::new(256.0, 380.0),
    )?;
    let body_warp = generate::wrap_in_warp(rig, ids, &[l.body], "Body", (6, 6), 16.0)?;
    rig.set_parent(RigNode::Deformer(neck), Some(NodeRef::Deformer(body_warp)))?;

    // 3. Generated motion ranges.
    generate::head_turn(
        rig,
        head,
        p(rig, "AngleX"),
        Some(p(rig, "AngleY")),
        &HeadTurnOptions::default(),
    )?;
    generate::head_turn(
        rig,
        body_warp,
        p(rig, "BodyAngleX"),
        None,
        &HeadTurnOptions {
            yaw: 14.0,
            depth: 0.45,
            center: Vec2::new(0.5, 0.35),
            ..Default::default()
        },
    )?;
    let angle_z = p(rig, "AngleZ");
    rig.bind_parameter(RigNode::Deformer(neck), angle_z, &[-30.0, 0.0, 30.0])?;
    if let Some(aether_document::rig::Deformer {
        kind: aether_document::rig::DeformerKind::Rotation(r),
        ..
    }) = rig.deformer_mut(neck)
    {
        r.keyforms.forms[0].angle = -12.0;
        r.keyforms.forms[2].angle = 12.0;
    }
    // Breathing lifts the shoulders a little.
    let breath = p(rig, "Breath");
    generate::sway(rig, RigNode::Mesh(l.body), breath, 0.0, Anchor::Bottom)?;
    if let Some(mesh) = rig.mesh_mut(l.body) {
        let top = mesh.bounds().min.y;
        let height = mesh.bounds().height();
        let lift: Vec<Vec2> = mesh
            .vertices
            .iter()
            .map(|v| Vec2::new(0.0, -5.0 * (1.0 - (v.y - top) / height).powi(2)))
            .collect();
        let last = mesh.keyforms.forms.len() - 1;
        mesh.keyforms.forms[last].offsets = lift;
    }

    for (side, eye, iris, lash, open) in [
        ("L", l.eye_l, l.iris_l, l.lash_l, p(rig, "EyeLOpen")),
        ("R", l.eye_r, l.iris_r, l.lash_r, p(rig, "EyeROpen")),
    ] {
        // Eyelids close onto a line two thirds of the way down the eye.
        generate::squash(rig, RigNode::Mesh(eye), open, 0.66)?;
        generate::squash(rig, RigNode::Mesh(iris), open, 0.62)?;
        // The lash slides down to form the closed-eye line.
        rig.bind_parameter(RigNode::Mesh(lash), open, &[0.0, 1.0])?;
        if let Some(mesh) = rig.mesh_mut(lash) {
            let bottom = mesh.bounds().max.y;
            for (o, v) in mesh.keyforms.forms[0].offsets.iter_mut().zip(&mesh.vertices) {
                *o = Vec2::new(0.0, 14.0 + (bottom - v.y) * 0.4);
            }
        }
        // Irises look around.
        for (param, axis) in [
            ("EyeBallX", Vec2::new(9.0, 0.0)),
            ("EyeBallY", Vec2::new(0.0, -6.0)),
        ] {
            let id = p(rig, param);
            rig.bind_parameter(RigNode::Mesh(iris), id, &[-1.0, 0.0, 1.0])?;
            if let Some(mesh) = rig.mesh_mut(iris) {
                let axes = mesh.keyforms.axes.clone();
                let a = axes.iter().position(|x| x.param == id).unwrap_or(0);
                let stride: usize = axes[..a].iter().map(|x| x.keys.len()).product();
                for (i, form) in mesh.keyforms.forms.iter_mut().enumerate() {
                    let k = (i / stride) % 3;
                    let shift = axis * (k as f32 - 1.0);
                    for o in &mut form.offsets {
                        *o += shift;
                    }
                }
            }
        }
        let _ = side;
    }
    for (brow, param) in [(l.brow_l, p(rig, "BrowLY")), (l.brow_r, p(rig, "BrowRY"))] {
        rig.bind_parameter(RigNode::Mesh(brow), param, &[-1.0, 0.0, 1.0])?;
        if let Some(mesh) = rig.mesh_mut(brow) {
            mesh.keyforms.forms[0]
                .offsets
                .iter_mut()
                .for_each(|o| *o = Vec2::new(0.0, 5.0));
            mesh.keyforms.forms[2]
                .offsets
                .iter_mut()
                .for_each(|o| *o = Vec2::new(0.0, -8.0));
        }
    }
    // The open mouth is collapsed shut at rest and opens with MouthOpenY.
    let mouth = p(rig, "MouthOpenY");
    generate::squash(rig, RigNode::Mesh(l.mouth_open), mouth, 0.3)?;
    rig.bind_parameter(RigNode::Mesh(l.mouth_line), mouth, &[0.0, 1.0])?;
    if let Some(mesh) = rig.mesh_mut(l.mouth_line) {
        mesh.keyforms.forms[1]
            .offsets
            .iter_mut()
            .for_each(|o| *o = Vec2::new(0.0, -4.0));
    }
    // Blush fades in with Cheek.
    let cheek = p(rig, "Cheek");
    rig.bind_parameter(RigNode::Mesh(l.cheeks), cheek, &[0.0, 1.0])?;
    if let Some(mesh) = rig.mesh_mut(l.cheeks) {
        mesh.keyforms.forms[0].opacity = 0.0;
    }
    // Hair sways; physics will drive these parameters.
    generate::sway(
        rig,
        RigNode::Mesh(l.front_hair),
        p(rig, "HairFront"),
        14.0,
        Anchor::Top,
    )?;
    generate::sway(
        rig,
        RigNode::Mesh(l.side_l),
        p(rig, "HairSide"),
        26.0,
        Anchor::Top,
    )?;
    generate::sway(
        rig,
        RigNode::Mesh(l.side_r),
        p(rig, "HairSide"),
        26.0,
        Anchor::Top,
    )?;
    generate::sway(
        rig,
        RigNode::Mesh(l.back_hair),
        p(rig, "HairBack"),
        18.0,
        Anchor::Top,
    )?;

    // 4. Physics, drivers and behaviours.
    generate::standard_physics(rig);
    rig.drivers
        .push(Driver::new(p(rig, "BodyAngleX"), "AngleX * 0.3"));
    rig.drivers.push(Driver::new(
        p(rig, "Cheek"),
        "smoothstep(0.2, 0.9, MouthOpenY) * 0.8",
    ));
    rig.behaviours = Behaviours::standard(&rig.parameters);

    // 5. A greeting: look right, nod, tilt, talk, look back.
    let mut motion = Motion::new("Greeting", 4.0, 24.0);
    let ease = Easing::EaseInOut;
    for (param, keys) in [
        (
            "AngleX",
            vec![(0.0, 0.0), (0.8, 24.0), (2.0, 20.0), (3.0, -18.0), (4.0, 0.0)],
        ),
        (
            "AngleY",
            vec![(0.0, 0.0), (1.1, -8.0), (1.5, 8.0), (2.4, 0.0), (4.0, 0.0)],
        ),
        (
            "AngleZ",
            vec![(0.0, 0.0), (1.0, 14.0), (2.4, 10.0), (3.2, -10.0), (4.0, 0.0)],
        ),
        (
            "EyeBallX",
            vec![(0.0, 0.0), (0.7, 0.7), (2.6, 0.5), (3.0, -0.6), (4.0, 0.0)],
        ),
        ("BrowLY", vec![(0.0, 0.0), (1.2, 1.0), (2.2, 0.3), (4.0, 0.0)]),
        ("BrowRY", vec![(0.0, 0.0), (1.2, 1.0), (2.2, 0.3), (4.0, 0.0)]),
    ] {
        let id = p(rig, param);
        let track = motion.track_mut(id);
        for (t, v) in keys {
            let i = track.set_key(t, v);
            track.keys[i].easing = ease;
        }
    }
    // Lip sync baked from synthetic speech: syllable-like bursts.
    let rate = 16_000;
    let samples: Vec<f32> = (0..(rate as f32 * 4.0) as usize)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let talking = (1.2..2.9).contains(&t);
            let syllable = (t * 6.5 * std::f32::consts::TAU).sin().max(0.0);
            let voice = (t * 220.0 * std::f32::consts::TAU).sin() * 0.6
                + (t * 880.0 * std::f32::consts::TAU).sin() * 0.2;
            if talking {
                voice * syllable
            } else {
                0.0
            }
        })
        .collect();
    let clip = AudioClip {
        samples,
        sample_rate: rate,
    };
    for track in clip.bake_lip_sync(24.0, mouth, Some(p(rig, "MouthForm")), 1.3, 0.04) {
        motion.tracks.push(track);
    }
    rig.motions.push(motion);

    rig.validate()?;
    println!(
        "rig: {} parameters, {} meshes ({} vertices), {} deformers, {} physics groups, {} drivers",
        rig.parameters.len(),
        rig.meshes.len(),
        rig.meshes.iter().map(|m| m.vertex_count()).sum::<usize>(),
        rig.deformers.len(),
        rig.physics.len(),
        rig.drivers.len()
    );
    let _ = Rect::ZERO;
    Ok(())
}

/// Mesh every layer and rig the document from its layer names — what the
/// editor's "Auto rig" button does.
fn auto_rig_document(
    doc: &mut Document,
) -> aether_core::Result<aether_document::rig::autorig::AutoRigReport> {
    use aether_document::rig::autorig::{auto_rig, PartInfo};
    let mut parts = Vec::new();
    for layer in doc.layers.iter() {
        let Some(pixmap) = layer.pixmap() else { continue };
        let Some(bounds) = aether_document::rig::automesh::opaque_bounds(pixmap, 8) else {
            continue;
        };
        let Some(generated) = auto_mesh(pixmap, &AutoMeshOptions::for_bounds(bounds, 1.2)) else {
            continue;
        };
        let mesh = ArtMesh::new(layer.id, generated.vertices, generated.triangles);
        parts.push(PartInfo {
            layer: layer.id,
            name: layer.name.clone(),
            groups: Vec::new(),
            bounds: mesh.bounds(),
        });
        doc.rig.set_mesh(mesh);
    }
    auto_rig(&mut doc.rig, &doc.ids, &parts)
}

// ------------------------------------------------------------ shape painting

fn blend(pm: &mut Pixmap, x: i32, y: i32, color: Rgba8, coverage: f32) {
    if coverage <= 0.0 || x < 0 || y < 0 || x >= pm.width() as i32 || y >= pm.height() as i32 {
        return;
    }
    let src_a = color.a as f32 / 255.0 * coverage.min(1.0);
    let dst = pm.get(x, y);
    let dst_a = dst.a as f32 / 255.0;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a <= 0.0 {
        return;
    }
    let mix = |s: u8, d: u8| ((s as f32 * src_a + d as f32 * dst_a * (1.0 - src_a)) / out_a).round() as u8;
    pm.set(
        x,
        y,
        Rgba8::new(
            mix(color.r, dst.r),
            mix(color.g, dst.g),
            mix(color.b, dst.b),
            (out_a * 255.0).round() as u8,
        ),
    );
}

fn fill_ellipse(pm: &mut Pixmap, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgba8) {
    let (x0, x1) = ((cx - rx - 2.0) as i32, (cx + rx + 2.0) as i32);
    let (y0, y1) = ((cy - ry - 2.0) as i32, (cy + ry + 2.0) as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5 - cx) / rx;
            let dy = (y as f32 + 0.5 - cy) / ry;
            let d = (dx * dx + dy * dy).sqrt();
            // Signed distance in pixels, approximately.
            let edge = (1.0 - d) * rx.min(ry);
            blend(pm, x, y, color, (edge + 0.5).clamp(0.0, 1.0));
        }
    }
}

fn fill_poly(pm: &mut Pixmap, points: &[(f32, f32)], color: Rgba8) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let inside = |px: f32, py: f32| {
        let mut odd = false;
        let n = points.len();
        for i in 0..n {
            let (ax, ay) = points[i];
            let (bx, by) = points[(i + 1) % n];
            if (ay > py) != (by > py) && px < ax + (py - ay) / (by - ay) * (bx - ax) {
                odd = !odd;
            }
        }
        odd
    };
    for y in y0 as i32 - 1..=y1 as i32 + 1 {
        for x in x0 as i32 - 1..=x1 as i32 + 1 {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    if inside(
                        x as f32 + (sx as f32 + 0.5) / 4.0,
                        y as f32 + (sy as f32 + 0.5) / 4.0,
                    ) {
                        hits += 1;
                    }
                }
            }
            blend(pm, x, y, color, hits as f32 / 16.0);
        }
    }
}

fn stroke(pm: &mut Pixmap, points: &[(f32, f32)], width: f32, color: Rgba8) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let r = width * 0.5;
    for y in (y0 - r - 2.0) as i32..=(y1 + r + 2.0) as i32 {
        for x in (x0 - r - 2.0) as i32..=(x1 + r + 2.0) as i32 {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let mut d = f32::MAX;
            for pair in points.windows(2) {
                let a = Vec2::new(pair[0].0, pair[0].1);
                let b = Vec2::new(pair[1].0, pair[1].1);
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                d = d.min((a + ab * t).distance(p));
            }
            blend(pm, x, y, color, (r - d + 0.5).clamp(0.0, 1.0));
        }
    }
}
