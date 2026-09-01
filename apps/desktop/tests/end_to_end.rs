//! End-to-end tests across every crate.
//!
//! These drive the same entry points the user interface does — the editor
//! state, the tools, the command history and the file layer — so a regression
//! anywhere in the chain shows up here even though no window is ever opened.

use aether_core::color::{Rgba, Rgba8};
use aether_core::input::{InputSample, Modifiers, PointerButton};
use aether_core::math::{IRect, Vec2};
use aether_document::command::LayerProperty;
use aether_document::selection::SelectionMode;
use aether_document::Document;
use aether_ui::shortcuts::Action;
use aether_ui::state::{PointerPhase, Workspace};
use aether_ui::tools::{ToolEvent, ToolId};
use aether_ui::EditorState;

fn event(x: f32, y: f32, t: f64) -> ToolEvent {
    ToolEvent {
        sample: InputSample::at(Vec2::new(x, y)).with_time(t),
        screen: Vec2::new(x, y),
        screen_delta: Vec2::ZERO,
        button: PointerButton::Primary,
        modifiers: Modifiers::NONE,
    }
}

fn stroke(state: &mut EditorState, points: &[(f32, f32)]) {
    let mut t = 0.0;
    for (i, (x, y)) in points.iter().enumerate() {
        t += 0.016;
        let ev = event(*x, *y, t);
        if i == 0 {
            state.tool_pointer(PointerPhase::Down, &ev);
        } else {
            state.tool_pointer(PointerPhase::Move, &ev);
        }
    }
    let last = points.last().copied().unwrap_or((0.0, 0.0));
    state.tool_pointer(PointerPhase::Up, &event(last.0, last.1, t + 0.016));
}

fn new_state(size: u32) -> EditorState {
    let mut state = EditorState::new(Document::new(size, size, "Session"));
    state.brush.size = 12.0;
    state.brush.hardness = 1.0;
    state.brush.smoothing = 0.0;
    state
}

fn layer_pixel(state: &EditorState, x: i32, y: i32) -> Rgba8 {
    state
        .doc
        .layers
        .get(state.doc.active_layer)
        .and_then(|l| l.pixmap())
        .map(|p| p.get(x, y))
        .unwrap_or(Rgba8::TRANSPARENT)
}

#[test]
fn a_full_session_paints_saves_reopens_and_exports() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path().join("session.aether");
    let png = dir.path().join("session.png");

    let mut state = new_state(128);
    state.primary = Rgba::rgb(0.9, 0.2, 0.2);
    stroke(&mut state, &[(20.0, 64.0), (60.0, 64.0), (100.0, 64.0)]);

    // A second layer, blended and partly transparent.
    let second = state.add_layer().expect("add layer");
    state.primary = Rgba::rgb(0.2, 0.4, 0.9);
    stroke(&mut state, &[(64.0, 20.0), (64.0, 100.0)]);
    state
        .set_layer_property(second, LayerProperty::Opacity(0.5))
        .expect("opacity");

    state.save_as(&project).expect("save");
    state.export_png(&png).expect("export");
    assert!(project.exists() && png.exists());

    let composite_before = state.compositor.render(&state.doc);

    // Reopen into a fresh session and confirm nothing drifted.
    let mut reopened = new_state(8);
    reopened.open_project(&project).expect("open");
    assert_eq!(reopened.doc.layer_count(), 2);
    assert_eq!((reopened.doc.width, reopened.doc.height), (128, 128));
    let composite_after = reopened.compositor.render(&reopened.doc);
    assert_eq!(
        composite_before, composite_after,
        "the artwork changed across save/load"
    );

    // The exported PNG matches the composite over the (transparent) background.
    let exported = aether_io::load_image(&png).expect("read png");
    assert_eq!(exported, composite_before);
}

#[test]
fn undo_walks_a_whole_session_back_to_an_empty_canvas() {
    let mut state = new_state(64);
    stroke(&mut state, &[(10.0, 10.0), (40.0, 40.0)]);
    state.add_layer().expect("add");
    stroke(&mut state, &[(20.0, 40.0), (50.0, 10.0)]);
    state.handle_action(Action::SelectAll);
    state.fill_selection().expect("fill");

    let steps = state.history.depth();
    assert!(steps >= 4, "expected several history entries, got {steps}");
    for _ in 0..steps {
        state.history.undo(&mut state.doc).expect("undo");
    }
    assert_eq!(state.doc.layer_count(), 1);
    assert_eq!(layer_pixel(&state, 25, 25), Rgba8::TRANSPARENT);
    assert!(!state.history.can_undo());

    // And forward again.
    while state.history.can_redo() {
        state.history.redo(&mut state.doc).expect("redo");
    }
    assert_eq!(state.doc.layer_count(), 2);
}

#[test]
fn tools_selections_and_masks_work_together() {
    let mut state = new_state(64);

    // Select the left half, then fill it.
    state.select_tool(ToolId::RectSelect);
    stroke(&mut state, &[(0.0, 0.0), (32.0, 64.0)]);
    assert_eq!(state.doc.selection.bounds(), Some(IRect::new(0, 0, 32, 64)));

    state.primary = Rgba::rgb(0.0, 0.6, 0.3);
    state.fill_selection().expect("fill");
    assert_ne!(layer_pixel(&state, 10, 10), Rgba8::TRANSPARENT);
    assert_eq!(layer_pixel(&state, 50, 10), Rgba8::TRANSPARENT);

    // Painting outside the selection does nothing.
    state.select_tool(ToolId::Brush);
    state.primary = Rgba::rgb(1.0, 0.0, 0.0);
    stroke(&mut state, &[(45.0, 32.0), (60.0, 32.0)]);
    assert_eq!(layer_pixel(&state, 50, 32), Rgba8::TRANSPARENT);

    // Turn the selection into a mask, then check the composite honours it.
    let id = state.doc.active_layer;
    state.deselect().expect("deselect");
    state
        .doc
        .selection
        .select_rect(64, 64, IRect::new(0, 0, 16, 64), SelectionMode::Replace);
    state.add_mask_from_selection(id).expect("mask");
    state.cache.invalidate();
    state.refresh();
    assert_ne!(state.composite().get(8, 8), Rgba8::TRANSPARENT);
    assert_eq!(
        state.composite().get(24, 8),
        Rgba8::TRANSPARENT,
        "mask should hide this"
    );
}

#[test]
fn the_incremental_cache_agrees_with_a_full_render_after_many_edits() {
    let mut state = new_state(256);
    state.refresh();
    for i in 0..6 {
        state.primary = Rgba::rgb(i as f32 / 6.0, 0.3, 0.7);
        let y = 20.0 + i as f32 * 30.0;
        stroke(&mut state, &[(20.0, y), (200.0, y)]);
        state.refresh();
    }
    state.add_layer().expect("add");
    stroke(&mut state, &[(30.0, 30.0), (220.0, 220.0)]);
    state.refresh();

    let full = state.compositor.render(&state.doc);
    assert_eq!(state.composite(), &full, "incremental compositing drifted");
}

#[test]
fn workspaces_change_settings_without_touching_the_artwork() {
    let mut state = new_state(64);
    stroke(&mut state, &[(10.0, 10.0), (50.0, 50.0)]);
    let before = state.compositor.render(&state.doc);

    state.set_workspace(Workspace::PixelArt);
    assert!(state.show_pixel_grid);
    state.set_workspace(Workspace::Compositing);
    assert!(!state.show_pixel_grid);

    let after = state.compositor.render(&state.doc);
    assert_eq!(before, after);
}

#[test]
fn importing_an_image_and_merging_produces_one_flat_layer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let png = dir.path().join("stamp.png");
    let stamp = aether_raster::Pixmap::filled(32, 32, Rgba8::new(255, 200, 0, 255));
    aether_io::save_png(&stamp, &png).expect("write");

    let mut state = new_state(64);
    if let Some(pm) = state
        .doc
        .layers
        .get_mut(state.doc.active_layer)
        .and_then(|l| l.pixmap_mut())
    {
        pm.fill(Rgba8::new(0, 0, 128, 255));
    }
    state.import_image_as_layer(&png).expect("import");
    assert_eq!(state.doc.layer_count(), 2);

    let top = state.doc.active_layer;
    state.merge_down(top).expect("merge");
    assert_eq!(state.doc.layer_count(), 1);
    assert_eq!(layer_pixel(&state, 10, 10), Rgba8::new(255, 200, 0, 255));
    assert_eq!(layer_pixel(&state, 50, 50), Rgba8::new(0, 0, 128, 255));
}

#[test]
fn a_locked_layer_survives_every_painting_tool() {
    let mut state = new_state(64);
    if let Some(layer) = state.doc.active_mut() {
        layer.locked = true;
    }
    for tool in [ToolId::Brush, ToolId::Eraser, ToolId::Bucket, ToolId::Move] {
        state.select_tool(tool);
        stroke(&mut state, &[(10.0, 10.0), (40.0, 40.0)]);
    }
    assert_eq!(layer_pixel(&state, 20, 20), Rgba8::TRANSPARENT);
    assert!(!state.history.can_undo(), "nothing should have been recorded");
}
