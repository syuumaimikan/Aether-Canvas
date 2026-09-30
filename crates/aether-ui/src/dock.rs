//! Docking layout.
//!
//! Panels live in an [`egui_dock`] tree, so every panel can be dragged, split,
//! tabbed or closed at runtime. A [`Workspace`] is just a saved starting tree:
//! switching workspace rebuilds the layout, it never touches the document.

use crate::canvas::CanvasView;
use crate::panels;
use crate::state::{EditorState, Workspace};
use egui_dock::{DockState, NodeIndex, SurfaceIndex};

/// Which panel a dock tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    /// The artwork.
    Canvas,
    /// Tool palette and tool options.
    Tools,
    /// Layer stack.
    Layers,
    /// Colour picker and palettes.
    Color,
    /// Brush settings.
    Brush,
    /// Undo history.
    History,
    /// Document properties.
    Properties,
    /// Rig parameters.
    Parameters,
    /// Rig hierarchy and inspector.
    Rig,
    /// Animation timeline.
    Timeline,
    /// Physics, behaviours, drivers and expressions.
    Dynamics,
    /// Keys that play motions and switch expressions.
    Hotkeys,
}

impl PanelKind {
    /// Translation key for the tab title.
    pub fn title_key(self) -> &'static str {
        match self {
            PanelKind::Canvas => "canvas.title",
            PanelKind::Tools => "tool.title",
            PanelKind::Layers => "layer.title",
            PanelKind::Color => "color.title",
            PanelKind::Brush => "brush.title",
            PanelKind::History => "history.title",
            PanelKind::Properties => "properties.title",
            PanelKind::Parameters => "param.title",
            PanelKind::Rig => "rig.title",
            PanelKind::Timeline => "timeline.title",
            PanelKind::Dynamics => "dyn.title",
            PanelKind::Hotkeys => "hotkey.title",
        }
    }
}

/// Build the starting layout for a workspace.
///
/// The proportions differ per workspace because the work does: illustration
/// wants a wide canvas and colour to hand, pixel art wants a bigger palette and
/// no brush panel clutter, compositing wants history and properties visible.
pub fn layout_for(workspace: Workspace) -> DockState<PanelKind> {
    let mut dock = DockState::new(vec![PanelKind::Canvas]);
    let surface = SurfaceIndex::main();
    let root = NodeIndex::root();

    match workspace {
        Workspace::Illustration => {
            let [canvas, left] = dock
                .main_surface_mut()
                .split_left(root, 0.16, vec![PanelKind::Tools]);
            let [_, right] = dock
                .main_surface_mut()
                .split_right(canvas, 0.78, vec![PanelKind::Layers]);
            dock.main_surface_mut()
                .split_below(right, 0.5, vec![PanelKind::Color, PanelKind::Brush]);
            let _ = (surface, left);
        }
        Workspace::PixelArt => {
            let [canvas, _tools] = dock
                .main_surface_mut()
                .split_left(root, 0.14, vec![PanelKind::Tools]);
            let [_, right] = dock
                .main_surface_mut()
                .split_right(canvas, 0.76, vec![PanelKind::Color]);
            dock.main_surface_mut()
                .split_below(right, 0.45, vec![PanelKind::Layers]);
        }
        Workspace::Compositing => {
            let [canvas, _tools] = dock
                .main_surface_mut()
                .split_left(root, 0.14, vec![PanelKind::Tools]);
            let [canvas, right] = dock.main_surface_mut().split_right(
                canvas,
                0.72,
                vec![PanelKind::Layers, PanelKind::Properties],
            );
            dock.main_surface_mut()
                .split_below(right, 0.55, vec![PanelKind::History]);
            let _ = canvas;
        }
        Workspace::Rigging => {
            let [canvas, _tools] = dock
                .main_surface_mut()
                .split_left(root, 0.13, vec![PanelKind::Tools]);
            let [_, right] =
                dock.main_surface_mut()
                    .split_right(canvas, 0.66, vec![PanelKind::Rig, PanelKind::Layers]);
            dock.main_surface_mut().split_below(
                right,
                0.46,
                vec![PanelKind::Parameters, PanelKind::Dynamics, PanelKind::Hotkeys],
            );
        }
        Workspace::Animation => {
            let [canvas, _tools] = dock
                .main_surface_mut()
                .split_left(root, 0.11, vec![PanelKind::Tools]);
            let [canvas, _right] = dock.main_surface_mut().split_right(
                canvas,
                0.72,
                vec![
                    PanelKind::Parameters,
                    PanelKind::Dynamics,
                    PanelKind::Hotkeys,
                    PanelKind::Layers,
                ],
            );
            dock.main_surface_mut()
                .split_below(canvas, 0.64, vec![PanelKind::Timeline]);
        }
    }
    dock
}

/// Renders dock tabs by delegating to the panel functions.
pub struct PanelViewer<'a> {
    /// Editor state the panels read and write.
    pub state: &'a mut EditorState,
    /// The canvas widget, kept across frames for its textures.
    pub canvas: &'a mut CanvasView,
}

impl egui_dock::TabViewer for PanelViewer<'_> {
    type Tab = PanelKind;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        self.state.tr(tab.title_key()).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            PanelKind::Canvas => self.canvas.ui(ui, self.state),
            PanelKind::Tools => panels::tools_panel(ui, self.state),
            PanelKind::Layers => panels::layers_panel(ui, self.state),
            PanelKind::Color => panels::color_panel(ui, self.state),
            PanelKind::Brush => panels::brush_panel(ui, self.state),
            PanelKind::History => panels::history_panel(ui, self.state),
            PanelKind::Properties => panels::properties_panel(ui, self.state),
            PanelKind::Parameters => panels::parameters_panel(ui, self.state),
            PanelKind::Rig => panels::rig_panel(ui, self.state),
            PanelKind::Timeline => panels::timeline_panel(ui, self.state),
            PanelKind::Dynamics => panels::dynamics_panel(ui, self.state),
            PanelKind::Hotkeys => panels::hotkeys_panel(ui, self.state),
        }
    }

    fn clear_background(&self, tab: &Self::Tab) -> bool {
        // The canvas paints its own backdrop edge to edge.
        *tab != PanelKind::Canvas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(dock: &DockState<PanelKind>) -> Vec<PanelKind> {
        dock.iter_all_tabs().map(|(_, tab)| *tab).collect()
    }

    #[test]
    fn every_workspace_shows_the_canvas() {
        for workspace in Workspace::ALL {
            let dock = layout_for(workspace);
            assert!(
                tabs(&dock).contains(&PanelKind::Canvas),
                "{workspace:?} has no canvas"
            );
        }
    }

    #[test]
    fn the_illustration_layout_has_the_painting_panels() {
        let dock = layout_for(Workspace::Illustration);
        let tabs = tabs(&dock);
        for expected in [
            PanelKind::Tools,
            PanelKind::Layers,
            PanelKind::Color,
            PanelKind::Brush,
        ] {
            assert!(tabs.contains(&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn the_compositing_layout_shows_history_and_properties() {
        let dock = layout_for(Workspace::Compositing);
        let tabs = tabs(&dock);
        assert!(tabs.contains(&PanelKind::History));
        assert!(tabs.contains(&PanelKind::Properties));
    }

    #[test]
    fn the_rigging_layouts_show_the_rig_panels() {
        let rigging = tabs(&layout_for(Workspace::Rigging));
        for expected in [
            PanelKind::Rig,
            PanelKind::Parameters,
            PanelKind::Dynamics,
            PanelKind::Tools,
        ] {
            assert!(rigging.contains(&expected), "rigging is missing {expected:?}");
        }
        let animation = tabs(&layout_for(Workspace::Animation));
        for expected in [PanelKind::Timeline, PanelKind::Parameters, PanelKind::Hotkeys] {
            assert!(animation.contains(&expected), "animation is missing {expected:?}");
        }
    }

    #[test]
    fn panel_titles_have_translation_keys() {
        for kind in [
            PanelKind::Canvas,
            PanelKind::Tools,
            PanelKind::Layers,
            PanelKind::Color,
            PanelKind::Brush,
            PanelKind::History,
            PanelKind::Properties,
            PanelKind::Parameters,
            PanelKind::Rig,
            PanelKind::Timeline,
            PanelKind::Dynamics,
            PanelKind::Hotkeys,
        ] {
            assert_ne!(
                crate::Language::English.tr(kind.title_key()),
                kind.title_key(),
                "{kind:?} has no translation"
            );
        }
    }
}
