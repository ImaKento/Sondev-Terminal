//! AI エージェントのフック・通知を、本体へ送る状態の変化に読み替える。
//!
//! - Claude Code: `sondev.exe --claude-hook`（フック。claude_session.rs の `run_hook`）
//! - Codex CLI : `sondev.exe --codex-notify <JSON>`（設定の `notify`。応答が終わるたびに呼ばれる）
//! - そのほか  : `sondev notify [--title <題>] <本文>`（スクリプトや任意のエージェントのフックから）

use serde_json::Value;

use crate::ipc::{self, AgentEvent, Message};

/// 通知の本文の長さの上限（文字数）
const MAX_BODY: usize = 200;

/// Claude Code のフックの入力 → 状態の変化
pub fn claude_event(v: &Value) -> Option<(AgentEvent, Option<String>)> {
    let message = v
        .get("message")
        .and_then(|m| m.as_str())
        .map(|m| shorten(m, MAX_BODY));
    let event = match v.get("hook_event_name").and_then(|e| e.as_str()) {
        None | Some("SessionStart") => AgentEvent::Start,
        Some("UserPromptSubmit") | Some("PreToolUse") | Some("PostToolUse") => AgentEvent::Working,
        Some("Stop") => AgentEvent::Done,
        Some("SessionEnd") => AgentEvent::End,
        Some("Notification") => {
            let kind = v.get("notification_type").and_then(|t| t.as_str());
            match kind {
                Some("permission_prompt") | Some("elicitation_dialog") => AgentEvent::Asking,
                Some("idle_prompt") => AgentEvent::IdleReminder,
                Some(_) => return None,
                // 古い Claude Code は種類を送らないので本文で見分ける
                None if message
                    .as_deref()
                    .is_some_and(|m| m.contains("waiting for your input")) =>
                {
                    AgentEvent::IdleReminder
                }
                None => AgentEvent::Asking,
            }
        }
        Some(_) => return None,
    };
    Some((event, message))
}

/// `sondev.exe --codex-notify <JSON>`: Codex CLI の notify から呼ばれる
pub fn run_codex_notify(args: &[String]) {
    let Some(v) = args
        .last()
        .and_then(|a| serde_json::from_str::<Value>(a).ok())
    else {
        return;
    };
    if let Some((event, message)) = codex_event(&v) {
        ipc::send(Message::Agent {
            agent: "codex".into(),
            event,
            message,
        });
    }
}

fn codex_event(v: &Value) -> Option<(AgentEvent, Option<String>)> {
    match v.get("type").and_then(|t| t.as_str())? {
        "agent-turn-complete" => Some((
            AgentEvent::Done,
            v.get("last-assistant-message")
                .and_then(|m| m.as_str())
                .map(|m| shorten(m, MAX_BODY)),
        )),
        _ => None,
    }
}

/// `sondev.exe --claude-statusline`: Claude Code のステータスラインとして呼ばれる。
/// 標準入力の JSON から利用上限（5 時間枠・週）を取り出して本体へ送り、
/// Claude Code の画面の下に出す 1 行（モデル名と使用率）を書く
pub fn run_statusline() {
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(v) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    let (five_hour, seven_day) = rate_limits(&v);
    if five_hour.is_some() || seven_day.is_some() {
        ipc::send(Message::Usage {
            five_hour,
            seven_day,
        });
    }
    println!("{}", statusline_text(&v, five_hour, seven_day, now_secs()));
}

/// ステータスラインの入力の `rate_limits.five_hour` / `rate_limits.seven_day`
/// （Pro / Max などのサブスクリプションで、最初の応答のあとにだけある）
fn rate_limits(v: &Value) -> (Option<ipc::LimitWindow>, Option<ipc::LimitWindow>) {
    let window = |name: &str| {
        let w = v.get("rate_limits")?.get(name)?;
        Some(ipc::LimitWindow {
            used_percentage: w.get("used_percentage")?.as_f64()?,
            resets_at: w.get("resets_at")?.as_i64()?,
        })
    };
    (window("five_hour"), window("seven_day"))
}

/// 「Opus · 5h 23%（あと 2時間13分） · 週 41%」
fn statusline_text(
    v: &Value,
    five_hour: Option<ipc::LimitWindow>,
    seven_day: Option<ipc::LimitWindow>,
    now: i64,
) -> String {
    let mut parts = Vec::new();
    if let Some(model) = v
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(|m| m.as_str())
    {
        parts.push(model.to_string());
    }
    if let Some(w) = five_hour {
        parts.push(format!(
            "5h {:.0}%（{}）",
            w.used_percentage,
            until_reset(w.resets_at, now)
        ));
    }
    if let Some(w) = seven_day {
        parts.push(format!("週 {:.0}%", w.used_percentage));
    }
    parts.join(" · ")
}

/// 枠が戻るまでの時間（「あと 2時間13分」）
pub fn until_reset(resets_at: i64, now: i64) -> String {
    let secs = (resets_at - now).max(0);
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("あと {d}日{h}時間")
    } else if h > 0 {
        format!("あと {h}時間{m}分")
    } else {
        format!("あと {}分", m.max(1))
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `sondev notify [--title <題>] <本文…>`: ペインから通知を出す
pub fn run_notify(args: &[String]) {
    let mut title = None;
    let mut body = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-t" | "--title" => title = it.next().cloned(),
            _ => body.push(a.clone()),
        }
    }
    let body = body.join(" ");
    let body = if body.trim().is_empty() {
        "完了しました".to_string()
    } else {
        shorten(&body, MAX_BODY)
    };
    if !ipc::send(Message::Notify { title, body }) {
        eprintln!("sondev notify: Sondev Terminal のペインの中で実行してください");
        std::process::exit(1);
    }
}

/// 1 行にまとめて、長ければ切り詰める
fn shorten(s: &str, max: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let mut t: String = one_line.chars().take(max).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentEvent, claude_event, codex_event, rate_limits, shorten, statusline_text, until_reset,
    };

    #[test]
    fn statusline() {
        let v = json!({
            "model": {"display_name": "Opus"},
            "rate_limits": {
                "five_hour": {"used_percentage": 23.5, "resets_at": 1000 + 2 * 3600 + 13 * 60},
                "seven_day": {"used_percentage": 41.2, "resets_at": 1000 + 3 * 86400}
            }
        });
        let (f, s) = rate_limits(&v);
        assert_eq!(f.unwrap().used_percentage, 23.5);
        assert_eq!(
            statusline_text(&v, f, s, 1000),
            "Opus · 5h 24%（あと 2時間13分） · 週 41%"
        );
        // サブスクリプションでない・最初の応答の前は rate_limits が無い
        let v = json!({"model": {"display_name": "Sonnet"}});
        assert_eq!(rate_limits(&v), (None, None));
        assert_eq!(statusline_text(&v, None, None, 0), "Sonnet");
        assert_eq!(until_reset(1000 + 3 * 86400 + 3600, 1000), "あと 3日1時間");
        assert_eq!(until_reset(1000, 2000), "あと 1分");
    }
    use serde_json::json;

    #[test]
    fn claude_events() {
        let ev = |v| claude_event(&v).map(|(e, _)| e);
        assert_eq!(
            ev(json!({"hook_event_name": "UserPromptSubmit"})),
            Some(AgentEvent::Working)
        );
        assert_eq!(
            ev(json!({"hook_event_name": "Stop"})),
            Some(AgentEvent::Done)
        );
        assert_eq!(
            ev(json!({"hook_event_name": "SessionEnd"})),
            Some(AgentEvent::End)
        );
        assert_eq!(ev(json!({"session_id": "x"})), Some(AgentEvent::Start));
        assert_eq!(
            ev(
                json!({"hook_event_name": "Notification", "notification_type": "permission_prompt",
                      "message": "Claude needs your permission to use Bash"})
            ),
            Some(AgentEvent::Asking)
        );
        assert_eq!(
            ev(json!({"hook_event_name": "Notification", "notification_type": "idle_prompt"})),
            Some(AgentEvent::IdleReminder)
        );
        assert_eq!(
            ev(
                json!({"hook_event_name": "Notification", "message": "Claude is waiting for your input"})
            ),
            Some(AgentEvent::IdleReminder)
        );
        assert_eq!(
            ev(json!({"hook_event_name": "Notification", "notification_type": "auth_success"})),
            None
        );
    }

    #[test]
    fn codex_events() {
        let v = json!({"type": "agent-turn-complete", "last-assistant-message": "Done.\nAll tests pass."});
        assert_eq!(
            codex_event(&v),
            Some((AgentEvent::Done, Some("Done. All tests pass.".into())))
        );
        assert_eq!(codex_event(&json!({"type": "other"})), None);
    }

    #[test]
    fn shorten_long() {
        assert_eq!(shorten("abc", 2), "ab…");
        assert_eq!(shorten("a\n b", 10), "a b");
    }
}
