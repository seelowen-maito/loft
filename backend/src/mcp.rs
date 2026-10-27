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

fn tools(me: Option<&Agent>) -> Value {
    let mut all = base_tools();
    all.as_array_mut().unwrap().extend(crate::tasks::mcp_tools());
    all.as_array_mut().unwrap().extend(crate::setup::mcp_tools());
    if me.is_some_and(crate::grill::enabled) {
        all.as_array_mut().unwrap().extend(crate::grill::mcp_tools());
        all.as_array_mut().unwrap().extend(crate::planning::mcp_tools());
        all.as_array_mut().unwrap().extend(crate::greenfield::mcp_tools());
    }
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
            "description": "Send a message to another agent. Returns as soon as it is delivered; it doesn't wait for an answer. The other agent runs (right away, or after its current run) and answers, if it does, with a message of its own that arrives here as a new turn.",
            "inputSchema": { "type": "object", "properties": {
                "agent": { "type": "string", "description": "The recipient's id or name." },
                "text": { "type": "string", "description": "The message." }
            }, "required": ["agent", "text"] }
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
        "tools/list" => json!({ "tools": tools(state.db.get_agent(&agent_id).ok().flatten().as_ref()) }),
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
