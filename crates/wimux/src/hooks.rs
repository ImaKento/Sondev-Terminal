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
    use super::{AgentEvent, claude_event, codex_event, shorten};
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
