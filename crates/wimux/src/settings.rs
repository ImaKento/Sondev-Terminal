//! 設定（`%APPDATA%\wimux\settings.json`）。設定画面から変更すると保存される。

use serde::{Deserialize, Serialize};

use crate::session;
use crate::theme::{DEFAULT_FONT_SIZE, DEFAULT_LINE_HEIGHT};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// テーマ名（theme.rs の THEMES）
    pub theme: String,
    /// ターミナルのフォント（None なら自動: JetBrains Mono → Cascadia Mono → …）
    pub font_family: Option<String>,
    pub font_size: f32,
    /// 行の高さ（文字サイズに対する倍率）
    pub line_height: f32,
    /// カーソルの形（アプリが形を指定したときはそちらが優先）
    pub cursor: CursorKind,
    /// 背景の不透明度（1.0 = 透過しない）
    pub opacity: f32,
    /// 透過したときの背景の効果
    pub backdrop: Backdrop,
    /// 起動時、前回 Claude Code を使っていたペインで `claude --continue` を実行する
    pub restore_claude: bool,
    /// AI（Playwright / Chrome DevTools MCP など）からブラウザを操作できるようにする。
    /// 有効にすると 127.0.0.1:9222 で操作用の口が開く（変更は再起動後に反映）
    pub browser_cdp: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorKind {
    Block,
    Beam,
    Underline,
}

/// 透過したときの背景の効果
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    /// そのまま透ける
    Transparent,
    /// 後ろをぼかす（すりガラス）
    Blur,
    /// Windows 11 の Mica
    Mica,
    /// Windows 11 の Mica Alt
    MicaAlt,
}

/// AI からブラウザを操作するときのポート
pub const CDP_PORT: u16 = 9222;

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "wimux".into(),
            font_family: None,
            font_size: DEFAULT_FONT_SIZE,
            line_height: DEFAULT_LINE_HEIGHT,
            cursor: CursorKind::Block,
            opacity: 1.0,
            backdrop: Backdrop::Blur,
            restore_claude: true,
            browser_cdp: false,
        }
    }
}

fn path() -> std::path::PathBuf {
    session::config_dir().join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) {
    let result = serde_json::to_vec_pretty(s)
        .map_err(std::io::Error::other)
        .and_then(|bytes| session::write_atomic(&path(), &bytes));
    if let Err(e) = result {
        log::warn!("設定を保存できませんでした: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn missing_fields_use_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme":"Nord"}"#).unwrap();
        assert_eq!(s.theme, "Nord");
        assert!(s.restore_claude);
        assert_eq!(s.font_size, crate::theme::DEFAULT_FONT_SIZE);
        assert_eq!(s.opacity, 1.0);
    }

    #[test]
    fn appearance_roundtrip() {
        let s: Settings = serde_json::from_str(
            r#"{"opacity":0.8,"backdrop":"mica_alt","cursor":"beam","line_height":1.4}"#,
        )
        .unwrap();
        assert_eq!(s.backdrop, super::Backdrop::MicaAlt);
        assert_eq!(s.cursor, super::CursorKind::Beam);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
