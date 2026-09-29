//! Opening what comes packed in a ZIP archive.
//!
//! Characters are usually handed around zipped: a Live2D sample download
//! holds the runtime model (`.model3.json` with its `.moc3`, textures,
//! physics and motions) next to the layered PSDs it was made from, and
//! packs of parts or projects arrive the same way. An [`Archive`] lists what
//! in it can be opened and opens any of it straight from the archive, with
//! nothing unpacked to disk:
//!
//! * **Live2D models** — every file the `.model3.json` refers to is read
//!   from beside it in the archive;
//! * **PSDs** — layered, as [`crate::psd`] reads them;
//! * **Aether projects**, and **images** that are not a model's textures;
//! * **archives inside the archive**, one level deep: their contents are
//!   listed as `outer.zip|inside/path`.
//!
//! Names are decoded as UTF-8, or as Shift_JIS when they are not valid
//! UTF-8 (archives made on Japanese Windows), so `素材分け.psd` shows as
//! itself. Folders macOS adds (`__MACOSX/`, `._*`) are skipped.

use crate::live2d_model::{import_live2d_with, Live2DImportOptions};
use aether_core::{AetherError, Result};
use aether_document::Document;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::Path;

/// Separates an archive inside the archive from a path within it.
pub const NESTED: char = '|';

/// What an entry opens as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    /// A Live2D Cubism model (`.model3.json`).
    Live2D,
    /// A layered Photoshop file.
    Psd,
    /// An Aether project.
    Project,
    /// A picture.
    Image,
}

impl EntryKind {
    /// The kind of the file at `path`, if it can be opened.
    pub fn of(path: &str) -> Option<Self> {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".model3.json") {
            return Some(Self::Live2D);
        }
        let extension = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        match extension {
            "psd" => Some(Self::Psd),
            "aether" => Some(Self::Project),
            "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "tif" | "tiff" => Some(Self::Image),
            _ => None,
        }
    }

    /// A short English label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Live2D => "Live2D model",
            Self::Psd => "PSD",
            Self::Project => "Aether project",
            Self::Image => "Image",
        }
    }
}

/// Something in an archive that can be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Path in the archive, `/`-separated; an entry of an archive inside
    /// the archive is `outer.zip|inside/path`.
    pub path: String,
    /// What it opens as.
    pub kind: EntryKind,
    /// Uncompressed size, bytes.
    pub size: u64,
}

impl ArchiveEntry {
    /// The file name, without folders.
    pub fn name(&self) -> &str {
        let last = self.path.rsplit(NESTED).next().unwrap_or(&self.path);
        last.rsplit('/').next().unwrap_or(last)
    }

    /// The name without its extension (`haru.model3.json` → `haru`).
    pub fn stem(&self) -> &str {
        let name = self.name();
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".model3.json") {
            return &name[..name.len() - ".model3.json".len()];
        }
        name.rsplit_once('.')
            .map(|(s, _)| s)
            .filter(|s| !s.is_empty())
            .unwrap_or(name)
    }
}

/// An opened entry.
pub struct Opened {
    /// The document.
    pub document: Document,
    /// What was left out or approximated.
    pub notes: Vec<String>,
}

trait Source: Read + Seek {}
impl<T: Read + Seek> Source for T {}

/// A ZIP archive, open for reading.
pub struct Archive {
    zip: RefCell<zip::ZipArchive<Box<dyn Source>>>,
    /// Decoded, normalised name → index, files only.
    files: BTreeMap<String, usize>,
}

/// Decode an entry name: UTF-8, else Shift_JIS, else what the ZIP library
/// made of it.
fn decode_name(raw: &[u8], fallback: &str) -> String {
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.to_string();
    }
    let (text, _, errors) = encoding_rs::SHIFT_JIS.decode(raw);
    if !errors {
        return text.into_owned();
    }
    fallback.to_string()
}

/// `a/./b/../c` → `a/c`, with `\` read as `/`.
fn normalise(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn is_junk(path: &str) -> bool {
    path.starts_with("__MACOSX/") || path.rsplit('/').next().is_some_and(|n| n.starts_with("._"))
}

impl Archive {
    /// Open the archive at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(path.as_ref())?;
        Self::from_source(Box::new(BufReader::new(file)))
    }

    /// Open an archive held in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::from_source(Box::new(Cursor::new(bytes)))
    }

    fn from_source(source: Box<dyn Source>) -> Result<Self> {
        let mut zip = zip::ZipArchive::new(source)
            .map_err(|e| AetherError::UnsupportedFormat(format!("not a ZIP archive: {e}")))?;
        let mut files = BTreeMap::new();
        for i in 0..zip.len() {
            let Ok(file) = zip.by_index_raw(i) else { continue };
            if !file.is_file() {
                continue;
            }
            let name = normalise(&decode_name(file.name_raw(), file.name()));
            if name.is_empty() || is_junk(&name) {
                continue;
            }
            files.entry(name).or_insert(i);
        }
        Ok(Self {
            zip: RefCell::new(zip),
            files,
        })
    }

    /// Every file in the archive (decoded, `/`-separated paths).
    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    fn size_of(&self, index: usize) -> u64 {
        self.zip
            .borrow_mut()
            .by_index_raw(index)
            .map(|f| f.size())
            .unwrap_or(0)
    }

    /// What can be opened, Live2D models first, then PSDs, projects and
    /// images; archives inside are listed through, one level deep.
    pub fn entries(&self) -> Vec<ArchiveEntry> {
        self.entries_at(0)
    }

    fn entries_at(&self, depth: usize) -> Vec<ArchiveEntry> {
        // A model's textures live in or under its folder: not worth listing.
        let model_folders: Vec<&str> = self
            .files
            .keys()
            .filter(|p| EntryKind::of(p) == Some(EntryKind::Live2D))
            .map(|p| folder_of(p))
            .collect();
        let in_model_folder = |p: &str| {
            model_folders
                .iter()
                .any(|f| f.is_empty() || p.starts_with(&format!("{f}/")))
        };
        let mut out = Vec::new();
        for (path, &index) in &self.files {
            if path.to_ascii_lowercase().ends_with(".zip") {
                if depth > 0 {
                    continue;
                }
                let Ok(inner) = self.read(path).and_then(Archive::from_bytes) else {
                    continue;
                };
                out.extend(inner.entries_at(depth + 1).into_iter().map(|e| ArchiveEntry {
                    path: format!("{path}{NESTED}{}", e.path),
                    ..e
                }));
                continue;
            }
            let Some(kind) = EntryKind::of(path) else { continue };
            if kind == EntryKind::Image && in_model_folder(path) {
                continue;
            }
            out.push(ArchiveEntry {
                path: path.clone(),
                kind,
                size: self.size_of(index),
            });
        }
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.path.cmp(&b.path)));
        out
    }

    /// The bytes of the file at `path` (matched exactly, then ignoring
    /// case, as archives made on Windows sometimes need).
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let wanted = normalise(path);
        let index = self.files.get(&wanted).copied().or_else(|| {
            let lower = wanted.to_lowercase();
            self.files
                .iter()
                .find(|(name, _)| name.to_lowercase() == lower)
                .map(|(_, &i)| i)
        });
        let index = index.ok_or_else(|| AetherError::Asset(format!("{wanted} is not in the archive")))?;
        let mut zip = self.zip.borrow_mut();
        let mut file = zip
            .by_index(index)
            .map_err(|e| AetherError::UnsupportedFormat(format!("{wanted}: {e}")))?;
        let mut bytes = Vec::with_capacity(file.size().min(1 << 30) as usize);
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Open the entry at `path` (as [`Archive::entries`] lists it).
    pub fn open_entry(&self, path: &str, options: &Live2DImportOptions) -> Result<Opened> {
        if let Some((outer, inner)) = path.split_once(NESTED) {
            let nested = Archive::from_bytes(self.read(outer)?)?;
            return nested.open_entry(inner, options);
        }
        let path = normalise(path);
        let kind = EntryKind::of(&path)
            .ok_or_else(|| AetherError::UnsupportedFormat(format!("{path}: cannot be opened")))?;
        let entry = ArchiveEntry {
            path: path.clone(),
            kind,
            size: 0,
        };
        let name = entry.stem().to_string();
        let plain = |document: Document| Opened {
            document,
            notes: Vec::new(),
        };
        match kind {
            EntryKind::Live2D => {
                let json = String::from_utf8(self.read(&path)?)
                    .map_err(|_| AetherError::serialization(format!("{path} is not UTF-8 text")))?;
                // Byte-order marks are legal in JSON files written on Windows.
                let json = json.trim_start_matches('\u{feff}');
                let folder = folder_of(&path).to_string();
                let read = |file: &str| {
                    let full = if folder.is_empty() {
                        file.to_string()
                    } else {
                        format!("{folder}/{file}")
                    };
                    self.read(&full)
                };
                let opened = import_live2d_with(json, &name, &read, options)?;
                Ok(Opened {
                    document: opened.document,
                    notes: opened.notes,
                })
            }
            EntryKind::Psd => {
                let mut doc = crate::psd::load_psd(&self.read(&path)?)?;
                doc.name = name;
                Ok(plain(doc))
            }
            EntryKind::Project => Ok(plain(crate::project::deserialize_project(&self.read(&path)?)?)),
            EntryKind::Image => {
                let pixmap = crate::image_io::decode_image(&self.read(&path)?)?;
                let mut doc = Document::new(pixmap.width(), pixmap.height(), name);
                if let Some(target) = doc.layers.get_mut(doc.active_layer).and_then(|l| l.pixmap_mut()) {
                    *target = pixmap;
                }
                Ok(plain(doc))
            }
        }
    }
}

/// Split `archive.zip#inside/path` into the archive and the entry. Paths
/// without `#` after `.zip` are returned whole.
pub fn split_entry(path: &str) -> (&str, Option<&str>) {
    let lower = path.to_ascii_lowercase();
    match lower.find(".zip#") {
        Some(at) => (&path[..at + 4], Some(&path[at + 5..])),
        None => (path, None),
    }
}

/// Is `path` a ZIP archive, by its name?
pub fn is_archive(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    /// A ZIP of `(name bytes, contents)`, names written as given (so they
    /// can be Shift_JIS, as Japanese Windows writes them).
    fn zip_of(files: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            // The ZIP writer takes names as text; raw bytes that are not
            // UTF-8 are patched in below.
            let placeholder = format!("{:0width$}", 0, width = name.len());
            let text = std::str::from_utf8(name)
                .map(str::to_string)
                .unwrap_or(placeholder);
            out.start_file(text, SimpleFileOptions::default()).unwrap();
            out.write_all(bytes).unwrap();
        }
        let mut bytes = out.finish().unwrap().into_inner();
        for (name, _) in files {
            if std::str::from_utf8(name).is_err() {
                let placeholder = vec![b'0'; name.len()];
                let mut at = 0;
                while let Some(i) = bytes[at..]
                    .windows(name.len())
                    .position(|w| w == placeholder.as_slice())
                {
                    bytes[at + i..at + i + name.len()].copy_from_slice(name);
                    at += i + name.len();
                }
            }
        }
        bytes
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let pixmap = aether_raster::Pixmap::new(w, h);
        crate::image_io::encode_image(&pixmap, &Default::default()).unwrap()
    }

    #[test]
    fn entries_are_found_sorted_and_textures_left_out() {
        let texture = png(4, 4);
        let bytes = zip_of(&[
            (b"pack/readme.txt", b"hello"),
            (b"pack/runtime/haru.model3.json", b"{}"),
            (b"pack/runtime/haru.2048/texture_00.png", &texture),
            (b"pack/cover.png", &texture),
            (b"pack/parts.psd", b"8BPS"),
            (b"__MACOSX/pack/._parts.psd", b"junk"),
        ]);
        let archive = Archive::from_bytes(bytes).unwrap();
        let entries = archive.entries();
        let listed: Vec<(&str, EntryKind)> = entries.iter().map(|e| (e.path.as_str(), e.kind)).collect();
        assert_eq!(
            listed,
            vec![
                ("pack/runtime/haru.model3.json", EntryKind::Live2D),
                ("pack/parts.psd", EntryKind::Psd),
                ("pack/cover.png", EntryKind::Image),
            ]
        );
        assert_eq!(entries[0].stem(), "haru");
        assert_eq!(entries[1].name(), "parts.psd");
    }

    #[test]
    fn shift_jis_names_are_decoded() {
        let (name, _, _) = encoding_rs::SHIFT_JIS.encode("キャラ/素材分け.psd");
        assert!(std::str::from_utf8(&name).is_err());
        let bytes = zip_of(&[(&name, b"8BPS")]);
        let archive = Archive::from_bytes(bytes).unwrap();
        let entries = archive.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "キャラ/素材分け.psd");
        assert_eq!(archive.read("キャラ/素材分け.psd").unwrap(), b"8BPS");
    }

    #[test]
    fn images_open_and_nested_archives_list_through() {
        let inner = zip_of(&[(b"art/face.png", &png(3, 2))]);
        let outer = zip_of(&[(b"samples/one.zip", &inner), (b"Cover.PNG", &png(5, 5))]);
        let archive = Archive::from_bytes(outer).unwrap();
        let paths: Vec<String> = archive.entries().into_iter().map(|e| e.path).collect();
        assert_eq!(paths, vec!["Cover.PNG", "samples/one.zip|art/face.png"]);
        let opened = archive
            .open_entry("samples/one.zip|art/face.png", &Default::default())
            .unwrap();
        assert_eq!((opened.document.width, opened.document.height), (3, 2));
        assert_eq!(opened.document.name, "face");
        // Case-insensitive lookup, and `.`/`..` in paths.
        assert!(archive.read("./x/../cover.png").is_ok());
    }

    #[test]
    fn entry_paths_split_after_the_archive() {
        assert_eq!(split_entry("a/b.zip#c/d.psd"), ("a/b.zip", Some("c/d.psd")));
        assert_eq!(
            split_entry("C:\\s\\B.ZIP#x.model3.json"),
            ("C:\\s\\B.ZIP", Some("x.model3.json"))
        );
        assert_eq!(split_entry("a/b.psd"), ("a/b.psd", None));
        assert!(is_archive(Path::new("x/Y.Zip")));
    }
}
