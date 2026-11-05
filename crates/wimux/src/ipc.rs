//! ペインの中で動くプログラム（Claude Code のフック・Codex の notify・`sondev notify`）から
//! wimux 本体へ知らせを届ける仕組み。
//!
//! フックの出力は端末に出ない（OSC を書いても届かない）ため、127.0.0.1 の TCP で受け取る。
//! - 本体は起動時に 127.0.0.1 の空いているポートで待ち受け、`WIMUX_IPC=<port>:<token>` をペインに渡す
//! - フックなどは `sondev.exe` を呼び、1 行の JSON を送って終わる（ペインは `WIMUX_PANE_ID` で特定）
//! - トークンが合わないものは捨てる（同じ PC のほかのプロセスからのなりすまし対策）

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// エージェント（Claude Code など）の状態の変化
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvent {
    /// セッション開始（待機中）
    Start,
    /// 作業を始めた（プロンプト送信など）
    Working,
    /// 許可・質問への回答を待って止まった
    Asking,
    /// 応答が終わった
    Done,
    /// しばらく入力がない（念押しの通知）
    IdleReminder,
    /// セッション終了
    End,
}

/// 本体に届く知らせ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Message {
    Agent {
        /// "claude" / "codex" など
        agent: String,
        event: AgentEvent,
        #[serde(default)]
        message: Option<String>,
    },
    Notify {
        #[serde(default)]
        title: Option<String>,
        body: String,
    },
    /// Claude の利用上限（5 時間枠・週）の使用率。Claude Code のステータスラインから届く
    Usage {
        #[serde(default)]
        five_hour: Option<LimitWindow>,
        #[serde(default)]
        seven_day: Option<LimitWindow>,
    },
}

/// 利用上限の 1 つの枠
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LimitWindow {
    /// 使った割合（0〜100）
    pub used_percentage: f64,
    /// 枠が戻る時刻（Unix 時刻・秒）
    pub resets_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    token: String,
    pane: String,
    #[serde(flatten)]
    message: Message,
}

/// 届いた知らせ（どのペインからか）
#[derive(Debug, Clone)]
pub struct Incoming {
    /// `WIMUX_PANE_ID`（`<プロセス ID>-<ペイン番号>`）
    pub pane: String,
    pub message: Message,
}

struct Server {
    env: String,
    rx: flume::Receiver<Incoming>,
}

static SERVER: OnceLock<Option<Server>> = OnceLock::new();

/// 待ち受けを始める（2 回目以降は何もしない）
pub fn start() {
    SERVER.get_or_init(|| match listen() {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("フックの受け口を開けませんでした（状態の通知はタイトルからの推定だけになります）: {e}");
            None
        }
    });
}

/// ペインに渡す `WIMUX_IPC` の値
pub fn env_value() -> Option<String> {
    SERVER.get()?.as_ref().map(|s| s.env.clone())
}

/// 届いた知らせの受け口
pub fn receiver() -> Option<flume::Receiver<Incoming>> {
    SERVER.get()?.as_ref().map(|s| s.rx.clone())
}

fn listen() -> std::io::Result<Server> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let (tx, rx) = flume::unbounded();
    let expected = token.clone();
    std::thread::Builder::new()
        .name("wimux-ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(incoming) = read_message(stream, &expected) {
                    let _ = tx.send(incoming);
                }
            }
        })?;
    Ok(Server {
        env: format!("{port}:{token}"),
        rx,
    })
}

fn read_message(mut stream: TcpStream, token: &str) -> Option<Incoming> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut buf = Vec::new();
    let _ = (&mut stream).take(64 * 1024).read_to_end(&mut buf);
    let env: Envelope = serde_json::from_slice(&buf).ok()?;
    if env.token != token || !crate::claude_session::valid_id(&env.pane) {
        return None;
    }
    Some(Incoming {
        pane: env.pane,
        message: env.message,
    })
}

/// 推測されにくいトークン（標準ライブラリのハッシュの乱数の種を使う。依存を増やさないため）
fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u32(std::process::id());
        if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            h.write_u128(t.as_nanos());
        }
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// ペインの中から本体へ送る（フック・`sondev notify` 用）。wimux の外では何もしない
pub fn send(message: Message) -> bool {
    let (Ok(ipc), Ok(pane)) = (std::env::var("WIMUX_IPC"), std::env::var("WIMUX_PANE_ID")) else {
        return false;
    };
    let Some((port, token)) = ipc.split_once(':') else {
        return false;
    };
    let Ok(port) = port.parse::<u16>() else {
        return false;
    };
    let env = Envelope {
        token: token.to_string(),
        pane,
        message,
    };
    let Ok(bytes) = serde_json::to_vec(&env) else {
        return false;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    // フックは Claude Code を待たせるので、つながらなければすぐあきらめる
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
        return false;
    };
    stream.write_all(&bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{AgentEvent, Envelope, Message};

    #[test]
    fn wire_format() {
        let env = Envelope {
            token: "t".into(),
            pane: "1-2".into(),
            message: Message::Agent {
                agent: "claude".into(),
                event: AgentEvent::Asking,
                message: Some("Claude needs your permission".into()),
            },
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains(r#""kind":"agent""#));
        assert!(json.contains(r#""event":"asking""#));
        let back: Envelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back.message, env.message);

        let n: Envelope =
            serde_json::from_str(r#"{"token":"t","pane":"1-2","kind":"notify","body":"hi"}"#)
                .unwrap();
        assert_eq!(
            n.message,
            Message::Notify {
                title: None,
                body: "hi".into()
            }
        );
    }
}
