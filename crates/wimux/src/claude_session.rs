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

/// `sondev.exe --claude-hook`: Claude Code の SessionStart フックとして呼ばれる
pub fn run_hook() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(key) = std::env::var("WIMUX_PANE_ID") else {
        return;
    };
    if !valid_id(&key) {
        return;
    }
    let Some(id) = serde_json::from_str::<serde_json::Value>(&input)
        .ok()
        .and_then(|v| v.get("session_id")?.as_str().map(str::to_string))
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
    let settings = serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": format!("\"{exe}\" --claude-hook"),
                }]
            }]
        }
    });
    let path = session::config_dir().join("claude-hook-settings.json");
    let bytes = serde_json::to_vec_pretty(&settings).ok()?;
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        session::write_atomic(&path, &bytes).ok()?;
    }
    Some(path)
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
    use super::valid_id;

    #[test]
    fn ids() {
        assert!(valid_id("3f2a9c1e-1234-4abc-9def-0123456789ab"));
        assert!(!valid_id(""));
        assert!(!valid_id("a; rm -rf /"));
        assert!(!valid_id("../x"));
    }
}
