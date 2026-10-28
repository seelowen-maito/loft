//! Minimal MCP server (streamable HTTP transport, JSON responses) exposed to each
//! agent at `/mcp/{agent_id}`. It gives agents memory, live status, avatar control,
//! permission requests and messaging between agents, and it is the
//! `--permission-prompt-tool` target.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::agents::{self, ask_permission, emit_agent, rule_matches, suggest_rule};
use crate::avatar::Avatar;
use crate::db::Agent;
use crate::state::{AppState, Event};

/// The two kinds of Agent message, as `message_agent` explains them to the sender.
const KINDS: &str = "`needs_action` when the recipient must act now (review, fix, decide, or tell the user something they must act on): it wakes them, right away or after their current run. `fyi` when they only need to know: it shows in their chat at once but doesn't wake them; they read it at the start of their next run and don't reply to it.";

/// Words and phrases that only acknowledge. A message made of nothing else (plus
/// punctuation, emoji and the recipient's name) carries nothing the recipient needs.
const ACKS: &[&str] = &[
    "thanks", "thank you", "thx", "ty", "tysm", "cheers", "much appreciated", "appreciated", "appreciate it",
    "so much", "very much", "a lot", "again", "for the update", "for the review", "for the heads up", "for letting me know",
    "ok", "okay", "k", "kk", "alright", "all right", "sure", "cool", "great", "perfect", "nice", "awesome", "all good",
    "got it", "gotcha", "understood", "noted", "roger", "roger that", "ack", "acknowledged", "copy", "copy that",
    "will do", "sounds good", "sounds great", "makes sense", "on it", "no problem", "np",
    // Waiting and standing by only as whole phrases: a bare "wait" or "next" is an instruction.
    "standing by", "waiting", "waiting for you", "waiting for your next", "waiting for your next task", "waiting for your next message",
    "waiting for your next instructions", "ill wait", "i will wait", "will wait", "ill be here", "i will be here", "im here",
    "im on it", "ill do it", "ready for the next", "ready for the next task", "ready for the next one", "ready for your next task",
    "ready when you are", "let me know", "if you need anything", "if you need anything else", "and",
];

/// Whether `text` is only an acknowledgement, matched as plain words: no model call.
fn only_acknowledges(text: &str, to: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        s.to_lowercase().replace(['\'', '\u{2019}'], "").split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
    };
    let mut phrases: Vec<Vec<String>> = ACKS.iter().map(|p| words(p)).chain([words(to)]).filter(|p| !p.is_empty()).collect();
    phrases.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let (text, mut i) = (words(text), 0);
    while i < text.len() {
        match phrases.iter().find(|p| text[i..].starts_with(p)) {
            Some(p) => i += p.len(),
            None => return false,
        }
    }
    true
}

fn tools(state: &AppState) -> Value {
    let mut all = base_tools();
    // With the messaging rules off, every message wakes its recipient, so the kind is optional.
    if !state.settings.get().messaging_rules {
        for t in all.as_array_mut().unwrap().iter_mut().filter(|t| t["name"] == "message_agent") {
            t["inputSchema"]["required"] = json!(["agent", "text"]);
        }
    }
    all.as_array_mut().unwrap().extend(crate::tasks::mcp_tools());
    all.as_array_mut().unwrap().extend(crate::setup::mcp_tools());
    all.as_array_mut().unwrap().extend(crate::grill::mcp_tools());
    all.as_array_mut().unwrap().extend(crate::planning::mcp_tools());
    all.as_array_mut().unwrap().extend(crate::greenfield::mcp_tools());
    all
}

fn base_tools() -> Value {
    json!([
        {
            "name": "remember",
            "description": "Save a durable long-term memory. It persists across conversations and session resets.",
            "inputSchema": { "type": "object", "properties": {
                "content": { "type": "string", "description": "The fact to remember, short and self-contained." },
                "tags": { "type": "string", "description": "Optional comma-separated tags." }
            }, "required": ["content"] }
        },
        {
            "name": "recall",
            "description": "Search long-term memory. Omit query to list everything.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string" }
            } }
        },
        {
            "name": "forget",
            "description": "Delete a memory by its id (the #number).",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "integer" }
            }, "required": ["id"] }
        },
        {
            "name": "set_status",
            "description": "Set the short live status line shown next to your avatar in the Loft UI.",
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string", "description": "Present tense, under 80 chars." }
            }, "required": ["text"] }
        },
        {
            "name": "request_permission",
            "description": "Ask the user to grant a permission rule (Claude Code syntax, e.g. 'Bash(npm *)', 'WebFetch', 'mcp__github'). Blocks until the user answers. When granted, it is saved to your config permanently.",
            "inputSchema": { "type": "object", "properties": {
                "permission": { "type": "string" },
                "reason": { "type": "string", "description": "Why you need it." }
            }, "required": ["permission", "reason"] }
        },
        {
            "name": "share_pull_request",
            "description": "Show a pull request you opened as a live card in the user's chat (status, checks, merge button). Not needed after `gh pr create`: that is detected automatically.",
            "inputSchema": { "type": "object", "properties": {
                "url": { "type": "string", "description": "https://github.com/<owner>/<repo>/pull/<number>" }
            }, "required": ["url"] }
        },
        {
            "name": "get_avatar",
            "description": "Return your current sticker avatar as SVG.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "set_avatar",
            "description": "Replace your sticker avatar. `svg` is a vector drawing with viewBox=\"0 0 100 100\" made of plain shapes (path, circle, ellipse, rect, polygon, line, g, gradients, clipPath). No text, images, styles or scripts. Loft adds the white die-cut sticker border and shadow.",
            "inputSchema": { "type": "object", "properties": {
                "svg": { "type": "string", "description": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">…</svg>" }
            }, "required": ["svg"] }
        },
        {
            "name": "list_agents",
            "description": "List the other Loft agents: id, name, title, project, and what each is doing now.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "read_agent_chat",
            "description": "Read another agent's chat: messages from the user and other agents, its replies, and its tool calls. Most recent last.",
            "inputSchema": { "type": "object", "properties": {
                "agent": { "type": "string", "description": "The agent's id or name." },
                "limit": { "type": "integer", "description": "How many recent messages to read (default 50, max 200)." }
            }, "required": ["agent"] }
        },
        {
            "name": "message_agent",
            "description": format!("Send a message to another agent. Returns as soon as it is delivered; it doesn't wait for an answer. Every message has a kind: {KINDS} A needs-action message is answered, if at all, with a message of its own that arrives here as a new turn."),
            "inputSchema": { "type": "object", "properties": {
                "agent": { "type": "string", "description": "The recipient's id or name." },
                "text": { "type": "string", "description": "The message." },
                "kind": { "type": "string", "enum": ["needs_action", "fyi"], "description": "needs_action: the recipient must act now; it wakes them. fyi: they only need to know; they read it on their next run." }
            }, "required": ["agent", "text", "kind"] }
        },
        {
            "name": "approval_prompt",
            "description": "Internal: Loft permission gate. Do not call directly.",
            "inputSchema": { "type": "object", "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" }
            }, "required": ["tool_name", "input"] }
        }
    ])
}

/// An agent by id, or by name ignoring case.
fn find_agent(state: &AppState, key: &str) -> Result<Agent, Value> {
    let key = key.trim();
    if key.is_empty() {
        return Err(tool_error("agent is required"));
    }
    let all = state.db.list_agents().map_err(|e| tool_error(e.to_string()))?;
    let i = all.iter().position(|a| a.id == key).or_else(|| all.iter().position(|a| a.name.eq_ignore_ascii_case(key)));
    i.map(|i| all[i].clone())
        .ok_or_else(|| tool_error(format!("No agent called {key}. Call list_agents to see who exists.")))
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

fn text(t: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }] })
}

fn tool_error(t: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }], "isError": true })
}

async fn call_tool(state: &AppState, agent_id: &str, name: &str, args: Value) -> Value {
    if let Ok(Some(me)) = state.db.get_agent(agent_id) {
        if let Some(r) = match crate::setup::mcp_call(state, &me, name, &args).await {
            None => match crate::planning::mcp_call(state, &me, name, &args).await {
                None => crate::greenfield::mcp_call(state, &me, name, &args).await,
                r => r,
            },
            r => r,
        } {
            return match r {
                Ok(t) => text(t),
                Err(e) => tool_error(e),
            };
        }
        let find = |k: &str| find_agent(state, k).map_err(|e| e["content"][0]["text"].as_str().unwrap_or("").to_string());
        if let Some(r) = crate::tasks::mcp_call(state, &me, name, &args, find).or_else(|| crate::grill::mcp_call(state, &me, name, &args)) {
            return match r {
                Ok(t) => text(t),
                Err(e) => tool_error(e),
            };
        }
    }
    let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
    match name {
        "remember" => {
            let content = s("content");
            if content.trim().is_empty() {
                return tool_error("content is required");
            }
            match state.db.add_memory(agent_id, content.trim(), s("tags").trim()) {
                Ok(m) => {
                    state.emit(Event::Memory { agent_id: agent_id.into() });
                    text(format!("Saved memory #{}", m.id))
                }
                Err(e) => tool_error(e.to_string()),
            }
        }
        "recall" => {
            let q = s("query");
            let res = if q.trim().is_empty() {
                state.db.list_memories(agent_id)
            } else {
                state.db.search_memories(agent_id, q.trim())
            };
            match res {
                Ok(ms) if ms.is_empty() => text("No matching memories."),
                Ok(ms) => text(
                    ms.iter()
                        .map(|m| format!("#{} ({}) [{}] {}", m.id, &m.created_at[..10], m.tags, m.content))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                Err(e) => tool_error(e.to_string()),
            }
        }
        "forget" => {
            let id = args["id"].as_i64().or_else(|| s("id").trim_start_matches('#').parse().ok());
            match id.map(|id| state.db.delete_memory(agent_id, id)) {
                Some(Ok(true)) => {
                    state.emit(Event::Memory { agent_id: agent_id.into() });
                    text("Forgotten.")
                }
                Some(Ok(false)) => tool_error("No memory with that id."),
                Some(Err(e)) => tool_error(e.to_string()),
                None => tool_error("id is required"),
            }
        }
        "set_status" => {
            agents::set_status_text(state, agent_id, &s("text"));
            text("Status updated.")
        }
        "share_pull_request" => {
            let links = crate::github::extract_pr_links(&s("url"));
            let Some((slug, n)) = links.into_iter().next() else {
                return tool_error("url must look like https://github.com/<owner>/<repo>/pull/<number>");
            };
            // The runner links the card to this tool call when its result comes back.
            match crate::github::get(state, &format!("/repos/{slug}/pulls/{n}")).await {
                Ok(_) => text(format!("The user can now see a card for https://github.com/{slug}/pull/{n}.")),
                Err(e) => tool_error(format!("Couldn't read that pull request: {e}")),
            }
        }
        "get_avatar" => match state.db.get_agent(agent_id) {
            Ok(Some(a)) => text(a.avatar.svg),
            _ => tool_error("agent not found"),
        },
        "set_avatar" => {
            let avatar = Avatar { svg: s("svg") };
            match avatar.validate() {
                Ok(av) => match state.db.update_agent(agent_id, |a| a.avatar = av) {
                    Ok(Some(a)) => {
                        emit_agent(state, &a);
                        text("Avatar updated. The user can see it now.")
                    }
                    _ => tool_error("agent not found"),
                },
                Err(e) => tool_error(format!("Invalid avatar: {e}. Fix it and call set_avatar again.")),
            }
        }
        "request_permission" => {
            let perm = s("permission").trim().to_string();
            if perm.is_empty() {
                return tool_error("permission is required");
            }
            let d = ask_permission(state, agent_id, "request", &perm, json!({}), &s("reason"), perm.clone()).await;
            if d.allow {
                text(format!("Granted: {perm} is now in your allowed tools."))
            } else {
                text(format!("Denied.{}", d.message.map(|m| format!(" User note: {m}")).unwrap_or_default()))
            }
        }
        "list_agents" => match state.db.list_agents() {
            Ok(all) => {
                let lines: Vec<String> = all
                    .iter()
                    .filter(|a| a.id != agent_id)
                    .map(|a| {
                        let title = if a.title.is_empty() { String::new() } else { format!(" ({})", a.title) };
                        let project = if a.project.trim().is_empty() { String::new() } else { format!(" · project {}", a.project.trim()) };
                        let doing = if agents::is_running(state, &a.id) {
                            format!("working: {}", if a.status_text.is_empty() { "…" } else { &a.status_text })
                        } else {
                            a.status.clone()
                        };
                        format!("- {}{} · id {}{project} · {}", a.name, title, a.id, doing)
                    })
                    .collect();
                if lines.is_empty() { text("You are the only agent.") } else { text(lines.join("\n")) }
            }
            Err(e) => tool_error(e.to_string()),
        },
        "read_agent_chat" => {
            let a = match find_agent(state, &s("agent")) {
                Ok(a) => a,
                Err(e) => return e,
            };
            let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 200);
            let msgs = match state.db.list_messages(&a.id, limit, None) {
                Ok(m) => m,
                Err(e) => return tool_error(e.to_string()),
            };
            let lines: Vec<String> = msgs
                .iter()
                .filter_map(|m| {
                    let who = match (m.role.as_str(), m.kind.as_str()) {
                        ("user", "text") => match m.meta["from_agent"]["name"].as_str() {
                            Some(n) => format!("agent {n}"),
                            None => "user".into(),
                        },
                        ("assistant", "text") => a.name.clone(),
                        ("assistant", "tool_use") => format!("{} (tool)", a.name),
                        (_, "error") => "error".into(),
                        (_, "clear") => return Some(format!("[{}] --- context cleared ---", &m.created_at[..16])),
                        _ => return None,
                    };
                    Some(format!("[{}] {who}: {}", &m.created_at[..16], clip(&m.content, 2000)))
                })
                .collect();
            if lines.is_empty() { text(format!("{}'s chat is empty.", a.name)) } else { text(lines.join("\n")) }
        }
        "message_agent" => {
            let to = match find_agent(state, &s("agent")) {
                Ok(a) => a,
                Err(e) => return e,
            };
            let body = s("text");
            if body.trim().is_empty() {
                return tool_error("text is required");
            }
            if to.id == agent_id {
                return tool_error("You can't message yourself.");
            }
            let Ok(Some(from)) = state.db.get_agent(agent_id) else {
                return tool_error("agent not found");
            };
            let rules = state.settings.get().messaging_rules;
            if rules && only_acknowledges(&body, &to.name) {
                return tool_error(format!(
                    "Not sent: this message only acknowledges (thanks, ok, got it, standing by…), and {} doesn't need it. Send a message only when it carries a finding, report, verdict or request; otherwise send nothing. If it answers a question, say what you decided.",
                    to.name
                ));
            }
            // With the messaging rules off, every message wakes its recipient, as before kinds existed.
            let fyi = match (rules, s("kind").as_str()) {
                (false, _) | (true, "needs_action") => false,
                (true, "fyi") => true,
                (true, "") => return tool_error(format!("Not sent: kind is required. Use {KINDS}")),
                (true, k) => return tool_error(format!("Not sent: kind {k:?} isn't one of the two. Use {KINDS}")),
            };
            if fyi {
                agents::fyi_from_agent(state, &from, &to, body.trim());
                return text(format!("Delivered as FYI. {} will read it at the start of its next run.", to.name));
            }
            let busy = agents::is_running(state, &to.id);
            agents::message_from_agent(state, &from, &to, body.trim());
            text(if busy {
                format!("Delivered. {} is busy and will read it when its current run ends.", to.name)
            } else {
                format!("Delivered. {} is reading it now.", to.name)
            })
        }
        "approval_prompt" => {
            let tool = s("tool_name");
            let input = args["input"].clone();
            // Rules granted mid-run are honored immediately, without re-prompting.
            let already = state
                .db
                .get_agent(agent_id)
                .ok()
                .flatten()
                .map(|a| a.allowed_tools.iter().any(|r| rule_matches(r, &tool, &input)))
                .unwrap_or(false);
            let d = if already {
                crate::state::Decision { allow: true, message: None }
            } else {
                let rule = suggest_rule(&tool, &input);
                ask_permission(state, agent_id, "prompt", &tool, input.clone(), "", rule).await
            };
            let body = if d.allow {
                json!({ "behavior": "allow", "updatedInput": input })
            } else {
                json!({ "behavior": "deny", "message": d.message.unwrap_or_else(|| "The user denied this action.".into()) })
            };
            text(body.to_string())
        }
        _ => tool_error(format!("unknown tool {name}")),
    }
}

pub async fn handle(State(state): State<AppState>, Path(agent_id): Path<String>, Json(req): Json<Value>) -> Response {
    if state.db.get_agent(&agent_id).ok().flatten().is_none() {
        return (StatusCode::NOT_FOUND, "unknown agent").into_response();
    }
    // Notifications and responses carry no id and need no reply.
    let Some(id) = req.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let method = req["method"].as_str().unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "loft", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Loft: your memory, status, avatar, and permission tools."
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools(&state) }),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            call_tool(&state, &agent_id, name, args).await
        }
        "resources/list" => json!({ "resources": [] }),
        "prompts/list" => json!({ "prompts": [] }),
        _ => {
            return Json(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("method not found: {method}") }
            }))
            .into_response();
        }
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::{fake_claude_state, until};
    use crate::agents::{enqueue, is_running, workspace};
    use std::time::Duration;

    async fn message(st: &AppState, from: &Agent, args: Value) -> Value {
        call_tool(st, &from.id, "message_agent", args).await
    }

    /// Each prompt the fake claude got, in order.
    fn prompts(st: &AppState, a: &Agent) -> Vec<String> {
        let log = std::fs::read_to_string(workspace(st, &a.id).join("prompts.log")).unwrap_or_default();
        log.split("\n---\n").filter(|p| !p.is_empty()).map(String::from).collect()
    }

    async fn run(st: &AppState, a: &Agent, text: &str) -> String {
        let n = prompts(st, a).len();
        enqueue(st, &a.id, Some(text.into()), "user");
        until("the run", 10, || !is_running(st, &a.id) && prompts(st, a).len() > n).await;
        prompts(st, a).pop().unwrap()
    }

    fn rules_off(st: &AppState) {
        let mut s = st.settings.get();
        s.messaging_rules = false;
        st.settings.save(s).unwrap();
    }

    fn hint(from: &Agent) -> String {
        format!("To answer {}, call `mcp__loft__message_agent`", from.name)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_without_a_kind_is_rejected_with_both_kinds_explained() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        for kind in [None, Some("urgent")] {
            let mut args = json!({ "agent": to.name, "text": "Review PR #12" });
            if let Some(k) = kind {
                args["kind"] = json!(k);
            }
            let r = message(&st, &from, args).await;
            let err = r["content"][0]["text"].as_str().unwrap();
            assert_eq!(r["isError"], json!(true));
            assert!(err.starts_with("Not sent") && err.contains("`needs_action` when the recipient must act now") && err.contains("`fyi` when they only need to know"), "{err}");
        }
        assert!(st.db.list_messages(&to.id, 50, None).unwrap().is_empty() && !is_running(&st, &to.id));
        // The tool says so up front.
        let tool = tools(&st).as_array().unwrap().iter().find(|t| t["name"] == "message_agent").unwrap().clone();
        assert_eq!(tool["inputSchema"]["required"], json!(["agent", "text", "kind"]));
        assert!(tool["description"].as_str().unwrap().contains(KINDS));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn needs_action_starts_a_run_with_the_reply_hint() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        let r = message(&st, &from, json!({ "agent": to.id, "text": "Review PR #12", "kind": "needs_action" })).await;
        assert!(r["content"][0]["text"].as_str().unwrap().starts_with("Delivered."), "{r}");
        until("the run", 10, || !is_running(&st, &to.id) && !prompts(&st, &to).is_empty()).await;
        let p = &prompts(&st, &to)[0];
        assert!(p.contains("[Message from agent Lead") && p.contains("Review PR #12") && p.contains(&hint(&from)), "{p}");
        let m = st.db.list_messages(&to.id, 50, None).unwrap().into_iter().find(|m| m.role == "user").unwrap();
        assert_eq!((m.content.as_str(), m.meta.get("fyi")), ("Review PR #12", None));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_fyi_starts_no_run_and_reaches_the_next_run_only() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        let r = message(&st, &from, json!({ "agent": to.name, "text": "Committed abc123 on main", "kind": "fyi" })).await;
        assert_eq!(r["content"][0]["text"], json!("Delivered as FYI. Worker will read it at the start of its next run."));
        // In the chat at once, marked, with no run.
        let m = st.db.list_messages(&to.id, 50, None).unwrap().into_iter().find(|m| m.role == "user").unwrap();
        assert_eq!((m.content.as_str(), m.meta["fyi"].clone(), m.meta["from_agent"]["name"].clone()), ("Committed abc123 on main", json!(true), json!("Lead")));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!is_running(&st, &to.id) && prompts(&st, &to).is_empty());

        // Whatever starts the next run (here the user), the FYI comes first, without the reply hint.
        let p = run(&st, &to, "what's next?").await;
        assert!(p.starts_with("[FYI from agent Lead") && p.contains("Committed abc123 on main") && p.ends_with("what's next?"), "{p}");
        assert!(!p.contains(&hint(&from)) && !p.contains("mcp__loft__message_agent"), "{p}");
        // Only once.
        let p = run(&st, &to, "and now?").await;
        assert!(!p.contains("Committed abc123"), "{p}");
        assert!(st.db.get_agent(&to.id).unwrap().unwrap().pending_fyis.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pending_fyis_survive_clear_and_resume() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        let fyi = |text: &'static str| message(&st, &from, json!({ "agent": to.id, "text": text, "kind": "fyi" }));
        run(&st, &to, "first").await;
        assert_eq!(st.db.get_agent(&to.id).unwrap().unwrap().session_id.as_deref(), Some("s1"));

        // Resumed: the next run goes on in the same session.
        fyi("Tests pass on abc123").await;
        let p = run(&st, &to, "resumed run").await;
        assert!(p.contains("Tests pass on abc123"), "{p}");
        let starts: Vec<_> = st.db.list_messages(&to.id, 50, None).unwrap().into_iter().filter(|m| m.kind == "run_start").collect();
        assert_eq!(starts.last().unwrap().meta["resumed"], json!(true));

        // Cleared: the session is dropped, the FYI isn't.
        fyi("Merged #12").await;
        assert!(crate::api::reset_session(axum::extract::State(st.clone()), axum::extract::Path(to.id.clone())).await.is_ok());
        let p = run(&st, &to, "fresh run").await;
        assert!(p.contains("Merged #12") && !p.contains("Tests pass on abc123"), "{p}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_the_global_setting_off_every_message_wakes_as_before() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        rules_off(&st);
        let tool = tools(&st).as_array().unwrap().iter().find(|t| t["name"] == "message_agent").unwrap().clone();
        assert_eq!(tool["inputSchema"]["required"], json!(["agent", "text"]));
        for (i, args) in [json!({ "agent": to.id, "text": "no kind" }), json!({ "agent": to.id, "text": "said fyi", "kind": "fyi" })].into_iter().enumerate() {
            let r = message(&st, &from, args.clone()).await;
            assert!(r["content"][0]["text"].as_str().unwrap().starts_with("Delivered."), "{r}");
            until("the run", 10, || !is_running(&st, &to.id) && prompts(&st, &to).len() > i).await;
            let p = &prompts(&st, &to)[i];
            assert!(p.contains(args["text"].as_str().unwrap()) && p.contains(&hint(&from)), "{p}");
        }
        assert!(st.db.list_messages(&to.id, 50, None).unwrap().iter().all(|m| m.meta.get("fyi").is_none()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_fyi_is_lost_to_concurrent_writes_of_the_same_agent() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        // The recipient's own run writes its status while FYIs to it arrive.
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let (st, from, to) = (st.clone(), from.clone(), to.clone());
                std::thread::spawn(move || {
                    for i in 0..100 {
                        agents::fyi_from_agent(&st, &from, &to, &format!("fyi {t}-{i}"));
                        agents::set_status_text(&st, &to.id, &format!("step {t}-{i}"));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(st.db.get_agent(&to.id).unwrap().unwrap().pending_fyis.len(), 400);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn compact_and_avatar_runs_leave_fyis_for_the_next_real_run() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        run(&st, &to, "first").await;
        message(&st, &from, json!({ "agent": to.id, "text": "Merged #12", "kind": "fyi" })).await;
        agents::compact(&st, &to.id, "");
        until("the compact run", 10, || !is_running(&st, &to.id) && prompts(&st, &to).len() == 2).await;
        assert_eq!(prompts(&st, &to)[1], "/compact");
        agents::design_avatar(&st, &to.id, "a robot");
        until("the avatar run", 10, || !is_running(&st, &to.id) && prompts(&st, &to).len() == 3).await;
        assert!(!prompts(&st, &to)[2].contains("Merged #12"));
        let p = run(&st, &to, "next").await;
        assert!(p.starts_with("[FYI from agent Lead") && p.contains("Merged #12"), "{p}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_that_fails_to_start_keeps_its_fyis() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        message(&st, &from, json!({ "agent": to.id, "text": "Merged #12", "kind": "fyi" })).await;
        let bin = st.settings.get().claude_bin;
        let mut s = st.settings.get();
        s.claude_bin = "/nonexistent/claude".into();
        st.settings.save(s).unwrap();
        enqueue(&st, &to.id, Some("go".into()), "user");
        until("the run to fail", 10, || !is_running(&st, &to.id) && st.db.list_messages(&to.id, 50, None).unwrap().iter().any(|m| m.kind == "error")).await;
        assert_eq!(st.db.get_agent(&to.id).unwrap().unwrap().pending_fyis.len(), 1);
        let mut s = st.settings.get();
        s.claude_bin = bin;
        st.settings.save(s).unwrap();
        let p = run(&st, &to, "again").await;
        assert!(p.contains("Merged #12"), "{p}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_that_only_acknowledges_is_refused_and_not_stored() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        for text in ["Thanks!", "ok 👍", "Got it, standing by.", "Thank you so much, Worker!", "👍", "Noted. Waiting for your next task.", "Sounds good, will do", "I’ll wait", "Roger that 🫡", "Thanks, I'm on it. Waiting for your next instructions.", "ok, ready for the next one"] {
            for kind in ["fyi", "needs_action"] {
                let r = message(&st, &from, json!({ "agent": to.id, "text": text, "kind": kind })).await;
                let err = r["content"][0]["text"].as_str().unwrap();
                assert_eq!(r["isError"], json!(true), "{text}");
                assert!(err.starts_with("Not sent: this message only acknowledges") && err.contains("finding, report, verdict or request"), "{err}");
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let a = st.db.get_agent(&to.id).unwrap().unwrap();
        assert!(st.db.list_messages(&to.id, 50, None).unwrap().is_empty() && a.pending_fyis.is_empty() && !is_running(&st, &to.id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_with_substance_goes_through_even_with_thanks() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        let texts = ["Thanks, tests pass at abc123", "Got it. One question: should #12 target main?", "OK, PR #12 needs a fix in mcp.rs", "Approved", "Done: merged #12", "Wait.", "OK, wait.", "Wait here.", "You wait", "Next.", "Great, next task", "Next task"];
        for text in texts {
            let r = message(&st, &from, json!({ "agent": to.id, "text": text, "kind": "fyi" })).await;
            assert!(r["content"][0]["text"].as_str().unwrap().starts_with("Delivered"), "{text}: {r}");
        }
        assert_eq!(st.db.get_agent(&to.id).unwrap().unwrap().pending_fyis.len(), texts.len());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_the_global_setting_off_nothing_is_refused() {
        let (st, to) = fake_claude_state();
        let from = crate::tasks::tests::agent(&st, "Lead");
        rules_off(&st);
        let r = message(&st, &from, json!({ "agent": to.id, "text": "Thanks!" })).await;
        assert!(r["content"][0]["text"].as_str().unwrap().starts_with("Delivered."), "{r}");
        until("the run", 10, || !is_running(&st, &to.id) && !prompts(&st, &to).is_empty()).await;
        assert!(prompts(&st, &to)[0].contains("Thanks!"));
    }
}
