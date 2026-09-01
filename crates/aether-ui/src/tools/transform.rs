//! The interactive transform tool.
//!
//! Scaling, rotating, skewing, distorting and warping are one tool with one
//! representation: a **destination quad**. The four corners of the layer's
//! content are dragged around, and everything the tool offers is a rule about
//! how a drag moves those corners:
//!
//! | Mode | Corner drag | Edge drag | Inside drag |
//! | --- | --- | --- | --- |
//! | Free | scale about the opposite corner | scale one axis | move |
//! | Distort | move that corner alone (perspective) | move that edge | move |
//! | Warp | move a grid node | — | move |
//!
//! Rendering is the same in every mode: the homography that maps the original
//! rectangle onto the current quad, optionally followed by the warp grid. That
//! is why a "free" transform and a perspective distort cannot drift apart —
//! there is only one resampler.
//!
//! The tool is **modal**: it stays live across many drags and only writes an
//! undo entry when confirmed with Enter (or by switching tool). Escape puts the
//! layer back exactly as it was.

use super::{Tool, ToolContext, ToolEvent, ToolId, ToolPreview, ToolSettings};
use aether_core::math::{IRect, Vec2};
use aether_document::command::RegionEdit;
use aether_document::LayerId;
use aether_raster::composite::{composite_pixmap, CompositeOptions};
use aether_raster::transform::{perspective_pixmap, Interpolation, Perspective};
use aether_raster::{DisplacementField, Pixmap};

/// How a drag reshapes the quad.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformMode {
    /// Scale, rotate and move; the quad stays a parallelogram.
    #[default]
    Free,
    /// Move each corner independently, producing a perspective projection.
    Distort,
    /// Deform through a grid of control points.
    Warp,
}

impl TransformMode {
    /// All modes, in toolbar order.
    pub const ALL: [TransformMode; 3] = [TransformMode::Free, TransformMode::Distort, TransformMode::Warp];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            TransformMode::Free => "Free",
            TransformMode::Distort => "Distort",
            TransformMode::Warp => "Warp",
        }
    }
}

/// What the pointer grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformHandle {
    /// One of the four corners, clockwise from the top-left.
    Corner(usize),
    /// The midpoint of one of the four edges, clockwise from the top.
    Edge(usize),
    /// A warp grid node.
    Node(usize, usize),
    /// The rotation ring outside the quad.
    Rotate,
    /// The inside of the quad: translate.
    Body,
}

/// Grid resolution used by warp mode.
const WARP_NODES: usize = 4;
/// Screen-space radius, in document pixels, for grabbing a handle.
const HANDLE_GRAB: f32 = 10.0;

/// Interactive scale / rotate / skew / distort / warp.
pub struct TransformTool {
    mode: TransformMode,
    /// Pixels as they were before the transform began.
    original: Option<Pixmap>,
    /// The rectangle of the original content being transformed.
    source: IRect,
    /// Current destination corners, clockwise from the top-left.
    quad: [Vec2; 4],
    /// Warp deformation applied after the quad mapping.
    warp: Option<DisplacementField>,
    layer: LayerId,
    grabbed: Option<TransformHandle>,
    last: Vec2,
    dirty: bool,
}

impl Default for TransformTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformTool {
    /// A tool with nothing being transformed yet.
    pub fn new() -> Self {
        Self {
            mode: TransformMode::Free,
            original: None,
            source: IRect::EMPTY,
            quad: [Vec2::ZERO; 4],
            warp: None,
            layer: LayerId::NONE,
            grabbed: None,
            last: Vec2::ZERO,
            dirty: false,
        }
    }

    /// The current mode.
    pub fn mode(&self) -> TransformMode {
        self.mode
    }

    /// Switch mode, keeping the transform in progress.
    ///
    /// Leaving warp mode discards the grid deformation, because a warp cannot
    /// be expressed as a quad and silently keeping it would make the handles
    /// lie about what will be rendered.
    pub fn set_mode(&mut self, mode: TransformMode) {
        if mode == self.mode {
            return;
        }
        if self.mode == TransformMode::Warp {
            self.warp = None;
            self.dirty = true;
        }
        if mode == TransformMode::Warp && self.original.is_some() {
            self.warp = Some(DisplacementField::new(self.quad_bounds(), WARP_NODES, WARP_NODES));
        }
        self.mode = mode;
    }

    /// True when a transform is in progress.
    pub fn is_active_transform(&self) -> bool {
        self.original.is_some()
    }

    /// Begin transforming the active layer, using the selection when there is one.
    fn begin(&mut self, ctx: &mut ToolContext) -> bool {
        if let Err(err) = ctx.doc.paint_target() {
            ctx.report(err.to_string());
            return false;
        }
        self.layer = ctx.doc.active_layer;
        let content = ctx
            .doc
            .layers
            .get(self.layer)
            .and_then(|l| l.pixmap())
            .map(|p| p.opaque_bounds())
            .unwrap_or(IRect::EMPTY);
        let source = match ctx.doc.selection.bounds() {
            Some(selection) => selection.intersect(&content),
            None => content,
        };
        if source.is_empty() {
            ctx.report("Nothing to transform on this layer");
            return false;
        }
        self.source = source;
        self.quad = rect_corners(source);
        self.warp = None;
        self.dirty = false;
        self.original = ctx.doc.layers.get(self.layer).and_then(|l| l.pixmap()).cloned();
        self.original.is_some()
    }

    /// Bounding box of the current quad, rounded outwards.
    fn quad_bounds(&self) -> IRect {
        let mut min = self.quad[0];
        let mut max = self.quad[0];
        for p in &self.quad[1..] {
            min = min.min(*p);
            max = max.max(*p);
        }
        IRect::from_bounds(
            min.x.floor() as i32,
            min.y.floor() as i32,
            max.x.ceil() as i32,
            max.y.ceil() as i32,
        )
    }

    fn center(&self) -> Vec2 {
        (self.quad[0] + self.quad[1] + self.quad[2] + self.quad[3]) * 0.25
    }

    fn edge_midpoint(&self, edge: usize) -> Vec2 {
        (self.quad[edge] + self.quad[(edge + 1) % 4]) * 0.5
    }

    /// Which handle is under `p`, if any.
    fn hit_test(&self, p: Vec2, tolerance: f32) -> Option<TransformHandle> {
        if let (TransformMode::Warp, Some(field)) = (self.mode, self.warp.as_ref()) {
            for row in 0..field.rows() {
                for col in 0..field.cols() {
                    let node = field.node_position(col, row) + field.node(col, row);
                    if node.distance(p) <= tolerance {
                        return Some(TransformHandle::Node(col, row));
                    }
                }
            }
        } else {
            for (i, corner) in self.quad.iter().enumerate() {
                if corner.distance(p) <= tolerance {
                    return Some(TransformHandle::Corner(i));
                }
            }
            for edge in 0..4 {
                if self.edge_midpoint(edge).distance(p) <= tolerance {
                    return Some(TransformHandle::Edge(edge));
                }
            }
        }
        if point_in_quad(p, &self.quad) {
            return Some(TransformHandle::Body);
        }
        // Just outside the quad, near a corner: the rotation ring.
        let center = self.center();
        let radius = self
            .quad
            .iter()
            .map(|c| c.distance(center))
            .fold(0.0f32, f32::max);
        if p.distance(center) <= radius + tolerance * 3.0 {
            return Some(TransformHandle::Rotate);
        }
        None
    }

    /// Move the grabbed handle to `p`.
    fn drag(&mut self, handle: TransformHandle, p: Vec2, delta: Vec2, constrain: bool) {
        match handle {
            TransformHandle::Body => {
                for corner in self.quad.iter_mut() {
                    *corner += delta;
                }
                // The warp grid is anchored to the quad, so it has to travel
                // with it while keeping each node's own displacement.
                let bounds = self.quad_bounds();
                if let Some(field) = &mut self.warp {
                    let mut moved = DisplacementField::new(bounds, field.cols(), field.rows());
                    for row in 0..field.rows() {
                        for col in 0..field.cols() {
                            moved.set_node(col, row, field.node(col, row));
                        }
                    }
                    *field = moved;
                }
            }
            TransformHandle::Rotate => {
                let center = self.center();
                let before = (p - delta) - center;
                let after = p - center;
                if before.length() > 1.0 && after.length() > 1.0 {
                    let mut angle = after.angle() - before.angle();
                    if constrain {
                        // Snap to 15 degree steps.
                        let step = std::f32::consts::FRAC_PI_8 * 0.5;
                        angle = (angle / step).round() * step;
                    }
                    for corner in self.quad.iter_mut() {
                        *corner = center + (*corner - center).rotated(angle);
                    }
                }
            }
            TransformHandle::Corner(index) => match self.mode {
                TransformMode::Distort | TransformMode::Warp => {
                    self.quad[index] = p;
                }
                TransformMode::Free => self.scale_from_corner(index, p, constrain),
            },
            TransformHandle::Edge(edge) => match self.mode {
                TransformMode::Distort | TransformMode::Warp => {
                    self.quad[edge] += delta;
                    self.quad[(edge + 1) % 4] += delta;
                }
                TransformMode::Free => self.scale_from_edge(edge, delta),
            },
            TransformHandle::Node(col, row) => {
                if let Some(field) = &mut self.warp {
                    let base = field.node_position(col, row);
                    field.set_node(col, row, p - base);
                }
            }
        }
        self.dirty = true;
    }

    /// Scale about the opposite corner, keeping the quad a parallelogram.
    fn scale_from_corner(&mut self, index: usize, p: Vec2, keep_aspect: bool) {
        let anchor = self.quad[(index + 2) % 4];
        let u = self.quad[(index + 1) % 4] - anchor;
        let v = self.quad[(index + 3) % 4] - anchor;
        let current = self.quad[index] - anchor;
        if current.length_squared() < 1e-6 {
            return;
        }
        let target = p - anchor;
        // Work in the quad's own axes so a rotated transform still scales
        // along its own edges rather than along the screen.
        let (mut su, mut sv) = (project_ratio(target, v, u), project_ratio(target, u, v));
        if keep_aspect {
            let s = (su.abs() + sv.abs()) * 0.5;
            su = s * su.signum();
            sv = s * sv.signum();
        }
        if !su.is_finite() || !sv.is_finite() {
            return;
        }
        let new_u = u * su;
        let new_v = v * sv;
        self.quad[(index + 1) % 4] = anchor + new_u;
        self.quad[(index + 3) % 4] = anchor + new_v;
        self.quad[index] = anchor + new_u + new_v;
    }

    /// Scale one axis by dragging an edge.
    fn scale_from_edge(&mut self, edge: usize, delta: Vec2) {
        let a = edge;
        let b = (edge + 1) % 4;
        // The edge's outward normal, so a drag sideways along the edge does nothing.
        let along = (self.quad[b] - self.quad[a]).normalized();
        let normal = Vec2::new(-along.y, along.x);
        let amount = normal * delta.dot(normal);
        self.quad[a] += amount;
        self.quad[b] += amount;
    }

    /// Redraw the layer with the current quad and warp.
    fn render_preview(&mut self, ctx: &mut ToolContext) {
        let (Some(original), false) = (self.original.as_ref(), self.source.is_empty()) else {
            return;
        };
        let piece = original.copy_rect(self.source);
        let src_corners = [
            Vec2::new(0.0, 0.0),
            Vec2::new(piece.width() as f32, 0.0),
            Vec2::new(piece.width() as f32, piece.height() as f32),
            Vec2::new(0.0, piece.height() as f32),
        ];
        let Some(homography) = Perspective::from_quads(src_corners, self.quad) else {
            // A quad dragged inside out has no valid mapping; keep the last
            // good preview rather than flashing an empty layer.
            return;
        };

        let mut transformed = perspective_pixmap(
            &piece,
            &homography,
            ctx.doc.width,
            ctx.doc.height,
            Interpolation::Bilinear,
        );
        if let Some(field) = &self.warp {
            transformed = field.apply(&transformed, Interpolation::Bilinear);
        }

        // Start from the original with the source area lifted out, then drop
        // the transformed copy on top.
        let mut result = original.clone();
        result.fill_rect(self.source, aether_core::color::Rgba8::TRANSPARENT);
        composite_pixmap(&mut result, &transformed, &CompositeOptions::normal(), None);

        let region = ctx.doc.bounds();
        if let Ok(target) = ctx.doc.paint_target() {
            target.paste_rect(&result, 0, 0);
        }
        ctx.doc.mark_dirty(region);
        self.dirty = false;
    }

    /// Discard the transform, restoring the layer.
    fn restore(&mut self, ctx: &mut ToolContext) {
        if let Some(original) = self.original.take() {
            if let Ok(target) = ctx.doc.paint_target() {
                target.paste_rect(&original, 0, 0);
            }
            let region = ctx.doc.bounds();
            ctx.doc.mark_dirty(region);
        }
        self.warp = None;
        self.grabbed = None;
        self.source = IRect::EMPTY;
    }
}

impl Tool for TransformTool {
    fn id(&self) -> ToolId {
        ToolId::Transform
    }

    fn sync_settings(&mut self, settings: &ToolSettings) {
        self.set_mode(settings.transform_mode);
    }

    fn pointer_down(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        if self.original.is_none() && !self.begin(ctx) {
            return;
        }
        // Scale the grab radius with zoom so handles stay grabbable when
        // zoomed out.
        let tolerance = HANDLE_GRAB / ctx.viewport.zoom.max(0.05);
        self.grabbed = self.hit_test(event.sample.position, tolerance);
        self.last = event.sample.position;
    }

    fn pointer_move(&mut self, ctx: &mut ToolContext, event: &ToolEvent) {
        let Some(handle) = self.grabbed else {
            return;
        };
        let p = event.sample.position;
        let delta = p - self.last;
        self.last = p;
        if delta.length_squared() < 1e-6 {
            return;
        }
        self.drag(handle, p, delta, event.modifiers.shift);
        self.render_preview(ctx);
    }

    fn pointer_up(&mut self, ctx: &mut ToolContext, _event: &ToolEvent) {
        self.grabbed = None;
        if self.dirty {
            self.render_preview(ctx);
        }
    }

    fn commit(&mut self, ctx: &mut ToolContext) {
        let Some(original) = self.original.take() else {
            return;
        };
        let region = ctx.doc.bounds();
        let after = ctx
            .doc
            .layers
            .get(self.layer)
            .and_then(|l| l.pixmap())
            .map(|p| p.copy_rect(region))
            .unwrap_or_else(|| Pixmap::new(region.width as u32, region.height as u32));
        let before = original.copy_rect(region);
        let edit = RegionEdit::new("Transform", self.layer, region, before, after);
        if !edit.is_noop() {
            ctx.history.push_applied(Box::new(edit));
        }
        self.warp = None;
        self.source = IRect::EMPTY;
        self.grabbed = None;
    }

    fn cancel(&mut self, ctx: &mut ToolContext) {
        self.restore(ctx);
    }

    fn preview(&self) -> Option<ToolPreview> {
        self.original.as_ref()?;
        match (self.mode, self.warp.as_ref()) {
            (TransformMode::Warp, Some(field)) => {
                let mut points = Vec::with_capacity(field.cols() * field.rows());
                for row in 0..field.rows() {
                    for col in 0..field.cols() {
                        points.push(field.node_position(col, row) + field.node(col, row));
                    }
                }
                Some(ToolPreview::Grid {
                    points,
                    cols: field.cols(),
                    rows: field.rows(),
                })
            }
            _ => Some(ToolPreview::Quad(self.quad)),
        }
    }

    fn is_active(&self) -> bool {
        self.grabbed.is_some()
    }

    fn is_pending(&self) -> bool {
        self.original.is_some()
    }
}

/// The four corners of a rectangle, clockwise from the top-left.
fn rect_corners(rect: IRect) -> [Vec2; 4] {
    [
        Vec2::new(rect.x as f32, rect.y as f32),
        Vec2::new(rect.right() as f32, rect.y as f32),
        Vec2::new(rect.right() as f32, rect.bottom() as f32),
        Vec2::new(rect.x as f32, rect.bottom() as f32),
    ]
}

/// How far `target` reaches along `axis`, measured after removing `other`.
fn project_ratio(target: Vec2, other: Vec2, axis: Vec2) -> f32 {
    // Solve target = s*axis + t*other for s using the 2D cross product, which
    // is stable for the near-degenerate quads a drag can produce.
    let denominator = axis.cross(other);
    if denominator.abs() < 1e-6 {
        return 1.0;
    }
    target.cross(other) / denominator
}

/// True when `p` is inside the (possibly rotated) quad.
fn point_in_quad(p: Vec2, quad: &[Vec2; 4]) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let edge = quad[(i + 1) % 4] - quad[i];
        let to_point = p - quad[i];
        let cross = edge.cross(to_point);
        if cross.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}
