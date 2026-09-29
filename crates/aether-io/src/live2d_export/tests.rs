use super::*;
use aether_core::color::Rgba8;
use aether_core::math::{vec2, IRect, Rect};
use aether_core::LayerId;
use aether_document::rig::{ArtMesh, Bone, KeyAxis, NodeRef, Parameter, Skin};
use aether_document::rigging::auto_rig_document;
use aether_live2d::oracle;

fn paint(doc: &mut Document, name: &str, rect: IRect, color: Rgba8) -> LayerId {
    let id = doc.add_raster_layer(name);
    if let Some(pixmap) = doc.layers.get_mut(id).and_then(|l| l.pixmap_mut()) {
        pixmap.fill_rect(rect, color);
    }
    id
}

/// A character painted in named parts and rigged with one click.
fn character() -> Document {
    let mut doc = Document::empty(512, 640, "Test character");
    let skin = Rgba8::new(250, 220, 200, 255);
    let hair = Rgba8::new(90, 60, 120, 255);
    paint(&mut doc, "Back hair", IRect::new(110, 110, 290, 370), hair);
    paint(
        &mut doc,
        "Body",
        IRect::new(100, 400, 300, 240),
        Rgba8::new(60, 90, 160, 255),
    );
    paint(&mut doc, "Neck", IRect::new(225, 360, 60, 60), skin);
    paint(&mut doc, "Face", IRect::new(150, 130, 210, 260), skin);
    for (side, x) in [("L", 180), ("R", 272)] {
        paint(
            &mut doc,
            &format!("Eye white {side}"),
            IRect::new(x, 240, 60, 45),
            Rgba8::WHITE,
        );
        paint(
            &mut doc,
            &format!("Iris {side}"),
            IRect::new(x + 15, 245, 30, 40),
            Rgba8::new(60, 120, 90, 255),
        );
        paint(
            &mut doc,
            &format!("Eyelash {side}"),
            IRect::new(x - 5, 230, 70, 25),
            Rgba8::BLACK,
        );
        paint(
            &mut doc,
            &format!("Brow {side}"),
            IRect::new(x, 210, 55, 15),
            hair,
        );
    }
    paint(
        &mut doc,
        "Mouth",
        IRect::new(236, 320, 40, 20),
        Rgba8::new(200, 80, 80, 255),
    );
    paint(
        &mut doc,
        "Cheek",
        IRect::new(180, 295, 150, 20),
        Rgba8::new(255, 150, 150, 120),
    );
    paint(&mut doc, "Front hair", IRect::new(130, 100, 250, 160), hair);
    paint(
        &mut doc,
        "Ribbon",
        IRect::new(330, 90, 60, 50),
        Rgba8::new(220, 40, 60, 255),
    );
    auto_rig_document(&mut doc, 1.0).expect("auto rig");
    doc
}

/// Cubism Core loads the file, passes its consistency check, and deforms
/// it as our evaluator does (skipped without Cubism Core).
fn check_with_core(moc: &[u8]) {
    let dump = match oracle::dump_with_core(moc, 12) {
        Ok(Some(d)) => d,
        Ok(None) => {
            eprintln!("no Cubism Core (crates/aether-live2d/oracle/run.sh); skipping");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    assert!(dump.consistent(), "Cubism Core's consistency check fails");
    let moc = Moc::read(moc).expect("reads");
    let result = oracle::compare(moc, &dump, 0.05);
    assert!(result.failures.is_empty(), "{}", result.failures.join("\n"));
}

fn json(export: &Live2DExport, path: &str) -> Value {
    serde_json::from_slice(export.file(path).unwrap_or_else(|| panic!("no {path}"))).expect("JSON")
}

#[test]
fn ids_are_cubism_safe_and_unique() {
    let mut used = BTreeSet::new();
    assert_eq!(unique_id("ParamAngleX", "Param", &mut used), "ParamAngleX");
    assert_eq!(unique_id("ParamAngleX", "Param", &mut used), "ParamAngleX_2");
    assert_eq!(unique_id("Hair front", "Param", &mut used), "Hair_front");
    assert_eq!(unique_id("2nd", "Param", &mut used), "Param_2nd");
    let japanese = unique_id("前髪", "Param", &mut used);
    assert!(japanese.starts_with("Param") && japanese.len() > 5, "{japanese}");
    assert_ne!(unique_id("後ろ髪", "Param", &mut used), japanese);
}

#[test]
fn a_rigged_character_exports_as_a_cubism_model() {
    let doc = character();
    let export = export_live2d(&doc, &Live2DExportOptions::default()).expect("exports");
    assert_eq!(export.name, "Test_character");
    let settings = json(&export, "Test_character.model3.json");
    assert_eq!(settings["Version"], 3);
    // Every referenced file is in the export.
    let refs = &settings["FileReferences"];
    let mut referenced = vec![
        refs["Moc"].as_str().unwrap().to_string(),
        refs["Physics"].as_str().expect("physics").to_string(),
        refs["DisplayInfo"].as_str().unwrap().to_string(),
    ];
    referenced.extend(
        refs["Textures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string()),
    );
    for group in refs["Motions"].as_object().expect("motions").values() {
        referenced.extend(
            group
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["File"].as_str().unwrap().to_string()),
        );
    }
    for path in &referenced {
        assert!(export.file(path).is_some(), "{path} is missing");
    }
    assert!(
        refs["Motions"].get("Idle").is_some(),
        "the idle motion plays as Cubism's Idle group"
    );
    let blink = settings["Groups"][0]["Ids"].as_array().unwrap();
    assert!(blink.iter().any(|id| id == "ParamEyeLOpen"), "{blink:?}");

    let moc = Moc::read(export.file("Test_character.moc3").unwrap()).expect("valid moc3");
    assert_eq!(moc.version, moc3::VERSION_42);
    assert!(moc.parameters.ids.iter().any(|id| id == "ParamAngleX"));
    assert_eq!(moc.art_meshes.ids.len(), doc.rig.meshes.len());
    // Textures are power-of-two squares.
    let page = crate::image_io::decode_image(export.file(refs["Textures"][0].as_str().unwrap()).unwrap())
        .expect("png");
    assert_eq!(page.width(), page.height());
    assert!(page.width().is_power_of_two());

    // Cubism deforms it as Aether does.
    let error = export.max_error.expect("checked");
    assert!(error < 2.0, "up to {error} px off: {:#?}", export.notes);
    check_with_core(export.file("Test_character.moc3").unwrap());

    // The physics fitted to the rig's.
    let physics = json(&export, refs["Physics"].as_str().unwrap());
    assert!(physics["Meta"]["PhysicsSettingCount"].as_u64().unwrap() >= 1);
    // Names in Aether show in Cubism apps.
    let cdi = json(&export, "Test_character.cdi3.json");
    assert!(cdi["Parts"].as_array().is_some());
    assert!(cdi["Parameters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["Id"] == "ParamAngleX"));
}

#[test]
fn exported_models_open_again() {
    let doc = character();
    let dir = tempfile::tempdir().expect("tempdir");
    let export = export_live2d_to_dir(&doc, dir.path(), &Live2DExportOptions::default()).expect("exports");
    let back = crate::live2d_model::import_live2d(
        dir.path().join(export.model_file()),
        &crate::live2d_model::Live2DImportOptions::default(),
    )
    .expect("opens");
    let rig = &back.document.rig;
    assert_eq!(rig.cubism.len(), 1);
    assert_eq!(rig.motions.len(), doc.rig.motions.len());
    assert!(rig.parameter_named("AngleX").is_some());
    assert!(rig.cubism[0].physics.is_some());
    // And exports again as it came.
    let again = export_live2d(&back.document, &Live2DExportOptions::default()).expect("exports again");
    assert_eq!(
        again.file(&format!("{}.moc3", again.name)),
        export.file(&format!("{}.moc3", export.name)),
    );
}

/// An arm: an upper-arm bone turned by one parameter and a forearm by
/// another, a sleeve riding the upper arm and a hand riding the forearm
/// rigidly, and a skinned forearm bending between them.
fn arm() -> Document {
    let mut doc = Document::empty(400, 300, "Arm");
    let sleeve = paint(
        &mut doc,
        "Sleeve",
        IRect::new(90, 130, 120, 40),
        Rgba8::new(40, 60, 160, 255),
    );
    let forearm = paint(
        &mut doc,
        "Forearm",
        IRect::new(190, 135, 120, 30),
        Rgba8::new(250, 210, 190, 255),
    );
    let hand = paint(
        &mut doc,
        "Hand",
        IRect::new(300, 130, 40, 40),
        Rgba8::new(240, 200, 180, 255),
    );
    let rig = &mut doc.rig;
    let shoulder = rig
        .add_parameter(Parameter::new(doc.ids.parameter(), "Shoulder", -30.0, 30.0, 0.0))
        .unwrap();
    let elbow = rig
        .add_parameter(Parameter::new(doc.ids.parameter(), "Elbow", 0.0, 1.0, 0.0))
        .unwrap();
    let upper_id = doc.ids.bone();
    let mut upper = Bone::new(upper_id, "Upper arm", vec2(100.0, 150.0), vec2(200.0, 150.0));
    upper
        .keyforms
        .add_axis(KeyAxis::new(shoulder, [-30.0, 30.0]).unwrap())
        .unwrap();
    upper.keyforms.forms[0].rotation = -40.0;
    upper.keyforms.forms[1].rotation = 40.0;
    let lower_id = doc.ids.bone();
    let mut lower = Bone::new(lower_id, "Forearm", vec2(200.0, 150.0), vec2(300.0, 150.0));
    lower.parent = Some(upper_id);
    lower
        .keyforms
        .add_axis(KeyAxis::new(elbow, [0.0, 1.0]).unwrap())
        .unwrap();
    lower.keyforms.forms[1].rotation = 90.0;
    rig.bones.push(upper);
    rig.bones.push(lower);

    let mut sleeve_mesh = ArtMesh::quad(sleeve, Rect::from_corners(vec2(90.0, 130.0), vec2(210.0, 170.0)));
    sleeve_mesh.parent = Some(NodeRef::Bone(upper_id));
    rig.set_mesh(sleeve_mesh);
    let mut hand_mesh = ArtMesh::quad(hand, Rect::from_corners(vec2(300.0, 130.0), vec2(340.0, 170.0)));
    hand_mesh.parent = Some(NodeRef::Bone(lower_id));
    rig.set_mesh(hand_mesh);

    // A strip of vertices along the forearm, blending from the upper arm
    // into the forearm across the elbow.
    let mut vertices = Vec::new();
    for i in 0..=6 {
        let x = 190.0 + i as f32 * 20.0;
        vertices.push(vec2(x, 135.0));
        vertices.push(vec2(x, 165.0));
    }
    let triangles = (0..6u32)
        .flat_map(|i| {
            let a = i * 2;
            [[a, a + 1, a + 2], [a + 1, a + 3, a + 2]]
        })
        .collect();
    let mut mesh = ArtMesh::new(forearm, vertices.clone(), triangles);
    let mut skin = Skin::new(vec![upper_id, lower_id], vertices.len());
    for (v, p) in vertices.iter().enumerate() {
        let t = ((p.x - 190.0) / 40.0).clamp(0.0, 1.0);
        skin.set_weight(v, 0, 1.0 - t);
        skin.set_weight(v, 1, t);
    }
    mesh.skin = Some(skin);
    rig.set_mesh(mesh);
    doc
}

#[test]
fn bones_become_rotation_deformers() {
    let doc = arm();
    let export = export_live2d(&doc, &Live2DExportOptions::default()).expect("exports");
    let moc = Moc::read(export.file("Arm.moc3").unwrap()).unwrap();
    assert_eq!(moc.rotations.binding.len(), 2, "one rotation per bone");
    // The forearm bone turns inside the upper arm's.
    let lower = moc
        .deformers
        .ids
        .iter()
        .position(|id| id == "Bone_Forearm")
        .unwrap();
    let upper = moc
        .deformers
        .ids
        .iter()
        .position(|id| id == "Bone_Upper_arm")
        .unwrap();
    assert_eq!(moc.deformers.parent_deformer[lower], upper as i32);
    let error = export.max_error.unwrap();
    assert!(error < 1.5, "up to {error} px off: {:#?}", export.notes);
    check_with_core(export.file("Arm.moc3").unwrap());
}

#[test]
fn opened_models_export_as_they_came() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/live2d-oracle/models/Hiyori/Hiyori.model3.json");
    if !path.exists() {
        eprintln!("no Live2D samples (crates/aether-live2d/oracle/run.sh); skipping");
        return;
    }
    let opened = crate::live2d_model::import_live2d(&path, &Default::default()).expect("opens");
    let export = export_live2d(&opened.document, &Live2DExportOptions::default()).expect("exports");
    assert!(export.notes.is_empty(), "{:?}", export.notes);
    let original = std::fs::read(path.with_file_name("Hiyori.moc3")).unwrap();
    let settings = json(&export, &export.model_file());
    let moc_path = settings["FileReferences"]["Moc"].as_str().unwrap();
    assert_eq!(
        export.file(moc_path).unwrap(),
        &original[..],
        "the moc3 is unchanged"
    );
    // Every motion and expression, physics, pose and display names.
    let motions: usize = settings["FileReferences"]["Motions"]
        .as_object()
        .unwrap()
        .values()
        .map(|g| g.as_array().unwrap().len())
        .sum();
    assert_eq!(motions, opened.document.rig.motions.len());
    for key in ["Physics", "Pose", "DisplayInfo"] {
        let file = settings["FileReferences"][key]
            .as_str()
            .unwrap_or_else(|| panic!("no {key}"));
        assert!(export.file(file).is_some(), "{file}");
    }
    // The motions still move the same parameters.
    let original_settings: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let first = original_settings["FileReferences"]["Motions"]["Idle"][0]["File"]
        .as_str()
        .unwrap();
    let before: Value = serde_json::from_slice(&std::fs::read(path.with_file_name(first)).unwrap()).unwrap();
    let exported_first = settings["FileReferences"]["Motions"]["Idle"][0]["File"]
        .as_str()
        .unwrap();
    let after = json(&export, exported_first);
    let ids = |v: &Value| -> BTreeSet<String> {
        v["Curves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| format!("{}:{}", c["Target"].as_str().unwrap(), c["Id"].as_str().unwrap()))
            .collect()
    };
    assert_eq!(ids(&before), ids(&after));
}
