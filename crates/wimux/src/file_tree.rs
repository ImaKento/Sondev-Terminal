//! 右側のファイル一覧（フォーカス中のペインのフォルダ）。UI に依存しない部分。
//!
//! - 一覧のルートはフォーカス中のペインの cwd。cd すると追従する
//! - フォルダはクリックで開く / 閉じる。開いたフォルダはルートごとに覚えておく（ワークスペースを
//!   切り替えて戻ってきても同じ開き方）
//! - フォルダの読み込みは UI スレッドの外で行う（`read_dirs`）。表示中は数秒ごとに読み直す
//! - git のリポジトリの中なら、変更のあるファイル（と、それを含むフォルダ）に色を付ける（`git_status`）

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 1 つのフォルダに表示する上限（node_modules などで固まらないように）
pub const MAX_ENTRIES: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// 読み込んだフォルダの中身
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// 上限を超えて表示しなかった数
    pub more: usize,
    /// 読めなかった（権限がない・消えた）
    pub error: bool,
}

/// git の状態（大きいほど優先して色を付ける）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GitMark {
    /// .gitignore で無視されている
    Ignored,
    /// 新しいファイル（まだ git add していない）
    Untracked,
    /// 追加した（git add 済みの新しいファイル）
    Added,
    /// 変更・名前の変更・削除（フォルダは中に変更がある）
    Modified,
    /// マージの衝突
    Conflict,
}

/// リポジトリ全体の git の状態
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// ファイル・フォルダごとの状態（変更のあるファイルを含むフォルダにも付ける）
    marks: HashMap<PathBuf, GitMark>,
    /// 中身ごと新しい・無視されているフォルダ（`foo/` と出るもの）。下のものすべてに効く
    dirs: Vec<(PathBuf, GitMark)>,
}

impl GitStatus {
    pub fn mark(&self, path: &Path) -> Option<GitMark> {
        if let Some(m) = self.marks.get(path) {
            return Some(*m);
        }
        self.dirs
            .iter()
            .filter(|(d, _)| path.starts_with(d))
            .map(|(_, m)| *m)
            .max()
    }
}

/// `git status --porcelain=v1 -z` の出力を読む（パスはリポジトリのいちばん上からの相対パス）
pub fn parse_git_status(top: &Path, out: &[u8]) -> GitStatus {
    let mut st = GitStatus::default();
    let mut items = out.split(|b| *b == 0);
    while let Some(item) = items.next() {
        if item.len() < 4 {
            continue;
        }
        let (x, y) = (item[0], item[1]);
        let rel = String::from_utf8_lossy(&item[3..]).into_owned();
        // 名前の変更・コピーは、次に元の名前が来る
        if x == b'R' || x == b'C' {
            items.next();
        }
        let mark = match (x, y) {
            (b'?', b'?') => GitMark::Untracked,
            (b'!', b'!') => GitMark::Ignored,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => GitMark::Conflict,
            (b'A', _) => GitMark::Added,
            _ => GitMark::Modified,
        };
        let is_dir = rel.ends_with('/');
        // git は / で区切る。Windows でもエントリのパスと同じ形になるよう 1 つずつつなぐ
        let mut path = top.to_path_buf();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            path.push(part);
        }
        if is_dir {
            st.dirs.push((path.clone(), mark));
        } else {
            st.marks.insert(path.clone(), mark);
        }
        // 無視されているものは親フォルダに色を付けない
        if mark == GitMark::Ignored {
            continue;
        }
        let mut parent = path.parent();
        while let Some(p) = parent {
            if !p.starts_with(top) {
                break;
            }
            let e = st.marks.entry(p.to_path_buf()).or_insert(mark);
            if *e < mark {
                *e = mark;
            }
            if p == top {
                break;
            }
            parent = p.parent();
        }
    }
    st
}

/// `dir` を含むリポジトリのいちばん上のフォルダ（.git があるところ）
pub fn git_top(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// リポジトリの状態を git に聞く（バックグラウンドで呼ぶ。git が無い・リポジトリでなければ None）
pub fn git_status(root: &Path) -> Option<GitStatus> {
    let top = git_top(root)?;
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C")
        .arg(&top)
        .args([
            // ユーザーの git の操作とぶつからないよう、index のロックを取らない
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--ignored=matching",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_git_status(&top, &out.stdout))
}

/// 表示する 1 行
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Entry {
        depth: usize,
        entry: Entry,
        /// フォルダが開いているか
        open: bool,
        git: Option<GitMark>,
    },
    /// 「… ほか N 件」
    More { depth: usize, count: usize },
    /// まだ読み込んでいない・読めない
    Note { depth: usize, text: &'static str },
}

#[derive(Debug, Default)]
pub struct FileTree {
    root: Option<PathBuf>,
    /// ルートごとの、開いているフォルダ
    expanded: HashMap<PathBuf, HashSet<PathBuf>>,
    listings: HashMap<PathBuf, Listing>,
    /// 読み込み中のフォルダ（同じフォルダを二重に読まない）
    loading: HashSet<PathBuf>,
    /// git の状態（リポジトリの外なら None）
    git: Option<GitStatus>,
    /// git status を実行中（二重に実行しない）
    git_loading: bool,
}

impl FileTree {
    /// ルートを変える。変わったら true（読み込みが要る）
    pub fn set_root(&mut self, root: Option<PathBuf>) -> bool {
        if self.root == root {
            return false;
        }
        self.root = root;
        // 前のルートの中身は捨てる（開き方は expanded に残っている）
        self.listings.clear();
        self.git = None;
        true
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// git status を始めてよければ true（実行中にする）
    pub fn start_git(&mut self) -> bool {
        if self.git_loading || self.root.is_none() {
            return false;
        }
        self.git_loading = true;
        true
    }

    /// git status の結果を入れる（`root` はそのとき聞いたルート）。変わったら true
    pub fn apply_git(&mut self, root: &Path, status: Option<GitStatus>) -> bool {
        self.git_loading = false;
        if self.root.as_deref() != Some(root) || self.git == status {
            return false;
        }
        self.git = status;
        true
    }

    fn expanded_set(&self) -> Option<&HashSet<PathBuf>> {
        self.expanded.get(self.root.as_ref()?)
    }

    pub fn is_open(&self, dir: &Path) -> bool {
        self.expanded_set().is_some_and(|s| s.contains(dir))
    }

    /// フォルダを開く / 閉じる。開いて中身がまだ無ければ true（読み込みが要る）
    pub fn toggle(&mut self, dir: &Path) -> bool {
        let Some(root) = self.root.clone() else {
            return false;
        };
        let set = self.expanded.entry(root).or_default();
        if !set.remove(dir) {
            set.insert(dir.to_path_buf());
            return !self.listings.contains_key(dir);
        }
        false
    }

    /// 読み直すフォルダ（ルートと、ルートから辿れる開いているフォルダ）
    pub fn dirs_to_load(&self) -> Vec<PathBuf> {
        let Some(root) = &self.root else {
            return Vec::new();
        };
        let mut out = vec![root.clone()];
        if let Some(set) = self.expanded_set() {
            let mut open: Vec<_> = set
                .iter()
                .filter(|d| d.starts_with(root) && *d != root)
                .cloned()
                .collect();
            open.sort();
            out.extend(open);
        }
        out
    }

    /// まだ読み込んでいない（読み込み中でもない）フォルダ。読み込みを始めたことにする
    pub fn take_missing(&mut self) -> Vec<PathBuf> {
        let missing: Vec<_> = self
            .dirs_to_load()
            .into_iter()
            .filter(|d| !self.listings.contains_key(d) && !self.loading.contains(d))
            .collect();
        self.loading.extend(missing.iter().cloned());
        missing
    }

    /// 読み込みを始めたことにする（定期的な読み直し）
    pub fn mark_loading(&mut self, dirs: &[PathBuf]) {
        self.loading.extend(dirs.iter().cloned());
    }

    /// 読み込んだ結果を入れる。中身が変わったら true
    pub fn apply(&mut self, results: Vec<(PathBuf, Listing)>) -> bool {
        let mut changed = false;
        for (dir, listing) in results {
            self.loading.remove(&dir);
            // ルートが変わったあとに届いた古い結果は捨てる
            let Some(root) = &self.root else {
                continue;
            };
            if !dir.starts_with(root) {
                continue;
            }
            if self.listings.get(&dir) != Some(&listing) {
                self.listings.insert(dir, listing);
                changed = true;
            }
        }
        changed
    }

    /// 表示する行（開いているフォルダの中身を字下げして並べる）
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            self.push_rows(root, 0, &mut out);
        }
        out
    }

    fn push_rows(&self, dir: &Path, depth: usize, out: &mut Vec<Row>) {
        let Some(listing) = self.listings.get(dir) else {
            out.push(Row::Note {
                depth,
                text: "読み込み中…",
            });
            return;
        };
        if listing.error {
            out.push(Row::Note {
                depth,
                text: "読めません",
            });
            return;
        }
        for entry in &listing.entries {
            let open = entry.is_dir && self.is_open(&entry.path);
            out.push(Row::Entry {
                depth,
                entry: entry.clone(),
                open,
                git: self.git.as_ref().and_then(|g| g.mark(&entry.path)),
            });
            if open {
                self.push_rows(&entry.path, depth + 1, out);
            }
        }
        if listing.more > 0 {
            out.push(Row::More {
                depth,
                count: listing.more,
            });
        }
    }
}

/// フォルダを読む（フォルダが先、それぞれ名前順。大文字・小文字は区別しない）
pub fn read_dir(dir: &Path) -> Listing {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Listing {
            error: true,
            ..Default::default()
        };
    };
    let mut entries: Vec<Entry> = rd
        .flatten()
        .map(|e| {
            let path = e.path();
            // シンボリックリンク・ジャンクションは先がフォルダならフォルダとして扱う
            let is_dir = e
                .file_type()
                .map(|t| t.is_dir() || (t.is_symlink() && path.is_dir()))
                .unwrap_or(false);
            Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                path,
                is_dir,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let more = entries.len().saturating_sub(MAX_ENTRIES);
    entries.truncate(MAX_ENTRIES);
    Listing {
        entries,
        more,
        error: false,
    }
}

/// いくつかのフォルダをまとめて読む（バックグラウンドで呼ぶ）
pub fn read_dirs(dirs: Vec<PathBuf>) -> Vec<(PathBuf, Listing)> {
    dirs.into_iter()
        .map(|d| {
            let l = read_dir(&d);
            (d, l)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{FileTree, GitMark, Row, parse_git_status, read_dir, read_dirs};

    #[test]
    fn git_status() {
        let top = std::path::Path::new("/repo");
        let out = b" M src/main.rs\0?? new.txt\0A  src/added.rs\0R  src/b.rs\0src/a.rs\0UU c.rs\0!! target/\0?? docs/\0";
        let st = parse_git_status(top, out);
        let m = |p: &str| st.mark(&top.join(p));
        assert_eq!(m("src/main.rs"), Some(GitMark::Modified));
        assert_eq!(m("new.txt"), Some(GitMark::Untracked));
        assert_eq!(m("src/added.rs"), Some(GitMark::Added));
        // 名前の変更は新しい名前に付け、元の名前は読み飛ばす
        assert_eq!(m("src/b.rs"), Some(GitMark::Modified));
        assert_eq!(m("src/a.rs"), None);
        assert_eq!(m("c.rs"), Some(GitMark::Conflict));
        // フォルダは中のいちばん強い状態
        assert_eq!(m("src"), Some(GitMark::Modified));
        // 無視・新しいフォルダは下のものすべてに効く。無視は親に広げない
        assert_eq!(m("target/debug/x"), Some(GitMark::Ignored));
        assert_eq!(m("docs/a/b.md"), Some(GitMark::Untracked));
        assert_eq!(m("README.md"), None);
    }

    #[test]
    fn tree() {
        let dir = std::env::temp_dir().join(format!("wimux-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/sub")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("b.txt"), "").unwrap();
        std::fs::write(dir.join("A.md"), "").unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();

        // フォルダが先、名前順（大文字・小文字は区別しない）
        let l = read_dir(&dir);
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [".git", "src", "A.md", "b.txt"]);

        let mut t = FileTree::default();
        assert!(t.set_root(Some(dir.clone())));
        assert!(!t.set_root(Some(dir.clone())));
        let missing = t.take_missing();
        assert_eq!(missing, vec![dir.clone()]);
        assert!(t.take_missing().is_empty(), "読み込み中は二重に読まない");
        assert!(matches!(t.rows()[0], Row::Note { .. }));
        assert!(t.apply(read_dirs(missing)));
        assert_eq!(t.rows().len(), 4);

        // src を開く → 中身を読む → 字下げして表示
        let src = dir.join("src");
        assert!(t.toggle(&src));
        let missing = t.take_missing();
        assert_eq!(missing, vec![src.clone()]);
        t.apply(read_dirs(missing));
        let rows = t.rows();
        assert_eq!(rows.len(), 6);
        assert!(matches!(&rows[1], Row::Entry { open: true, .. }));
        assert!(matches!(&rows[2], Row::Entry { depth: 1, entry, .. } if entry.name == "sub"));
        // 変わっていなければ changed = false
        assert!(!t.apply(read_dirs(t.dirs_to_load())));

        // 別のフォルダへ移って戻ると、開き方は覚えている
        t.set_root(Some(src.clone()));
        t.set_root(Some(dir.clone()));
        assert!(t.is_open(&src));
        // 閉じる
        assert!(!t.toggle(&src));
        assert!(!t.is_open(&src));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable() {
        let l = read_dir(std::path::Path::new("/no/such/dir/wimux"));
        assert!(l.error);
    }
}
