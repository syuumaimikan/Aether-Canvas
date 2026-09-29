//! Finding and opening artwork wherever it is: a file, a folder of samples,
//! or an entry in a ZIP archive.
//!
//! A *spec* names one thing to open: a path (`haru.psd`,
//! `runtime/haru.model3.json`, a folder holding a model) or an entry of an
//! archive, `pack.zip#runtime/haru.model3.json`. A bare archive opens its
//! one model, PSD or project when it holds just one. [`scan_folder`] lists
//! everything openable under a folder, looking inside archives — the
//! editor's sample library, and `aether-canvas --list`.

use crate::archive::{is_archive, split_entry, Archive, EntryKind, Opened};
use crate::live2d_model::{import_live2d, Live2DImportOptions};
use aether_core::{AetherError, Result};
use aether_document::Document;
use std::path::{Path, PathBuf};

/// Something to open, found by [`scan_folder`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryItem {
    /// The file on disk (the archive, for an entry of one).
    pub file: PathBuf,
    /// The path inside the archive.
    pub entry: Option<String>,
    /// What it opens as.
    pub kind: EntryKind,
    /// Size in bytes (uncompressed, for an entry).
    pub size: u64,
}

impl LibraryItem {
    /// The spec that opens it: the path, or `archive.zip#entry`.
    pub fn spec(&self) -> String {
        match &self.entry {
            Some(entry) => format!("{}#{entry}", self.file.display()),
            None => self.file.display().to_string(),
        }
    }

    /// The name to show: the file name without its extension.
    pub fn title(&self) -> String {
        let name = match &self.entry {
            Some(entry) => entry
                .rsplit(['/', crate::archive::NESTED])
                .next()
                .unwrap_or(entry)
                .to_string(),
            None => self
                .file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        stem(&name).to_string()
    }

    /// Where it is, relative to `root`: `pack.zip › runtime`, `characters`.
    pub fn location(&self, root: &Path) -> String {
        let file = self.file.strip_prefix(root).unwrap_or(&self.file);
        let mut parts: Vec<String> = Vec::new();
        match &self.entry {
            Some(entry) => {
                parts.push(file.display().to_string());
                let inside = entry.replace(crate::archive::NESTED, " › ");
                if let Some((folder, _)) = inside.rsplit_once('/') {
                    parts.push(folder.to_string());
                }
            }
            None => {
                if let Some(folder) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
                    parts.push(folder.display().to_string());
                }
            }
        }
        parts.join(" › ")
    }

    /// Open it.
    pub fn open(&self, options: &Live2DImportOptions) -> Result<Opened> {
        match &self.entry {
            Some(entry) => Archive::open(&self.file)?.open_entry(entry, options),
            None => open_file(&self.file, options),
        }
    }
}

fn stem(name: &str) -> &str {
    if name.to_ascii_lowercase().ends_with(".model3.json") {
        return &name[..name.len() - ".model3.json".len()];
    }
    name.rsplit_once('.')
        .map(|(s, _)| s)
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
}

/// How deep [`scan_folder`] looks.
const MAX_DEPTH: usize = 6;

/// Everything under `dir` that can be opened, entries of archives
/// included, sorted by kind and then by path. A model's textures are left
/// out, as are hidden files and folders; archives that cannot be read are
/// skipped.
pub fn scan_folder(dir: &Path) -> Vec<LibraryItem> {
    let mut files = Vec::new();
    collect(dir, 0, &mut files);
    files.sort();
    let model_folders: Vec<PathBuf> = files
        .iter()
        .filter(|f| kind_of(f) == Some(EntryKind::Live2D))
        .filter_map(|f| f.parent().map(Path::to_path_buf))
        .collect();
    let mut items = Vec::new();
    for file in files {
        if is_archive(&file) {
            let Ok(archive) = Archive::open(&file) else {
                continue;
            };
            items.extend(archive.entries().into_iter().map(|e| LibraryItem {
                file: file.clone(),
                entry: Some(e.path),
                kind: e.kind,
                size: e.size,
            }));
            continue;
        }
        let Some(kind) = kind_of(&file) else { continue };
        if kind == EntryKind::Image && model_folders.iter().any(|m| file.starts_with(m)) {
            continue;
        }
        let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        items.push(LibraryItem {
            file,
            entry: None,
            kind,
            size,
        });
    }
    items.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.spec().cmp(&b.spec())));
    items
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        let hidden = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.') || n == "__MACOSX");
        if hidden {
            continue;
        }
        if path.is_dir() {
            if depth < MAX_DEPTH {
                collect(&path, depth + 1, out);
            }
        } else {
            out.push(path);
        }
    }
}

/// What the file at `path` opens as, by its name.
pub fn kind_of(path: &Path) -> Option<EntryKind> {
    EntryKind::of(&path.file_name()?.to_string_lossy())
}

/// Open a spec (see the module documentation).
pub fn open_spec(spec: &str, options: &Live2DImportOptions) -> Result<Opened> {
    match split_entry(spec) {
        (file, Some(entry)) => Archive::open(file)?.open_entry(entry, options),
        (file, None) => open_file(Path::new(file), options),
    }
}

/// Open a file or folder on disk: a project, PSD, Live2D model (or the
/// folder holding one), image, or an archive holding one thing to open.
pub fn open_file(path: &Path, options: &Live2DImportOptions) -> Result<Opened> {
    let plain = |document: Document| Opened {
        document,
        notes: Vec::new(),
    };
    if path.is_dir() {
        let opened = import_live2d(path, options)?;
        return Ok(Opened {
            document: opened.document,
            notes: opened.notes,
        });
    }
    if is_archive(path) {
        let archive = Archive::open(path)?;
        let entries = archive.entries();
        let main: Vec<_> = entries.iter().filter(|e| e.kind != EntryKind::Image).collect();
        let only = match (main.as_slice(), entries.as_slice()) {
            ([one], _) => Some(*one),
            ([], [one]) => Some(one),
            _ => None,
        };
        return match only {
            Some(entry) => archive.open_entry(&entry.path, options),
            None if entries.is_empty() => Err(AetherError::UnsupportedFormat(format!(
                "{} holds no model, PSD, project or image",
                path.display()
            ))),
            None => {
                let list: Vec<String> = entries
                    .iter()
                    .map(|e| format!("  {}#{}  ({})", path.display(), e.path, e.kind.label()))
                    .collect();
                Err(AetherError::invalid(format!(
                    "{} holds {} things to open; name one:\n{}",
                    path.display(),
                    entries.len(),
                    list.join("\n")
                )))
            }
        };
    }
    match kind_of(path) {
        Some(EntryKind::Live2D) => {
            let opened = import_live2d(path, options)?;
            Ok(Opened {
                document: opened.document,
                notes: opened.notes,
            })
        }
        Some(EntryKind::Psd) => Ok(plain(crate::psd::load_psd_file(path)?)),
        Some(EntryKind::Project) => Ok(plain(crate::project::load_project(path)?)),
        _ => {
            let pixmap = crate::image_io::load_image(path)?;
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut doc = Document::new(pixmap.width(), pixmap.height(), name);
            if let Some(target) = doc.layers.get_mut(doc.active_layer).and_then(|l| l.pixmap_mut()) {
                *target = pixmap;
            }
            Ok(plain(doc))
        }
    }
}

/// Where the sample library looks by default, in order: `AETHER_SAMPLES`,
/// `assets_sample` in the working folder, then beside the executable and
/// up to three folders above it (so `target/release/…` finds the one in a
/// source checkout). Only folders that exist are returned.
pub fn default_sample_folders() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("AETHER_SAMPLES") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("assets_sample"));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        for _ in 0..4 {
            let Some(d) = dir else { break };
            candidates.push(d.join("assets_sample"));
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for c in candidates {
        if c.is_dir() {
            let c = c.canonicalize().unwrap_or(c);
            if !found.contains(&c) {
                found.push(c);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn zip_file(path: &Path, files: &[(&str, &[u8])]) {
        let mut out = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in files {
            out.start_file(*name, SimpleFileOptions::default()).unwrap();
            out.write_all(bytes).unwrap();
        }
        out.finish().unwrap();
    }

    fn png() -> Vec<u8> {
        crate::image_io::encode_image(&aether_raster::Pixmap::new(6, 4), &Default::default()).unwrap()
    }

    #[test]
    fn a_sample_folder_lists_files_and_what_archives_hold() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("loose/model/tex")).unwrap();
        std::fs::write(root.join("loose/model/a.model3.json"), "{}").unwrap();
        std::fs::write(root.join("loose/model/tex/texture_00.png"), png()).unwrap();
        std::fs::write(root.join("loose/art.png"), png()).unwrap();
        std::fs::write(root.join(".hidden.psd"), "8BPS").unwrap();
        zip_file(
            &root.join("pack.zip"),
            &[("pack/b.psd", b"8BPS"), ("pack/runtime/b.model3.json", b"{}")],
        );
        let items = scan_folder(root);
        let specs: Vec<(String, EntryKind)> = items
            .iter()
            .map(|i| {
                let spec = i.spec();
                let rel = spec
                    .strip_prefix(&format!("{}/", root.display()))
                    .unwrap_or(&spec)
                    .to_string();
                (rel, i.kind)
            })
            .collect();
        assert_eq!(
            specs,
            vec![
                ("loose/model/a.model3.json".to_string(), EntryKind::Live2D),
                (
                    "pack.zip#pack/runtime/b.model3.json".to_string(),
                    EntryKind::Live2D
                ),
                ("pack.zip#pack/b.psd".to_string(), EntryKind::Psd),
                ("loose/art.png".to_string(), EntryKind::Image),
            ]
        );
        assert_eq!(items[1].title(), "b");
        assert_eq!(items[1].location(root), "pack.zip › pack/runtime");
        assert_eq!(items[3].location(root), "loose");
    }

    #[test]
    fn archives_open_their_only_thing_or_say_what_to_choose() {
        let dir = tempfile::tempdir().unwrap();
        let one = dir.path().join("one.zip");
        zip_file(&one, &[("art/cover.png", &png()), ("readme.txt", b"hi")]);
        let opened = open_file(&one, &Default::default()).unwrap();
        assert_eq!(opened.document.name, "cover");
        let spec = format!("{}#art/cover.png", one.display());
        assert_eq!(open_spec(&spec, &Default::default()).unwrap().document.width, 6);

        let two = dir.path().join("two.zip");
        zip_file(&two, &[("a.png", &png()), ("b.png", &png())]);
        let Err(error) = open_file(&two, &Default::default()) else {
            panic!("two images need a choice")
        };
        let message = error.to_string();
        assert!(
            message.contains("two.zip#a.png") && message.contains("two.zip#b.png"),
            "{message}"
        );
    }
}
