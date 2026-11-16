//! Replying to a message: the user's message carries `meta.reply_to`, a short quote of an earlier message in the
//! same agent's chat, and the agent gets that message (who, when, its text) before the user's.

use crate::db::{Agent, Message};
use crate::state::AppState;
use serde_json::{json, Value};

/// The excerpt kept on the reply, for clients to draw the quote without fetching the original.
const EXCERPT_CHARS: usize = 200;
/// How much of the quoted message the agent gets.
const QUOTE_CHARS: usize = 2000;

/// Who wrote `m`: an agent's name, or None for the user.
fn author(m: &Message, agent: &Agent) -> Option<String> {
    if let Some(name) = m.meta["from_agent"]["name"].as_str() {
        return Some(name.to_string());
    }
    (m.role != "user").then(|| agent.name.clone())
}

fn cut(s: &str, max: usize) -> (String, bool) {
    match s.char_indices().nth(max) {
        Some((i, _)) => (s[..i].to_string(), true),
        None => (s.to_string(), false),
    }
}

/// Only chat messages can be quoted: the user's, another agent's and the agent's text, not run markers, tools or thinking.
fn quotable(m: &Message) -> bool {
    m.kind == "text" && (m.role == "user" || m.role == "assistant")
}

/// `meta.reply_to` for a reply to message `id`: refused unless it's a chat message in `agent`'s own chat.
pub fn reply_meta(state: &AppState, agent: &Agent, id: i64) -> Result<Value, String> {
    let m = state.db.get_message(&agent.id, id).ok().flatten().ok_or("The quoted message isn't in this chat.")?;
    if !quotable(&m) {
        return Err("Only chat messages can be quoted.".into());
    }
    let one_line = m.content.split_whitespace().collect::<Vec<_>>().join(" ");
    let (excerpt, long) = cut(&one_line, EXCERPT_CHARS);
    Ok(json!({
        "id": m.id,
        "role": m.role,
        "author": author(&m, agent),
        "created_at": m.created_at,
        "excerpt": if long { format!("{excerpt}…") } else { excerpt },
    }))
}

/// What the agent reads before the user's message when it replies to one: who said it, when, and the text, each
/// line quoted with `> ` so the text can't pass for Loft's own words.
pub fn prompt_block(state: &AppState, agent_id: &str, meta: &Value) -> Option<String> {
    let id = meta["reply_to"]["id"].as_i64()?;
    let m = state.db.get_message(agent_id, id).ok().flatten()?;
    let who = match meta["reply_to"]["author"].as_str() {
        Some(name) => format!("{name}'s message"),
        None => "their own message".to_string(),
    };
    let when = m.created_at.get(..16).unwrap_or(&m.created_at).replace('T', " ");
    let total = m.content.chars().count();
    let (text, long) = cut(m.content.trim(), QUOTE_CHARS);
    let quoted = text.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n");
    let note = if long { format!("\n(Quote cut: the first {QUOTE_CHARS} of {total} characters.)") } else { String::new() };
    Some(format!("[The user is replying to {who} from {when} UTC:]\n{quoted}{note}"))
}

#[cfg(test)]
mod tests {
    use crate::tasks::tests::{agent, busy, state};
    use axum::extract::{Path, State};
    use axum::Json;
    use serde_json::{json, Value};

    async fn send(st: &crate::state::AppState, to: &str, text: &str, reply_to: Value) -> Result<Value, String> {
        crate::api::send_message(State(st.clone()), Path(to.into()), Json(serde_json::from_value(json!({ "text": text, "reply_to": reply_to })).unwrap()))
            .await
            .map(|Json(v)| v)
            .map_err(|e| e.1)
    }

    fn queued(st: &crate::state::AppState, id: &str) -> String {
        st.rt.lock().unwrap().queued.get(id).map(|q| q.join("\n---\n")).unwrap_or_default()
    }

    #[tokio::test]
    async fn a_reply_is_stored_listed_and_reaches_the_agent_quote_first() {
        let st = state();
        let a = agent(&st, "Dev");
        busy(&st, &a);
        let asked = crate::agents::add_msg(&st, &a.id, None, "assistant", "text", "Which port should\nthe server use?", json!({})).unwrap();
        let r = send(&st, &a.id, "7420, please", json!(asked.id)).await.unwrap();
        let quote = &r["message"]["meta"]["reply_to"];
        assert_eq!((quote["id"].clone(), quote["author"].clone(), quote["excerpt"].clone()), (json!(asked.id), json!("Dev"), json!("Which port should the server use?")));

        let listed = st.db.list_messages(&a.id, 10, None).unwrap();
        assert_eq!(listed.last().unwrap().meta["reply_to"], *quote);

        let q = queued(&st, &a.id);
        let when = asked.created_at[..16].replace('T', " ");
        assert!(q.starts_with(&format!("[The user is replying to Dev's message from {when} UTC:]\n> Which port should\n> the server use?\n\n7420, please")), "{q}");
    }

    #[tokio::test]
    async fn a_reply_to_the_users_own_message_says_so() {
        let st = state();
        let a = agent(&st, "Dev");
        busy(&st, &a);
        let mine = send(&st, &a.id, "Ship it on Friday", Value::Null).await.unwrap();
        st.rt.lock().unwrap().queued.remove(&a.id);
        let r = send(&st, &a.id, "Make that Monday", mine["message"]["id"].clone()).await.unwrap();
        assert_eq!(r["message"]["meta"]["reply_to"]["author"], Value::Null);
        assert!(queued(&st, &a.id).starts_with("[The user is replying to their own message from "));
    }

    #[tokio::test]
    async fn a_reply_to_another_agents_message_or_a_missing_one_is_refused() {
        let st = state();
        let a = agent(&st, "Dev");
        let b = agent(&st, "QA");
        let theirs = crate::agents::add_msg(&st, &b.id, None, "assistant", "text", "Tests pass", json!({})).unwrap();
        for id in [json!(theirs.id), json!(999_999)] {
            assert_eq!(send(&st, &a.id, "ok?", id).await.unwrap_err(), "The quoted message isn't in this chat.");
        }
        assert!(st.db.list_messages(&a.id, 10, None).unwrap().is_empty(), "nothing is stored");
    }

    #[tokio::test]
    async fn only_chat_messages_can_be_quoted() {
        let st = state();
        let a = agent(&st, "Dev");
        for (role, kind) in [("system", "run_start"), ("system", "clear"), ("assistant", "thinking"), ("assistant", "tool_use"), ("tool", "tool_result")] {
            let m = crate::agents::add_msg(&st, &a.id, None, role, kind, "user", json!({})).unwrap();
            assert_eq!(send(&st, &a.id, "hello?", json!(m.id)).await.unwrap_err(), "Only chat messages can be quoted.", "{kind}");
        }
    }

    #[tokio::test]
    async fn a_long_quote_is_cut_for_the_agent_and_the_excerpt() {
        let st = state();
        let a = agent(&st, "Dev");
        busy(&st, &a);
        let long = "é".repeat(5000);
        let m = crate::agents::add_msg(&st, &a.id, None, "assistant", "text", &long, json!({})).unwrap();
        let r = send(&st, &a.id, "too long", json!(m.id)).await.unwrap();
        assert_eq!(r["message"]["meta"]["reply_to"]["excerpt"].as_str().unwrap().chars().count(), 201, "200 and an ellipsis");
        let q = queued(&st, &a.id);
        assert!(q.contains(&format!("> {}\n(Quote cut: the first 2000 of 5000 characters.)\n\ntoo long", "é".repeat(2000))), "{q}");
    }

    #[tokio::test]
    async fn a_reply_from_another_agent_is_quoted_with_its_name() {
        let st = state();
        let a = agent(&st, "Dev");
        busy(&st, &a);
        let from = crate::agents::add_msg(&st, &a.id, None, "user", "text", "Review done", json!({ "from_agent": { "id": "x", "name": "QA" } })).unwrap();
        let r = send(&st, &a.id, "thanks", json!(from.id)).await.unwrap();
        assert_eq!(r["message"]["meta"]["reply_to"]["author"], json!("QA"));
        assert!(queued(&st, &a.id).starts_with("[The user is replying to QA's message from "));
    }
}
