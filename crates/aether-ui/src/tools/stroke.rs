//! The brush and the eraser.

use super::{StrokeSnapshot, Tool, ToolContext, ToolEvent, ToolId};
use aether_core::blend::BlendMode;
use aether_core::color::Rgba;
use aether_core::input::{InputSample, PointerButton};
use aether_core::math::{IRect, Vec2};
use aether_document::command::RegionEdit;
use aether_document::LayerId;
use aether_raster::composite::{erase_masked, fill_masked, CompositeOptions};
use aether_raster::{Pixmap, StrokeState};

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
