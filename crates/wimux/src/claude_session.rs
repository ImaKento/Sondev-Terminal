//! ペインごとの Claude Code のセッション ID を覚える仕組み。
//!
//! `claude --continue` はフォルダ内で最後のセッションを開くので、同じフォルダの
//! 別々のタブで違うセッションを使っていると、再開時に同じセッションになってしまう。
//! そこで Claude Code のフック（SessionStart）でセッション ID を受け取り、ペインごとに
//! ファイルへ書いておく。次回の起動では `claude --resume <ID>` で再開する。
//!
//! 1. シェル連携スクリプトの `claude` 関数が `--settings <このフック設定>` を付けて起動
//! 2. セッションが始まる（起動・再開・/clear）たびに `sondev.exe --claude-hook` が呼ばれる
//! 3. 標準入力の JSON から session_id を取り出し、`claude-sessions\<WIMUX_PANE_ID>` に書く
//! 4. wimux は保存のたびにそのファイルを読み、session.json に記録する
//!
//! 同じフックで、作業開始・入力待ち・完了・終了も本体へ送る（ipc.rs / hooks.rs）。
//! タイトルやベルからの推定より確実に、サイドバーの丸と通知を出せる。

use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::session;

fn dir() -> PathBuf {
    session::config_dir().join("claude-sessions")
}

/// ペイン ID（WIMUX_PANE_ID）。同時に複数起動しても混ざらないようにプロセス ID を含める
pub fn pane_key(id: impl std::fmt::Display) -> String {
    format!("{}-{id}", std::process::id())
}

/// フックから書かれた、そのペインの今のセッション ID
pub fn read(key: &str) -> Option<String> {
    let id = std::fs::read_to_string(dir().join(key)).ok()?;
    let id = id.trim();
    valid_id(id).then(|| id.to_string())
}

/// セッション ID として安全な文字だけか（コマンドに埋め込むので確かめる）
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `sondev.exe --claude-hook`: Claude Code のフックとして呼ばれる。
/// 標準入力の JSON を読み、セッション ID を記録して、状態の変化を本体へ送る
pub fn run_hook() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&input) else {
        return;
    };
    let event = v.get("hook_event_name").and_then(|e| e.as_str());
    // 古いフック設定（SessionStart だけ）は hook_event_name を見ずに呼ばれていた
    if matches!(event, None | Some("SessionStart")) {
        record_session(&v);
    }
    if let Some((event, message)) = crate::hooks::claude_event(&v) {
        crate::ipc::send(crate::ipc::Message::Agent {
            agent: "claude".into(),
            event,
            message,
        });
    }
}

/// フックの入力からセッション ID を取り出し、ペインごとのファイルに書く
fn record_session(v: &serde_json::Value) {
    let Ok(key) = std::env::var("WIMUX_PANE_ID") else {
        return;
    };
    if !valid_id(&key) {
        return;
    }
    let Some(id) = v
        .get("session_id")
        .and_then(|s| s.as_str())
        .filter(|id| valid_id(id))
    else {
        return;
    };
    let _ = session::write_atomic(&dir().join(key), id.as_bytes());
}

/// フック設定（`claude --settings` に渡すファイル）を書き出して、その場所を返す
pub fn write_hook_settings() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // Windows の Claude Code は Git Bash でフックを動かすことがあるので、/ 区切りにしておく
    let exe = exe.to_string_lossy().replace('\\', "/");
    // セッション ID の記録（SessionStart）と、状態の通知（作業開始・入力待ち・完了・終了）。
    // PostToolUse は許可したツールが終わった合図（入力待ち → 作業中に戻す）。
    // PreToolUse は許可の確認より前に呼ばれ、入力待ちの判定に役立たないので使わない
    let hook = serde_json::json!([{
        "hooks": [{
            "type": "command",
            "command": format!("\"{exe}\" --claude-hook"),
            "timeout": 5,
        }]
    }]);
    let mut settings = serde_json::json!({
        "hooks": {
            "SessionStart": hook,
            "UserPromptSubmit": hook,
            "PostToolUse": hook,
            "Notification": hook,
            "Stop": hook,
            "SessionEnd": hook,
        }
    });
    // ステータスライン: Claude の利用上限（5 時間枠・週）を受け取ってサイドバーに出す。
    // --settings の statusLine はユーザーの設定より優先されるので、ユーザーが自分の
    // ステータスラインを使っているときは入れない（その人の表示を消さないため）
    if !user_has_statusline() {
        settings["statusLine"] = serde_json::json!({
            "type": "command",
            "command": format!("\"{exe}\" --claude-statusline"),
        });
    }
    let path = session::config_dir().join("claude-hook-settings.json");
    let bytes = serde_json::to_vec_pretty(&settings).ok()?;
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        session::write_atomic(&path, &bytes).ok()?;
    }
    Some(path)
}

/// ユーザーが Claude Code に自分のステータスラインを設定しているか（~/.claude/settings.json）
fn user_has_statusline() -> bool {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|h| PathBuf::from(h).join(".claude"))
        });
    let Some(dir) = dir else {
        return false;
    };
    std::fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v.get("statusLine").is_some_and(|s| !s.is_null()))
}

/// Sondev のブラウザを Claude Code から操作するための MCP の設定（`claude --mcp-config` に渡す）。
/// 設定の「AI からブラウザを操作」が ON のときだけ書き出し、OFF なら消す。
/// これがあれば `claude mcp add` を手でしなくても、Sondev のペインで起動した Claude は
/// どのプロジェクトでも Sondev のブラウザ（127.0.0.1:9222）を操作できる
pub fn write_mcp_config(browser_cdp: bool) {
    let path = mcp_config_path();
    if !(browser_cdp && crate::browser::SUPPORTED) {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let config = mcp_config(
        crate::settings::CDP_PORT,
        &session::config_dir().join("playwright-mcp"),
    );
    let Ok(bytes) = serde_json::to_vec_pretty(&config) else {
        return;
    };
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice())
        && let Err(e) = session::write_atomic(&path, &bytes)
    {
        log::warn!("ブラウザ操作の MCP 設定を書き出せませんでした: {e}");
    }
}

/// ペインに渡す MCP 設定の場所（書き出してあれば）
pub fn mcp_config_file() -> Option<PathBuf> {
    let path = mcp_config_path();
    path.is_file().then_some(path)
}

fn mcp_config_path() -> PathBuf {
    session::config_dir().join("claude-mcp.json")
}

/// Playwright MCP で、すでに開いている Sondev のブラウザ（CDP）につなぐ設定。
/// 作業記録（ページのスナップショット・コンソールのログ）はプロジェクトのフォルダではなく
/// wimux の保存先に書く（git の未追跡ファイルにならないように）
fn mcp_config(port: u16, output_dir: &std::path::Path) -> serde_json::Value {
    let mut args: Vec<String> = Vec::new();
    // Windows の npx はバッチファイルなので cmd /c 経由で起動する（Claude Code の推奨）
    let command = if cfg!(windows) {
        args.extend(["/c".into(), "npx".into()]);
        "cmd"
    } else {
        "npx"
    };
    args.extend([
        "-y".into(),
        "@playwright/mcp@latest".into(),
        "--cdp-endpoint".into(),
        format!("http://127.0.0.1:{port}"),
        "--output-dir".into(),
        output_dir.to_string_lossy().into_owned(),
    ]);
    serde_json::json!({
        "mcpServers": {
            "sondev-browser": { "command": command, "args": args }
        }
    })
}

/// 古い記録（1 週間より前）を消す
pub fn cleanup() {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return;
    };
    let limit = Duration::from_secs(7 * 24 * 3600);
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > limit);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mcp_config, valid_id};

    #[test]
    fn mcp_config_points_at_sondev_browser() {
        let v = mcp_config(9222, std::path::Path::new("/tmp/out"));
        let server = &v["mcpServers"]["sondev-browser"];
        let args: Vec<&str> = server["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        assert!(
            args.windows(2)
                .any(|w| w == ["--cdp-endpoint", "http://127.0.0.1:9222"])
        );
        assert!(args.windows(2).any(|w| w == ["--output-dir", "/tmp/out"]));
        assert!(args.contains(&"@playwright/mcp@latest"));
    }

    #[test]
    fn ids() {
        assert!(valid_id("3f2a9c1e-1234-4abc-9def-0123456789ab"));
        assert!(!valid_id(""));
        assert!(!valid_id("a; rm -rf /"));
        assert!(!valid_id("../x"));
    }
}
