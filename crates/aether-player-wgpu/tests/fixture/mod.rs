//! A small scene that exercises every drawing path, shared by the GPU
//! parity test and the `draw_fixture` example (which exports it for the
//! other runtimes' tests).

use aether_core::blend::BlendMode;
use aether_core::color::Rgba8;
use aether_core::math::{IRect, Rect, Vec2};
use aether_core::LayerId;
use aether_document::layer::Layer;
use aether_document::rig::{ArtMesh, Parameter, RigNode};
use aether_document::Document;

fn add_layer(doc: &mut Document, name: &str, rect: IRect, color: Rgba8, blend: BlendMode) -> LayerId {
    let id = doc.ids.layer();
    let mut layer = Layer::raster(id, name, doc.width, doc.height);
    layer.blend_mode = blend;
    if let Some(pixels) = layer.pixmap_mut() {
        pixels.fill_rect(rect, color);
        // A soft edge, so filtering and alpha are exercised too.
        for x in rect.x..rect.right() {
            pixels.set(x, rect.y, Rgba8::new(color.r, color.g, color.b, 90));
        }
    }
    doc.layers.insert(layer, None, usize::MAX).expect("insert");
    id
}

/// Every drawing path: a rigged mesh keyed for movement, opacity, tint and
/// draw order; a layer clipped to it; multiply, screen and add layers; and
/// translucency.
pub fn scene() -> Document {
    let mut doc = Document::empty(160, 120, "gpu");
    add_layer(
        &mut doc,
        "Back",
        IRect::new(0, 0, 160, 120),
        Rgba8::new(235, 225, 205, 255),
        BlendMode::Normal,
    );
    let arm = add_layer(
        &mut doc,
        "Arm",
        IRect::new(20, 30, 60, 26),
        Rgba8::new(210, 70, 70, 255),
        BlendMode::Normal,
    );
    let stripe = add_layer(
        &mut doc,
        "Stripe",
        IRect::new(0, 36, 160, 8),
        Rgba8::new(40, 40, 210, 230),
        BlendMode::Normal,
    );
    doc.layers.get_mut(stripe).unwrap().clipping = true;
    add_layer(
        &mut doc,
        "Shade",
        IRect::new(90, 20, 50, 70),
        Rgba8::new(120, 140, 255, 255),
        BlendMode::Multiply,
    );
    add_layer(
        &mut doc,
        "Glow",
        IRect::new(10, 70, 60, 40),
        Rgba8::new(200, 90, 20, 200),
        BlendMode::Screen,
    );
    add_layer(
        &mut doc,
        "Spark",
        IRect::new(100, 80, 40, 30),
        Rgba8::new(60, 60, 30, 255),
        BlendMode::Add,
    );
    let ghost = add_layer(
        &mut doc,
        "Ghost",
        IRect::new(60, 10, 40, 40),
        Rgba8::new(20, 160, 90, 255),
        BlendMode::Normal,
    );
    doc.layers.get_mut(ghost).unwrap().opacity = 0.45;

    let swing = doc
        .rig
        .add_parameter(Parameter::new(doc.ids.parameter(), "Swing", 0.0, 1.0, 0.0))
        .unwrap();
    let rect = Rect::from_min_size(Vec2::new(16.0, 26.0), Vec2::new(68.0, 34.0));
    let mut vertices = Vec::new();
    for j in 0..=3 {
        for i in 0..=6 {
            vertices.push(Vec2::new(
                rect.min.x + rect.width() * i as f32 / 6.0,
                rect.min.y + rect.height() * j as f32 / 3.0,
            ));
        }
    }
    let mut triangles = Vec::new();
    for j in 0..3u32 {
        for i in 0..6u32 {
            let a = j * 7 + i;
            triangles.push([a, a + 1, a + 8]);
            triangles.push([a, a + 8, a + 7]);
        }
    }
    doc.rig.set_mesh(ArtMesh::new(arm, vertices, triangles));
    doc.rig
        .bind_parameter(RigNode::Mesh(arm), swing, &[0.0, 1.0])
        .unwrap();
    let form = &mut doc.rig.mesh_mut(arm).unwrap().keyforms.forms[1];
    for (k, o) in form.offsets.iter_mut().enumerate() {
        // Bend: the far end rises.
        let column = (k % 7) as f32 / 6.0;
        *o = Vec2::new(18.0 * column, -24.0 * column * column);
    }
    form.opacity = 0.8;
    form.multiply = [1.0, 0.75, 0.6];
    form.screen = [0.0, 0.1, 0.2];
    form.draw_order = 6.0;
    doc
}
