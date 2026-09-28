//! The runtime model format.
//!
//! A runtime model is a `model.json` file plus one or more PNG texture pages.
//! The JSON holds everything needed to animate and draw the character:
//!
//! * `parts` — one textured triangle mesh per drawn layer, with rest
//!   positions in document pixels and texture coordinates in `0..=1`;
//! * `tree` — the draw order, mirroring the layer tree (groups, and clipping
//!   layers riding on their base) so keyed draw-order offsets sort siblings
//!   exactly as they do in the editor;
//! * `rig` — the unmodified rig: parameters, deformers, bones, meshes,
//!   physics, drivers, motions, expressions and behaviours.
//!
//! Everything is plain data. See `docs/RUNTIME.md` for the full description.

use aether_core::math::{Transform2D, Vec2};
use aether_core::LayerId;
use aether_rig::{flat, Rig};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Value of [`Model::format`].
pub const FORMAT: &str = "aether-model";

/// Current value of [`Model::version`]. Readers accept this version and
/// older ones.
pub const VERSION: u32 = 1;

/// A model that cannot be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelError(pub String);

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ModelError {}

fn error(message: impl Into<String>) -> ModelError {
    ModelError(message.into())
}

/// A texture page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Texture {
    /// File name, relative to `model.json`.
    pub file: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// How a part combines with what is drawn below it.
///
/// These are the modes every GPU can do with fixed-function blending on
/// premultiplied colour. The exporter maps the editor's other modes onto the
/// nearest of these and reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum BlendKind {
    /// Source-over.
    #[default]
    Normal = 0,
    /// Multiply.
    Multiply = 1,
    /// Screen.
    Screen = 2,
    /// Additive.
    Add = 3,
}

impl BlendKind {
    /// The mode for a numeric code, as used by the C API.
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Normal),
            1 => Some(Self::Multiply),
            2 => Some(Self::Screen),
            3 => Some(Self::Add),
            _ => None,
        }
    }
}

/// One drawn layer: a textured triangle mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// The layer this part draws; rig meshes are keyed by it.
    pub layer: LayerId,
    /// Layer name, for hit testing and debugging.
    #[serde(default)]
    pub name: String,
    /// Texture page.
    pub texture: u32,
    /// Rest positions, document pixels, as a flat `[x0, y0, x1, y1, ...]`
    /// array. When the rig has a mesh for [`Part::layer`], these equal its
    /// rest vertices.
    #[serde(with = "flat")]
    pub vertices: Vec<Vec2>,
    /// Texture coordinates in `0..=1`, one per vertex, flat.
    #[serde(with = "flat")]
    pub uvs: Vec<Vec2>,
    /// Triangles as vertex indices, flat.
    #[serde(with = "flat_triangles")]
    pub triangles: Vec<[u32; 3]>,
    /// Opacity, including every enclosing group's.
    pub opacity: f32,
    /// Blend mode.
    #[serde(default)]
    pub blend: BlendKind,
    /// Affine transform applied after deformation (the layer's and its
    /// groups' transforms, composed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform2D>,
}

/// One entry of the draw tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    /// A group; its children sort among themselves.
    Group {
        /// Position among the siblings in the layer tree (sort key before
        /// draw-order offsets).
        index: u32,
        /// Group name.
        #[serde(default)]
        name: String,
        /// Children, bottom first.
        children: Vec<Node>,
    },
    /// A part, plus the parts clipped to it.
    Part {
        /// Position among the siblings in the layer tree.
        index: u32,
        /// The base part.
        part: u32,
        /// Parts drawn only where the base is, bottom first.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        clipped: Vec<u32>,
    },
}

/// A complete runtime model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// Always [`FORMAT`].
    pub format: String,
    /// Format version.
    pub version: u32,
    /// Model name.
    #[serde(default)]
    pub name: String,
    /// Canvas width, pixels.
    pub width: u32,
    /// Canvas height, pixels.
    pub height: u32,
    /// Texture pages.
    pub textures: Vec<Texture>,
    /// Drawn parts.
    pub parts: Vec<Part>,
    /// Draw order, bottom first.
    pub tree: Vec<Node>,
    /// The rig.
    pub rig: Rig,
}

impl Model {
    /// An empty model of the given canvas size.
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            format: FORMAT.to_string(),
            version: VERSION,
            name: name.into(),
            width,
            height,
            textures: Vec::new(),
            parts: Vec::new(),
            tree: Vec::new(),
            rig: Rig::default(),
        }
    }

    /// Parse and validate `model.json`.
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        let mut model: Model =
            serde_json::from_str(text).map_err(|e| error(format!("not a valid model: {e}")))?;
        model.rig.repair();
        model.validate()?;
        Ok(model)
    }

    /// Serialise as `model.json`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("models serialise")
    }

    /// Check every cross-reference, so a player never indexes out of range.
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.format != FORMAT {
            return Err(error(format!("unknown format {:?}", self.format)));
        }
        if self.version == 0 || self.version > VERSION {
            return Err(error(format!(
                "model version {} is newer than this runtime (supports {VERSION})",
                self.version
            )));
        }
        if self.width == 0 || self.height == 0 {
            return Err(error("the canvas is empty"));
        }
        for (i, part) in self.parts.iter().enumerate() {
            let name = if part.name.is_empty() {
                format!("#{i}")
            } else {
                part.name.clone()
            };
            if part.texture as usize >= self.textures.len() {
                return Err(error(format!("part {name} uses a missing texture")));
            }
            if part.uvs.len() != part.vertices.len() {
                return Err(error(format!(
                    "part {name} has {} uvs for {} vertices",
                    part.uvs.len(),
                    part.vertices.len()
                )));
            }
            let count = part.vertices.len() as u32;
            if part.triangles.iter().flatten().any(|&v| v >= count) {
                return Err(error(format!("part {name} has a triangle past its vertices")));
            }
            if !part.opacity.is_finite() {
                return Err(error(format!("part {name} has an invalid opacity")));
            }
            if part.vertices.iter().chain(&part.uvs).any(|p| !p.is_finite()) {
                return Err(error(format!("part {name} has a non-finite coordinate")));
            }
            if let Some(mesh) = self.rig.mesh(part.layer) {
                if mesh.vertices.len() != part.vertices.len() {
                    return Err(error(format!(
                        "part {name} has {} vertices but its rig mesh has {}",
                        part.vertices.len(),
                        mesh.vertices.len()
                    )));
                }
            }
        }
        let mut seen = BTreeSet::new();
        let mut check = |part: u32| -> Result<(), ModelError> {
            if part as usize >= self.parts.len() {
                return Err(error(format!("the draw tree names missing part {part}")));
            }
            if !seen.insert(part) {
                return Err(error(format!("the draw tree draws part {part} twice")));
            }
            Ok(())
        };
        let mut stack: Vec<&Node> = self.tree.iter().collect();
        while let Some(node) = stack.pop() {
            match node {
                Node::Group { children, .. } => stack.extend(children),
                Node::Part { part, clipped, .. } => {
                    check(*part)?;
                    for c in clipped {
                        check(*c)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Flat serialisation for triangle lists.
mod flat_triangles {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(triangles: &[[u32; 3]], serializer: S) -> Result<S::Ok, S::Error> {
        let flat: Vec<u32> = triangles.iter().flatten().copied().collect();
        flat.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<[u32; 3]>, D::Error> {
        let flat = Vec::<u32>::deserialize(deserializer)?;
        if flat.len() % 3 != 0 {
            return Err(D::Error::custom(
                "a triangle list needs a multiple of three indices",
            ));
        }
        Ok(flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_part(layer: u64) -> Part {
        Part {
            layer: LayerId(layer),
            name: format!("part {layer}"),
            texture: 0,
            vertices: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(4.0, 0.0),
                Vec2::new(4.0, 4.0),
                Vec2::new(0.0, 4.0),
            ],
            uvs: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            opacity: 1.0,
            blend: BlendKind::Normal,
            transform: None,
        }
    }

    fn model() -> Model {
        let mut model = Model::new("test", 8, 8);
        model.textures.push(Texture {
            file: "texture_0.png".into(),
            width: 4,
            height: 4,
        });
        model.parts.push(quad_part(1));
        model.parts.push(quad_part(2));
        model.tree.push(Node::Part {
            index: 0,
            part: 0,
            clipped: vec![1],
        });
        model
    }

    #[test]
    fn models_round_trip_through_json() {
        let model = model();
        let text = model.to_json();
        assert!(text.contains(r#""triangles":[0,1,2,0,2,3]"#), "{text}");
        let back = Model::from_json(&text).expect("load");
        assert_eq!(back, model);
    }

    #[test]
    fn broken_references_are_rejected() {
        let mut bad = model();
        bad.parts[0].triangles.push([0, 1, 9]);
        assert!(bad.validate().is_err());

        let mut bad = model();
        bad.parts[1].texture = 3;
        assert!(bad.validate().is_err());

        let mut bad = model();
        bad.tree.push(Node::Part {
            index: 1,
            part: 1,
            clipped: vec![],
        });
        assert!(bad.validate().unwrap_err().0.contains("twice"));

        let mut bad = model();
        bad.version = VERSION + 1;
        assert!(bad.validate().unwrap_err().0.contains("newer"));

        assert!(Model::from_json("{").is_err());
        assert!(Model::from_json(r#"{"format":"x"}"#).is_err());
    }
}
