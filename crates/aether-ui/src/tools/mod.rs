//! Editing tools.
//!
//! A tool turns pointer input into commands. Tools never touch pixels directly:
//! they build a [`Command`](aether_document::Command) and hand it to the
//! history, so everything they do is undoable and reachable from tests.
//!
//! Tools live in submodules: [`stroke`] (brush and eraser), [`paint`]
//! (bucket and eyedropper), [`select`] (marquee, lasso, wand), [`placement`]
//! (move and pan), [`transform`], [`liquify`] and [`rig`] (mesh, deform and
//! bone).
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

pub mod liquify;
pub mod paint;
pub mod placement;
pub mod rig;
pub mod select;
pub mod stroke;
pub mod transform;

pub use liquify::{LiquifyMode, LiquifyTool};
pub use paint::{BucketTool, EyedropperTool};
pub use placement::{MoveTool, PanTool};
pub use rig::{BoneTool, DeformTool, MeshTool};
pub use select::{LassoTool, MagicWandTool, MarqueeShape, MarqueeTool};
pub use stroke::{StrokeMode, StrokeTool};
pub use transform::{TransformHandle, TransformMode, TransformTool};

use crate::icons;
use aether_core::color::Rgba;
use aether_core::input::{InputSample, Modifiers, PointerButton};
use aether_core::math::{IRect, Vec2};
use aether_document::selection::SelectionMode;
use aether_document::{Document, History};
use aether_raster::tile::TileIter;
use aether_raster::{BrushPreset, Pixmap};
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
    /// Scale, rotate, skew, distort or warp the active layer.
    Transform,
    /// Push pixels around with a brush.
    Liquify,
    /// Pan the view.
    Pan,
    /// Edit the active layer's mesh.
    Mesh,
    /// Shape keyforms of the selected rig object.
    Deform,
    /// Draw bones.
    Bone,
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
            ToolId::Transform => "tool.transform",
            ToolId::Liquify => "tool.liquify",
            ToolId::Pan => "tool.pan",
            ToolId::Mesh => "tool.mesh",
            ToolId::Deform => "tool.deform",
            ToolId::Bone => "tool.bone",
            ToolId::Custom(name) => name,
        }
    }

    /// Short icon glyph for the toolbar.
    pub fn glyph(self) -> &'static str {
        match self {
            ToolId::Brush => icons::BRUSH,
            ToolId::Eraser => icons::ERASER,
            ToolId::Bucket => icons::BUCKET,
            ToolId::Eyedropper => icons::EYEDROPPER,
            ToolId::RectSelect => icons::RECT_SELECT,
            ToolId::EllipseSelect => icons::ELLIPSE_SELECT,
            ToolId::Lasso => icons::LASSO,
            ToolId::MagicWand => icons::WAND,
            ToolId::Move => icons::MOVE,
            ToolId::Transform => icons::TRANSFORM,
            ToolId::Liquify => icons::LIQUIFY,
            ToolId::Pan => icons::PAN,
            ToolId::Mesh => icons::MESH,
            ToolId::Deform => icons::DEFORM,
            ToolId::Bone => icons::BONE,
            ToolId::Custom(_) => icons::PLUGIN,
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
    /// Rig selection and rig-tool settings.
    pub rig: &'a mut crate::rigging::RigToolState,
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
    /// A four-corner transform cage with grab handles.
    Quad([Vec2; 4]),
    /// A deformation grid, row-major with `cols` points per row.
    Grid {
        /// Node positions in document space.
        points: Vec<Vec2>,
        /// Nodes per row.
        cols: usize,
        /// Number of rows.
        rows: usize,
    },
    /// A rubber-band rectangle in document space.
    Rect(IRect),
    /// A rubber-band ellipse in document space.
    Ellipse(IRect),
    /// A freehand outline in document space.
    Polyline(Vec<Vec2>),
    /// A straight segment in document space (a bone being drawn).
    Line(Vec2, Vec2),
}

/// Tool settings the user edits in the tool-options panel.
///
/// Modal tools keep their own working state (the transform cage, the liquify
/// field) but read their *settings* from here, so a panel can retune a live
/// gesture without needing to reach inside the tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolSettings {
    /// How the transform tool interprets a drag.
    pub transform_mode: TransformMode,
    /// What a liquify drag does.
    pub liquify_mode: LiquifyMode,
    /// Liquify brush radius in document pixels.
    pub liquify_radius: f32,
    /// Liquify strength, `0..=1`.
    pub liquify_strength: f32,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            transform_mode: TransformMode::Free,
            liquify_mode: LiquifyMode::Push,
            liquify_radius: 60.0,
            liquify_strength: 0.5,
        }
    }
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

    /// Take on settings edited in the tool-options panel.
    fn sync_settings(&mut self, _settings: &ToolSettings) {}

    /// Apply a modal tool's pending edit (Enter, or switching tool).
    ///
    /// Most tools finish on pointer-up and do nothing here; the transform and
    /// liquify tools stay live across many gestures until confirmed.
    fn commit(&mut self, _ctx: &mut ToolContext) {}

    /// True while the tool holds an uncommitted edit.
    fn is_pending(&self) -> bool {
        false
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
            Box::new(TransformTool::new()),
            Box::new(LiquifyTool::new()),
            Box::new(PanTool::new()),
            Box::new(MeshTool::new()),
            Box::new(DeformTool::new()),
            Box::new(BoneTool::new()),
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

    /// Push panel settings into the selected tool.
    pub fn sync_settings(&mut self, settings: &ToolSettings) {
        self.active_mut().sync_settings(settings);
    }

    /// True when the selected tool holds an uncommitted edit.
    pub fn has_pending_edit(&self) -> bool {
        self.active().is_pending()
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
    use aether_core::color::Rgba8;
    use aether_core::math::IRect;
    use aether_raster::BrushPreset;

    struct Harness {
        doc: Document,
        history: History,
        viewport: Viewport,
        brush: BrushPreset,
        primary: Rgba,
        composite: Pixmap,
        rig: crate::rigging::RigToolState,
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
                rig: crate::rigging::RigToolState::default(),
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
                rig: &mut self.rig,
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

    fn filled_square(h: &mut Harness, rect: IRect) {
        if let Some(pm) = h
            .doc
            .layers
            .get_mut(h.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill_rect(rect, Rgba8::WHITE);
        }
    }

    #[test]
    fn the_transform_tool_moves_content_and_commits_once() {
        let mut h = Harness::new();
        filled_square(&mut h, IRect::new(10, 10, 40, 40));
        let mut tool = TransformTool::new();

        // Grab the middle of the content, well clear of the edge handles.
        drag(&mut tool, &mut h, &[(30.0, 30.0), (42.0, 30.0)]);
        assert!(tool.is_pending(), "the transform stays live until confirmed");
        assert_eq!(h.pixel(12, 30), Rgba8::TRANSPARENT, "content left its old place");
        assert_ne!(h.pixel(42, 30), Rgba8::TRANSPARENT, "and arrived at the new one");
        assert!(!h.history.can_undo(), "nothing is recorded before the commit");

        {
            let mut ctx = h.ctx();
            tool.commit(&mut ctx);
        }
        assert_eq!(h.history.depth(), 1, "a whole transform session is one undo step");
        h.history.undo(&mut h.doc).expect("undo");
        assert_ne!(h.pixel(12, 30), Rgba8::TRANSPARENT, "undo puts the content back");
    }

    #[test]
    fn cancelling_a_transform_restores_the_layer() {
        let mut h = Harness::new();
        filled_square(&mut h, IRect::new(10, 10, 40, 40));
        let before = h
            .doc
            .layers
            .get(h.doc.active_layer)
            .and_then(|l| l.pixmap())
            .cloned();
        let mut tool = TransformTool::new();
        drag(&mut tool, &mut h, &[(30.0, 30.0), (42.0, 42.0)]);
        {
            let mut ctx = h.ctx();
            tool.cancel(&mut ctx);
        }
        assert_eq!(
            h.doc
                .layers
                .get(h.doc.active_layer)
                .and_then(|l| l.pixmap())
                .cloned(),
            before
        );
        assert!(!h.history.can_undo());
    }

    #[test]
    fn dragging_a_transform_corner_scales_the_content() {
        let mut h = Harness::new();
        filled_square(&mut h, IRect::new(10, 10, 30, 30));
        let mut tool = TransformTool::new();
        // The content bounds are (10,10)-(40,40); drag the bottom-right corner out.
        drag(&mut tool, &mut h, &[(40.0, 40.0), (58.0, 58.0)]);
        {
            let mut ctx = h.ctx();
            tool.commit(&mut ctx);
        }
        assert_ne!(
            h.pixel(52, 52),
            Rgba8::TRANSPARENT,
            "the shape should now reach further"
        );
        assert_ne!(
            h.pixel(12, 12),
            Rgba8::TRANSPARENT,
            "the anchored corner stays put"
        );
    }

    #[test]
    fn an_empty_layer_cannot_be_transformed() {
        let mut h = Harness::new();
        let mut tool = TransformTool::new();
        drag(&mut tool, &mut h, &[(20.0, 20.0), (30.0, 30.0)]);
        assert!(!tool.is_pending());
        assert!(!h.history.can_undo());
    }

    #[test]
    fn warp_mode_offers_a_grid_of_handles() {
        let mut h = Harness::new();
        filled_square(&mut h, IRect::new(10, 10, 40, 40));
        let mut tool = TransformTool::new();
        drag(&mut tool, &mut h, &[(30.0, 30.0), (31.0, 30.0)]);
        tool.sync_settings(&ToolSettings {
            transform_mode: TransformMode::Warp,
            ..Default::default()
        });
        match tool.preview() {
            Some(ToolPreview::Grid { points, cols, rows }) => {
                assert_eq!(points.len(), cols * rows);
                assert!(cols >= 3 && rows >= 3, "a warp needs interior handles");
            }
            other => panic!("expected a warp grid, got {other:?}"),
        }
    }

    #[test]
    fn the_liquify_brush_pushes_pixels_and_commits_once() {
        let mut h = Harness::new();
        filled_square(&mut h, IRect::new(20, 20, 24, 24));
        let mut tool = LiquifyTool::new();
        tool.sync_settings(&ToolSettings {
            liquify_mode: LiquifyMode::Push,
            liquify_radius: 30.0,
            liquify_strength: 1.0,
            ..Default::default()
        });
        drag(&mut tool, &mut h, &[(32.0, 32.0), (40.0, 32.0), (48.0, 32.0)]);

        assert!(tool.is_pending());
        assert_ne!(
            h.pixel(46, 32),
            Rgba8::TRANSPARENT,
            "pixels should have been dragged right"
        );
        {
            let mut ctx = h.ctx();
            tool.commit(&mut ctx);
        }
        assert_eq!(h.history.depth(), 1, "a liquify session is one undo step");
        h.history.undo(&mut h.doc).expect("undo");
        assert_eq!(h.pixel(46, 32), Rgba8::TRANSPARENT);
    }

    #[test]
    fn cancelling_liquify_restores_the_layer() {
        let mut h = Harness::new();
        filled_square(&mut h, IRect::new(20, 20, 24, 24));
        let before = h
            .doc
            .layers
            .get(h.doc.active_layer)
            .and_then(|l| l.pixmap())
            .cloned();
        let mut tool = LiquifyTool::new();
        tool.sync_settings(&ToolSettings {
            liquify_strength: 1.0,
            ..Default::default()
        });
        drag(&mut tool, &mut h, &[(32.0, 32.0), (44.0, 32.0)]);
        {
            let mut ctx = h.ctx();
            tool.cancel(&mut ctx);
        }
        assert_eq!(
            h.doc
                .layers
                .get(h.doc.active_layer)
                .and_then(|l| l.pixmap())
                .cloned(),
            before
        );
    }

    #[test]
    fn the_toolbox_offers_the_phase_two_tools() {
        let mut tools = ToolBox::standard();
        assert!(tools.select(ToolId::Transform));
        assert!(tools.select(ToolId::Liquify));
        assert!(tools.ids().contains(&ToolId::Transform));
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
