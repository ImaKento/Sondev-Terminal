//! ターミナルの文字列から URL・ファイルパスを見つける（Ctrl+クリックで開く）。
//!
//! 画面のセル → 文字列への変換は view.rs 側で行い、ここは文字列だけを扱う（テストしやすいように）。
//!
//! 対応する形:
//! - `https://example.com/a?b=c`（http / https / file / ftp）
//! - `src/main.rs`、`./src/main.rs:42`、`src/main.rs:42:7`、`C:\repo\a.rs:10`、`/home/x/a.rs`、`~/a.txt`
//! - `src/main.rs(42,7)`（MSVC 形式）

use std::path::{Path, PathBuf};

/// 見つかったリンク
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Url(String),
    Path {
        path: String,
        line: Option<u32>,
        col: Option<u32>,
    },
}

/// 区切りとして扱う文字（パス・URL には含めない）
fn is_delim(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\''
                | '`'
                | '<'
                | '>'
                | '|'
                | '['
                | ']'
                | '{'
                | '}'
                | '│'
                | '─'
                | '╭'
                | '╮'
                | '╰'
                | '╯'
                | '┃'
                | '•'
                | '·'
                | '⎿'
        )
        || ('\u{2500}'..='\u{257F}').contains(&c) // 罫線（Claude Code の枠など）
}

/// 末尾に付いていても、リンクの一部とはみなさない文字
fn is_trailing_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | ':' | '!' | '?' | '、' | '。' | '）' | '」'
    )
}

/// `chars` の `at` 番目の文字を含むリンクを探す。見つかれば（文字の範囲, リンク）
pub fn find_at(chars: &[char], at: usize) -> Option<(std::ops::Range<usize>, Link)> {
    if at >= chars.len() || is_delim(chars[at]) {
        return None;
    }
    // 区切りまで左右に広げる
    let mut start = at;
    while start > 0 && !is_delim(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && !is_delim(chars[end]) {
        end += 1;
    }

    // URL: トークンの途中に http:// などがあれば、そこから始める（`url=https://…` など）
    let token: String = chars[start..end].iter().collect();
    for scheme in ["https://", "http://", "file://", "ftp://"] {
        if let Some(byte_pos) = token.find(scheme) {
            let s = start + token[..byte_pos].chars().count();
            if at < s {
                break;
            }
            let mut e = end;
            trim_url_end(chars, s, &mut e);
            if at >= e || e - s <= scheme.len() {
                return None;
            }
            let url: String = chars[s..e].iter().collect();
            return Some((s..e, Link::Url(url)));
        }
    }

    // パス: かっこも区切りにする（Claude Code の `Update(src/a.rs)` など）
    let paren = |c: char| is_delim(c) || matches!(c, '(' | ')' | '（' | '）' | '「' | '」');
    if paren(chars[at]) {
        return None;
    }
    let mut s = at;
    while s > 0 && !paren(chars[s - 1]) {
        s -= 1;
    }
    let mut e = at + 1;
    while e < chars.len() && !paren(chars[e]) {
        e += 1;
    }
    // MSVC 形式 `file(12,3)` の行番号
    let paren_pos = (e < chars.len() && chars[e] == '(')
        .then(|| {
            let rest: String = chars[e..chars.len().min(e + 24)].iter().collect();
            parse_paren_position(&rest)
        })
        .flatten();
    while e > s && is_trailing_punct(chars[e - 1]) {
        e -= 1;
    }
    if s >= e || at >= e {
        return None;
    }
    let text: String = chars[s..e].iter().collect();
    let (path, l, c) = match paren_pos {
        Some((l, c)) => (text, Some(l), c),
        None => split_line_col(&text),
    };
    if !looks_like_path(&path) {
        return None;
    }
    Some((
        s..e,
        Link::Path {
            path,
            line: l,
            col: c,
        },
    ))
}

/// URL の末尾の句読点・対応の取れない閉じかっこを落とす
fn trim_url_end(chars: &[char], s: usize, e: &mut usize) {
    while *e > s {
        let c = chars[*e - 1];
        if is_trailing_punct(c) {
            *e -= 1;
            continue;
        }
        if c == ')' {
            let opens = chars[s..*e].iter().filter(|c| **c == '(').count();
            let closes = chars[s..*e].iter().filter(|c| **c == ')').count();
            if closes > opens {
                *e -= 1;
                continue;
            }
        }
        break;
    }
}

/// `(12,3)…` / `(12)…` → Some((12, Some(3)))
fn parse_paren_position(text: &str) -> Option<(u32, Option<u32>)> {
    let inner = text.strip_prefix('(')?;
    let inner = &inner[..inner.find(')')?];
    let mut it = inner.split(',');
    let line = it.next()?.trim().parse::<u32>().ok()?;
    let col = it.next().and_then(|n| n.trim().parse::<u32>().ok());
    Some((line, col))
}

/// `a.rs:12:3` → ("a.rs", Some(12), Some(3))。`C:\a.rs` のドライブ文字は残す
fn split_line_col(text: &str) -> (String, Option<u32>, Option<u32>) {
    let mut parts: Vec<&str> = text.rsplitn(3, ':').collect();
    parts.reverse();
    let num = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    match parts.as_slice() {
        [p, l, c] if num(l).is_some() && num(c).is_some() && !is_drive(p) => {
            (p.to_string(), num(l), num(c))
        }
        [p, l] if num(l).is_some() && !is_drive(p) => (p.to_string(), num(l), None),
        [a, b, l] if num(l).is_some() && is_drive(a) => (format!("{a}:{b}"), num(l), None),
        _ => (text.to_string(), None, None),
    }
}

fn is_drive(s: &str) -> bool {
    s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic()
}

/// パスらしいか（区切り文字か拡張子がある）。実在するかは `resolve` で確かめる
fn looks_like_path(p: &str) -> bool {
    if p.is_empty() || p.len() > 1024 || p.chars().all(|c| c == '.' || c == '/' || c == '\\') {
        return false;
    }
    if p.contains('/') || p.contains('\\') || p.starts_with('~') {
        return true;
    }
    // 拡張子（`main.rs`、`README.md`）。`1.5` や `e.g` のようなものは除く
    match p.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=8).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// パスを cwd から解決し、実在すれば絶対パスを返す
pub fn resolve(path: &str, cwd: Option<&str>) -> Option<PathBuf> {
    let expanded: PathBuf =
        if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
            home()?.join(rest)
        } else if path == "~" {
            home()?
        } else {
            PathBuf::from(path)
        };
    // git の差分表示の `a/src/x.rs` `b/src/x.rs` にも対応する
    let mut candidates = vec![expanded.clone()];
    if let Some(rest) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
        candidates.push(PathBuf::from(rest));
    }
    for p in candidates {
        let full = if p.is_absolute() {
            p
        } else {
            Path::new(cwd?).join(p)
        };
        if full.exists() {
            return Some(full);
        }
    }
    None
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{Link, find_at, split_line_col};

    fn at(s: &str, needle: &str) -> Option<Link> {
        let chars: Vec<char> = s.chars().collect();
        let byte = s.find(needle).expect("needle");
        let idx = s[..byte].chars().count();
        find_at(&chars, idx).map(|(_, l)| l)
    }

    fn path(p: &str, line: Option<u32>, col: Option<u32>) -> Option<Link> {
        Some(Link::Path {
            path: p.into(),
            line,
            col,
        })
    }

    #[test]
    fn urls() {
        assert_eq!(
            at("see https://example.com/a?b=c.", "example"),
            Some(Link::Url("https://example.com/a?b=c".into()))
        );
        assert_eq!(
            at("(https://en.wikipedia.org/wiki/Rust_(language))", "wiki"),
            Some(Link::Url(
                "https://en.wikipedia.org/wiki/Rust_(language)".into()
            ))
        );
        assert_eq!(
            at("url=http://localhost:3000/x", "localhost"),
            Some(Link::Url("http://localhost:3000/x".into()))
        );
        assert_eq!(
            at("<https://a.dev>", "a.dev"),
            Some(Link::Url("https://a.dev".into()))
        );
        // 日本語の文の中
        assert_eq!(
            at("詳しくは https://a.dev/x 、を参照", "a.dev"),
            Some(Link::Url("https://a.dev/x".into()))
        );
    }

    #[test]
    fn paths() {
        assert_eq!(
            at("  src/main.rs:42:7 error", "main"),
            path("src/main.rs", Some(42), Some(7))
        );
        assert_eq!(
            at("Edited ./a/b.ts:10.", "b.ts"),
            path("./a/b.ts", Some(10), None)
        );
        assert_eq!(
            at("at C:\\repo\\x.rs:12", "repo"),
            path("C:\\repo\\x.rs", Some(12), None)
        );
        assert_eq!(
            at("at C:\\repo\\x.rs", "repo"),
            path("C:\\repo\\x.rs", None, None)
        );
        assert_eq!(
            at("Update(README.md)", "README"),
            path("README.md", None, None)
        );
        assert_eq!(
            at("foo.cpp(12,5): error", "foo"),
            path("foo.cpp", Some(12), Some(5))
        );
        assert_eq!(
            at("open `Cargo.toml` now", "Cargo"),
            path("Cargo.toml", None, None)
        );
        assert_eq!(at("│ src/lib.rs │", "lib"), path("src/lib.rs", None, None));
        assert_eq!(at("~/notes.md", "notes"), path("~/notes.md", None, None));
    }

    #[test]
    fn not_links() {
        assert_eq!(at("version 1.5 released", "1.5"), None);
        assert_eq!(at("hello world", "world"), None);
        assert_eq!(at("...", "."), None);
    }

    #[test]
    fn line_col() {
        assert_eq!(split_line_col("C:\\a.rs"), ("C:\\a.rs".into(), None, None));
        assert_eq!(
            split_line_col("C:\\a.rs:3"),
            ("C:\\a.rs".into(), Some(3), None)
        );
        assert_eq!(
            split_line_col("a.rs:3:4"),
            ("a.rs".into(), Some(3), Some(4))
        );
        assert_eq!(split_line_col("a:b"), ("a:b".into(), None, None));
    }
}
