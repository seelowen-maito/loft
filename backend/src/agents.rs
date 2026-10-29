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
    let mut v = client_json(a);
    run_fields(state, &a.id, &mut v);
    v["unread_reply"] = json!(a.unread_reply());
    state.emit(Event::AgentUpdated { agent: v });
}

/// The agent as clients see it: without the frozen memory index, which only claude needs.
pub fn client_json(a: &Agent) -> Value {
    let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("memory_index");
    }
    v
}

/// `running`, plus how the current run is doing: `last_output_at` (the last event from claude),
/// `waiting_on` / `waiting_since` while the agent has replied but waits on background tasks, and the
/// `subagents` it has running. Clients show "waiting" or "no activity for N min" from these, with a restart.
pub fn run_fields(state: &AppState, agent_id: &str, v: &mut Value) {
    let rt = state.rt.lock().unwrap();
    let info = rt.runs.get(agent_id);
    v["running"] = json!(rt.running.contains_key(agent_id));
    v["last_output_at"] = json!(info.map(|i| &i.last_output_at));
    v["waiting_on"] = json!(info.map_or(&vec![], |i| &i.waiting_on));
    v["waiting_since"] = json!(info.and_then(|i| i.waiting_since.as_ref()));
    v["subagents"] = json!(info.map_or(&vec![], |i| &i.subagents));
}

/// Pushes the agent's run fields after they changed outside a database update.
fn emit_run(state: &AppState, agent_id: &str) {
    if let Ok(Some(a)) = state.db.get_agent(agent_id) {
        emit_agent(state, &a);
    }
}

/// After the reply, what the run still waits on: every background task in the status line, the shells and
/// monitors in `waiting_on`. Pushes the change.
fn show_waiting(state: &AppState, agent_id: &str, bg: &[(String, String)], shells: &[String]) {
    if let Some(i) = state.rt.lock().unwrap().runs.get_mut(agent_id) {
        i.waiting_on = shells.to_vec();
    }
    if bg.is_empty() {
        emit_run(state, agent_id);
    } else {
        let names = bg.iter().map(|(_, d)| d.as_str()).collect::<Vec<_>>().join(", ");
        set_status_text(state, agent_id, &format!("Waiting on: {names}"));
    }
}

/// A run the user started has ended with a reply or an error: the agent has an Unread reply.
fn mark_replied(state: &AppState, agent_id: &str) {
    if let Ok(Some(a)) = state.db.update_agent(agent_id, |a| a.replied_at = Some(now())) {
        emit_agent(state, &a);
    }
}

/// The user has seen the agent's chat: clear its Unread reply in every client.
pub fn mark_read(state: &AppState, agent_id: &str) -> Result<Option<Agent>> {
    let a = state.db.update_agent(agent_id, |a| a.read_at = Some(now()))?;
    if let Some(a) = &a {
        emit_agent(state, a);
    }
    Ok(a)
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

/// Fill in context sizes for agents that haven't replied since this field was added.
pub async fn backfill_context(state: AppState) {
    let Ok(agents) = state.db.list_agents() else { return };
    for a in agents.into_iter().filter(|a| a.context_tokens.is_none()) {
        let Some(sid) = a.session_id.clone() else { continue };
        let n = tokio::task::spawn_blocking(move || crate::sessions::context_of(&sid)).await.ok().flatten();
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

/// The agent's current memories as they go in the system prompt. Runs use the copy frozen when the
/// Session started (`Agent::memory_index`), so this is only built again for a new Session.
fn memory_block(state: &AppState, agent_id: &str) -> String {
    memory_index(&state.db.list_memories(agent_id).unwrap_or_default())
}

/// About 3,000 tokens of titles, at roughly 4 characters a token.
const MEMORY_INDEX_CHARS: usize = 12_000;

/// Titles only, last saved or updated first, up to `MEMORY_INDEX_CHARS`; the bodies and any older memories are
/// read with `recall`.
fn memory_index(memories: &[crate::db::Memory]) -> String {
    if memories.is_empty() {
        return "(no memories yet)".to_string();
    }
    let mut lines = Vec::new();
    let mut used = 0;
    for m in memories.iter().rev() {
        let line = format!("- #{} {}", m.id, m.title);
        used += line.chars().count() + 1;
        if used > MEMORY_INDEX_CHARS {
            break;
        }
        lines.push(line);
    }
    let older = memories.len() - lines.len();
    if older > 0 {
        lines.push(format!("({older} older memor{}: use `mcp__loft__recall` to reach them.)", if older == 1 { "y" } else { "ies" }));
    }
    lines.join("\n")
}

/// Loft's rules for Agent messages, the same for every agent and every run (so the prompt cache holds).
/// Standing instructions can add to them but not remove them; only the global setting turns them off.
const MESSAGING_RULES: &str = "

# Messaging rules
These rules are Loft's and apply to every message you send another agent with `mcp__loft__message_agent`, whatever your standing instructions say. Standing instructions can add to them but not remove them.
1. Send a message only when it carries something the recipient needs: a finding, report, verdict or request.
2. Never send thanks, acknowledgements, re-confirmations or \"waiting\" notes.
3. Send a verdict only to the agent that acts on it, not to everyone.
4. Mark needs action only when the recipient must act now (review, fix, decide, or tell the user something they must act on); otherwise FYI.
5. Don't reply to an FYI.";

pub fn system_prompt(state: &AppState, a: &Agent) -> String {
    let ws = workspace(state, &a.id);
    let mem_block = a.memory_index.clone().unwrap_or_else(|| memory_block(state, &a.id));
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
- When a repo you work on has `GLOSSARY.md` or `docs/adr/`, read them before you start and use the glossary's terms in code, docs and chat; respect the decisions the ADRs record.
- Assign every GitHub issue you create to the user (`gh issue create --assignee @me`), unless the user explicitly says to assign no one or someone else.
- `@path` in a user message (`@docs/plan.md`, `@../imole/`) points at a file or folder relative to `{at}`, not to your working directory. Read it when it matters to the request.

# Runs
Each message starts a run that ends with your reply. Claude Code background work doesn't outlive it: `run_in_background` commands, `Monitor`, `ScheduleWakeup` and background subagents are stopped as soon as a new message arrives, and until then you look busy and messages wait. So finish the work, or wait for it inside the run (a foreground command with a long enough `timeout`, or a polling loop), before you reply. Anything that must keep running after the reply, like a dev server, goes detached from a normal Bash call: `nohup <command> > <log> 2>&1 &`. To check on it later, say so in the reply; your next message or wake-up is the next turn.

# Showing your work
The user usually reads this chat from another device, where `localhost` doesn't reach this machine.
- Dev servers (web apps, APIs, docs sites): start them detached (see Runs), listening on all interfaces (`vite --host`, `next dev -H 0.0.0.0`, `python3 -m http.server --bind 0.0.0.0`), then check `curl -sI http://{host}:<port>` answers and reply with the link `http://{host}:<port>`. Never share a `localhost` or `127.0.0.1` link. If the framework rejects the host name, allow it in its config (e.g. Next.js `allowedDevOrigins`).
- iOS apps: run them in the Simulator on this Mac. Use the booted device (`xcrun simctl list devices booted`), or boot one with `xcrun simctl boot <udid>` and `open -a Simulator`. Build with `xcodebuild -scheme <scheme> -destination 'platform=iOS Simulator,id=<udid>' -derivedDataPath build`, then `xcrun simctl install booted <path to .app>` and `xcrun simctl launch booted <bundle id>`. The user watches it live at [Simulator](/simulator) in Loft, so include that link in your reply. To check the screen yourself, save `xcrun simctl io booted screenshot <file>.png` and read the image.{axe}

# Long-term memory
You have persistent memory that survives across conversations and session resets. The titles of your memories as of this session's start, newest first (ones you save, change or forget now show here from your next session):
{mem_block}

Read a memory's full body with `mcp__loft__recall` when its title looks relevant; it searches titles and bodies. Use `mcp__loft__remember` for durable facts worth keeping: user preferences, decisions, project facts, and what you did and why. Give each memory a one-line title and a short, self-contained body. When a fact changes or a memory already covers it, rewrite that memory with `mcp__loft__update` instead of saving a near-copy. Use `mcp__loft__forget` to remove stale ones.

# Loft tools
- `mcp__loft__set_status`: a short, live, present-tense line shown next to your avatar (e.g. "Reading CI logs for the API"). Update it when you start a distinct phase of work.
- `mcp__loft__request_permission`: ask the user to grant you a tool permission (for example `Bash(npm *)`, `WebFetch`, or `mcp__github`) and explain why. Once granted, it is saved to your config. If a tool call is blocked, the user is prompted automatically, so you only need this for access you plan to use ahead of time.
- Other agents: `mcp__loft__list_agents` lists the other Loft agents, `mcp__loft__read_agent_chat` reads any agent's chat with the user, and `mcp__loft__message_agent` sends an agent a message. Sending returns right away; the other agent answers later with a message of its own, which arrives here as a new turn. Messages from agents are marked as such; a message without that mark is from the user.
- Loft issues: Loft keeps a shared list of Loft issues (the work items, once called tasks) that the user follows from their phone. `mcp__loft__list_issues` shows it. When you work on one (a message starting with `[Task <id>]`), report progress with `mcp__loft__update_issue`: a short `note` as you go, then `done` with a `summary`, or `failed`/`blocked` with an `error`. A message starting with `[Comment on task <id>` is the user commenting on it: answer with `mcp__loft__comment_issue` so the answer stays with the issue. To delegate part of one, call `mcp__loft__create_issue` with `parent_id`, the `agent` to do it and `start: true`; you're told when it finishes. The old `*_task` tool names still work.
- New agents: `mcp__loft__list_projects` lists the user's repos by activity, `mcp__loft__list_agent_templates` the ready-made agents, and `mcp__loft__create_agent` creates one. Only create agents the user asked for or agreed to; the user may be asked to approve each one. You can't delete agents, and must never try: only the user deletes agents.
- Pull requests: when you open one with `gh pr create`, Loft shows the user a live PR card (status, checks, a merge button) automatically. If you opened a PR some other way, call `mcp__loft__share_pull_request` with its URL so the card appears. Always include the PR link in your reply.
- `mcp__loft__get_avatar` / `mcp__loft__set_avatar`: your face is a vector sticker. When the user asks you to change your look, design a new one and call `set_avatar` with an `svg` string: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">` drawn only with path, circle, ellipse, rect, polygon, line, g, gradients and clipPath (no text, images, styles or scripts). Loft cuts a white die-cut border around your silhouette and adds the shadow, so don't draw a border or a background. The style is bold and friendly, like a sheet of stickers:
  - Your body is your face: one chunky, flat-colored shape (a blob, star, drop, cloud, or an object that fits your job like a note or a map pin) filling most of the canvas, roughly from 8 to 92 on both axes.
  - Big googly eyes are the focus: white ovals about 22 wide and 25 tall, set close together, each with a dark (#17171c) oval pupil about 9x13, tilted slightly and glancing up or to the side. Add a small mouth as a dark stroke about 4 wide with round caps (a smile, smirk or open mouth). Winks and blush are fine.
  - Flat, saturated colors: one body color plus a slightly darker shade along its lower edge for depth. No black outlines and no busy gradients.
  - Accessories are welcome (a hat, glasses, a tool you hold, a leaf), but keep them chunky and touching the body so the sticker stays one piece, and never let them hide the eyes.{messaging}

# Subagents
{subagents}{grill}

# Style
Be concise in chat. Lead with the answer or the result. When you finish a task, say what you did and where the output lives."#,
        name = a.name,
        subagents = subagents,
        grill = crate::grill::prompt_section(),
        messaging = if state.settings.get().messaging_rules { MESSAGING_RULES } else { "" },
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
    // The user's own message, or a schedule (or "Run now"): its reply is meant for them.
    let for_user = matches!(trigger, "schedule" | "manual") || (text.is_some() && meta["from_agent"].is_null());
    let msg = text.as_ref().and_then(|t| add_msg(state, agent_id, None, "user", "text", t, meta));
    start(state, agent_id, prompt, trigger, for_user);
    msg
}

/// Run the agent with `prompt` (queued behind a running run) without recording a user message.
/// `trigger` "grill" runs quietly (only errors reach the chat: the grill tools post what the user sees).
pub fn wake(state: &AppState, agent_id: &str, prompt: String, trigger: &'static str) {
    start(state, agent_id, Some(prompt), trigger, false);
}

/// `wake` for the user's own words recorded elsewhere (a task comment sent to the agent): its reply is an Unread reply.
pub fn wake_for_user(state: &AppState, agent_id: &str, prompt: String) {
    start(state, agent_id, Some(prompt), "user", true);
}

/// Have a new agent draw its own sticker from `brief`, in a quiet run: only `set_avatar`'s result shows.
pub fn design_avatar(state: &AppState, agent_id: &str, brief: &str) {
    let prompt = format!(
        "[Loft: you were just created] This is not a message from the user. Before any other work, design your sticker avatar.\nBrief: {brief}\n\nRead your name, title, personality and standing instructions as well: the sticker should say who you are and what you do at a glance. Follow the avatar style in your instructions and call `mcp__loft__set_avatar`; if it is rejected, fix the SVG and call it again. Do nothing else in this run.",
        brief = brief.trim()
    );
    start(state, agent_id, Some(prompt), "avatar", false);
}

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
    start(state, agent_id, Some(cmd), "compact", false);
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
    start(state, &to.id, Some(prompt), "agent", false);
}

/// Deliver an FYI from one agent to another: it shows in the recipient's chat at once, marked as FYI,
/// but starts no run. The recipient reads it at the start of its next run, whatever starts that.
pub fn fyi_from_agent(state: &AppState, from: &Agent, to: &Agent, text: &str) {
    add_msg(state, &to.id, None, "user", "text", text, json!({ "from_agent": { "id": from.id, "name": from.name }, "fyi": true }));
    let prompt = format!(
        "[FYI from agent {name} (id {id}), not from the user. It needs no action and no reply.]\n{text}",
        name = from.name,
        id = from.id,
    );
    let _ = state.db.add_pending_fyi(&to.id, &prompt);
}

/// The most FYIs put in front of one prompt; older ones are only counted.
const MAX_FYIS_PER_PROMPT: usize = 20;

/// Put the pending FYIs, oldest first, in front of the prompt: the newest MAX_FYIS_PER_PROMPT,
/// after a line saying how many older ones were left out.
fn with_fyis(fyis: &[(i64, String)], prompt: &str) -> String {
    if fyis.is_empty() {
        return prompt.to_string();
    }
    let skip = fyis.len().saturating_sub(MAX_FYIS_PER_PROMPT);
    let mut parts: Vec<String> = vec![];
    if skip > 0 {
        parts.push(format!("[{skip} older FYIs from agents were left out; the newest {MAX_FYIS_PER_PROMPT} follow.]"));
    }
    parts.extend(fyis[skip..].iter().map(|(_, p)| p.clone()));
    parts.push(prompt.to_string());
    parts.join("\n\n")
}

/// `for_user`: the run answers the user (their message or a schedule), so its reply leaves an Unread reply.
fn start(state: &AppState, agent_id: &str, text: Option<String>, trigger: &'static str, for_user: bool) {
    let prompt = text.unwrap_or_else(|| {
        format!(
            "[Autonomous check-in — {}] This is a scheduled wake-up, not a message from the user. Review your standing instructions and memory, then do whatever is most useful right now. If there is nothing worth doing, say so in one line.",
            Utc::now().format("%Y-%m-%d %H:%M UTC")
        )
    });
    let kill = {
        let mut rt = state.rt.lock().unwrap();
        if rt.running.contains_key(agent_id) {
            if trigger == "grill" {
                rt.grill_queued.entry(agent_id.to_string()).or_default().push(prompt);
            } else {
                rt.queued.entry(agent_id.to_string()).or_default().push(prompt);
                if for_user {
                    rt.queued_for_user.insert(agent_id.to_string());
                }
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
        let mut for_user = for_user;
        loop {
            if trigger == "grill" {
                prompt = crate::grill::with_brief(&state, &agent_id, prompt);
            }
            // `/compact` must stay the whole prompt, and the avatar run does nothing else: FYIs wait for a real run.
            let fyis = if trigger != "compact" && trigger != "avatar" { state.db.take_pending_fyis(&agent_id).unwrap_or_default() } else { vec![] };
            let full = with_fyis(&fyis, &prompt);
            let mut delivered = false;
            if let Err(e) = run_once(&state, &agent_id, &full, trigger, for_user, kill.clone(), &mut delivered).await {
                // Claude never got the prompt, so the FYIs never reached it: keep them for the next run.
                if !delivered {
                    let _ = state.db.restore_pending_fyis(&agent_id, &fyis);
                }
                add_msg(&state, &agent_id, None, "system", "error", &e.to_string(), json!({}));
                set_status(&state, &agent_id, "error", &e.to_string());
                if for_user {
                    mark_replied(&state, &agent_id);
                }
            }
            let next = {
                let mut rt = state.rt.lock().unwrap();
                match (rt.queued.remove(&agent_id), rt.grill_queued.remove(&agent_id)) {
                    (Some(q), g) if !q.is_empty() => {
                        // User work first; grill follow-ups wait for the run after it.
                        if let Some(g) = g {
                            rt.grill_queued.insert(agent_id.clone(), g);
                        }
                        let u = rt.queued_for_user.remove(&agent_id);
                        Some((q.join("\n\n"), "user", u))
                    }
                    (_, Some(g)) if !g.is_empty() => Some((g.join("\n\n"), "grill", false)),
                    _ => {
                        rt.running.remove(&agent_id);
                        None
                    }
                }
            };
            match next {
                Some((p, t, u)) => {
                    prompt = p;
                    trigger = t;
                    for_user = u;
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
            "update" => "Updating a memory".into(),
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
            "set_schedule" => "Changing its schedule".into(),
            "comment_issue" | "comment_task" => "Commenting on a Loft issue".into(),
            "get_issue" | "get_task" => "Reading a Loft issue".into(),
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

/// Points the agent at Claude session `sid`. A different session than its last one brings the running
/// total last recorded for it (0 for a new one), so the next run's cost is only what that run adds.
fn set_session(state: &AppState, agent_id: &str, sid: &str) -> Result<()> {
    let same = state.db.get_agent(agent_id)?.is_some_and(|a| a.session_id.as_deref() == Some(sid));
    let seed = if same { None } else { Some(state.db.session_total(agent_id, sid)?) };
    state.db.update_agent(agent_id, |a| {
        a.session_id = Some(sid.to_string());
        if let Some(t) = seed {
            a.session_cost_usd = t;
        }
    })?;
    Ok(())
}

/// `delivered` is set once a claude process has the prompt and acts on it (a stale session doesn't).
async fn run_once(state: &AppState, agent_id: &str, prompt: &str, trigger: &str, for_user: bool, kill: Arc<Notify>, delivered: &mut bool) -> Result<()> {
    let mut resume = true;
    loop {
        match run_process(state, agent_id, prompt, trigger, for_user, kill.clone(), resume, delivered).await? {
            RunOutcome::Done => return Ok(()),
            RunOutcome::StaleSession if trigger == "compact" => anyhow::bail!("Nothing to compact: the session is gone."),
            RunOutcome::StaleSession if resume => {
                *delivered = false;
                // The stored Claude session is gone; start fresh (memory survives).
                if let Ok(Some(a)) = state.db.update_agent(agent_id, |a| {
                    a.session_id = None;
                    a.context_tokens = None;
                    a.grill_briefed = false;
                    a.memory_index = None;
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
    for_user: bool,
    kill: Arc<Notify>,
    resume: bool,
    delivered: &mut bool,
) -> Result<RunOutcome> {
    let mut agent = state.db.get_agent(agent_id)?.ok_or_else(|| anyhow::anyhow!("agent not found"))?;
    if !resume || agent.session_id.is_none() || agent.memory_index.is_none() {
        // A new Session (or the first run since Clear or Compact): freeze its memories for the runs to come.
        let block = memory_block(state, agent_id);
        agent = state.db.update_agent(agent_id, |a| a.memory_index = Some(block))?.unwrap_or(agent);
    }
    let quiet = trigger == "grill" || trigger == "avatar";
    // Set once the run has called a grill tool: the tools post what the user sees, so any text the agent
    // writes after that (models narrate despite being told not to) would only repeat or spoil it.
    let mut grill_spoke = false;
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
            "grill" => "Following the interview…".into(),
            "avatar" => "Drawing its avatar…".into(),
            _ => "Thinking…".into(),
        };
        a.last_run_at = Some(started.to_rfc3339());
    })? {
        emit_agent(state, &a);
    }
    run_msg(quiet, state, agent_id, rid, "system", "run_start", trigger, json!({ "trigger": trigger, "resumed": resume && agent.session_id.is_some(), "for_user": for_user }));

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
    *delivered = true;
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut stderr = child.stderr.take().unwrap();
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s).await;
        s
    });

    let mut got_result = false;
    let mut got_any = false;
    // Last top-level reply and any error, for mirroring the answer to chats that @-mentioned the agent.
    let mut last_text = String::new();
    let mut run_error: Option<String> = None;
    // tool_use id -> (name, input), to recognise PR-opening calls when their results arrive
    let mut tool_calls: std::collections::HashMap<String, (String, Value)> = Default::default();
    let mut killed = false;
    // Claude Code background tasks (shells, monitors, background subagents) still running: id -> description.
    // claude -p stays alive for them and starts another turn when one finishes.
    let mut bg: Vec<(String, String)> = Vec::new();
    // The shells and monitors among them, which the waiting bar names; the subagent bar shows the subagents.
    let mut shells: Vec<String> = Vec::new();
    // Set at a final result with no background tasks left: claude should exit by then, or it is stopped.
    let mut exit_by: Option<tokio::time::Instant> = None;
    // The agent has given its reply (a result) and hasn't started another turn since.
    let mut replied = false;
    let mut held_end: Option<Value> = None;
    let preempt = Arc::new(Notify::new());
    state.rt.lock().unwrap().runs.insert(agent_id.to_string(), crate::state::RunInfo { last_output_at: now(), for_user, ..Default::default() });
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
                        Some(g) if !g.is_empty() && !user_work => g.insert(0, note),
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
                        set_session(state, agent_id, sid)?;
                    }
                }
                // Manual /compact and Claude Code's own auto-compaction both land here.
                Some("compact_boundary") => {
                    let m = &ev["compact_metadata"];
                    run_msg(quiet, state, agent_id, rid, "system", "compact", "Context compacted.", json!({
                        "trigger": m["trigger"], "pre_tokens": m["pre_tokens"], "post_tokens": m["post_tokens"],
                    }));
                    set_context_tokens(state, agent_id, m["post_tokens"].as_u64());
                    // The summary may have dropped the grill instructions: the next grill run resends them.
                    // The cache is rebuilt after a compaction anyway, so the next run takes fresh memories.
                    state.db.update_agent(agent_id, |a| {
                        a.grill_briefed = false;
                        a.memory_index = None;
                    })?;
                }
                Some("permission_denied") => {
                    run_msg(quiet, state, agent_id, rid, "system", "notice", ev["message"].as_str().unwrap_or("Permission denied"), json!({ "tool": ev["tool_name"] }));
                }
                // Ties a subagent's Agent call to its task, and says whether it runs in the background.
                Some("task_started") | Some("task_updated") => {
                    let use_id = ev["tool_use_id"].as_str();
                    let task_id = ev["task_id"].as_str();
                    let background = ev["is_backgrounded"].as_bool().or(ev["patch"]["is_backgrounded"].as_bool()) == Some(true);
                    if let Some(i) = state.rt.lock().unwrap().runs.get_mut(agent_id) {
                        for s in i.subagents.iter_mut().filter(|s| use_id == Some(s.id.as_str()) || (task_id.is_some() && s.task_id.as_deref() == task_id)) {
                            s.task_id = task_id.map(String::from).or(s.task_id.take());
                            s.background |= background;
                        }
                    }
                }
                Some("background_tasks_changed") => {
                    let tasks = ev["tasks"].as_array().cloned().unwrap_or_default();
                    let desc = |t: &Value| t["description"].as_str().unwrap_or("background task").to_string();
                    bg = tasks.iter().map(|t| (t["task_id"].as_str().unwrap_or("").to_string(), desc(t))).collect();
                    shells = tasks.iter().filter(|t| t["task_type"] != "local_agent").map(desc).collect();
                    // A launched background subagent runs until the list drops it. Without its task id, until
                    // no subagent the agent started itself is left.
                    let any_own = tasks.iter().any(|t| t["task_type"] == "local_agent" && t["parent_task_id"].is_null());
                    let ended = state.rt.lock().unwrap().runs.get_mut(agent_id).is_some_and(|i| {
                        let n = i.subagents.len();
                        i.subagents.retain(|s| {
                            !s.launched
                                || match &s.task_id {
                                    Some(t) => bg.iter().any(|(id, _)| id == t),
                                    None => any_own,
                                }
                        });
                        i.subagents.len() < n
                    });
                    if ended && !replied {
                        emit_run(state, agent_id);
                    }
                    if replied {
                        show_waiting(state, agent_id, &bg, &shells);
                    }
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
                                // Only the agent's own subagents count; one a subagent starts runs inside it.
                                if sub.is_none() && matches!(name, "Agent" | "Task") {
                                    if let Some(i) = state.rt.lock().unwrap().runs.get_mut(agent_id) {
                                        let s = |k: &str| b["input"][k].as_str().unwrap_or("").to_string();
                                        let background = b["input"]["run_in_background"].as_bool() == Some(true);
                                        i.subagents.push(crate::state::Subagent { id: id.to_string(), description: s("description"), kind: s("subagent_type"), started_at: now(), task_id: None, background, launched: false });
                                    }
                                    emit_run(state, agent_id);
                                }
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
                        // A background subagent's result only says it launched; it ends when the task list drops it.
                        let ended = state.rt.lock().unwrap().runs.get_mut(agent_id).is_some_and(|i| {
                            let n = i.subagents.len();
                            i.subagents.retain_mut(|s| {
                                if s.id == use_id && s.background {
                                    s.launched = true;
                                }
                                s.id != use_id || s.background
                            });
                            i.subagents.len() < n
                        });
                        if ended {
                            emit_run(state, agent_id);
                        }
                        if let Some((name, input)) = tool_calls.get(&use_id) {
                            if ok && crate::prs::creates_pr(name, input) {
                                for (slug, n) in crate::github::extract_pr_links(&full) {
                                    let (state, agent_id, use_id) = (state.clone(), agent_id.to_string(), use_id.clone());
                                    tokio::spawn(async move {
                                        let _ = crate::prs::register(&state, &agent_id, &slug, n, Some(use_id)).await;
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
                    if let Some(i) = state.rt.lock().unwrap().runs.get_mut(agent_id) {
                        i.waiting_since = Some(now());
                    }
                    show_waiting(state, agent_id, &bg, &shells);
                    let mut rt = state.rt.lock().unwrap();
                    rt.waiting.insert(agent_id.to_string(), preempt.clone());
                    if rt.queued.get(agent_id).is_some_and(|q| !q.is_empty()) || rt.grill_queued.get(agent_id).is_some_and(|q| !q.is_empty()) {
                        // Messages came in during the run; they shouldn't wait for the background work too.
                        preempt.notify_one();
                    }
                }
                // On --resume Claude Code reports the session's running total; the run cost what it added.
                let total = ev["total_cost_usd"].as_f64().unwrap_or(0.0);
                let mut cost = total;
                if let Some(sid) = ev["session_id"].as_str() {
                    set_session(state, agent_id, sid)?;
                    state.db.update_agent(agent_id, |a| {
                        cost = crate::db::run_cost(total, a.session_cost_usd);
                        a.session_cost_usd = total;
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
                    "session_cost_usd": total,
                    "session_id": ev["session_id"],
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

    let had_subagents = {
        let mut rt = state.rt.lock().unwrap();
        rt.waiting.remove(agent_id);
        rt.runs.remove(agent_id).is_some_and(|i| !i.subagents.is_empty())
    };
    if had_subagents {
        emit_run(state, agent_id);
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
    if for_user && !quiet && (!last_text.is_empty() || run_error.is_some()) {
        mark_replied(state, agent_id);
    }
    if !mirrors.is_empty() && !killed {
        crate::mentions::mirror(state, &agent, &mirrors, &last_text, run_error.is_some());
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
pub(crate) mod tests {
    use super::*;

    #[test]
    fn prompt_says_background_work_ends_with_the_reply() {
        let st = crate::tasks::tests::state();
        let p = system_prompt(&st, &crate::tasks::tests::agent(&st, "Worker"));
        assert!(p.contains("# Runs") && p.contains("doesn't outlive it") && p.contains("nohup <command>"));
        assert!(!p.contains("start them in the background"));
    }

    #[test]
    fn every_agent_is_told_to_use_the_project_s_glossary_and_adrs() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let p = system_prompt(&st, &a);
        assert!(p.contains("`GLOSSARY.md`") && p.contains("`docs/adr/`"), "{p}");
    }

    #[test]
    fn every_agent_assigns_the_issues_it_creates_to_the_user() {
        let st = crate::tasks::tests::state();
        let p = system_prompt(&st, &crate::tasks::tests::agent(&st, "Worker"));
        assert!(p.contains("Assign every GitHub issue you create to the user") && p.contains("unless the user explicitly says to assign no one or someone else"), "{p}");
    }

    /// The messaging-rules section of a system prompt, if it has one.
    fn messaging_section(p: &str) -> Option<&str> {
        let start = p.find("# Messaging rules\n")?;
        let end = start + p[start..].find("\n\n# ").unwrap();
        Some(&p[start..end])
    }

    #[test]
    fn every_agent_gets_loft_s_messaging_rules_whatever_its_standing_instructions() {
        let st = crate::tasks::tests::state();
        st.rt.lock().unwrap().running.insert(crate::setup::LOFT_ID.into(), Arc::new(Notify::new()));
        crate::setup::ensure_loft(&st);
        let guide = st.db.get_agent(crate::setup::LOFT_ID).unwrap().unwrap();
        let mut empty = crate::tasks::tests::agent(&st, "Empty");
        empty.prompt = String::new();
        let mut chatty = crate::tasks::tests::agent(&st, "Chatty");
        chatty.prompt = "Thank every agent that messages you, and send your verdicts to the whole team.".into();
        for a in [&guide, &empty, &chatty] {
            let p = system_prompt(&st, a);
            let section = messaging_section(&p).unwrap_or_else(|| panic!("{} has no messaging rules: {p}", a.name));
            assert_eq!(section, MESSAGING_RULES.trim_start());
            for rule in [
                "1. Send a message only when it carries something the recipient needs: a finding, report, verdict or request.",
                "2. Never send thanks, acknowledgements, re-confirmations or \"waiting\" notes.",
                "3. Send a verdict only to the agent that acts on it, not to everyone.",
                "4. Mark needs action only when the recipient must act now (review, fix, decide, or tell the user something they must act on); otherwise FYI.",
                "5. Don't reply to an FYI.",
            ] {
                assert!(section.contains(rule), "missing {rule}");
            }
            assert!(section.contains("whatever your standing instructions say"));
            // Next to the Loft tools.
            assert!(p.find("# Loft tools").unwrap() < p.find("# Messaging rules").unwrap());
        }
    }

    #[test]
    fn the_messaging_rules_are_the_same_on_every_run() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let first = system_prompt(&st, &a);
        // What changes between runs (memories, the agent's own settings) leaves the section alone.
        st.db.add_memory(&a.id, "First run", "Something learned in the first run", "").unwrap();
        let mut later = a.clone();
        later.prompt = "New standing instructions".into();
        later.interval_minutes = 30;
        let second = system_prompt(&st, &later);
        assert_ne!(first, second);
        assert_eq!(messaging_section(&first), messaging_section(&second));
        assert!(messaging_section(&first).is_some());
    }

    #[test]
    fn the_global_setting_turns_the_messaging_rules_off_for_every_agent() {
        let st = crate::tasks::tests::state();
        assert!(st.settings.get().messaging_rules, "on by default");
        let a = crate::tasks::tests::agent(&st, "Worker");
        let mut s = st.settings.get();
        s.messaging_rules = false;
        st.settings.save(s).unwrap();
        let p = system_prompt(&st, &a);
        assert_eq!(messaging_section(&p), None, "{p}");
        assert!(!p.contains("Messaging rules"));
        assert!(p.contains("# Loft tools") && p.contains("# Subagents"));
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

    /// Stands in for `claude -p`: logs each prompt to `prompts.log` and system prompt to `system.log` in the
    /// workspace (a compaction for `/compact`), replies, and then
    /// for "linger" keeps a background task alive, for "hang" stays up with none, like the stuck runs did;
    /// "busy" never replies, "slow" replays an empty result first, "fail" ends in an error, "ask" replies with a question,
    /// "delegate" starts an Explore subagent, which starts one of its own, and waits for a `go` file before the
/// subagent returns and for `go2` before the reply; "orphan" replies with a subagent that never returned.
/// "bg-agent" launches two background subagents next to a shell and a monitor; the task list drops one subagent at
/// `go` and the other with the monitor at `go2`, and the reply waits for `go3` (with "linger" they stay listed instead, "no-task-id" leaves out their task_started,
/// "early" also replies right after the launch).
/// "crash" exits without a result and "garbled" writes output that isn't text, which fails the run. "session-gone"
    /// with --resume answers like a session claude no longer has, and leaves itself unrunnable so the fresh retry fails to start.
    /// It records SIGTERM in `terminated`, as claude would end its background shells then.
    const FAKE_CLAUDE: &str = r#"#!/bin/sh
prompt=$(cat)
printf '%s\n---\n' "$prompt" >> prompts.log
case "$prompt" in *session-gone*) case " $* " in *" --resume "*)
  chmod -x "$0"
  echo 'No conversation found with session ID: s1' >&2
  exit 1;;
esac;;
esac
while [ $# -gt 0 ]; do
  [ "$1" = --append-system-prompt ] && printf '%s\n---\n' "$2" >> system.log
  [ "$1" = --resume ] && echo "$2" >> resumed.log
  shift
done
cost=$(printf '%s' "$prompt" | sed -n 's/.*cost=\([0-9.]*\).*/\1/p')
sid=$(printf '%s' "$prompt" | sed -n 's/.*sid=\([a-z0-9-]*\).*/\1/p')
: "${cost:=0}" "${sid:=s1}"
echo '{"type":"system","subtype":"init","session_id":"'"$sid"'"}'
case "$prompt" in /compact*)
  echo '{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual","pre_tokens":9000,"post_tokens":900}}';;
esac
case "$prompt" in *crash*) exit 1;; esac
case "$prompt" in *garbled*) printf '\377\n'; exit 0;; esac
case "$prompt" in *busy*)
  trap 'kill $!; exit 143' TERM
  sleep 600 &
  wait;;
esac
case "$prompt" in *slow*)
  echo '{"type":"result","subtype":"success","result":"","num_turns":0,"session_id":"s1","total_cost_usd":0}'
  sleep 1;;
esac
case "$prompt" in *fail*)
  echo '{"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom","num_turns":1,"session_id":"s1","total_cost_usd":0}'
  exit 1;;
esac
case "$prompt" in *delegate*|*orphan*)
  echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a1","name":"Agent","input":{"description":"Find the parser","subagent_type":"Explore","prompt":"x"}}]}}'
  echo '{"type":"assistant","parent_tool_use_id":"a1","message":{"content":[{"type":"tool_use","id":"n1","name":"Agent","input":{"description":"Nested","subagent_type":"general-purpose","prompt":"y"}}]}}';;
esac
case "$prompt" in *delegate*)
  while [ ! -f go ]; do sleep 0.05; done
  echo '{"type":"user","parent_tool_use_id":"a1","message":{"content":[{"type":"tool_result","tool_use_id":"n1","content":"nested done"}]}}'
  echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a1","content":"found it"}]}}'
  while [ ! -f go2 ]; do sleep 0.05; done;;
esac
case "$prompt" in *bg-agent*)
  for i in 1 2; do
    echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"b'$i'","name":"Agent","input":{"description":"Audit '$i'","subagent_type":"Explore","prompt":"x","run_in_background":true}}]}}'
  done
  echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"s1","task_type":"local_bash","description":"Run the QA suite"},{"task_id":"m1","task_type":"monitor","description":"Watch the logs"},{"task_id":"t1","task_type":"local_agent","subagent_type":"Explore","description":"Audit 1"},{"task_id":"t2","task_type":"local_agent","subagent_type":"Explore","description":"Audit 2"}]}'
  for i in 1 2; do
    case "$prompt" in *no-task-id*) ;; *)
      echo '{"type":"system","subtype":"task_started","task_id":"t'$i'","tool_use_id":"b'$i'","description":"Audit '$i'","task_type":"local_agent","is_backgrounded":true}';;
    esac
    echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b'$i'","content":"Async agent launched successfully."}]}}'
  done
  case "$prompt" in *early*)
    echo '{"type":"assistant","message":{"content":[{"type":"text","text":"launched"}]}}'
    echo '{"type":"result","subtype":"success","result":"launched","num_turns":1,"session_id":"s1","total_cost_usd":0}';;
  esac
  case "$prompt" in *linger*) ;; *)
    while [ ! -f go ]; do sleep 0.05; done
    echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"s1","task_type":"local_bash","description":"Run the QA suite"},{"task_id":"m1","task_type":"monitor","description":"Watch the logs"},{"task_id":"t2","task_type":"local_agent","subagent_type":"Explore","description":"Audit 2"}]}'
    while [ ! -f go2 ]; do sleep 0.05; done
    echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"s1","task_type":"local_bash","description":"Run the QA suite"}]}'
    while [ ! -f go3 ]; do sleep 0.05; done;;
  esac;;
esac
case "$prompt" in *bg-agent*) ;; *linger*)
  echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b1","description":"Run the QA suite"}]}';;
esac
reply=started
case "$prompt" in *ask*) reply='Which one?';; esac
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"'"$reply"'"}]}}'
echo '{"type":"result","subtype":"success","result":"'"$reply"'","num_turns":1,"session_id":"'"$sid"'","total_cost_usd":'"$cost"'}'
case "$prompt" in *linger*|*hang*)
  trap 'kill $!; echo yes > terminated; exit 143' TERM
  sleep 600 &
  wait;;
esac
"#;

    pub(crate) fn fake_claude_state() -> (AppState, Agent) {
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

    pub(crate) async fn until(what: &str, secs: u64, f: impl Fn() -> bool) {
        let end = tokio::time::Instant::now() + Duration::from_secs(secs);
        while !f() {
            assert!(tokio::time::Instant::now() < end, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn status_text(st: &AppState, id: &str) -> String {
        st.db.get_agent(id).unwrap().unwrap().status_text
    }

    /// The memory part of each system prompt the fake claude was started with, in order.
    fn memory_parts(st: &AppState, id: &str) -> Vec<String> {
        let log = std::fs::read_to_string(workspace(st, id).join("system.log")).unwrap_or_default();
        log.split("\n---\n")
            .filter_map(|p| Some(p.split_once("# Long-term memory")?.1.split_once("# Loft tools")?.0.to_string()))
            .collect()
    }

    async fn run(st: &AppState, id: &str, text: &str, runs: usize) {
        enqueue(st, id, Some(text.into()), "user");
        until("the run", 10, || !is_running(st, id) && memory_parts(st, id).len() == runs).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memory_part_stays_the_same_for_a_session_and_is_rebuilt_after_clear_and_compact() {
        let (st, a) = fake_claude_state();
        let first = st.db.add_memory(&a.id, "Deploys go through staging", "Every deploy goes to staging first.", "").unwrap();
        run(&st, &a.id, "hello", 1).await;

        // Saved by the agent or in the Memory tab, changes wait for the next Session.
        st.db.add_memory(&a.id, "User prefers short replies", "Keep chat replies to a few lines.", "").unwrap();
        st.db.delete_memory(&a.id, first.id).unwrap();
        run(&st, &a.id, "again", 2).await;
        let parts = memory_parts(&st, &a.id);
        assert_eq!(parts[0], parts[1]);
        assert!(parts[1].contains("Deploys go through staging") && !parts[1].contains("short replies"), "{}", parts[1]);

        let _ = crate::api::reset_session(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await.unwrap();
        run(&st, &a.id, "after clear", 3).await;
        let parts = memory_parts(&st, &a.id);
        assert!(parts[2].contains("short replies") && !parts[2].contains("Deploys go through staging"), "{}", parts[2]);

        st.db.add_memory(&a.id, "Release on Fridays", "Releases ship on Friday mornings.", "").unwrap();
        compact(&st, &a.id, "");
        until("the compaction", 10, || !is_running(&st, &a.id) && memory_parts(&st, &a.id).len() == 4).await;
        assert_eq!(memory_parts(&st, &a.id)[3], parts[2], "the compaction run itself still sends the frozen part");
        run(&st, &a.id, "after compact", 5).await;
        assert!(memory_parts(&st, &a.id)[4].contains("Release on Fridays"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_run_costs_what_it_added_to_the_session() {
        let (st, a) = fake_claude_state();
        let costs = || -> Vec<f64> {
            let mut msgs = st.db.list_messages(&a.id, 200, None).unwrap();
            msgs.sort_by_key(|m| m.id);
            msgs.iter().filter(|m| m.kind == "run_end").map(|m| (m.meta["cost_usd"].as_f64().unwrap() * 1e6).round() / 1e6).collect()
        };
        // Claude Code reports the resumed session's running total.
        let s1 = "0b4355f1-11b6-4668-809e-1dc392203ef7";
        for (i, c) in ["0.005", "0.006", "0.007"].iter().enumerate() {
            run(&st, &a.id, &format!("sid={s1} cost={c}"), i + 1).await;
        }
        assert_eq!(costs(), vec![0.005, 0.001, 0.001]);
        assert!((st.db.get_agent(&a.id).unwrap().unwrap().total_cost_usd - 0.007).abs() < 1e-9);

        // After a Clear the new session counts from 0, even when its total tops the old one's.
        let _ = crate::api::reset_session(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await.unwrap();
        run(&st, &a.id, "sid=s2 cost=0.009", 4).await;
        run(&st, &a.id, "sid=s2 cost=0.010", 5).await;
        assert_eq!(costs()[3..], [0.009, 0.001]);

        // Resuming the first session picks up from its last total, not from 0.
        let dir = crate::sessions::tests::tmp();
        crate::sessions::tests::write(&dir, s1, &[r#"{"type":"user","timestamp":"2026-10-20T09:00:00Z","message":{"content":"hello"}}"#]);
        let _ = crate::api::resume_from(&st, &a.id, dir, s1.into()).await.unwrap();
        run(&st, &a.id, &format!("sid={s1} cost=0.008"), 6).await;
        // A total that went down is a fresh session: it counts whole, never negative.
        run(&st, &a.id, &format!("sid={s1} cost=0.002"), 7).await;
        assert_eq!(costs()[5..], [0.001, 0.002]);
        let a = st.db.get_agent(&a.id).unwrap().unwrap();
        assert!((a.total_cost_usd - 0.020).abs() < 1e-9, "{}", a.total_cost_usd);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resuming_a_past_session_resumes_it_with_a_fresh_memory_index() {
        let (st, a) = fake_claude_state();
        st.db.add_memory(&a.id, "Deploys go through staging", "Every deploy goes to staging first.", "").unwrap();
        run(&st, &a.id, "hello", 1).await;
        st.db.add_memory(&a.id, "User prefers short replies", "Keep chat replies to a few lines.", "").unwrap();

        let dir = crate::sessions::tests::tmp();
        let past = "0b4355f1-11b6-4668-809e-1dc392203ef7";
        crate::sessions::tests::write(&dir, past, &[
            r#"{"type":"user","timestamp":"2026-10-20T09:00:00Z","message":{"content":"Fix the login bug"}}"#,
            r#"{"type":"assistant","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":120000}}}"#,
        ]);
        let resume = |sid: &str| crate::api::resume_from(&st, &a.id, dir.clone(), sid.into());
        assert_eq!(resume("feed-404").await.unwrap_err().0, axum::http::StatusCode::NOT_FOUND);
        st.rt.lock().unwrap().running.insert(a.id.clone(), Arc::new(Notify::new()));
        assert_eq!(resume(past).await.unwrap_err().0, axum::http::StatusCode::CONFLICT, "not while it runs");
        st.rt.lock().unwrap().running.remove(&a.id);

        let _ = resume(past).await.unwrap();
        let now = st.db.get_agent(&a.id).unwrap().unwrap();
        assert_eq!(now.session_id.as_deref(), Some(past));
        assert_eq!(now.context_tokens, Some(120010), "the header shows the resumed Session's size");
        let marker = st.db.list_messages(&a.id, 50, None).unwrap().into_iter().find(|m| m.kind == "resume").unwrap();
        assert_eq!(marker.content, "Resumed the Session from 2026-10-20.");
        assert_eq!(resume(past).await.unwrap_err().0, axum::http::StatusCode::BAD_REQUEST, "already its Session");

        run(&st, &a.id, "where were we?", 2).await;
        let resumed = std::fs::read_to_string(workspace(&st, &a.id).join("resumed.log")).unwrap();
        assert_eq!(resumed.lines().last(), Some(past), "the next run resumes the picked Session");
        assert!(memory_parts(&st, &a.id)[1].contains("short replies"), "the memory index is built afresh");
    }

    /// A transcripts folder holding one past Session, and that Session's id.
    fn past_session() -> (std::path::PathBuf, String) {
        let dir = crate::sessions::tests::tmp();
        let past = "0b4355f1-11b6-4668-809e-1dc392203ef7";
        crate::sessions::tests::write(&dir, past, &[r#"{"type":"user","timestamp":"2026-10-20T09:00:00Z","message":{"content":"Fix the login bug"}}"#]);
        (dir, past.into())
    }

    #[tokio::test]
    async fn resuming_a_past_session_drops_the_queued_messages() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        {
            let mut rt = st.rt.lock().unwrap();
            rt.queued.insert(a.id.clone(), vec!["deploy to staging".into()]);
            rt.grill_queued.insert(a.id.clone(), vec!["[Loft: grill round]".into()]);
        }
        let (dir, past) = past_session();
        let _ = crate::api::resume_from(&st, &a.id, dir, past).await.unwrap();
        let rt = st.rt.lock().unwrap();
        assert!(rt.queued.get(&a.id).is_none_or(|q| q.is_empty()), "{:?}", rt.queued.get(&a.id));
        assert!(rt.grill_queued.get(&a.id).is_none_or(|q| q.is_empty()), "{:?}", rt.grill_queued.get(&a.id));
    }

    #[tokio::test]
    async fn resuming_a_past_session_briefs_the_next_grill_again() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Lead");
        assert!(crate::grill::with_brief(&st, &a.id, "round 1".into()).contains("# Planning: grilling"));
        assert_eq!(crate::grill::with_brief(&st, &a.id, "round 2".into()), "round 2", "briefed in this Session");
        let (dir, past) = past_session();
        let _ = crate::api::resume_from(&st, &a.id, dir, past).await.unwrap();
        assert!(crate::grill::with_brief(&st, &a.id, "round 3".into()).contains("# Planning: grilling"), "the resumed Session never had it");
    }

    #[test]
    fn memory_index_lists_titles_newest_first_and_never_bodies() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        assert!(system_prompt(&st, &a).contains("(no memories yet)"));
        let old = st.db.add_memory(&a.id, "Deploys go through staging", "BODY-ONE every deploy goes to staging first.", "ops").unwrap();
        let new = st.db.add_memory(&a.id, "User prefers short replies", "BODY-TWO keep replies to a few lines.", "").unwrap();
        let p = system_prompt(&st, &a);
        let (o, n) = (format!("- #{} Deploys go through staging", old.id), format!("- #{} User prefers short replies", new.id));
        assert!(p.find(&n).unwrap() < p.find(&o).unwrap(), "newest first: {p}");
        assert!(!p.contains("BODY-") && !p.contains("older memor"), "{p}");
    }

    #[test]
    fn memory_index_keeps_the_newest_that_fit_and_says_how_many_older_exist() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let ids: Vec<i64> = (0..400).map(|i| st.db.add_memory(&a.id, &format!("Status of epic {i:03}: {}", "x".repeat(40)), "body", "").unwrap().id).collect();
        let block = memory_block(&st, &a.id);
        assert!(block.chars().count() <= MEMORY_INDEX_CHARS + 100, "about 3k tokens");
        let lines: Vec<&str> = block.lines().collect();
        let listed = lines.len() - 1;
        assert!(listed > 100 && listed < 400, "{listed}");
        assert!(lines[0].starts_with(&format!("- #{} Status of epic 399", ids[399])));
        assert!(lines[listed - 1].contains(&format!("epic {:03}", 400 - listed)), "the newest ones, without gaps");
        assert_eq!(*lines.last().unwrap(), format!("({} older memories: use `mcp__loft__recall` to reach them.)", 400 - listed));
    }

    #[test]
    fn an_updated_memory_moves_to_the_top_of_the_index() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let ids: Vec<i64> = (0..400).map(|i| st.db.add_memory(&a.id, &format!("Status of epic {i:03}: {}", "x".repeat(40)), "body", "").unwrap().id).collect();
        st.db.update_memory(&a.id, ids[0], None, Some("Epic 000 shipped.")).unwrap();
        let block = memory_block(&st, &a.id);
        let lines: Vec<&str> = block.lines().collect();
        assert!(lines[0].starts_with(&format!("- #{} Status of epic 000", ids[0])), "{}", lines[0]);
        assert!(lines[1].starts_with(&format!("- #{} Status of epic 399", ids[399])));
        assert!(!block.contains(&format!("- #{} ", ids[1])), "an old untouched one falls out");
        assert_eq!(*lines.last().unwrap(), format!("({} older memories: use `mcp__loft__recall` to reach them.)", 400 - (lines.len() - 1)));
    }

    #[test]
    fn clients_never_see_the_frozen_memory_index() {
        let st = crate::tasks::tests::state();
        let mut a = crate::tasks::tests::agent(&st, "Worker");
        a.memory_index = Some("- #1 secret".into());
        assert!(client_json(&a).get("memory_index").is_none());
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
        assert_eq!(v["subagents"], json!([]), "a background shell isn't a subagent");
        assert!(v["running"] == json!(true) && v["waiting_since"].is_string() && v["last_output_at"].is_string(), "{v}");
        let r = crate::api::restart_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await;
        assert!(r.is_ok());
        until("the restarted run to end", 10, || !is_running(&st, &a.id)).await;
        assert_eq!(fields(&st, &a.id), json!({ "running": false, "last_output_at": null, "waiting_on": [], "waiting_since": null, "subagents": [] }));
        let log = std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap();
        assert!(log.contains("restarted your last run (it was waiting on Run the QA suite)"), "{log}");
    }

    fn subagents(st: &AppState, id: &str) -> Vec<Value> {
        fields(st, id)["subagents"].as_array().cloned().unwrap_or_default()
    }

    /// The `subagents` of every agent-updated event pushed for the agent, from now on.
    fn pushed_subagents(st: &AppState, id: &str) -> Arc<std::sync::Mutex<Vec<Value>>> {
        let seen = Arc::new(std::sync::Mutex::new(vec![]));
        let (mut events, id, out) = (st.events.subscribe(), id.to_string(), seen.clone());
        tokio::spawn(async move {
            while let Ok(ev) = events.recv().await {
                let ev = serde_json::to_value(ev).unwrap();
                if ev["type"] == "agent_updated" && ev["agent"]["id"] == json!(id) {
                    out.lock().unwrap().push(json!({ "running": ev["agent"]["running"], "ids": ev["agent"]["subagents"].as_array().unwrap().iter().map(|s| s["id"].clone()).collect::<Vec<_>>() }));
                }
            }
        });
        seen
    }

    fn last_push(seen: &std::sync::Mutex<Vec<Value>>) -> Value {
        seen.lock().unwrap().last().cloned().unwrap_or_default()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_json_shows_its_running_subagents() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        let pushed = pushed_subagents(&st, &a.id);
        enqueue(&st, &a.id, Some("delegate".into()), "user");
        until("the nested call", 10, || st.db.list_messages(&a.id, 50, None).unwrap().iter().any(|m| m.content.contains("Nested"))).await;
        let subs = subagents(&st, &a.id);
        assert_eq!(subs.len(), 1, "the subagent's own Agent call isn't counted: {subs:?}");
        assert!(subs[0]["id"] == "a1" && subs[0]["description"] == "Find the parser" && subs[0]["type"] == "Explore" && subs[0]["started_at"].is_string(), "{subs:?}");
        until("the start pushed", 10, || last_push(&pushed)["ids"] == json!(["a1"])).await;
        std::fs::write(ws.join("go"), "").unwrap();
        until("the subagent's result", 10, || subagents(&st, &a.id).is_empty()).await;
        assert!(is_running(&st, &a.id), "the result ends the subagent, not the run");
        // Nothing else changes before `go2`: only the result's own push can tell clients.
        until("the result pushed", 10, || last_push(&pushed) == json!({ "running": true, "ids": [] })).await;
        std::fs::write(ws.join("go2"), "").unwrap();
        until("the run to end", 10, || !is_running(&st, &a.id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_run_ending_clears_its_subagents() {
        let (st, a) = fake_claude_state();
        let pushed = pushed_subagents(&st, &a.id);
        enqueue(&st, &a.id, Some("orphan".into()), "user");
        until("the run to end", 10, || !is_running(&st, &a.id) && st.db.list_messages(&a.id, 50, None).unwrap().iter().any(|m| m.kind == "run_end")).await;
        assert!(subagents(&st, &a.id).is_empty());
        let seen = pushed.lock().unwrap().clone();
        let started = seen.iter().position(|p| p["ids"] == json!(["a1"])).expect("the subagent was pushed while the run went on");
        let cleared = seen[started..].iter().position(|p| p["ids"] == json!([])).expect("the run's end was pushed") + started;
        assert_eq!(seen[cleared]["running"], json!(true), "the end of the run itself clears them, before the agent goes idle: {seen:?}");

        let (st, a) = fake_claude_state();
        enqueue(&st, &a.id, Some("delegate".into()), "user");
        until("the subagent", 10, || subagents(&st, &a.id).len() == 1).await;
        assert!(crate::api::restart_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await.is_ok());
        until("the restarted run to end", 10, || !is_running(&st, &a.id)).await;
        assert!(subagents(&st, &a.id).is_empty());
    }

    fn subagent_ids(st: &AppState, id: &str) -> Vec<Value> {
        subagents(st, id).iter().map(|s| s["id"].clone()).collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_background_subagent_counts_until_the_task_list_drops_it() {
        for prompt in ["bg-agent", "bg-agent no-task-id"] {
            let (st, a) = fake_claude_state();
            let ws = workspace(&st, &a.id);
            let pushed = pushed_subagents(&st, &a.id);
            enqueue(&st, &a.id, Some(prompt.into()), "user");
            until("the launch results", 10, || st.db.list_messages(&a.id, 50, None).unwrap().iter().filter(|m| m.content.contains("launched successfully")).count() == 2).await;
            assert_eq!(subagent_ids(&st, &a.id), vec![json!("b1"), json!("b2")], "{prompt}: the launch doesn't end them, and the shell isn't a subagent");
            std::fs::write(ws.join("go"), "").unwrap();
            if prompt == "bg-agent" {
                until("the task list to drop b1 only", 10, || subagent_ids(&st, &a.id) == vec![json!("b2")]).await;
                until("the drop pushed", 10, || last_push(&pushed)["ids"] == json!(["b2"])).await;
            }
            // Without task ids, they end together once no subagent of the agent's own is listed.
            std::fs::write(ws.join("go2"), "").unwrap();
            until("the task list to drop them", 10, || subagents(&st, &a.id).is_empty()).await;
            until("the drop pushed", 10, || last_push(&pushed) == json!({ "running": true, "ids": [] })).await;
            std::fs::write(ws.join("go3"), "").unwrap();
            until("the run to end", 10, || !is_running(&st, &a.id)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_waiting_bar_names_shells_and_follows_the_task_list() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        let status = || st.db.get_agent(&a.id).unwrap().unwrap().status_text;
        enqueue(&st, &a.id, Some("bg-agent early".into()), "user");
        until("the reply", 10, || fields(&st, &a.id)["waiting_since"].is_string()).await;
        // The subagent bar shows the subagents; the waiting bar only the shell and the monitor.
        assert_eq!(fields(&st, &a.id)["waiting_on"], json!(["Run the QA suite", "Watch the logs"]));
        assert_eq!(subagent_ids(&st, &a.id), vec![json!("b1"), json!("b2")]);
        assert_eq!(status(), "Waiting on: Run the QA suite, Watch the logs, Audit 1, Audit 2");
        std::fs::write(ws.join("go"), "").unwrap();
        until("the task list to drop b1", 10, || subagent_ids(&st, &a.id) == vec![json!("b2")]).await;
        until("the status to drop it", 10, || status() == "Waiting on: Run the QA suite, Watch the logs, Audit 2").await;
        std::fs::write(ws.join("go2"), "").unwrap();
        until("the status to drop b2 and the monitor", 10, || status() == "Waiting on: Run the QA suite").await;
        assert!(subagents(&st, &a.id).is_empty());
        assert_eq!(fields(&st, &a.id)["waiting_on"], json!(["Run the QA suite"]));
        std::fs::write(ws.join("go3"), "").unwrap();
        until("the run to end", 10, || !is_running(&st, &a.id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_message_stopping_the_run_clears_its_background_subagents() {
        let (st, a) = fake_claude_state();
        enqueue(&st, &a.id, Some("bg-agent linger".into()), "user");
        until("the reply", 10, || !fields(&st, &a.id)["waiting_on"].as_array().unwrap().is_empty()).await;
        assert_eq!(subagents(&st, &a.id).len(), 2, "still running after the reply");
        enqueue(&st, &a.id, Some("next".into()), "user");
        until("the new message to run", 10, || !is_running(&st, &a.id)).await;
        assert!(workspace(&st, &a.id).join("terminated").exists());
        assert!(subagents(&st, &a.id).is_empty());
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

    fn unread(st: &AppState, id: &str) -> bool {
        st.db.get_agent(id).unwrap().unwrap().unread_reply()
    }

    fn send(st: &AppState, id: &str, text: &str) -> impl std::future::Future<Output = crate::api::ApiResult<Value>> {
        crate::api::send_message(axum::extract::State(st.clone()), axum::extract::Path(id.to_string()), axum::Json(serde_json::from_value(json!({ "text": text })).unwrap()))
    }

    async fn idle(st: &AppState, id: &str) {
        until("the runs to end", 10, || !is_running(st, id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reply_to_the_user_is_unread_until_marked_read_everywhere() {
        let (st, a) = fake_claude_state();
        assert!(!unread(&st, &a.id));
        let _ = send(&st, &a.id, "hello").await.ok().unwrap();
        idle(&st, &a.id).await;
        let got = crate::api::get_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await.ok().unwrap().0;
        assert_eq!(got["unread_reply"], json!(true));

        let mut events = st.events.subscribe();
        let r = crate::api::mark_read(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await.ok().unwrap().0;
        assert_eq!(r["unread_reply"], json!(false));
        let ev = serde_json::to_value(events.try_recv().unwrap()).unwrap();
        assert_eq!((ev["type"].clone(), ev["agent"]["id"].clone(), ev["agent"]["unread_reply"].clone()), (json!("agent_updated"), json!(a.id), json!(false)), "other browsers hear about it");
        assert!(crate::api::mark_read(axum::extract::State(st.clone()), axum::extract::Path("nope".into())).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_and_scheduled_runs_leave_an_unread_reply() {
        let (st, a) = fake_claude_state();
        let _ = send(&st, &a.id, "fail").await.ok().unwrap();
        idle(&st, &a.id).await;
        assert!(st.db.list_messages(&a.id, 50, None).unwrap().iter().any(|m| m.kind == "error" && m.content == "boom"));
        assert!(unread(&st, &a.id), "an error answers the user too");

        let b = crate::tasks::tests::agent(&st, "Scheduled");
        enqueue(&st, &b.id, None, "schedule");
        idle(&st, &b.id).await;
        assert!(unread(&st, &b.id));

        let c = crate::tasks::tests::agent(&st, "Run now");
        let _ = crate::api::run_now(axum::extract::State(st.clone()), axum::extract::Path(c.id.clone())).await.ok().unwrap();
        idle(&st, &c.id).await;
        assert!(unread(&st, &c.id), "Run now is the user starting it");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_that_cannot_start_leaves_an_unread_reply() {
        let (st, a) = fake_claude_state();
        let mut s = st.settings.get();
        s.claude_bin = "/nonexistent/claude".into();
        st.settings.save(s).unwrap();
        let _ = send(&st, &a.id, "hello").await.ok().unwrap();
        idle(&st, &a.id).await;
        assert!(st.db.list_messages(&a.id, 50, None).unwrap().iter().any(|m| m.kind == "error" && m.content.contains("failed to start")));
        assert!(unread(&st, &a.id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_task_comment_sent_to_the_agent_counts() {
        let (st, a) = fake_claude_state();
        let (t, _) = crate::tasks::create(&st, &json!({ "title": "Fix it", "agent_id": a.id }), "user").unwrap();
        let body = serde_json::from_value(json!({ "text": "Any update?" })).unwrap();
        let _ = crate::tasks::add_comment(axum::extract::State(st.clone()), axum::extract::Path(t.id.clone()), axum::Json(body)).await.ok().unwrap();
        idle(&st, &a.id).await;
        assert!(std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap().contains("Any update?"));
        assert!(unread(&st, &a.id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runs_started_only_by_agents_leave_none() {
        let (st, a) = fake_claude_state();
        let lead = crate::tasks::tests::agent(&st, "Lead");
        message_from_agent(&st, &lead, &a, "status?");
        idle(&st, &a.id).await;
        assert!(!unread(&st, &a.id), "a direct message from an agent");

        // Queued while the agent is busy: it runs next with trigger "user", but no message of the user's started it.
        message_from_agent(&st, &lead, &a, "busy");
        until("the run to start", 10, || st.db.get_agent(&a.id).unwrap().unwrap().status == "running").await;
        message_from_agent(&st, &lead, &a, "and this");
        stop(&st, &a.id);
        idle(&st, &a.id).await;
        assert!(std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap().contains("and this"));
        assert!(!unread(&st, &a.id), "an agent's message queued behind a run");

        design_avatar(&st, &a.id, "a fox");
        idle(&st, &a.id).await;
        wake(&st, &a.id, "[Loft: grill] next round".into(), "grill");
        idle(&st, &a.id).await;
        assert!(!unread(&st, &a.id), "avatar and grill runs are quiet");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_queued_run_mixing_the_users_message_and_an_agents_counts() {
        let (st, a) = fake_claude_state();
        let lead = crate::tasks::tests::agent(&st, "Lead");
        message_from_agent(&st, &lead, &a, "busy");
        until("the run to start", 10, || st.db.get_agent(&a.id).unwrap().unwrap().status == "running").await;
        message_from_agent(&st, &lead, &a, "from the lead");
        let _ = send(&st, &a.id, "from me").await.ok().unwrap();
        stop(&st, &a.id);
        idle(&st, &a.id).await;
        assert!(unread(&st, &a.id));
    }

    async fn restart(st: &AppState, id: &str) {
        let _ = crate::api::restart_agent(axum::extract::State(st.clone()), axum::extract::Path(id.to_string())).await.ok().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_run_keeps_answering_the_user() {
        let (st, a) = fake_claude_state();
        let running = |st: &AppState, id: &str| st.db.get_agent(id).unwrap().unwrap().status == "running";
        let _ = send(&st, &a.id, "busy").await.ok().unwrap();
        until("the run to start", 10, || running(&st, &a.id)).await;
        restart(&st, &a.id).await;
        idle(&st, &a.id).await;
        assert!(std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap().contains("restarted your last run"));
        assert!(unread(&st, &a.id), "the restarted run's reply");

        let b = crate::tasks::tests::agent(&st, "Failing");
        let _ = send(&st, &b.id, "busy").await.ok().unwrap();
        until("the run to start", 10, || running(&st, &b.id)).await;
        let mut s = st.settings.get();
        s.claude_bin = "/nonexistent/claude".into();
        st.settings.save(s).unwrap();
        restart(&st, &b.id).await;
        idle(&st, &b.id).await;
        assert!(st.db.list_messages(&b.id, 50, None).unwrap().iter().any(|m| m.kind == "error" && m.content.contains("failed to start")));
        assert!(unread(&st, &b.id), "the restarted run's error");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restarting_an_agents_run_leaves_none() {
        let (st, a) = fake_claude_state();
        let lead = crate::tasks::tests::agent(&st, "Lead");
        message_from_agent(&st, &lead, &a, "busy");
        until("the run to start", 10, || st.db.get_agent(&a.id).unwrap().unwrap().status == "running").await;
        restart(&st, &a.id).await;
        idle(&st, &a.id).await;
        assert!(std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap().contains("restarted your last run"));
        assert!(!unread(&st, &a.id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restarting_runs_whats_queued_as_whoever_sent_it() {
        let (st, a) = fake_claude_state();
        let lead = crate::tasks::tests::agent(&st, "Lead");
        let asked = |st: &AppState| crate::needs_you::snapshot(st).unwrap()["items"].as_array().unwrap().iter().any(|i| i["type"] == "question");
        let running = |st: &AppState, id: &str| st.db.get_agent(id).unwrap().unwrap().status == "running";
        // My run is restarted with only an agent's message queued: that message runs next, as the agent's.
        let _ = send(&st, &a.id, "busy").await.ok().unwrap();
        until("the run to start", 10, || running(&st, &a.id)).await;
        message_from_agent(&st, &lead, &a, "ask");
        restart(&st, &a.id).await;
        idle(&st, &a.id).await;
        assert!(!std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap().contains("restarted your last run"));
        assert!(!unread(&st, &a.id) && !asked(&st), "only an agent sent the queued message");

        // My message is queued: it still counts.
        let b = crate::tasks::tests::agent(&st, "Mine");
        message_from_agent(&st, &lead, &b, "busy");
        until("the run to start", 10, || running(&st, &b.id)).await;
        let _ = send(&st, &b.id, "hello").await.ok().unwrap();
        restart(&st, &b.id).await;
        idle(&st, &b.id).await;
        assert!(unread(&st, &b.id), "my queued message");

        // Mine and an agent's queued together: still counts.
        let c = crate::tasks::tests::agent(&st, "Mixed");
        message_from_agent(&st, &lead, &c, "busy");
        until("the run to start", 10, || running(&st, &c.id)).await;
        message_from_agent(&st, &lead, &c, "hello");
        let _ = send(&st, &c.id, "hello").await.ok().unwrap();
        restart(&st, &c.id).await;
        idle(&st, &c.id).await;
        assert!(unread(&st, &c.id), "my message among the queued ones");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_a_run_the_user_started_asks_them_something() {
        let (st, a) = fake_claude_state();
        let lead = crate::tasks::tests::agent(&st, "Lead");
        let asked = |st: &AppState| crate::needs_you::snapshot(st).unwrap()["items"].as_array().unwrap().iter().any(|i| i["type"] == "question");
        message_from_agent(&st, &lead, &a, "ask");
        idle(&st, &a.id).await;
        assert_eq!(st.db.list_messages(&a.id, 50, None).unwrap().iter().rev().find(|m| m.role == "assistant").map(|m| m.content.clone()), Some("Which one?".into()));
        assert!(!asked(&st), "an agent started the run");

        let _ = send(&st, &a.id, "ask").await.ok().unwrap();
        idle(&st, &a.id).await;
        assert!(asked(&st));
    }
}
