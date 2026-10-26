//! Agent runtime: each run spawns a headless Claude Code process inside the agent's
//! workspace, streams its events into the chat log, and routes permission prompts
//! to the UI through the Loft MCP server.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Notify, oneshot};

use crate::db::{Agent, Message, PermissionRequest, now};
use crate::schedule;
use crate::state::{AppState, Decision, Event};

pub const PERMISSION_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How long claude gets to exit after its final result before it is stopped, and after SIGTERM before SIGKILL.
#[cfg(not(test))]
const EXIT_GRACE: Duration = Duration::from_secs(10);
#[cfg(test)]
const EXIT_GRACE: Duration = Duration::from_millis(500);
/// How long claude gets, once its last background task has finished after the reply, to start a turn about it or exit.
#[cfg(not(test))]
const FOLLOW_UP_GRACE: Duration = Duration::from_secs(120);
#[cfg(test)]
const FOLLOW_UP_GRACE: Duration = Duration::from_secs(2);

pub fn workspace(state: &AppState, agent_id: &str) -> PathBuf {
    state.cfg.storage_dir.join(agent_id)
}

/// Where `@path` in this agent's chat starts: its project's folder when the project names one inside the
/// projects folder (`work/isa`), else its workspace (no project, or a project that's just a tag).
pub fn at_base(state: &AppState, a: &Agent) -> PathBuf {
    let projects = &state.settings.boot().projects_dir;
    let project = projects.join(a.project.trim());
    match (a.project.trim().is_empty(), project.canonicalize(), projects.canonicalize()) {
        (false, Ok(p), Ok(root)) if p.starts_with(&root) && p.is_dir() => p,
        _ => workspace(state, &a.id),
    }
}

pub fn emit_agent(state: &AppState, a: &Agent) {
    let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
    run_fields(state, &a.id, &mut v);
    state.emit(Event::AgentUpdated { agent: v });
}

/// `running`, plus how the current run is doing: `last_output_at` (the last event from claude),
/// and `waiting_on` / `waiting_since` while the agent has replied but waits on background tasks.
/// Clients show "waiting" or "no activity for N min" from these, with a restart.
pub fn run_fields(state: &AppState, agent_id: &str, v: &mut Value) {
    let rt = state.rt.lock().unwrap();
    let info = rt.runs.get(agent_id);
    v["running"] = json!(rt.running.contains_key(agent_id));
    v["last_output_at"] = json!(info.map(|i| &i.last_output_at));
    v["waiting_on"] = json!(info.map_or(&vec![], |i| &i.waiting_on));
    v["waiting_since"] = json!(info.and_then(|i| i.waiting_since.as_ref()));
}

pub fn is_running(state: &AppState, agent_id: &str) -> bool {
    state.rt.lock().unwrap().running.contains_key(agent_id)
}

fn set_status(state: &AppState, agent_id: &str, status: &str, text: &str) {
    if let Ok(Some(a)) = state.db.update_agent(agent_id, |a| {
        a.status = status.into();
        a.status_text = text.chars().take(160).collect();
    }) {
        emit_agent(state, &a);
    }
}

pub fn set_status_text(state: &AppState, agent_id: &str, text: &str) {
    if let Ok(Some(a)) = state.db.update_agent(agent_id, |a| a.status_text = text.chars().take(160).collect()) {
        emit_agent(state, &a);
    }
}

fn set_context_tokens(state: &AppState, agent_id: &str, n: Option<u64>) {
    if n.is_none() || state.db.get_agent(agent_id).ok().flatten().is_some_and(|a| a.context_tokens == n) {
        return;
    }
    if let Ok(Some(a)) = state.db.update_agent(agent_id, |a| a.context_tokens = n) {
        emit_agent(state, &a);
    }
}

/// Context size from the end of a Claude Code session transcript: the last main-thread reply's input,
/// or the compacted size if a compaction came after it.
fn transcript_context(sid: &str) -> Option<u64> {
    let projects = dirs::home_dir()?.join(".claude/projects");
    let file = std::fs::read_dir(projects).ok()?.flatten().map(|d| d.path().join(format!("{sid}.jsonl"))).find(|p| p.exists())?;
    let mut last = None;
    for line in std::io::BufRead::lines(std::io::BufReader::new(std::fs::File::open(file).ok()?)).map_while(Result::ok) {
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        match (ev["type"].as_str(), ev["subtype"].as_str()) {
            (Some("assistant"), _) if !ev["isSidechain"].as_bool().unwrap_or(false) => {
                let u = &ev["message"]["usage"];
                if u.is_object() {
                    last = Some(["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"].iter().filter_map(|k| u[*k].as_u64()).sum());
                }
            }
            (Some("system"), Some("compact_boundary")) => last = ev["compactMetadata"]["postTokens"].as_u64(),
            _ => {}
        }
    }
    last
}

/// Fill in context sizes for agents that haven't replied since this field was added.
pub async fn backfill_context(state: AppState) {
    let Ok(agents) = state.db.list_agents() else { return };
    for a in agents.into_iter().filter(|a| a.context_tokens.is_none()) {
        let Some(sid) = a.session_id.clone() else { continue };
        let n = tokio::task::spawn_blocking(move || transcript_context(&sid)).await.ok().flatten();
        set_context_tokens(&state, &a.id, n);
    }
}

pub(crate) fn add_msg(state: &AppState, agent_id: &str, run_id: Option<&str>, role: &str, kind: &str, content: &str, meta: Value) -> Option<Message> {
    match state.db.add_message(agent_id, run_id, role, kind, content, meta) {
        Ok(m) => {
            state.emit(Event::Message { message: serde_json::to_value(&m).unwrap() });
            Some(m)
        }
        Err(e) => {
            tracing::error!("add_message: {e}");
            None
        }
    }
}

// ---- permissions ------------------------------------------------------

/// Does an allow-rule (Claude Code syntax) cover this tool call?
pub fn rule_matches(rule: &str, tool: &str, input: &Value) -> bool {
    let rule = rule.trim();
    if rule == tool {
        return true;
    }
    // "mcp__server" covers every tool of that server.
    if rule.starts_with("mcp__") && tool.starts_with(&format!("{rule}__")) {
        return true;
    }
    if let Some((name, spec)) = rule.split_once('(') {
        let spec = spec.trim_end_matches(')');
        if name != tool {
            return false;
        }
        if tool == "Bash" {
            let cmd = input["command"].as_str().unwrap_or("").trim();
            let prefix = spec.trim_end_matches('*').trim_end_matches(':').trim();
            if !spec.ends_with('*') {
                return cmd == spec;
            }
            // Refuse to auto-approve compound commands on a prefix rule.
            if cmd.contains("&&") || cmd.contains("||") || cmd.contains(';') || cmd.contains('|') {
                return false;
            }
            return cmd == prefix || cmd.starts_with(&format!("{prefix} "));
        }
        if let Some(domain) = spec.strip_prefix("domain:") {
            let url = input["url"].as_str().unwrap_or("");
            return url.contains(&format!("://{domain}")) || url.contains(&format!(".{domain}"));
        }
        return spec == "*" || spec.is_empty();
    }
    false
}

/// Rule suggested for "always allow" on a blocked tool call.
pub fn suggest_rule(tool: &str, input: &Value) -> String {
    match tool {
        "Bash" => {
            let cmd = input["command"].as_str().unwrap_or("").trim();
            let first = cmd.split_whitespace().next().unwrap_or("");
            if first.is_empty() { "Bash".into() } else { format!("Bash({first} *)") }
        }
        "WebFetch" => {
            let url = input["url"].as_str().unwrap_or("");
            let host = url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("");
            if host.is_empty() { "WebFetch".into() } else { format!("WebFetch(domain:{host})") }
        }
        t => t.to_string(),
    }
}

/// Create a permission request, surface it in the chat, and block until the user decides.
pub async fn ask_permission(
    state: &AppState,
    agent_id: &str,
    source: &str,
    tool_name: &str,
    input: Value,
    reason: &str,
    rule: String,
) -> Decision {
    let req = PermissionRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: agent_id.into(),
        source: source.into(),
        tool_name: tool_name.into(),
        input,
        reason: reason.into(),
        rule,
        status: "pending".into(),
        created_at: now(),
    };
    let (tx, rx) = oneshot::channel();
    state.rt.lock().unwrap().waiters.insert(req.id.clone(), tx);
    if let Err(e) = state.db.save_permission(&req) {
        return Decision { allow: false, message: Some(format!("loft error: {e}")) };
    }
    add_msg(state, agent_id, None, "system", "permission", &req.tool_name, json!({ "request_id": req.id }));
    state.emit(Event::Permission { request: serde_json::to_value(&req).unwrap() });
    let name = state.db.get_agent(agent_id).ok().flatten().map(|a| a.name).unwrap_or_default();
    let why = if reason.is_empty() { String::new() } else { format!(": {}", crate::notify::clip(reason, 200)) };
    crate::notify::send(state, crate::notify::Notice::Permission, format!("{name} needs permission to use {tool_name}{why}"));
    let prev = state.db.get_agent(agent_id).ok().flatten().map(|a| a.status_text).unwrap_or_default();
    set_status(state, agent_id, "waiting", &format!("Waiting for permission: {tool_name}"));

    let decision = match tokio::time::timeout(PERMISSION_TIMEOUT, rx).await {
        Ok(Ok(d)) => d,
        _ => {
            state.rt.lock().unwrap().waiters.remove(&req.id);
            if let Ok(Some(mut r)) = state.db.get_permission(&req.id) {
                r.status = "expired".into();
                let _ = state.db.save_permission(&r);
                state.emit(Event::Permission { request: serde_json::to_value(&r).unwrap() });
            }
            Decision { allow: false, message: Some("The user did not respond to the permission request in time.".into()) }
        }
    };
    if is_running(state, agent_id) {
        set_status(state, agent_id, "running", &prev);
    }
    decision
}

/// Resolve a pending request from the UI. `always` persists the rule into the agent config.
pub fn decide(state: &AppState, id: &str, decision: &str, rule: Option<String>, note: Option<String>) -> Result<PermissionRequest> {
    let mut req = state.db.get_permission(id)?.ok_or_else(|| anyhow::anyhow!("no such request"))?;
    anyhow::ensure!(req.status == "pending", "request already {}", req.status);
    if let Some(r) = rule.filter(|r| !r.trim().is_empty()) {
        req.rule = r.trim().to_string();
    }
    let allow = decision != "deny";
    // Explicit requests are always about config, so any grant persists.
    let persist = decision == "allow_always" || (allow && req.source == "request");
    req.status = match (allow, persist) {
        (false, _) => "denied",
        (true, true) => "allowed_always",
        (true, false) => "allowed_once",
    }
    .into();
    if persist {
        let rule = req.rule.clone();
        if let Some(a) = state.db.update_agent(&req.agent_id, |a| {
            if !a.allowed_tools.contains(&rule) {
                a.allowed_tools.push(rule.clone());
            }
            a.disallowed_tools.retain(|d| d != &rule);
        })? {
            emit_agent(state, &a);
        }
    }
    state.db.save_permission(&req)?;
    state.emit(Event::Permission { request: serde_json::to_value(&req).unwrap() });
    let waiter = state.rt.lock().unwrap().waiters.remove(id);
    if let Some(tx) = waiter {
        let _ = tx.send(Decision { allow, message: note });
    }
    Ok(req)
}

// ---- prompt assembly --------------------------------------------------

pub fn system_prompt(state: &AppState, a: &Agent) -> String {
    let ws = workspace(state, &a.id);
    let memories = state.db.list_memories(&a.id).unwrap_or_default();
    let mem_block = if memories.is_empty() {
        "(no memories yet)".to_string()
    } else {
        // Keep the prompt bounded; older memories stay reachable via recall.
        let skip = memories.len().saturating_sub(80);
        memories[skip..]
            .iter()
            .map(|m| {
                let tags = if m.tags.is_empty() { String::new() } else { format!(" [{}]", m.tags) };
                format!("- #{} ({}){}: {}", m.id, &m.created_at[..10], tags, m.content)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let schedule = if a.interval_minutes > 0 {
        let mut s = format!("You are woken up autonomously every {} minutes.", a.interval_minutes);
        if !a.quiet_hours.is_empty() {
            s += &format!(
                " Wake-ups pause during the user's quiet hours ({}, local time) so background work doesn't use their Claude usage while they're working; the user can still message you then.",
                schedule::describe(&a.quiet_hours)
            );
        }
        s
    } else {
        "You only run when the user messages you.".to_string()
    };
    let subagents = match a.max_subagents {
        Some(0) => "Subagents are turned off for you. Do the work yourself.".to_string(),
        cap => {
            let mut s = "Use the Agent tool only when delegating keeps your own context small: a broad search across many files or repos (`Explore`), or a self-contained job whose details you won't need afterwards. Do quick, single-step work yourself (reading a file, one command, a small edit): every subagent starts a fresh context and costs more than doing it inline.".to_string();
            if !a.subagent_model.is_empty() {
                s += &format!(" Subagents run on `{}`.", a.subagent_model);
            }
            if let Some(n) = cap {
                s += &format!(" At most {n} run at once.");
            }
            s
        }
    };
    format!(
        r#"# Identity
You are **{name}**{title}, an autonomous agent living inside Loft — the user's local developer command center. You talk with the user through the Loft chat UI, which shows your replies, reasoning, and every tool call.

# Personality
{personality}

# Standing instructions
{prompt}

# Schedule
{schedule}

# Workspace
- Your private workspace is `{ws}`. It is your current working directory. Put every file you create there: scripts, notes, scratch data, and clones of repositories.
- The user's projects live in `{projects}`. Never edit them in place. To work on one, clone it into your workspace (`git clone {projects}/<name>` or clone from its remote) and work on the clone.
- Keep a tidy workspace: a `notes/` folder for longer documents is a good idea.
- `@path` in a user message (`@docs/plan.md`, `@../imole/`) points at a file or folder relative to `{at}`, not to your working directory. Read it when it matters to the request.

# Runs
Each message starts a run that ends with your reply. Claude Code background work doesn't outlive it: `run_in_background` commands, `Monitor`, `ScheduleWakeup` and background subagents are stopped as soon as a new message arrives, and until then you look busy and messages wait. So finish the work, or wait for it inside the run (a foreground command with a long enough `timeout`, or a polling loop), before you reply. Anything that must keep running after the reply, like a dev server, goes detached from a normal Bash call: `nohup <command> > <log> 2>&1 &`. To check on it later, say so in the reply; your next message or wake-up is the next turn.

# Showing your work
The user usually reads this chat from another device, where `localhost` doesn't reach this machine.
- Dev servers (web apps, APIs, docs sites): start them detached (see Runs), listening on all interfaces (`vite --host`, `next dev -H 0.0.0.0`, `python3 -m http.server --bind 0.0.0.0`), then check `curl -sI http://{host}:<port>` answers and reply with the link `http://{host}:<port>`. Never share a `localhost` or `127.0.0.1` link. If the framework rejects the host name, allow it in its config (e.g. Next.js `allowedDevOrigins`).
- iOS apps: run them in the Simulator on this Mac. Use the booted device (`xcrun simctl list devices booted`), or boot one with `xcrun simctl boot <udid>` and `open -a Simulator`. Build with `xcodebuild -scheme <scheme> -destination 'platform=iOS Simulator,id=<udid>' -derivedDataPath build`, then `xcrun simctl install booted <path to .app>` and `xcrun simctl launch booted <bundle id>`. The user watches it live at [Simulator](/simulator) in Loft, so include that link in your reply. To check the screen yourself, save `xcrun simctl io booted screenshot <file>.png` and read the image.{axe}

# Long-term memory
You have persistent memory that survives across conversations and session resets. Current memories:
{mem_block}

Use `mcp__loft__remember` for durable facts worth keeping: user preferences, decisions, project facts, and what you did and why. Keep each memory short and self-contained. Use `mcp__loft__forget` to remove stale ones and `mcp__loft__recall` to search.

# Loft tools
- `mcp__loft__set_status`: a short, live, present-tense line shown next to your avatar (e.g. "Reading CI logs for the API"). Update it when you start a distinct phase of work.
- `mcp__loft__request_permission`: ask the user to grant you a tool permission (for example `Bash(npm *)`, `WebFetch`, or `mcp__github`) and explain why. Once granted, it is saved to your config. If a tool call is blocked, the user is prompted automatically, so you only need this for access you plan to use ahead of time.
- Other agents: `mcp__loft__list_agents` lists the other Loft agents, `mcp__loft__read_agent_chat` reads any agent's chat with the user, and `mcp__loft__message_agent` sends an agent a message. Sending returns right away; the other agent answers later with a message of its own, which arrives here as a new turn. Messages from agents are marked as such; a message without that mark is from the user.
- Tasks: Loft keeps a shared task list the user follows from their phone. `mcp__loft__list_tasks` shows it. When you work on a task (a message starting with `[Task <id>]`), report progress with `mcp__loft__update_task`: a short `note` as you go, then `done` with a `summary`, or `failed`/`blocked` with an `error`. A message starting with `[Comment on task <id>` is the user commenting on that task: answer with `mcp__loft__comment_task` so the answer stays with the task. To delegate part of a task, call `mcp__loft__create_task` with `parent_id`, the `agent` to do it and `start: true`; you're told when it finishes.
- New agents: `mcp__loft__list_projects` lists the user's repos by activity, `mcp__loft__list_agent_templates` the ready-made agents, and `mcp__loft__create_agent` creates one. Only create agents the user asked for or agreed to; the user may be asked to approve each one. You can't delete agents, and must never try: only the user deletes agents.
- Pull requests: when you open one with `gh pr create`, Loft shows the user a live PR card (status, checks, a merge button) automatically. If you opened a PR some other way, call `mcp__loft__share_pull_request` with its URL so the card appears. Always include the PR link in your reply.
- `mcp__loft__get_avatar` / `mcp__loft__set_avatar`: your face is a vector sticker. When the user asks you to change your look, design a new one and call `set_avatar` with an `svg` string: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">` drawn only with path, circle, ellipse, rect, polygon, line, g, gradients and clipPath (no text, images, styles or scripts). Loft cuts a white die-cut border around your silhouette and adds the shadow, so don't draw a border or a background. The style is bold and friendly, like a sheet of stickers:
  - Your body is your face: one chunky, flat-colored shape (a blob, star, drop, cloud, or an object that fits your job like a note or a map pin) filling most of the canvas, roughly from 8 to 92 on both axes.
  - Big googly eyes are the focus: white ovals about 22 wide and 25 tall, set close together, each with a dark (#17171c) oval pupil about 9x13, tilted slightly and glancing up or to the side. Add a small mouth as a dark stroke about 4 wide with round caps (a smile, smirk or open mouth). Winks and blush are fine.
  - Flat, saturated colors: one body color plus a slightly darker shade along its lower edge for depth. No black outlines and no busy gradients.
  - Accessories are welcome (a hat, glasses, a tool you hold, a leaf), but keep them chunky and touching the body so the sticker stays one piece, and never let them hide the eyes.

# Subagents
{subagents}{grill}

# Style
Be concise in chat. Lead with the answer or the result. When you finish a task, say what you did and where the output lives."#,
        name = a.name,
        subagents = subagents,
        grill = crate::grill::prompt_section(a),
        title = if a.title.is_empty() { String::new() } else { format!(" ({})", a.title) },
        personality = if a.personality.trim().is_empty() { "Helpful, direct, and curious." } else { a.personality.trim() },
        prompt = if a.prompt.trim().is_empty() { "(none yet — ask the user what they'd like you to own)" } else { a.prompt.trim() },
        ws = ws.display(),
        at = at_base(state, a).display(),
        projects = state.settings.boot().projects_dir.display(),
        axe = crate::simulator::axe()
            .map(|p| format!(" To drive the app, use AXe at `{}`: `tap -x <x> -y <y> --tap-style physical`, `swipe`, `type`, `describe-ui` (coordinates in points; see `--help`), always with `--udid <udid>`.", p.display()))
            .unwrap_or_default(),
        host = crate::tailnet::host().unwrap_or_else(|| "<this Mac's Tailscale or LAN host name>".into()),
    )
}

// ---- running ----------------------------------------------------------

/// Queue a run. User text is recorded immediately; if a run is active it is
/// delivered as soon as that run finishes.
pub fn enqueue(state: &AppState, agent_id: &str, text: Option<String>, trigger: &'static str) {
    enqueue_with(state, agent_id, text, json!({}), trigger);
}

/// `enqueue`, with `meta` stored on the user message (e.g. a client id or task id). Returns that message.
pub fn enqueue_with(state: &AppState, agent_id: &str, text: Option<String>, meta: Value, trigger: &'static str) -> Option<Message> {
    let prompt = text.as_ref().map(|t| {
        let p = crate::uploads::prompt(t, &meta);
        let refs: Vec<String> = meta["task_refs"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        if refs.is_empty() { p } else { format!("{p}\n\n{}", crate::tasks::reference_header(state, &refs)) }
    });
    let msg = text.as_ref().and_then(|t| add_msg(state, agent_id, None, "user", "text", t, meta));
    start(state, agent_id, prompt, trigger);
    msg
}

/// Run the agent with `prompt` (queued behind a running run) without recording a user message.
/// `trigger` "grill" runs quietly (only errors reach the chat: the grill tools post what the user sees);
/// "grill_revise" also runs on the cheaper `GRILL_REVISE_MODEL`.
pub fn wake(state: &AppState, agent_id: &str, prompt: String, trigger: &'static str) {
    start(state, agent_id, Some(prompt), trigger);
}

/// Have a new agent draw its own sticker from `brief`, in a quiet run: only `set_avatar`'s result shows.
pub fn design_avatar(state: &AppState, agent_id: &str, brief: &str) {
    let prompt = format!(
        "[Loft: you were just created] This is not a message from the user. Before any other work, design your sticker avatar.\nBrief: {brief}\n\nRead your name, title, personality and standing instructions as well: the sticker should say who you are and what you do at a glance. Follow the avatar style in your instructions and call `mcp__loft__set_avatar`; if it is rejected, fix the SVG and call it again. Do nothing else in this run.",
        brief = brief.trim()
    );
    start(state, agent_id, Some(prompt), "avatar");
}

/// Model for the background runs that revise a grill's question queue after each answer.
pub const GRILL_REVISE_MODEL: &str = "haiku";

/// `add_msg` for a run's output; quiet (grill) runs keep only errors.
fn run_msg(quiet: bool, state: &AppState, agent_id: &str, rid: Option<&str>, role: &str, kind: &str, content: &str, meta: Value) -> Option<Message> {
    if quiet && kind != "error" {
        return None;
    }
    add_msg(state, agent_id, rid, role, kind, content, meta)
}

/// `/compact`: Claude Code summarises the session itself. The command is not
/// shown as a user message; the `compact` marker it produces is.
pub fn compact(state: &AppState, agent_id: &str, instructions: &str) {
    let cmd = if instructions.is_empty() { "/compact".to_string() } else { format!("/compact {instructions}") };
    start(state, agent_id, Some(cmd), "compact");
}

/// Deliver a message from one agent to another. It shows in the recipient's chat
/// under the sender's sticker, and the recipient is told how to answer.
pub fn message_from_agent(state: &AppState, from: &Agent, to: &Agent, text: &str) {
    message_from_agent_with(state, from, to, text, json!({}));
}

/// `message_from_agent`, with extra `meta` on the stored message (e.g. which task it is about).
pub fn message_from_agent_with(state: &AppState, from: &Agent, to: &Agent, text: &str, mut meta: Value) {
    meta["from_agent"] = json!({ "id": from.id, "name": from.name });
    add_msg(state, &to.id, None, "user", "text", text, meta);
    let prompt = format!(
        "[Message from agent {name} (id {id}), not from the user]\n{text}\n\n[Your replies in this chat are seen by the user, not by {name}. To answer {name}, call `mcp__loft__message_agent` with agent \"{id}\".]",
        name = from.name,
        id = from.id,
    );
    start(state, &to.id, Some(prompt), "agent");
}

fn start(state: &AppState, agent_id: &str, text: Option<String>, trigger: &'static str) {
    let prompt = text.unwrap_or_else(|| {
        format!(
            "[Autonomous check-in — {}] This is a scheduled wake-up, not a message from the user. Review your standing instructions and memory, then do whatever is most useful right now. If there is nothing worth doing, say so in one line.",
            Utc::now().format("%Y-%m-%d %H:%M UTC")
        )
    });
    let kill = {
        let mut rt = state.rt.lock().unwrap();
        if rt.running.contains_key(agent_id) {
            if trigger.starts_with("grill") {
                rt.grill_queued.entry(agent_id.to_string()).or_default().push((prompt, trigger));
            } else {
                rt.queued.entry(agent_id.to_string()).or_default().push(prompt);
            }
            if let Some(w) = rt.waiting.get(agent_id) {
                w.notify_one();
            }
            return;
        }
        let kill = Arc::new(Notify::new());
        rt.running.insert(agent_id.to_string(), kill.clone());
        kill
    };
    let state = state.clone();
    let agent_id = agent_id.to_string();
    tokio::spawn(async move {
        let mut prompt = prompt;
        let mut trigger = trigger;
        loop {
            if let Err(e) = run_once(&state, &agent_id, &prompt, trigger, kill.clone()).await {
                add_msg(&state, &agent_id, None, "system", "error", &e.to_string(), json!({}));
                set_status(&state, &agent_id, "error", &e.to_string());
            }
            let next = {
                let mut rt = state.rt.lock().unwrap();
                match (rt.queued.remove(&agent_id), rt.grill_queued.remove(&agent_id)) {
                    (Some(q), g) if !q.is_empty() => {
                        // User work first; grill follow-ups wait for the run after it.
                        if let Some(g) = g {
                            rt.grill_queued.insert(agent_id.clone(), g);
                        }
                        Some((q.join("\n\n"), "user"))
                    }
                    (_, Some(g)) if !g.is_empty() => {
                        // Merged grill work runs on the cheaper model only if all of it is revision.
                        let t = if g.iter().all(|(_, t)| *t == "grill_revise") { "grill_revise" } else { "grill" };
                        Some((g.into_iter().map(|(p, _)| p).collect::<Vec<_>>().join("\n\n"), t))
                    }
                    _ => {
                        rt.running.remove(&agent_id);
                        None
                    }
                }
            };
            match next {
                Some((p, t)) => {
                    prompt = p;
                    trigger = t;
                }
                None => break,
            }
        }
        if let Ok(Some(a)) = state.db.get_agent(&agent_id) {
            emit_agent(&state, &a);
        }
    });
}

pub fn stop(state: &AppState, agent_id: &str) -> bool {
    let rt = state.rt.lock().unwrap();
    match rt.running.get(agent_id) {
        Some(n) => {
            n.notify_one();
            true
        }
        None => false,
    }
}

fn tool_summary(name: &str, input: &Value) -> String {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    let short = |p: String| p.rsplit('/').next().unwrap_or(&p).to_string();
    match name {
        "Bash" => {
            let d = s("description");
            if d.is_empty() { format!("$ {}", s("command")) } else { d }
        }
        "Read" => format!("Reading {}", short(s("file_path"))),
        "Write" => format!("Writing {}", short(s("file_path"))),
        "Edit" | "MultiEdit" => format!("Editing {}", short(s("file_path"))),
        "Glob" => format!("Finding {}", s("pattern")),
        "Grep" => format!("Searching for \"{}\"", s("pattern")),
        "WebFetch" => format!("Fetching {}", s("url")),
        "WebSearch" => format!("Searching the web: {}", s("query")),
        "Task" | "Agent" => format!("Delegating: {}", s("description")),
        "TodoWrite" => "Updating plan".into(),
        n if n.starts_with("mcp__loft__") => match &n[15..] {
            "remember" => "Saving a memory".into(),
            "recall" => "Searching memory".into(),
            "forget" => "Forgetting a memory".into(),
            "set_avatar" => "Redrawing avatar".into(),
            "get_avatar" => "Looking in the mirror".into(),
            "set_status" => s("text"),
            "request_permission" => format!("Requesting {}", s("permission")),
            "list_agents" => "Looking up the other agents".into(),
            "read_agent_chat" => format!("Reading {}'s chat", s("agent")),
            "message_agent" => format!("Messaging {}", s("agent")),
            "list_projects" => "Looking through the projects".into(),
            "list_agent_templates" => "Reading the agent templates".into(),
            "create_agent" => format!("Creating {}", if s("name").is_empty() { s("template") } else { s("name") }),
            "comment_task" => "Commenting on a task".into(),
            "get_task" => "Reading a task".into(),
            other => other.replace('_', " "),
        },
        n => n.replace("mcp__", "").replace("__", " › "),
    }
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|p| match p["type"].as_str() {
                Some("text") => p["text"].as_str().unwrap_or("").to_string(),
                Some("image") => "[image]".into(),
                _ => p.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… ({} more bytes)", &s[..end], s.len() - end)
}

fn build_command(state: &AppState, a: &Agent, resume: bool) -> Command {
    let mut servers = serde_json::Map::new();
    if let Value::Object(user) = &a.mcp_servers {
        for (k, v) in user {
            servers.insert(k.clone(), v.clone());
        }
    }
    servers.insert(
        "loft".into(),
        json!({ "type": "http", "url": format!("http://127.0.0.1:{}/mcp/{}", state.cfg.port, a.id) }),
    );
    let mcp_config = json!({ "mcpServers": servers }).to_string();

    let mut allowed: Vec<String> = a.allowed_tools.clone();
    allowed.push("mcp__loft".into());

    let mut cmd = Command::new(state.settings.get().claude_bin);
    cmd.current_dir(workspace(state, &a.id))
        .arg("-p")
        .args(["--output-format", "stream-json", "--verbose"])
        .args(["--append-system-prompt", &system_prompt(state, a)])
        .args(["--system-prompt-snapshot", "off"])
        .args(["--mcp-config", &mcp_config])
        .arg("--strict-mcp-config")
        // Isolate agents from the user's own Claude Code settings, hooks and plugins.
        .args(["--setting-sources", ""])
        .args(["--permission-prompt-tool", "mcp__loft__approval_prompt"])
        .args(["--permission-mode", &a.permission_mode])
        .args(["--name", &format!("loft · {}", a.name)])
        .env("MCP_TOOL_TIMEOUT", (PERMISSION_TIMEOUT.as_millis() + 60_000).to_string())
        .env("CLAUDE_CODE_ENTRYPOINT", "loft")
        // Vite rejects requests for hostnames it doesn't know; let dev servers answer on the tailnet name.
        .env("__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS", crate::tailnet::host().unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if !a.model.is_empty() {
        cmd.args(["--model", &a.model]);
    }
    if !a.effort.is_empty() {
        cmd.args(["--effort", &a.effort]);
    }
    cmd.arg("--allowedTools").arg(allowed.join(","));
    if !a.subagent_model.is_empty() {
        // FORCE makes the built-in Explore and Plan subagents follow it too.
        cmd.env("CLAUDE_CODE_SUBAGENT_MODEL", &a.subagent_model).env("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", "1");
    }
    let mut disallowed = a.disallowed_tools.clone();
    match a.max_subagents {
        Some(0) => disallowed.push("Agent".into()),
        Some(n) => {
            cmd.env("CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS", n.to_string());
        }
        None => {}
    }
    if !disallowed.is_empty() {
        cmd.arg("--disallowedTools").arg(disallowed.join(","));
    }
    if resume {
        if let Some(sid) = &a.session_id {
            cmd.args(["--resume", sid]);
        }
    }
    cmd
}

async fn run_once(state: &AppState, agent_id: &str, prompt: &str, trigger: &str, kill: Arc<Notify>) -> Result<()> {
    let mut resume = true;
    loop {
        match run_process(state, agent_id, prompt, trigger, kill.clone(), resume).await? {
            RunOutcome::Done => return Ok(()),
            RunOutcome::StaleSession if trigger == "compact" => anyhow::bail!("Nothing to compact: the session is gone."),
            RunOutcome::StaleSession if resume => {
                // The stored Claude session is gone; start fresh (memory survives).
                if let Ok(Some(a)) = state.db.update_agent(agent_id, |a| {
                    a.session_id = None;
                    a.context_tokens = None;
                }) {
                    emit_agent(state, &a);
                }
                resume = false;
            }
            RunOutcome::StaleSession => anyhow::bail!("claude could not start a session"),
        }
    }
}

/// Stop claude the way it can clean up after itself: SIGTERM ends its background shells too, while
/// SIGKILL would orphan them. SIGKILL only if it is still alive after the grace period.
async fn terminate(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if tokio::time::timeout(EXIT_GRACE, child.wait()).await.is_ok() {
            return;
        }
    }
    let _ = child.kill().await;
}

enum RunOutcome {
    Done,
    StaleSession,
}

async fn run_process(
    state: &AppState,
    agent_id: &str,
    prompt: &str,
    trigger: &str,
    kill: Arc<Notify>,
    resume: bool,
) -> Result<RunOutcome> {
    let mut agent = state.db.get_agent(agent_id)?.ok_or_else(|| anyhow::anyhow!("agent not found"))?;
    let quiet = trigger.starts_with("grill") || trigger == "avatar";
    // Set once the run has called a grill tool: the tools post what the user sees, so any text the agent
    // writes after that (models narrate despite being told not to) would only repeat or spoil it.
    let mut grill_spoke = false;
    if trigger == "grill_revise" {
        agent.model = GRILL_REVISE_MODEL.into();
    }
    std::fs::create_dir_all(workspace(state, agent_id))?;
    // Chats this agent was @-mentioned in; its reply is copied there at the end of the run.
    let mirrors = crate::mentions::take_mirrors(state, agent_id, prompt);
    let run_id = uuid::Uuid::new_v4().to_string();
    let rid = Some(run_id.as_str());

    let started = Utc::now();
    if let Some(a) = state.db.update_agent(agent_id, |a| {
        a.status = "running".into();
        a.status_text = match trigger {
            "schedule" => "Waking up…".into(),
            "compact" => "Compacting context…".into(),
            "grill" | "grill_revise" => "Following the interview…".into(),
            "avatar" => "Drawing its avatar…".into(),
            _ => "Thinking…".into(),
        };
        a.last_run_at = Some(started.to_rfc3339());
    })? {
        emit_agent(state, &a);
    }
    run_msg(quiet, state, agent_id, rid, "system", "run_start", trigger, json!({ "trigger": trigger, "resumed": resume && agent.session_id.is_some() }));

    let image_line = crate::uploads::stdin_line(state, agent_id, prompt);
    let mut cmd = build_command(state, &agent, resume);
    if image_line.is_some() {
        cmd.args(["--input-format", "stream-json"]);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to start `{}`: {e}", state.settings.get().claude_bin))?;
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(image_line.as_deref().unwrap_or(prompt).as_bytes()).await?;
    }
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut stderr = child.stderr.take().unwrap();
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s).await;
        s
    });

    let mut got_result = false;
    let mut got_any = false;
    // Last top-level reply and any error, for the end-of-run iMessage.
    let mut last_text = String::new();
    let mut run_error: Option<String> = None;
    // tool_use id -> (name, input), to recognise PR-opening calls when their results arrive
    let mut tool_calls: std::collections::HashMap<String, (String, Value)> = Default::default();
    let mut killed = false;
    // Claude Code background tasks (shells, monitors, background subagents) still running: id -> description.
    // claude -p stays alive for them and starts another turn when one finishes.
    let mut bg: Vec<(String, String)> = Vec::new();
    // Set at a final result with no background tasks left: claude should exit by then, or it is stopped.
    let mut exit_by: Option<tokio::time::Instant> = None;
    // The agent has given its reply (a result) and hasn't started another turn since.
    let mut replied = false;
    let mut held_end: Option<Value> = None;
    let preempt = Arc::new(Notify::new());
    state.rt.lock().unwrap().runs.insert(agent_id.to_string(), crate::state::RunInfo { last_output_at: now(), ..Default::default() });
    loop {
        let waiting = replied && !bg.is_empty();
        let line = tokio::select! {
            l = lines.next_line() => l?,
            _ = kill.notified() => {
                terminate(&mut child).await;
                killed = true;
                None
            }
            _ = preempt.notified(), if waiting => {
                // New work arrived while the reply was done and only background tasks were left: stop them
                // and tell the agent, in the chat and at the top of its next prompt.
                let names = bg.iter().map(|(_, d)| format!("\"{d}\"")).collect::<Vec<_>>().join(", ");
                run_msg(quiet, state, agent_id, rid, "system", "notice", &format!("Stopped background work to read new messages: {names}."), json!({ "stopped_tasks": bg.iter().map(|(_, d)| d).collect::<Vec<_>>() }));
                let note = format!(
                    "[Loft: your background work ({names}) was stopped when new messages arrived. Background tasks don't outlive a reply here; restart it if you still need it and wait for it within the run.]"
                );
                {
                    let mut rt = state.rt.lock().unwrap();
                    let user_work = rt.queued.get(agent_id).is_some_and(|q| !q.is_empty());
                    match rt.grill_queued.get_mut(agent_id) {
                        // Only grill follow-ups are waiting: tell the agent there, rather than starting a visible run.
                        Some(g) if !g.is_empty() && !user_work => g.insert(0, (note, "grill")),
                        _ => rt.queued.entry(agent_id.to_string()).or_default().insert(0, note),
                    }
                }
                terminate(&mut child).await;
                None
            }
            _ = async { tokio::time::sleep_until(exit_by.unwrap()).await }, if exit_by.is_some() => {
                // Replied with nothing left in the background, yet still alive: don't hold the queue for it.
                terminate(&mut child).await;
                None
            }
        };
        let Some(line) = line else { break };
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        got_any = true;
        if let Some(i) = state.rt.lock().unwrap().runs.get_mut(agent_id) {
            i.last_output_at = now();
        }
        let sub = ev["parent_tool_use_id"].as_str().map(|s| s.to_string());
        match ev["type"].as_str() {
            Some("system") => match ev["subtype"].as_str() {
                Some("init") => {
                    if let Some(sid) = ev["session_id"].as_str() {
                        let sid = sid.to_string();
                        state.db.update_agent(agent_id, |a| a.session_id = Some(sid))?;
                    }
                }
                // Manual /compact and Claude Code's own auto-compaction both land here.
                Some("compact_boundary") => {
                    let m = &ev["compact_metadata"];
                    run_msg(quiet, state, agent_id, rid, "system", "compact", "Context compacted.", json!({
                        "trigger": m["trigger"], "pre_tokens": m["pre_tokens"], "post_tokens": m["post_tokens"],
                    }));
                    set_context_tokens(state, agent_id, m["post_tokens"].as_u64());
                }
                Some("permission_denied") => {
                    run_msg(quiet, state, agent_id, rid, "system", "notice", ev["message"].as_str().unwrap_or("Permission denied"), json!({ "tool": ev["tool_name"] }));
                }
                Some("background_tasks_changed") => {
                    bg = ev["tasks"]
                        .as_array()
                        .map(|ts| ts.iter().map(|t| (t["task_id"].as_str().unwrap_or("").to_string(), t["description"].as_str().unwrap_or("background task").to_string())).collect())
                        .unwrap_or_default();
                    if replied && bg.is_empty() {
                        // The last one finished after the reply; claude starts a turn to report it, or exits.
                        exit_by = Some(tokio::time::Instant::now() + FOLLOW_UP_GRACE);
                    }
                }
                _ => {}
            },
            Some("assistant") => {
                if replied {
                    // A background task finished and the agent picked the work back up.
                    replied = false;
                    exit_by = None;
                    let mut rt = state.rt.lock().unwrap();
                    rt.waiting.remove(agent_id);
                    if let Some(i) = rt.runs.get_mut(agent_id) {
                        i.waiting_on.clear();
                        i.waiting_since = None;
                    }
                }
                // Everything sent as input this turn is the context: fresh, cache-read and cache-written tokens.
                let u = &ev["message"]["usage"];
                if sub.is_none() && u.is_object() {
                    let n = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"].iter().filter_map(|k| u[*k].as_u64()).sum();
                    set_context_tokens(state, agent_id, Some(n));
                }
                let blocks = ev["message"]["content"].as_array().cloned().unwrap_or_default();
                for b in blocks {
                    let meta_sub = json!({ "subagent": sub });
                    match b["type"].as_str() {
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or("");
                            if !t.trim().is_empty() && !(grill_spoke && sub.is_none()) {
                                if sub.is_none() {
                                    last_text = t.to_string();
                                }
                                run_msg(quiet, state, agent_id, rid, "assistant", "text", t, meta_sub);
                                set_status_text(state, agent_id, "Writing…");
                            }
                        }
                        Some("thinking") => {
                            let t = b["thinking"].as_str().unwrap_or("");
                            if !t.trim().is_empty() {
                                run_msg(quiet, state, agent_id, rid, "assistant", "thinking", t, meta_sub);
                                set_status_text(state, agent_id, "Thinking…");
                            }
                        }
                        Some("redacted_thinking") => {
                            run_msg(quiet, state, agent_id, rid, "assistant", "thinking", "(reasoning redacted)", meta_sub);
                        }
                        Some("tool_use") => {
                            let name = b["name"].as_str().unwrap_or("tool");
                            let summary = tool_summary(name, &b["input"]);
                            if let Some(id) = b["id"].as_str() {
                                tool_calls.insert(id.to_string(), (name.to_string(), b["input"].clone()));
                            }
                            // Grill tools post what the user sees themselves; their rows would only clutter the interview.
                            let hide = quiet || name.starts_with("mcp__loft__grill_");
                            if name.starts_with("mcp__loft__grill_") && name != "mcp__loft__grill_state" {
                                grill_spoke = true;
                            }
                            run_msg(hide, state, agent_id, rid, "assistant", "tool_use", &summary, json!({
                                "id": b["id"], "name": name, "input": b["input"], "subagent": sub,
                            }));
                            if name.starts_with("mcp__loft__grill_") {
                                set_status_text(state, agent_id, "Preparing the next question…");
                            } else if name != "mcp__loft__set_status" {
                                set_status_text(state, agent_id, &summary);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                let blocks = ev["message"]["content"].as_array().cloned().unwrap_or_default();
                for b in blocks {
                    if b["type"].as_str() == Some("tool_result") {
                        let full = result_text(&b["content"]);
                        let use_id = b["tool_use_id"].as_str().unwrap_or("").to_string();
                        let ok = !b["is_error"].as_bool().unwrap_or(false);
                        if let Some((name, input)) = tool_calls.get(&use_id) {
                            if ok && crate::prs::creates_pr(name, input) {
                                for (slug, n) in crate::github::extract_pr_links(&full) {
                                    let (state, agent_id, use_id, name) = (state.clone(), agent_id.to_string(), use_id.clone(), agent.name.clone());
                                    tokio::spawn(async move {
                                        if let Ok(pr) = crate::prs::register(&state, &agent_id, &slug, n, Some(use_id)).await {
                                            let title = if pr.title.is_empty() { String::new() } else { format!(": {}", pr.title) };
                                            crate::notify::send(&state, crate::notify::Notice::PrOpened, format!("{name} opened {slug}#{n}{title}\n{}", pr.url));
                                        }
                                    });
                                }
                            }
                        }
                        let text = truncate(&full, 6000);
                        let hide = quiet || tool_calls.get(&use_id).is_some_and(|(n, _)| n.starts_with("mcp__loft__grill_"));
                        run_msg(hide, state, agent_id, rid, "tool", "tool_result", &text, json!({
                            "tool_use_id": b["tool_use_id"], "is_error": b["is_error"].as_bool().unwrap_or(false), "subagent": sub,
                        }));
                    }
                }
            }
            Some("result") => {
                got_result = true;
                // Resuming a session whose background task was stopped first replays an empty result
                // (no turns); the real turn follows, however long the model thinks first.
                replied = ev["num_turns"].as_u64() != Some(0);
                if replied && bg.is_empty() {
                    exit_by = Some(tokio::time::Instant::now() + EXIT_GRACE);
                } else if replied {
                    let names = bg.iter().map(|(_, d)| d.as_str()).collect::<Vec<_>>().join(", ");
                    if let Some(i) = state.rt.lock().unwrap().runs.get_mut(agent_id) {
                        i.waiting_on = bg.iter().map(|(_, d)| d.clone()).collect();
                        i.waiting_since = Some(now());
                    }
                    set_status_text(state, agent_id, &format!("Waiting on: {names}"));
                    let mut rt = state.rt.lock().unwrap();
                    rt.waiting.insert(agent_id.to_string(), preempt.clone());
                    if rt.queued.get(agent_id).is_some_and(|q| !q.is_empty()) || rt.grill_queued.get(agent_id).is_some_and(|q| !q.is_empty()) {
                        // Messages came in during the run; they shouldn't wait for the background work too.
                        preempt.notify_one();
                    }
                }
                let cost = ev["total_cost_usd"].as_f64().unwrap_or(0.0);
                if let Some(sid) = ev["session_id"].as_str() {
                    let sid = sid.to_string();
                    state.db.update_agent(agent_id, |a| {
                        a.session_id = Some(sid);
                        a.total_cost_usd += cost;
                    })?;
                }
                if ev["is_error"].as_bool().unwrap_or(false) {
                    let msg = ev["result"].as_str().or(ev["subtype"].as_str()).unwrap_or("run failed");
                    run_msg(quiet, state, agent_id, rid, "system", "error", msg, json!({}));
                    run_error = Some(msg.to_string());
                }
                let end = json!({
                    "cost_usd": cost,
                    "duration_ms": ev["duration_ms"],
                    "num_turns": ev["num_turns"],
                    "is_error": ev["is_error"],
                    "usage": ev["usage"],
                });
                if replied || run_error.is_some() {
                    run_msg(quiet, state, agent_id, rid, "system", "run_end", "", end);
                    held_end = None;
                } else {
                    // An empty replay: its run end would read as a finished run; post it only if nothing follows.
                    held_end = Some(end);
                }
            }
            _ => {}
        }
    }

    {
        let mut rt = state.rt.lock().unwrap();
        rt.waiting.remove(agent_id);
        rt.runs.remove(agent_id);
    }
    if let Some(end) = held_end {
        run_msg(quiet, state, agent_id, rid, "system", "run_end", "", end);
    }
    // A process the agent left behind can hold the pipes open after claude exits; don't wait on them.
    let status = match tokio::time::timeout(EXIT_GRACE, child.wait()).await {
        Ok(s) => s?,
        Err(_) => {
            terminate(&mut child).await;
            child.wait().await?
        }
    };
    let err = tokio::time::timeout(EXIT_GRACE, stderr_task).await.ok().and_then(|r| r.ok()).unwrap_or_default();
    if !got_result {
        if !killed && resume && agent.session_id.is_some() && !got_any && err.to_lowercase().contains("no conversation found") {
            return Ok(RunOutcome::StaleSession);
        }
        if !killed {
            let tail: String = err.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            let text = format!("claude exited ({status}) without a result.\n{tail}");
            run_msg(quiet, state, agent_id, rid, "system", "error", &text, json!({}));
            run_error = Some(text);
        }
    }
    if killed {
        // Stop ends the current run only: what was sent meanwhile still gets an answer.
        let (n, restarted) = {
            let mut rt = state.rt.lock().unwrap();
            // Loft's own notes to the agent aren't messages anyone sent.
            let n = rt.queued.get(agent_id).map_or(0, |q| q.iter().filter(|p| !p.starts_with("[Loft:")).count());
            (n, rt.restarting.remove(agent_id))
        };
        let what = if restarted { "Restarted by user." } else { "Stopped by user." };
        let text = match n {
            0 => what.to_string(),
            1 => format!("{what} The queued message runs next."),
            n => format!("{what} The {n} queued messages run next."),
        };
        run_msg(quiet, state, agent_id, rid, "system", "notice", &text, json!({ "queued": n }));
        if !got_result {
            run_msg(quiet, state, agent_id, rid, "system", "run_end", "", json!({ "stopped": true }));
        }
    }
    let (st, text) = if !killed && !got_result { ("error", "Last run failed") } else { ("idle", "") };
    set_status(state, agent_id, st, text);
    if !mirrors.is_empty() && !killed {
        crate::mentions::mirror(state, &agent, &mirrors, &last_text, run_error.is_some());
    }
    // Work that came in by iMessage is answered there; the agent may have more queued, so only once idle.
    let reply_by_imessage = {
        let mut rt = state.rt.lock().unwrap();
        let more = rt.queued.get(agent_id).is_some_and(|q| !q.is_empty());
        !more && rt.imessage_reply.remove(agent_id)
    };
    if reply_by_imessage {
        crate::notify::send_reply(state, match &run_error {
            _ if killed => format!("{} was stopped.", agent.name),
            Some(e) => format!("{}'s run failed: {}", agent.name, crate::notify::clip(e, 300)),
            None if last_text.trim().is_empty() => format!("{} finished without a reply.", agent.name),
            None => format!("{}: {}", agent.name, last_text.trim()),
        });
    } else if !killed && trigger != "compact" && !quiet {
        // Stopped runs and compactions stay quiet; scheduled and user-started runs are separate notices.
        let name = &agent.name;
        let notice = if trigger == "schedule" { crate::notify::Notice::ScheduledRun } else { crate::notify::Notice::ManualRun };
        crate::notify::send(state, notice, match run_error {
            Some(e) => format!("{name}'s run failed: {}", crate::notify::clip(&e, 300)),
            None if last_text.trim().is_empty() => format!("{name} finished."),
            None => format!("{name}: {}", crate::notify::clip(&last_text, 300)),
        });
    }
    Ok(RunOutcome::Done)
}

// ---- scheduler --------------------------------------------------------

pub async fn scheduler(state: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    loop {
        tick.tick().await;
        // Hold wake-ups while an update waits for idle; overdue agents run after the restart.
        if crate::updater::pending(&state) {
            continue;
        }
        let Ok(agents) = state.db.list_agents() else { continue };
        let now = Utc::now();
        for a in agents {
            if a.interval_minutes <= 0 {
                if a.next_run_at.is_some() {
                    let _ = state.db.update_agent(&a.id, |a| a.next_run_at = None);
                }
                continue;
            }
            // The next wake-up, pushed past quiet hours.
            let next = schedule::next_wake(&a.quiet_hours, (now + chrono::Duration::minutes(a.interval_minutes)).with_timezone(&Local))
                .with_timezone(&Utc)
                .to_rfc3339();
            let due = a
                .next_run_at
                .as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc));
            match due {
                None => {
                    if let Ok(Some(a)) = state.db.update_agent(&a.id, |a| a.next_run_at = Some(next)) {
                        emit_agent(&state, &a);
                    }
                }
                Some(t) if t <= now => {
                    // Came due inside quiet hours (they changed, or the Mac slept): wait them out.
                    if let Some(until) = schedule::quiet_until(&a.quiet_hours, now.with_timezone(&Local)) {
                        let until = until.with_timezone(&Utc).to_rfc3339();
                        if let Ok(Some(a)) = state.db.update_agent(&a.id, |a| a.next_run_at = Some(until)) {
                            emit_agent(&state, &a);
                        }
                        continue;
                    }
                    if let Ok(Some(a)) = state.db.update_agent(&a.id, |a| a.next_run_at = Some(next)) {
                        emit_agent(&state, &a);
                    }
                    if !is_running(&state, &a.id) {
                        enqueue(&state, &a.id, None, "schedule");
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_says_background_work_ends_with_the_reply() {
        let st = crate::tasks::tests::state();
        let p = system_prompt(&st, &crate::tasks::tests::agent(&st, "Worker"));
        assert!(p.contains("# Runs") && p.contains("doesn't outlive it") && p.contains("nohup <command>"));
        assert!(!p.contains("start them in the background"));
    }

    #[test]
    fn rules() {
        let git = json!({ "command": "git status" });
        assert!(rule_matches("Bash(git *)", "Bash", &git));
        assert!(rule_matches("Bash(git:*)", "Bash", &git));
        assert!(rule_matches("Bash", "Bash", &git));
        assert!(!rule_matches("Bash(npm *)", "Bash", &git));
        assert!(!rule_matches("Bash(git *)", "Bash", &json!({ "command": "git status && rm -rf /" })));
        assert!(!rule_matches("Bash(git *)", "Bash", &json!({ "command": "gitx" })));
        assert!(rule_matches("mcp__github", "mcp__github__list_prs", &json!({})));
        assert!(!rule_matches("mcp__git", "mcp__github__list_prs", &json!({})));
        assert!(rule_matches("WebFetch(domain:docs.rs)", "WebFetch", &json!({ "url": "https://docs.rs/x" })));
        assert_eq!(suggest_rule("Bash", &json!({ "command": "npm test" })), "Bash(npm *)");
    }

    /// Stands in for `claude -p`: logs each prompt to `prompts.log` in the workspace, replies, and then
    /// for "linger" keeps a background task alive, for "hang" stays up with none, like the stuck runs did;
/// "busy" never replies, "slow" replays an empty result first.
    /// It records SIGTERM in `terminated`, as claude would end its background shells then.
    const FAKE_CLAUDE: &str = r#"#!/bin/sh
prompt=$(cat)
printf '%s\n---\n' "$prompt" >> prompts.log
echo '{"type":"system","subtype":"init","session_id":"s1"}'
case "$prompt" in *busy*)
  trap 'kill $!; exit 143' TERM
  sleep 600 &
  wait;;
esac
case "$prompt" in *slow*)
  echo '{"type":"result","subtype":"success","result":"","num_turns":0,"session_id":"s1","total_cost_usd":0}'
  sleep 1;;
esac
case "$prompt" in *linger*)
  echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b1","description":"Run the QA suite"}]}';;
esac
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"started"}]}}'
echo '{"type":"result","subtype":"success","result":"started","num_turns":1,"session_id":"s1","total_cost_usd":0}'
case "$prompt" in *linger*|*hang*)
  trap 'kill $!; echo yes > terminated; exit 143' TERM
  sleep 600 &
  wait;;
esac
"#;

    fn fake_claude_state() -> (AppState, Agent) {
        let mut st = crate::tasks::tests::state();
        let dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        let bin = dir.join("fake-claude");
        std::fs::write(&bin, FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        st.settings = Arc::new(crate::settings::Store::load(&dir).unwrap());
        let mut s = st.settings.get();
        s.claude_bin = bin.to_string_lossy().into();
        s.projects_dir = dir.clone();
        st.settings.save(s).unwrap();
        let a = crate::tasks::tests::agent(&st, "Worker");
        (st, a)
    }

    async fn until(what: &str, secs: u64, f: impl Fn() -> bool) {
        let end = tokio::time::Instant::now() + Duration::from_secs(secs);
        while !f() {
            assert!(tokio::time::Instant::now() < end, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn status_text(st: &AppState, id: &str) -> String {
        st.db.get_agent(id).unwrap().unwrap().status_text
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reply_with_live_background_task_does_not_hold_the_queue() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        enqueue(&st, &a.id, Some("start the suite, linger".into()), "user");
        until("the reply", 10, || status_text(&st, &a.id).starts_with("Waiting on: Run the QA suite")).await;
        // With nothing queued, the background task is left to finish so the agent can still report on it.
        tokio::time::sleep(EXIT_GRACE * 3).await;
        assert!(is_running(&st, &a.id) && !ws.join("terminated").exists());

        enqueue(&st, &a.id, Some("next instructions".into()), "user");
        until("the queued message to run", 10, || !is_running(&st, &a.id)).await;
        assert!(ws.join("terminated").exists(), "claude was stopped with SIGTERM, so its background shells end too");
        let log = std::fs::read_to_string(ws.join("prompts.log")).unwrap();
        let second = log.split("---").nth(1).unwrap();
        assert!(second.contains("(\"Run the QA suite\") was stopped") && second.contains("next instructions"), "{second}");
        let msgs = st.db.list_messages(&a.id, 50, None).unwrap();
        assert!(msgs.iter().any(|m| m.kind == "notice" && m.content.contains("Run the QA suite")));
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().status, "idle");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn messages_queued_during_the_run_preempt_background_work_at_the_reply() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        // "follow up" arrives during the run; at the reply it must not sit behind the background task.
        enqueue(&st, &a.id, Some("linger".into()), "user");
        enqueue(&st, &a.id, Some("follow up".into()), "user");
        until("the queue to drain", 10, || !is_running(&st, &a.id)).await;
        assert!(ws.join("terminated").exists());
        assert!(std::fs::read_to_string(ws.join("prompts.log")).unwrap().contains("follow up"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_result_before_a_slow_turn_does_not_end_the_run() {
        let (st, a) = fake_claude_state();
        enqueue(&st, &a.id, Some("slow".into()), "user");
        until("the run to end", 10, || !is_running(&st, &a.id)).await;
        let msgs = st.db.list_messages(&a.id, 50, None).unwrap();
        assert!(msgs.iter().any(|m| m.kind == "text" && m.content == "started"), "the turn after the empty result was cut off");
        assert_eq!(msgs.iter().filter(|m| m.kind == "run_end").count(), 1, "the empty result doesn't count as a finished run");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_keeps_the_queue() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        enqueue(&st, &a.id, Some("busy".into()), "user");
        until("the run to start", 10, || ws.join("prompts.log").exists()).await;
        enqueue(&st, &a.id, Some("sent while it looked stuck".into()), "user");
        let r = crate::api::stop_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await;
        assert_eq!(r.ok().unwrap().0["ok"], json!(true));
        until("the queue to run", 10, || !is_running(&st, &a.id)).await;
        assert!(std::fs::read_to_string(ws.join("prompts.log")).unwrap().contains("sent while it looked stuck"));
        let msgs = st.db.list_messages(&a.id, 50, None).unwrap();
        assert!(msgs.iter().any(|m| m.kind == "notice" && m.content == "Stopped by user. The queued message runs next."));
    }

    fn fields(st: &AppState, id: &str) -> Value {
        let mut v = json!({});
        run_fields(st, id, &mut v);
        v
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_json_shows_what_the_run_waits_on() {
        let (st, a) = fake_claude_state();
        enqueue(&st, &a.id, Some("linger".into()), "user");
        until("the reply", 10, || fields(&st, &a.id)["waiting_on"] == json!(["Run the QA suite"])).await;
        let v = fields(&st, &a.id);
        assert!(v["running"] == json!(true) && v["waiting_since"].is_string() && v["last_output_at"].is_string(), "{v}");
        let r = crate::api::restart_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await;
        assert!(r.is_ok());
        until("the restarted run to end", 10, || !is_running(&st, &a.id)).await;
        assert_eq!(fields(&st, &a.id), json!({ "running": false, "last_output_at": null, "waiting_on": [], "waiting_since": null }));
        let log = std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap();
        assert!(log.contains("restarted your last run (it was waiting on Run the QA suite)"), "{log}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restart_keeps_the_queue_and_says_so() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        let restart = || crate::api::restart_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone()));
        assert!(restart().await.is_err(), "nothing to restart while idle");
        enqueue(&st, &a.id, Some("busy".into()), "user");
        until("the run to start", 10, || ws.join("prompts.log").exists()).await;
        enqueue(&st, &a.id, Some("still there?".into()), "user");
        assert!(restart().await.is_ok());
        until("the queue to run", 10, || !is_running(&st, &a.id)).await;
        let log = std::fs::read_to_string(ws.join("prompts.log")).unwrap();
        assert!(log.contains("still there?") && !log.contains("restarted your last run"), "queued work is the next prompt as is: {log}");
        let msgs = st.db.list_messages(&a.id, 50, None).unwrap();
        assert!(msgs.iter().any(|m| m.kind == "notice" && m.content == "Restarted by user. The queued message runs next."));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_mention_reaches_the_agent_and_its_reply_comes_back() {
        let (st, orch) = fake_claude_state();
        let dev = crate::tasks::tests::agent(&st, "Web Dev");
        let send = |text: &str| crate::api::send_message(
            axum::extract::State(st.clone()),
            axum::extract::Path(orch.id.clone()),
            axum::Json(serde_json::from_value(json!({ "text": text })).unwrap()),
        );
        // Opens with the mention: only Web Dev runs.
        let r = send("@Web Dev what's in your workspace?").await.ok().unwrap().0;
        assert_eq!(r["mentioned"][0]["name"], json!("Web Dev"));
        until("Web Dev's run", 10, || !is_running(&st, &dev.id) && st.db.list_messages(&orch.id, 50, None).unwrap().iter().any(|m| m.meta["mention_reply"] == json!(true))).await;
        assert!(!workspace(&st, &orch.id).join("prompts.log").exists(), "the chat's own agent wasn't woken");
        let prompt = std::fs::read_to_string(workspace(&st, &dev.id).join("prompts.log")).unwrap();
        assert!(prompt.starts_with(&format!("{}Worker's chat", crate::mentions::TAG)) && prompt.contains("what's in your workspace?"), "{prompt}");
        let back = st.db.list_messages(&orch.id, 50, None).unwrap().into_iter().find(|m| m.meta["mention_reply"] == json!(true)).unwrap();
        assert_eq!((back.role.as_str(), back.content.as_str(), back.meta["from_agent"]["name"].clone()), ("assistant", "started", json!("Web Dev")));
        let mine = st.db.list_messages(&orch.id, 50, None).unwrap().into_iter().find(|m| m.role == "user").unwrap();
        assert_eq!((mine.meta["mentions"][0]["id"].clone(), mine.meta["mentions_only"].clone()), (json!(dev.id), json!(true)));
        let theirs = st.db.list_messages(&dev.id, 50, None).unwrap().into_iter().find(|m| m.role == "user").unwrap();
        assert_eq!(theirs.meta["mentioned_in"]["id"], json!(orch.id));

        // A later run of Web Dev that isn't about a mention isn't copied anywhere.
        enqueue(&st, &dev.id, Some("unrelated".into()), "user");
        until("the unrelated run", 10, || !is_running(&st, &dev.id) && std::fs::read_to_string(workspace(&st, &dev.id).join("prompts.log")).unwrap().contains("unrelated")).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mirrored = st.db.list_messages(&orch.id, 50, None).unwrap().into_iter().filter(|m| m.meta["mention_reply"] == json!(true)).count();
        assert_eq!(mirrored, 1);

        // A mention mid-message reaches both.
        let _ = send("Please plan it, and @Web Dev take a look too").await.ok().unwrap();
        until("both runs", 10, || !is_running(&st, &dev.id) && !is_running(&st, &orch.id) && workspace(&st, &orch.id).join("prompts.log").exists()).await;

        // Another project's agent isn't reachable, even by name, and doesn't shadow this one's.
        let mut far = crate::tasks::tests::agent(&st, "Web Dev");
        far.project = "imole".into();
        st.db.save_agent(&far).unwrap();
        let r = send("@Web Dev again").await.ok().unwrap().0;
        assert_eq!(r["mentioned"], json!([{ "id": dev.id, "name": "Web Dev" }]));
    }

    #[test]
    fn at_paths_start_at_the_projects_folder_when_there_is_one() {
        let st = crate::tasks::tests::state();
        let dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        let projects = dir.join("projects");
        std::fs::create_dir_all(projects.join("work/isa")).unwrap();
        std::fs::write(dir.join("settings.json"), json!({ "projects_dir": projects }).to_string()).unwrap();
        let st = AppState { settings: std::sync::Arc::new(crate::settings::Store::load(&dir).unwrap()), ..st };
        let mut a = crate::tasks::tests::agent(&st, "Dev");
        a.project = "work/isa".into();
        assert_eq!(at_base(&st, &a), projects.join("work/isa").canonicalize().unwrap());
        for tag in ["Marketing", "../..", ""] {
            a.project = tag.into();
            assert_eq!(at_base(&st, &a), workspace(&st, &a.id), "{tag:?} falls back to the workspace");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn process_left_alive_after_the_reply_is_stopped() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        enqueue(&st, &a.id, Some("hang".into()), "user");
        until("the run to end", 10, || !is_running(&st, &a.id)).await;
        assert!(ws.join("terminated").exists());
        let a = st.db.get_agent(&a.id).unwrap().unwrap();
        assert_eq!((a.status.as_str(), a.session_id.as_deref()), ("idle", Some("s1")));
    }
}
