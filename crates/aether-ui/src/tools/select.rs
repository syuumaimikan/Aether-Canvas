//! Selection tools: marquee, lasso and magic wand.

use super::{Tool, ToolContext, ToolEvent, ToolId, ToolPreview};
use aether_core::math::{IRect, Vec2};
use aether_document::command::SetSelectionCommand;
use aether_document::selection::Selection;
use aether_raster::{fill, Pixmap};

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
