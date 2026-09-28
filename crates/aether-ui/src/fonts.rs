//! Font setup.
//!
//! egui ships Latin, symbol and emoji fonts but nothing for Japanese, Chinese
//! or Korean, so without help the Japanese UI would render as empty boxes.
//! Rather than bundling a multi-megabyte CJK font, the application looks for
//! one the operating system already has and installs it as a fallback: text
//! still uses egui's fonts first, and CJK characters fall through to the
//! system font.
//!
//! `AETHER_CJK_FONT` can point at any `.ttf`/`.otf`/`.ttc` file to override
//! the search.

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Well-known locations of CJK-capable fonts, most preferred first.
const CANDIDATES: &[&str] = &[
    // Windows
    "C:\\Windows\\Fonts\\YuGothM.ttc",
    "C:\\Windows\\Fonts\\YuGothR.ttc",
    "C:\\Windows\\Fonts\\meiryo.ttc",
    "C:\\Windows\\Fonts\\msgothic.ttc",
    // macOS
    "/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc",
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/Library/Fonts/Arial Unicode.ttf",
    // Linux
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJKjp-Regular.otf",
    "/usr/share/fonts/truetype/noto/NotoSansJP-Regular.ttf",
    "/usr/share/fonts/opentype/ipafont-gothic/ipagp.ttf",
    "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf",
    "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
    "/usr/share/fonts/truetype/takao-gothic/TakaoPGothic.ttf",
    "/usr/share/fonts/truetype/vlgothic/VL-PGothic-Regular.ttf",
];

/// The first CJK font found on this machine.
pub fn find_cjk_font() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AETHER_CJK_FONT").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    CANDIDATES
        .iter()
        .map(Path::new)
        .find(|p| p.is_file())
        .map(Path::to_path_buf)
}

/// Add a system CJK font as a fallback for every font family.
///
/// Returns the font used, or `None` when the machine has none (the UI then
/// still works; only CJK text shows as boxes).
pub fn install_cjk_fallback(ctx: &egui::Context) -> Option<PathBuf> {
    let path = find_cjk_font()?;
    let bytes = std::fs::read(&path).ok()?;
    let mut definitions = egui::FontDefinitions::default();
    definitions
        .font_data
        .insert("cjk-fallback".into(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        definitions
            .families
            .entry(family)
            .or_default()
            .push("cjk-fallback".into());
    }
    ctx.set_fonts(definitions);
    tracing::info!("CJK fallback font: {}", path.display());
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_font_path_wins_when_it_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("font.ttf");
        std::fs::write(&path, b"not really a font").expect("write");
        // No other test reads this variable, so setting it cannot race.
        std::env::set_var("AETHER_CJK_FONT", &path);
        assert_eq!(find_cjk_font(), Some(path));
        std::env::remove_var("AETHER_CJK_FONT");
    }
}
