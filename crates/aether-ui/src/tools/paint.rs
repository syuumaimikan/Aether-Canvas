//! Tools that act on a single click: flood fill and colour picking.

use super::{Tool, ToolContext, ToolEvent, ToolId};
use aether_core::color::Rgba8;
use aether_core::input::PointerButton;
use aether_document::command::RegionEdit;
use aether_raster::composite::{fill_masked, CompositeOptions};
use aether_raster::{fill, Pixmap};

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
