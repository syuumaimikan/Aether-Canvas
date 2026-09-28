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
        // Japanese (and other CJK) UI text needs a system font.
        crate::fonts::install_cjk_fallback(&cc.egui_ctx);
        // Rig poses draw on the GPU when the document allows it exactly.
        if let Some(render_state) = cc.wgpu_render_state.as_ref() {
            app.canvas
                .set_gpu_preview(crate::gpu_preview::GpuPreview::from_render_state(render_state));
        }
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

    /// Switch workspace: rebuild the panel layout and apply its defaults.
    pub fn set_workspace(&mut self, workspace: Workspace) {
        self.state.set_workspace(workspace);
        self.dock = layout_for(workspace);
    }

    /// Open a project, a Live2D model or an image, choosing by extension.
    fn open_path(&mut self, path: PathBuf) {
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let result = if extension == project::EXTENSION {
            self.state.open_project(&path)
        } else if file_name.ends_with(".model3.json") || extension == "moc3" || path.is_dir() {
            // A .moc3 or a folder: the model settings file beside or in it.
            let target = if extension == "moc3" {
                path.parent().map(PathBuf::from).unwrap_or_default()
            } else {
                path.clone()
            };
            self.state.open_live2d(&target).map(|_| ())
        } else if extension == "psd" {
            self.state.open_psd(&path)
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
            .add_filter(
                "Aether project, Live2D model, Photoshop or image",
                &[
                    project::EXTENSION,
                    "json",
                    "moc3",
                    "psd",
                    "png",
                    "jpg",
                    "jpeg",
                    "webp",
                    "tiff",
                    "bmp",
                    "gif",
                ],
            )
            .add_filter("Aether project", &[project::EXTENSION])
            .add_filter("Live2D model (.model3.json)", &["json", "moc3"])
            .add_filter("Photoshop", &["psd"])
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
                if ui
                    .button(lang.tr("menu.file.open_live2d"))
                    .on_hover_text(lang.tr("menu.file.open_live2d_hint"))
                    .clicked()
                {
                    let picked = rfd::FileDialog::new()
                        .add_filter("Live2D model (.model3.json)", &["json", "moc3"])
                        .pick_file();
                    if let Some(path) = picked {
                        self.open_path(path);
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button(lang.tr("menu.file.export_psd")).clicked() {
                    let picked = rfd::FileDialog::new()
                        .add_filter("Photoshop", &["psd"])
                        .set_file_name(format!("{}.psd", self.state.doc.name))
                        .save_file();
                    if let Some(path) = picked {
                        if let Err(error) = self.state.export_psd(path) {
                            self.state.report_error("Export PSD", &error);
                        }
                    }
                    ui.close();
                }
                if ui
                    .button(lang.tr("menu.file.export_model"))
                    .on_hover_text(lang.tr("menu.file.export_model_hint"))
                    .clicked()
                {
                    self.state.export_runtime_model_via_dialog();
                    ui.close();
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

            ui.menu_button(lang.tr("menu.rig"), |ui| self.rig_menu(ui));

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

    /// The Rig menu: the rigging operations most used, one click away.
    fn rig_menu(&mut self, ui: &mut egui::Ui) {
        let lang = self.state.language;
        let state = &mut self.state;
        let mut result: Option<(&str, aether_core::Result<()>)> = None;
        if ui
            .button(format!("{} {}", crate::icons::WAND, lang.tr("rig.auto_rig")))
            .on_hover_text(lang.tr("rig.auto_rig_hint"))
            .clicked()
        {
            result = Some(("Auto rig", state.auto_rig().map(|_| ())));
            ui.close();
        }
        ui.separator();
        if ui.button(lang.tr("rig.mesh_layer")).clicked() {
            let layer = state.doc.active_layer;
            result = Some(("Mesh", state.mesh_layer(layer)));
            ui.close();
        }
        if ui.button(lang.tr("rig.mesh_all")).clicked() {
            result = Some(("Mesh", state.mesh_all_layers(None).map(|_| ())));
            ui.close();
        }
        if ui.button(lang.tr("rig.add_warp")).clicked() {
            result = Some(("Warp", state.add_warp_deformer("Warp")));
            ui.close();
        }
        if ui.button(lang.tr("rig.add_rotation")).clicked() {
            result = Some(("Rotation", state.add_rotation_deformer("Rotation")));
            ui.close();
        }
        ui.separator();
        if ui.button(lang.tr("param.standard")).clicked() {
            result = Some(("Parameters", state.add_standard_parameters().map(|_| ())));
            ui.close();
        }
        if ui.button(lang.tr("dyn.standard")).clicked() {
            result = Some(("Physics", state.add_standard_dynamics().map(|_| ())));
            ui.close();
        }
        ui.separator();
        for (key, action) in [
            ("menu.rig.play", Action::PlayPause),
            ("menu.rig.simulate", Action::ToggleSimulation),
            ("menu.rig.reset_pose", Action::ResetPose),
            ("menu.rig.key_all", Action::KeyAll),
        ] {
            let label = lang.tr(key);
            let shortcut = self.state.shortcuts.display(action);
            if ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked() {
                self.run(action);
                ui.close();
            }
        }
        ui.separator();
        let state = &mut self.state;
        for (key, kind) in [
            ("timeline.export_gif", crate::rigging::ExportKind::Gif),
            ("timeline.export_apng", crate::rigging::ExportKind::Apng),
            ("timeline.export_png", crate::rigging::ExportKind::PngSequence),
            ("timeline.export_sheet", crate::rigging::ExportKind::SpriteSheet),
        ] {
            if ui.button(lang.tr(key)).clicked() {
                state.export_animation_via_dialog(kind);
                ui.close();
            }
        }
        if let Some((context, Err(error))) = result {
            self.state.report_error(context, &error);
        }
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
                    ui.label(crate::icons::UNSAVED).on_hover_text("Unsaved changes");
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
        self.draw(ui);
    }
}

impl AetherApp {
    /// Draw one frame of the whole application into `ui`.
    ///
    /// Separate from [`eframe::App::ui`] so the complete interface can be
    /// driven headlessly — the smoke tests below render every workspace and
    /// every rig inspector through here without a window or a GPU.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.apply_theme(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.state.title()));

        if let Some(action) = self.state.shortcuts.consume(&ctx) {
            self.run(action);
        }

        // Advance physics, behaviours and playback; keep repainting while
        // anything is moving.
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if self.state.tick_rig(dt) {
            ctx.request_repaint();
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

    // ------------------------------------------------------------------
    // Headless smoke tests: the whole interface, every workspace and every
    // rig inspector, rendered through egui without a window.

    use aether_core::math::{IRect, Vec2};
    use aether_document::rig::RigNode;

    /// A document with painted layers and a rig touching every feature.
    fn rigged_app() -> AetherApp {
        let mut doc = Document::new(256, 256, "rigged");
        let face = doc.active_layer;
        if let Some(pm) = doc.layers.get_mut(face).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(64, 48, 128, 140), Rgba8::rgb(250, 220, 200));
        }
        let mut app = AetherApp::with_document(doc);
        let state = app.state_mut();
        let hair = state.add_layer().expect("layer");
        if let Some(pm) = state.doc.layers.get_mut(hair).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(56, 32, 144, 60), Rgba8::rgb(90, 60, 140));
        }
        state.mesh_all_layers(None).expect("mesh");
        state.add_standard_parameters().expect("params");
        state.rig.tool.selection = Some(RigNode::Mesh(face));
        state.add_warp_deformer("Head").expect("warp");
        let x = state.doc.rig.parameter_named("AngleX").expect("x").id;
        let y = state.doc.rig.parameter_named("AngleY").expect("y").id;
        state.generate_head_turn(x, Some(y)).expect("turn");
        state.rig.tool.selection = Some(RigNode::Mesh(hair));
        let sway = state.doc.rig.parameter_named("HairFront").expect("hair").id;
        state
            .generate_sway(sway, 10.0, aether_document::rig::generate::Anchor::Top)
            .expect("sway");
        state.add_rotation_deformer("Tilt").expect("rotation");
        state
            .add_bone(Vec2::new(128.0, 200.0), Vec2::new(128.0, 150.0), None)
            .expect("bone");
        let Some(RigNode::Bone(upper)) = state.rig.tool.selection else {
            panic!("bone")
        };
        state
            .add_bone(Vec2::new(128.0, 150.0), Vec2::new(128.0, 110.0), Some(upper))
            .expect("bone");
        state
            .add_bone(Vec2::new(160.0, 120.0), Vec2::new(170.0, 120.0), None)
            .expect("target");
        state.add_standard_dynamics().expect("physics");
        let body = state.doc.rig.parameter_named("BodyAngleX").expect("body").id;
        state.add_driver(body, "AngleX * 0.3").expect("driver");
        let m = state.add_motion("Idle").expect("motion");
        state.rig.playhead = 0.5;
        state.set_parameter_value(x, 20.0).expect("key");
        state.rig.selected_key = Some((x, 0));
        let _ = m;
        app
    }

    fn frame(app: &mut AetherApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    }

    #[test]
    fn every_workspace_and_inspector_renders_without_panicking() {
        let ctx = egui::Context::default();
        let mut app = rigged_app();
        let nodes: Vec<Option<RigNode>> = {
            let rig = &app.state().doc.rig;
            let mut nodes = vec![None];
            nodes.extend(rig.meshes.iter().map(|m| Some(RigNode::Mesh(m.layer))));
            nodes.extend(rig.deformers.iter().map(|d| Some(RigNode::Deformer(d.id))));
            nodes.extend(rig.bones.iter().map(|b| Some(RigNode::Bone(b.id))));
            nodes
        };
        for workspace in Workspace::ALL {
            app.set_workspace(workspace);
            for node in &nodes {
                app.state_mut().rig.tool.selection = *node;
                frame(&mut app, &ctx, Vec::new());
            }
        }
        // Dialogs and modes.
        let x = app.state().doc.rig.parameters[0].clone();
        app.state_mut().rig.editing_parameter = Some(x.clone());
        app.state_mut().rig.editing_driver = Some((x.id, "AngleX * 2 +".into()));
        app.state_mut().rig.playing = true;
        app.state_mut().rig.simulate = true;
        app.state_mut().rig.follow_pointer = true;
        for tool in [
            crate::tools::ToolId::Mesh,
            crate::tools::ToolId::Deform,
            crate::tools::ToolId::Bone,
        ] {
            app.state_mut().select_tool(tool);
            frame(&mut app, &ctx, Vec::new());
        }
        assert!(app.state().rig.playhead > 0.0);
    }

    #[test]
    fn dragging_on_the_canvas_with_the_deform_tool_edits_the_keyform() {
        let ctx = egui::Context::default();
        let mut app = rigged_app();
        app.set_workspace(Workspace::Rigging);
        let state = app.state_mut();
        state.rig.animate = false;
        state.rig.motion = None;
        let face = state
            .doc
            .rig
            .meshes
            .iter()
            .find(|m| m.name == "Layer 1")
            .map(|m| m.layer)
            .expect("face mesh");
        state.rig.tool.selection = Some(RigNode::Mesh(face));
        state.rig.tool.radius = 400.0;
        state.reset_pose();
        state.select_tool(crate::tools::ToolId::Deform);
        // Lay out once, then find where the canvas put the document centre.
        frame(&mut app, &ctx, Vec::new());
        frame(&mut app, &ctx, Vec::new());
        let before = app.state().history.entries().len();
        let viewport = app.state().viewport;
        let centre = viewport.doc_to_screen(Vec2::new(128.0, 128.0));
        // The canvas tab sits right of the tools column; find its origin by
        // probing: the viewport maps into the canvas rect, whose left edge is
        // the tools column width.
        let origin = egui::pos2(1600.0 * 0.13 + 6.0, 24.0 + 6.0);
        let press = egui::pos2(origin.x + centre.x, origin.y + centre.y);
        let pointer = |pos: egui::Pos2, pressed: Option<bool>| {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            events
        };
        frame(&mut app, &ctx, pointer(press, None));
        frame(&mut app, &ctx, pointer(press, Some(true)));
        for step in 1..=6 {
            let p = press + egui::vec2(step as f32 * 6.0, 0.0);
            frame(&mut app, &ctx, pointer(p, None));
        }
        frame(
            &mut app,
            &ctx,
            pointer(press + egui::vec2(36.0, 0.0), Some(false)),
        );
        frame(&mut app, &ctx, Vec::new());
        let after = app.state().history.entries().len();
        assert_eq!(
            after,
            before + 1,
            "one undoable deform step: {:?}",
            app.state().status
        );
        let moved = app
            .state()
            .doc
            .rig
            .mesh(face)
            .map(|m| {
                m.keyforms
                    .forms
                    .iter()
                    .any(|f| f.offsets.iter().any(|o| o.x > 1.0))
            })
            .unwrap_or(false);
        assert!(moved, "the drag pushed vertices right");
    }
}
