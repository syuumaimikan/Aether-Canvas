//! Live2D Cubism models inside a rig.
//!
//! An imported Live2D model keeps its own deformation data (the `.moc3`
//! arrays) and is evaluated exactly as Cubism Core would by
//! [`aether_live2d::Model`]; everything that works through parameters —
//! motions, expressions, physics, behaviours, drivers, face tracking, the
//! timeline — drives it like any other rig. Each [`CubismRig`] links the
//! model's parameters and part opacities to rig [`Parameter`]s and draws on
//! one layer (the model's textures live in that layer).
//!
//! [`Parameter`]: crate::param::Parameter

use crate::rig::ResolvedParams;
use aether_core::math::Vec2;
use aether_core::{LayerId, ParameterId};
use aether_live2d::moc3::{self, Moc};
use aether_live2d::Model;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A pose group (`pose3.json`): parts of which only one shows at a time.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PoseGroup {
    /// Part indices, each with the parts that follow its opacity.
    pub parts: Vec<PosePart>,
}

/// One member of a pose group.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PosePart {
    /// The part.
    pub part: usize,
    /// Parts whose opacity follows this one's.
    #[serde(default)]
    pub links: Vec<usize>,
}

/// A named hit area (`model3.json` `HitAreas`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HitArea {
    /// The drawable tested.
    pub drawable: String,
    /// The name scripts see.
    pub name: String,
}

/// A Live2D model driven by the rig.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CubismRig {
    /// The layer the model draws on (a Live2D model layer holding the
    /// textures).
    pub layer: LayerId,
    /// The model data.
    #[serde(with = "moc_base64")]
    pub moc: Arc<Moc>,
    /// The rig parameter driving each model parameter, index-aligned; an
    /// unlinked parameter stays at its default.
    pub parameters: Vec<Option<ParameterId>>,
    /// The rig parameter (0..1) driving each part's opacity, index-aligned;
    /// unlinked parts keep the opacity the model gives them.
    #[serde(default)]
    pub parts: Vec<Option<ParameterId>>,
    /// Canvas pixels to document pixels: `document = canvas × scale + offset`.
    pub scale: f32,
    /// See `scale`.
    pub offset: Vec2,
    /// Texture file names, in page order (for export).
    #[serde(default)]
    pub texture_files: Vec<String>,
    /// Pose groups (`pose3.json`).
    #[serde(default)]
    pub pose: Vec<PoseGroup>,
    /// Fade time for pose switches, seconds.
    #[serde(default = "default_fade")]
    pub pose_fade: f32,
    /// Hit areas.
    #[serde(default)]
    pub hit_areas: Vec<HitArea>,
    /// The model's physics (`physics3.json`), simulated as the Cubism
    /// Framework does while the rig plays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<serde_json::Value>,
    /// Parts of the original `model3.json` and companions that the editor
    /// does not interpret (layout, user data, display names), kept for
    /// export.
    #[serde(default)]
    pub extra: serde_json::Value,
}

fn default_fade() -> f32 {
    0.5
}

mod moc_base64 {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(moc: &Arc<Moc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&aether_live2d::base64::encode(&moc.write()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Moc>, D::Error> {
        let text = String::deserialize(d)?;
        let bytes = aether_live2d::base64::decode(&text)
            .ok_or_else(|| serde::de::Error::custom("the Live2D model data is not base64"))?;
        Moc::read(&bytes)
            .map(Arc::new)
            .map_err(|e| serde::de::Error::custom(format!("the Live2D model data is damaged: {e}")))
    }
}

/// A drawable at the current pose.
#[derive(Clone, Debug, PartialEq)]
pub struct CubismDrawable {
    /// Vertices in document pixels.
    pub positions: Vec<Vec2>,
    /// Final opacity.
    pub opacity: f32,
    /// Multiply tint.
    pub multiply: [f32; 3],
    /// Screen tint.
    pub screen: [f32; 3],
    /// Enabled and not fully transparent.
    pub visible: bool,
}

/// How a drawable blends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubismBlend {
    /// Source-over.
    Normal,
    /// Additive (Cubism's "add"; before 5.3 the only other two modes).
    Add,
    /// Multiplicative.
    Multiply,
    /// Cubism 5.3 blend modes, by their Cubism number (see
    /// [`CubismStatic::blend_code`]).
    Other(i32),
}

/// Per-drawable data that does not change with the pose.
#[derive(Clone, Debug, PartialEq)]
pub struct CubismStatic {
    /// Drawable id.
    pub id: String,
    /// Texture page.
    pub texture: usize,
    /// Texture coordinates, u right and v **down** (0..1 over the page).
    pub uvs: Vec<Vec2>,
    /// Triangles.
    pub triangles: Vec<[u32; 3]>,
    /// Drawables whose coverage clips this one.
    pub masks: Vec<usize>,
    /// Draw only outside the masks.
    pub invert_mask: bool,
    /// Both faces drawn (otherwise back faces are culled).
    pub double_sided: bool,
    /// Blend mode.
    pub blend: CubismBlend,
    /// Raw Cubism 5.3 blend code (colour type in the low byte), or 0.
    pub blend_code: i32,
}

/// A Live2D model at the current pose.
#[derive(Clone, Debug, PartialEq)]
pub struct CubismPose {
    /// The layer it draws on.
    pub layer: LayerId,
    /// Every drawable, index-aligned with the model.
    pub drawables: Vec<CubismDrawable>,
    /// Visible drawables, in the order they are drawn.
    pub order: Vec<usize>,
}

impl CubismPose {
    /// Bounding box of every visible drawable, in document pixels.
    pub fn bounds(&self) -> aether_core::math::Rect {
        let points: Vec<Vec2> = self
            .order
            .iter()
            .flat_map(|&i| self.drawables[i].positions.iter().copied())
            .collect();
        crate::mesh::bounds_of(&points)
    }
}

impl CubismRig {
    /// A model on `layer`, unlinked, at canvas scale.
    pub fn new(layer: LayerId, moc: Moc) -> Self {
        let n = moc.parameters.ids.len();
        let parts = moc.parts.ids.len();
        Self {
            layer,
            moc: Arc::new(moc),
            parameters: vec![None; n],
            parts: vec![None; parts],
            scale: 1.0,
            offset: Vec2::ZERO,
            texture_files: Vec::new(),
            pose: Vec::new(),
            pose_fade: default_fade(),
            hit_areas: Vec::new(),
            physics: None,
            extra: serde_json::Value::Null,
        }
    }

    /// Every rig parameter this model reads.
    pub fn uses(&self, id: ParameterId) -> bool {
        self.parameters.contains(&Some(id)) || self.parts.contains(&Some(id))
    }

    /// Forget a deleted parameter.
    pub fn forget_parameter(&mut self, id: ParameterId) {
        for p in self.parameters.iter_mut().chain(self.parts.iter_mut()) {
            if *p == Some(id) {
                *p = None;
            }
        }
    }

    /// Canvas pixels to document pixels.
    pub fn to_document(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(x * self.scale + self.offset.x, y * self.scale + self.offset.y)
    }

    /// Evaluate at resolved rig parameter values.
    pub fn evaluate(&self, params: &ResolvedParams) -> CubismPose {
        self.evaluate_with(|id| params.value(id))
    }

    /// Evaluate with part opacities from a running pose (see
    /// [`CubismRuntime`]) instead of the instant pose-group rule.
    pub fn evaluate_with_parts(&self, params: &ResolvedParams, parts: &[f32]) -> CubismPose {
        let mut model = self.model_at(|id| params.value(id));
        for (o, &v) in model.part_opacities.iter_mut().zip(parts) {
            *o = v;
        }
        model.update();
        self.pose_from(&model)
    }

    /// Evaluate at the model's own defaults (the rest pose).
    pub fn evaluate_rest(&self) -> CubismPose {
        self.evaluate_with(|_| None)
    }

    /// Evaluate, reading each linked parameter through `value`.
    pub fn evaluate_with(&self, value: impl Fn(ParameterId) -> Option<f32>) -> CubismPose {
        let mut model = self.model_at(value);
        model.update();
        self.pose_from(&model)
    }

    /// A model with parameters and (instant-rule) part opacities set, not
    /// yet updated.
    fn model_at(&self, value: impl Fn(ParameterId) -> Option<f32>) -> Model {
        let mut model = Model::new(self.moc.clone());
        for (i, link) in self.parameters.iter().enumerate() {
            if let Some(v) = link.and_then(&value) {
                model.parameter_values[i] = v;
            }
        }
        for (i, link) in self.parts.iter().enumerate() {
            if let Some(v) = link.and_then(&value) {
                model.part_opacities[i] = v;
            }
        }
        // Pose groups: the first member whose part parameter is on shows,
        // the rest hide; linked parts follow their member.
        for group in &self.pose {
            let on = |p: &PosePart| {
                self.parts
                    .get(p.part)
                    .copied()
                    .flatten()
                    .and_then(&value)
                    .is_some_and(|v| v > 0.001)
            };
            let visible = group.parts.iter().position(on).unwrap_or(0);
            for (k, member) in group.parts.iter().enumerate() {
                let opacity = if k == visible { 1.0 } else { 0.0 };
                for &part in std::iter::once(&member.part).chain(&member.links) {
                    if let Some(o) = model.part_opacities.get_mut(part) {
                        *o = opacity;
                    }
                }
            }
        }
        model
    }

    fn pose_from(&self, model: &Model) -> CubismPose {
        let c = self.moc.canvas;
        let drawables: Vec<CubismDrawable> = (0..model.drawable_count())
            .map(|i| {
                let positions = model
                    .drawable_positions(i)
                    .chunks_exact(2)
                    .map(|p| {
                        self.to_document(
                            c.origin_x + p[0] * c.pixels_per_unit,
                            c.origin_y - p[1] * c.pixels_per_unit,
                        )
                    })
                    .collect();
                let m = model.drawable_multiply(i);
                let s = model.drawable_screen(i);
                CubismDrawable {
                    positions,
                    opacity: model.drawable_opacity(i),
                    multiply: [m[0], m[1], m[2]],
                    screen: [s[0], s[1], s[2]],
                    visible: model.drawable_visible(i),
                }
            })
            .collect();
        let mut order: Vec<usize> = (0..drawables.len()).filter(|&i| drawables[i].visible).collect();
        order.sort_by_key(|&i| model.drawable_render_order(i));
        CubismPose {
            layer: self.layer,
            drawables,
            order,
        }
    }

    /// Static data of every drawable.
    pub fn statics(&self) -> Vec<CubismStatic> {
        let model = Model::new(self.moc.clone());
        let a = &self.moc.art_meshes;
        let y_reversed = self.moc.canvas.flags & moc3::CANVAS_Y_REVERSED != 0;
        (0..model.drawable_count())
            .map(|i| {
                // Core reports v up; flip back to image rows.
                let uvs = model
                    .drawable_uvs(i)
                    .chunks_exact(2)
                    .map(|uv| Vec2::new(uv[0], if y_reversed { uv[1] } else { 1.0 - uv[1] }))
                    .collect();
                let triangles = model
                    .drawable_indices(i)
                    .chunks_exact(3)
                    .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
                    .collect();
                let flags = a.flags[i];
                let blend_code = if self.moc.version >= moc3::VERSION_53 {
                    a.blend_mode[i]
                } else {
                    0
                };
                let blend = if self.moc.version >= moc3::VERSION_53 {
                    match blend_code & 0xff {
                        0 => CubismBlend::Normal,
                        1 | 3 => CubismBlend::Add,
                        2 | 6 => CubismBlend::Multiply,
                        other => CubismBlend::Other(other),
                    }
                } else if flags & moc3::FLAG_ADDITIVE != 0 {
                    CubismBlend::Add
                } else if flags & moc3::FLAG_MULTIPLICATIVE != 0 {
                    CubismBlend::Multiply
                } else {
                    CubismBlend::Normal
                };
                CubismStatic {
                    id: a.ids[i].clone(),
                    texture: a.texture[i].max(0) as usize,
                    uvs,
                    triangles,
                    masks: model
                        .drawable_masks(i)
                        .iter()
                        .filter(|&&m| m >= 0)
                        .map(|&m| m as usize)
                        .collect(),
                    invert_mask: flags & moc3::FLAG_INVERTED_MASK != 0,
                    double_sided: flags & moc3::FLAG_DOUBLE_SIDED != 0,
                    blend,
                    blend_code,
                }
            })
            .collect()
    }

    /// The document-space size of the model's canvas.
    pub fn canvas_size(&self) -> Vec2 {
        Vec2::new(
            self.moc.canvas.width * self.scale,
            self.moc.canvas.height * self.scale,
        )
    }
}

/// Time-dependent state of one Live2D model while the rig plays: its
/// physics and its pose-group fades.
#[derive(Clone, Debug)]
pub struct CubismRuntime {
    /// The model layer.
    pub layer: LayerId,
    moc: Arc<Moc>,
    physics: Option<aether_live2d::Physics>,
    settled: bool,
    parts: Vec<f32>,
}

impl CubismRuntime {
    /// State for `model`, at rest.
    pub fn new(model: &CubismRig) -> Self {
        let physics = model
            .physics
            .as_ref()
            .and_then(aether_live2d::Physics::from_json)
            .map(|mut p| {
                p.bind(&model.moc.parameters.ids);
                p
            });
        let mut parts: Vec<f32> = model
            .moc
            .parts
            .visible
            .iter()
            .map(|&v| if v != 0 { 1.0 } else { 0.0 })
            .collect();
        // Pose groups start on their first member.
        for group in &model.pose {
            for (k, member) in group.parts.iter().enumerate() {
                let o = if k == 0 { 1.0 } else { 0.0 };
                for &part in std::iter::once(&member.part).chain(&member.links) {
                    if let Some(p) = parts.get_mut(part) {
                        *p = o;
                    }
                }
            }
        }
        Self {
            layer: model.layer,
            moc: model.moc.clone(),
            physics,
            settled: false,
            parts,
        }
    }

    /// True while this state still belongs to `model`.
    pub fn matches(&self, model: &CubismRig) -> bool {
        self.layer == model.layer && Arc::ptr_eq(&self.moc, &model.moc)
    }

    /// Part opacities after pose fades.
    pub fn part_opacities(&self) -> &[f32] {
        &self.parts
    }

    /// Advance `dt` seconds: run physics on `values` (writing its outputs)
    /// when `simulate` is set, and fade pose groups toward the parts their parameters select.
    pub fn step(
        &mut self,
        model: &CubismRig,
        parameters: &[crate::param::Parameter],
        values: &mut crate::param::ParamValues,
        dt: f32,
        simulate: bool,
    ) {
        let m = &*model.moc;
        let n = m.parameters.ids.len();
        let lookup = |i: usize| -> Option<ParameterId> { model.parameters.get(i).copied().flatten() };
        let mut current: Vec<f32> = (0..n)
            .map(|i| {
                lookup(i)
                    .and_then(|id| {
                        values
                            .get(&id)
                            .copied()
                            .or_else(|| parameters.iter().find(|p| p.id == id).map(|p| p.default))
                    })
                    .unwrap_or(m.parameters.default[i])
            })
            .collect();
        if let Some(physics) = self.physics.as_mut().filter(|_| simulate) {
            let before = current.clone();
            let mut p = aether_live2d::physics::Parameters {
                values: &mut current,
                min: &m.parameters.min,
                max: &m.parameters.max,
                default: &m.parameters.default,
            };
            if self.settled {
                physics.evaluate(&mut p, dt as f64);
            } else {
                physics.stabilize(&mut p);
                self.settled = true;
            }
            for i in 0..n {
                if current[i] != before[i] {
                    if let Some(id) = lookup(i) {
                        values.insert(id, current[i]);
                    }
                }
            }
        }

        // Pose: Cubism's fade (a member whose parameter is on fades in over
        // `pose_fade`; the others fade out no slower than it, keeping the
        // sum near opaque), then links copy their member.
        let value = |part: usize| -> f32 {
            model
                .parts
                .get(part)
                .copied()
                .flatten()
                .and_then(|id| {
                    values
                        .get(&id)
                        .copied()
                        .or_else(|| parameters.iter().find(|p| p.id == id).map(|p| p.default))
                })
                .unwrap_or(0.0)
        };
        let dt = dt.max(0.0);
        for group in &model.pose {
            let mut visible = None;
            let mut new_opacity = 1.0f32;
            for (k, member) in group.parts.iter().enumerate() {
                if value(member.part) > 0.001 {
                    if visible.is_some() {
                        break;
                    }
                    visible = Some(k);
                    new_opacity = if model.pose_fade == 0.0 {
                        1.0
                    } else {
                        (self.parts.get(member.part).copied().unwrap_or(0.0) + dt / model.pose_fade).min(1.0)
                    };
                }
            }
            let visible = match visible {
                Some(v) => v,
                None => {
                    new_opacity = 1.0;
                    0
                }
            };
            const PHI: f32 = 0.5;
            const BACK_THRESHOLD: f32 = 0.15;
            for (k, member) in group.parts.iter().enumerate() {
                let Some(o) = self.parts.get_mut(member.part) else {
                    continue;
                };
                if k == visible {
                    *o = new_opacity;
                } else {
                    let mut a1 = if new_opacity < PHI {
                        new_opacity * (PHI - 1.0) / PHI + 1.0
                    } else {
                        (1.0 - new_opacity) * PHI / (1.0 - PHI)
                    };
                    let back = (1.0 - a1) * (1.0 - new_opacity);
                    if back > BACK_THRESHOLD {
                        a1 = 1.0 - BACK_THRESHOLD / (1.0 - new_opacity);
                    }
                    if *o > a1 {
                        *o = a1;
                    }
                }
            }
            for member in &group.parts {
                let o = self.parts.get(member.part).copied().unwrap_or(0.0);
                for &link in &member.links {
                    if let Some(p) = self.parts.get_mut(link) {
                        *p = o;
                    }
                }
            }
        }
        // Parts outside pose groups follow their parameter directly.
        let in_pose: Vec<usize> = model
            .pose
            .iter()
            .flat_map(|g| {
                g.parts
                    .iter()
                    .flat_map(|m| std::iter::once(m.part).chain(m.links.iter().copied()))
            })
            .collect();
        for (part, link) in model.parts.iter().enumerate() {
            if link.is_some() && !in_pose.contains(&part) {
                if let Some(o) = self.parts.get_mut(part) {
                    *o = value(part);
                }
            }
        }
    }
}
