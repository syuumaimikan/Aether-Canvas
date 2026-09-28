//! Icon glyphs.
//!
//! The UI draws its icons as text, so an icon only works if one of the fonts
//! in egui's proportional family actually has the glyph — otherwise it shows
//! as an empty box. Every icon lives here, and a test parses the bundled
//! fonts' character maps to prove each one renders.

/// Brush tool.
pub const BRUSH: &str = "✏";
/// Eraser tool.
pub const ERASER: &str = "⬜";
/// Flood fill.
pub const BUCKET: &str = "🌊";
/// Eyedropper.
pub const EYEDROPPER: &str = "💧";
/// Rectangular marquee.
pub const RECT_SELECT: &str = "□";
/// Elliptical marquee.
pub const ELLIPSE_SELECT: &str = "○";
/// Lasso.
pub const LASSO: &str = "〰";
/// Magic wand.
pub const WAND: &str = "✨";
/// Move tool.
pub const MOVE: &str = "✚";
/// Transform tool.
pub const TRANSFORM: &str = "📐";
/// Liquify tool.
pub const LIQUIFY: &str = "🌀";
/// Pan tool.
pub const PAN: &str = "✋";
/// Mesh tool and mesh nodes.
pub const MESH: &str = "🕸";
/// Deform tool.
pub const DEFORM: &str = "👆";
/// Bone tool and bone nodes.
pub const BONE: &str = "💪";
/// Plugin tools.
pub const PLUGIN: &str = "⚙";
/// Warp deformer nodes.
pub const WARP: &str = "⊞";
/// Rotation deformer nodes.
pub const ROTATION: &str = "⟲";
/// Add.
pub const ADD: &str = "➕";
/// New group.
pub const GROUP: &str = "🗀";
/// Duplicate.
pub const DUPLICATE: &str = "🗐";
/// Delete.
pub const DELETE: &str = "🗑";
/// Move up.
pub const UP: &str = "⬆";
/// Move down.
pub const DOWN: &str = "⬇";
/// Merge down.
pub const MERGE_DOWN: &str = "⏬";
/// Swap colours.
pub const SWAP: &str = "🔁";
/// Layer mask.
pub const MASK: &str = "🎭";
/// A key at the current value.
pub const KEY_ON: &str = "♦";
/// No key at the current value.
pub const KEY_OFF: &str = "◊";
/// Driven by an expression.
pub const DRIVEN: &str = "ƒ";
/// Animated in the current motion.
pub const ANIMATED: &str = "⏺";
/// Close / remove from a list.
pub const CLOSE: &str = "✕";
/// Warning.
pub const WARNING: &str = "⚠";
/// Go to start.
pub const TO_START: &str = "⏮";
/// Previous frame.
pub const PREV_FRAME: &str = "⏪";
/// Play.
pub const PLAY: &str = "▶";
/// Pause.
pub const PAUSE: &str = "⏸";
/// Next frame.
pub const NEXT_FRAME: &str = "⏩";
/// Stop.
pub const STOP: &str = "⏹";
/// Unsaved changes.
pub const UNSAVED: &str = "●";

/// Every icon, for the coverage test.
pub const ALL: &[&str] = &[
    BRUSH,
    ERASER,
    BUCKET,
    EYEDROPPER,
    RECT_SELECT,
    ELLIPSE_SELECT,
    LASSO,
    WAND,
    MOVE,
    TRANSFORM,
    LIQUIFY,
    PAN,
    MESH,
    DEFORM,
    BONE,
    PLUGIN,
    WARP,
    ROTATION,
    ADD,
    GROUP,
    DUPLICATE,
    DELETE,
    UP,
    DOWN,
    MERGE_DOWN,
    SWAP,
    MASK,
    KEY_ON,
    KEY_OFF,
    DRIVEN,
    ANIMATED,
    CLOSE,
    WARNING,
    TO_START,
    PREV_FRAME,
    PLAY,
    PAUSE,
    NEXT_FRAME,
    STOP,
    UNSAVED,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn u16_at(d: &[u8], i: usize) -> Option<u16> {
        Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]))
    }

    fn u32_at(d: &[u8], i: usize) -> Option<u32> {
        Some(u32::from_be_bytes([
            *d.get(i)?,
            *d.get(i + 1)?,
            *d.get(i + 2)?,
            *d.get(i + 3)?,
        ]))
    }

    /// Code points covered by a TrueType font (cmap formats 4 and 12).
    fn coverage(d: &[u8]) -> HashSet<u32> {
        let mut out = HashSet::new();
        let tables = u16_at(d, 4).unwrap_or(0) as usize;
        let cmap = (0..tables).find_map(|i| {
            let at = 12 + 16 * i;
            (d.get(at..at + 4) == Some(b"cmap"))
                .then(|| u32_at(d, at + 8))
                .flatten()
        });
        let Some(cmap) = cmap.map(|o| o as usize) else {
            return out;
        };
        let subtables = u16_at(d, cmap + 2).unwrap_or(0) as usize;
        for i in 0..subtables {
            let Some(offset) = u32_at(d, cmap + 4 + 8 * i + 4) else {
                continue;
            };
            let st = cmap + offset as usize;
            match u16_at(d, st) {
                Some(4) => {
                    let segs = u16_at(d, st + 6).unwrap_or(0) as usize / 2;
                    for s in 0..segs {
                        let end = u16_at(d, st + 14 + 2 * s).unwrap_or(0) as u32;
                        let start = u16_at(d, st + 16 + 2 * segs + 2 * s).unwrap_or(1) as u32;
                        out.extend(start..=end);
                    }
                }
                Some(12) => {
                    let groups = u32_at(d, st + 12).unwrap_or(0) as usize;
                    for g in 0..groups {
                        let start = u32_at(d, st + 16 + 12 * g).unwrap_or(1);
                        let end = u32_at(d, st + 20 + 12 * g).unwrap_or(0);
                        out.extend(start..=end);
                    }
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn every_icon_has_a_glyph_in_the_proportional_fonts() {
        let definitions = egui::FontDefinitions::default();
        let family = &definitions.families[&egui::FontFamily::Proportional];
        let mut covered = HashSet::new();
        for name in family {
            if let Some(data) = definitions.font_data.get(name) {
                covered.extend(coverage(&data.font));
            }
        }
        assert!(covered.contains(&('A' as u32)), "the cmap parser found no glyphs");
        let missing: Vec<&str> = ALL
            .iter()
            .copied()
            .filter(|icon| icon.chars().any(|c| !covered.contains(&(c as u32))))
            .collect();
        assert!(
            missing.is_empty(),
            "these icons would render as empty boxes: {missing:?}"
        );
    }
}
