//! ウィンドウの位置・大きさ・最大化（`%APPDATA%\wimux\window.json`）。
//! 閉じたときの状態を覚えて、次の起動で同じように開く。初回は最大化で開く。

use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::session;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub maximized: bool,
    /// 最大化を戻したときの位置と大きさ（論理座標）
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

fn path() -> std::path::PathBuf {
    session::config_dir().join("window.json")
}

impl SavedWindow {
    pub fn from_bounds(b: WindowBounds) -> Self {
        let (maximized, r) = match b {
            WindowBounds::Windowed(r) => (false, r),
            WindowBounds::Maximized(r) | WindowBounds::Fullscreen(r) => (true, r),
        };
        Self {
            maximized,
            x: f32::from(r.origin.x),
            y: f32::from(r.origin.y),
            w: f32::from(r.size.width),
            h: f32::from(r.size.height),
        }
    }

    fn rect(&self) -> Bounds<Pixels> {
        Bounds::new(point(px(self.x), px(self.y)), size(px(self.w), px(self.h)))
    }
}

pub fn load() -> Option<SavedWindow> {
    let text = std::fs::read_to_string(path()).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(w: &SavedWindow) {
    let result = serde_json::to_vec_pretty(w)
        .map_err(std::io::Error::other)
        .and_then(|bytes| session::write_atomic(&path(), &bytes));
    if let Err(e) = result {
        log::warn!("ウィンドウの位置を保存できませんでした: {e}");
    }
}

/// 起動時のウィンドウの状態。
/// 前回の位置が今のどの画面にも入らない（モニターを外したなど）ときや初回は、最大化で開く
pub fn initial_bounds(cx: &App) -> WindowBounds {
    let fallback = Bounds::centered(None, size(px(1200.), px(760.)), cx);
    let Some(saved) = load() else {
        return WindowBounds::Maximized(fallback);
    };
    let rect = saved.rect();
    let on_screen = saved.w >= 200.
        && saved.h >= 150.
        && cx.displays().iter().any(|d| {
            // タイトルの行（上端の一部）が見える画面があれば OK
            let top = Bounds::new(rect.origin, size(rect.size.width, px(40.)));
            d.bounds().intersects(&top)
        });
    match (saved.maximized, on_screen) {
        (true, true) => WindowBounds::Maximized(rect),
        (false, true) => WindowBounds::Windowed(rect),
        _ => WindowBounds::Maximized(fallback),
    }
}
