//! The layer-tree compositor.
//!
//! Compositing walks the tree bottom-to-top, keeping one *backdrop* buffer that
//! each layer is blended into. Four cases need more than a straight blend:
//!
//! * **Clipping groups.** A run of clipping layers sitting on a base layer is
//!   composited into a private buffer first, with the base's alpha as a mask,
//!   and the result is blended into the backdrop using the *base's* opacity and
//!   blend mode.
//! * **Isolated groups.** The group's children are composited onto an empty
//!   buffer, so the group's blend mode applies to the group as a whole.
//! * **Pass-through groups.** Children blend directly against the backdrop, as
//!   if the group were not there. A pass-through group with an opacity, mask or
//!   blend mode of its own cannot behave that way, so it is isolated instead.
//! * **Adjustment layers.** The adjustment is evaluated against the current
//!   backdrop and the result is blended back in, which is what makes it
//!   non-destructive and maskable.
//!
//! A layer's non-destructive effect stack is evaluated between producing its
//! content and blending it in, so a drop shadow lands behind the layer but in
//! front of everything below it.
//!
//! ## Rigged layers
//!
//! When the document's rig binds a mesh to a raster layer, the rig is posed
//! once per pass and the layer's pixels are redrawn through the deformed mesh
//! *as the layer's content*. Everything downstream — effects, masks, clipping,
//! all blend modes, adjustment layers above — applies to the deformed result
//! exactly as it would to painted pixels. Keyed draw order moves a rigged
//! layer among its siblings, carrying its clipping layers with it. A mesh at
//! rest draws the layer directly, so binding a mesh never changes a single
//! pixel until something moves.

use aether_core::blend::BlendMode;
use aether_core::color::Rgba8;
use aether_core::math::IRect;
use aether_core::LayerId;
use aether_document::layer::{Layer, LayerContent};
use aether_document::rig::RigPose;
use aether_document::{Background, Document};
use aether_raster::composite::{composite_pixmap, CompositeOptions};
use aether_raster::mesh::{draw_textured_mesh, MeshDrawOptions};
use aether_raster::transform::{transform_pixmap, Interpolation};
use aether_raster::{Mask, Pixmap};
use std::borrow::Cow;
use std::collections::HashMap;

/// Renders content for a plugin-defined layer.
///
/// Registering a renderer is how a plugin's layer type becomes visible without
/// the compositor knowing anything about it. Unclaimed custom layers are
/// skipped, and their data is preserved on save.
pub trait CustomContentRenderer: Send + Sync {
    /// Produce the layer's pixels in document space, or `None` to skip it.
    fn render(&self, layer: &Layer, width: u32, height: u32) -> Option<Pixmap>;
}

/// Knobs for one render pass.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Limit work to this document-space rectangle.
    pub region: Option<IRect>,
    /// Draw the document background beneath the layers.
    pub include_background: bool,
    /// Render only this layer (used by thumbnails and by "view layer alone").
    pub solo_layer: Option<LayerId>,
    /// Resampling used when a layer carries a transform.
    pub interpolation: Interpolation,
    /// Pose rigged layers. When false, rigged layers draw as painted, which is
    /// what the rig editor shows in "rest pose" mode.
    pub deform: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            region: None,
            include_background: true,
            solo_layer: None,
            interpolation: Interpolation::Bilinear,
            deform: true,
        }
    }
}

impl RenderOptions {
    /// Restrict the pass to `region`.
    pub fn with_region(mut self, region: IRect) -> Self {
        self.region = Some(region);
        self
    }
}

/// Composites documents into flat pixel buffers.
#[derive(Default)]
pub struct Compositor {
    custom: HashMap<String, Box<dyn CustomContentRenderer>>,
}

impl std::fmt::Debug for Compositor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compositor")
            .field("custom_renderers", &self.custom.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Everything one pass needs, threaded through the recursive walk.
struct Pass<'a> {
    doc: &'a Document,
    options: &'a RenderOptions,
    region: IRect,
    pose: Option<RigPose>,
}

impl Pass<'_> {
    /// The posed mesh for a layer, when it is deformed away from rest.
    fn deformed(&self, id: LayerId) -> Option<&aether_document::rig::MeshPose> {
        self.pose.as_ref()?.mesh(id).filter(|m| !m.rest)
    }

    /// Where a layer's pixels can be, when that is cheaply known to be less
    /// than the whole pass: a deformed mesh with no effects or transform
    /// draws only inside its posed bounds. Compositing outside it would
    /// visit nothing but transparent pixels.
    fn content_limit(&self, layer: &Layer) -> IRect {
        if layer.has_effects() || !layer.transform.is_identity() {
            return self.region;
        }
        match (&layer.content, self.deformed(layer.id)) {
            (LayerContent::Raster(_), Some(pose)) => {
                pose.bounds().to_irect_outer().expanded(1).intersect(&self.region)
            }
            _ => self.region,
        }
    }

    /// Keyed draw-order offset of a layer.
    fn draw_order(&self, id: LayerId) -> f32 {
        self.pose
            .as_ref()
            .and_then(|p| p.mesh(id))
            .map(|m| m.draw_order)
            .unwrap_or(0.0)
    }
}

impl Compositor {
    /// A compositor with no plugin renderers registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a renderer for a [`LayerContent::Custom`] type tag.
    pub fn register_custom(&mut self, kind: impl Into<String>, renderer: Box<dyn CustomContentRenderer>) {
        self.custom.insert(kind.into(), renderer);
    }

    /// Composite the whole document into a new buffer.
    pub fn render(&self, doc: &Document) -> Pixmap {
        self.render_with(doc, &RenderOptions::default())
    }

    /// Composite with explicit options.
    pub fn render_with(&self, doc: &Document, options: &RenderOptions) -> Pixmap {
        let mut target = Pixmap::new(doc.width, doc.height);
        self.render_into(doc, &mut target, options);
        target
    }

    fn pass<'a>(&self, doc: &'a Document, options: &'a RenderOptions, region: IRect) -> Pass<'a> {
        let pose = (options.deform && !doc.rig.is_inert()).then(|| doc.rig.evaluate());
        Pass {
            doc,
            options,
            region,
            pose,
        }
    }

    /// Composite into an existing document-sized buffer.
    ///
    /// Only `options.region` is touched, which is what makes incremental
    /// repaints cheap: a brush dab re-composites a few hundred pixels, not the
    /// whole canvas.
    pub fn render_into(&self, doc: &Document, target: &mut Pixmap, options: &RenderOptions) {
        let region = options
            .region
            .unwrap_or_else(|| doc.bounds())
            .intersect(&doc.bounds())
            .intersect(&target.bounds());
        if region.is_empty() {
            return;
        }

        // Start from the background (or nothing) rather than whatever the
        // buffer happened to contain.
        match (options.include_background, doc.background) {
            (true, Background::Solid(color)) => target.fill_rect(region, color),
            _ => target.fill_rect(region, Rgba8::TRANSPARENT),
        }

        let pass = self.pass(doc, options, region);
        self.composite_children(&pass, None, target);
    }

    /// Composite one layer on its own, ignoring the rest of the tree.
    ///
    /// Used for layer thumbnails.
    pub fn render_layer(&self, doc: &Document, id: LayerId) -> Pixmap {
        let mut target = Pixmap::new(doc.width, doc.height);
        let options = RenderOptions {
            include_background: false,
            solo_layer: Some(id),
            ..Default::default()
        };
        let Some(layer) = doc.layers.get(id) else {
            return target;
        };
        let pass = self.pass(doc, &options, doc.bounds());
        if let Some(source) = self.layer_source(&pass, layer, &target) {
            let opts = CompositeOptions {
                blend: BlendMode::Normal,
                opacity: 1.0,
                offset: (0, 0),
                region: Some(pass.region),
                alpha_lock: false,
            };
            composite_pixmap(&mut target, &source, &opts, None);
        }
        target
    }

    /// Composite the children of `parent` onto `backdrop`.
    fn composite_children(&self, pass: &Pass, parent: Option<LayerId>, backdrop: &mut Pixmap) {
        let doc = pass.doc;
        let children: &[LayerId] = doc.layers.children_of(parent);

        // Group each base layer with the run of clipping layers riding on it.
        // Clipping layers at the very bottom have no base and are skipped.
        let mut units: Vec<(f32, LayerId, Vec<LayerId>)> = Vec::new();
        for (index, &id) in children.iter().enumerate() {
            let Some(layer) = doc.layers.get(id) else {
                continue;
            };
            if layer.clipping {
                if let Some(unit) = units.last_mut() {
                    unit.2.push(id);
                }
                continue;
            }
            units.push((index as f32 + pass.draw_order(id), id, Vec::new()));
        }
        // Keyed draw order moves whole units; a stable sort keeps the tree
        // order wherever no offsets are keyed.
        if units
            .iter()
            .enumerate()
            .any(|(i, u)| i > 0 && u.0 < units[i - 1].0)
        {
            units.sort_by(|a, b| a.0.total_cmp(&b.0));
        }

        for (_, id, clip_run) in &units {
            let Some(layer) = doc.layers.get(*id) else {
                continue;
            };
            if self.is_hidden(pass, layer) {
                continue;
            }
            if clip_run.is_empty() {
                self.composite_layer(pass, layer, backdrop);
            } else {
                self.composite_clipping_group(pass, layer, clip_run, backdrop);
            }
        }
    }

    fn is_hidden(&self, pass: &Pass, layer: &Layer) -> bool {
        if !layer.visible || layer.opacity <= 0.0 {
            return true;
        }
        if let Some(solo) = pass.options.solo_layer {
            // A solo layer's ancestors still have to be traversed.
            if layer.id != solo && !pass.doc.layers.subtree_ids(layer.id).contains(&solo) {
                return true;
            }
        }
        false
    }

    /// Blend one layer into the backdrop.
    fn composite_layer(&self, pass: &Pass, layer: &Layer, backdrop: &mut Pixmap) {
        // A pass-through group blends its children straight into the backdrop.
        if let LayerContent::Group(group) = &layer.content {
            let needs_isolation = group.isolate
                || layer.opacity < 1.0
                || layer.blend_mode != BlendMode::Normal
                || layer.active_mask().is_some();
            if !needs_isolation {
                self.composite_children(pass, Some(layer.id), backdrop);
                return;
            }
        }

        let limit = pass.content_limit(layer);
        if limit.is_empty() {
            return;
        }
        let Some(source) = self.layer_source(pass, layer, backdrop) else {
            return;
        };
        let opts = CompositeOptions {
            blend: layer.blend_mode,
            opacity: layer.opacity,
            offset: (0, 0),
            region: Some(limit),
            alpha_lock: false,
        };
        composite_pixmap(backdrop, &source, &opts, layer.active_mask());
    }

    /// Composite a base layer plus the clipping layers riding on it.
    fn composite_clipping_group(
        &self,
        pass: &Pass,
        base: &Layer,
        clipped: &[LayerId],
        backdrop: &mut Pixmap,
    ) {
        // Nothing in the group can show outside the base.
        let limit = pass.content_limit(base);
        if limit.is_empty() {
            return;
        }
        let Some(base_pixels) = self.layer_source(pass, base, backdrop) else {
            return;
        };
        // The clipping shape is the base layer's own alpha.
        let clip_mask = Mask::from_alpha(&base_pixels);

        let mut group = base_pixels.into_owned();
        for id in clipped {
            let Some(layer) = pass.doc.layers.get(*id) else {
                continue;
            };
            if self.is_hidden(pass, layer) {
                continue;
            }
            let Some(source) = self.layer_source(pass, layer, &group) else {
                continue;
            };
            let mut mask = clip_mask.clone();
            if let Some(layer_mask) = layer.active_mask() {
                mask.multiply(layer_mask);
            }
            let opts = CompositeOptions {
                blend: layer.blend_mode,
                opacity: layer.opacity,
                offset: (0, 0),
                region: Some(limit),
                alpha_lock: false,
            };
            composite_pixmap(&mut group, &source, &opts, Some(&mask));
        }

        let opts = CompositeOptions {
            blend: base.blend_mode,
            opacity: base.opacity,
            offset: (0, 0),
            region: Some(limit),
            alpha_lock: false,
        };
        composite_pixmap(backdrop, &group, &opts, base.active_mask());
    }

    /// Produce the pixels a layer contributes, in document space.
    ///
    /// `backdrop` is what sits below the layer; adjustment layers read it.
    /// Plain raster layers are borrowed rather than copied: at 4K a copy per
    /// layer per dab would dominate the cost of painting.
    fn layer_source<'p>(
        &self,
        pass: &'p Pass,
        layer: &'p Layer,
        backdrop: &Pixmap,
    ) -> Option<Cow<'p, Pixmap>> {
        let doc = pass.doc;
        let region = pass.region;
        let source: Cow<'p, Pixmap> = match &layer.content {
            LayerContent::Raster(raster) => match (pass.deformed(layer.id), doc.rig.mesh(layer.id)) {
                (Some(pose), Some(mesh)) => {
                    let mut deformed = Pixmap::new(doc.width, doc.height);
                    let opts = MeshDrawOptions {
                        region: pass.content_limit(layer),
                        opacity: pose.opacity,
                        multiply: pose.multiply,
                        screen: pose.screen,
                        interpolation: pass.options.interpolation,
                    };
                    draw_textured_mesh(
                        &mut deformed,
                        &raster.pixmap,
                        &pose.positions,
                        &mesh.vertices,
                        &mesh.triangles,
                        &opts,
                    );
                    Cow::Owned(deformed)
                }
                _ => Cow::Borrowed(&raster.pixmap),
            },
            LayerContent::Fill(fill) => {
                let mut pm = Pixmap::new(doc.width, doc.height);
                pm.fill_rect(region, fill.color);
                Cow::Owned(pm)
            }
            LayerContent::Live2D(content) => {
                let model = doc.rig.cubism.iter().find(|c| c.layer == layer.id)?;
                // Without a pose (the rest view, or deformation off) the
                // model shows at its defaults.
                let rest;
                let pose = match pass.pose.as_ref().and_then(|p| p.cubism(layer.id)) {
                    Some(pose) => pose,
                    None => {
                        rest = model.evaluate_rest();
                        &rest
                    }
                };
                Cow::Owned(crate::live2d::render(
                    model,
                    &content.textures,
                    pose,
                    doc.width,
                    doc.height,
                    region,
                    pass.options.interpolation,
                ))
            }
            LayerContent::Adjustment(adjustment) => {
                let mut adjusted = backdrop.clone();
                adjustment.adjustment.apply(&mut adjusted, Some(region));
                Cow::Owned(adjusted)
            }
            LayerContent::Group(_) => {
                // Isolated group: composite the children onto an empty buffer.
                let mut buffer = Pixmap::new(doc.width, doc.height);
                self.composite_children(pass, Some(layer.id), &mut buffer);
                Cow::Owned(buffer)
            }
            LayerContent::Custom { kind, .. } => {
                let renderer = self.custom.get(kind)?;
                Cow::Owned(renderer.render(layer, doc.width, doc.height)?)
            }
        };

        let source = if layer.transform.is_identity() {
            source
        } else {
            Cow::Owned(transform_pixmap(
                &source,
                &layer.transform,
                doc.width,
                doc.height,
                pass.options.interpolation,
            ))
        };

        // Effects run last, so they see the layer exactly as it will be blended
        // — including its transform — but before opacity, mask and blend mode.
        if layer.has_effects() {
            Some(Cow::Owned(aether_raster::effect::apply_stack(
                &source,
                &layer.effects,
            )))
        } else {
            Some(source)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;
    use aether_document::layer::{GroupContent, Layer, LayerContent};
    use aether_raster::adjust::Adjustment;

    fn doc_with_layers(n: usize) -> (Document, Vec<LayerId>) {
        let mut doc = Document::empty(8, 8, "test");
        let mut ids = Vec::new();
        for i in 0..n {
            let id = doc.next_layer_id();
            doc.layers
                .push_top(Layer::raster(id, format!("L{i}"), 8, 8))
                .expect("insert");
            ids.push(id);
        }
        doc.active_layer = ids.last().copied().unwrap_or(LayerId::NONE);
        (doc, ids)
    }

    fn fill_layer(doc: &mut Document, id: LayerId, color: Rgba8) {
        if let Some(pm) = doc.layers.get_mut(id).and_then(|l| l.pixmap_mut()) {
            pm.fill(color);
        }
    }

    #[test]
    fn an_empty_document_renders_transparent() {
        let doc = Document::empty(4, 4, "empty");
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(2, 2), Rgba8::TRANSPARENT);
    }

    #[test]
    fn the_top_layer_wins_at_full_opacity() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::BLACK);
        fill_layer(&mut doc, ids[1], Rgba8::WHITE);
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(4, 4), Rgba8::WHITE);
    }

    #[test]
    fn hidden_layers_do_not_contribute() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::BLACK);
        fill_layer(&mut doc, ids[1], Rgba8::WHITE);
        if let Some(l) = doc.layers.get_mut(ids[1]) {
            l.visible = false;
        }
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(4, 4), Rgba8::BLACK);
    }

    #[test]
    fn opacity_blends_the_stack() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::BLACK);
        fill_layer(&mut doc, ids[1], Rgba8::WHITE);
        if let Some(l) = doc.layers.get_mut(ids[1]) {
            l.opacity = 0.5;
        }
        let out = Compositor::new().render(&doc);
        assert!(
            (out.get(4, 4).r as i32 - 128).abs() <= 2,
            "got {:?}",
            out.get(4, 4)
        );
    }

    #[test]
    fn multiply_blend_darkens() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::rgb(200, 200, 200));
        fill_layer(&mut doc, ids[1], Rgba8::rgb(128, 128, 128));
        if let Some(l) = doc.layers.get_mut(ids[1]) {
            l.blend_mode = BlendMode::Multiply;
        }
        let out = Compositor::new().render(&doc);
        assert!(
            out.get(4, 4).r < 110,
            "multiply should darken, got {:?}",
            out.get(4, 4)
        );
    }

    #[test]
    fn a_solid_background_shows_through_transparent_layers() {
        let (mut doc, _) = doc_with_layers(1);
        doc.background = Background::Solid(Rgba8::rgb(20, 40, 60));
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(1, 1), Rgba8::rgb(20, 40, 60));
    }

    #[test]
    fn layer_masks_hide_pixels() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::BLACK);
        fill_layer(&mut doc, ids[1], Rgba8::WHITE);
        let mut mask = Mask::new(8, 8);
        mask.fill_rect(IRect::new(0, 0, 4, 8), 255);
        if let Some(l) = doc.layers.get_mut(ids[1]) {
            l.mask = Some(mask);
        }
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(1, 1), Rgba8::WHITE, "inside the mask");
        assert_eq!(out.get(6, 1), Rgba8::BLACK, "outside the mask");
    }

    #[test]
    fn clipping_layers_are_limited_to_the_base_alpha() {
        let (mut doc, ids) = doc_with_layers(2);
        // Base covers only the left half.
        if let Some(pm) = doc.layers.get_mut(ids[0]).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(0, 0, 4, 8), Rgba8::BLACK);
        }
        fill_layer(&mut doc, ids[1], Rgba8::WHITE);
        if let Some(l) = doc.layers.get_mut(ids[1]) {
            l.clipping = true;
        }
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(1, 1), Rgba8::WHITE, "clipped layer shows over the base");
        assert_eq!(
            out.get(6, 1),
            Rgba8::TRANSPARENT,
            "clipped layer must not escape the base"
        );
    }

    #[test]
    fn base_opacity_applies_to_the_whole_clipping_group() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::BLACK);
        fill_layer(&mut doc, ids[1], Rgba8::WHITE);
        if let Some(l) = doc.layers.get_mut(ids[1]) {
            l.clipping = true;
        }
        if let Some(l) = doc.layers.get_mut(ids[0]) {
            l.opacity = 0.5;
        }
        let out = Compositor::new().render(&doc);
        assert!(
            (out.get(4, 4).a as i32 - 128).abs() <= 2,
            "group opacity ignored: {:?}",
            out.get(4, 4)
        );
    }

    #[test]
    fn adjustment_layers_affect_what_is_below() {
        let (mut doc, ids) = doc_with_layers(1);
        fill_layer(&mut doc, ids[0], Rgba8::rgb(10, 20, 30));
        let adj = doc.next_layer_id();
        doc.layers
            .push_top(Layer::adjustment(adj, "Invert", Adjustment::Invert))
            .expect("insert");
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(4, 4), Rgba8::rgb(245, 235, 225));
    }

    #[test]
    fn masked_adjustments_only_affect_the_masked_area() {
        let (mut doc, ids) = doc_with_layers(1);
        fill_layer(&mut doc, ids[0], Rgba8::rgb(10, 20, 30));
        let adj = doc.next_layer_id();
        let mut layer = Layer::adjustment(adj, "Invert", Adjustment::Invert);
        let mut mask = Mask::new(8, 8);
        mask.fill_rect(IRect::new(0, 0, 4, 8), 255);
        layer.mask = Some(mask);
        doc.layers.push_top(layer).expect("insert");
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(1, 1), Rgba8::rgb(245, 235, 225));
        assert_eq!(out.get(6, 1), Rgba8::rgb(10, 20, 30));
    }

    #[test]
    fn isolated_groups_blend_as_a_unit() {
        let mut doc = Document::empty(8, 8, "test");
        let base = doc.next_layer_id();
        doc.layers
            .push_top(Layer::raster(base, "base", 8, 8))
            .expect("base");
        fill_layer(&mut doc, base, Rgba8::rgb(200, 200, 200));

        let group_id = doc.next_layer_id();
        let mut group = Layer::group(group_id, "G");
        if let LayerContent::Group(g) = &mut group.content {
            *g = GroupContent {
                children: Vec::new(),
                isolate: true,
                collapsed: false,
            };
        }
        group.blend_mode = BlendMode::Multiply;
        doc.layers.push_top(group).expect("group");

        let child = doc.next_layer_id();
        doc.layers
            .insert(Layer::raster(child, "child", 8, 8), Some(group_id), 0)
            .expect("child");
        fill_layer(&mut doc, child, Rgba8::rgb(128, 128, 128));

        let out = Compositor::new().render(&doc);
        assert!(
            out.get(4, 4).r < 110,
            "group blend mode ignored: {:?}",
            out.get(4, 4)
        );
    }

    #[test]
    fn pass_through_groups_are_transparent_to_the_stack() {
        let mut doc = Document::empty(8, 8, "test");
        let group_id = doc.next_layer_id();
        doc.layers.push_top(Layer::group(group_id, "G")).expect("group");
        let child = doc.next_layer_id();
        doc.layers
            .insert(Layer::raster(child, "child", 8, 8), Some(group_id), 0)
            .expect("child");
        fill_layer(&mut doc, child, Rgba8::WHITE);

        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(4, 4), Rgba8::WHITE);
    }

    #[test]
    fn group_opacity_forces_isolation() {
        let mut doc = Document::empty(8, 8, "test");
        let group_id = doc.next_layer_id();
        let mut group = Layer::group(group_id, "G");
        group.opacity = 0.5;
        doc.layers.push_top(group).expect("group");
        let child = doc.next_layer_id();
        doc.layers
            .insert(Layer::raster(child, "child", 8, 8), Some(group_id), 0)
            .expect("child");
        fill_layer(&mut doc, child, Rgba8::WHITE);

        let out = Compositor::new().render(&doc);
        assert!(
            (out.get(4, 4).a as i32 - 128).abs() <= 2,
            "group opacity ignored: {:?}",
            out.get(4, 4)
        );
    }

    #[test]
    fn unclaimed_custom_layers_are_skipped_not_fatal() {
        let (mut doc, ids) = doc_with_layers(1);
        fill_layer(&mut doc, ids[0], Rgba8::WHITE);
        let custom = doc.next_layer_id();
        doc.layers
            .push_top(Layer::with_content(
                custom,
                "Plugin",
                LayerContent::Custom {
                    kind: "unknown.plugin".into(),
                    payload: serde_json::Value::Null,
                },
            ))
            .expect("insert");
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(4, 4), Rgba8::WHITE);
    }

    #[test]
    fn registered_custom_renderers_are_used() {
        struct RedBox;
        impl CustomContentRenderer for RedBox {
            fn render(&self, _layer: &Layer, width: u32, height: u32) -> Option<Pixmap> {
                Some(Pixmap::filled(width, height, Rgba8::new(255, 0, 0, 255)))
            }
        }
        let (mut doc, _) = doc_with_layers(0);
        let custom = doc.next_layer_id();
        doc.layers
            .push_top(Layer::with_content(
                custom,
                "Plugin",
                LayerContent::Custom {
                    kind: "test.red".into(),
                    payload: serde_json::Value::Null,
                },
            ))
            .expect("insert");
        let mut compositor = Compositor::new();
        compositor.register_custom("test.red", Box::new(RedBox));
        let out = compositor.render(&doc);
        assert_eq!(out.get(4, 4), Rgba8::new(255, 0, 0, 255));
    }

    #[test]
    fn region_rendering_matches_a_full_render() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::rgb(30, 60, 90));
        if let Some(pm) = doc.layers.get_mut(ids[1]).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(2, 2, 4, 4), Rgba8::WHITE);
        }
        let compositor = Compositor::new();
        let full = compositor.render(&doc);
        let mut partial = Pixmap::new(8, 8);
        for region in [IRect::new(0, 0, 8, 4), IRect::new(0, 4, 8, 4)] {
            compositor.render_into(&doc, &mut partial, &RenderOptions::default().with_region(region));
        }
        assert_eq!(full, partial, "region rendering must be seamless");
    }

    #[test]
    fn layer_thumbnails_ignore_the_rest_of_the_stack() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::BLACK);
        fill_layer(&mut doc, ids[1], Rgba8::WHITE);
        let thumb = Compositor::new().render_layer(&doc, ids[0]);
        assert_eq!(thumb.get(4, 4), Rgba8::BLACK);
    }
}

#[cfg(test)]
mod effect_tests {
    use super::*;
    use aether_core::color::Rgba8;
    use aether_document::layer::Layer;
    use aether_raster::{EffectKind, LayerEffect};

    fn doc_with_square() -> (Document, LayerId) {
        let mut doc = Document::empty(64, 64, "test");
        let id = doc.next_layer_id();
        doc.layers
            .push_top(Layer::raster(id, "L", 64, 64))
            .expect("insert");
        if let Some(pm) = doc.layers.get_mut(id).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(24, 24, 16, 16), Rgba8::new(200, 200, 200, 255));
        }
        doc.active_layer = id;
        (doc, id)
    }

    #[test]
    fn a_layer_effect_shows_up_in_the_composite() {
        let (mut doc, id) = doc_with_square();
        let out_before = Compositor::new().render(&doc);
        assert_eq!(out_before.get(45, 45).a, 0);

        if let Some(layer) = doc.layers.get_mut(id) {
            layer.effects.push(LayerEffect::new(EffectKind::DropShadow {
                dx: 8.0,
                dy: 8.0,
                radius: 2.0,
                color: Rgba8::BLACK,
                opacity: 1.0,
            }));
        }
        let out = Compositor::new().render(&doc);
        assert!(out.get(45, 45).a > 0, "the shadow should be composited");
        assert_eq!(
            out.get(32, 32),
            Rgba8::new(200, 200, 200, 255),
            "artwork unchanged"
        );
    }

    #[test]
    fn a_disabled_effect_costs_nothing_and_changes_nothing() {
        let (mut doc, id) = doc_with_square();
        let plain = Compositor::new().render(&doc);
        if let Some(layer) = doc.layers.get_mut(id) {
            layer.effects.push(LayerEffect {
                kind: EffectKind::Blur { sigma: 8.0 },
                enabled: false,
            });
        }
        assert_eq!(Compositor::new().render(&doc), plain);
    }

    #[test]
    fn effects_are_applied_before_layer_opacity() {
        let (mut doc, id) = doc_with_square();
        if let Some(layer) = doc.layers.get_mut(id) {
            layer.effects.push(LayerEffect::new(EffectKind::ColorOverlay {
                color: Rgba8::rgb(255, 0, 0),
                opacity: 1.0,
                blend: BlendMode::Normal,
            }));
            layer.opacity = 0.5;
        }
        let out = Compositor::new().render(&doc);
        let pixel = out.get(32, 32);
        assert_eq!(pixel.r, 255);
        assert!(
            (pixel.a as i32 - 128).abs() <= 2,
            "layer opacity should still apply: {pixel:?}"
        );
    }

    #[test]
    fn effects_stay_inside_a_layer_mask() {
        let (mut doc, id) = doc_with_square();
        if let Some(layer) = doc.layers.get_mut(id) {
            layer.effects.push(LayerEffect::new(EffectKind::Glow {
                radius: 10.0,
                intensity: 2.0,
                color: Rgba8::rgb(255, 0, 0),
            }));
            let mut mask = Mask::new(64, 64);
            mask.fill_rect(IRect::new(0, 0, 32, 64), 255);
            layer.mask = Some(mask);
        }
        let out = Compositor::new().render(&doc);
        assert!(out.get(20, 32).a > 0, "glow inside the mask");
        assert_eq!(out.get(50, 32).a, 0, "glow must not escape the mask");
    }
}

#[cfg(test)]
mod rig_tests {
    use super::*;
    use aether_core::color::Rgba8;
    use aether_core::math::{vec2, Rect};
    use aether_core::ParameterId;
    use aether_document::layer::Layer;
    use aether_document::rig::{ArtMesh, KeyAxis, Parameter};
    use aether_raster::{EffectKind, LayerEffect};

    fn doc_with_layers(n: usize) -> (Document, Vec<LayerId>) {
        let mut doc = Document::empty(64, 64, "rig");
        let mut ids = Vec::new();
        for i in 0..n {
            let id = doc.next_layer_id();
            doc.layers
                .push_top(Layer::raster(id, format!("L{i}"), 64, 64))
                .expect("insert");
            ids.push(id);
        }
        (doc, ids)
    }

    fn fill_layer(doc: &mut Document, id: LayerId, color: Rgba8) {
        if let Some(pm) = doc.layers.get_mut(id).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(0, 0, 8, 8), color);
        }
    }

    /// A 64×64 document whose single layer holds a red 8×8 square at (8, 8),
    /// meshed with a quad over the square's neighbourhood and keyed to slide
    /// 30 px right when parameter `P` is at 1.
    fn rigged_square() -> (Document, LayerId, ParameterId) {
        let (mut doc, ids) = doc_with_layers(1);
        let id = ids[0];
        if let Some(pm) = doc.layers.get_mut(id).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(8, 8, 8, 8), Rgba8::rgb(255, 0, 0));
        }
        let param = doc.ids.parameter();
        doc.rig
            .add_parameter(Parameter::new(param, "Slide", 0.0, 1.0, 0.0))
            .expect("param");
        let mut mesh = ArtMesh::quad(id, Rect::from_corners(vec2(4.0, 4.0), vec2(20.0, 20.0)));
        mesh.keyforms
            .add_axis(KeyAxis::new(param, [0.0, 1.0]).expect("axis"))
            .expect("axis");
        for o in &mut mesh.keyforms.forms[1].offsets {
            *o = vec2(30.0, 0.0);
        }
        doc.rig.set_mesh(mesh);
        (doc, id, param)
    }

    #[test]
    fn a_rigged_layer_at_rest_renders_exactly_as_painted() {
        let (doc, _, _) = rigged_square();
        let mut plain = doc.clone();
        plain.rig = Default::default();
        assert_eq!(Compositor::new().render(&doc), Compositor::new().render(&plain));
    }

    #[test]
    fn posing_a_parameter_moves_the_layer() {
        let (mut doc, _, param) = rigged_square();
        doc.rig.set_value(param, 1.0);
        let out = Compositor::new().render(&doc);
        assert_eq!(out.get(10, 10).a, 0, "the square left its painted position");
        assert_eq!(out.get(40, 10), Rgba8::rgb(255, 0, 0), "and arrived 30 px right");
        doc.rig.set_value(param, 0.5);
        let half = Compositor::new().render(&doc);
        assert_eq!(
            half.get(25, 10),
            Rgba8::rgb(255, 0, 0),
            "halfway at half the value"
        );
    }

    #[test]
    fn rigged_layers_keep_blend_modes_opacity_and_effects() {
        let (mut doc, id, param) = rigged_square();
        doc.rig.set_value(param, 1.0);
        if let Some(layer) = doc.layers.get_mut(id) {
            layer.opacity = 0.5;
            layer.effects.push(LayerEffect::new(EffectKind::ColorOverlay {
                color: Rgba8::rgb(0, 0, 255),
                opacity: 1.0,
                blend: BlendMode::Normal,
            }));
        }
        let out = Compositor::new().render(&doc);
        let px = out.get(40, 10);
        assert_eq!((px.r, px.b), (0, 255), "the effect sees the deformed pixels");
        assert!((px.a as i32 - 128).abs() <= 2, "and layer opacity still applies");
    }

    #[test]
    fn keyed_opacity_and_tint_reach_the_composite() {
        let (mut doc, id, param) = rigged_square();
        if let Some(mesh) = doc.rig.mesh_mut(id) {
            mesh.keyforms.forms[1]
                .offsets
                .iter_mut()
                .for_each(|o| *o = vec2(0.0, 0.0));
            mesh.keyforms.forms[1].opacity = 0.0;
            mesh.keyforms.forms[1].multiply = [0.0, 1.0, 1.0];
        }
        doc.rig.set_value(param, 0.5);
        let out = Compositor::new().render(&doc);
        let px = out.get(10, 10);
        assert!((px.a as i32 - 128).abs() <= 2, "half faded: {px:?}");
        assert!((px.r as i32 - 128).abs() <= 2, "half tinted: {px:?}");
    }

    #[test]
    fn keyed_draw_order_moves_a_layer_above_its_sibling() {
        let (mut doc, ids) = doc_with_layers(2);
        fill_layer(&mut doc, ids[0], Rgba8::rgb(255, 0, 0));
        fill_layer(&mut doc, ids[1], Rgba8::rgb(0, 255, 0));
        assert_eq!(Compositor::new().render(&doc).get(4, 4), Rgba8::rgb(0, 255, 0));
        let param = doc.ids.parameter();
        doc.rig
            .add_parameter(Parameter::new(param, "Front", 0.0, 1.0, 0.0))
            .expect("param");
        let mut mesh = ArtMesh::quad(ids[0], Rect::from_corners(vec2(0.0, 0.0), vec2(8.0, 8.0)));
        mesh.keyforms
            .add_axis(KeyAxis::new(param, [0.0, 1.0]).expect("axis"))
            .expect("axis");
        mesh.keyforms.forms[1].draw_order = 1.5;
        doc.rig.set_mesh(mesh);
        doc.rig.set_value(param, 1.0);
        assert_eq!(
            Compositor::new().render(&doc).get(4, 4),
            Rgba8::rgb(255, 0, 0),
            "the bottom layer was keyed in front"
        );
    }

    #[test]
    fn deformation_can_be_switched_off_for_the_rest_pose_view() {
        let (mut doc, _, param) = rigged_square();
        doc.rig.set_value(param, 1.0);
        let options = RenderOptions {
            deform: false,
            ..Default::default()
        };
        let out = Compositor::new().render_with(&doc, &options);
        assert_eq!(out.get(10, 10), Rgba8::rgb(255, 0, 0));
    }

    #[test]
    fn region_renders_of_a_posed_rig_match_a_full_render() {
        let (mut doc, _, param) = rigged_square();
        doc.rig.set_value(param, 0.7);
        let compositor = Compositor::new();
        let full = compositor.render(&doc);
        let mut partial = full.clone();
        partial.fill(Rgba8::rgb(1, 2, 3));
        for region in [IRect::new(0, 0, 32, 64), IRect::new(32, 0, 32, 64)] {
            compositor.render_into(&doc, &mut partial, &RenderOptions::default().with_region(region));
        }
        assert_eq!(partial, full);
    }
}
