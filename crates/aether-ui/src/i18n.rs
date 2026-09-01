//! Localisation.
//!
//! No user-visible string is written inline in a widget. Every one goes through
//! [`Language::tr`], which looks the key up in a table. Adding a language means
//! adding a column, not hunting through the UI code.
//!
//! Keys are dotted and descriptive (`menu.file.save_as`), and an unknown key
//! renders as the key itself rather than panicking, so a missing translation is
//! visible but never fatal.

use serde::{Deserialize, Serialize};

/// Languages shipped with the application.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// English.
    #[default]
    English,
    /// Japanese.
    Japanese,
}

impl Language {
    /// All languages, in menu order.
    pub const ALL: [Language; 2] = [Language::English, Language::Japanese];

    /// The name of the language in the language itself.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Japanese => "日本語",
        }
    }

    /// Look up `key`, falling back to English and then to the key itself.
    pub fn tr(self, key: &str) -> &'static str {
        let table = match self {
            Language::English => EN,
            Language::Japanese => JA,
        };
        lookup(table, key)
            .or_else(|| lookup(EN, key))
            .unwrap_or_else(|| leak_key(key))
    }
}

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|i| table[i].1)
}

/// Missing keys are shown verbatim; leaking the few that occur during
/// development is cheaper than a panic in front of a user.
fn leak_key(key: &str) -> &'static str {
    Box::leak(key.to_string().into_boxed_str())
}

/// English strings. **Keep sorted by key** — lookup is a binary search.
static EN: &[(&str, &str)] = &[
    ("brush.blend", "Blend"),
    ("brush.flow", "Flow"),
    ("brush.hardness", "Hardness"),
    ("brush.opacity", "Opacity"),
    ("brush.presets", "Presets"),
    ("brush.size", "Size"),
    ("brush.smoothing", "Stabilizer"),
    ("brush.spacing", "Spacing"),
    ("brush.title", "Brush"),
    ("canvas.title", "Canvas"),
    ("color.hex", "Hex"),
    ("color.palette", "Palette"),
    ("color.primary", "Primary"),
    ("color.secondary", "Secondary"),
    ("color.swap", "Swap"),
    ("color.title", "Color"),
    ("dialog.cancel", "Cancel"),
    ("dialog.create", "Create"),
    ("dialog.height", "Height"),
    ("dialog.name", "Name"),
    ("dialog.new_document", "New Document"),
    ("dialog.unsaved_body", "This document has unsaved changes."),
    ("dialog.unsaved_discard", "Discard changes"),
    ("dialog.unsaved_title", "Unsaved changes"),
    ("dialog.width", "Width"),
    ("history.title", "History"),
    ("layer.add", "Add layer"),
    ("layer.add_group", "Add group"),
    ("layer.alpha_lock", "Lock transparency"),
    ("layer.blend", "Blend"),
    ("layer.clipping", "Clip to layer below"),
    ("layer.delete", "Delete layer"),
    ("layer.duplicate", "Duplicate layer"),
    ("layer.lock", "Lock"),
    ("layer.mask_add", "Add mask from selection"),
    ("layer.mask_remove", "Remove mask"),
    ("layer.merge_down", "Merge down"),
    ("layer.move_down", "Move down"),
    ("layer.move_up", "Move up"),
    ("layer.opacity", "Opacity"),
    ("layer.title", "Layers"),
    ("layer.visible", "Visible"),
    ("menu.edit", "Edit"),
    ("menu.edit.clear", "Clear Layer"),
    ("menu.edit.deselect", "Deselect"),
    ("menu.edit.invert_selection", "Invert Selection"),
    ("menu.edit.redo", "Redo"),
    ("menu.edit.select_all", "Select All"),
    ("menu.edit.undo", "Undo"),
    ("menu.file", "File"),
    ("menu.file.export_png", "Export PNG..."),
    ("menu.file.new", "New..."),
    ("menu.file.open", "Open..."),
    ("menu.file.quit", "Quit"),
    ("menu.file.save", "Save"),
    ("menu.file.save_as", "Save As..."),
    ("menu.help", "Help"),
    ("menu.help.about", "About Aether Canvas"),
    ("menu.help.shortcuts", "Keyboard Shortcuts"),
    ("menu.image", "Image"),
    ("menu.image.resize", "Canvas Size..."),
    ("menu.language", "Language"),
    ("menu.layer", "Layer"),
    ("menu.view", "View"),
    ("menu.view.fit", "Fit to Window"),
    ("menu.view.mirror", "Mirror Canvas"),
    ("menu.view.pixel_grid", "Pixel Grid"),
    ("menu.view.reset", "Actual Size"),
    ("menu.view.rotate_left", "Rotate View Left"),
    ("menu.view.rotate_reset", "Reset Rotation"),
    ("menu.view.rotate_right", "Rotate View Right"),
    ("menu.view.zoom_in", "Zoom In"),
    ("menu.view.zoom_out", "Zoom Out"),
    ("menu.window", "Window"),
    ("menu.window.theme", "Theme"),
    ("menu.workspace", "Workspace"),
    ("properties.title", "Properties"),
    ("status.ready", "Ready"),
    ("theme.dark", "Dark"),
    ("theme.high_contrast", "High Contrast"),
    ("theme.light", "Light"),
    ("tool.brush", "Brush"),
    ("tool.bucket", "Bucket Fill"),
    ("tool.ellipse_select", "Ellipse Select"),
    ("tool.eraser", "Eraser"),
    ("tool.eyedropper", "Eyedropper"),
    ("tool.lasso", "Lasso Select"),
    ("tool.move", "Move Layer"),
    ("tool.options", "Tool Options"),
    ("tool.pan", "Pan"),
    ("tool.rect_select", "Rectangle Select"),
    ("tool.title", "Tools"),
    ("tool.tolerance", "Tolerance"),
    ("tool.wand", "Magic Wand"),
    ("workspace.compositing", "Compositing"),
    ("workspace.illustration", "Illustration"),
    ("workspace.pixel_art", "Pixel Art"),
];

/// Japanese strings. **Keep sorted by key.**
static JA: &[(&str, &str)] = &[
    ("brush.blend", "合成モード"),
    ("brush.flow", "流量"),
    ("brush.hardness", "硬さ"),
    ("brush.opacity", "不透明度"),
    ("brush.presets", "プリセット"),
    ("brush.size", "サイズ"),
    ("brush.smoothing", "手ブレ補正"),
    ("brush.spacing", "間隔"),
    ("brush.title", "ブラシ"),
    ("canvas.title", "キャンバス"),
    ("color.hex", "16進"),
    ("color.palette", "パレット"),
    ("color.primary", "描画色"),
    ("color.secondary", "背景色"),
    ("color.swap", "入れ替え"),
    ("color.title", "カラー"),
    ("dialog.cancel", "キャンセル"),
    ("dialog.create", "作成"),
    ("dialog.height", "高さ"),
    ("dialog.name", "名前"),
    ("dialog.new_document", "新規ドキュメント"),
    ("dialog.unsaved_body", "保存されていない変更があります。"),
    ("dialog.unsaved_discard", "変更を破棄"),
    ("dialog.unsaved_title", "未保存の変更"),
    ("dialog.width", "幅"),
    ("history.title", "ヒストリー"),
    ("layer.add", "レイヤーを追加"),
    ("layer.add_group", "グループを追加"),
    ("layer.alpha_lock", "透明部分をロック"),
    ("layer.blend", "合成モード"),
    ("layer.clipping", "下のレイヤーでクリッピング"),
    ("layer.delete", "レイヤーを削除"),
    ("layer.duplicate", "レイヤーを複製"),
    ("layer.lock", "ロック"),
    ("layer.mask_add", "選択範囲からマスクを作成"),
    ("layer.mask_remove", "マスクを削除"),
    ("layer.merge_down", "下のレイヤーと結合"),
    ("layer.move_down", "下へ移動"),
    ("layer.move_up", "上へ移動"),
    ("layer.opacity", "不透明度"),
    ("layer.title", "レイヤー"),
    ("layer.visible", "表示"),
    ("menu.edit", "編集"),
    ("menu.edit.clear", "レイヤーを消去"),
    ("menu.edit.deselect", "選択を解除"),
    ("menu.edit.invert_selection", "選択範囲を反転"),
    ("menu.edit.redo", "やり直し"),
    ("menu.edit.select_all", "すべてを選択"),
    ("menu.edit.undo", "取り消し"),
    ("menu.file", "ファイル"),
    ("menu.file.export_png", "PNG形式で書き出し..."),
    ("menu.file.new", "新規..."),
    ("menu.file.open", "開く..."),
    ("menu.file.quit", "終了"),
    ("menu.file.save", "保存"),
    ("menu.file.save_as", "名前を付けて保存..."),
    ("menu.help", "ヘルプ"),
    ("menu.help.about", "Aether Canvas について"),
    ("menu.help.shortcuts", "ショートカット一覧"),
    ("menu.image", "イメージ"),
    ("menu.image.resize", "キャンバスサイズ..."),
    ("menu.language", "言語"),
    ("menu.layer", "レイヤー"),
    ("menu.view", "表示"),
    ("menu.view.fit", "ウィンドウに合わせる"),
    ("menu.view.mirror", "左右反転表示"),
    ("menu.view.pixel_grid", "ピクセルグリッド"),
    ("menu.view.reset", "100%表示"),
    ("menu.view.rotate_left", "左に回転"),
    ("menu.view.rotate_reset", "回転をリセット"),
    ("menu.view.rotate_right", "右に回転"),
    ("menu.view.zoom_in", "拡大"),
    ("menu.view.zoom_out", "縮小"),
    ("menu.window", "ウィンドウ"),
    ("menu.window.theme", "テーマ"),
    ("menu.workspace", "ワークスペース"),
    ("properties.title", "プロパティ"),
    ("status.ready", "準備完了"),
    ("theme.dark", "ダーク"),
    ("theme.high_contrast", "ハイコントラスト"),
    ("theme.light", "ライト"),
    ("tool.brush", "ブラシ"),
    ("tool.bucket", "塗りつぶし"),
    ("tool.ellipse_select", "楕円選択"),
    ("tool.eraser", "消しゴム"),
    ("tool.eyedropper", "スポイト"),
    ("tool.lasso", "投げなわ選択"),
    ("tool.move", "レイヤー移動"),
    ("tool.options", "ツールオプション"),
    ("tool.pan", "手のひら"),
    ("tool.rect_select", "長方形選択"),
    ("tool.title", "ツール"),
    ("tool.tolerance", "許容値"),
    ("tool.wand", "自動選択"),
    ("workspace.compositing", "コンポジット"),
    ("workspace.illustration", "イラスト"),
    ("workspace.pixel_art", "ドット絵"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn is_sorted(table: &[(&str, &str)]) -> bool {
        table.windows(2).all(|w| w[0].0 < w[1].0)
    }

    #[test]
    fn tables_are_sorted_for_binary_search() {
        assert!(is_sorted(EN), "the English table is out of order");
        assert!(is_sorted(JA), "the Japanese table is out of order");
    }

    #[test]
    fn every_english_key_has_a_japanese_translation() {
        let missing: Vec<&str> = EN
            .iter()
            .map(|(k, _)| *k)
            .filter(|k| lookup(JA, k).is_none())
            .collect();
        assert!(missing.is_empty(), "untranslated keys: {missing:?}");
    }

    #[test]
    fn japanese_has_no_stray_keys() {
        let extra: Vec<&str> = JA
            .iter()
            .map(|(k, _)| *k)
            .filter(|k| lookup(EN, k).is_none())
            .collect();
        assert!(extra.is_empty(), "keys with no English original: {extra:?}");
    }

    #[test]
    fn lookup_returns_the_right_language() {
        assert_eq!(Language::English.tr("menu.file"), "File");
        assert_eq!(Language::Japanese.tr("menu.file"), "ファイル");
    }

    #[test]
    fn unknown_keys_render_as_themselves() {
        assert_eq!(Language::English.tr("nope.not.here"), "nope.not.here");
    }
}
