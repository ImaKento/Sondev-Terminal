//! コマンドの一覧とキー割り当て。
//!
//! ここにある表 `COMMANDS` から、次の 3 つを作る:
//! - GPUI のキー割り当て（`key_bindings`）。settings.json の `keybindings` で変更・解除できる
//! - コマンドパレット（Ctrl+Shift+P）の一覧
//! - ブラウザ（WebView2）にフォーカスがあるときに、wimux に回すキーの一覧（`forwarded_action`）
//!
//! settings.json の例:
//! ```json
//! "keybindings": {
//!   "ctrl-w": "",                 // 既定の割り当てを外す（シェルの単語削除に戻す）
//!   "ctrl-alt-w": "close_tab"     // 新しく割り当てる
//! }
//! ```

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::RwLock;

use gpui_kit::*;

/// wimux のコマンド
#[derive(Debug)]
pub struct Command {
    /// settings.json で使う名前
    pub id: &'static str,
    /// GPUI のアクション名
    action: &'static str,
    /// コマンドパレットに出す名前
    pub label: &'static str,
    /// 既定のキー
    keys: &'static [&'static str],
    /// ターミナルの中だけで効く（検索など）
    terminal_only: bool,
}

const fn cmd(
    id: &'static str,
    action: &'static str,
    label: &'static str,
    keys: &'static [&'static str],
) -> Command {
    Command {
        id,
        action,
        label,
        keys,
        terminal_only: false,
    }
}

pub const COMMANDS: &[Command] = &[
    cmd(
        "command_palette",
        "wimux::CommandPalette",
        "コマンドパレット",
        &["ctrl-shift-p"],
    ),
    cmd(
        "new_workspace",
        "wimux::NewWorkspace",
        "新しいワークスペース",
        &["ctrl-shift-n"],
    ),
    cmd(
        "next_workspace",
        "wimux::NextWorkspace",
        "次のワークスペース",
        &["ctrl-shift-pagedown"],
    ),
    cmd(
        "prev_workspace",
        "wimux::PrevWorkspace",
        "前のワークスペース",
        &["ctrl-shift-pageup"],
    ),
    cmd(
        "rename_workspace",
        "wimux::RenameWorkspace",
        "ワークスペース名の変更",
        &["ctrl-shift-r"],
    ),
    cmd(
        "workspace_1",
        "wimux::SelectWorkspace1",
        "1 番目のワークスペース",
        &["ctrl-1"],
    ),
    cmd(
        "workspace_2",
        "wimux::SelectWorkspace2",
        "2 番目のワークスペース",
        &["ctrl-2"],
    ),
    cmd(
        "workspace_3",
        "wimux::SelectWorkspace3",
        "3 番目のワークスペース",
        &["ctrl-3"],
    ),
    cmd(
        "workspace_4",
        "wimux::SelectWorkspace4",
        "4 番目のワークスペース",
        &["ctrl-4"],
    ),
    cmd(
        "workspace_5",
        "wimux::SelectWorkspace5",
        "5 番目のワークスペース",
        &["ctrl-5"],
    ),
    cmd(
        "workspace_6",
        "wimux::SelectWorkspace6",
        "6 番目のワークスペース",
        &["ctrl-6"],
    ),
    cmd(
        "workspace_7",
        "wimux::SelectWorkspace7",
        "7 番目のワークスペース",
        &["ctrl-7"],
    ),
    cmd(
        "workspace_8",
        "wimux::SelectWorkspace8",
        "8 番目のワークスペース",
        &["ctrl-8"],
    ),
    cmd(
        "workspace_last",
        "wimux::SelectWorkspace9",
        "最後のワークスペース",
        &["ctrl-9"],
    ),
    cmd(
        "new_tab",
        "wimux::NewTab",
        "新しいタブ",
        &["ctrl-t", "ctrl-shift-t"],
    ),
    cmd(
        "close_tab",
        "wimux::ClosePane",
        "タブを閉じる",
        &["ctrl-w", "ctrl-shift-w"],
    ),
    cmd(
        "next_tab",
        "wimux::NextTab",
        "次のタブ",
        &["ctrl-]", "ctrl-tab"],
    ),
    cmd(
        "prev_tab",
        "wimux::PrevTab",
        "前のタブ",
        &["ctrl-[", "ctrl-shift-tab"],
    ),
    cmd("split_right", "wimux::SplitRight", "右に分割", &["ctrl-d"]),
    cmd(
        "split_down",
        "wimux::SplitDown",
        "下に分割",
        &["ctrl-shift-d", "ctrl-shift-e"],
    ),
    cmd(
        "split_browser",
        "wimux::SplitBrowser",
        "右に分割してブラウザを開く",
        &["ctrl-shift-l"],
    ),
    cmd(
        "open_browser",
        "wimux::OpenBrowser",
        "ブラウザのタブを開く",
        &["ctrl-shift-o"],
    ),
    cmd(
        "focus_left",
        "wimux::FocusLeft",
        "左の区画へ",
        &["ctrl-left", "alt-left"],
    ),
    cmd(
        "focus_right",
        "wimux::FocusRight",
        "右の区画へ",
        &["ctrl-right", "alt-right"],
    ),
    cmd(
        "focus_up",
        "wimux::FocusUp",
        "上の区画へ",
        &["ctrl-up", "alt-up"],
    ),
    cmd(
        "focus_down",
        "wimux::FocusDown",
        "下の区画へ",
        &["ctrl-down", "alt-down"],
    ),
    cmd(
        "resize_left",
        "wimux::ResizeLeft",
        "境界を左へ動かす",
        &["alt-shift-left"],
    ),
    cmd(
        "resize_right",
        "wimux::ResizeRight",
        "境界を右へ動かす",
        &["alt-shift-right"],
    ),
    cmd(
        "resize_up",
        "wimux::ResizeUp",
        "境界を上へ動かす",
        &["alt-shift-up"],
    ),
    cmd(
        "resize_down",
        "wimux::ResizeDown",
        "境界を下へ動かす",
        &["alt-shift-down"],
    ),
    cmd(
        "jump_attention",
        "wimux::JumpToAttention",
        "入力待ちのペインへ移動",
        &["ctrl-shift-u"],
    ),
    cmd(
        "folder_picker",
        "wimux::OpenDirPicker",
        "フォルダ履歴から移動（cdi）",
        &["ctrl-shift-g"],
    ),
    cmd(
        "zoom_in",
        "wimux::ZoomIn",
        "文字を大きく",
        &["ctrl-=", "ctrl-+", "ctrl-shift-="],
    ),
    cmd("zoom_out", "wimux::ZoomOut", "文字を小さく", &["ctrl--"]),
    cmd(
        "zoom_reset",
        "wimux::ZoomReset",
        "文字の大きさを元に戻す",
        &["ctrl-0"],
    ),
    cmd(
        "toggle_sidebar",
        "wimux::ToggleSidebar",
        "サイドバーの表示 / 非表示",
        &["ctrl-shift-b"],
    ),
    cmd("settings", "wimux::OpenSettings", "設定", &["ctrl-,"]),
    Command {
        id: "search",
        action: "terminal::SearchTerminal",
        label: "スクロールバックを検索",
        keys: &["ctrl-shift-f"],
        terminal_only: true,
    },
];

pub const CONTEXT: &str = "Wimux";
/// TerminalView の key_context と同じ名前
pub const TERMINAL_CONTEXT: &str = "Terminal";

/// 割り当ての結果（settings.json の変更を反映したもの）。コマンド id → キー
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub keys: Vec<(&'static Command, Vec<String>)>,
}

/// 既定のキー割り当てに settings.json の `keybindings` を重ねる。
/// 値が空文字列なら、そのキーの既定の割り当てを外す
pub fn resolve(overrides: &BTreeMap<String, String>) -> Resolved {
    let norm_overrides: Vec<(String, &str)> = overrides
        .iter()
        .map(|(k, v)| (normalize(k), v.trim()))
        .collect();
    let mut keys: Vec<(&'static Command, Vec<String>)> = COMMANDS
        .iter()
        .map(|c| {
            let ks = c
                .keys
                .iter()
                .map(|k| normalize(k))
                .filter(|k| !norm_overrides.iter().any(|(o, _)| o == k))
                .collect();
            (c, ks)
        })
        .collect();
    for (key, id) in norm_overrides {
        if id.is_empty() {
            continue;
        }
        match keys.iter_mut().find(|(c, _)| c.id == id) {
            Some((_, ks)) => ks.push(key),
            None => log::warn!("keybindings: 知らないコマンド {id:?}（{key}）"),
        }
    }
    Resolved { keys }
}

/// "ctrl-shift-t" / "Shift+Ctrl+T" → (["ctrl", "shift"], "t")。区切りは - か +。
/// 区切りの直後が区切り文字なら、それがキー（"ctrl--" → Ctrl+マイナス、"ctrl-+" → Ctrl+プラス）
fn split_key(k: &str) -> (Vec<String>, String) {
    let mut parts = Vec::new();
    let mut cur = String::new();
    for c in k.trim().to_ascii_lowercase().chars() {
        if (c == '-' || c == '+') && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    let key = if cur.is_empty() {
        parts.pop().unwrap_or_default()
    } else {
        cur
    };
    (parts, key)
}

/// キーの書き方をそろえる（"Shift+Ctrl+T" → "ctrl-shift-t"）。修飾キーは ctrl → alt → shift の順
pub fn normalize(k: &str) -> String {
    let (mods, key) = split_key(k);
    let has = |m: &str| mods.iter().any(|p| p == m);
    let mut out = String::new();
    if has("ctrl") || has("control") {
        out.push_str("ctrl-");
    }
    if has("alt") {
        out.push_str("alt-");
    }
    if has("shift") {
        out.push_str("shift-");
    }
    out.push_str(&key);
    out
}

/// GPUI に登録するキー割り当て
pub fn key_bindings(resolved: &Resolved, cx: &App) -> Vec<KeyBinding> {
    let wimux: Option<Rc<KeyBindingContextPredicate>> =
        KeyBindingContextPredicate::parse(CONTEXT).ok().map(Rc::new);
    let terminal: Option<Rc<KeyBindingContextPredicate>> =
        KeyBindingContextPredicate::parse(TERMINAL_CONTEXT)
            .ok()
            .map(Rc::new);
    let mut out = Vec::new();
    for (c, keys) in &resolved.keys {
        for k in keys {
            let Ok(action) = cx.build_action(c.action, None) else {
                log::warn!("アクション {} を作れませんでした", c.action);
                continue;
            };
            let ctx = if c.terminal_only {
                terminal.clone()
            } else {
                wimux.clone()
            };
            match KeyBinding::load(k, action, ctx, false, None, cx) {
                Ok(b) => out.push(b),
                Err(e) => log::warn!("keybindings: {k:?} を読めませんでした: {e}"),
            }
        }
    }
    out.extend(fixed_bindings());
    out
}

/// 変更できない割り当て（gpui-kit の Root が横取りするキーを、ターミナル・入力欄に戻す）
fn fixed_bindings() -> Vec<KeyBinding> {
    let t = Some(TERMINAL_CONTEXT);
    vec![
        // gpui-kit の Root は Tab / Shift+Tab（フォーカス移動）と Ctrl+C（コピー）を
        // 横取りするので、ターミナル内では無効にしてシェルにそのまま届ける
        KeyBinding::new("tab", NoAction, t),
        KeyBinding::new("shift-tab", NoAction, t),
        KeyBinding::new("ctrl-c", NoAction, t),
        // 入力欄（URL 欄など）でも Ctrl+C などは入力欄自身で処理する
        KeyBinding::new("ctrl-c", NoAction, Some("TextField")),
        KeyBinding::new("ctrl-x", NoAction, Some("TextField")),
        KeyBinding::new("ctrl-a", NoAction, Some("TextField")),
        KeyBinding::new("ctrl-v", NoAction, Some("TextField")),
        KeyBinding::new("tab", NoAction, Some("TextField")),
        // 入力欄では Ctrl+←→ は単語単位のカーソル移動
        KeyBinding::new("ctrl-left", NoAction, Some("TextField")),
        KeyBinding::new("ctrl-right", NoAction, Some("TextField")),
    ]
}

// ---------------------------------------------------------------------------
// ブラウザから回すキー
// ---------------------------------------------------------------------------

/// ブラウザにフォーカスがあるとき wimux に回すキー（キー → GPUI のアクション名）。
/// ブラウザの部品は別のスレッドではなく UI スレッドで呼ばれるが、wimux の状態には触れないので
/// グローバルに置いておく
static FORWARD: RwLock<Option<HashMap<String, &'static str>>> = RwLock::new(None);

/// ブラウザ（Ctrl+C / Ctrl+V など）では使わせたいキー。wimux に割り当てがあっても回さない
const BROWSER_KEYS: &[&str] = &[
    "ctrl-c",
    "ctrl-v",
    "ctrl-x",
    "ctrl-a",
    "ctrl-z",
    "ctrl-y",
    "ctrl-f",
    "ctrl-r",
    "ctrl-p",
    "ctrl-s",
    "ctrl-left",
    "ctrl-right",
    "alt-left",
    "alt-right",
];

pub fn set_forwarded(resolved: &Resolved) {
    let map: HashMap<String, &'static str> = resolved
        .keys
        .iter()
        .filter(|(c, _)| !c.terminal_only)
        .flat_map(|(c, keys)| keys.iter().map(move |k| (k.clone(), c.action)))
        .filter(|(k, _)| !BROWSER_KEYS.contains(&k.as_str()))
        .filter(|(k, _)| k.starts_with("ctrl-") || k.starts_with("alt-"))
        .collect();
    *FORWARD.write().unwrap() = Some(map);
}

/// ブラウザで押されたキー（ctrl-shift-t の形）を wimux に回すなら、そのアクション名
pub fn forwarded_action(key: &str) -> Option<&'static str> {
    FORWARD.read().unwrap().as_ref()?.get(key).copied()
}

/// コマンドの今のキー（表示用、"Ctrl+Shift+P" の形）
pub fn display_keys(keys: &[String]) -> String {
    keys.iter()
        .map(|k| {
            let (mods, key) = split_key(k);
            let mut s = String::new();
            for m in &mods {
                s.push_str(match m.as_str() {
                    "ctrl" => "Ctrl+",
                    "alt" => "Alt+",
                    "shift" => "Shift+",
                    other => other,
                });
            }
            let key = match key.as_str() {
                "pagedown" => "PageDown".to_string(),
                "pageup" => "PageUp".to_string(),
                "tab" => "Tab".to_string(),
                "left" => "←".to_string(),
                "right" => "→".to_string(),
                "up" => "↑".to_string(),
                "down" => "↓".to_string(),
                k => k.to_uppercase(),
            };
            s.push_str(&key);
            s
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// コマンド id → GPUI のアクション名
pub fn action_name(id: &str) -> Option<&'static str> {
    COMMANDS.iter().find(|c| c.id == id).map(|c| c.action)
}

#[cfg(test)]
mod tests {
    use super::{COMMANDS, display_keys, normalize, resolve};
    use std::collections::BTreeMap;

    #[test]
    fn normalize_keys() {
        assert_eq!(normalize("Shift+Ctrl+T"), "ctrl-shift-t");
        assert_eq!(normalize("ctrl-shift-pagedown"), "ctrl-shift-pagedown");
        assert_eq!(normalize("ctrl--"), "ctrl--");
        assert_eq!(normalize("alt-shift-left"), "alt-shift-left");
        assert_eq!(normalize("ctrl-]"), "ctrl-]");
        assert_eq!(normalize("ctrl-+"), "ctrl-+");
        assert_eq!(normalize("Ctrl++"), "ctrl-+");
    }

    #[test]
    fn overrides() {
        let mut o = BTreeMap::new();
        o.insert("ctrl-w".to_string(), String::new());
        o.insert("Ctrl+Alt+W".to_string(), "close_tab".to_string());
        let r = resolve(&o);
        let close = &r.keys.iter().find(|(c, _)| c.id == "close_tab").unwrap().1;
        assert_eq!(
            close,
            &vec!["ctrl-shift-w".to_string(), "ctrl-alt-w".to_string()]
        );
        // ほかのコマンドは既定のまま
        let new_tab = &r.keys.iter().find(|(c, _)| c.id == "new_tab").unwrap().1;
        assert_eq!(
            new_tab,
            &vec!["ctrl-t".to_string(), "ctrl-shift-t".to_string()]
        );
    }

    #[test]
    fn ids_unique_and_display() {
        for (i, a) in COMMANDS.iter().enumerate() {
            assert!(COMMANDS[i + 1..].iter().all(|b| b.id != a.id), "{}", a.id);
        }
        assert_eq!(
            display_keys(&["ctrl-shift-p".into(), "ctrl--".into()]),
            "Ctrl+Shift+P / Ctrl+-"
        );
    }
}
