//! Editor state.
//!
//! [`EditorState`] is everything the application knows: the document, the undo
//! history, the render cache, the view, the tools and the user's settings. The
//! widgets in [`crate::panels`] read and drive it, but hold no state of their
//! own, which keeps every operation reachable without a window — the tests in
//! this module drive the same entry points the menus do.

use crate::i18n::Language;
use crate::shortcuts::{Action, ShortcutMap};
use crate::theme::Theme;
use crate::tools::{ToolBox, ToolContext, ToolEvent, ToolId, ToolSettings};
use aether_core::blend::BlendMode;
use aether_core::color::{Rgba, Rgba8};
use aether_core::math::IRect;
use aether_core::{AetherError, LayerId, Result};
use aether_document::command::SetAdjustmentCommand;
use aether_document::command::{
    AddLayerCommand, DeleteLayerCommand, LayerProperty, MoveLayerCommand, RegionEdit, ResizeCanvasCommand,
    SetLayerEffectsCommand, SetLayerMaskCommand, SetLayerPropertyCommand, SetSelectionCommand, Transaction,
};
use aether_document::layer::{Layer, LayerContent};
use aether_document::selection::{Selection, SelectionMode};
use aether_document::{Document, History};
use aether_io::project;
use aether_io::{ExportSettings, ImageFormat};
use aether_raster::adjust::Adjustment;
use aether_raster::composite::{composite_pixmap, CompositeOptions};
use aether_raster::effect::{EffectKind, LayerEffect};
use aether_raster::transform::Interpolation;
use aether_raster::{BrushPreset, Mask, Pixmap};
use aether_render::{Compositor, RenderCache, Viewport};
use std::path::{Path, PathBuf};

/// A named arrangement of panels and defaults for a kind of work.
///
/// Switching workspace never changes the document — only the layout and a few
/// tool defaults. The pixel-art workspace, for example, turns on the pixel grid
/// and switches the canvas to nearest-neighbour sampling so pixels stay crisp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Workspace {
    /// Painting and illustration.
    #[default]
    Illustration,
    /// Pixel art: pixel grid on, hard square brush, nearest-neighbour view.
    PixelArt,
    /// Compositing and review: history and properties to hand.
    Compositing,
    /// Rigging: meshes, deformers, bones, parameters and physics.
    Rigging,
    /// Animation: the timeline, parameters and live preview.
    Animation,
}

impl Workspace {
    /// All workspaces, in menu order.
    pub const ALL: [Workspace; 5] = [
        Workspace::Illustration,
        Workspace::PixelArt,
        Workspace::Compositing,
        Workspace::Rigging,
        Workspace::Animation,
    ];

    /// Translation key for the workspace name.
    pub fn key(self) -> &'static str {
        match self {
            Workspace::Illustration => "workspace.illustration",
            Workspace::PixelArt => "workspace.pixel_art",
            Workspace::Compositing => "workspace.compositing",
            Workspace::Rigging => "workspace.rigging",
            Workspace::Animation => "workspace.animation",
        }
    }
}

/// Where a stroke's pressure values come from.
///
/// Windowing stacks differ in what they expose. `winit` forwards a touch
/// device's force, which is what a pen reports on iPadOS and on Windows/Wayland
/// digitisers that present as touch — so [`PressureSource::Device`] is real
/// pressure where the platform provides it. Tablets that only speak Wintab or
/// the Windows Ink API do not reach us yet, so [`PressureSource::Speed`] gives
/// a usable taper from stroke velocity, and [`PressureSource::Off`] disables
/// dynamics entirely.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PressureSource {
    /// Use the device's reported force, falling back to full pressure.
    #[default]
    Device,
    /// Derive pressure from how fast the pointer is moving.
    Speed,
    /// Always full pressure.
    Off,
}

impl PressureSource {
    /// All sources, in menu order.
    pub const ALL: [PressureSource; 3] = [PressureSource::Device, PressureSource::Speed, PressureSource::Off];

    /// Translation key for the source's name.
    pub fn key(self) -> &'static str {
        match self {
            PressureSource::Device => "pressure.device",
            PressureSource::Speed => "pressure.speed",
            PressureSource::Off => "pressure.off",
        }
    }

    /// Resolve a pressure value for one input sample.
    ///
    /// `device` is the force the platform reported, if any; `speed` is the
    /// pointer's speed in document pixels per second.
    pub fn resolve(self, device: Option<f32>, speed: f32) -> f32 {
        match self {
            PressureSource::Device => device.unwrap_or(1.0).clamp(0.0, 1.0),
            PressureSource::Speed => {
                // Fast strokes read as light. The knee is around 900 px/s,
                // which is a brisk but not frantic pen movement.
                let t = (speed / 900.0).clamp(0.0, 1.0);
                (1.0 - 0.75 * t).clamp(0.25, 1.0)
            }
            PressureSource::Off => 1.0,
        }
    }
}

/// The whole application state.
pub struct EditorState {
    /// The document being edited.
    pub doc: Document,
    /// Undo history for `doc`.
    pub history: History,
    /// Compositor used for the canvas, thumbnails and export.
    pub compositor: Compositor,
    /// Incremental composite cache.
    pub cache: RenderCache,
    /// Canvas view transform.
    pub viewport: Viewport,
    /// Available tools and the active one.
    pub tools: ToolBox,
    /// Settings the tool-options panel edits.
    pub tool_settings: ToolSettings,
    /// Where stroke pressure comes from.
    pub pressure_source: PressureSource,
    /// The brush the paint tools use.
    pub brush: BrushPreset,
    /// Saved brush presets.
    pub brush_presets: Vec<BrushPreset>,
    /// Foreground colour.
    pub primary: Rgba,
    /// Background colour.
    pub secondary: Rgba,
    /// Swatches shown in the colour panel.
    pub palette: Vec<Rgba8>,
    /// Recently used colours, most recent first.
    pub recent_colors: Vec<Rgba8>,
    /// Flood fill / magic wand tolerance.
    pub tolerance: f32,
    /// Whether fill and wand look at the composite instead of one layer.
    pub sample_all_layers: bool,
    /// How new selections combine with the existing one.
    pub selection_mode: SelectionMode,
    /// Where the document was loaded from or last saved to.
    pub path: Option<PathBuf>,
    /// Message for the status bar.
    pub status: String,
    /// UI language.
    pub language: Language,
    /// UI theme.
    pub theme: Theme,
    /// Active workspace.
    pub workspace: Workspace,
    /// Whether to draw the per-pixel grid when zoomed in.
    pub show_pixel_grid: bool,
    /// Sampling used to display the canvas.
    pub view_interpolation: Interpolation,
    /// Key bindings.
    pub shortcuts: ShortcutMap,
    /// Set when the application should close.
    pub quit_requested: bool,
    /// Rigging and animation state: selection, timeline, live preview.
    pub rig: crate::rigging::RigEditor,
    /// The result of the last export that has something to say, shown in
    /// a window until dismissed.
    pub export_report: Option<ExportReport>,
    /// The sample library window.
    pub library: crate::library::SampleLibrary,
    /// An archive holding several things, waiting for a choice.
    pub archive_choice: Option<crate::library::ArchiveChoice>,
    /// A file being opened on a worker thread.
    pub pending_open: Option<crate::library::PendingOpen>,
    /// The tutorials window.
    pub tutorial: crate::tutorial::TutorialState,
}

/// What an export did, for the report window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportReport {
    /// Where it went and what was written.
    pub summary: String,
    /// What was approximated or left out.
    pub notes: Vec<String>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new(Document::new(1920, 1080, "Untitled"))
    }
}

impl EditorState {
    /// State around an existing document.
    pub fn new(doc: Document) -> Self {
        let presets = BrushPreset::builtin();
        let brush = presets.first().cloned().unwrap_or_default();
        Self {
            doc,
            history: History::default(),
            compositor: Compositor::new(),
            cache: RenderCache::new(),
            viewport: Viewport::default(),
            tools: ToolBox::standard(),
            tool_settings: ToolSettings::default(),
            pressure_source: PressureSource::default(),
            brush,
            brush_presets: presets,
            primary: Rgba::BLACK,
            secondary: Rgba::WHITE,
            palette: default_palette(),
            recent_colors: Vec::new(),
            tolerance: 0.1,
            sample_all_layers: false,
            selection_mode: SelectionMode::Replace,
            path: None,
            status: String::new(),
            language: Language::default(),
            theme: Theme::default(),
            workspace: Workspace::default(),
            show_pixel_grid: false,
            view_interpolation: Interpolation::Bilinear,
            shortcuts: ShortcutMap::standard(),
            quit_requested: false,
            rig: crate::rigging::RigEditor::default(),
            export_report: None,
            library: Default::default(),
            archive_choice: None,
            pending_open: None,
            tutorial: Default::default(),
        }
    }

    /// Translate a UI string.
    pub fn tr(&self, key: &str) -> &'static str {
        self.language.tr(key)
    }

    /// Window title: document name, unsaved marker and file path.
    pub fn title(&self) -> String {
        let dirty = if self.history.has_unsaved_changes() {
            "*"
        } else {
            ""
        };
        match &self.path {
            Some(path) => format!("{}{} — Aether Canvas", dirty, path.display()),
            None => format!("{}{} — Aether Canvas", dirty, self.doc.name),
        }
    }

    /// Re-composite whatever changed and return the region that moved.
    pub fn refresh(&mut self) -> IRect {
        self.cache.update(&mut self.doc, &self.compositor)
    }

    /// The current composite.
    pub fn composite(&self) -> &Pixmap {
        self.cache.image()
    }

    /// Set the status line.
    pub fn report(&mut self, message: impl Into<String>) {
        self.status = message.into();
    }

    /// Report an error to the status line and to the log.
    pub fn report_error(&mut self, context: &str, error: &AetherError) {
        tracing::error!("{context}: {error}");
        self.status = format!("{context}: {error}");
    }

    // ---------------------------------------------------------------- tools

    /// Push the tool-options settings into the active tool.
    ///
    /// Called once per frame so a modal tool picks up a mode change made in
    /// the panel while its gesture is still live.
    pub fn sync_tools(&mut self) {
        let settings = self.tool_settings;
        self.tools.sync_settings(&settings);
    }

    /// True when the active tool is holding an edit that Enter would apply.
    pub fn has_pending_tool_edit(&self) -> bool {
        self.tools.has_pending_edit()
    }

    /// Apply the active tool's pending edit.
    pub fn commit_tool(&mut self) {
        self.with_tool(|tool, ctx| tool.commit(ctx));
    }

    /// Discard the active tool's pending edit.
    pub fn cancel_tool(&mut self) {
        self.with_tool(|tool, ctx| tool.cancel(ctx));
    }

    /// Run `f` with the active tool and a context borrowing the rest of state.
    fn with_tool(&mut self, f: impl FnOnce(&mut dyn crate::tools::Tool, &mut ToolContext)) {
        // Split the borrows: the toolbox is taken out of `self` for the call so
        // the tool can hold `&mut self.doc` at the same time.
        let mut tools = std::mem::take(&mut self.tools);
        let composite = self.cache.image().clone();
        let mut ctx = ToolContext {
            doc: &mut self.doc,
            history: &mut self.history,
            viewport: &mut self.viewport,
            brush: &self.brush,
            primary: &mut self.primary,
            secondary: self.secondary,
            tolerance: self.tolerance,
            sample_all_layers: self.sample_all_layers,
            selection_mode: self.selection_mode,
            composite: &composite,
            status: None,
            rig: &mut self.rig.tool,
        };
        f(tools.active_mut(), &mut ctx);
        let status = ctx.status.take();
        self.tools = tools;
        if let Some(message) = status {
            self.status = message;
        }
    }

    /// Send a pointer event to the active tool.
    pub fn tool_pointer(&mut self, phase: PointerPhase, event: &ToolEvent) {
        self.sync_tools();
        self.with_tool(|tool, ctx| match phase {
            PointerPhase::Down => tool.pointer_down(ctx, event),
            PointerPhase::Move => tool.pointer_move(ctx, event),
            PointerPhase::Up => tool.pointer_up(ctx, event),
            PointerPhase::Cancel => tool.cancel(ctx),
        });
        if phase == PointerPhase::Up || phase == PointerPhase::Down {
            self.remember_color();
        }
    }

    /// Select a tool.
    ///
    /// A pending transform or liquify session is applied first: switching tool
    /// is a confirmation everywhere else, and silently discarding the work
    /// would be the surprising choice.
    pub fn select_tool(&mut self, id: ToolId) {
        if id == self.tools.active_id() {
            return;
        }
        if self.has_pending_tool_edit() {
            self.commit_tool();
        }
        if self.tools.select(id) {
            self.status = self.tr(id.label_key()).to_string();
            // Mesh editing happens on the rest pose, where vertex positions
            // are also texture coordinates.
            self.rig.rest_view = id == ToolId::Mesh;
        }
    }

    fn remember_color(&mut self) {
        let color = self.primary.to_rgba8();
        if self.recent_colors.first() == Some(&color) {
            return;
        }
        self.recent_colors.retain(|c| *c != color);
        self.recent_colors.insert(0, color);
        self.recent_colors.truncate(16);
    }

    // ------------------------------------------------------------- documents

    /// Replace the document, resetting history and view.
    pub fn set_document(&mut self, doc: Document, path: Option<PathBuf>) {
        self.doc = doc;
        self.history = History::default();
        self.cache.invalidate();
        self.rig = crate::rigging::RigEditor::default();
        self.path = path;
        let size = self.viewport.size;
        self.viewport = Viewport::fitted(self.doc.width, self.doc.height, size);
        self.status = self.tr("status.ready").to_string();
    }

    /// Start a new document.
    pub fn new_document(&mut self, width: u32, height: u32, name: impl Into<String>) {
        self.set_document(Document::new(width, height, name), None);
    }

    /// Open a project file.
    pub fn open_project(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref().to_path_buf();
        let doc = project::load_project(&path)?;
        self.set_document(doc, Some(path));
        Ok(())
    }

    /// Open a layered Photoshop document: layers, folders, blend modes,
    /// opacity, clipping and masks come across.
    pub fn open_psd(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let doc = aether_io::psd::load_psd_file(path.as_ref())?;
        self.set_document(doc, None);
        self.status = format!("Imported {} layers", self.doc.layer_count());
        Ok(())
    }

    /// Open a Live2D Cubism model (its `.model3.json`, or the folder holding
    /// it) as a new document. Returns what could not be carried over.
    pub fn open_live2d(&mut self, path: impl AsRef<Path>) -> Result<Vec<String>> {
        let imported = aether_io::live2d_model::import_live2d(
            path.as_ref(),
            &aether_io::live2d_model::Live2DImportOptions::default(),
        )?;
        self.set_document(imported.document, None);
        let rig = &self.doc.rig;
        let summary = format!(
            "Opened Live2D model: {} parameters, {} motions, {} expressions",
            rig.parameters.len(),
            rig.motions.len(),
            rig.expressions.len()
        );
        self.status = crate::rigging::with_notes(summary, &imported.notes);
        Ok(imported.notes)
    }

    /// Write the document as a layered Photoshop file.
    pub fn export_psd(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let composite = self.compositor.render(&self.doc);
        let bytes = aether_io::psd::save_psd(&self.doc, &composite)?;
        std::fs::write(path, bytes)?;
        self.status = format!("Exported {}", path.display());
        Ok(())
    }

    /// Import an image as a new document.
    pub fn open_image(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let pixmap = aether_io::load_image(path.as_ref())?;
        let name = path
            .as_ref()
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Imported".into());
        let mut doc = Document::new(pixmap.width(), pixmap.height(), name);
        if let Some(target) = doc.layers.get_mut(doc.active_layer).and_then(|l| l.pixmap_mut()) {
            *target = pixmap;
        }
        doc.mark_all_dirty();
        self.set_document(doc, None);
        Ok(())
    }

    /// Import an image as a new layer in the current document.
    pub fn import_image_as_layer(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let pixmap = aether_io::load_image(path.as_ref())?;
        let name = path
            .as_ref()
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Imported".into());
        let id = self.doc.next_layer_id();
        let mut layer = Layer::raster(id, name, self.doc.width, self.doc.height);
        if let Some(target) = layer.pixmap_mut() {
            target.paste_rect(&pixmap, 0, 0);
        }
        let command = AddLayerCommand::above_active(&self.doc, layer);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Save to the current path, or report that one is needed.
    pub fn save(&mut self) -> Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(AetherError::invalid("no file path yet; use Save As"));
        };
        self.save_as(path)
    }

    /// Save to `path` and remember it.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let mut path = path.as_ref().to_path_buf();
        if path.extension().is_none() {
            path.set_extension(project::EXTENSION);
        }
        self.doc.metadata.touch();
        project::save_project(&self.doc, &path)?;
        self.path = Some(path.clone());
        self.history.mark_saved();
        self.status = format!("Saved {}", path.display());
        Ok(())
    }

    /// Export the flattened composite as an image file.
    pub fn export_image(&mut self, path: impl AsRef<Path>, settings: &ExportSettings) -> Result<()> {
        let path = path.as_ref();
        let composite = self.compositor.render(&self.doc);
        aether_io::export_image(&composite, path, settings)?;
        self.status = format!("Exported {}", path.display());
        Ok(())
    }

    /// Export a PNG, choosing the format from the extension when possible.
    pub fn export_png(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let format = ImageFormat::from_path(path.as_ref()).unwrap_or(ImageFormat::Png);
        let settings = ExportSettings {
            format,
            ..Default::default()
        };
        self.export_image(path, &settings)
    }

    /// Save the current settings as a new preset.
    pub fn add_current_brush_as_preset(&mut self) {
        let mut preset = self.brush.clone();
        preset.sanitize();
        // A duplicate name would make the preset list ambiguous.
        if self.brush_presets.iter().any(|p| p.name == preset.name) {
            preset.name = format!("{} copy", preset.name);
        }
        self.status = format!("Saved preset '{}'", preset.name);
        self.brush_presets.push(preset);
    }

    /// Write every preset to a JSON file.
    pub fn export_brush_presets(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        aether_io::save_presets(&self.brush_presets, path)?;
        self.status = format!(
            "Exported {} presets to {}",
            self.brush_presets.len(),
            path.display()
        );
        Ok(())
    }

    /// Load presets from a JSON file, appending them to the library.
    ///
    /// Loading never replaces the library: an imported set is additive, and a
    /// name that already exists is suffixed rather than silently overwriting
    /// the artist's own brush.
    pub fn import_brush_presets(&mut self, path: impl AsRef<Path>) -> Result<usize> {
        let loaded = aether_io::load_presets(path.as_ref())?;
        let count = loaded.len();
        for mut preset in loaded {
            while self.brush_presets.iter().any(|p| p.name == preset.name) {
                preset.name = format!("{} (imported)", preset.name);
            }
            self.brush_presets.push(preset);
        }
        self.status = format!("Imported {count} presets");
        Ok(count)
    }

    /// Ask for a path, then export the preset library.
    pub fn export_brush_presets_via_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Brush presets", &["json"])
            .set_file_name("aether-brushes.json")
            .save_file();
        if let Some(path) = picked {
            if let Err(error) = self.export_brush_presets(path) {
                self.report_error("Export presets", &error);
            }
        }
    }

    /// Ask for a path, then import presets.
    pub fn import_brush_presets_via_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Brush presets", &["json"])
            .pick_file();
        if let Some(path) = picked {
            if let Err(error) = self.import_brush_presets(path) {
                self.report_error("Import presets", &error);
            }
        }
    }

    /// True when there is work that would be lost by closing.
    pub fn has_unsaved_changes(&self) -> bool {
        self.history.has_unsaved_changes()
    }

    // ---------------------------------------------------------------- layers

    /// Add an empty raster layer above the active one.
    pub fn add_layer(&mut self) -> Result<LayerId> {
        let id = self.doc.next_layer_id();
        let name = format!("Layer {}", self.doc.layer_count() + 1);
        let layer = Layer::raster(id, name, self.doc.width, self.doc.height);
        let command = AddLayerCommand::above_active(&self.doc, layer);
        self.history.execute(&mut self.doc, Box::new(command))?;
        Ok(id)
    }

    /// Add an empty group above the active layer.
    pub fn add_group(&mut self) -> Result<LayerId> {
        let id = self.doc.next_layer_id();
        let layer = Layer::group(id, format!("Group {}", self.doc.layer_count() + 1));
        let command = AddLayerCommand::above_active(&self.doc, layer);
        self.history.execute(&mut self.doc, Box::new(command))?;
        Ok(id)
    }

    /// Delete a layer and everything inside it.
    pub fn delete_layer(&mut self, id: LayerId) -> Result<()> {
        if self.doc.layers.len() <= 1 {
            return Err(AetherError::document("a document needs at least one layer"));
        }
        self.history
            .execute(&mut self.doc, Box::new(DeleteLayerCommand::new(id)))
    }

    /// Duplicate a layer, including its pixels and mask.
    pub fn duplicate_layer(&mut self, id: LayerId) -> Result<LayerId> {
        let source = self.doc.layers.try_get(id)?.clone();
        if source.is_group() {
            return Err(AetherError::document("duplicating groups is not supported yet"));
        }
        let new_id = self.doc.next_layer_id();
        let mut copy = source.clone();
        copy.id = new_id;
        copy.name = format!("{} copy", source.name);
        let parent = self.doc.layers.parent_of(id);
        let index = self.doc.layers.index_of(id).map(|i| i + 1).unwrap_or(0);
        let command = AddLayerCommand::new(copy, parent, index);
        // A rigged layer's copy gets its own copy of the mesh, keyforms and
        // all, in the same undo step.
        match self.doc.rig.mesh(id).cloned() {
            Some(mut mesh) => {
                mesh.layer = new_id;
                mesh.glue.clear();
                if !mesh.name.is_empty() {
                    mesh.name = format!("{} copy", mesh.name);
                }
                let mut rig = self.doc.rig.clone();
                rig.set_mesh(mesh);
                let mut transaction = Transaction::new("Duplicate Layer");
                transaction.push(Box::new(command));
                transaction.push(Box::new(aether_document::SetRigCommand::new(
                    "Duplicate mesh",
                    rig,
                )));
                self.history.execute(&mut self.doc, Box::new(transaction))?;
            }
            None => self.history.execute(&mut self.doc, Box::new(command))?,
        }
        Ok(new_id)
    }

    /// Merge the active layer down into the one below it.
    ///
    /// Flattening the pixels and removing the upper layer are batched into one
    /// transaction, so a single undo restores both halves.
    pub fn merge_down(&mut self, id: LayerId) -> Result<()> {
        let below = self
            .doc
            .layers
            .sibling_below(id)
            .ok_or_else(|| AetherError::document("there is no layer below this one"))?;
        let upper = self.doc.layers.try_get(id)?.clone();
        let Some(upper_pixels) = upper.pixmap().cloned() else {
            return Err(AetherError::document("only raster layers can be merged"));
        };
        let Some(lower_pixels) = self.doc.layers.try_get(below)?.pixmap().cloned() else {
            return Err(AetherError::document("the layer below holds no pixels"));
        };

        let region = self.doc.bounds();
        let opts = CompositeOptions {
            blend: upper.blend_mode,
            opacity: upper.opacity,
            offset: (0, 0),
            region: Some(region),
            alpha_lock: false,
        };
        let before = lower_pixels.copy_rect(region);
        let mut after = before.clone();
        composite_pixmap(&mut after, &upper_pixels, &opts, upper.active_mask());

        let mut transaction = Transaction::new("Merge Down");
        transaction.push(Box::new(RegionEdit::new(
            "Merge Down",
            below,
            region,
            before,
            after,
        )));
        transaction.push(Box::new(DeleteLayerCommand::new(id)));
        self.history.execute(&mut self.doc, Box::new(transaction))?;
        self.doc.set_active_layer(below);
        Ok(())
    }

    /// Move a layer one place up in its parent.
    pub fn move_layer_up(&mut self, id: LayerId) -> Result<()> {
        let parent = self.doc.layers.parent_of(id);
        let index = self
            .doc
            .layers
            .index_of(id)
            .ok_or_else(|| AetherError::document("layer is not in the tree"))?;
        if index + 1 >= self.doc.layers.children_of(parent).len() {
            return Ok(());
        }
        let command = MoveLayerCommand::new(id, parent, index + 2);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Move a layer one place down in its parent.
    pub fn move_layer_down(&mut self, id: LayerId) -> Result<()> {
        let parent = self.doc.layers.parent_of(id);
        let index = self
            .doc
            .layers
            .index_of(id)
            .ok_or_else(|| AetherError::document("layer is not in the tree"))?;
        if index == 0 {
            return Ok(());
        }
        let command = MoveLayerCommand::new(id, parent, index - 1);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Change one property of one layer.
    pub fn set_layer_property(&mut self, id: LayerId, property: LayerProperty) -> Result<()> {
        let command = SetLayerPropertyCommand::new(id, property);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Replace a layer's effect stack.
    pub fn set_layer_effects(&mut self, id: LayerId, effects: Vec<LayerEffect>, label: &str) -> Result<()> {
        let command = SetLayerEffectsCommand::new(label, id, effects);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Add a non-destructive adjustment layer above the active layer.
    pub fn add_adjustment_layer(&mut self, adjustment: Adjustment) -> Result<LayerId> {
        let id = self.doc.next_layer_id();
        let layer = Layer::adjustment(id, adjustment.name(), adjustment);
        let command = AddLayerCommand::above_active(&self.doc, layer);
        self.history.execute(&mut self.doc, Box::new(command))?;
        Ok(id)
    }

    /// Edit the adjustment on an adjustment layer.
    pub fn set_adjustment(&mut self, id: LayerId, adjustment: Adjustment) -> Result<()> {
        // The adjustment lives in the layer's content, so this is a content
        // edit rather than a property change; it goes through the same
        // effect-stack command shape to keep slider drags coalescing.
        let layer = self.doc.layers.try_get(id)?;
        let LayerContent::Adjustment(_) = &layer.content else {
            return Err(AetherError::document("that layer is not an adjustment layer"));
        };
        let command = SetAdjustmentCommand::new(id, adjustment);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Apply a filter destructively to the active layer, inside the selection.
    ///
    /// The same kernels back the non-destructive effect stack; this path is for
    /// when an artist wants the result baked in.
    pub fn apply_filter(&mut self, kind: EffectKind) -> Result<()> {
        let id = self.doc.active_layer;
        let region = self.doc.selection.bounds().unwrap_or_else(|| self.doc.bounds());
        let selection = self.doc.selection.mask().cloned();
        let effect = LayerEffect::new(kind);
        let label = format!("Filter: {}", effect.name());
        let bounds = self.doc.bounds();
        let edit = RegionEdit::capture(&mut self.doc, id, bounds, label, |pixmap| {
            let filtered = effect.apply(pixmap);
            match &selection {
                // Restricted to the selection: composite the filtered copy back
                // through the selection mask so the edges stay soft.
                Some(mask) => {
                    let opts = CompositeOptions {
                        blend: BlendMode::Normal,
                        opacity: 1.0,
                        offset: (0, 0),
                        region: Some(region),
                        alpha_lock: false,
                    };
                    // Clear first: a filter can reduce alpha, and a plain
                    // composite could never take coverage away.
                    for y in region.y..region.bottom() {
                        for x in region.x..region.right() {
                            let coverage = mask.get(x, y);
                            if coverage == 0 {
                                continue;
                            }
                            let original = pixmap.get(x, y);
                            let result = filtered.get(x, y);
                            let t = coverage as f32 / 255.0;
                            pixmap.set(x, y, original.to_rgba().lerp(result.to_rgba(), t).to_rgba8());
                        }
                    }
                    let _ = opts;
                }
                None => *pixmap = filtered,
            }
            Ok(())
        })?;
        if !edit.is_noop() {
            self.history.push_applied(Box::new(edit));
        }
        Ok(())
    }

    /// Turn the current selection into a mask on `id`.
    pub fn add_mask_from_selection(&mut self, id: LayerId) -> Result<()> {
        let mask = match self.doc.selection.mask() {
            Some(mask) => mask.clone(),
            None => Mask::filled(self.doc.width, self.doc.height, 255),
        };
        let command = SetLayerMaskCommand::new(id, Some(mask));
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Remove a layer's mask.
    pub fn remove_mask(&mut self, id: LayerId) -> Result<()> {
        let command = SetLayerMaskCommand::new(id, None);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Erase the active layer, within the selection when there is one.
    pub fn clear_layer(&mut self) -> Result<()> {
        let id = self.doc.active_layer;
        let region = self.doc.selection.bounds().unwrap_or_else(|| self.doc.bounds());
        let mask = self.doc.selection.mask().cloned();
        let edit = RegionEdit::capture(&mut self.doc, id, region, "Clear Layer", |pixmap| {
            match &mask {
                Some(mask) => {
                    aether_raster::composite::erase_masked(pixmap, mask, 1.0, Some(region));
                }
                None => pixmap.fill_rect(region, Rgba8::TRANSPARENT),
            }
            Ok(())
        })?;
        if !edit.is_noop() {
            self.history.push_applied(Box::new(edit));
        }
        Ok(())
    }

    /// Fill the selection (or the whole layer) with the primary colour.
    pub fn fill_selection(&mut self) -> Result<()> {
        let id = self.doc.active_layer;
        let region = self.doc.selection.bounds().unwrap_or_else(|| self.doc.bounds());
        let color = self.primary;
        let mask = self.doc.selection.mask().cloned();
        let blend = self.brush.blend;
        let edit = RegionEdit::capture(&mut self.doc, id, region, "Fill", |pixmap| {
            let opts = CompositeOptions {
                blend,
                opacity: 1.0,
                offset: (0, 0),
                region: Some(region),
                alpha_lock: false,
            };
            match &mask {
                Some(mask) => {
                    aether_raster::composite::fill_masked(pixmap, color, mask, &opts);
                }
                None => {
                    let full = Mask::filled(pixmap.width(), pixmap.height(), 255);
                    aether_raster::composite::fill_masked(pixmap, color, &full, &opts);
                }
            }
            Ok(())
        })?;
        if !edit.is_noop() {
            self.history.push_applied(Box::new(edit));
        }
        Ok(())
    }

    /// Resize the canvas.
    pub fn resize_canvas(&mut self, width: u32, height: u32) -> Result<()> {
        let command = ResizeCanvasCommand::new(width, height);
        self.history.execute(&mut self.doc, Box::new(command))?;
        self.cache.invalidate();
        Ok(())
    }

    // ------------------------------------------------------------ selection

    /// Select the whole canvas.
    pub fn select_all(&mut self) -> Result<()> {
        let mut selection = Selection::none();
        selection.select_all(self.doc.width, self.doc.height);
        let command = SetSelectionCommand::new("Select All", selection);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Clear the selection.
    pub fn deselect(&mut self) -> Result<()> {
        if !self.doc.selection.is_active() {
            return Ok(());
        }
        let command = SetSelectionCommand::new("Deselect", Selection::none());
        self.history.execute(&mut self.doc, Box::new(command))
    }

    /// Invert the selection.
    pub fn invert_selection(&mut self) -> Result<()> {
        let mut selection = self.doc.selection.clone();
        selection.invert(self.doc.width, self.doc.height);
        let command = SetSelectionCommand::new("Invert Selection", selection);
        self.history.execute(&mut self.doc, Box::new(command))
    }

    // ------------------------------------------------------------------ view

    /// Frame the whole document.
    pub fn zoom_fit(&mut self) {
        self.viewport.fit(self.doc.width, self.doc.height);
    }

    /// Reset to 100%.
    pub fn zoom_reset(&mut self) {
        self.viewport.reset(self.doc.width, self.doc.height);
    }

    /// Zoom about the centre of the view.
    pub fn zoom_by(&mut self, factor: f32) {
        let center = self.viewport.size * 0.5;
        self.viewport.zoom_at(center, factor);
    }

    /// Switch workspace, applying its defaults.
    pub fn set_workspace(&mut self, workspace: Workspace) {
        self.workspace = workspace;
        match workspace {
            Workspace::PixelArt => {
                self.show_pixel_grid = true;
                self.view_interpolation = Interpolation::Nearest;
                if let Some(preset) = self.brush_presets.iter().find(|p| p.name == "Pixel") {
                    self.brush = preset.clone();
                }
                self.brush.smoothing = 0.0;
            }
            Workspace::Illustration | Workspace::Compositing => {
                self.show_pixel_grid = false;
                self.view_interpolation = Interpolation::Bilinear;
            }
            Workspace::Rigging => {
                self.show_pixel_grid = false;
                self.view_interpolation = Interpolation::Bilinear;
                self.rig.animate = false;
                self.rig.playing = false;
            }
            Workspace::Animation => {
                self.show_pixel_grid = false;
                self.view_interpolation = Interpolation::Bilinear;
                if self.rig.motion.is_none() && !self.doc.rig.motions.is_empty() {
                    self.rig.motion = Some(0);
                }
                self.rig.animate = self.rig.motion.is_some();
            }
        }
        self.status = self.tr(workspace.key()).to_string();
    }

    // --------------------------------------------------------------- actions

    /// Run a menu or keyboard action.
    ///
    /// Actions that need a file dialog return `false` so the caller (which owns
    /// the window) can open one; everything else is handled here and is
    /// therefore testable without a UI.
    pub fn handle_action(&mut self, action: Action) -> bool {
        let result: Result<()> = match action {
            Action::Undo => self.history.undo(&mut self.doc).map(|_| ()),
            Action::Redo => self.history.redo(&mut self.doc).map(|_| ()),
            Action::ClearLayer => self.clear_layer(),
            Action::SelectAll => self.select_all(),
            Action::Deselect => self.deselect(),
            Action::InvertSelection => self.invert_selection(),
            Action::AddLayer => self.add_layer().map(|_| ()),
            Action::DeleteLayer => {
                let id = self.doc.active_layer;
                self.delete_layer(id)
            }
            Action::DuplicateLayer => {
                let id = self.doc.active_layer;
                self.duplicate_layer(id).map(|_| ())
            }
            Action::MergeDown => {
                let id = self.doc.active_layer;
                self.merge_down(id)
            }
            Action::ZoomIn => {
                self.zoom_by(1.25);
                Ok(())
            }
            Action::ZoomOut => {
                self.zoom_by(0.8);
                Ok(())
            }
            Action::ZoomFit => {
                self.zoom_fit();
                Ok(())
            }
            Action::ZoomReset => {
                self.zoom_reset();
                Ok(())
            }
            Action::RotateLeft => {
                self.viewport.rotation -= std::f32::consts::FRAC_PI_8;
                Ok(())
            }
            Action::RotateRight => {
                self.viewport.rotation += std::f32::consts::FRAC_PI_8;
                Ok(())
            }
            Action::ResetRotation => {
                self.viewport.rotation = 0.0;
                Ok(())
            }
            Action::MirrorView => {
                self.viewport.mirror = !self.viewport.mirror;
                Ok(())
            }
            Action::TogglePixelGrid => {
                self.show_pixel_grid = !self.show_pixel_grid;
                Ok(())
            }
            Action::ToolBrush => {
                self.select_tool(ToolId::Brush);
                Ok(())
            }
            Action::ToolEraser => {
                self.select_tool(ToolId::Eraser);
                Ok(())
            }
            Action::ToolBucket => {
                self.select_tool(ToolId::Bucket);
                Ok(())
            }
            Action::ToolEyedropper => {
                self.select_tool(ToolId::Eyedropper);
                Ok(())
            }
            Action::ToolRectSelect => {
                self.select_tool(ToolId::RectSelect);
                Ok(())
            }
            Action::ToolMove => {
                self.select_tool(ToolId::Move);
                Ok(())
            }
            Action::ToolTransform => {
                self.select_tool(ToolId::Transform);
                Ok(())
            }
            Action::ToolLiquify => {
                self.select_tool(ToolId::Liquify);
                Ok(())
            }
            Action::ToolPan => {
                self.select_tool(ToolId::Pan);
                Ok(())
            }
            Action::BrushLarger => {
                self.brush.size = (self.brush.size * 1.25).min(2000.0);
                self.brush.sanitize();
                Ok(())
            }
            Action::BrushSmaller => {
                self.brush.size = (self.brush.size * 0.8).max(0.5);
                self.brush.sanitize();
                Ok(())
            }
            Action::SwapColors => {
                std::mem::swap(&mut self.primary, &mut self.secondary);
                Ok(())
            }
            Action::ToolMesh => {
                self.select_tool(ToolId::Mesh);
                Ok(())
            }
            Action::ToolDeform => {
                self.select_tool(ToolId::Deform);
                Ok(())
            }
            Action::ToolBone => {
                self.select_tool(ToolId::Bone);
                Ok(())
            }
            Action::PlayPause => {
                self.toggle_playback();
                Ok(())
            }
            Action::NextFrame => {
                self.step_frames(1);
                Ok(())
            }
            Action::PreviousFrame => {
                self.step_frames(-1);
                Ok(())
            }
            Action::KeyAll => self.key_all_at_playhead(),
            Action::ToggleSimulation => {
                self.rig.simulate = !self.rig.simulate;
                Ok(())
            }
            Action::ResetPose => {
                self.reset_pose();
                Ok(())
            }
            Action::Save if self.path.is_some() => self.save(),
            // These need a file dialog, which only the windowed layer can open.
            Action::NewDocument
            | Action::OpenDocument
            | Action::Save
            | Action::SaveAs
            | Action::ExportPng => return false,
        };
        if let Err(error) = result {
            self.report_error(action.label(), &error);
        }
        true
    }
}

/// Which phase of a pointer gesture is being delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    /// Button pressed.
    Down,
    /// Pointer moved with the button held.
    Move,
    /// Button released.
    Up,
    /// Gesture abandoned (focus lost, Escape).
    Cancel,
}

/// A neutral starting palette: greys plus primaries and skin-tone midtones.
fn default_palette() -> Vec<Rgba8> {
    vec![
        Rgba8::rgb(0, 0, 0),
        Rgba8::rgb(64, 64, 64),
        Rgba8::rgb(128, 128, 128),
        Rgba8::rgb(192, 192, 192),
        Rgba8::rgb(255, 255, 255),
        Rgba8::rgb(214, 68, 68),
        Rgba8::rgb(232, 138, 62),
        Rgba8::rgb(240, 205, 92),
        Rgba8::rgb(112, 186, 106),
        Rgba8::rgb(72, 148, 208),
        Rgba8::rgb(96, 96, 190),
        Rgba8::rgb(150, 96, 190),
        Rgba8::rgb(244, 214, 196),
        Rgba8::rgb(214, 170, 142),
        Rgba8::rgb(158, 112, 88),
        Rgba8::rgb(92, 62, 48),
    ]
}

/// Blend modes offered in the layer and brush panels.
pub fn blend_modes() -> &'static [BlendMode] {
    &BlendMode::ALL
}

/// True when `layer` can be painted on right now.
pub fn is_paintable(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Raster(_)) && !layer.locked && layer.visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::input::{InputSample, Modifiers, PointerButton};
    use aether_core::math::Vec2;
    use aether_raster::adjust::Adjustment;
    use aether_raster::effect::{EffectKind, LayerEffect};

    fn state() -> EditorState {
        EditorState::new(Document::new(64, 64, "test"))
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

    fn pixel(state: &EditorState, x: i32, y: i32) -> Rgba8 {
        state
            .doc
            .layers
            .get(state.doc.active_layer)
            .and_then(|l| l.pixmap())
            .map(|p| p.get(x, y))
            .unwrap_or(Rgba8::TRANSPARENT)
    }

    #[test]
    fn a_new_state_is_ready_to_paint() {
        let mut s = state();
        assert_eq!(s.tools.active_id(), ToolId::Brush);
        assert!(s.doc.paint_target().is_ok());
        assert!(!s.has_unsaved_changes());
    }

    #[test]
    fn painting_through_the_state_updates_pixels_and_history() {
        let mut s = state();
        s.primary = Rgba::rgb(0.0, 1.0, 0.0);
        s.brush.size = 10.0;
        s.brush.hardness = 1.0;
        s.brush.smoothing = 0.0;
        s.tool_pointer(PointerPhase::Down, &event(10.0, 10.0, 0.016));
        s.tool_pointer(PointerPhase::Move, &event(30.0, 10.0, 0.032));
        s.tool_pointer(PointerPhase::Up, &event(30.0, 10.0, 0.048));

        assert_eq!(pixel(&s, 20, 10), Rgba8::new(0, 255, 0, 255));
        assert!(s.has_unsaved_changes());
        assert_eq!(s.history.depth(), 1);
    }

    #[test]
    fn the_composite_cache_follows_the_document() {
        let mut s = state();
        s.refresh();
        s.brush.size = 8.0;
        s.tool_pointer(PointerPhase::Down, &event(32.0, 32.0, 0.016));
        s.tool_pointer(PointerPhase::Up, &event(32.0, 32.0, 0.032));
        let region = s.refresh();
        assert!(!region.is_empty());
        assert_ne!(s.composite().get(32, 32), Rgba8::TRANSPARENT);
    }

    #[test]
    fn undo_and_redo_run_through_actions() {
        let mut s = state();
        s.brush.size = 8.0;
        s.tool_pointer(PointerPhase::Down, &event(32.0, 32.0, 0.016));
        s.tool_pointer(PointerPhase::Up, &event(32.0, 32.0, 0.032));
        assert_ne!(pixel(&s, 32, 32), Rgba8::TRANSPARENT);
        assert!(s.handle_action(Action::Undo));
        assert_eq!(pixel(&s, 32, 32), Rgba8::TRANSPARENT);
        assert!(s.handle_action(Action::Redo));
        assert_ne!(pixel(&s, 32, 32), Rgba8::TRANSPARENT);
    }

    #[test]
    fn layer_actions_add_and_delete() {
        let mut s = state();
        assert_eq!(s.doc.layer_count(), 1);
        s.handle_action(Action::AddLayer);
        assert_eq!(s.doc.layer_count(), 2);
        s.handle_action(Action::DeleteLayer);
        assert_eq!(s.doc.layer_count(), 1);
        // The last layer is protected.
        s.handle_action(Action::DeleteLayer);
        assert_eq!(s.doc.layer_count(), 1);
        assert!(
            s.status.contains("at least one layer"),
            "unhelpful status: {}",
            s.status
        );
    }

    #[test]
    fn duplicating_copies_the_pixels() {
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::WHITE);
        }
        let original = s.doc.active_layer;
        let copy = s.duplicate_layer(original).expect("duplicate");
        assert_ne!(copy, original);
        assert_eq!(
            s.doc
                .layers
                .get(copy)
                .and_then(|l| l.pixmap())
                .map(|p| p.get(1, 1)),
            Some(Rgba8::WHITE)
        );
        assert_eq!(s.doc.layers.roots(), &[original, copy]);
    }

    #[test]
    fn merge_down_flattens_and_removes_the_upper_layer() {
        let mut s = state();
        let lower = s.doc.active_layer;
        if let Some(pm) = s.doc.layers.get_mut(lower).and_then(|l| l.pixmap_mut()) {
            pm.fill(Rgba8::new(0, 0, 255, 255));
        }
        let upper = s.add_layer().expect("add");
        if let Some(pm) = s.doc.layers.get_mut(upper).and_then(|l| l.pixmap_mut()) {
            pm.fill_rect(IRect::new(0, 0, 32, 64), Rgba8::new(255, 0, 0, 255));
        }
        s.merge_down(upper).expect("merge");

        assert_eq!(s.doc.layer_count(), 1);
        let merged = s.doc.layers.get(lower).and_then(|l| l.pixmap()).expect("pixels");
        assert_eq!(merged.get(10, 10), Rgba8::new(255, 0, 0, 255));
        assert_eq!(merged.get(50, 10), Rgba8::new(0, 0, 255, 255));

        s.history.undo(&mut s.doc).expect("undo");
        assert_eq!(s.doc.layer_count(), 2, "one undo must restore both halves");
        assert_eq!(
            s.doc
                .layers
                .get(lower)
                .and_then(|l| l.pixmap())
                .map(|p| p.get(10, 10)),
            Some(Rgba8::new(0, 0, 255, 255))
        );
    }

    #[test]
    fn merging_the_bottom_layer_is_refused() {
        let mut s = state();
        let id = s.doc.active_layer;
        assert!(s.merge_down(id).is_err());
    }

    #[test]
    fn moving_layers_reorders_the_stack() {
        let mut s = state();
        let first = s.doc.active_layer;
        let second = s.add_layer().expect("add");
        assert_eq!(s.doc.layers.roots(), &[first, second]);
        s.move_layer_down(second).expect("down");
        assert_eq!(s.doc.layers.roots(), &[second, first]);
        s.move_layer_up(second).expect("up");
        assert_eq!(s.doc.layers.roots(), &[first, second]);
    }

    #[test]
    fn clear_layer_respects_the_selection() {
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::WHITE);
        }
        s.doc
            .selection
            .select_rect(64, 64, IRect::new(0, 0, 32, 64), SelectionMode::Replace);
        s.clear_layer().expect("clear");
        assert_eq!(pixel(&s, 10, 10).a, 0);
        assert_eq!(pixel(&s, 50, 10), Rgba8::WHITE);
    }

    #[test]
    fn selection_actions_round_trip() {
        let mut s = state();
        s.handle_action(Action::SelectAll);
        assert!(s.doc.selection.is_active());
        s.handle_action(Action::InvertSelection);
        assert!(!s.doc.selection.is_active() || s.doc.selection.bounds().is_none());
        s.handle_action(Action::Undo);
        assert!(s.doc.selection.is_active());
        s.handle_action(Action::Deselect);
        assert!(!s.doc.selection.is_active());
    }

    #[test]
    fn mask_from_selection_hides_part_of_the_layer() {
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::WHITE);
        }
        s.doc
            .selection
            .select_rect(64, 64, IRect::new(0, 0, 32, 64), SelectionMode::Replace);
        let id = s.doc.active_layer;
        s.add_mask_from_selection(id).expect("mask");
        s.refresh();
        assert_eq!(s.composite().get(10, 10), Rgba8::WHITE);
        assert_eq!(s.composite().get(50, 10), Rgba8::TRANSPARENT);
        s.remove_mask(id).expect("remove");
        s.cache.invalidate();
        s.refresh();
        assert_eq!(s.composite().get(50, 10), Rgba8::WHITE);
    }

    #[test]
    fn save_and_reopen_round_trips_through_the_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("doc.aether");
        let mut s = state();
        s.brush.size = 12.0;
        s.tool_pointer(PointerPhase::Down, &event(20.0, 20.0, 0.016));
        s.tool_pointer(PointerPhase::Up, &event(20.0, 20.0, 0.032));
        s.save_as(&path).expect("save");
        assert!(!s.has_unsaved_changes(), "saving clears the dirty marker");

        let painted = pixel(&s, 20, 20);
        let mut other = state();
        other.open_project(&path).expect("open");
        assert_eq!(
            other
                .doc
                .layers
                .get(other.doc.active_layer)
                .and_then(|l| l.pixmap())
                .map(|p| p.get(20, 20)),
            Some(painted)
        );
    }

    #[test]
    fn saving_without_a_path_asks_for_one() {
        let mut s = state();
        let err = s.save().expect_err("must refuse");
        assert!(err.to_string().contains("Save As"));
    }

    #[test]
    fn exported_png_matches_the_composite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.png");
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::new(10, 120, 200, 255));
        }
        s.export_png(&path).expect("export");
        let back = aether_io::load_image(&path).expect("read back");
        assert_eq!(back.get(5, 5), Rgba8::new(10, 120, 200, 255));
        assert_eq!((back.width(), back.height()), (64, 64));
    }

    #[test]
    fn live2d_models_open_as_documents() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/live2d-oracle/models/Hiyori/Hiyori.model3.json");
        if !path.exists() {
            eprintln!("no Live2D samples (crates/aether-live2d/oracle/run.sh); skipping");
            return;
        }
        let mut s = state();
        s.open_live2d(&path).expect("open");
        assert_eq!(s.doc.rig.cubism.len(), 1);
        assert!(s.status.contains("Live2D"));
        assert!(!s.history.can_undo());
    }

    #[test]
    fn psd_files_open_and_export_with_their_layers() {
        let mut state = state();
        state.add_layer().expect("layer");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("parts.psd");
        state.export_psd(&path).expect("export");
        state.new_document(8, 8, "other");
        state.open_psd(&path).expect("open");
        assert_eq!(state.doc.layer_count(), 2);
        assert_eq!(state.doc.name, "parts");
    }

    #[test]
    fn importing_an_image_creates_a_matching_document() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("in.png");
        let source = Pixmap::filled(20, 10, Rgba8::new(1, 2, 3, 255));
        aether_io::save_png(&source, &path).expect("write");

        let mut s = state();
        s.open_image(&path).expect("import");
        assert_eq!((s.doc.width, s.doc.height), (20, 10));
        assert_eq!(pixel(&s, 5, 5), Rgba8::new(1, 2, 3, 255));
    }

    #[test]
    fn importing_as_a_layer_keeps_the_document() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("in.png");
        aether_io::save_png(&Pixmap::filled(8, 8, Rgba8::WHITE), &path).expect("write");
        let mut s = state();
        s.import_image_as_layer(&path).expect("import");
        assert_eq!(s.doc.layer_count(), 2);
        assert_eq!((s.doc.width, s.doc.height), (64, 64));
        s.history.undo(&mut s.doc).expect("undo");
        assert_eq!(s.doc.layer_count(), 1);
    }

    #[test]
    fn resize_canvas_is_undoable() {
        let mut s = state();
        s.resize_canvas(32, 16).expect("resize");
        assert_eq!((s.doc.width, s.doc.height), (32, 16));
        s.history.undo(&mut s.doc).expect("undo");
        assert_eq!((s.doc.width, s.doc.height), (64, 64));
    }

    #[test]
    fn the_pixel_art_workspace_changes_the_defaults() {
        let mut s = state();
        s.set_workspace(Workspace::PixelArt);
        assert!(s.show_pixel_grid);
        assert_eq!(s.view_interpolation, Interpolation::Nearest);
        assert_eq!(s.brush.smoothing, 0.0);
        s.set_workspace(Workspace::Illustration);
        assert!(!s.show_pixel_grid);
    }

    #[test]
    fn file_actions_defer_to_the_window_layer() {
        let mut s = state();
        assert!(
            !s.handle_action(Action::OpenDocument),
            "opening needs a file dialog"
        );
        assert!(!s.handle_action(Action::SaveAs));
        assert!(s.handle_action(Action::ZoomIn), "view actions are handled here");
    }

    #[test]
    fn brush_size_shortcuts_stay_in_range() {
        let mut s = state();
        for _ in 0..100 {
            s.handle_action(Action::BrushSmaller);
        }
        assert!(s.brush.size >= 0.1);
        for _ in 0..100 {
            s.handle_action(Action::BrushLarger);
        }
        assert!(s.brush.size <= 5000.0);
    }

    #[test]
    fn adjustment_layers_can_be_added_and_retuned() {
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::rgb(10, 20, 30));
        }
        let id = s.add_adjustment_layer(Adjustment::Invert).expect("add");
        s.cache.invalidate();
        s.refresh();
        assert_eq!(s.composite().get(4, 4), Rgba8::rgb(245, 235, 225));

        // Retune it, then undo back through both steps.
        s.set_adjustment(id, Adjustment::Grayscale).expect("retune");
        s.cache.invalidate();
        s.refresh();
        let grey = s.composite().get(4, 4);
        assert_eq!(grey.r, grey.g);
        s.history.undo(&mut s.doc).expect("undo retune");
        s.history.undo(&mut s.doc).expect("undo add");
        assert_eq!(s.doc.layer_count(), 1);
    }

    #[test]
    fn retuning_a_raster_layer_as_an_adjustment_is_refused() {
        let mut s = state();
        let id = s.doc.active_layer;
        assert!(s.set_adjustment(id, Adjustment::Invert).is_err());
    }

    #[test]
    fn layer_effects_show_up_in_the_composite_and_undo() {
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill_rect(IRect::new(20, 20, 10, 10), Rgba8::WHITE);
        }
        let id = s.doc.active_layer;
        s.set_layer_effects(
            id,
            vec![LayerEffect::new(EffectKind::DropShadow {
                dx: 6.0,
                dy: 6.0,
                radius: 2.0,
                color: Rgba8::BLACK,
                opacity: 1.0,
            })],
            "Add Effect",
        )
        .expect("effects");
        s.cache.invalidate();
        s.refresh();
        assert!(s.composite().get(34, 34).a > 0, "the shadow should be visible");

        s.history.undo(&mut s.doc).expect("undo");
        s.cache.invalidate();
        s.refresh();
        assert_eq!(s.composite().get(34, 34).a, 0);
    }

    #[test]
    fn filters_are_destructive_and_undoable() {
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::rgb(10, 20, 30));
        }
        s.apply_filter(EffectKind::Adjust {
            adjustment: Adjustment::Invert,
        })
        .expect("filter");
        assert_eq!(pixel(&s, 5, 5), Rgba8::rgb(245, 235, 225));
        s.history.undo(&mut s.doc).expect("undo");
        assert_eq!(pixel(&s, 5, 5), Rgba8::rgb(10, 20, 30));
    }

    #[test]
    fn filters_stay_inside_the_selection() {
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill(Rgba8::rgb(10, 20, 30));
        }
        s.doc
            .selection
            .select_rect(64, 64, IRect::new(0, 0, 32, 64), SelectionMode::Replace);
        s.apply_filter(EffectKind::Adjust {
            adjustment: Adjustment::Invert,
        })
        .expect("filter");
        assert_eq!(pixel(&s, 5, 5), Rgba8::rgb(245, 235, 225), "inside the selection");
        assert_eq!(pixel(&s, 50, 5), Rgba8::rgb(10, 20, 30), "outside it");
    }

    #[test]
    fn brush_presets_round_trip_through_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("brushes.json");
        let mut s = state();
        s.brush.name = "My Pen".into();
        s.brush.size = 33.0;
        s.add_current_brush_as_preset();
        let count = s.brush_presets.len();
        s.export_brush_presets(&path).expect("export");

        let mut other = state();
        let imported = other.import_brush_presets(&path).expect("import");
        assert_eq!(imported, count);
        assert!(other.brush_presets.iter().any(|p| p.size == 33.0));
    }

    #[test]
    fn importing_presets_never_overwrites_an_existing_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("brushes.json");
        let mut s = state();
        s.export_brush_presets(&path).expect("export");
        let before = s.brush_presets.len();
        s.import_brush_presets(&path).expect("import");
        assert_eq!(s.brush_presets.len(), before * 2);
        let names: std::collections::HashSet<&str> =
            s.brush_presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.len(), s.brush_presets.len(), "names must stay unique");
    }

    #[test]
    fn pressure_sources_behave_as_described() {
        // A device that reports force is trusted.
        assert_eq!(PressureSource::Device.resolve(Some(0.4), 0.0), 0.4);
        // Without a reported force the brush behaves like a mouse.
        assert_eq!(PressureSource::Device.resolve(None, 500.0), 1.0);
        // Speed mode tapers: faster is lighter.
        let slow = PressureSource::Speed.resolve(None, 50.0);
        let fast = PressureSource::Speed.resolve(None, 1200.0);
        assert!(slow > fast, "slow={slow} fast={fast}");
        assert!(fast >= 0.25, "the taper must not vanish entirely");
        // Off ignores everything.
        assert_eq!(PressureSource::Off.resolve(Some(0.1), 900.0), 1.0);
    }

    #[test]
    fn switching_tool_commits_a_pending_transform() {
        let mut s = state();
        if let Some(pm) = s
            .doc
            .layers
            .get_mut(s.doc.active_layer)
            .and_then(|l| l.pixmap_mut())
        {
            pm.fill_rect(IRect::new(10, 10, 40, 40), Rgba8::WHITE);
        }
        s.select_tool(ToolId::Transform);
        s.tool_pointer(PointerPhase::Down, &event(30.0, 30.0, 0.016));
        s.tool_pointer(PointerPhase::Move, &event(42.0, 30.0, 0.032));
        s.tool_pointer(PointerPhase::Up, &event(42.0, 30.0, 0.048));
        assert!(s.has_pending_tool_edit());

        s.select_tool(ToolId::Brush);
        assert!(!s.has_pending_tool_edit(), "switching tool applies the transform");
        assert_eq!(s.history.depth(), 1);
    }

    #[test]
    fn swapping_colours_exchanges_them() {
        let mut s = state();
        s.primary = Rgba::rgb(1.0, 0.0, 0.0);
        s.secondary = Rgba::rgb(0.0, 0.0, 1.0);
        s.handle_action(Action::SwapColors);
        assert_eq!(s.primary.to_rgba8(), Rgba8::new(0, 0, 255, 255));
        assert_eq!(s.secondary.to_rgba8(), Rgba8::new(255, 0, 0, 255));
    }

    #[test]
    fn recent_colours_track_use_without_duplicates() {
        let mut s = state();
        s.primary = Rgba::rgb(1.0, 0.0, 0.0);
        s.tool_pointer(PointerPhase::Down, &event(5.0, 5.0, 0.016));
        s.tool_pointer(PointerPhase::Up, &event(5.0, 5.0, 0.032));
        s.tool_pointer(PointerPhase::Down, &event(9.0, 9.0, 0.048));
        s.tool_pointer(PointerPhase::Up, &event(9.0, 9.0, 0.064));
        assert_eq!(s.recent_colors.len(), 1);
        assert_eq!(s.recent_colors[0], Rgba8::new(255, 0, 0, 255));
    }
}
