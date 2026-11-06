//! wimux — Windows 向けの軽量ワークスペース型ターミナル

// リリースビルドではコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 表示名（ウィンドウのタイトル・インストーラーなど）。開発中の名前は wimux
pub const APP_NAME: &str = "Sondev Terminal";

mod browser;
mod claude_session;
mod dirs;
mod file_tree;
mod hooks;
mod ipc;
mod keymap;
mod notify;
mod open;
mod picker;
mod session;
mod settings;
mod terminal;
mod text_field;
mod theme;
mod update;
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
        // Claude Code のステータスライン（利用上限を本体へ送る）
        Some("--claude-statusline") => return hooks::run_statusline(),
        // Codex CLI の notify（最後の引数が JSON）
        Some("--codex-notify") => return hooks::run_codex_notify(&args[1..]),
        // `sondev notify [--title <題>] <本文>`
        Some("notify") => return hooks::run_notify(&args[1..]),
        _ => {}
    }
    env_logger::init();
    // 保存先を以前の名前（wimux）から SondevTerminal へ移す（初回だけ）
    session::migrate_dirs();
    // フック・通知の受け口（ペインの環境変数 WIMUX_IPC で場所を渡す）
    ipc::start();
    claude_session::cleanup();
    let s = settings::load();
    browser::init(s.browser_cdp.then_some(settings::CDP_PORT));
    // Claude Code から Sondev のブラウザを操作する MCP の設定（ON のときだけ）
    claude_session::write_mcp_config(s.browser_cdp);

    let keybindings = s.keybindings.clone();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        // キー割り当て（settings.json の keybindings で変更できる）
        let keys = keymap::resolve(&keybindings);
        keymap::set_forwarded(&keys);
        let bindings = keymap::key_bindings(&keys, cx);
        cx.bind_keys(bindings);

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
        let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| window::WimuxWindow::new(window, cx))
        })
        .expect("ウィンドウを開けませんでした");
        // gpui-kit の Root は、いちばん下に gpui-component のテーマの背景（既定のライトテーマでは白）を塗る。
        // 透過したときに後ろが白く濁るので、Root の背景は透明にする（色は wimux 側で塗る）
        let _ = handle.update(cx, |_, window, cx| {
            gpui_kit::base::Root::update(window, cx, |root, _, cx| {
                root.style().background = Some(gpui_kit::transparent_black().into());
                cx.notify();
            });
        });

        // 最後のウィンドウを閉じたら終了
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
