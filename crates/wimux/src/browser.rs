//! ブラウザペイン。
//!
//! wry（Windows では WebView2 = Edge と同じ描画部品）でページを表示する。
//!
//! 重ね方:
//! GPUI（Windows）は自分の描画を子ウィンドウより「上」に重ねる（DirectComposition の topmost）。
//! そのため WebView2 を子ウィンドウにすると隠れて見えない。
//! そこで、メインウィンドウに「所有」された枠なしの小さなウィンドウ（`host::HostWindow`）を作り、
//! その中に WebView2 を入れて、区画の位置（画面座標）に重ねている。
//! 所有されたウィンドウは常にメインウィンドウの上に表示され、最小化も一緒になる。
//! ウィンドウを動かしたときは window.rs のタイマーで位置を追従させる（`reposition`）。
//!
//! 注意:
//! - ブラウザの上に GPUI の要素（ポップアップなど）は描けない。
//!   ポップアップを出すときは `set_visible(false)` で一時的に隠す（window.rs）
//! - ブラウザにフォーカスがある間は、wimux のキー操作（Ctrl+Shift+… など）は効かない

use std::cell::Cell;
use std::path::PathBuf;

use gpui_kit::{Bounds, Pixels};
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use wry::{PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

use crate::workspace::PaneId;

/// 親ウィンドウ（GPUI のウィンドウ）のハンドル。
/// GPUI の `Window` を借りずにブラウザを作れるよう、生のハンドルだけを持つ
#[derive(Clone, Copy)]
pub struct ParentWindow(pub RawWindowHandle);

impl HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: ウィンドウはアプリの終了まで生きている（ブラウザは常にそれより先に破棄される）
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// ブラウザから UI へのお知らせ
#[derive(Debug, Clone)]
pub enum BrowserMsg {
    Title(String),
    Url(String),
    Loading(bool),
}

pub type BrowserSender = flume::Sender<(PaneId, BrowserMsg)>;

pub struct Browser {
    // フィールドは上から順に破棄される。WebView を先に、入れ物のウィンドウを後に
    webview: WebView,
    host: host::HostWindow,
    /// 区画の位置（メインウィンドウ内の論理座標）
    last_bounds: Cell<Option<Bounds<Pixels>>>,
    scale: Cell<f32>,
    /// 前回置いた位置（画面上の物理座標）。変わったときだけ OS に伝える
    placed: Cell<Option<(i32, i32, i32, i32)>>,
    visible: Cell<bool>,
}

impl Browser {
    pub fn new(
        owner: ParentWindow,
        context: &mut WebContext,
        id: PaneId,
        url: &str,
        tx: BrowserSender,
    ) -> anyhow::Result<Self> {
        let host = host::HostWindow::new(owner.0)?;
        let tx_title = tx.clone();
        let tx_load = tx;
        let webview = WebViewBuilder::new_with_web_context(context)
            .with_url(url)
            .with_bounds(Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: PhysicalSize::new(1u32, 1u32).into(),
            })
            .with_focused(false)
            .with_devtools(true)
            .with_hotkeys_zoom(true)
            .with_document_title_changed_handler(move |title| {
                let _ = tx_title.send((id, BrowserMsg::Title(title)));
            })
            .with_on_page_load_handler(move |event, url| {
                let loading = matches!(event, PageLoadEvent::Started);
                let _ = tx_load.send((id, BrowserMsg::Loading(loading)));
                let _ = tx_load.send((id, BrowserMsg::Url(url)));
            })
            .build_as_child(&host)?;
        Ok(Self {
            webview,
            host,
            last_bounds: Cell::new(None),
            scale: Cell::new(1.0),
            placed: Cell::new(None),
            visible: Cell::new(false),
        })
    }

    /// 区画の位置（メインウィンドウ内の論理座標）を覚えて、そこに重ねる
    pub fn set_bounds(&self, b: Bounds<Pixels>, scale: f32) {
        self.last_bounds.set(Some(b));
        self.scale.set(scale);
        self.reposition();
    }

    /// 覚えている位置に重ね直す（メインウィンドウを動かしたとき用。変化がなければ何もしない）
    pub fn reposition(&self) {
        if !self.visible.get() {
            return;
        }
        let Some(b) = self.last_bounds.get() else {
            return;
        };
        let s = self.scale.get();
        let x = (f32::from(b.origin.x) * s).round() as i32;
        let y = (f32::from(b.origin.y) * s).round() as i32;
        let w = (f32::from(b.size.width) * s).round().max(1.) as i32;
        let h = (f32::from(b.size.height) * s).round().max(1.) as i32;
        let Some(screen) = self.host.place(x, y, w, h) else {
            return;
        };
        let size_changed = self
            .placed
            .get()
            .is_none_or(|(_, _, pw, ph)| pw != w || ph != h);
        self.placed.set(Some(screen));
        if size_changed {
            let _ = self.webview.set_bounds(Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: PhysicalSize::new(w as u32, h as u32).into(),
            });
        }
    }

    pub fn set_visible(&self, visible: bool) {
        if self.visible.get() == visible {
            return;
        }
        self.visible.set(visible);
        if visible {
            self.placed.set(None);
            self.reposition();
        }
        self.host.show(visible);
        let _ = self.webview.set_visible(visible);
    }

    /// 今表示しているページの URL（ページ内の移動も反映される）
    pub fn current_url(&self) -> Option<String> {
        self.webview.url().ok().filter(|u| !u.is_empty())
    }

    pub fn load(&self, url: &str) {
        let _ = self.webview.load_url(url);
    }

    pub fn back(&self) {
        let _ = self.webview.evaluate_script("history.back()");
    }

    pub fn forward(&self) {
        let _ = self.webview.evaluate_script("history.forward()");
    }

    pub fn reload(&self) {
        let _ = self.webview.reload();
    }
}

/// WebView2 を入れる、メインウィンドウに所有された枠なしウィンドウ
#[cfg(any(windows, wimux_check))]
mod host {
    use std::num::NonZeroIsize;
    use std::sync::Once;

    use anyhow::{Context as _, bail};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SW_HIDE,
        SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow, WNDCLASSEXW,
        WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_POPUP,
    };
    use wry::raw_window_handle::{
        HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
    };

    const CLASS_NAME: &str = "WimuxBrowserHost";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub struct HostWindow {
        hwnd: HWND,
        owner: HWND,
    }

    impl HostWindow {
        pub fn new(owner: RawWindowHandle) -> anyhow::Result<Self> {
            let RawWindowHandle::Win32(owner) = owner else {
                bail!("Windows のウィンドウではありません");
            };
            let owner = owner.hwnd.get() as HWND;
            let class = wide(CLASS_NAME);
            // SAFETY: Win32 API の呼び出し。引数はすべて有効な値
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                static REGISTER: Once = Once::new();
                REGISTER.call_once(|| {
                    let wc = WNDCLASSEXW {
                        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                        style: 0,
                        lpfnWndProc: Some(wndproc),
                        cbClsExtra: 0,
                        cbWndExtra: 0,
                        hInstance: instance,
                        hIcon: std::ptr::null_mut(),
                        hCursor: std::ptr::null_mut(),
                        hbrBackground: std::ptr::null_mut(),
                        lpszMenuName: std::ptr::null(),
                        lpszClassName: class.as_ptr(),
                        hIconSm: std::ptr::null_mut(),
                    };
                    RegisterClassExW(&wc);
                });
                // WS_POPUP + 親に owner を渡すと「owner に所有されたウィンドウ」になる
                // （常に owner の上に表示され、owner と一緒に最小化される）
                let hwnd = CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_POPUP | WS_CLIPCHILDREN,
                    0,
                    0,
                    1,
                    1,
                    owner,
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if hwnd.is_null() {
                    return Err(std::io::Error::last_os_error())
                        .context("ブラウザ用のウィンドウを作れませんでした");
                }
                Ok(Self { hwnd, owner })
            }
        }

        /// メインウィンドウのクライアント座標（物理ピクセル）で指定した位置に置く。
        /// 置いた画面座標を返す
        pub fn place(&self, x: i32, y: i32, w: i32, h: i32) -> Option<(i32, i32, i32, i32)> {
            let mut pt = POINT { x, y };
            // SAFETY: 有効なウィンドウハンドルと POINT へのポインタ
            unsafe {
                if ClientToScreen(self.owner, &mut pt) == 0 {
                    return None;
                }
                SetWindowPos(
                    self.hwnd,
                    std::ptr::null_mut(),
                    pt.x,
                    pt.y,
                    w,
                    h,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            Some((pt.x, pt.y, w, h))
        }

        pub fn show(&self, visible: bool) {
            // SAFETY: 有効なウィンドウハンドル
            unsafe {
                ShowWindow(self.hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
        }
    }

    impl HasWindowHandle for HostWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let hwnd = NonZeroIsize::new(self.hwnd as isize).ok_or(HandleError::Unavailable)?;
            let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
            // SAFETY: ウィンドウはこの構造体が生きている間は破棄されない
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    impl Drop for HostWindow {
        fn drop(&mut self) {
            // SAFETY: 自分で作ったウィンドウ
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }
}

/// Windows 以外（ブラウザペインは未対応）
#[cfg(not(any(windows, wimux_check)))]
mod host {
    use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};

    pub struct HostWindow;

    impl HostWindow {
        pub fn new(_owner: RawWindowHandle) -> anyhow::Result<Self> {
            anyhow::bail!("この OS ではブラウザペインは使えません")
        }
        pub fn place(&self, _x: i32, _y: i32, _w: i32, _h: i32) -> Option<(i32, i32, i32, i32)> {
            None
        }
        pub fn show(&self, _visible: bool) {}
    }

    impl HasWindowHandle for HostWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::NotSupported)
        }
    }
}

/// ブラウザのデータ（Cookie など）の保存先
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(crate::session::config_dir);
    base.join("wimux").join("WebView2")
}

/// 起動時の準備。`cdp_port` を指定すると、AI（Playwright / Chrome DevTools MCP など）から
/// そのポートでブラウザを操作できるようにする（Windows の WebView2 のみ）。
pub fn init(cdp_port: Option<u16>) {
    #[cfg(windows)]
    if let Some(port) = cdp_port {
        // SAFETY: 起動直後、ほかのスレッドを作る前に 1 度だけ呼ぶ
        unsafe {
            std::env::set_var(
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                format!("--remote-debugging-port={port}"),
            );
        }
    }
    #[cfg(not(windows))]
    let _ = cdp_port;
}

/// この OS でブラウザペインが使えるか。
/// Linux は GPUI のウィンドウが X11(xcb) / Wayland のハンドルで、wry が子ウィンドウとして扱えないため未対応
pub const SUPPORTED: bool = cfg!(windows);

/// 入力をそれらしい URL にする
/// - `3000` → `http://localhost:3000`
/// - `localhost:5173/x` → `http://localhost:5173/x`
/// - `example.com` → `https://example.com`
/// - それ以外の文字列 → Google 検索
pub fn normalize_url(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "about:blank".into();
    }
    if s.contains("://") || s.starts_with("about:") || s.starts_with("file:") {
        return s.to_string();
    }
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
        return format!("http://localhost:{s}");
    }
    let host = s.split(['/', '?']).next().unwrap_or(s);
    let is_local =
        host.starts_with("localhost") || host.starts_with("127.") || host.starts_with("0.0.0.0");
    if is_local {
        return format!("http://{s}");
    }
    if host.contains('.') && !s.contains(' ') {
        return format!("https://{s}");
    }
    let q: String = s
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://www.google.com/search?q={q}")
}

/// タブに出す短い名前（ページタイトルがなければホスト名）
pub fn short_label(title: Option<&str>, url: &str) -> String {
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        return t.trim().to_string();
    }
    if url.is_empty() {
        return "新しいタブ".into();
    }
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split('/').next().unwrap_or(rest).to_string()
}

#[cfg(test)]
mod tests {
    use super::{normalize_url, short_label};

    #[test]
    fn urls() {
        assert_eq!(normalize_url("3000"), "http://localhost:3000");
        assert_eq!(normalize_url("localhost:5173/a"), "http://localhost:5173/a");
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("https://x.dev/p"), "https://x.dev/p");
        assert_eq!(
            normalize_url("rust 非同期"),
            "https://www.google.com/search?q=rust+%E9%9D%9E%E5%90%8C%E6%9C%9F"
        );
    }

    #[test]
    fn labels() {
        assert_eq!(short_label(None, "https://github.com/a/b"), "github.com");
        assert_eq!(short_label(Some(" Docs "), "https://x"), "Docs");
    }
}
