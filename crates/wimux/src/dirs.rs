//! ディレクトリ履歴（cdi 用）。zoxide と同じ「よく行く × 最近行った」でスコアを付ける。
//!
//! - シェル連携から cwd が届くたびに `record` する
//! - `%APPDATA%\wimux\dirs.json` に保存
//! - 初回（ファイルがないとき）は zoxide の履歴があれば取り込む

use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 1000;
/// ランクの合計がこれを超えたら全体を縮める（古い場所が自然に消えていく）
const MAX_TOTAL_RANK: f64 = 10_000.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    /// 表示用の元のパス（大文字小文字を保持）
    path: String,
    rank: f64,
    last_access: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DirHistory {
    entries: HashMap<String, Entry>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Windows ではパスの大文字小文字を区別しないので、キーは小文字で正規化する
fn key_of(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    let trimmed = if trimmed.ends_with(':') {
        path
    } else {
        trimmed
    };
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

impl DirHistory {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => {
                let mut h = Self::default();
                h.import_zoxide();
                h
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::session::write_atomic(path, &serde_json::to_vec(self)?)
    }

    pub fn record(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        let key = key_of(path);
        let entry = self.entries.entry(key).or_insert(Entry {
            path: String::new(),
            rank: 0.0,
            last_access: 0,
        });
        entry.path = path.to_string();
        entry.rank += 1.0;
        entry.last_access = now();
        self.age();
    }

    fn age(&mut self) {
        let total: f64 = self.entries.values().map(|e| e.rank).sum();
        if total > MAX_TOTAL_RANK {
            for e in self.entries.values_mut() {
                e.rank *= 0.9;
            }
            self.entries.retain(|_, e| e.rank >= 1.0);
        }
        if self.entries.len() > MAX_ENTRIES {
            let mut scored: Vec<(String, f64)> = self
                .entries
                .iter()
                .map(|(k, e)| (k.clone(), score(e, now())))
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (k, _) in scored.into_iter().skip(MAX_ENTRIES) {
                self.entries.remove(&k);
            }
        }
    }

    /// 検索。キーワード（空白区切り）をすべて含むものをスコア順で返す（存在しないフォルダは除く）
    pub fn query(&self, query: &str, limit: usize) -> Vec<String> {
        let keywords: Vec<String> = query.split_whitespace().map(|k| k.to_lowercase()).collect();
        let t = now();
        let mut hits: Vec<(&String, f64)> = self
            .entries
            .iter()
            .filter(|(key, _)| {
                let lower = key.to_lowercase();
                keywords.iter().all(|k| lower.contains(k.as_str()))
            })
            .map(|(_, e)| {
                let path = &e.path;
                // 最後のキーワードがフォルダ名に含まれていれば少し優先（zoxide と同じ考え方）
                let bonus = keywords.last().map_or(1.0, |k| {
                    let name = path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
                    if name.contains(k.as_str()) { 2.0 } else { 1.0 }
                });
                (path, score(e, t) * bonus)
            })
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        hits.into_iter()
            .map(|(p, _)| p)
            .filter(|p| Path::new(p).is_dir())
            .take(limit)
            .cloned()
            .collect()
    }

    /// zoxide の履歴を取り込む（`zoxide query -l -s` の出力: "  12.0 C:\path"）
    fn import_zoxide(&mut self) {
        let mut cmd = std::process::Command::new("zoxide");
        cmd.args(["query", "--list", "--score"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let Ok(out) = cmd.output() else { return };
        if !out.status.success() {
            return;
        }
        let t = now();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let line = line.trim();
            let Some((score, path)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            let Ok(rank) = score.trim().parse::<f64>() else {
                continue;
            };
            self.entries.insert(
                key_of(path.trim()),
                Entry {
                    path: display_path(path.trim()),
                    rank: rank.max(1.0),
                    // zoxide のスコアには時間が含まれているので「最近」として扱う
                    last_access: t,
                },
            );
        }
        log::info!(
            "zoxide から {} 件の履歴を取り込みました",
            self.entries.len()
        );
    }
}

/// zoxide と同じ時間減衰
fn score(e: &Entry, now: u64) -> f64 {
    let age = now.saturating_sub(e.last_access);
    let factor = if age < 3600 {
        4.0
    } else if age < 86_400 {
        2.0
    } else if age < 604_800 {
        0.5
    } else {
        0.25
    };
    e.rank * factor
}

/// 表示用。Windows のドライブ文字だけは大文字に戻す（キーは小文字で持っているため）
fn display_path(p: &str) -> String {
    let mut chars = p.chars();
    match (chars.next(), chars.next()) {
        (Some(d), Some(':')) if d.is_ascii_alphabetic() => {
            format!("{}{}", d.to_ascii_uppercase(), &p[1..])
        }
        _ => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_orders_by_frequency_and_filters() {
        let dir = std::env::temp_dir();
        let a = dir.join("wimux_test_a");
        let b = dir.join("wimux_test_b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let (a, b) = (
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
        );

        let mut h = DirHistory::default();
        h.record(&a);
        h.record(&b);
        h.record(&b);
        h.record("/definitely/not/exists");

        let all = h.query("", 10);
        assert_eq!(all.len(), 2);
        assert!(all[0].ends_with("wimux_test_b"));
        let only_a = h.query("test_a", 10);
        assert_eq!(only_a.len(), 1);
        assert!(only_a[0].ends_with("wimux_test_a"));
    }
}
