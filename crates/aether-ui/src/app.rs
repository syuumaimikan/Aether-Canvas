//! The application shell: menus, dialogs, status bar and the dock area.

use crate::canvas::CanvasView;
use crate::dock::{layout_for, PanelKind, PanelViewer};
use crate::i18n::Language;
use crate::shortcuts::{Action, Binding};
use crate::state::{EditorState, Workspace};
use crate::theme::Theme;
use aether_document::Document;
use aether_io::project;
use aether_raster::adjust::Adjustment;
use aether_raster::effect::EffectKind;
use egui_dock::{DockArea, DockState, Style};
use std::path::PathBuf;

/// Parameters of the "new document" dialog.
struct NewDocumentDialog {
    open: bool,
    width: u32,
    height: u32,
    name: String,
}

impl Default for NewDocumentDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: 1920,
            height: 1080,
            name: "Untitled".into(),
        }
    }
}

/// A destructive filter waiting to be applied.
struct FilterDialog {
    open: bool,
    kind: EffectKind,
}

impl Default for FilterDialog {
    fn default() -> Self {
        Self {
            open: false,
            kind: EffectKind::Blur { sigma: 4.0 },
        }
    }
}

/// The eframe application.
pub struct AetherApp {
    state: EditorState,
    dock: DockState<PanelKind>,
    canvas: CanvasView,
    new_dialog: NewDocumentDialog,
    filter_dialog: FilterDialog,
    show_about: bool,
    show_shortcuts: bool,
    rebinding: Option<Action>,
    confirm_close: bool,
    applied_theme: Option<Theme>,
}

impl Default for AetherApp {
    fn default() -> Self {
        Self::with_document(Document::new(1920, 1080, "Untitled"))
    }
}

impl AetherApp {
    /// Build the application, optionally opening `path` at startup.
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let mut app = Self::default();
        app.apply_theme(&cc.egui_ctx);
        if let Some(path) = path {
            app.open_path(path);
        }
        app
    }

    /// Build the application around an existing document.
    pub fn with_document(doc: Document) -> Self {
        let state = EditorState::new(doc);
        let dock = layout_for(state.workspace);
        Self {
            state,
            dock,
            canvas: CanvasView::new(),
            new_dialog: NewDocumentDialog::default(),
            filter_dialog: FilterDialog::default(),
            show_about: false,
            show_shortcuts: false,
            rebinding: None,
            confirm_close: false,
            applied_theme: None,
        }
    }

    /// The editor state, for tests and embedding.
    pub fn state(&self) -> &EditorState {
        &self.state
    }

    /// The editor state, mutably.
    pub fn state_mut(&mut self) -> &mut EditorState {
        &mut self.state
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme == Some(self.state.theme) {
            return;
        }
        ctx.set_visuals(self.state.theme.visuals());
        self.applied_theme = Some(self.state.theme);
    }

    fn set_workspace(&mut self, workspace: Workspace) {
        self.state.set_workspace(workspace);
        self.dock = layout_for(workspace);
    }

    /// Open a project or an image, choosing by extension.
    fn open_path(&mut self, path: PathBuf) {
        let is_project = path
            .extension()
            .map(|e| e.eq_ignore_ascii_case(project::EXTENSION))
            .unwrap_or(false);
        let result = if is_project {
            self.state.open_project(&path)
        } else {
            self.state.open_image(&path)
        };
        if let Err(error) = result {
            self.state.report_error("Open", &error);
        }
    }

    // ------------------------------------------------------------ file menu

    fn pick_and_open(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Aether project", &[project::EXTENSION])
            .add_filter("Images", &["png", "jpg", "jpeg", "webp", "tiff", "bmp", "gif"])
            .pick_file();
        if let Some(path) = picked {
            self.open_path(path);
        }
    }

    fn pick_and_save(&mut self) {
        let default = self
            .state
            .path
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("{}.{}", self.state.doc.name, project::EXTENSION)));
        let picked = rfd::FileDialog::new()
            .add_filter("Aether project", &[project::EXTENSION])
            .set_file_name(
                default
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            )
            .save_file();
        if let Some(path) = picked {
            if let Err(error) = self.state.save_as(path) {
                self.state.report_error("Save", &error);
            }
        }
    }

    fn pick_and_export(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("WebP", &["webp"])
            .set_file_name(format!("{}.png", self.state.doc.name))
            .save_file();
        if let Some(path) = picked {
            if let Err(error) = self.state.export_png(path) {
                self.state.report_error("Export", &error);
            }
        }
    }

    /// Run an action, opening a file dialog for the ones that need one.
    fn run(&mut self, action: Action) {
        if self.state.handle_action(action) {
            return;
        }
        match action {
            Action::NewDocument => self.new_dialog.open = true,
            Action::OpenDocument => self.pick_and_open(),
            Action::Save | Action::SaveAs => self.pick_and_save(),
            Action::ExportPng => self.pick_and_export(),
            _ => {}
        }
    }

    // --------------------------------------------------------------- widgets

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            let lang = self.state.language;
            ui.menu_button(lang.tr("menu.file"), |ui| {
                for (key, action) in [
                    ("menu.file.new", Action::NewDocument),
                    ("menu.file.open", Action::OpenDocument),
                    ("menu.file.save", Action::Save),
                    ("menu.file.save_as", Action::SaveAs),
                    ("menu.file.export_png", Action::ExportPng),
                ] {
                    if self.menu_item(ui, key, action) {
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(lang.tr("menu.file.quit")).clicked() {
                    self.state.quit_requested = true;
                    ui.close();
                }
            });

            ui.menu_button(lang.tr("menu.edit"), |ui| {
                for (key, action) in [
                    ("menu.edit.undo", Action::Undo),
                    ("menu.edit.redo", Action::Redo),
                    ("menu.edit.clear", Action::ClearLayer),
                    ("menu.edit.select_all", Action::SelectAll),
                    ("menu.edit.deselect", Action::Deselect),
                    ("menu.edit.invert_selection", Action::InvertSelection),
                ] {
                    if self.menu_item(ui, key, action) {
                        ui.close();
                    }
                }
                ui.separator();
                if self.menu_item(ui, "menu.edit.transform", Action::ToolTransform) {
                    ui.close();
                }
                let pending = self.state.has_pending_tool_edit();
                if ui
                    .add_enabled(pending, egui::Button::new(lang.tr("menu.edit.apply")))
                    .clicked()
                {
                    self.state.commit_tool();
                    ui.close();
                }
            });

            ui.menu_button(lang.tr("menu.layer"), |ui| {
                for (key, action) in [
                    ("layer.add", Action::AddLayer),
                    ("layer.duplicate", Action::DuplicateLayer),
                    ("layer.delete", Action::DeleteLayer),
                    ("layer.merge_down", Action::MergeDown),
                ] {
                    if self.menu_item(ui, key, action) {
                        ui.close();
                    }
                }
                ui.separator();
                ui.menu_button(lang.tr("menu.layer.new_adjustment"), |ui| {
                    for adjustment in Adjustment::presets() {
                        if ui.button(adjustment.name()).clicked() {
                            if let Err(error) = self.state.add_adjustment_layer(adjustment) {
                                self.state.report_error("Adjustment layer", &error);
                            }
                            ui.close();
                        }
                    }
                });
            });

            ui.menu_button(lang.tr("menu.filter"), |ui| {
                for (label, kind) in [
                    ("menu.filter.blur", EffectKind::Blur { sigma: 4.0 }),
                    (
                        "menu.filter.sharpen",
                        EffectKind::Sharpen {
                            amount: 0.6,
                            radius: 1.5,
                        },
                    ),
                    (
                        "menu.filter.motion_blur",
                        EffectKind::MotionBlur {
                            angle: 0.0,
                            distance: 16.0,
                        },
                    ),
                    (
                        "menu.filter.grain",
                        EffectKind::Grain {
                            amount: 0.15,
                            seed: 1,
                            monochrome: true,
                        },
                    ),
                ] {
                    if ui.button(lang.tr(label)).clicked() {
                        self.filter_dialog = FilterDialog { open: true, kind };
                        ui.close();
                    }
                }
            });

            ui.menu_button(lang.tr("menu.image"), |ui| {
                if ui.button(lang.tr("menu.image.resize")).clicked() {
                    // The properties panel owns canvas resizing; make sure it is visible.
                    self.focus_panel(PanelKind::Properties);
                    ui.close();
                }
            });

            ui.menu_button(lang.tr("menu.view"), |ui| {
                for (key, action) in [
                    ("menu.view.zoom_in", Action::ZoomIn),
                    ("menu.view.zoom_out", Action::ZoomOut),
                    ("menu.view.fit", Action::ZoomFit),
                    ("menu.view.reset", Action::ZoomReset),
                    ("menu.view.rotate_left", Action::RotateLeft),
                    ("menu.view.rotate_right", Action::RotateRight),
                    ("menu.view.rotate_reset", Action::ResetRotation),
                    ("menu.view.mirror", Action::MirrorView),
                    ("menu.view.pixel_grid", Action::TogglePixelGrid),
                ] {
                    if self.menu_item(ui, key, action) {
                        ui.close();
                    }
                }
            });

            ui.menu_button(lang.tr("menu.workspace"), |ui| {
                for workspace in Workspace::ALL {
                    let selected = self.state.workspace == workspace;
                    if ui.selectable_label(selected, lang.tr(workspace.key())).clicked() {
                        self.set_workspace(workspace);
                        ui.close();
                    }
                }
            });

            ui.menu_button(lang.tr("menu.window"), |ui| {
                ui.menu_button(lang.tr("menu.window.theme"), |ui| {
                    for theme in Theme::ALL {
                        let selected = self.state.theme == theme;
                        if ui.selectable_label(selected, lang.tr(theme.key())).clicked() {
                            self.state.theme = theme;
                            ui.close();
                        }
                    }
                });
                ui.menu_button(lang.tr("menu.language"), |ui| {
                    for language in Language::ALL {
                        let selected = self.state.language == language;
                        if ui.selectable_label(selected, language.native_name()).clicked() {
                            self.state.language = language;
                            ui.close();
                        }
                    }
                });
            });

            ui.menu_button(lang.tr("menu.help"), |ui| {
                if ui.button(lang.tr("menu.help.shortcuts")).clicked() {
                    self.show_shortcuts = true;
                    ui.close();
                }
                if ui.button(lang.tr("menu.help.about")).clicked() {
                    self.show_about = true;
                    ui.close();
                }
            });
        });
    }

    /// A menu entry showing its keyboard shortcut; returns true when clicked.
    fn menu_item(&mut self, ui: &mut egui::Ui, key: &str, action: Action) -> bool {
        let label = self.state.language.tr(key);
        let shortcut = self.state.shortcuts.display(action);
        let enabled = match action {
            Action::Undo => self.state.history.can_undo(),
            Action::Redo => self.state.history.can_redo(),
            _ => true,
        };
        let response = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut));
        if response.clicked() {
            self.run(action);
            return true;
        }
        false
    }

    /// Make sure a panel is present, adding it if the user closed it.
    fn focus_panel(&mut self, kind: PanelKind) {
        if let Some(path) = self.dock.find_tab(&kind) {
            let _ = self.dock.set_active_tab(path);
        } else {
            self.dock.push_to_focused_leaf(kind);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let tool = self.state.tr(self.state.tools.active_id().label_key());
            ui.label(tool);
            ui.separator();
            ui.label(format!("{:.0}%", self.state.viewport.zoom * 100.0));
            ui.separator();
            ui.label(format!("{} × {}", self.state.doc.width, self.state.doc.height));
            ui.separator();
            let layer = self
                .state
                .doc
                .active()
                .map(|l| l.name.clone())
                .unwrap_or_else(|| "—".into());
            ui.label(layer);
            ui.separator();
            let message = if self.state.status.is_empty() {
                self.state.tr("status.ready").to_string()
            } else {
                self.state.status.clone()
            };
            ui.label(message);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.state.has_unsaved_changes() {
                    ui.label("●").on_hover_text("Unsaved changes");
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.new_document_dialog(ctx);
        self.filter_dialog(ctx);
        self.about_window(ctx);
        self.shortcuts_window(ctx);
        self.close_confirmation(ctx);
    }

    fn new_document_dialog(&mut self, ctx: &egui::Context) {
        if !self.new_dialog.open {
            return;
        }
        let lang = self.state.language;
        let mut open = true;
        let mut create = false;
        let mut cancel = false;
        egui::Window::new(lang.tr("dialog.new_document"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("new-doc").num_columns(2).show(ui, |ui| {
                    ui.label(lang.tr("dialog.name"));
                    ui.text_edit_singleline(&mut self.new_dialog.name);
                    ui.end_row();
                    ui.label(lang.tr("dialog.width"));
                    ui.add(egui::DragValue::new(&mut self.new_dialog.width).range(1..=16384));
                    ui.end_row();
                    ui.label(lang.tr("dialog.height"));
                    ui.add(egui::DragValue::new(&mut self.new_dialog.height).range(1..=16384));
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    for (label, w, h) in [
                        ("HD", 1920u32, 1080u32),
                        ("4K", 3840, 2160),
                        ("A4 300dpi", 2480, 3508),
                        ("Sprite 64", 64, 64),
                    ] {
                        if ui.small_button(label).clicked() {
                            self.new_dialog.width = w;
                            self.new_dialog.height = h;
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(lang.tr("dialog.create")).clicked() {
                        create = true;
                    }
                    if ui.button(lang.tr("dialog.cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if create {
            let dialog = &self.new_dialog;
            let (w, h, name) = (dialog.width, dialog.height, dialog.name.clone());
            self.state.new_document(w, h, name);
            self.new_dialog.open = false;
        } else {
            self.new_dialog.open = open;
        }
    }

    /// Parameters for a destructive filter, applied on confirmation.
    fn filter_dialog(&mut self, ctx: &egui::Context) {
        if !self.filter_dialog.open {
            return;
        }
        let lang = self.state.language;
        let mut open = true;
        let mut cancel = false;
        let mut apply = false;
        let title = crate::panels::effect_name(&self.filter_dialog.kind);
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                crate::panels::effect_editor(ui, &mut self.filter_dialog.kind, "filter-dialog");
                if self.state.doc.selection.is_active() {
                    ui.label("Applies inside the selection.");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(lang.tr("dialog.apply")).clicked() {
                        apply = true;
                    }
                    if ui.button(lang.tr("dialog.cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if apply {
            let kind = self.filter_dialog.kind.clone();
            if let Err(error) = self.state.apply_filter(kind) {
                self.state.report_error("Filter", &error);
            }
            self.filter_dialog.open = false;
        } else if cancel {
            self.filter_dialog.open = false;
        } else {
            self.filter_dialog.open = open;
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut open = self.show_about;
        egui::Window::new(self.state.tr("menu.help.about"))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.heading("Aether Canvas");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("An integrated 2D creative environment.");
                ui.separator();
                ui.label("Project files are plain ZIP archives holding JSON and PNG,");
                ui.label("so your artwork stays readable with ordinary tools.");
            });
        self.show_about = open;
    }

    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = self.show_shortcuts;
        let mut pending: Option<(Action, Binding)> = None;
        egui::Window::new(self.state.tr("menu.help.shortcuts"))
            .open(&mut open)
            .default_height(420.0)
            .show(ctx, |ui| {
                if ui.button("Restore defaults").clicked() {
                    self.state.shortcuts.reset();
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("shortcut-grid")
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            for action in Action::ALL {
                                ui.label(action.label());
                                let display = self.state.shortcuts.display(action);
                                let waiting = self.rebinding == Some(action);
                                let text = if waiting {
                                    "press a key…".to_string()
                                } else {
                                    display
                                };
                                if ui.button(text).clicked() {
                                    self.rebinding = Some(action);
                                }
                                ui.end_row();
                            }
                        });
                });

                // Capture the next key press for the action being rebound.
                if let Some(action) = self.rebinding {
                    let captured = ui.input(|i| {
                        i.events.iter().find_map(|event| match event {
                            egui::Event::Key {
                                key,
                                pressed: true,
                                modifiers,
                                ..
                            } => Some(Binding {
                                key: *key,
                                modifiers: *modifiers,
                            }),
                            _ => None,
                        })
                    });
                    if let Some(binding) = captured {
                        if binding.key != egui::Key::Escape {
                            pending = Some((action, binding));
                        }
                        self.rebinding = None;
                    }
                }
            });
        if let Some((action, binding)) = pending {
            self.state.shortcuts.rebind(action, binding);
        }
        self.show_shortcuts = open;
    }

    fn close_confirmation(&mut self, ctx: &egui::Context) {
        if !self.confirm_close {
            return;
        }
        let lang = self.state.language;
        let mut open = true;
        let mut cancel = false;
        let mut decision: Option<bool> = None;
        egui::Window::new(lang.tr("dialog.unsaved_title"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(lang.tr("dialog.unsaved_body"));
                ui.horizontal(|ui| {
                    if ui.button(lang.tr("menu.file.save")).clicked() {
                        decision = Some(true);
                    }
                    if ui.button(lang.tr("dialog.unsaved_discard")).clicked() {
                        decision = Some(false);
                    }
                    if ui.button(lang.tr("dialog.cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        match decision {
            Some(true) => {
                if self.state.path.is_some() {
                    if let Err(error) = self.state.save() {
                        self.state.report_error("Save", &error);
                    }
                } else {
                    self.pick_and_save();
                }
                if !self.state.has_unsaved_changes() {
                    self.confirm_close = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Some(false) => {
                self.confirm_close = false;
                self.state.history.mark_saved();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => {
                if !open {
                    self.confirm_close = false;
                }
            }
        }
    }
}

impl eframe::App for AetherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_theme(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.state.title()));

        if let Some(action) = self.state.shortcuts.consume(&ctx) {
            self.run(action);
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let mut viewer = PanelViewer {
                    state: &mut self.state,
                    canvas: &mut self.canvas,
                };
                DockArea::new(&mut self.dock)
                    .style(Style::from_egui(ui.style().as_ref()))
                    .show_inside(ui, &mut viewer);
            });

        self.dialogs(&ctx);

        // Closing with unsaved work asks first.
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if (close_requested || self.state.quit_requested) && self.state.has_unsaved_changes() {
            self.state.quit_requested = false;
            self.confirm_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if self.state.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;

    #[test]
    fn the_app_starts_with_a_document_and_a_layout() {
        let app = AetherApp::with_document(Document::new(32, 32, "test"));
        assert_eq!(app.state().doc.layer_count(), 1);
        assert!(app.dock.iter_all_tabs().count() >= 2);
    }

    #[test]
    fn switching_workspace_rebuilds_the_layout_but_keeps_the_document() {
        let mut app = AetherApp::with_document(Document::new(32, 32, "test"));
        let active = app.state().doc.active_layer;
        if let Some(pm) = app
            .state_mut()
            .doc
            .layers
            .get_mut(active)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::WHITE);
        }
        app.set_workspace(Workspace::PixelArt);
        assert_eq!(app.state().workspace, Workspace::PixelArt);
        assert!(app.state().show_pixel_grid);
        assert_eq!(
            app.state()
                .doc
                .layers
                .get(app.state().doc.active_layer)
                .and_then(|l| l.pixmap())
                .map(|p| p.get(1, 1)),
            Some(Rgba8::WHITE),
            "switching workspace must not touch the artwork"
        );
    }

    #[test]
    fn non_dialog_actions_run_without_a_window() {
        let mut app = AetherApp::with_document(Document::new(32, 32, "test"));
        app.run(Action::AddLayer);
        assert_eq!(app.state().doc.layer_count(), 2);
        app.run(Action::Undo);
        assert_eq!(app.state().doc.layer_count(), 1);
    }
}
