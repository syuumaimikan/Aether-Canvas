//! Posed geometry: what the renderer draws.
//!
//! A [`RigPose`] is the output of evaluating a rig — final vertex positions,
//! opacity, tint and draw order for every mesh. It is deliberately dumb data
//! so the compositor, the canvas overlay and exporters can all consume it
//! without knowing how it was produced.

use aether_core::math::{IRect, Rect, Vec2};
use aether_core::LayerId;
use std::collections::BTreeMap;

/// One mesh at the current pose.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPose {
    /// The layer the mesh draws.
    pub layer: LayerId,
    /// Final vertex positions, document space.
    pub positions: Vec<Vec2>,
    /// Opacity multiplier.
    pub opacity: f32,
    /// Multiply tint.
    pub multiply: [f32; 3],
    /// Screen tint.
    pub screen: [f32; 3],
    /// Draw-order offset among siblings.
    pub draw_order: f32,
    /// True when the mesh leaves its layer exactly as painted, so the layer
    /// can be drawn directly instead of through the mesh.
    pub rest: bool,
}

impl MeshPose {
    /// Bounding box of the posed vertices.
    pub fn bounds(&self) -> Rect {
        crate::mesh::bounds_of(&self.positions)
    }

    /// True when drawing this pose could produce different pixels from
    /// `other`.
    pub fn differs_from(&self, other: &MeshPose) -> bool {
        self.rest != other.rest
            || (self.opacity - other.opacity).abs() > 1e-5
            || self.multiply != other.multiply
            || self.screen != other.screen
            || (self.draw_order - other.draw_order).abs() > 1e-5
            || self.positions.len() != other.positions.len()
            || self
                .positions
                .iter()
                .zip(&other.positions)
                .any(|(a, b)| a.distance(*b) > 1e-3)
    }
}

/// Every mesh at the current pose.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RigPose {
    /// Mesh poses by layer.
    pub meshes: BTreeMap<LayerId, MeshPose>,
    /// Live2D models, one per model layer.
    pub cubism: Vec<crate::cubism::CubismPose>,
}

/// What changed between two poses, for incremental re-rendering.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PoseChange {
    /// Document area covered by moved meshes, before and after.
    pub area: IRect,
    /// Layers whose mesh switched between drawn-as-painted and deformed (the
    /// caller must also redraw everything the layer's own pixels cover).
    pub toggled: Vec<LayerId>,
    /// Layers whose draw order changed (their siblings must be redrawn).
    pub reordered: Vec<LayerId>,
}

impl PoseChange {
    /// True when nothing needs redrawing.
    pub fn is_empty(&self) -> bool {
        self.area.is_empty() && self.toggled.is_empty() && self.reordered.is_empty()
    }
}

impl RigPose {
    /// The pose of one layer's mesh.
    pub fn mesh(&self, layer: LayerId) -> Option<&MeshPose> {
        self.meshes.get(&layer)
    }

    /// The pose of the Live2D model drawn on `layer`.
    pub fn cubism(&self, layer: LayerId) -> Option<&crate::cubism::CubismPose> {
        self.cubism.iter().find(|c| c.layer == layer)
    }

    /// What needs redrawing to go from `self` to `next`.
    pub fn change_to(&self, next: &RigPose) -> PoseChange {
        let mut change = PoseChange::default();
        let add = |pose: &MeshPose, change: &mut PoseChange| {
            change.area = change.area.union(&pose.bounds().to_irect_outer().expanded(2));
        };
        for (layer, after) in &next.meshes {
            match self.meshes.get(layer) {
                Some(before) if !before.differs_from(after) => {}
                Some(before) => {
                    add(before, &mut change);
                    add(after, &mut change);
                    if before.rest != after.rest {
                        change.toggled.push(*layer);
                    }
                    if (before.draw_order - after.draw_order).abs() > 1e-5 {
                        change.reordered.push(*layer);
                    }
                }
                None => {
                    add(after, &mut change);
                    change.toggled.push(*layer);
                }
            }
        }
        for (layer, before) in &self.meshes {
            if !next.meshes.contains_key(layer) {
                add(before, &mut change);
                change.toggled.push(*layer);
            }
        }
        // A Live2D model redraws as a whole wherever it was or now is.
        for after in &next.cubism {
            match self.cubism(after.layer) {
                Some(before) if before == after => {}
                Some(before) => {
                    change.area = change.area.union(&before.bounds().to_irect_outer().expanded(2));
                    change.area = change.area.union(&after.bounds().to_irect_outer().expanded(2));
                }
                None => {
                    change.area = change.area.union(&after.bounds().to_irect_outer().expanded(2));
                    change.toggled.push(after.layer);
                }
            }
        }
        for before in &self.cubism {
            if next.cubism(before.layer).is_none() {
                change.area = change.area.union(&before.bounds().to_irect_outer().expanded(2));
                change.toggled.push(before.layer);
            }
        }
        change
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::math::vec2;

    fn pose(x: f32, rest: bool) -> MeshPose {
        MeshPose {
            layer: LayerId(1),
            positions: vec![vec2(x, 0.0), vec2(x + 10.0, 10.0)],
            opacity: 1.0,
            multiply: [1.0; 3],
            screen: [0.0; 3],
            draw_order: 0.0,
            rest,
        }
    }

    #[test]
    fn identical_poses_need_no_redraw() {
        let mut a = RigPose::default();
        a.meshes.insert(LayerId(1), pose(0.0, true));
        assert!(a.change_to(&a.clone()).is_empty());
    }

    #[test]
    fn moved_meshes_dirty_both_old_and_new_bounds() {
        let mut a = RigPose::default();
        a.meshes.insert(LayerId(1), pose(0.0, true));
        let mut b = RigPose::default();
        b.meshes.insert(LayerId(1), pose(50.0, false));
        let change = a.change_to(&b);
        assert!(change.area.contains(1, 1));
        assert!(change.area.contains(55, 5));
        assert_eq!(change.toggled, vec![LayerId(1)]);
    }
}
