//! PTY 出力から wimux が使う OSC シーケンスだけを拾い出すスキャナ。
//!
//! バイト列はそのまま alacritty_terminal にも流す（未知の OSC は向こうで無視される）。
//! ここでは「シェル連携」に必要な情報だけを抜き出して `ShellEvent` にする。
//!
//! 対応しているシーケンス:
//! - `OSC 9;9;"<path>"`                 … カレントディレクトリ（Windows Terminal 方式）
//! - `OSC 7;file://<host>/<path>`       … カレントディレクトリ（xterm/WezTerm 方式）
//! - `OSC 1337;SetUserVar=<NAME>=<b64>` … 任意の変数（conda 環境名など）
//! - `OSC 9;<message>`                  … 通知（iTerm2 方式。Claude Code も出せる）
//! - `OSC 777;notify;<title>;<body>`    … 通知（urxvt / Ghostty 方式）

use base64::Engine as _;

/// シェルから届いたイベント。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    Cwd(String),
    UserVar { name: String, value: String },
    Notify { title: Option<String>, body: String },
}

const MAX_OSC_LEN: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Osc,
    OscEsc,
}

/// 状態を持ったストリーミングスキャナ。チャンク境界をまたいでも正しく動く。
pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
}

impl Default for OscScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl OscScanner {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            buf: Vec::with_capacity(256),
        }
    }

    /// バイト列を流し込み、見つかったイベントを `out` に追加する。
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<ShellEvent>) {
        for &b in bytes {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Esc;
                    }
                }
                State::Esc => {
                    if b == b']' {
                        self.buf.clear();
                        self.state = State::Osc;
                    } else if b == 0x1b {
                        self.state = State::Esc;
                    } else {
                        self.state = State::Ground;
                    }
                }
                State::Osc => match b {
                    0x07 => self.finish(out),
                    0x1b => self.state = State::OscEsc,
                    _ => {
                        if self.buf.len() < MAX_OSC_LEN {
                            self.buf.push(b);
                        } else {
                            // 異常に長い OSC は捨てる
                            self.state = State::Ground;
                        }
                    }
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish(out);
                    } else if b == b']' {
                        // ESC ] が来たら新しい OSC として扱う
                        self.buf.clear();
                        self.state = State::Osc;
                    } else {
                        self.state = State::Ground;
                    }
                }
            }
        }
    }

    fn finish(&mut self, out: &mut Vec<ShellEvent>) {
        self.state = State::Ground;
        let payload = String::from_utf8_lossy(&self.buf).into_owned();
        if let Some(ev) = parse_osc(&payload) {
            out.push(ev);
        }
    }
}

fn parse_osc(payload: &str) -> Option<ShellEvent> {
    let (code, rest) = payload.split_once(';')?;
    match code {
        "9" => {
            if let Some(path) = rest.strip_prefix("9;") {
                let path = path.trim().trim_matches('"');
                (!path.is_empty()).then(|| ShellEvent::Cwd(path.to_string()))
            } else if rest
                .split_once(';')
                .is_some_and(|(sub, _)| !sub.is_empty() && sub.bytes().all(|b| b.is_ascii_digit()))
            {
                // 9;4;… (プログレス) など ConEmu 系のサブコマンドは通知ではない
                None
            } else {
                Some(ShellEvent::Notify {
                    title: None,
                    body: rest.to_string(),
                })
            }
        }
        "7" => parse_file_url(rest).map(ShellEvent::Cwd),
        "1337" => {
            let kv = rest.strip_prefix("SetUserVar=")?;
            let (name, b64) = kv.split_once('=')?;
            let value = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .ok()?;
            Some(ShellEvent::UserVar {
                name: name.to_string(),
                value: String::from_utf8_lossy(&value).into_owned(),
            })
        }
        "777" => {
            let rest = rest.strip_prefix("notify;")?;
            let (title, body) = rest.split_once(';').unwrap_or(("", rest));
            Some(ShellEvent::Notify {
                title: (!title.is_empty()).then(|| title.to_string()),
                body: body.to_string(),
            })
        }
        _ => None,
    }
}

/// `file://host/C:/Users/x` → `C:\Users\x`、`file://host/home/x` → `/home/x`
fn parse_file_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let decoded = percent_decode(path);
    let bytes = decoded.as_bytes();
    // "/C:/..." 形式なら Windows パスに直す
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        Some(decoded[1..].replace('/', "\\"))
    } else {
        Some(decoded)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<ShellEvent> {
        let mut s = OscScanner::new();
        let mut out = Vec::new();
        for c in chunks {
            s.feed(c, &mut out);
        }
        out
    }

    #[test]
    fn cwd_windows_terminal_style() {
        let ev = scan(&[b"PS> \x1b]9;9;\"C:\\Users\\kento\\wimux\"\x1b\\"]);
        assert_eq!(ev, vec![ShellEvent::Cwd("C:\\Users\\kento\\wimux".into())]);
    }

    #[test]
    fn cwd_split_across_chunks() {
        let ev = scan(&[b"\x1b]9;9;C:\\Us", b"ers\x07"]);
        assert_eq!(ev, vec![ShellEvent::Cwd("C:\\Users".into())]);
    }

    #[test]
    fn cwd_osc7() {
        let ev = scan(&[b"\x1b]7;file://imaken/C:/Users/kento/my%20dir\x07"]);
        assert_eq!(ev, vec![ShellEvent::Cwd("C:\\Users\\kento\\my dir".into())]);
    }

    #[test]
    fn user_var() {
        // "py311" を base64 化したもの
        let ev = scan(&[b"\x1b]1337;SetUserVar=WIMUX_CONDA=cHkzMTE=\x07"]);
        assert_eq!(
            ev,
            vec![ShellEvent::UserVar {
                name: "WIMUX_CONDA".into(),
                value: "py311".into()
            }]
        );
    }

    #[test]
    fn notifications() {
        let ev = scan(&[
            b"\x1b]9;Claude is waiting\x07\x1b]777;notify;Claude;Done\x1b\\\x1b]9;4;1;50\x07",
        ]);
        assert_eq!(
            ev,
            vec![
                ShellEvent::Notify {
                    title: None,
                    body: "Claude is waiting".into()
                },
                ShellEvent::Notify {
                    title: Some("Claude".into()),
                    body: "Done".into()
                },
            ]
        );
    }

    #[test]
    fn ignores_other_sequences() {
        let ev = scan(&[b"\x1b[31mred\x1b[0m\x1b]0;title\x07"]);
        assert!(ev.is_empty());
    }
}
