//! Moving content and moving the view.

use super::{Tool, ToolContext, ToolEvent, ToolId};
use aether_core::color::Rgba8;
use aether_core::math::{IRect, Vec2};
use aether_document::command::RegionEdit;
use aether_document::LayerId;
use aether_raster::Pixmap;

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
