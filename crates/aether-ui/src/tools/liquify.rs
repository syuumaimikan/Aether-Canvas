//! The liquify brush.
//!
//! Liquify does not paint: it accumulates a [`DisplacementField`] and resamples
//! the layer through it. Keeping the deformation as a field rather than as
//! pixels is what lets a stroke be pushed further, twirled, and then relaxed
//! back out — each gesture edits the field, and the pixels are always rebuilt
//! from the untouched original.
//!
//! Like the transform tool this is **modal**: the field stays live until the
//! artist confirms with Enter, so a whole liquify session is one undo step
//! rather than one per dab.

use super::{Tool, ToolContext, ToolEvent, ToolId, ToolPreview, ToolSettings};
use aether_core::math::{IRect, Vec2};
use aether_document::command::RegionEdit;
use aether_document::LayerId;
use aether_raster::transform::Interpolation;
use aether_raster::warp::DEFAULT_CELL;
use aether_raster::{DisplacementField, Pixmap};

/// What a liquify drag does to the field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LiquifyMode {
    /// Drag pixels along with the cursor.
    #[default]
    Push,
    /// Rotate pixels around the cursor.
    Twirl,
    /// Pull pixels towards the cursor.
    Pinch,
    /// Push pixels away from the cursor.
    Bloat,
    /// Fade the deformation back out.
    Restore,
}

impl LiquifyMode {
    /// All modes, in toolbar order.
    pub const ALL: [LiquifyMode; 5] = [
        LiquifyMode::Push,
        LiquifyMode::Twirl,
        LiquifyMode::Pinch,
        LiquifyMode::Bloat,
        LiquifyMode::Restore,
    ];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            LiquifyMode::Push => "Push",
            LiquifyMode::Twirl => "Twirl",
            LiquifyMode::Pinch => "Pinch",
            LiquifyMode::Bloat => "Bloat",
            LiquifyMode::Restore => "Restore",
        }
    }
}

/// Push, twirl, pinch and bloat pixels with a brush.
pub struct LiquifyTool {
    /// What a drag does.
    pub mode: LiquifyMode,
    /// Brush radius in document pixels.
    pub radius: f32,
    /// Strength of each application, `0..=1`.
    pub strength: f32,
    original: Option<Pixmap>,
    field: Option<DisplacementField>,
    layer: LayerId,
    last: Vec2,
    dragging: bool,
}

impl Default for LiquifyTool {
    fn default() -> Self {
        Self::new()
    }
}

impl LiquifyTool {
    /// A tool with no session in progress.
    pub fn new() -> Self {
        Self {
            mode: LiquifyMode::Push,
            radius: 60.0,
            strength: 0.5,
            original: None,
            field: None,
            layer: LayerId::NONE,
            last: Vec2::ZERO,
            dragging: false,
        }
    }

    /// True when there is an uncommitted deformation.
    pub fn is_active_session(&self) -> bool {
        self.original.is_some()
    }

    fn begin(&mut self, ctx: &mut ToolContext) -> bool {
        if let Err(err) = ctx.doc.paint_target() {
            ctx.report(err.to_string());
            return false;
        }
        self.layer = ctx.doc.active_layer;
        self.original = ctx.doc.layers.get(self.layer).and_then(|l| l.pixmap()).cloned();
        if self.original.is_none() {
            return false;
        }
        self.field = Some(DisplacementField::with_cell_size(ctx.doc.bounds(), DEFAULT_CELL));
        true
    }

    /// Apply one gesture step and redraw the neighbourhood.
    fn apply_step(&mut self, ctx: &mut ToolContext, position: Vec2, delta: Vec2) {
        let (Some(field), Some(original)) = (self.field.as_mut(), self.original.as_ref()) else {
            return;
        };
        let radius = self.radius.max(2.0);
        let strength = self.strength.clamp(0.0, 1.0);
        match self.mode {
            LiquifyMode::Push => field.push(position, radius, delta, strength),
            LiquifyMode::Twirl => {
                // Drag length sets the turn, so a slow circle twists gently.
                let angle = (delta.length() / radius) * strength * std::f32::consts::PI;
                let sign = if delta.x + delta.y >= 0.0 { 1.0 } else { -1.0 };
                field.twirl(position, radius, angle * sign, 1.0)
            }
            LiquifyMode::Pinch => field.pinch(position, radius, 0.08 * strength, 1.0),
            LiquifyMode::Bloat => field.pinch(position, radius, -0.08 * strength, 1.0),
            LiquifyMode::Restore => field.relax(position, radius, strength),
        }

        // Only the brush neighbourhood can have changed; rebuild it from the
        // original so repeated passes never compound resampling blur.
        let region = IRect::from_bounds(
            (position.x - radius * 1.5).floor() as i32,
            (position.y - radius * 1.5).floor() as i32,
            (position.x + radius * 1.5).ceil() as i32,
            (position.y + radius * 1.5).ceil() as i32,
        )
        .intersect(&ctx.doc.bounds());
        if region.is_empty() {
            return;
        }
        let patch = field
            .apply_region(original, Interpolation::Bilinear, Some(region))
            .copy_rect(region);
        if let Ok(target) = ctx.doc.paint_target() {
            target.paste_rect(&patch, region.x, region.y);
        }
        ctx.doc.mark_dirty(region);
    }

    fn restore(&mut self, ctx: &mut ToolContext) {
        if let Some(original) = self.original.take() {
            if let Ok(target) = ctx.doc.paint_target() {
                target.paste_rect(&original, 0, 0);
            }
            let region = ctx.doc.bounds();
            ctx.doc.mark_dirty(region);
        }
        self.field = None;
        self.dragging = false;
    }
}

impl Tool for LiquifyTool {
    fn id(&self) -> ToolId {
        ToolId::Liquify
    }

    fn sync_settings(&mut self, settings: &ToolSettings) {
        self.mode = settings.liquify_mode;
        self.radius = settings.liquify_radius.max(2.0);
        self.strength = settings.liquify_strength.clamp(0.0, 1.0);
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        if self.original.is_none() && !self.begin(ctx) {
            return;
        }
        self.dragging = true;
        self.last = event.sample.position;
        // Pinch, bloat and restore act on a held cursor, so they need a first
        // application without any movement.
        if !matches!(self.mode, LiquifyMode::Push | LiquifyMode::Twirl) {
            self.apply_step(ctx, event.sample.position, Vec2::ZERO);
        }
    }

    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        if !self.dragging {
            return;
        }
        let position = event.sample.position;
        let delta = position - self.last;
        self.last = position;
        if delta.length_squared() < 1e-4 && matches!(self.mode, LiquifyMode::Push | LiquifyMode::Twirl) {
            return;
        }
        self.apply_step(ctx, position, delta);
    }

    fn pointer_up(&mut self, _ctx: &mut ToolContext, _event: &ToolEvent) {
        self.dragging = false;
    }

    fn commit(&mut self, ctx: &mut ToolContext) {
        let Some(original) = self.original.take() else {
            return;
        };
        let region = self
            .field
            .as_ref()
            .map(|f| f.affected_bounds().intersect(&ctx.doc.bounds()))
            .unwrap_or_else(|| ctx.doc.bounds());
        self.field = None;
        if region.is_empty() {
            return;
        }
        let after = ctx
            .doc
            .layers
            .get(self.layer)
            .and_then(|l| l.pixmap())
            .map(|p| p.copy_rect(region))
            .unwrap_or_else(|| Pixmap::new(region.width as u32, region.height as u32));
        let edit = RegionEdit::new("Liquify", self.layer, region, original.copy_rect(region), after);
        if !edit.is_noop() {
            ctx.history.push_applied(Box::new(edit));
        }
    }

    fn cancel(&mut self, ctx: &mut ToolContext) {
        self.restore(ctx);
    }

    fn preview(&self) -> Option<ToolPreview> {
        None
    }

    fn is_active(&self) -> bool {
        self.dragging
    }

    fn is_pending(&self) -> bool {
        self.original.is_some()
    }
}
