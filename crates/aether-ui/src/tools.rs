//! Editing tools.
//!
//! A tool turns pointer input into commands. Tools never touch pixels directly:
//! they build a [`Command`](aether_document::Command) and hand it to the
//! history, so everything they do is undoable and reachable from tests.
//!
//! ## Live strokes without losing undo
//!
//! Painting has to be visible while the pointer is down, but the history entry
//! can only be built when the stroke ends and its extent is known. The stroke
//! tools solve this with a [`StrokeSnapshot`]: the first time a stroke touches
//! a tile, that tile's original pixels are copied aside. Each frame the touched
//! tiles are restored from the snapshot and the whole accumulated stroke is
//! re-composited, which is also what stops overlapping dabs inside one stroke
//! from darkening each other. When the pointer lifts, the snapshot provides the
//! "before" image for the undo command.

use aether_core::blend::BlendMode;
use aether_core::color::{Rgba, Rgba8};
use aether_core::input::{InputSample, Modifiers, PointerButton};
use aether_core::math::{IRect, Vec2};
use aether_document::command::{RegionEdit, SetSelectionCommand};
use aether_document::selection::{Selection, SelectionMode};
use aether_document::{Document, History, LayerId};
use aether_raster::composite::{erase_masked, fill_masked, CompositeOptions};
use aether_raster::tile::TileIter;
use aether_raster::{fill, BrushPreset, Pixmap, StrokeState};
use aether_render::Viewport;
use std::collections::BTreeMap;

/// Identifies a tool for the toolbar and for shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolId {
    /// Paint with the current brush.
    Brush,
    /// Erase with the current brush shape.
    Eraser,
    /// Flood fill.
    Bucket,
    /// Pick a colour from the canvas.
    Eyedropper,
    /// Rectangular marquee.
    RectSelect,
    /// Elliptical marquee.
    EllipseSelect,
    /// Freehand lasso.
    Lasso,
    /// Select by colour similarity.
    MagicWand,
    /// Move the active layer's pixels.
    Move,
    /// Pan the view.
    Pan,
    /// A tool contributed by a plugin.
    Custom(&'static str),
}

impl ToolId {
    /// Translation key for the tool's name.
    pub fn label_key(self) -> &'static str {
        match self {
            ToolId::Brush => "tool.brush",
            ToolId::Eraser => "tool.eraser",
            ToolId::Bucket => "tool.bucket",
            ToolId::Eyedropper => "tool.eyedropper",
            ToolId::RectSelect => "tool.rect_select",
            ToolId::EllipseSelect => "tool.ellipse_select",
            ToolId::Lasso => "tool.lasso",
            ToolId::MagicWand => "tool.wand",
            ToolId::Move => "tool.move",
            ToolId::Pan => "tool.pan",
            ToolId::Custom(name) => name,
        }
    }

    /// Short icon glyph for the toolbar.
    pub fn glyph(self) -> &'static str {
        match self {
            ToolId::Brush => "✏",
            ToolId::Eraser => "⌫",
            ToolId::Bucket => "🪣",
            ToolId::Eyedropper => "💧",
            ToolId::RectSelect => "▭",
            ToolId::EllipseSelect => "◯",
            ToolId::Lasso => "✎",
            ToolId::MagicWand => "✨",
            ToolId::Move => "✥",
            ToolId::Pan => "✋",
            ToolId::Custom(_) => "＊",
        }
    }
}

/// One pointer event, already mapped into document space.
#[derive(Clone, Copy, Debug)]
pub struct ToolEvent {
    /// Position, pressure and timing in document space.
    pub sample: InputSample,
    /// Raw screen position, for view-space tools like panning.
    pub screen: Vec2,
    /// Movement since the previous event, in screen pixels.
    pub screen_delta: Vec2,
    /// Which button is involved.
    pub button: PointerButton,
    /// Modifier keys.
    pub modifiers: Modifiers,
}

/// Everything a tool is allowed to touch.
pub struct ToolContext<'a> {
    /// The document being edited.
    pub doc: &'a mut Document,
    /// The undo history.
    pub history: &'a mut History,
    /// The canvas view (pan/zoom tools adjust it).
    pub viewport: &'a mut Viewport,
    /// The current brush.
    pub brush: &'a BrushPreset,
    /// Foreground colour; the eyedropper writes to it.
    pub primary: &'a mut Rgba,
    /// Background colour.
    pub secondary: Rgba,
    /// Flood-fill / wand tolerance in `0..=1`.
    pub tolerance: f32,
    /// Whether flood fill and the wand sample the composite instead of the layer.
    pub sample_all_layers: bool,
    /// How new selections combine with the current one.
    pub selection_mode: SelectionMode,
    /// The last composited image, for sampling.
    pub composite: &'a Pixmap,
    /// Set by a tool to report something to the status bar.
    pub status: Option<String>,
}

impl ToolContext<'_> {
    /// Report a message to the status bar.
    pub fn report(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
    }
}

/// What a tool wants drawn on top of the canvas while it is active.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolPreview {
    /// A rubber-band rectangle in document space.
    Rect(IRect),
    /// A rubber-band ellipse in document space.
    Ellipse(IRect),
    /// A freehand outline in document space.
    Polyline(Vec<Vec2>),
}

/// The interface every tool implements.
pub trait Tool {
    /// Which tool this is.
    fn id(&self) -> ToolId;

    /// Pointer pressed.
    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent);

    /// Pointer moved (only delivered while a gesture is in progress).
    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent);

    /// Pointer released.
    fn pointer_up(&mut self, ctx: &mut ToolContext, event: &ToolEvent);

    /// Abandon any in-progress gesture without committing it.
    fn cancel(&mut self, _ctx: &mut ToolContext) {}

    /// Overlay to draw while the gesture is in progress.
    fn preview(&self) -> Option<ToolPreview> {
        None
    }

    /// True while the tool is mid-gesture.
    fn is_active(&self) -> bool {
        false
    }
}

/// Original pixels of the tiles a gesture has touched.
///
/// Copying whole tiles (rather than the exact dirty rectangle) keeps the
/// bookkeeping simple and bounded: a stroke across a 4000x4000 canvas copies
/// only the tiles it actually crossed.
#[derive(Debug, Default)]
pub struct StrokeSnapshot {
    tiles: BTreeMap<(i32, i32), Pixmap>,
    bounds: IRect,
}

impl StrokeSnapshot {
    /// An empty snapshot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Copy any tile overlapping `rect` that has not been captured yet.
    pub fn capture(&mut self, source: &Pixmap, rect: IRect) {
        for (index, _) in TileIter::new(rect.intersect(&source.bounds())) {
            let key = (index.tx, index.ty);
            if self.tiles.contains_key(&key) {
                continue;
            }
            let tile_rect = index.rect().intersect(&source.bounds());
            if tile_rect.is_empty() {
                continue;
            }
            self.tiles.insert(key, source.copy_rect(tile_rect));
            self.bounds = self.bounds.union(&tile_rect);
        }
    }

    /// Put the captured pixels back into `target` for the tiles covering `rect`.
    pub fn restore(&self, target: &mut Pixmap, rect: IRect) {
        for (index, _) in TileIter::new(rect) {
            if let Some(tile) = self.tiles.get(&(index.tx, index.ty)) {
                let origin = index.rect();
                target.paste_rect(tile, origin.x, origin.y);
            }
        }
    }

    /// Rebuild the original contents of `rect` from the captured tiles.
    pub fn extract(&self, rect: IRect) -> Pixmap {
        let mut out = Pixmap::new(rect.width.max(0) as u32, rect.height.max(0) as u32);
        for (index, _) in TileIter::new(rect) {
            if let Some(tile) = self.tiles.get(&(index.tx, index.ty)) {
                let origin = index.rect();
                out.paste_rect(tile, origin.x - rect.x, origin.y - rect.y);
            }
        }
        out
    }

    /// Union of every captured tile.
    pub fn bounds(&self) -> IRect {
        self.bounds
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.tiles.clear();
        self.bounds = IRect::EMPTY;
    }

    /// True when nothing has been captured.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

/// Whether a stroke lays down colour or removes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeMode {
    /// Paint with the primary colour.
    Paint,
    /// Erase.
    Erase,
}

/// The brush and the eraser: same engine, different final compositing step.
pub struct StrokeTool {
    mode: StrokeMode,
    stroke: Option<StrokeState>,
    snapshot: StrokeSnapshot,
    layer: LayerId,
    color: Rgba,
    dirty: IRect,
    seed: u64,
    last_time: f64,
    last_position: Vec2,
}

impl StrokeTool {
    /// A paint or erase tool.
    pub fn new(mode: StrokeMode) -> Self {
        Self {
            mode,
            stroke: None,
            snapshot: StrokeSnapshot::new(),
            layer: LayerId::NONE,
            color: Rgba::BLACK,
            dirty: IRect::EMPTY,
            seed: 1,
            last_time: 0.0,
            last_position: Vec2::ZERO,
        }
    }

    fn begin(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        if let Err(err) = ctx.doc.paint_target() {
            ctx.report(err.to_string());
            return;
        }
        self.layer = ctx.doc.active_layer;
        self.color = if event.button == PointerButton::Secondary {
            ctx.secondary
        } else {
            *ctx.primary
        };
        self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut preset = ctx.brush.clone();
        if self.mode == StrokeMode::Erase {
            // Erasing always removes alpha; a blend mode here would be meaningless.
            preset.blend = BlendMode::Normal;
        }
        self.stroke = Some(StrokeState::begin(
            preset,
            ctx.doc.width,
            ctx.doc.height,
            self.seed,
        ));
        self.snapshot.clear();
        self.dirty = IRect::EMPTY;
        self.last_time = event.sample.time;
        self.last_position = event.sample.position;
        self.push_sample(ctx, event);
    }

    /// Feed one sample and re-composite the affected tiles.
    fn push_sample(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let Some(stroke) = self.stroke.as_mut() else {
            return;
        };
        // Derive velocity so brushes can taper even without a pressure-capable device.
        let dt = (event.sample.time - self.last_time).max(1e-4) as f32;
        let velocity = event.sample.position.distance(self.last_position) / dt;
        self.last_time = event.sample.time;
        self.last_position = event.sample.position;
        let sample = InputSample {
            velocity,
            ..event.sample
        };

        let touched = stroke.push(sample);
        if touched.is_empty() {
            return;
        }
        self.dirty = self.dirty.union(&touched);

        let alpha_lock = ctx
            .doc
            .layers
            .get(self.layer)
            .map(|l| l.alpha_lock)
            .unwrap_or(false);
        let selection = ctx.doc.selection.mask().cloned();
        let opacity = stroke.opacity_for_last_sample();
        let blend = stroke.preset().blend;
        let coverage = stroke.buffer().clone();

        let Ok(target) = ctx.doc.paint_target() else {
            return;
        };
        // Tiles the stroke has newly reached must be saved before being drawn on.
        self.snapshot.capture(target, touched);
        // Re-composite the whole stroke over the untouched original, so
        // overlapping dabs do not build up.
        let region = touched;
        self.snapshot.restore(target, region);

        let mut mask = coverage;
        if let Some(sel) = &selection {
            mask.multiply(sel);
        }
        let opts = CompositeOptions {
            blend,
            opacity,
            offset: (0, 0),
            region: Some(region),
            alpha_lock,
        };
        match self.mode {
            StrokeMode::Paint => {
                fill_masked(target, self.color, &mask, &opts);
            }
            StrokeMode::Erase => {
                erase_masked(target, &mask, opacity, Some(region));
            }
        }
        ctx.doc.mark_dirty(region);
    }

    fn finish(&mut self, ctx: &mut ToolContext) {
        let Some(stroke) = self.stroke.take() else {
            return;
        };
        let dirty = self.dirty.intersect(&ctx.doc.bounds());
        if stroke.is_empty() || dirty.is_empty() {
            self.snapshot.clear();
            return;
        }
        let before = self.snapshot.extract(dirty);
        let after = ctx
            .doc
            .layers
            .get(self.layer)
            .and_then(|l| l.pixmap())
            .map(|p| p.copy_rect(dirty))
            .unwrap_or_else(|| Pixmap::new(dirty.width as u32, dirty.height as u32));
        let label = match self.mode {
            StrokeMode::Paint => "Brush Stroke",
            StrokeMode::Erase => "Erase",
        };
        let edit = RegionEdit::new(label, self.layer, dirty, before, after);
        if !edit.is_noop() {
            ctx.history.push_applied(Box::new(edit));
        }
        self.snapshot.clear();
        self.dirty = IRect::EMPTY;
    }
}

impl Tool for StrokeTool {
    fn id(&self) -> ToolId {
        match self.mode {
            StrokeMode::Paint => ToolId::Brush,
            StrokeMode::Erase => ToolId::Eraser,
        }
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        self.begin(ctx, event);
    }

    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        if self.stroke.is_some() {
            self.push_sample(ctx, event);
        }
    }

    fn pointer_up(&mut self, ctx: &mut ToolContext, _event: &ToolEvent) {
        self.finish(ctx);
    }

    fn cancel(&mut self, ctx: &mut ToolContext) {
        // Roll the layer back to the pre-stroke pixels.
        if self.stroke.take().is_some() {
            let region = self.dirty;
            if let Ok(target) = ctx.doc.paint_target() {
                self.snapshot.restore(target, region);
            }
            ctx.doc.mark_dirty(region);
        }
        self.snapshot.clear();
        self.dirty = IRect::EMPTY;
    }

    fn is_active(&self) -> bool {
        self.stroke.is_some()
    }
}

/// Flood fill.
pub struct BucketTool;

impl Tool for BucketTool {
    fn id(&self) -> ToolId {
        ToolId::Bucket
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let point = event.sample.position;
        let (x, y) = (point.x.floor() as i32, point.y.floor() as i32);
        if !ctx.doc.bounds().contains(x, y) {
            return;
        }
        if let Err(err) = ctx.doc.paint_target() {
            ctx.report(err.to_string());
            return;
        }
        let layer = ctx.doc.active_layer;
        let source = if ctx.sample_all_layers {
            ctx.composite.clone()
        } else {
            ctx.doc
                .layers
                .get(layer)
                .and_then(|l| l.pixmap())
                .cloned()
                .unwrap_or_else(|| Pixmap::new(ctx.doc.width, ctx.doc.height))
        };
        let mut mask = fill::flood_fill_mask(&source, x, y, ctx.tolerance);
        if let Some(sel) = ctx.doc.selection.mask() {
            mask.multiply(sel);
        }
        let region = mask.coverage_bounds();
        if region.is_empty() {
            ctx.report("Nothing to fill here");
            return;
        }
        let color = if event.button == PointerButton::Secondary {
            ctx.secondary
        } else {
            *ctx.primary
        };
        let alpha_lock = ctx.doc.layers.get(layer).map(|l| l.alpha_lock).unwrap_or(false);
        let blend = ctx.brush.blend;

        let edit = RegionEdit::capture(ctx.doc, layer, region, "Bucket Fill", |pixmap| {
            let opts = CompositeOptions {
                blend,
                opacity: 1.0,
                offset: (0, 0),
                region: Some(region),
                alpha_lock,
            };
            fill_masked(pixmap, color, &mask, &opts);
            Ok(())
        });
        match edit {
            Ok(edit) if !edit.is_noop() => ctx.history.push_applied(Box::new(edit)),
            Ok(_) => {}
            Err(err) => ctx.report(err.to_string()),
        }
    }

    fn pointer_move(&mut self, _ctx: &mut ToolContext, _event: &ToolEvent) {}

    fn pointer_up(&mut self, _ctx: &mut ToolContext, _event: &ToolEvent) {}
}

/// Pick a colour from the canvas.
pub struct EyedropperTool;

impl EyedropperTool {
    fn sample(&self, ctx: &mut ToolContext, event: &ToolEvent) {
        let p = event.sample.position;
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        let color = if ctx.sample_all_layers {
            ctx.composite.get(x, y)
        } else {
            ctx.doc
                .layers
                .get(ctx.doc.active_layer)
                .and_then(|l| l.pixmap())
                .map(|pm| pm.get(x, y))
                .unwrap_or(Rgba8::TRANSPARENT)
        };
        if color.a == 0 {
            return;
        }
        *ctx.primary = color.to_rgba().with_alpha(1.0);
        ctx.report(format!("Picked {}", color.to_hex()));
    }
}

impl Tool for EyedropperTool {
    fn id(&self) -> ToolId {
        ToolId::Eyedropper
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        self.sample(ctx, event);
    }

    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        self.sample(ctx, event);
    }

    fn pointer_up(&mut self, _ctx: &mut ToolContext, _event: &ToolEvent) {}
}

/// The shape a marquee tool drags out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarqueeShape {
    /// Rectangular.
    Rect,
    /// Elliptical.
    Ellipse,
}

/// Rectangular and elliptical selection.
pub struct MarqueeTool {
    shape: MarqueeShape,
    start: Option<Vec2>,
    current: Vec2,
}

impl MarqueeTool {
    /// A marquee tool of the given shape.
    pub fn new(shape: MarqueeShape) -> Self {
        Self {
            shape,
            start: None,
            current: Vec2::ZERO,
        }
    }

    fn rect(&self) -> IRect {
        let Some(start) = self.start else {
            return IRect::EMPTY;
        };
        let min = start.min(self.current);
        let max = start.max(self.current);
        IRect::from_bounds(
            min.x.round() as i32,
            min.y.round() as i32,
            max.x.round() as i32,
            max.y.round() as i32,
        )
    }
}

impl Tool for MarqueeTool {
    fn id(&self) -> ToolId {
        match self.shape {
            MarqueeShape::Rect => ToolId::RectSelect,
            MarqueeShape::Ellipse => ToolId::EllipseSelect,
        }
    }

    fn pointer_down(&mut self, _ctx: &mut ToolContext, event: &ToolEvent) {
        self.start = Some(event.sample.position);
        self.current = event.sample.position;
    }

    fn pointer_move(&mut self, _ctx: &mut ToolContext, event: &ToolEvent) {
        if self.start.is_some() {
            self.current = event.sample.position;
        }
    }

    fn pointer_up(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        if self.start.is_none() {
            return;
        }
        self.current = event.sample.position;
        let rect = self.rect();
        self.start = None;
        if rect.is_empty() {
            // A click with no drag clears the selection, like every other editor.
            if ctx.doc.selection.is_active() {
                let command = SetSelectionCommand::new("Deselect", Selection::none());
                let _ = ctx.history.execute(ctx.doc, Box::new(command));
            }
            return;
        }
        let mut selection = ctx.doc.selection.clone();
        let (w, h) = (ctx.doc.width, ctx.doc.height);
        match self.shape {
            MarqueeShape::Rect => selection.select_rect(w, h, rect, ctx.selection_mode),
            MarqueeShape::Ellipse => selection.select_ellipse(w, h, rect, ctx.selection_mode),
        }
        let command = SetSelectionCommand::new("Select", selection);
        if let Err(err) = ctx.history.execute(ctx.doc, Box::new(command)) {
            ctx.report(err.to_string());
        }
    }

    fn cancel(&mut self, _ctx: &mut ToolContext) {
        self.start = None;
    }

    fn preview(&self) -> Option<ToolPreview> {
        let rect = self.rect();
        if self.start.is_none() || rect.is_empty() {
            return None;
        }
        Some(match self.shape {
            MarqueeShape::Rect => ToolPreview::Rect(rect),
            MarqueeShape::Ellipse => ToolPreview::Ellipse(rect),
        })
    }

    fn is_active(&self) -> bool {
        self.start.is_some()
    }
}

/// Freehand lasso selection.
pub struct LassoTool {
    points: Vec<Vec2>,
}

impl LassoTool {
    /// A lasso with no points yet.
    pub fn new() -> Self {
        Self { points: Vec::new() }
    }
}

impl Default for LassoTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for LassoTool {
    fn id(&self) -> ToolId {
        ToolId::Lasso
    }

    fn pointer_down(&mut self, _ctx: &mut ToolContext, event: &ToolEvent) {
        self.points.clear();
        self.points.push(event.sample.position);
    }

    fn pointer_move(&mut self, _ctx: &mut ToolContext, event: &ToolEvent) {
        if self.points.is_empty() {
            return;
        }
        // Drop samples closer than a pixel; they add cost and no shape.
        if self
            .points
            .last()
            .map(|p| p.distance(event.sample.position) > 1.0)
            .unwrap_or(true)
        {
            self.points.push(event.sample.position);
        }
    }

    fn pointer_up(&mut self, ctx: &mut ToolContext, _event: &ToolEvent) {
        if self.points.len() < 3 {
            self.points.clear();
            return;
        }
        let mut selection = ctx.doc.selection.clone();
        selection.select_polygon(ctx.doc.width, ctx.doc.height, &self.points, ctx.selection_mode);
        self.points.clear();
        let command = SetSelectionCommand::new("Lasso Select", selection);
        if let Err(err) = ctx.history.execute(ctx.doc, Box::new(command)) {
            ctx.report(err.to_string());
        }
    }

    fn cancel(&mut self, _ctx: &mut ToolContext) {
        self.points.clear();
    }

    fn preview(&self) -> Option<ToolPreview> {
        if self.points.len() < 2 {
            return None;
        }
        Some(ToolPreview::Polyline(self.points.clone()))
    }

    fn is_active(&self) -> bool {
        !self.points.is_empty()
    }
}

/// Select by colour similarity.
pub struct MagicWandTool;

impl Tool for MagicWandTool {
    fn id(&self) -> ToolId {
        ToolId::MagicWand
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let p = event.sample.position;
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        if !ctx.doc.bounds().contains(x, y) {
            return;
        }
        let source = if ctx.sample_all_layers {
            ctx.composite.clone()
        } else {
            ctx.doc
                .layers
                .get(ctx.doc.active_layer)
                .and_then(|l| l.pixmap())
                .cloned()
                .unwrap_or_else(|| Pixmap::new(ctx.doc.width, ctx.doc.height))
        };
        let mask = fill::flood_fill_mask(&source, x, y, ctx.tolerance);
        if mask.is_empty() {
            return;
        }
        let mut selection = ctx.doc.selection.clone();
        selection.combine(mask, ctx.selection_mode);
        let command = SetSelectionCommand::new("Magic Wand", selection);
        if let Err(err) = ctx.history.execute(ctx.doc, Box::new(command)) {
            ctx.report(err.to_string());
        }
    }

    fn pointer_move(&mut self, _ctx: &mut ToolContext, _event: &ToolEvent) {}

    fn pointer_up(&mut self, _ctx: &mut ToolContext, _event: &ToolEvent) {}
}

/// Move the active layer's pixels.
pub struct MoveTool {
    origin: Option<Vec2>,
    source: Option<Pixmap>,
    region: IRect,
    layer: LayerId,
    offset: (i32, i32),
}

impl MoveTool {
    /// A move tool with no gesture in progress.
    pub fn new() -> Self {
        Self {
            origin: None,
            source: None,
            region: IRect::EMPTY,
            layer: LayerId::NONE,
            offset: (0, 0),
        }
    }

    fn apply_offset(&mut self, ctx: &mut ToolContext, dx: i32, dy: i32) {
        let (Some(source), false) = (self.source.as_ref(), self.region.is_empty()) else {
            return;
        };
        self.offset = (dx, dy);
        let region = self.region;
        let Ok(target) = ctx.doc.paint_target() else {
            return;
        };
        // Clear the whole affected area, then stamp the content at its new place.
        target.fill_rect(region, Rgba8::TRANSPARENT);
        target.paste_rect(source, region.x + dx, region.y + dy);
        ctx.doc.mark_dirty(region);
    }
}

impl Default for MoveTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for MoveTool {
    fn id(&self) -> ToolId {
        ToolId::Move
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        if let Err(err) = ctx.doc.paint_target() {
            ctx.report(err.to_string());
            return;
        }
        self.layer = ctx.doc.active_layer;
        let content = ctx
            .doc
            .layers
            .get(self.layer)
            .and_then(|l| l.pixmap())
            .map(|p| p.opaque_bounds())
            .unwrap_or(IRect::EMPTY);
        if content.is_empty() {
            ctx.report("Layer is empty");
            return;
        }
        // The affected region is where the content is now plus anywhere it can
        // be dragged to, clipped to the canvas.
        self.region = ctx.doc.bounds();
        self.source = ctx
            .doc
            .layers
            .get(self.layer)
            .and_then(|l| l.pixmap())
            .map(|p| p.copy_rect(self.region));
        self.origin = Some(event.sample.position);
        self.offset = (0, 0);
    }

    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let Some(origin) = self.origin else {
            return;
        };
        let delta = event.sample.position - origin;
        self.apply_offset(ctx, delta.x.round() as i32, delta.y.round() as i32);
    }

    fn pointer_up(&mut self, ctx: &mut ToolContext, _event: &ToolEvent) {
        let (Some(source), Some(_)) = (self.source.take(), self.origin.take()) else {
            return;
        };
        let region = self.region;
        if self.offset == (0, 0) || region.is_empty() {
            return;
        }
        let after = ctx
            .doc
            .layers
            .get(self.layer)
            .and_then(|l| l.pixmap())
            .map(|p| p.copy_rect(region))
            .unwrap_or_else(|| Pixmap::new(region.width as u32, region.height as u32));
        let edit = RegionEdit::new("Move Layer", self.layer, region, source, after);
        if !edit.is_noop() {
            ctx.history.push_applied(Box::new(edit));
        }
        self.region = IRect::EMPTY;
    }

    fn cancel(&mut self, ctx: &mut ToolContext) {
        if let (Some(source), true) = (self.source.take(), !self.region.is_empty()) {
            let region = self.region;
            if let Ok(target) = ctx.doc.paint_target() {
                target.paste_rect(&source, region.x, region.y);
            }
            ctx.doc.mark_dirty(region);
        }
        self.origin = None;
        self.region = IRect::EMPTY;
    }

    fn is_active(&self) -> bool {
        self.origin.is_some()
    }
}

/// Drag the canvas.
pub struct PanTool {
    active: bool,
}

impl PanTool {
    /// A pan tool with no gesture in progress.
    pub fn new() -> Self {
        Self { active: false }
    }
}

impl Default for PanTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for PanTool {
    fn id(&self) -> ToolId {
        ToolId::Pan
    }

    fn pointer_down(&mut self, _ctx: &mut ToolContext, _event: &ToolEvent) {
        self.active = true;
    }

    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        if self.active {
            ctx.viewport.pan_by_screen(event.screen_delta);
        }
    }

    fn pointer_up(&mut self, _ctx: &mut ToolContext, _event: &ToolEvent) {
        self.active = false;
    }

    fn is_active(&self) -> bool {
        self.active
    }
}

/// The set of available tools and which one is selected.
///
/// Tools are boxed so a plugin can add one at runtime without this type
/// knowing every possible tool at compile time.
pub struct ToolBox {
    tools: Vec<Box<dyn Tool>>,
    active: usize,
}

impl Default for ToolBox {
    fn default() -> Self {
        Self::standard()
    }
}

impl ToolBox {
    /// The built-in tools, brush first.
    pub fn standard() -> Self {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(StrokeTool::new(StrokeMode::Paint)),
            Box::new(StrokeTool::new(StrokeMode::Erase)),
            Box::new(BucketTool),
            Box::new(EyedropperTool),
            Box::new(MarqueeTool::new(MarqueeShape::Rect)),
            Box::new(MarqueeTool::new(MarqueeShape::Ellipse)),
            Box::new(LassoTool::new()),
            Box::new(MagicWandTool),
            Box::new(MoveTool::new()),
            Box::new(PanTool::new()),
        ];
        Self { tools, active: 0 }
    }

    /// Add a tool (used by plugins).
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// Ids of every tool, in toolbar order.
    pub fn ids(&self) -> Vec<ToolId> {
        self.tools.iter().map(|t| t.id()).collect()
    }

    /// The selected tool's id.
    pub fn active_id(&self) -> ToolId {
        self.tools
            .get(self.active)
            .map(|t| t.id())
            .unwrap_or(ToolId::Brush)
    }

    /// Select a tool by id; returns false when no such tool exists.
    pub fn select(&mut self, id: ToolId) -> bool {
        match self.tools.iter().position(|t| t.id() == id) {
            Some(index) => {
                self.active = index;
                true
            }
            None => false,
        }
    }

    /// The selected tool.
    pub fn active_mut(&mut self) -> &mut dyn Tool {
        // `standard()` always creates at least one tool, and `select` only ever
        // stores a valid index, so this cannot be out of range.
        let index = self.active.min(self.tools.len().saturating_sub(1));
        self.tools[index].as_mut()
    }

    /// The selected tool, immutably.
    pub fn active(&self) -> &dyn Tool {
        let index = self.active.min(self.tools.len().saturating_sub(1));
        self.tools[index].as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_raster::BrushPreset;

    struct Harness {
        doc: Document,
        history: History,
        viewport: Viewport,
        brush: BrushPreset,
        primary: Rgba,
        composite: Pixmap,
    }

    impl Harness {
        fn new() -> Self {
            let doc = Document::new(64, 64, "test");
            Self {
                doc,
                history: History::default(),
                viewport: Viewport::default(),
                brush: BrushPreset {
                    size: 8.0,
                    hardness: 1.0,
                    spacing: 0.1,
                    smoothing: 0.0,
                    ..Default::default()
                },
                primary: Rgba::rgb(1.0, 0.0, 0.0),
                composite: Pixmap::new(64, 64),
            }
        }

        fn ctx(&mut self) -> ToolContext<'_> {
            ToolContext {
                doc: &mut self.doc,
                history: &mut self.history,
                viewport: &mut self.viewport,
                brush: &self.brush,
                primary: &mut self.primary,
                secondary: Rgba::WHITE,
                tolerance: 0.1,
                sample_all_layers: false,
                selection_mode: SelectionMode::Replace,
                composite: &self.composite,
                status: None,
            }
        }

        fn pixel(&self, x: i32, y: i32) -> Rgba8 {
            self.doc
                .layers
                .get(self.doc.active_layer)
                .and_then(|l| l.pixmap())
                .map(|p| p.get(x, y))
                .unwrap_or(Rgba8::TRANSPARENT)
        }
    }

    fn event(x: f32, y: f32, t: f64) -> ToolEvent {
        ToolEvent {
            sample: InputSample::at(Vec2::new(x, y)).with_time(t),
            screen: Vec2::new(x, y),
            screen_delta: Vec2::ZERO,
            button: PointerButton::Primary,
            modifiers: Modifiers::NONE,
        }
    }

    fn drag(tool: &mut dyn Tool, h: &mut Harness, points: &[(f32, f32)]) {
        let mut t = 0.0;
        for (i, (x, y)) in points.iter().enumerate() {
            t += 0.016;
            let ev = event(*x, *y, t);
            let mut ctx = h.ctx();
            if i == 0 {
                tool.pointer_down(&mut ctx, &ev);
            } else {
                tool.pointer_move(&mut ctx, &ev);
            }
        }
        let last = points.last().copied().unwrap_or((0.0, 0.0));
        let ev = event(last.0, last.1, t + 0.016);
        let mut ctx = h.ctx();
        tool.pointer_up(&mut ctx, &ev);
    }

    #[test]
    fn a_brush_stroke_paints_and_is_undoable() {
        let mut h = Harness::new();
        let mut tool = StrokeTool::new(StrokeMode::Paint);
        drag(&mut tool, &mut h, &[(10.0, 32.0), (20.0, 32.0), (40.0, 32.0)]);

        assert_eq!(h.pixel(20, 32), Rgba8::new(255, 0, 0, 255));
        assert!(h.history.can_undo(), "the stroke must be in the history");
        h.history.undo(&mut h.doc).expect("undo");
        assert_eq!(h.pixel(20, 32), Rgba8::TRANSPARENT);
        h.history.redo(&mut h.doc).expect("redo");
        assert_eq!(h.pixel(20, 32), Rgba8::new(255, 0, 0, 255));
    }

    #[test]
    fn a_stroke_is_one_history_entry() {
        let mut h = Harness::new();
        let mut tool = StrokeTool::new(StrokeMode::Paint);
        drag(
            &mut tool,
            &mut h,
            &[(5.0, 5.0), (15.0, 15.0), (25.0, 25.0), (35.0, 35.0)],
        );
        assert_eq!(h.history.depth(), 1);
    }

    #[test]
    fn overlapping_dabs_do_not_darken_at_partial_opacity() {
        let mut h = Harness::new();
        h.brush.opacity = 0.5;
        h.brush.flow = 1.0;
        let mut tool = StrokeTool::new(StrokeMode::Paint);
        // A stroke that doubles back over itself.
        drag(&mut tool, &mut h, &[(10.0, 32.0), (40.0, 32.0), (10.0, 32.0)]);
        let alpha = h.pixel(25, 32).a as i32;
        assert!(
            (alpha - 128).abs() <= 4,
            "self-overlap darkened the stroke: alpha={alpha}"
        );
    }

    #[test]
    fn a_cancelled_stroke_leaves_no_trace() {
        let mut h = Harness::new();
        let mut tool = StrokeTool::new(StrokeMode::Paint);
        {
            let ev = event(10.0, 10.0, 0.016);
            let mut ctx = h.ctx();
            tool.pointer_down(&mut ctx, &ev);
        }
        {
            let ev = event(30.0, 10.0, 0.032);
            let mut ctx = h.ctx();
            tool.pointer_move(&mut ctx, &ev);
        }
        assert_ne!(h.pixel(20, 10), Rgba8::TRANSPARENT);
        {
            let mut ctx = h.ctx();
            tool.cancel(&mut ctx);
        }
        assert_eq!(h.pixel(20, 10), Rgba8::TRANSPARENT);
        assert!(!h.history.can_undo());
    }

    #[test]
    fn painting_is_confined_to_the_selection() {
        let mut h = Harness::new();
        h.doc
            .selection
            .select_rect(64, 64, IRect::new(0, 0, 32, 64), SelectionMode::Replace);
        let mut tool = StrokeTool::new(StrokeMode::Paint);
        drag(&mut tool, &mut h, &[(5.0, 32.0), (60.0, 32.0)]);
        assert_ne!(h.pixel(10, 32), Rgba8::TRANSPARENT, "inside the selection");
        assert_eq!(h.pixel(50, 32), Rgba8::TRANSPARENT, "outside the selection");
    }

    #[test]
    fn the_eraser_removes_paint() {
        let mut h = Harness::new();
        if let Some(pm) = h
            .doc
            .layers
            .get_mut(h.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::new(0, 0, 255, 255));
        }
        let mut tool = StrokeTool::new(StrokeMode::Erase);
        drag(&mut tool, &mut h, &[(10.0, 32.0), (40.0, 32.0)]);
        assert_eq!(h.pixel(25, 32).a, 0);
        assert_eq!(h.pixel(25, 5).a, 255, "outside the stroke is untouched");
        h.history.undo(&mut h.doc).expect("undo");
        assert_eq!(h.pixel(25, 32).a, 255);
    }

    #[test]
    fn locked_layers_refuse_strokes_with_an_explanation() {
        let mut h = Harness::new();
        if let Some(layer) = h.doc.active_mut() {
            layer.locked = true;
        }
        let mut tool = StrokeTool::new(StrokeMode::Paint);
        let ev = event(10.0, 10.0, 0.016);
        let mut ctx = h.ctx();
        tool.pointer_down(&mut ctx, &ev);
        let status = ctx.status.clone();
        assert!(status.unwrap_or_default().contains("locked"));
        assert!(!h.history.can_undo());
    }

    #[test]
    fn alpha_locked_layers_only_repaint_existing_pixels() {
        let mut h = Harness::new();
        if let Some(pm) = h
            .doc
            .layers
            .get_mut(h.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill_rect(IRect::new(0, 0, 32, 64), Rgba8::new(0, 0, 255, 255));
        }
        if let Some(layer) = h.doc.active_mut() {
            layer.alpha_lock = true;
        }
        let mut tool = StrokeTool::new(StrokeMode::Paint);
        drag(&mut tool, &mut h, &[(5.0, 32.0), (60.0, 32.0)]);
        assert_eq!(
            h.pixel(10, 32),
            Rgba8::new(255, 0, 0, 255),
            "existing pixels repaint"
        );
        assert_eq!(h.pixel(50, 32), Rgba8::TRANSPARENT, "empty pixels stay empty");
    }

    #[test]
    fn the_bucket_fills_a_bounded_region() {
        let mut h = Harness::new();
        if let Some(pm) = h
            .doc
            .layers
            .get_mut(h.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::WHITE);
            pm.fill_rect(IRect::new(32, 0, 1, 64), Rgba8::BLACK);
        }
        let mut tool = BucketTool;
        let ev = event(10.0, 10.0, 0.016);
        let mut ctx = h.ctx();
        tool.pointer_down(&mut ctx, &ev);

        assert_eq!(h.pixel(5, 5), Rgba8::new(255, 0, 0, 255));
        assert_eq!(
            h.pixel(40, 5),
            Rgba8::WHITE,
            "the fill must not cross the divider"
        );
        assert!(h.history.can_undo());
    }

    #[test]
    fn the_eyedropper_updates_the_primary_colour() {
        let mut h = Harness::new();
        if let Some(pm) = h
            .doc
            .layers
            .get_mut(h.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::new(10, 200, 30, 255));
        }
        let mut tool = EyedropperTool;
        let ev = event(5.0, 5.0, 0.016);
        let mut ctx = h.ctx();
        tool.pointer_down(&mut ctx, &ev);
        assert_eq!(h.primary.to_rgba8(), Rgba8::new(10, 200, 30, 255));
    }

    #[test]
    fn the_marquee_creates_an_undoable_selection() {
        let mut h = Harness::new();
        let mut tool = MarqueeTool::new(MarqueeShape::Rect);
        drag(&mut tool, &mut h, &[(8.0, 8.0), (24.0, 24.0)]);
        assert_eq!(h.doc.selection.bounds(), Some(IRect::new(8, 8, 16, 16)));
        h.history.undo(&mut h.doc).expect("undo");
        assert!(!h.doc.selection.is_active());
    }

    #[test]
    fn clicking_with_the_marquee_clears_the_selection() {
        let mut h = Harness::new();
        h.doc.selection.select_all(64, 64);
        let mut tool = MarqueeTool::new(MarqueeShape::Rect);
        drag(&mut tool, &mut h, &[(8.0, 8.0)]);
        assert!(!h.doc.selection.is_active());
    }

    #[test]
    fn the_lasso_needs_at_least_three_points() {
        let mut h = Harness::new();
        let mut tool = LassoTool::new();
        drag(&mut tool, &mut h, &[(8.0, 8.0), (9.0, 9.0)]);
        assert!(!h.doc.selection.is_active());

        let mut tool = LassoTool::new();
        drag(
            &mut tool,
            &mut h,
            &[(8.0, 8.0), (40.0, 8.0), (40.0, 40.0), (8.0, 40.0)],
        );
        assert!(h.doc.selection.is_active());
    }

    #[test]
    fn the_move_tool_shifts_pixels_and_is_undoable() {
        let mut h = Harness::new();
        if let Some(pm) = h
            .doc
            .layers
            .get_mut(h.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill_rect(IRect::new(4, 4, 8, 8), Rgba8::WHITE);
        }
        let mut tool = MoveTool::new();
        drag(&mut tool, &mut h, &[(6.0, 6.0), (16.0, 16.0)]);
        assert_eq!(h.pixel(5, 5), Rgba8::TRANSPARENT, "content left its old place");
        assert_eq!(h.pixel(15, 15), Rgba8::WHITE, "content arrived at the new place");
        h.history.undo(&mut h.doc).expect("undo");
        assert_eq!(h.pixel(5, 5), Rgba8::WHITE);
    }

    #[test]
    fn panning_moves_the_viewport_not_the_document() {
        let mut h = Harness::new();
        let before = h.viewport.center;
        let mut tool = PanTool::new();
        {
            let ev = event(10.0, 10.0, 0.016);
            let mut ctx = h.ctx();
            tool.pointer_down(&mut ctx, &ev);
        }
        {
            let mut ev = event(30.0, 10.0, 0.032);
            ev.screen_delta = Vec2::new(20.0, 0.0);
            let mut ctx = h.ctx();
            tool.pointer_move(&mut ctx, &ev);
        }
        assert_ne!(h.viewport.center, before);
        assert!(!h.history.can_undo(), "panning is not an edit");
    }

    #[test]
    fn the_toolbox_selects_by_id() {
        let mut tools = ToolBox::standard();
        assert_eq!(tools.active_id(), ToolId::Brush);
        assert!(tools.select(ToolId::Eraser));
        assert_eq!(tools.active_id(), ToolId::Eraser);
        assert!(!tools.select(ToolId::Custom("nope")));
        assert_eq!(tools.active_id(), ToolId::Eraser);
    }

    #[test]
    fn snapshots_capture_each_tile_once() {
        let mut source = Pixmap::filled(600, 600, Rgba8::WHITE);
        let mut snapshot = StrokeSnapshot::new();
        snapshot.capture(&source, IRect::new(10, 10, 4, 4));
        let bounds = snapshot.bounds();
        snapshot.capture(&source, IRect::new(12, 12, 4, 4));
        assert_eq!(
            snapshot.bounds(),
            bounds,
            "the same tile must not be captured twice"
        );

        source.fill(Rgba8::BLACK);
        snapshot.restore(&mut source, IRect::new(0, 0, 256, 256));
        assert_eq!(
            source.get(5, 5),
            Rgba8::WHITE,
            "restore brings back the original pixels"
        );
        assert_eq!(
            source.get(300, 300),
            Rgba8::BLACK,
            "untouched tiles stay as they are"
        );
    }
}
