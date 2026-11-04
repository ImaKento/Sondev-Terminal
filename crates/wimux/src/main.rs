//! wimux — Windows 向けの軽量ワークスペース型ターミナル

// リリースビルドではコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 表示名（ウィンドウのタイトル・インストーラーなど）。開発中の名前は wimux
pub const APP_NAME: &str = "Sondev Terminal";

mod browser;
mod claude_session;
mod dirs;
mod hooks;
mod ipc;
mod notify;
mod open;
mod picker;
mod session;
mod settings;
mod terminal;
mod text_field;
mod theme;
mod window;
mod window_state;
mod workspace;

use gpui_kit::*;

fn main() {
    // ペインの中から呼ばれたとき（ウィンドウは開かない）
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // Claude Code のフック
        Some("--claude-hook") => return claude_session::run_hook(),
        // Codex CLI の notify（最後の引数が JSON）
        Some("--codex-notify") => return hooks::run_codex_notify(&args[1..]),
        // `sondev notify [--title <題>] <本文>`
        Some("notify") => return hooks::run_notify(&args[1..]),
        _ => {}
    }
    env_logger::init();
    // フック・通知の受け口（ペインの環境変数 WIMUX_IPC で場所を渡す）
    ipc::start();
    claude_session::cleanup();
    let s = settings::load();
    browser::init(s.browser_cdp.then_some(settings::CDP_PORT));

    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys(window::key_bindings());

        let options = WindowOptions {
            // 前回閉じたときの位置・大きさ・最大化を復元（初回は最大化）
            window_bounds: Some(window_state::initial_bounds(cx)),
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                // Windows では OS のタイトルバーを消し、タブの行と一体にする
                appears_transparent: window::custom_titlebar(),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| window::WimuxWindow::new(window, cx))
        })
        .expect("ウィンドウを開けませんでした");

        // 最後のウィンドウを閉じたら終了
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
