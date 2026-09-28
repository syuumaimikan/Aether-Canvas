//! Layers.
//!
//! Every node in the layer tree is a [`Layer`]: a shared set of properties plus
//! a [`LayerContent`] payload that says what kind of thing it is.
//!
//! Splitting "common properties" from "content" matters. Opacity, blend mode,
//! masking, clipping and locking behave identically for a painted layer, a
//! group and a plugin-defined layer, so the compositor implements them once.
//! Adding a new kind of layer means adding a payload, not touching the
//! compositor's per-layer bookkeeping.
//!
//! Plugin extensibility is handled by [`LayerContent::Custom`], which carries a
//! type tag and an opaque JSON payload. A project that uses a plugin the
//! current build does not have still round-trips through save/load with its
//! data intact instead of being silently dropped.

use aether_core::blend::BlendMode;
use aether_core::color::Rgba8;
use aether_core::math::{IRect, Transform2D};
use aether_core::LayerId;
use aether_raster::adjust::Adjustment;
use aether_raster::{LayerEffect, Mask, Pixmap};
use serde::{Deserialize, Serialize};

/// A colour tag shown in the layer panel, for organising a busy stack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorLabel {
    /// No label.
    #[default]
    None,
    /// Red label.
    Red,
    /// Orange label.
    Orange,
    /// Yellow label.
    Yellow,
    /// Green label.
    Green,
    /// Blue label.
    Blue,
    /// Purple label.
    Purple,
    /// Grey label.
    Gray,
}

impl ColorLabel {
    /// All labels in picker order.
    pub const ALL: [ColorLabel; 8] = [
        ColorLabel::None,
        ColorLabel::Red,
        ColorLabel::Orange,
        ColorLabel::Yellow,
        ColorLabel::Green,
        ColorLabel::Blue,
        ColorLabel::Purple,
        ColorLabel::Gray,
    ];

    /// The swatch colour, or `None` for [`ColorLabel::None`].
    pub fn swatch(self) -> Option<Rgba8> {
        match self {
            ColorLabel::None => None,
            ColorLabel::Red => Some(Rgba8::rgb(224, 82, 82)),
            ColorLabel::Orange => Some(Rgba8::rgb(226, 145, 62)),
            ColorLabel::Yellow => Some(Rgba8::rgb(219, 199, 76)),
            ColorLabel::Green => Some(Rgba8::rgb(106, 187, 106)),
            ColorLabel::Blue => Some(Rgba8::rgb(83, 141, 219)),
            ColorLabel::Purple => Some(Rgba8::rgb(153, 108, 209)),
            ColorLabel::Gray => Some(Rgba8::rgb(140, 140, 145)),
        }
    }
}

/// Painted pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RasterContent {
    /// The pixel buffer, in document coordinates.
    pub pixmap: Pixmap,
}

impl RasterContent {
    /// An empty transparent buffer of the given size.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            pixmap: Pixmap::new(width, height),
        }
    }
}

/// A folder of layers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GroupContent {
    /// Children in render order: index `0` is the **bottom** of the stack.
    pub children: Vec<LayerId>,
    /// When true the group composites its children into an isolated buffer
    /// first, so the group's own blend mode applies to the result as a whole.
    ///
    /// When false the group is "pass through": children blend against
    /// everything below the group, exactly as if the group were not there.
    pub isolate: bool,
    /// Collapsed state in the layer panel; purely presentational.
    pub collapsed: bool,
}

/// A non-destructive colour operation applied to everything below.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdjustmentContent {
    /// The operation and its parameters.
    pub adjustment: Adjustment,
}

/// A flat colour, usually paired with a mask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FillContent {
    /// The colour to fill with.
    pub color: Rgba8,
}

/// A Live2D model: its texture pages. The model's deformation data and
/// parameter links are in the rig (`Rig::cubism`), which draws it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Live2DContent {
    /// Texture atlas pages, straight alpha, as in the model's PNG files.
    pub textures: Vec<Pixmap>,
}

/// What a layer actually contains.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayerContent {
    /// Painted pixels.
    Raster(RasterContent),
    /// A folder of other layers.
    Group(GroupContent),
    /// A colour adjustment applied to the layers below.
    Adjustment(AdjustmentContent),
    /// A flat colour fill.
    Fill(FillContent),
    /// A Live2D model.
    Live2D(Live2DContent),
    /// Content owned by a plugin.
    ///
    /// The core application preserves `kind` and `payload` verbatim across
    /// save/load and undo, and skips the layer when compositing if no plugin
    /// claims `kind`.
    Custom {
        /// Reverse-DNS style type tag, e.g. `"studio.example.halftone"`.
        kind: String,
        /// Plugin-owned data.
        payload: serde_json::Value,
    },
}

/// A lightweight discriminant for UI and filtering, without the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerKind {
    /// [`LayerContent::Raster`].
    Raster,
    /// [`LayerContent::Group`].
    Group,
    /// [`LayerContent::Adjustment`].
    Adjustment,
    /// [`LayerContent::Fill`].
    Fill,
    /// [`LayerContent::Live2D`].
    Live2D,
    /// [`LayerContent::Custom`].
    Custom,
}

impl LayerKind {
    /// Short label for the layer panel.
    pub fn label(self) -> &'static str {
        match self {
            LayerKind::Raster => "Raster",
            LayerKind::Group => "Group",
            LayerKind::Adjustment => "Adjustment",
            LayerKind::Fill => "Fill",
            LayerKind::Live2D => "Live2D",
            LayerKind::Custom => "Custom",
        }
    }
}

/// One node of the layer tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Stable identity within the project.
    pub id: LayerId,
    /// Display name.
    pub name: String,
    /// Whether the layer contributes to the composite.
    pub visible: bool,
    /// Whether editing tools may modify the layer.
    pub locked: bool,
    /// Whether painting is restricted to already-opaque pixels.
    pub alpha_lock: bool,
    /// Opacity in `0..=1`.
    pub opacity: f32,
    /// How the layer combines with the layers below.
    pub blend_mode: BlendMode,
    /// Layer-local transform, applied when compositing.
    pub transform: Transform2D,
    /// When true the layer is clipped to the alpha of the layer below it.
    pub clipping: bool,
    /// Optional coverage mask in document coordinates.
    pub mask: Option<Mask>,
    /// Whether the mask is currently applied.
    pub mask_enabled: bool,
    /// Organisational colour tag.
    pub color_label: ColorLabel,
    /// Non-destructive effects, applied in order after the content is produced
    /// and before the layer is blended into the backdrop.
    #[serde(default)]
    pub effects: Vec<LayerEffect>,
    /// The payload.
    pub content: LayerContent,
}

impl Layer {
    /// A new raster layer of the given document size.
    pub fn raster(id: LayerId, name: impl Into<String>, width: u32, height: u32) -> Self {
        Self::with_content(id, name, LayerContent::Raster(RasterContent::new(width, height)))
    }

    /// A new empty group.
    pub fn group(id: LayerId, name: impl Into<String>) -> Self {
        Self::with_content(id, name, LayerContent::Group(GroupContent::default()))
    }

    /// A new adjustment layer.
    pub fn adjustment(id: LayerId, name: impl Into<String>, adjustment: Adjustment) -> Self {
        Self::with_content(
            id,
            name,
            LayerContent::Adjustment(AdjustmentContent { adjustment }),
        )
    }

    /// A new fill layer.
    pub fn fill(id: LayerId, name: impl Into<String>, color: Rgba8) -> Self {
        Self::with_content(id, name, LayerContent::Fill(FillContent { color }))
    }

    /// A layer with arbitrary content and default properties.
    pub fn with_content(id: LayerId, name: impl Into<String>, content: LayerContent) -> Self {
        Self {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            alpha_lock: false,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            transform: Transform2D::IDENTITY,
            clipping: false,
            mask: None,
            mask_enabled: true,
            color_label: ColorLabel::None,
            effects: Vec::new(),
            content,
        }
    }

    /// The discriminant of [`Layer::content`].
    pub fn kind(&self) -> LayerKind {
        match &self.content {
            LayerContent::Raster(_) => LayerKind::Raster,
            LayerContent::Group(_) => LayerKind::Group,
            LayerContent::Adjustment(_) => LayerKind::Adjustment,
            LayerContent::Fill(_) => LayerKind::Fill,
            LayerContent::Live2D(_) => LayerKind::Live2D,
            LayerContent::Custom { .. } => LayerKind::Custom,
        }
    }

    /// The pixel buffer, for raster layers.
    pub fn pixmap(&self) -> Option<&Pixmap> {
        match &self.content {
            LayerContent::Raster(r) => Some(&r.pixmap),
            _ => None,
        }
    }

    /// The pixel buffer for editing, for raster layers.
    pub fn pixmap_mut(&mut self) -> Option<&mut Pixmap> {
        match &mut self.content {
            LayerContent::Raster(r) => Some(&mut r.pixmap),
            _ => None,
        }
    }

    /// Children, for groups.
    pub fn children(&self) -> &[LayerId] {
        match &self.content {
            LayerContent::Group(g) => &g.children,
            _ => &[],
        }
    }

    /// True when this layer can hold children.
    pub fn is_group(&self) -> bool {
        matches!(self.content, LayerContent::Group(_))
    }

    /// True when a painting tool may write to this layer right now.
    pub fn is_paintable(&self) -> bool {
        !self.locked && matches!(self.content, LayerContent::Raster(_))
    }

    /// True when this layer has at least one enabled effect.
    pub fn has_effects(&self) -> bool {
        self.effects.iter().any(|e| e.enabled)
    }

    /// The mask, if one exists and is enabled.
    pub fn active_mask(&self) -> Option<&Mask> {
        if self.mask_enabled {
            self.mask.as_ref()
        } else {
            None
        }
    }

    /// Bounding box of the layer's own content, in document coordinates.
    ///
    /// Adjustment and fill layers cover the whole canvas, so they report `None`
    /// and the caller uses the document bounds instead.
    pub fn content_bounds(&self) -> Option<IRect> {
        match &self.content {
            LayerContent::Raster(r) => Some(r.pixmap.opaque_bounds()),
            _ => None,
        }
    }

    /// Resize this layer's buffers to a new canvas size, anchoring at the origin.
    pub fn resize_canvas(&mut self, width: u32, height: u32) {
        if let LayerContent::Raster(r) = &mut self.content {
            r.pixmap = r.pixmap.resized_canvas(width, height);
        }
        if let Some(mask) = &self.mask {
            self.mask = Some(mask.resized(width, height));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_layers_start_empty_and_visible() {
        let layer = Layer::raster(LayerId(1), "Layer 1", 8, 8);
        assert!(layer.visible);
        assert_eq!(layer.opacity, 1.0);
        assert_eq!(layer.kind(), LayerKind::Raster);
        assert!(layer.is_paintable());
        assert_eq!(layer.pixmap().map(|p| p.width()), Some(8));
        assert!(layer.content_bounds().unwrap_or(IRect::EMPTY).is_empty());
    }

    #[test]
    fn locked_layers_are_not_paintable() {
        let mut layer = Layer::raster(LayerId(1), "L", 4, 4);
        layer.locked = true;
        assert!(!layer.is_paintable());
    }

    #[test]
    fn groups_are_not_paintable_but_hold_children() {
        let mut group = Layer::group(LayerId(2), "Folder");
        assert!(!group.is_paintable());
        assert!(group.is_group());
        if let LayerContent::Group(g) = &mut group.content {
            g.children.push(LayerId(3));
        }
        assert_eq!(group.children(), &[LayerId(3)]);
    }

    #[test]
    fn disabled_masks_are_ignored() {
        let mut layer = Layer::raster(LayerId(1), "L", 4, 4);
        layer.mask = Some(Mask::filled(4, 4, 255));
        assert!(layer.active_mask().is_some());
        layer.mask_enabled = false;
        assert!(layer.active_mask().is_none());
    }

    #[test]
    fn resizing_grows_pixels_and_mask() {
        let mut layer = Layer::raster(LayerId(1), "L", 4, 4);
        layer.mask = Some(Mask::filled(4, 4, 255));
        layer.resize_canvas(8, 6);
        assert_eq!(layer.pixmap().map(|p| (p.width(), p.height())), Some((8, 6)));
        assert_eq!(layer.mask.as_ref().map(|m| m.width()), Some(8));
    }

    #[test]
    fn custom_content_round_trips_through_serde() {
        let layer = Layer::with_content(
            LayerId(9),
            "Plugin layer",
            LayerContent::Custom {
                kind: "studio.example.halftone".into(),
                payload: serde_json::json!({ "dots": 12, "angle": 45.0 }),
            },
        );
        let text = serde_json::to_string(&layer).expect("serialize");
        let back: Layer = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back, layer);
        assert_eq!(back.kind(), LayerKind::Custom);
    }
}
