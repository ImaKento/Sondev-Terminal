//! セッション（ワークスペース・タブ・分割レイアウト・ペインごとの状態）の保存と読み込み。
//!
//! 保存先: `%APPDATA%\wimux\session.json`（`WIMUX_CONFIG_DIR` で変更可）

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 1: ワークスペース → タブ → 分割
/// 2: ワークスペース → 分割 → タブ（区画ごとにタブを持つ）
pub const SESSION_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    /// 左右に並べる
    Horizontal,
    /// 上下に並べる
    Vertical,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFile {
    pub version: u32,
    pub active_workspace: usize,
    pub workspaces: Vec<WorkspaceState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// ユーザーが付けた名前（なければフォルダ名を自動表示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 分割レイアウト（v2）。葉は `NodeState::Group`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<NodeState>,
    /// フォーカスしていた区画（レイアウトを左上から順に数えた番号）
    #[serde(default)]
    pub focused: usize,
    /// v1 形式（読み込みのみ）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<TabState>,
    #[serde(default)]
    pub active_tab: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    pub layout: NodeState,
    /// フォーカスしていたペイン（レイアウトを左上から順に数えた番号）
    pub focused: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum NodeState {
    /// 区画（タブの並び）
    Group { active: usize, tabs: Vec<PaneState> },
    /// v1 形式の葉（区画にタブ 1 つとして読み込む）
    Pane(PaneState),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<NodeState>,
        second: Box<NodeState>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaneState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conda: Option<String>,
    /// 終了時に Claude Code が起動していたか
    #[serde(default)]
    pub claude: bool,
    /// そのペインの Claude Code のセッション ID（`claude --resume <ID>` で再開する）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session: Option<String>,
    /// 終了時に動いていた Claude 以外のエージェント（"codex" など。シェルのコマンド名）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// そのエージェントのセッション ID（Codex の thread-id。`codex resume <ID>` で再開する）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<String>,
    /// 終了時に入っていた Docker のコンテナ（`dlist --enter <名前>` で入り直す）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker: Option<String>,
    /// ファイルの表示のタブならそのファイル
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// ブラウザのタブなら URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// 保存先のフォルダ名（`%APPDATA%\SondevTerminal`・`%LOCALAPPDATA%\SondevTerminal`）。
/// 空白を入れないのは、フックのコマンドなどに埋め込むときに引用で壊れないようにするため
pub const DIR_NAME: &str = "SondevTerminal";
/// 以前の保存先のフォルダ名（開発名）。起動時に `DIR_NAME` へ移す（`migrate_dirs`）
const OLD_DIR_NAME: &str = "wimux";

/// 設定・セッションの保存先（`%APPDATA%\SondevTerminal`）。`WIMUX_CONFIG_DIR` で変えられる
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WIMUX_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    pick_dir(&roaming_parent())
}

/// 設定を置く親フォルダ（Windows は %APPDATA%）
fn roaming_parent() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata);
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config")
}

/// ブラウザのデータなど大きいものを置く親フォルダ（Windows は %LOCALAPPDATA%）
pub fn local_parent() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(roaming_parent)
}

/// 親フォルダの中の保存先。新しい名前のフォルダがまだ無く、古い名前（wimux）のものがあれば
/// そちらを使う（移せなかったとき。前の設定を失わないように）
pub fn pick_dir(parent: &Path) -> PathBuf {
    let new = parent.join(DIR_NAME);
    let old = parent.join(OLD_DIR_NAME);
    if !new.exists() && old.is_dir() {
        old
    } else {
        new
    }
}

/// 古い保存先（`wimux`）を新しい名前（`SondevTerminal`）に移す。起動時に 1 回だけ呼ぶ。
/// 移せなかったとき（古い版が起動中でファイルが使われているなど）は古い場所をそのまま使う
pub fn migrate_dirs() {
    if std::env::var_os("WIMUX_CONFIG_DIR").is_none() {
        migrate_dir(&roaming_parent());
    }
    migrate_dir(&local_parent());
}

fn migrate_dir(parent: &Path) {
    let old = parent.join(OLD_DIR_NAME);
    let new = parent.join(DIR_NAME);
    if old.is_dir() && !new.exists() {
        match std::fs::rename(&old, &new) {
            Ok(()) => log::info!("保存先を移しました: {} → {}", old.display(), new.display()),
            Err(e) => log::warn!(
                "保存先を移せませんでした（{} のまま使います）: {e}",
                old.display()
            ),
        }
    }
}

pub fn session_path() -> PathBuf {
    config_dir().join("session.json")
}

pub fn dirs_path() -> PathBuf {
    config_dir().join("dirs.json")
}

pub fn load_session(path: &Path) -> Option<SessionFile> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<SessionFile>(&text) {
        Ok(s) if !s.workspaces.is_empty() => Some(s),
        Ok(_) => None,
        Err(e) => {
            log::warn!("session.json を読めませんでした: {e}");
            None
        }
    }
}

pub fn save_session(path: &Path, session: &SessionFile) -> std::io::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(session)?)
}

/// 書き込み途中で落ちても壊れないよう、一時ファイルに書いてから置き換える
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_old_dir() {
        let parent =
            std::env::temp_dir().join(format!("wimux-migrate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(parent.join(OLD_DIR_NAME)).unwrap();
        std::fs::write(parent.join(OLD_DIR_NAME).join("settings.json"), "{}").unwrap();
        // 移す前は古い場所を使う
        assert_eq!(pick_dir(&parent), parent.join(OLD_DIR_NAME));
        migrate_dir(&parent);
        assert!(parent.join(DIR_NAME).join("settings.json").is_file());
        assert!(!parent.join(OLD_DIR_NAME).exists());
        assert_eq!(pick_dir(&parent), parent.join(DIR_NAME));
        // 新しい場所がもうあれば何もしない
        std::fs::create_dir_all(parent.join(OLD_DIR_NAME)).unwrap();
        migrate_dir(&parent);
        assert!(parent.join(OLD_DIR_NAME).is_dir());
        assert_eq!(pick_dir(&parent), parent.join(DIR_NAME));
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn roundtrip() {
        let s = SessionFile {
            version: SESSION_VERSION,
            active_workspace: 0,
            workspaces: vec![WorkspaceState {
                name: Some("研究".into()),
                focused: 1,
                layout: Some(NodeState::Split {
                    axis: Axis::Horizontal,
                    ratio: 0.5,
                    first: Box::new(NodeState::Group {
                        active: 0,
                        tabs: vec![PaneState {
                            cwd: Some("C:\\work".into()),
                            conda: Some("labelme".into()),
                            claude: true,
                            claude_session: Some("3f2a9c1e-1234".into()),
                            ..Default::default()
                        }],
                    }),
                    second: Box::new(NodeState::Group {
                        active: 1,
                        tabs: vec![
                            PaneState {
                                agent: Some("codex".into()),
                                agent_session: Some("0199-abcd".into()),
                                ..Default::default()
                            },
                            PaneState::default(),
                        ],
                    }),
                }),
                ..Default::default()
            }],
        };
        let dir = std::env::temp_dir().join("wimux_session_test");
        let path = dir.join("session.json");
        save_session(&path, &s).unwrap();
        let loaded = load_session(&path).unwrap();
        assert_eq!(loaded.workspaces[0].name.as_deref(), Some("研究"));
        match loaded.workspaces[0].layout.as_ref().unwrap() {
            NodeState::Split { first, second, .. } => {
                match first.as_ref() {
                    NodeState::Group { tabs, .. } => {
                        assert!(tabs[0].claude);
                        assert_eq!(tabs[0].conda.as_deref(), Some("labelme"));
                    }
                    _ => panic!(),
                }
                match second.as_ref() {
                    NodeState::Group { active, tabs } => {
                        assert_eq!(*active, 1);
                        assert_eq!(tabs.len(), 2);
                        assert_eq!(tabs[0].agent.as_deref(), Some("codex"));
                        assert_eq!(tabs[0].agent_session.as_deref(), Some("0199-abcd"));
                    }
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn reads_v1() {
        let json = r#"{"version":1,"active_workspace":0,"workspaces":[{"active_tab":0,"tabs":[
            {"focused":0,"layout":{"type":"pane","cwd":"/tmp","claude":false}}]}]}"#;
        let s: SessionFile = serde_json::from_str(json).unwrap();
        assert_eq!(s.workspaces[0].tabs.len(), 1);
        assert!(s.workspaces[0].layout.is_none());
    }
}
