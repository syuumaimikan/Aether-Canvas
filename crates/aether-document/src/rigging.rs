//! Document-level rigging: meshing painted layers and rigging a whole
//! document from its layer names.
//!
//! These are the building blocks the editor wraps in undoable commands, and
//! what headless pipelines (`aether-canvas --auto-rig`) call directly.

use crate::layer::LayerContent;
use crate::Document;
use aether_core::{LayerId, Result};
use aether_rig::automesh::{self, AutoMeshOptions};
use aether_rig::autorig::{self, AutoRigReport, PartInfo};
use aether_rig::ArtMesh;

/// Generated meshes for the painted layers under `root` (the whole document
/// for `None`) that have no mesh yet. `density` scales vertex spacing (1 is
/// the editor's default).
pub fn missing_meshes(doc: &Document, root: Option<LayerId>, density: f32) -> Vec<ArtMesh> {
    let ids: Vec<LayerId> = match root {
        Some(r) => doc.layers.subtree_ids(r),
        None => doc.layers.iter().map(|l| l.id).collect(),
    };
    let mut meshes = Vec::new();
    for id in ids {
        if doc.rig.mesh(id).is_some() {
            continue;
        }
        let Some(layer) = doc.layers.get(id) else {
            continue;
        };
        let LayerContent::Raster(raster) = &layer.content else {
            continue;
        };
        let Some(bounds) = automesh::opaque_bounds(&raster.pixmap, 8) else {
            continue;
        };
        let options = AutoMeshOptions::for_bounds(bounds, density);
        if let Some(generated) = automesh::auto_mesh(&raster.pixmap, &options) {
            let mut mesh = ArtMesh::new(id, generated.vertices, generated.triangles);
            mesh.name = layer.name.clone();
            meshes.push(mesh);
        }
    }
    meshes
}

/// What the auto-rigger needs to know about each meshed raster layer: its
/// name, the names of the groups around it, and its bounds. `pending` are
/// meshes about to be added, counted as if they were already in the rig.
pub fn rig_parts(doc: &Document, pending: &[ArtMesh]) -> Vec<PartInfo> {
    let mut parts = Vec::new();
    for layer in doc.layers.iter() {
        if !matches!(layer.content, LayerContent::Raster(_)) {
            continue;
        }
        let bounds = match (
            doc.rig.mesh(layer.id),
            pending.iter().find(|m| m.layer == layer.id),
        ) {
            (Some(m), _) | (None, Some(m)) => m.bounds(),
            (None, None) => continue,
        };
        let mut groups = Vec::new();
        let mut current = doc.layers.parent_of(layer.id);
        while let Some(id) = current {
            if let Some(g) = doc.layers.get(id) {
                groups.insert(0, g.name.clone());
            }
            current = doc.layers.parent_of(id);
        }
        parts.push(PartInfo {
            layer: layer.id,
            name: layer.name.clone(),
            groups,
            bounds,
        });
    }
    parts
}

/// Mesh every painted layer and rig the document from its layer names, in
/// place and without undo (see [`aether_rig::autorig`]).
pub fn auto_rig_document(doc: &mut Document, density: f32) -> Result<AutoRigReport> {
    let meshes = missing_meshes(doc, None, density);
    let parts = rig_parts(doc, &meshes);
    for mesh in meshes {
        doc.rig.set_mesh(mesh);
    }
    autorig::auto_rig(&mut doc.rig, &doc.ids, &parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;
    use aether_core::math::IRect;

    fn paint(doc: &mut Document, name: &str, rect: IRect) -> LayerId {
        let id = doc.add_raster_layer(name);
        if let Some(pixmap) = doc.layers.get_mut(id).and_then(|l| l.pixmap_mut()) {
            pixmap.fill_rect(rect, Rgba8::new(200, 150, 120, 255));
        }
        id
    }

    #[test]
    fn a_named_document_rigs_itself() {
        let mut doc = Document::empty(200, 240, "face");
        let face = paint(&mut doc, "Face", IRect::new(50, 40, 100, 120));
        paint(&mut doc, "Eye L", IRect::new(70, 80, 20, 12));
        paint(&mut doc, "Eye R", IRect::new(110, 80, 20, 12));
        paint(&mut doc, "Mouth", IRect::new(85, 130, 30, 8));
        doc.add_raster_layer("Empty");

        let report = auto_rig_document(&mut doc, 1.0).expect("auto rig");
        assert_eq!(
            doc.rig.meshes.len(),
            4,
            "every painted layer is meshed; the empty one is not"
        );
        assert!(doc.rig.mesh(face).is_some());
        assert!(doc.rig.parameter_named("AngleX").is_some());
        assert!(doc.rig.parameter_named("EyeLOpen").is_some());
        assert!(report.deformers > 0);
        // Nothing left to mesh.
        assert!(missing_meshes(&doc, None, 1.0).is_empty());
    }
}
