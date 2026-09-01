//! The canvas widget.
//!
//! Responsibilities, in order:
//!
//! 1. keep a GPU texture in sync with the composite, uploading only the region
//!    the render cache reports as changed;
//! 2. draw the checkerboard, the artwork, the selection and the guides through
//!    the viewport transform, so pan, zoom, rotation and mirror all work;
//! 3. turn pointer input into document-space [`ToolEvent`]s.
//!
//! The widget owns no artwork state — only GPU handles and the small amount of
//! bookkeeping needed to avoid redundant uploads.

use crate::state::{EditorState, PointerPhase};
use crate::tools::{ToolEvent, ToolId, ToolPreview};
use aether_core::input::{InputSample, Modifiers, PointerButton};
use aether_core::math::Vec2 as DocVec;
use aether_raster::transform::Interpolation;
use egui::{
    Color32, ColorImage, Mesh, Pos2, Rect, Sense, Shape, Stroke, TextureHandle, TextureOptions, Ui, Vec2,
};

/// Zoom level above which single pixels are large enough to draw a grid.
const PIXEL_GRID_MIN_ZOOM: f32 = 6.0;

/// Canvas widget state: textures and pointer bookkeeping.
#[derive(Default)]
pub struct CanvasView {
    composite: Option<TextureHandle>,
    checker: Option<TextureHandle>,
    selection: Option<TextureHandle>,
    selection_revision: Option<u64>,
    last_screen: Option<Vec2>,
    stroke_start: f64,
    painting: bool,
    space_pan: bool,
}

impl CanvasView {
    /// A view with no textures allocated yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Draw the canvas and handle its input.
    pub fn ui(&mut self, ui: &mut Ui, state: &mut EditorState) {
        let size = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        state.viewport.size = DocVec::new(rect.width(), rect.height());

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, state.theme.canvas_backdrop());

        self.sync_composite(ui, state);
        self.draw_checkerboard(&painter, rect, state);
        self.draw_composite(&painter, rect, state);
        self.draw_selection(ui, &painter, rect, state);
        self.draw_canvas_border(&painter, rect, state);
        self.draw_pixel_grid(&painter, rect, state);

        self.handle_input(ui, &response, rect, state);
        self.draw_cursor(ui, &painter, rect, state);
    }

    /// Upload the parts of the composite that changed since the last frame.
    fn sync_composite(&mut self, ui: &mut Ui, state: &mut EditorState) {
        let changed = state.refresh();
        let image = state.composite();
        if image.is_empty() {
            self.composite = None;
            return;
        }
        let options = match state.view_interpolation {
            Interpolation::Nearest => TextureOptions::NEAREST,
            Interpolation::Bilinear => TextureOptions::LINEAR,
        };

        let needs_full = match &self.composite {
            Some(handle) => handle.size() != [image.width() as usize, image.height() as usize],
            None => true,
        };

        if needs_full {
            let full = ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.data(),
            );
            self.composite = Some(ui.ctx().load_texture("aether-composite", full, options));
            return;
        }
        if changed.is_empty() {
            return;
        }
        // Partial upload: only the dirty rectangle crosses the bus.
        let patch = image.copy_rect(changed);
        let patch_image = ColorImage::from_rgba_unmultiplied(
            [patch.width() as usize, patch.height() as usize],
            patch.data(),
        );
        if let Some(handle) = &mut self.composite {
            handle.set_partial([changed.x as usize, changed.y as usize], patch_image, options);
        }
    }

    /// The four corners of the canvas in widget coordinates.
    fn canvas_quad(&self, rect: Rect, state: &EditorState) -> [Pos2; 4] {
        let vp = state.viewport;
        let (w, h) = (state.doc.width as f32, state.doc.height as f32);
        let corners = [
            DocVec::new(0.0, 0.0),
            DocVec::new(w, 0.0),
            DocVec::new(w, h),
            DocVec::new(0.0, h),
        ];
        corners.map(|p| {
            let s = vp.doc_to_screen(p);
            Pos2::new(rect.min.x + s.x, rect.min.y + s.y)
        })
    }

    fn draw_checkerboard(&mut self, painter: &egui::Painter, rect: Rect, state: &EditorState) {
        let texture = self.checker_texture_for(painter, state);
        let quad = self.canvas_quad(rect, state);
        // Keep the squares a constant size on screen regardless of zoom.
        let cell_screen = 8.0f32;
        let repeats_x = (state.doc.width as f32 * state.viewport.zoom) / (cell_screen * 2.0);
        let repeats_y = (state.doc.height as f32 * state.viewport.zoom) / (cell_screen * 2.0);
        let uv = [
            Pos2::new(0.0, 0.0),
            Pos2::new(repeats_x, 0.0),
            Pos2::new(repeats_x, repeats_y),
            Pos2::new(0.0, repeats_y),
        ];
        painter.add(quad_mesh(texture, quad, uv, Color32::WHITE));
    }

    fn checker_texture_for(&mut self, painter: &egui::Painter, _state: &EditorState) -> egui::TextureId {
        // `checker_texture` needs a `Ui`; the handle is created on the first
        // frame and reused afterwards.
        match &self.checker {
            Some(handle) => handle.id(),
            None => {
                let light = Color32::from_rgb(0x50, 0x52, 0x57);
                let dark = Color32::from_rgb(0x44, 0x46, 0x4a);
                let image = ColorImage {
                    size: [2, 2],
                    pixels: vec![light, dark, dark, light],
                    source_size: egui::vec2(2.0, 2.0),
                };
                let handle =
                    painter
                        .ctx()
                        .load_texture("aether-checker", image, TextureOptions::NEAREST_REPEAT);
                let id = handle.id();
                self.checker = Some(handle);
                id
            }
        }
    }

    fn draw_composite(&self, painter: &egui::Painter, rect: Rect, state: &EditorState) {
        let Some(texture) = &self.composite else {
            return;
        };
        let quad = self.canvas_quad(rect, state);
        let uv = [
            Pos2::new(0.0, 0.0),
            Pos2::new(1.0, 0.0),
            Pos2::new(1.0, 1.0),
            Pos2::new(0.0, 1.0),
        ];
        painter.add(quad_mesh(texture.id(), quad, uv, Color32::WHITE));
    }

    /// Tint the selected region, rebuilding the overlay only when it changes.
    fn draw_selection(&mut self, ui: &mut Ui, painter: &egui::Painter, rect: Rect, state: &EditorState) {
        let revision = state.history.revision();
        if self.selection_revision != Some(revision) {
            self.selection_revision = Some(revision);
            self.selection = state.doc.selection.mask().map(|mask| {
                let pixels = mask
                    .data()
                    .iter()
                    .map(|coverage| Color32::from_rgba_unmultiplied(80, 150, 255, coverage / 5))
                    .collect();
                let image = ColorImage {
                    size: [mask.width() as usize, mask.height() as usize],
                    pixels,
                    source_size: egui::vec2(mask.width() as f32, mask.height() as f32),
                };
                ui.ctx()
                    .load_texture("aether-selection", image, TextureOptions::LINEAR)
            });
        }
        let Some(texture) = &self.selection else {
            return;
        };
        let quad = self.canvas_quad(rect, state);
        let uv = [
            Pos2::new(0.0, 0.0),
            Pos2::new(1.0, 0.0),
            Pos2::new(1.0, 1.0),
            Pos2::new(0.0, 1.0),
        ];
        painter.add(quad_mesh(texture.id(), quad, uv, Color32::WHITE));

        // Outline the selection bounds so its extent is readable at any zoom.
        if let Some(bounds) = state.doc.selection.bounds() {
            let stroke = Stroke::new(1.0, Color32::from_rgb(120, 180, 255));
            painter.add(Shape::closed_line(doc_rect_points(rect, state, bounds), stroke));
        }
    }

    fn draw_canvas_border(&self, painter: &egui::Painter, rect: Rect, state: &EditorState) {
        let quad = self.canvas_quad(rect, state);
        painter.add(Shape::closed_line(
            quad.to_vec(),
            Stroke::new(1.0, Color32::from_gray(90)),
        ));
    }

    /// One-pixel grid, drawn only when pixels are big enough to see.
    fn draw_pixel_grid(&self, painter: &egui::Painter, rect: Rect, state: &EditorState) {
        if !state.show_pixel_grid || state.viewport.zoom < PIXEL_GRID_MIN_ZOOM {
            return;
        }
        let vp = state.viewport;
        let visible = vp.visible_document_rect();
        let x0 = visible.min.x.floor().max(0.0) as i32;
        let x1 = (visible.max.x.ceil() as i32).min(state.doc.width as i32);
        let y0 = visible.min.y.floor().max(0.0) as i32;
        let y1 = (visible.max.y.ceil() as i32).min(state.doc.height as i32);
        // Guard against pathological view states producing millions of lines.
        if (x1 - x0) > 4096 || (y1 - y0) > 4096 {
            return;
        }
        let stroke = Stroke::new(0.5, Color32::from_black_alpha(70));
        let to_screen = |p: DocVec| {
            let s = vp.doc_to_screen(p);
            Pos2::new(rect.min.x + s.x, rect.min.y + s.y)
        };
        for x in x0..=x1 {
            painter.line_segment(
                [
                    to_screen(DocVec::new(x as f32, y0 as f32)),
                    to_screen(DocVec::new(x as f32, y1 as f32)),
                ],
                stroke,
            );
        }
        for y in y0..=y1 {
            painter.line_segment(
                [
                    to_screen(DocVec::new(x0 as f32, y as f32)),
                    to_screen(DocVec::new(x1 as f32, y as f32)),
                ],
                stroke,
            );
        }
    }

    /// Brush outline and in-progress tool previews.
    fn draw_cursor(&self, ui: &Ui, painter: &egui::Painter, rect: Rect, state: &EditorState) {
        if let Some(preview) = state.tools.active().preview() {
            let stroke = Stroke::new(1.0, Color32::from_rgb(160, 200, 255));
            match preview {
                ToolPreview::Rect(r) => {
                    painter.add(Shape::closed_line(doc_rect_points(rect, state, r), stroke));
                }
                ToolPreview::Ellipse(r) => {
                    let points = doc_rect_points(rect, state, r);
                    // Approximate the ellipse by its inscribed polygon.
                    let center = Pos2::new(
                        (points[0].x + points[2].x) * 0.5,
                        (points[0].y + points[2].y) * 0.5,
                    );
                    let rx = (points[1].x - points[0].x).abs() * 0.5;
                    let ry = (points[3].y - points[0].y).abs() * 0.5;
                    let ellipse: Vec<Pos2> = (0..48)
                        .map(|i| {
                            let a = i as f32 / 48.0 * std::f32::consts::TAU;
                            Pos2::new(center.x + rx * a.cos(), center.y + ry * a.sin())
                        })
                        .collect();
                    painter.add(Shape::closed_line(ellipse, stroke));
                }
                ToolPreview::Polyline(points) => {
                    let screen: Vec<Pos2> = points
                        .iter()
                        .map(|p| {
                            let s = state.viewport.doc_to_screen(*p);
                            Pos2::new(rect.min.x + s.x, rect.min.y + s.y)
                        })
                        .collect();
                    painter.add(Shape::closed_line(screen, stroke));
                }
            }
        }

        // Brush outline follows the cursor for the painting tools.
        let paints = matches!(state.tools.active_id(), ToolId::Brush | ToolId::Eraser);
        if !paints {
            return;
        }
        let Some(pos) = ui.ctx().pointer_latest_pos() else {
            return;
        };
        if !rect.contains(pos) {
            return;
        }
        let radius = state.brush.size * 0.5 * state.viewport.zoom;
        if radius > 1.0 && radius < 4000.0 {
            painter.circle_stroke(pos, radius, Stroke::new(1.0, Color32::from_white_alpha(160)));
        }
    }

    /// Pointer and wheel handling.
    fn handle_input(&mut self, ui: &mut Ui, response: &egui::Response, rect: Rect, state: &mut EditorState) {
        let ctx = ui.ctx().clone();
        let modifiers = ctx.input(|i| i.modifiers);
        let mods = Modifiers {
            shift: modifiers.shift,
            ctrl: modifiers.command || modifiers.ctrl,
            alt: modifiers.alt,
        };

        // Wheel zooms about the cursor; that is the behaviour every editor has.
        if response.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                if let Some(pos) = ctx.pointer_latest_pos() {
                    let anchor = DocVec::new(pos.x - rect.min.x, pos.y - rect.min.y);
                    let factor = if scroll > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    state.viewport.zoom_at(anchor, factor);
                }
            }
        }

        // Space or the middle button pans, whatever tool is selected.
        let space = ctx.input(|i| i.key_down(egui::Key::Space));
        let middle = ctx.input(|i| i.pointer.middle_down());
        self.space_pan = space || middle;

        let time = ctx.input(|i| i.time);
        let Some(pos) = response
            .interact_pointer_pos()
            .or_else(|| ctx.pointer_latest_pos())
        else {
            self.last_screen = None;
            return;
        };
        let screen = Vec2::new(pos.x - rect.min.x, pos.y - rect.min.y);
        let delta = self.last_screen.map(|last| screen - last).unwrap_or(Vec2::ZERO);
        self.last_screen = Some(screen);

        if self.space_pan {
            if response.dragged() || middle {
                state.viewport.pan_by_screen(DocVec::new(delta.x, delta.y));
            }
            return;
        }

        let doc_point = state.viewport.screen_to_doc(DocVec::new(screen.x, screen.y));
        let button = if ctx.input(|i| i.pointer.secondary_down()) {
            PointerButton::Secondary
        } else {
            PointerButton::Primary
        };
        let event = ToolEvent {
            sample: InputSample {
                position: doc_point,
                pressure: 1.0,
                tilt: DocVec::ZERO,
                rotation: 0.0,
                velocity: 0.0,
                time: time - self.stroke_start,
                modifiers: mods,
            },
            screen: DocVec::new(screen.x, screen.y),
            screen_delta: DocVec::new(delta.x, delta.y),
            button,
            modifiers: mods,
        };

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.painting {
            state.tool_pointer(PointerPhase::Cancel, &event);
            self.painting = false;
            return;
        }

        if response.drag_started() || (response.clicked() && !self.painting) {
            self.stroke_start = time;
            self.painting = true;
            state.tool_pointer(PointerPhase::Down, &event);
        } else if response.dragged() && self.painting {
            state.tool_pointer(PointerPhase::Move, &event);
        }
        if (response.drag_stopped() || response.clicked()) && self.painting {
            state.tool_pointer(PointerPhase::Up, &event);
            self.painting = false;
        }
    }
}

/// Build a textured quad.
fn quad_mesh(texture: egui::TextureId, corners: [Pos2; 4], uv: [Pos2; 4], tint: Color32) -> Shape {
    let mut mesh = Mesh::with_texture(texture);
    for (pos, uv) in corners.iter().zip(uv.iter()) {
        mesh.colored_vertex(*pos, tint);
        // `colored_vertex` does not take a uv, so patch the last vertex.
        if let Some(vertex) = mesh.vertices.last_mut() {
            vertex.uv = *uv;
        }
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

/// The four corners of a document rectangle, in widget coordinates.
fn doc_rect_points(rect: Rect, state: &EditorState, r: aether_core::math::IRect) -> Vec<Pos2> {
    let vp = state.viewport;
    [
        DocVec::new(r.x as f32, r.y as f32),
        DocVec::new(r.right() as f32, r.y as f32),
        DocVec::new(r.right() as f32, r.bottom() as f32),
        DocVec::new(r.x as f32, r.bottom() as f32),
    ]
    .iter()
    .map(|p| {
        let s = vp.doc_to_screen(*p);
        Pos2::new(rect.min.x + s.x, rect.min.y + s.y)
    })
    .collect()
}
