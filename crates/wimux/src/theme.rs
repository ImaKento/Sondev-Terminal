//! 配色と文字サイズ。
//!
//! - テーマはターミナルの配色（16 色＋前景・背景）だけを定義し、
//!   サイドバーやタブなど UI の色はそこから自動で作る（`derive`）
//! - 今のテーマは `th()` で取得する（設定画面で切り替えるとすぐ反映される）

use std::sync::RwLock;

use gpui_kit::{Hsla, Rgba};

use crate::terminal::colors::{self, DEFAULT_PALETTE, Palette, rgb};

/// ターミナルの既定の文字サイズ（Ghostty / cmux の既定に合わせて 13）
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
/// ターミナルの行の高さ（文字サイズに対する倍率）の既定値
pub const DEFAULT_LINE_HEIGHT: f32 = 1.25;

// UI の文字サイズ
pub const UI_TEXT: f32 = 13.0;
pub const UI_TAB: f32 = 12.5;
pub const UI_SMALL: f32 = 11.5;

// 寸法
pub const SIDEBAR_WIDTH: f32 = 210.0;
/// 右側のファイル一覧の幅と 1 行の高さ
pub const FILE_TREE_WIDTH: f32 = 250.0;
pub const FILE_TREE_ROW: f32 = 22.0;
/// 区画ごとのタブの行（上端の区画ではタイトルバーを兼ねる）
pub const TAB_BAR_HEIGHT: f32 = 36.0;
pub const TAB_HEIGHT: f32 = 29.0;

/// 閉じるボタンのホバー色（Windows 標準）
pub const CLOSE_RED: u32 = 0xc42b1c;
/// 削除など危険な操作の文字色
pub const DANGER: u32 = 0xf47174;

/// 最小化・最大化・閉じるボタンのアイコン用フォント（Windows 10 / 11 に標準で入っている）
pub const CAPTION_FONT: &str = "Segoe MDL2 Assets";

// ---------------------------------------------------------------------------
// テーマ定義
// ---------------------------------------------------------------------------

pub struct ThemeDef {
    pub name: &'static str,
    pub palette: Palette,
    /// アクセント色（選択中のワークスペースなど）。None なら青（ansi[4]）
    pub accent: Option<u32>,
}

macro_rules! palette {
    ($fg:expr, $bg:expr, $cursor:expr, [$($c:expr),* $(,)?]) => {
        Palette {
            foreground: rgb($fg),
            background: rgb($bg),
            cursor: rgb($cursor),
            ansi: [$(rgb($c)),*],
        }
    };
}

pub const THEMES: &[ThemeDef] = &[
    ThemeDef {
        name: "wimux",
        palette: DEFAULT_PALETTE,
        accent: Some(0x4f8cff),
    },
    ThemeDef {
        name: "Tokyo Night",
        palette: palette!(
            0xc0caf5,
            0x1a1b26,
            0xc0caf5,
            [
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "Catppuccin Mocha",
        palette: palette!(
            0xcdd6f4,
            0x1e1e2e,
            0xf5e0dc,
            [
                0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
                0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "Dracula",
        palette: palette!(
            0xf8f8f2,
            0x282a36,
            0xf8f8f2,
            [
                0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "Nord",
        palette: palette!(
            0xd8dee9,
            0x2e3440,
            0xd8dee9,
            [
                0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
            ]
        ),
        accent: Some(0x88c0d0),
    },
    ThemeDef {
        name: "Gruvbox Dark",
        palette: palette!(
            0xebdbb2,
            0x282828,
            0xebdbb2,
            [
                0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
                0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
            ]
        ),
        accent: Some(0x83a598),
    },
    ThemeDef {
        name: "One Light",
        palette: palette!(
            0x383a42,
            0xfafafa,
            0x526fff,
            [
                0x383a42, 0xe45649, 0x50a14f, 0xc18401, 0x4078f2, 0xa626a4, 0x0184bc, 0xa0a1a7,
                0x696c77, 0xe45649, 0x50a14f, 0xc18401, 0x4078f2, 0xa626a4, 0x0184bc, 0xffffff,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "Tokyo Night Storm",
        palette: palette!(
            0xc0caf5,
            0x24283b,
            0xc0caf5,
            [
                0x1d202f, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "One Dark",
        palette: palette!(
            0xabb2bf,
            0x282c34,
            0x528bff,
            [
                0x3f4451, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                0x5c6370, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "GitHub Dark",
        palette: palette!(
            0xc9d1d9,
            0x0d1117,
            0x58a6ff,
            [
                0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4,
                0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff,
            ]
        ),
        accent: Some(0x2f81f7),
    },
    ThemeDef {
        name: "Monokai",
        palette: palette!(
            0xf8f8f2,
            0x272822,
            0xf8f8f0,
            [
                0x3e3d32, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2,
                0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf9f8f5,
            ]
        ),
        accent: Some(0xf92672),
    },
    ThemeDef {
        name: "Solarized Dark",
        palette: palette!(
            0x839496,
            0x002b36,
            0x93a1a1,
            [
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x586e75, 0xcb4b16, 0x859900, 0xb58900, 0x268bd2, 0x6c71c4, 0x2aa198, 0xfdf6e3,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "Rose Pine",
        palette: palette!(
            0xe0def4,
            0x191724,
            0xe0def4,
            [
                0x26233a, 0xeb6f92, 0x31748f, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xebbcba, 0xe0def4,
                0x6e6a86, 0xeb6f92, 0x31748f, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xebbcba, 0xe0def4,
            ]
        ),
        accent: Some(0xc4a7e7),
    },
    ThemeDef {
        name: "Kanagawa",
        palette: palette!(
            0xdcd7ba,
            0x1f1f28,
            0xc8c093,
            [
                0x16161d, 0xc34043, 0x76946a, 0xc0a36e, 0x7e9cd8, 0x957fb8, 0x6a9589, 0xc8c093,
                0x727169, 0xe82424, 0x98bb6c, 0xe6c384, 0x7fb4ca, 0x938aa9, 0x7aa89f, 0xdcd7ba,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "Everforest",
        palette: palette!(
            0xd3c6aa,
            0x2d353b,
            0xd3c6aa,
            [
                0x475258, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
                0x859289, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
            ]
        ),
        accent: Some(0xa7c080),
    },
    ThemeDef {
        name: "Ayu Dark",
        palette: palette!(
            0xbfbdb6,
            0x0b0e14,
            0xe6b450,
            [
                0x1e232b, 0xea6c73, 0x7fd962, 0xf9af4f, 0x53bdfa, 0xcda1fa, 0x90e1c6, 0xc7c7c7,
                0x686868, 0xf07178, 0xaad94c, 0xffb454, 0x59c2ff, 0xd2a6ff, 0x95e6cb, 0xffffff,
            ]
        ),
        accent: Some(0xe6b450),
    },
    ThemeDef {
        name: "Catppuccin Latte",
        palette: palette!(
            0x4c4f69,
            0xeff1f5,
            0xdc8a78,
            [
                0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
                0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
            ]
        ),
        accent: Some(0x8839ef),
    },
    ThemeDef {
        name: "GitHub Light",
        palette: palette!(
            0x24292f,
            0xffffff,
            0x0969da,
            [
                0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
                0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "Solarized Light",
        palette: palette!(
            0x657b83,
            0xfdf6e3,
            0x586e75,
            [
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0x93a1a1,
                0x586e75, 0xcb4b16, 0x859900, 0xb58900, 0x268bd2, 0x6c71c4, 0x2aa198, 0x002b36,
            ]
        ),
        accent: None,
    },
    ThemeDef {
        name: "Gruvbox Light",
        palette: palette!(
            0x3c3836,
            0xfbf1c7,
            0x3c3836,
            [
                0x3c3836, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0x7c6f64,
                0x928374, 0x9d0006, 0x79740e, 0xb57614, 0x076678, 0x8f3f71, 0x427b58, 0x282828,
            ]
        ),
        accent: Some(0x076678),
    },
];

pub fn find(name: &str) -> &'static ThemeDef {
    THEMES
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .unwrap_or(&THEMES[0])
}

// ---------------------------------------------------------------------------
// UI の色
// ---------------------------------------------------------------------------

/// UI で使う色一式（テーマから自動で作る）
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    /// 明るいテーマか（今はテストでのみ使用。明るいテーマ向けの細かな調整用）
    #[allow(dead_code)]
    pub is_light: bool,
    /// ウィンドウの枠（サイドバー・タブの行）
    pub chrome_bg: u32,
    /// ターミナルの背景
    pub term_bg: u32,
    pub hover_bg: u32,
    pub hover_bg_strong: u32,
    /// サイドバーで選択中のワークスペース
    pub selected_bg: u32,
    /// 選択中のワークスペースの補足文字
    pub selected_meta: u32,
    pub accent: u32,
    pub divider: u32,
    pub panel_border: u32,
    pub menu_bg: u32,
    /// ターミナルの文字選択
    pub selection: u32,
    pub text: u32,
    pub text_dim: u32,
    pub text_faint: u32,
    pub green: u32,
    pub green_dim: u32,
    pub amber: u32,
    pub red: u32,
}

pub fn hex(c: alacritty_terminal::vte::ansi::Rgb) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

/// a と b を t（0〜1）で混ぜる
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let x = ((a >> shift) & 0xff) as f32;
        let y = ((b >> shift) & 0xff) as f32;
        ((x + (y - x) * t).round() as u32) & 0xff
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

fn luminance(c: u32) -> f32 {
    let r = ((c >> 16) & 0xff) as f32;
    let g = ((c >> 8) & 0xff) as f32;
    let b = (c & 0xff) as f32;
    (0.299 * r + 0.587 * g + 0.114 * b) / 255.0
}

pub fn derive(def: &ThemeDef) -> Colors {
    let p = &def.palette;
    let bg = hex(p.background);
    let fg = hex(p.foreground);
    let is_light = luminance(bg) > 0.5;
    let accent = def.accent.unwrap_or_else(|| hex(p.ansi[4]));
    let chrome = if is_light {
        mix(bg, 0x000000, 0.05)
    } else {
        mix(bg, 0x000000, 0.4)
    };
    Colors {
        is_light,
        chrome_bg: chrome,
        term_bg: bg,
        hover_bg: mix(chrome, fg, 0.07),
        hover_bg_strong: mix(chrome, fg, 0.14),
        selected_bg: if is_light {
            accent
        } else {
            mix(accent, 0x000000, 0.28)
        },
        selected_meta: mix(0xffffff, accent, 0.2),
        accent,
        divider: mix(bg, fg, 0.10),
        panel_border: mix(chrome, fg, 0.15),
        menu_bg: mix(chrome, fg, 0.05),
        selection: mix(bg, accent, 0.35),
        text: fg,
        text_dim: mix(fg, bg, 0.42),
        text_faint: mix(fg, bg, 0.62),
        green: hex(p.ansi[2]),
        green_dim: mix(hex(p.ansi[2]), bg, 0.2),
        amber: hex(p.ansi[3]),
        red: hex(p.ansi[1]),
    }
}

static CURRENT: RwLock<Option<Colors>> = RwLock::new(None);

// ---------------------------------------------------------------------------
// 見た目の設定（透明度・行の高さ）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Look {
    opacity: f32,
    line_height: f32,
}

static LOOK: RwLock<Look> = RwLock::new(Look {
    opacity: 1.0,
    line_height: DEFAULT_LINE_HEIGHT,
});

/// 背景の不透明度（1.0 = 透過しない）
pub fn opacity() -> f32 {
    LOOK.read().unwrap().opacity
}

/// 背景を透過しているか
pub fn translucent() -> bool {
    opacity() < 0.999
}

pub fn set_opacity(v: f32) {
    LOOK.write().unwrap().opacity = v.clamp(0.2, 1.0);
}

/// ターミナルの行の高さ（文字サイズに対する倍率）
pub fn line_height() -> f32 {
    LOOK.read().unwrap().line_height
}

pub fn set_line_height(v: f32) {
    LOOK.write().unwrap().line_height = v.clamp(1.0, 2.0);
}

/// 背景用の色（透明度の設定を反映）。サイドバー・タブの行・ターミナルの背景に使う
pub fn bg(c: u32) -> Hsla {
    Rgba {
        r: ((c >> 16) & 0xff) as f32 / 255.0,
        g: ((c >> 8) & 0xff) as f32 / 255.0,
        b: (c & 0xff) as f32 / 255.0,
        a: opacity(),
    }
    .into()
}

/// 今のテーマの UI 色
pub fn th() -> Colors {
    if let Some(c) = *CURRENT.read().unwrap() {
        return c;
    }
    derive(&THEMES[0])
}

/// テーマを切り替える（UI 色とターミナルの配色の両方）
pub fn apply(name: &str) {
    let def = find(name);
    *CURRENT.write().unwrap() = Some(derive(def));
    colors::set_current(def.palette);
}

#[cfg(test)]
mod tests {
    use super::{THEMES, derive, find, mix};

    #[test]
    fn themes_derive() {
        for t in THEMES {
            let c = derive(t);
            assert_ne!(c.text, c.term_bg, "{}", t.name);
        }
        assert!(derive(find("One Light")).is_light);
        assert!(!derive(find("tokyo night")).is_light);
        assert_eq!(find("存在しない").name, "wimux");
        assert!(derive(find("GitHub Light")).is_light);
        assert!(derive(find("Solarized Light")).is_light);
        // 名前の重複がないこと
        for (i, a) in THEMES.iter().enumerate() {
            for b in &THEMES[i + 1..] {
                assert!(!a.name.eq_ignore_ascii_case(b.name));
            }
        }
    }

    #[test]
    fn mix_endpoints() {
        assert_eq!(mix(0x000000, 0xffffff, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xffffff, 1.0), 0xffffff);
    }
}
