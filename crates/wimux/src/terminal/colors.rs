//! 端末の色（vte の Color）を GPUI の Hsla に変換する。

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use std::sync::RwLock;

use gpui_kit::{Hsla, Rgba};

/// 今使っている配色（PTY の読み取りスレッドからも参照するのでグローバルに置く）
static CURRENT: RwLock<Palette> = RwLock::new(DEFAULT_PALETTE);

pub fn current() -> Palette {
    *CURRENT.read().unwrap()
}

pub fn set_current(p: Palette) {
    *CURRENT.write().unwrap() = p;
}

/// ターミナルの配色（テーマごとに theme.rs で定義）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub ansi: [Rgb; 16],
}

pub const fn rgb(hex: u32) -> Rgb {
    Rgb {
        r: (hex >> 16) as u8,
        g: (hex >> 8) as u8,
        b: hex as u8,
    }
}

pub const DEFAULT_PALETTE: Palette = Palette {
    foreground: rgb(0xe4e4e7),
    background: rgb(0x161619), // theme::TERM_BG と揃える
    cursor: rgb(0xd4d4d8),
    ansi: [
        rgb(0x26262b), // black
        rgb(0xf47174), // red
        rgb(0x5fd38d), // green
        rgb(0xf2c94c), // yellow
        rgb(0x5b9cf8), // blue
        rgb(0xc58af9), // magenta
        rgb(0x3fc7d6), // cyan
        rgb(0xd4d4d8), // white
        rgb(0x5d5d67), // bright black
        rgb(0xff9797), // bright red
        rgb(0x8be8ae), // bright green
        rgb(0xffe08a), // bright yellow
        rgb(0x8dbbff), // bright blue
        rgb(0xdcb3ff), // bright magenta
        rgb(0x7fe3ee), // bright cyan
        rgb(0xfafafa), // bright white
    ],
};

pub fn to_hsla(c: Rgb) -> Hsla {
    Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

/// 256 色パレットのインデックス → RGB（16〜255 は xterm 標準の計算式）。
fn indexed(i: u8, palette: &Palette) -> Rgb {
    match i {
        0..=15 => palette.ansi[i as usize],
        16..=231 => {
            let i = i - 16;
            let step = |v: u8| if v == 0 { 0 } else { v * 40 + 55 };
            Rgb {
                r: step(i / 36),
                g: step((i / 6) % 6),
                b: step(i % 6),
            }
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            Rgb { r: v, g: v, b: v }
        }
    }
}

fn dim(c: Rgb) -> Rgb {
    Rgb {
        r: (c.r as u16 * 2 / 3) as u8,
        g: (c.g as u16 * 2 / 3) as u8,
        b: (c.b as u16 * 2 / 3) as u8,
    }
}

/// Color を RGB に解決する。OSC 4 / 10 / 11 でアプリが上書きした色（`overrides`）を優先する。
pub fn resolve(color: Color, overrides: &Colors, palette: &Palette) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(i) => overrides[i as usize].unwrap_or_else(|| indexed(i, palette)),
        Color::Named(named) => {
            if let Some(c) = overrides[named] {
                return c;
            }
            let idx = named as usize;
            match named {
                NamedColor::Foreground | NamedColor::BrightForeground => palette.foreground,
                NamedColor::Background => palette.background,
                NamedColor::Cursor => palette.cursor,
                NamedColor::DimForeground => dim(palette.foreground),
                _ if idx < 16 => palette.ansi[idx],
                // DimBlack.. DimWhite
                _ => {
                    let base = idx.saturating_sub(NamedColor::DimBlack as usize).min(7);
                    dim(palette.ansi[base])
                }
            }
        }
    }
}

/// 色の問い合わせ（OSC 4 / 10 / 11 / 12）に答えるための既定色。
/// `index`: 0〜255 はパレット番号、256 以降は NamedColor（前景・背景・カーソル…）。
pub fn default_rgb(index: usize) -> Rgb {
    let palette = &current();
    if index < 256 {
        return indexed(index as u8, palette);
    }
    match index {
        i if i == NamedColor::Foreground as usize => palette.foreground,
        i if i == NamedColor::Background as usize => palette.background,
        i if i == NamedColor::Cursor as usize => palette.cursor,
        _ => palette.foreground,
    }
}
