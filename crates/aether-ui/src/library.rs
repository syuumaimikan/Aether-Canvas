//! The sample library, and opening what comes in ZIP archives.
//!
//! Samples arrive as a folder of downloads: ZIPs holding Live2D models and
//! the layered PSDs they were made from, loose PSDs, images. The library
//! window lists everything in such a folder that can be opened — looking
//! inside the archives — and opens it with a click. By default it looks
//! for an `assets_sample` folder (see
//! [`aether_io::library::default_sample_folders`]); any folder can be
//! chosen.
//!
//! Opening a ZIP from File ▸ Open does the same for one archive: its only
//! model or PSD opens straight away, or a window asks which one.
//!
//! Big PSDs take seconds to read, so opening happens on a worker thread;
//! the editor stays responsive and shows what it is opening.

use crate::state::EditorState;
use aether_core::Result;
use aether_io::archive::{Archive, ArchiveEntry, EntryKind, Opened};
use aether_io::library::LibraryItem;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

/// The sample library window's state.
#[derive(Default)]
pub struct SampleLibrary {
    /// Whether the window is shown.
    pub open: bool,
    /// The folder listed.
    pub folder: Option<PathBuf>,
    /// What it holds.
    pub items: Vec<LibraryItem>,
    /// Only items whose name or location contains this.
    pub filter: String,
    /// Only items of this kind.
    pub kind: Option<EntryKind>,
    /// Rig PSDs from their layer names as soon as they open.
    pub auto_rig_psd: bool,
    scanned: bool,
}

impl SampleLibrary {
    /// Items passing the filters.
    pub fn visible(&self) -> Vec<&LibraryItem> {
        let needle = self.filter.to_lowercase();
        let root = self.folder.clone().unwrap_or_default();
        self.items
            .iter()
            .filter(|i| self.kind.is_none_or(|k| i.kind == k))
            .filter(|i| {
                needle.is_empty()
                    || i.title().to_lowercase().contains(&needle)
                    || i.location(&root).to_lowercase().contains(&needle)
            })
            .collect()
    }
}

/// An archive holding several things, waiting for a choice.
pub struct ArchiveChoice {
    /// The archive.
    pub file: PathBuf,
    /// What it holds.
    pub entries: Vec<ArchiveEntry>,
}

/// Something being opened on a worker thread.
pub struct PendingOpen {
    /// What, for the status line.
    pub label: String,
    /// Where it came from, for Save to default to (projects only).
    path: Option<PathBuf>,
    /// Rig it from layer names once open.
    auto_rig: bool,
    result: Receiver<Result<Opened>>,
}

fn kind_key(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Live2D => "library.kind.live2d",
        EntryKind::Psd => "library.kind.psd",
        EntryKind::Project => "library.kind.project",
        EntryKind::Image => "library.kind.image",
    }
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else {
        format!("{} KB", bytes.div_ceil(1000))
    }
}

impl EditorState {
    /// Show the library, listing the default sample folder the first time.
    pub fn open_sample_library(&mut self) {
        self.library.open = true;
        if !self.library.scanned {
            if self.library.folder.is_none() {
                self.library.folder = aether_io::library::default_sample_folders().into_iter().next();
            }
            self.rescan_library();
        }
    }

    /// List the library folder again.
    pub fn rescan_library(&mut self) {
        self.library.scanned = true;
        self.library.items = match &self.library.folder {
            Some(folder) => aether_io::library::scan_folder(folder),
            None => Vec::new(),
        };
    }

    /// List another folder.
    pub fn set_library_folder(&mut self, folder: PathBuf) {
        self.library.folder = Some(folder);
        self.rescan_library();
    }

    /// Open something from the library (on a worker thread).
    pub fn open_library_item(&mut self, item: &LibraryItem) {
        let auto_rig = self.library.auto_rig_psd && item.kind == EntryKind::Psd;
        let path = (item.entry.is_none() && item.kind == EntryKind::Project).then(|| item.file.clone());
        let item = item.clone();
        self.start_open(item.title(), path, auto_rig, move || {
            item.open(&Default::default())
        });
    }

    /// Open a ZIP archive: its one model, PSD or project straight away,
    /// otherwise ask which (see [`EditorState::archive_choice`]).
    pub fn open_archive(&mut self, file: PathBuf) -> Result<()> {
        let entries = Archive::open(&file)?.entries();
        let main: Vec<&ArchiveEntry> = entries.iter().filter(|e| e.kind != EntryKind::Image).collect();
        let only = match (main.as_slice(), entries.as_slice()) {
            ([one], _) => Some((*one).clone()),
            ([], [one]) => Some(one.clone()),
            _ => None,
        };
        match only {
            Some(entry) => self.open_archive_entry(file, &entry),
            None if entries.is_empty() => {
                return Err(aether_core::AetherError::UnsupportedFormat(format!(
                    "{} holds no model, PSD, project or image",
                    file.display()
                )))
            }
            None => self.archive_choice = Some(ArchiveChoice { file, entries }),
        }
        Ok(())
    }

    /// Open one entry of an archive (on a worker thread).
    pub fn open_archive_entry(&mut self, file: PathBuf, entry: &ArchiveEntry) {
        let item = LibraryItem {
            file,
            entry: Some(entry.path.clone()),
            kind: entry.kind,
            size: entry.size,
        };
        self.open_library_item(&item);
    }

    fn start_open(
        &mut self,
        label: String,
        path: Option<PathBuf>,
        auto_rig: bool,
        open: impl FnOnce() -> Result<Opened> + Send + 'static,
    ) {
        let (send, result) = channel();
        std::thread::spawn(move || {
            let _ = send.send(open());
        });
        self.status = format!("{} {label}", self.tr("library.opening"));
        self.pending_open = Some(PendingOpen {
            label,
            path,
            auto_rig,
            result,
        });
    }

    /// True while something is being opened.
    pub fn is_opening(&self) -> bool {
        self.pending_open.is_some()
    }

    /// Take in what a worker finished opening. Returns true when a document
    /// arrived (or failed to).
    pub fn poll_open(&mut self) -> bool {
        let Some(pending) = &self.pending_open else {
            return false;
        };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(aether_core::AetherError::invalid("the file could not be read"))
            }
        };
        let pending = self.pending_open.take().expect("checked above");
        match result {
            Ok(opened) => self.adopt_opened(opened, pending.path, pending.auto_rig),
            Err(error) => self.report_error(&format!("Open {}", pending.label), &error),
        }
        true
    }

    /// Make an opened document the current one, and say what it holds.
    pub fn adopt_opened(&mut self, opened: Opened, path: Option<PathBuf>, auto_rig: bool) {
        let live2d = !opened.document.rig.cubism.is_empty();
        let psd = !live2d && opened.document.layer_count() > 1 && path.is_none();
        self.set_document(opened.document, path);
        let summary = if live2d {
            let rig = &self.doc.rig;
            format!(
                "Opened Live2D model: {} parameters, {} motions, {} expressions",
                rig.parameters.len(),
                rig.motions.len(),
                rig.expressions.len()
            )
        } else if psd {
            format!(
                "Imported {} layers — {}",
                self.doc.layer_count(),
                self.tr("library.psd_hint")
            )
        } else {
            format!("Opened {}", self.doc.name)
        };
        self.status = crate::rigging::with_notes(summary, &opened.notes);
        if auto_rig {
            match self.auto_rig() {
                // The auto rig's own summary, parts left out and A/B
                // alternatives included.
                Ok(_) => {
                    self.status = format!("Imported {} layers — {}", self.doc.layer_count(), self.status)
                }
                Err(error) => self.report_error("Auto rig", &error),
            }
        }
    }
}

/// One openable item: an Open button, what it is, its size, its name and,
/// under it, where it is. Returns true when Open was clicked.
fn entry_row(
    ui: &mut egui::Ui,
    enabled: bool,
    open: &str,
    kind: &str,
    title: &str,
    location: &str,
    size: u64,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        clicked = ui.add_enabled(enabled, egui::Button::new(open)).clicked();
        ui.add_sized([96.0, 18.0], egui::Label::new(kind));
        ui.add_sized([64.0, 18.0], egui::Label::new(human_size(size)));
        ui.vertical(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate());
            ui.add(egui::Label::new(egui::RichText::new(location).weak()).truncate())
                .on_hover_text(location);
        });
    });
    ui.separator();
    clicked
}

/// The sample library window.
pub fn library_window(state: &mut EditorState, ctx: &egui::Context) {
    if !state.library.open {
        return;
    }
    let lang = state.language;
    let mut open = true;
    let mut chosen: Option<LibraryItem> = None;
    let mut choose_folder = false;
    let mut rescan = false;
    egui::Window::new(lang.tr("library.title"))
        .id(egui::Id::new("sample-library"))
        .default_width(560.0)
        .default_height(480.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(lang.tr("library.folder"));
                let folder = state
                    .library
                    .folder
                    .as_ref()
                    .map(|f| f.display().to_string())
                    .unwrap_or_else(|| "—".into());
                ui.monospace(folder);
            });
            ui.horizontal(|ui| {
                if ui.button(lang.tr("library.choose_folder")).clicked() {
                    choose_folder = true;
                }
                if ui.button(lang.tr("library.rescan")).clicked() {
                    rescan = true;
                }
                ui.checkbox(&mut state.library.auto_rig_psd, lang.tr("library.auto_rig_psd"));
            });
            ui.horizontal(|ui| {
                ui.label(lang.tr("library.filter"));
                ui.text_edit_singleline(&mut state.library.filter);
                let kinds = [
                    (None, "library.all"),
                    (Some(EntryKind::Live2D), "library.kind.live2d"),
                    (Some(EntryKind::Psd), "library.kind.psd"),
                    (Some(EntryKind::Image), "library.kind.image"),
                ];
                for (kind, key) in kinds {
                    if ui
                        .selectable_label(state.library.kind == kind, lang.tr(key))
                        .clicked()
                    {
                        state.library.kind = kind;
                    }
                }
            });
            ui.separator();
            if state.library.folder.is_none() {
                ui.label(lang.tr("library.no_folder"));
                return;
            }
            let root = state.library.folder.clone().unwrap_or_default();
            let visible = state.library.visible();
            if visible.is_empty() {
                ui.label(lang.tr("library.empty"));
                return;
            }
            let busy = state.pending_open.is_some();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for item in visible {
                        let open = entry_row(
                            ui,
                            !busy,
                            lang.tr("library.open"),
                            lang.tr(kind_key(item.kind)),
                            &item.title(),
                            &item.location(&root),
                            item.size,
                        );
                        if open {
                            chosen = Some(item.clone());
                        }
                    }
                });
        });
    if !open {
        state.library.open = false;
    }
    if choose_folder {
        let mut dialog = rfd::FileDialog::new();
        if let Some(folder) = &state.library.folder {
            dialog = dialog.set_directory(folder);
        }
        if let Some(folder) = dialog.pick_folder() {
            state.set_library_folder(folder);
        }
    } else if rescan {
        state.rescan_library();
    }
    if let Some(item) = chosen {
        state.open_library_item(&item);
    }
}

/// Which entry of an archive to open.
pub fn archive_choice_window(state: &mut EditorState, ctx: &egui::Context) {
    let Some(choice) = &state.archive_choice else {
        return;
    };
    let lang = state.language;
    let file = choice.file.clone();
    let entries = choice.entries.clone();
    let mut open = true;
    let mut chosen: Option<ArchiveEntry> = None;
    let mut cancel = false;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    egui::Window::new(format!("{} — {name}", lang.tr("archive.choose")))
        .id(egui::Id::new("archive-choice"))
        .collapsible(false)
        .default_width(480.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(lang.tr("archive.choose_hint"));
            ui.separator();
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for entry in &entries {
                    let open = entry_row(
                        ui,
                        true,
                        lang.tr("library.open"),
                        lang.tr(kind_key(entry.kind)),
                        entry.stem(),
                        &entry.path,
                        entry.size,
                    );
                    if open {
                        chosen = Some(entry.clone());
                    }
                }
            });
            ui.separator();
            if ui.button(lang.tr("dialog.cancel")).clicked() {
                cancel = true;
            }
        });
    if let Some(entry) = chosen {
        state.archive_choice = None;
        state.open_archive_entry(file, &entry);
    } else if cancel || !open {
        state.archive_choice = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn png() -> Vec<u8> {
        aether_io::image_io::encode_image(&aether_raster::Pixmap::new(8, 6), &Default::default()).unwrap()
    }

    fn zip_file(path: &std::path::Path, files: &[(&str, &[u8])]) {
        let mut out = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in files {
            out.start_file(*name, SimpleFileOptions::default()).unwrap();
            out.write_all(bytes).unwrap();
        }
        out.finish().unwrap();
    }

    fn wait(state: &mut EditorState) {
        for _ in 0..500 {
            if state.poll_open() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("opening never finished");
    }

    #[test]
    fn the_library_lists_a_folder_and_opens_from_archives() {
        let dir = tempfile::tempdir().unwrap();
        zip_file(
            &dir.path().join("pack.zip"),
            &[("art/a.png", &png()), ("art/b.png", &png())],
        );
        let mut state = EditorState::default();
        state.set_library_folder(dir.path().to_path_buf());
        state.library.open = true;
        assert_eq!(state.library.items.len(), 2);
        state.library.filter = "b".into();
        let visible: Vec<LibraryItem> = state.library.visible().into_iter().cloned().collect();
        assert_eq!(visible.len(), 1);
        state.open_library_item(&visible[0]);
        assert!(state.is_opening());
        wait(&mut state);
        assert_eq!(state.doc.name, "b");
        assert_eq!((state.doc.width, state.doc.height), (8, 6));
    }

    #[test]
    fn an_archive_with_several_things_asks_which() {
        let dir = tempfile::tempdir().unwrap();
        let several = dir.path().join("several.zip");
        zip_file(&several, &[("a.png", &png()), ("b.png", &png())]);
        let mut state = EditorState::default();
        state.open_archive(several.clone()).unwrap();
        let choice = state.archive_choice.as_ref().expect("asks");
        assert_eq!(choice.entries.len(), 2);
        let entry = choice.entries[1].clone();
        state.archive_choice = None;
        state.open_archive_entry(several, &entry);
        wait(&mut state);
        assert_eq!(state.doc.name, "b");

        let one = dir.path().join("one.zip");
        zip_file(&one, &[("only.png", &png()), ("readme.txt", b"hi")]);
        state.open_archive(one).unwrap();
        assert!(state.archive_choice.is_none(), "one thing opens straight away");
        wait(&mut state);
        assert_eq!(state.doc.name, "only");
    }
}
