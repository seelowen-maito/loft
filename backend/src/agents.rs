//! Agent runtime: each run spawns a headless Claude Code process inside the agent's
//! workspace, streams its events into the chat log, and routes permission prompts
//! to the UI through the Loft MCP server.

use std::path::{Path, PathBuf};
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

/// How long a tool call that waits for the user may run: the permission timeout plus a minute. It sets claude's
/// MCP and Bash timeouts and the `timeout` the prompt tells agents to give a kill, so the three can't drift apart.
pub const TOOL_WAIT: Duration = Duration::from_secs(PERMISSION_TIMEOUT.as_secs() + 60);

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
/// projects folder (`work/notes`), else its workspace (no project, or a project that's just a tag).
pub fn at_base(state: &AppState, a: &Agent) -> PathBuf {
    let projects = &state.settings.boot().projects_dir;
    let project = projects.join(a.project.trim());
    match (a.project.trim().is_empty(), project.canonicalize(), projects.canonicalize()) {
        (false, Ok(p), Ok(root)) if p.starts_with(&root) && p.is_dir() => p,
        _ => workspace(state, &a.id),
    }
}

/// Whether the agent has had no run for the idle timeout (Settings): not running, and its last run started, and
/// ended, at least that long before `now`. An agent that never ran is idle.
pub fn idle(state: &AppState, a: &Agent, now: std::time::SystemTime) -> bool {
    let timeout = Duration::from_secs(u64::from(state.settings.get().idle_timeout_hours) * 3600);
    let rt = state.rt.lock().unwrap();
    if rt.running.contains_key(&a.id) {
        return false;
    }
    let at = |t: &Option<String>| t.as_deref().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(std::time::SystemTime::from);
    let last = at(&a.last_run_at).max(at(&a.last_run_ended_at));
    last.is_none_or(|t| now.duration_since(t).unwrap_or_default() >= timeout)
}

pub fn emit_agent(state: &AppState, a: &Agent) {
    let mut v = client_json(a);
    run_fields(state, &a.id, &mut v);
    v["unread_reply"] = json!(a.unread_reply());
    v["fresh_context_applies"] = json!(crate::delivery::fresh_context_applies(state, a));
    // The settings panel takes the saved agent from this event: a fixed project clears its flag.
    v["project_problems"] = crate::repos::project_problems(state, a);
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
    v["queued_messages"] = json!(rt.queued_messages.get(agent_id).map_or(&vec![], |q| q));
    // A Redraw waiting behind the current run or drawing now: the Redraw button shows it's busy.
    v["redrawing"] = json!(rt.redraw_queued.contains(agent_id) || rt.redrawing.contains(agent_id));
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

/// A run quiet this long with nothing to wait on is restarted, as if the user pressed Restart.
const RESTART_QUIET: Duration = Duration::from_secs(20 * 60);
/// Automatic restarts in a row with no output between; then it's left to the user (Needs you says why).
pub(crate) const MAX_AUTO_RESTARTS: u32 = 2;

/// Restarts runs that went quiet with nothing to wait on: no tool call, background work or subagent still
/// going, no permission or interview question waiting on the user. A dropped API connection looks like this.
pub(crate) async fn restart_quiet(state: &AppState) {
    let due: Vec<(String, i64)> = {
        let rt = state.rt.lock().unwrap();
        rt.running
            .keys()
            .filter_map(|id| {
                let i = rt.runs.get(id)?;
                let quiet = DateTime::parse_from_rfc3339(&i.last_output_at).ok().map(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds())?;
                let idle = i.waiting_on.is_empty() && i.subagents.is_empty() && i.tools_open.is_empty() && quiet >= RESTART_QUIET.as_secs() as i64;
                (idle && rt.auto_restarts.get(id).copied().unwrap_or(0) < MAX_AUTO_RESTARTS).then(|| (id.clone(), quiet / 60))
            })
            .collect()
    };
    for (id, minutes) in due {
        let asking = state.db.list_permissions(Some(&id)).map_or(true, |ps| ps.iter().any(|p| p.status == "pending"));
        let interviewing = crate::grill::active(state, &id).is_some_and(|s| crate::grill::open_round(&s).is_some());
        if asking || interviewing {
            continue;
        }
        *state.rt.lock().unwrap().auto_restarts.entry(id.clone()).or_default() += 1;
        tracing::info!("restarting {id}: no output for {minutes} min");
        let _ = crate::api::restart_run(state.clone(), id, "Loft").await;
    }
}

/// Checks for quiet runs once a minute.
pub async fn quiet_watcher(state: AppState) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        restart_quiet(&state).await;
    }
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
    ask_permission_for(state, agent_id, source, tool_name, input, reason, rule, PERMISSION_TIMEOUT).await
}

/// Marks a request expired when it ends unanswered: at the timeout, or when whoever asked stops waiting (a kill
/// whose command was stopped), so a later answer can't act on it.
struct Unanswered<'a> {
    state: &'a AppState,
    id: String,
    answered: bool,
}

impl Drop for Unanswered<'_> {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        self.state.rt.lock().unwrap().waiters.remove(&self.id);
        if let Ok(Some(mut r)) = self.state.db.get_permission(&self.id)
            && r.status == "pending" {
                r.status = "expired".into();
                let _ = self.state.db.save_permission(&r);
                self.state.emit(Event::Permission { request: serde_json::to_value(&r).unwrap() });
            }
    }
}

/// `ask_permission`, waiting at most `timeout` for the user.
#[allow(clippy::too_many_arguments)]
pub async fn ask_permission_for(
    state: &AppState,
    agent_id: &str,
    source: &str,
    tool_name: &str,
    input: Value,
    reason: &str,
    rule: String,
    timeout: Duration,
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

    let mut pending = Unanswered { state, id: req.id.clone(), answered: false };
    let decision = match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(d)) => {
            pending.answered = true;
            d
        }
        _ => Decision { allow: false, message: Some("The user did not respond to the permission request in time.".into()) },
    };
    drop(pending);
    if is_running(state, agent_id) {
        set_status(state, agent_id, "running", &prev);
    }
    decision
}

/// Resolve a pending request from the UI. `always` persists the rule into the agent config.
pub fn decide(state: &AppState, id: &str, decision: &str, rule: Option<String>, note: Option<String>) -> Result<PermissionRequest> {
    let mut req = state.db.get_permission(id)?.ok_or_else(|| anyhow::anyhow!("no such request"))?;
    anyhow::ensure!(req.status == "pending", "request already {}", req.status);
    // A kill's rule is the one Loft made from the processes, not one posted with the answer.
    if let Some(r) = rule.filter(|r| !r.trim().is_empty() && req.tool_name != "Kill") {
        req.rule = r.trim().to_string();
    }
    let allow = decision != "deny";
    // Explicit requests are always about config, so any grant persists. A kill of a process the agent didn't start
    // saves only a rule naming a program (`Kill(<path>)`), never a bare `Kill`.
    if req.tool_name == "Kill" && decision == "allow_always" {
        anyhow::ensure!(crate::processes::kill_spec(&req.rule).is_some(), "there is no rule for this kill: allow it once");
    }
    // A change to a user's checkout is allowed one command at a time: a saved rule would turn the guard off there.
    anyhow::ensure!(!(req.tool_name == "Checkout" && decision == "allow_always"), "a change to your checkout can only be allowed once");
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
5. Don't reply to an FYI.
6. When a needs-action message hands over a ticket or is about one, name it in `ticket`, as the issue, not its pull request: the recipient's context may be cleared between tickets, never within one.";

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
You are **{name}** ({role}), an autonomous agent living inside Loft — the user's local developer command center. You talk with the user through the Loft chat UI, which shows your replies, reasoning, and every tool call.

# Personality
{personality}

# Standing instructions
{prompt}

# Schedule
{schedule}

# Workspace
- Your private workspace is `{ws}`. It is your current working directory. Put every file you create there: scripts, notes, scratch data, and clones of repositories.
- The user's projects live in `{projects}`. Never edit them in place. To work on one, clone it into your workspace (`git clone {projects}/<name>` or clone from its remote) and work on the clone.
- Keep that one clone per project and reuse it for every ticket and review: make a branch per ticket in it, and to check a PR run `git fetch origin pull/<N>/head` and `git checkout --detach <sha>` (the SHA your verdict names). Never clone a project again for a ticket or a review while that clone exists; if it's gone (Loft deletes a clone when everything in it is pushed), clone the project again first and carry on. Never name a folder after a PR. If you need two branches at once, `git worktree add` from your own clone.
- Before each new branch, `git fetch origin` and create it from the remote: `git switch -c <branch> origin/<base>`, where `<base>` is the branch named in the ticket's `## Branch` section, or otherwise the default branch (`origin/HEAD`, whether it's `main` or `master`). Before each build or review round, `git fetch origin` and `git pull --ff-only` on your branch. Never build a ticket on a local default branch you haven't fetched.
- Before you submit a ticket, mark a PR ready or start a review round, `git fetch origin` and `git merge origin/<base>` into your branch (merge, not rebase, so nobody force-pushes), resolve any conflicts, re-run the tests, then push. Loft refuses a submission whose branch is behind its base.
- If your clone's `origin` isn't the project's GitHub remote (`git remote get-url origin`), `git remote set-url origin <the project's GitHub URL>` before fetching.
- When your PR is merged: `git fetch --prune origin`, `git switch --detach origin/<base>`, then `git branch -d <branch>` (never `-D`: git refuses if it isn't merged, and then you leave it and say so). If you used a worktree for it, `git worktree remove <path>` without `--force`.
{team}- Keep a tidy workspace. Knowledge (decisions, how things work, gotchas worth keeping) goes in your memory, or in the repo's docs or ADRs when the team needs it, not in files. Reports (QA or security write-ups, plans, audits) go in `notes/`, kept small: text with a few images. Debug and test output (logs, `.xcresult` bundles, recordings, screenshots for a PR, fixtures) goes in `scratch/<ticket number>/` (for example `scratch/412/`, or `scratch/` when there is no ticket; Loft clears that after a week); Loft deletes a ticket's folder once the ticket is done, and test output left in `notes/` after a week, so never keep anything you need later in either.
- When a repo you work on has `GLOSSARY.md` or `docs/adr/`, read them before you start and use the glossary's terms in code, docs and chat; respect the decisions the ADRs record.
- Assign every GitHub issue you create to the user (`gh issue create --assignee @me`), unless the user explicitly says to assign no one or someone else.
- `@path` in a user message (`@docs/plan.md`, `@../imole/`) points at a file or folder relative to `{at}`, not to your working directory. Read it when it matters to the request.

# Runs
Each message starts a run that ends with your reply. Claude Code background work doesn't outlive it: `run_in_background` commands, `Monitor`, `ScheduleWakeup` and background subagents are stopped as soon as a new message arrives, and until then you look busy and messages wait. So finish the work, or wait for it inside the run (a foreground command with a long enough `timeout`, or a polling loop), before you reply. Anything that must keep running after the reply, like a dev server, goes detached from a normal Bash call: `nohup <command> > <log> 2>&1 &`. Start servers with `nohup … &`, never with a daemon option (`--daemonize`, `--detach`, `pg_ctl start`). Stop your own processes any way (`kill $pid`, `pkill -f vite`, `killall node`): Loft lets kills reach only what your runs started or what runs in your workspace, and asks the user before stopping anything else (give that Bash call a `timeout` of {tool_wait}: it waits for them). To check on it later, say so in the reply; your next message or wake-up is the next turn.

# Showing your work
The user usually reads this chat from another device, where `localhost` doesn't reach this machine.
- Dev servers (web apps, APIs, docs sites): serve them only on your own ports, {ports} (`$PORT` is the first, `${range_var}` the whole range): they are yours on every run, and no other agent uses them. Start them detached (see Runs), listening on all interfaces (`vite --host --port $PORT`, `next dev -H 0.0.0.0` (it reads `$PORT`), `python3 -m http.server $PORT --bind 0.0.0.0`), then check `curl -sI http://{host}:<port>` answers and reply with the link `http://{host}:<port>`, a port in your range. Never share a `localhost` or `127.0.0.1` link. If the framework rejects the host name, allow it in its config (e.g. Next.js `allowedDevOrigins`).
- iOS apps: run them in the Simulator on this Mac, only on your own device (`<udid>` below), never `booted` or another one: other agents run theirs. {device} Boot it with `xcrun simctl boot <udid>` and `open -a Simulator`. Build with `xcodebuild -scheme <scheme> -destination 'platform=iOS Simulator,id=<udid>' -IDEBuildLocationStyle=Unique`, with no `-derivedDataPath`, so the output goes to Xcode's DerivedData and not into your workspace. Find the .app with the same flags and `-showBuildSettings` (`BUILT_PRODUCTS_DIR`, `FULL_PRODUCT_NAME`), then `xcrun simctl install <udid> "$BUILT_PRODUCTS_DIR/$FULL_PRODUCT_NAME"` and `xcrun simctl launch <udid> <bundle id>`. The user watches it live at [Simulator](/simulator) in Loft, so include that link in your reply. To check the screen yourself, save `xcrun simctl io <udid> screenshot <file>.png` and read the image.{axe}

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
- Pull requests: when you open one with `gh pr create`, Loft shows the user a live PR card (status, checks, a merge button) automatically. If you opened a PR some other way, call `mcp__loft__share_pull_request` with its URL so the card appears. Always include the PR link in your reply. If you check pull requests (tests, QA or security) and are a reviewer, set your verdict with `mcp__loft__set_pr_verdict` (the PR url, `kind`, `pass` or `fail`, and the head `sha` you checked) instead of editing labels by hand.
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
        team = team_lines(a),
        subagents = subagents,
        grill = crate::grill::prompt_section(),
        messaging = if state.settings.get().messaging_rules { MESSAGING_RULES } else { "" },
        role = crate::planning::role_label(if a.role.is_empty() { "custom" } else { &a.role }),
        personality = if a.personality.trim().is_empty() { "Helpful, direct, and curious." } else { a.personality.trim() },
        prompt = if a.prompt.trim().is_empty() { "(none yet — ask the user what they'd like you to own)" } else { a.prompt.trim() },
        ws = ws.display(),
        at = at_base(state, a).display(),
        projects = state.settings.boot().projects_dir.display(),
        axe = crate::simulator::axe()
            .map(|p| format!(" To drive the app, use AXe at `{}`: `tap -x <x> -y <y> --tap-style physical`, `swipe`, `type`, `describe-ui` (coordinates in points; see `--help`), always with `--udid <udid>`.", p.display()))
            .unwrap_or_default(),
        host = crate::tailnet::host().unwrap_or_else(|| "<this Mac's Tailscale or LAN host name>".into()),
        ports = crate::ports::range(state, &a.id).map(|r| format!("{}–{}", r.start(), r.end())).unwrap_or_else(|_| "your port range".into()),
        range_var = crate::ports::RANGE_VAR,
        tool_wait = TOOL_WAIT.as_millis(),
        device = match state.db.agent_device(&a.id).ok().flatten() {
            Some(udid) => format!(
                "Your device's UDID is `{udid}` (also in `${var}`); if simctl says it doesn't exist, call `mcp__loft__simulator_device` for a new one.",
                var = crate::simulator::DEVICE_VAR
            ),
            None => "You have none yet: call `mcp__loft__simulator_device` the first time you need one. It creates your device and gives you its UDID.".into(),
        },
    )
}

/// The Workspace lines naming its group and projects, main first; none when it has neither.
fn team_lines(a: &Agent) -> String {
    let mut s = String::new();
    if !a.group.trim().is_empty() {
        s += &format!("- You belong to the group **{}**. `mcp__loft__list_agents` shows its other agents and their projects.\n", a.group.trim());
    }
    if let Some((main, rest)) = a.projects.split_first() {
        let rest = rest.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ");
        s += &format!("- Your main project is `{main}`: the issues you create go there unless you pass another `project`.");
        if !rest.is_empty() {
            s += &format!(" You also work in {rest}. You can only create issues in your own projects; when you plan (grill, spec, tickets), say which one with `project`.");
        }
        s += "\n";
    }
    s
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
        let p = if refs.is_empty() { p } else { format!("{p}\n\n{}", crate::tasks::reference_header(state, &refs)) };
        // A task that is both attached and tagged is described once, by its reference header.
        let tags: Vec<Value> = meta["tags"].as_array().into_iter().flatten().filter(|t| !t["task_id"].as_str().is_some_and(|id| refs.iter().any(|r| r == id))).cloned().collect();
        match crate::tags::block(&Value::Array(tags)) {
            b if b.is_empty() => p,
            b => format!("{p}\n\n{b}"),
        }
    });
    // The user's own message, or a schedule (or "Run now"): its reply is meant for them.
    let for_user = matches!(trigger, "schedule" | "manual") || (text.is_some() && meta["from_agent"].is_null());
    let msg = text.as_ref().and_then(|t| add_msg(state, agent_id, None, "user", "text", t, meta));
    start(state, agent_id, prompt, trigger, for_user);
    msg
}

/// The Fresh context choices, the default first.
pub const FRESH_CONTEXT: &[&str] = &["above_80k", "above_100k", "above_120k", "above_150k", "above_180k", "above_200k", "off", "always"];

/// The context size the clients warn at (out of the smart zone).
pub const CONTEXT_WARN_AT: u64 = 80_000;

/// The context size an "above_Nk" setting clears from: N thousand tokens (80k for anything unknown).
fn fresh_threshold(setting: &str) -> u64 {
    setting.strip_prefix("above_").and_then(|n| n.strip_suffix('k')).and_then(|n| n.parse::<u64>().ok()).map_or(CONTEXT_WARN_AT, |n| n * 1000)
}

/// What belongs to one Claude session: dropped together whenever the next run starts a fresh one.
fn reset_session_fields(a: &mut Agent) {
    a.session_id = None;
    a.context_tokens = None;
    a.grill_briefed = false;
    a.memory_index = None;
}

/// Drop the Claude session so the next run starts with empty context, leaving `text` as the `clear` line in
/// the chat. Long-term memory carries over, and the old session stays in `/resume`.
pub fn clear_context(state: &AppState, agent_id: &str, text: &str, meta: Value) -> Option<Agent> {
    let a = state.db.update_agent(agent_id, reset_session_fields).ok()??;
    if let Ok(m) = state.db.add_message(agent_id, None, "system", "clear", text, meta) {
        state.emit(crate::state::Event::Message { message: json!(m) });
    }
    emit_agent(state, &a);
    Some(a)
}

/// The ticket boundary, shared by a Delivery wake and an agent's message naming a ticket: records that the
/// agent now works on `ticket` (see `Agent::delivery_ticket`) and says whether its Fresh context setting
/// applies before the run. Only on a ticket it hasn't worked on yet, so review rounds keep their context,
/// and for a Lead only as the agent building the tickets (`builder`): otherwise it coordinates, and is
/// never cleared.
fn ticket_boundary(state: &AppState, agent_id: &str, ticket: &str, builder: bool) -> bool {
    let Ok(Some(a)) = state.db.get_agent(agent_id) else { return false };
    if a.delivery_ticket.as_deref() == Some(ticket) {
        return false;
    }
    let _ = state.db.update_agent(agent_id, |a| a.delivery_ticket = Some(ticket.to_string()));
    builder || a.role != "lead"
}

/// A Delivery wake on `ticket`: at a ticket boundary, the Fresh context setting applies just before the
/// run, behind a running one if needed.
/// `wake` is the step it's for: when the run that takes it ends, Loft checks the step moved on.
pub fn wake_on_ticket(state: &AppState, agent_id: &str, ticket: &str, builder: bool, prompt: String, wake: crate::delivery::Wake) {
    let fresh = ticket_boundary(state, agent_id, ticket, builder);
    start_with(state, agent_id, Some(prompt), "delivery", false, fresh, Some(wake), None, false);
}

/// A Resume wake on `ticket`, the one its Delivery stalled on: the agent carries on with it, so it's never a
/// ticket boundary, whatever the agent worked on meanwhile.
pub fn resume_on_ticket(state: &AppState, agent_id: &str, ticket: &str, prompt: String, wake: crate::delivery::Wake) {
    let _ = state.db.update_agent(agent_id, |a| a.delivery_ticket = Some(ticket.to_string()));
    start_with(state, agent_id, Some(prompt), "delivery", false, false, Some(wake), None, false);
}

/// How `Agent::delivery_ticket` names the ticket an agent's message is about, the way Delivery names it:
/// `owner/repo#N` for a GitHub issue given as `N`, `#N`, `owner/repo#N` or its link (a bare number is on
/// `repo`, the recipient's project's repo, when it has one), and a Loft issue id as it is. None when blank.
/// A pull request is refused, since its number isn't its ticket's, and so is anything else with a `#`
/// (like "PR #183 (#174)"): Loft issue ids have none.
pub fn message_ticket_key(raw: &str, repo: Option<&str>) -> Result<Option<String>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let refuse = || Err(format!("Not sent: ticket {raw:?} isn't a ticket's issue. Name the issue itself (#N, owner/repo#N or its issue link), not its pull request: a PR's number isn't its ticket's."));
    let num = |s: &str| s.parse::<u64>().ok();
    let link = raw.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
    if let Some(path) = link.strip_prefix("github.com/") {
        let parts: Vec<&str> = path.split(['/', '#', '?']).collect();
        return match parts[..] {
            [owner, name, "issues", n, ..] if num(n).is_some() => Ok(Some(format!("{owner}/{name}#{n}"))),
            _ => refuse(),
        };
    }
    let (left, n) = raw.rsplit_once('#').unwrap_or(("", raw));
    match (left.trim(), num(n.trim())) {
        ("", Some(n)) => Ok(Some(repo.map_or(format!("#{n}"), |r| format!("{r}#{n}")))),
        (r, Some(n)) if r.contains('/') && !r.contains(char::is_whitespace) => Ok(Some(format!("{r}#{n}"))),
        _ if raw.contains(['#', '/']) => refuse(),
        _ => Ok(Some(raw.to_string())),
    }
}

/// The start of a run: if its prompt starts a new ticket (`ticket_boundary`), apply the agent's Fresh
/// context setting now, between runs.
pub(crate) fn fresh_boundary(state: &AppState, agent_id: &str) {
    if state.rt.lock().unwrap().fresh_before.remove(agent_id) {
        fresh_if_due(state, agent_id);
    }
}

/// At a ticket boundary: clear the agent's context if its setting says so. Off never, Always whenever it
/// has a session, and Above Nk (Above 80k is the default) when its context is at N thousand tokens or above.
fn fresh_if_due(state: &AppState, agent_id: &str) {
    let Ok(Some(a)) = state.db.get_agent(agent_id) else { return };
    let due = a.session_id.is_some()
        && match a.fresh_context.as_str() {
            "off" => false,
            "always" => true,
            setting => a.context_tokens.is_some_and(|n| n >= fresh_threshold(setting)),
        };
    if due {
        clear_context(state, agent_id, "Context cleared for a new ticket. Long-term memory carries over.", json!({ "fresh_context": a.fresh_context }));
    }
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

/// A Delivery done wake: not the user's words, but its reply is news for them, so it's an Unread reply.
pub fn wake_with_reply(state: &AppState, agent_id: &str, prompt: String) {
    start(state, agent_id, Some(prompt), "delivery", true);
}

/// Have a new agent draw its own sticker from `brief`, in a quiet run: only `set_avatar`'s result shows.
pub fn design_avatar(state: &AppState, agent_id: &str, brief: &str) {
    let prompt = format!(
        "[Loft: you were just created] This is not a message from the user. Before any other work, design your sticker avatar.\nBrief: {brief}\n\nRead your name, role, personality and standing instructions as well: the sticker should say who you are and what you do at a glance. Follow the avatar style in your instructions and call `mcp__loft__set_avatar`; if it is rejected, fix the SVG and call it again. Do nothing else in this run.",
        brief = brief.trim()
    );
    start(state, agent_id, Some(prompt), "avatar", false);
}

/// Redraw: have the agent draw a new sticker that fits who it is, in a quiet run on a throwaway session, so its
/// own Session (chat and context) stays as it was. On a busy agent it waits for the rest of the queue.
pub fn redraw_avatar(state: &AppState, agent_id: &str) {
    start_with(state, agent_id, Some(REDRAW_PROMPT.into()), "avatar", false, false, None, None, true);
}

const REDRAW_PROMPT: &str = "[Loft: redraw your sticker] This is not a message from the user. The user asked you to draw a new sticker avatar.\n\nRead your role, personality and standing instructions, then your main project's README and top-level files: the sticker should say who you are and what you do at a glance. Follow the avatar style in your instructions and call `mcp__loft__set_avatar`; if it is rejected, fix the SVG and call it again. Do nothing else in this run.";

/// `add_msg` for a run's output; quiet (grill) runs keep only errors. Their run still counts in the All-time spend.
#[allow(clippy::too_many_arguments)]
fn run_msg(quiet: bool, state: &AppState, agent_id: &str, rid: Option<&str>, role: &str, kind: &str, content: &str, meta: Value) -> Option<Message> {
    if quiet && kind != "error" {
        if kind == "run_end"
            && let Err(e) = state.db.add_unsaved_run_end(&meta) {
                tracing::error!("all-time spend: {e}");
            }
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
#[cfg(test)]
pub fn message_from_agent(state: &AppState, from: &Agent, to: &Agent, text: &str) {
    message_from_agent_with(state, from, to, text, json!({}));
}

/// `message_from_agent`, with extra `meta` on the stored message (e.g. which task it is about).
pub fn message_from_agent_with(state: &AppState, from: &Agent, to: &Agent, text: &str, meta: Value) {
    message_on_ticket(state, from, to, text, meta, None);
}

/// `message_from_agent_with` about `ticket` (a `message_ticket_key`), if it names one: at a ticket boundary,
/// the recipient's Fresh context setting applies before the run, as on a Delivery wake. A Lead builds the
/// tickets itself when its team has no Dev. Returns the message's id when it waits for a run to end.
pub fn message_on_ticket(state: &AppState, from: &Agent, to: &Agent, text: &str, mut meta: Value, ticket: Option<&str>) -> Option<i64> {
    let fresh = ticket.is_some_and(|t| ticket_boundary(state, &to.id, t, crate::delivery::fresh_context_applies(state, to)));
    if let Some(t) = ticket {
        meta["ticket"] = json!(t);
    }
    meta["from_agent"] = json!({ "id": from.id, "name": from.name });
    let msg = add_msg(state, &to.id, None, "user", "text", text, meta).map(|m| m.id);
    let prompt = format!(
        "[Message from agent {name} (id {id}), not from the user]\n{text}\n\n[Your replies in this chat are seen by the user, not by {name}. To answer {name}, call `mcp__loft__message_agent` with agent \"{id}\".]",
        name = from.name,
        id = from.id,
    );
    start_with(state, &to.id, Some(prompt), "agent", false, fresh, None, msg, false);
    let queued = msg.filter(|id| state.rt.lock().unwrap().queued_messages.get(&to.id).is_some_and(|q| q.contains(id)));
    if queued.is_some() {
        emit_run(state, &to.id);
    }
    queued
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

/// The sender id of an FYI from Loft itself: no agent has it (agents get UUIDs, and the guide agent is `setup::LOFT_ID`,
/// "loft"), so the chat names it "Loft" instead of finding an agent.
pub const LOFT_SENDER: &str = "loft-delivery";

/// An FYI from Loft itself (a Delivery that stalled), shown as from "Loft": delivered like an agent's FYI, with
/// the agent's next run.
pub fn fyi_from_loft(state: &AppState, to: &Agent, text: &str) {
    add_msg(state, &to.id, None, "user", "text", text, json!({ "from_agent": { "id": LOFT_SENDER, "name": "Loft" }, "fyi": true }));
    let _ = state.db.add_pending_fyi(&to.id, &format!("[FYI from Loft, not from the user. It needs no action and no reply.]\n{text}"));
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
    start_with(state, agent_id, text, trigger, for_user, false, None, None, false);
}

/// `start`; `fresh`: the prompt starts a new Delivery ticket, so the run that delivers it first applies
/// the agent's Fresh context setting (`fresh_if_due`). `delivery`: the Delivery step the prompt wakes the
/// agent for, kept with the prompt (queued with it, then on the run that takes it). `msg`: the agent message
/// the prompt carries, shown as queued while it waits (`Runtime::queued_messages`). `throwaway`: a Redraw, run on
/// a session of its own (`redraw_avatar`).
#[allow(clippy::too_many_arguments)]
fn start_with(state: &AppState, agent_id: &str, text: Option<String>, trigger: &'static str, for_user: bool, fresh: bool, delivery: Option<crate::delivery::Wake>, msg: Option<i64>, throwaway: bool) {
    let prompt = text.unwrap_or_else(|| {
        format!(
            "[Autonomous check-in — {}] This is a scheduled wake-up, not a message from the user. Review your standing instructions and memory, then do whatever is most useful right now. If there is nothing worth doing, say so in one line.",
            Utc::now().format("%Y-%m-%d %H:%M UTC")
        )
    });
    let kill = {
        let mut rt = state.rt.lock().unwrap();
        if fresh {
            rt.fresh_before.insert(agent_id.to_string());
        }
        if rt.running.contains_key(agent_id) {
            if throwaway {
                // One drawing run at a time: a Redraw already running or queued covers this click.
                let queued = !rt.redrawing.contains(agent_id) && rt.redraw_queued.insert(agent_id.to_string());
                // It waits for the run as it is: background work the agent waits on isn't stopped for it.
                drop(rt);
                if queued {
                    // Clients hear it's queued (`redrawing`) before the run it waits for ends.
                    emit_run(state, agent_id);
                }
                return;
            } else if trigger == "grill" {
                rt.grill_queued.entry(agent_id.to_string()).or_default().push(prompt);
            } else {
                rt.queued.entry(agent_id.to_string()).or_default().push(prompt.clone());
                if for_user {
                    rt.queued_for_user.insert(agent_id.to_string());
                }
                if let Some(w) = delivery {
                    rt.delivery_queued.entry(agent_id.to_string()).or_default().push((w, Some(prompt)));
                }
                if let Some(id) = msg {
                    rt.queued_messages.entry(agent_id.to_string()).or_default().push(id);
                }
            }
            if let Some(w) = rt.waiting.get(agent_id) {
                w.notify_one();
            }
            return;
        }
        let kill = Arc::new(Notify::new());
        rt.running.insert(agent_id.to_string(), kill.clone());
        if throwaway {
            rt.redrawing.insert(agent_id.to_string());
        }
        if let Some(w) = delivery {
            rt.delivery_running.entry(agent_id.to_string()).or_default().push(w);
        }
        kill
    };
    let state = state.clone();
    let agent_id = agent_id.to_string();
    tokio::spawn(async move {
        let mut prompt = prompt;
        let mut trigger = trigger;
        let mut for_user = for_user;
        let mut throwaway = throwaway;
        loop {
            // Only between runs: a ticket boundary queued behind a run waits for it to end. A Redraw leaves the Session alone.
            if !throwaway {
                fresh_boundary(&state, &agent_id);
            }
            if trigger == "grill" {
                prompt = crate::grill::with_brief(&state, &agent_id, prompt);
            }
            // `/compact` must stay the whole prompt, and the avatar run does nothing else: FYIs wait for a real run.
            let fyis = if trigger != "compact" && trigger != "avatar" { state.db.take_pending_fyis(&agent_id).unwrap_or_default() } else { vec![] };
            let full = with_fyis(&fyis, &prompt);
            let mut delivered = false;
            let failed = match run_once(&state, &agent_id, &full, trigger, for_user, throwaway, kill.clone(), &mut delivered).await {
                Ok(failed) => failed,
                Err(e) => {
                    // Claude never got the prompt, so the FYIs never reached it: keep them for the next run.
                    if !delivered {
                        let _ = state.db.restore_pending_fyis(&agent_id, &fyis);
                    }
                    add_msg(&state, &agent_id, None, "system", "error", &e.to_string(), json!({}));
                    set_status(&state, &agent_id, "error", &e.to_string());
                    if for_user {
                        mark_replied(&state, &agent_id);
                    }
                    true
                }
            };
            crate::delivery::run_ended(&state, &agent_id, failed).await;
            let (next, push) = {
                let mut rt = state.rt.lock().unwrap();
                let drew = rt.redrawing.remove(&agent_id);
                // The run that takes the queue delivers its agent messages.
                let delivered = rt.queued_messages.remove(&agent_id).is_some();
                let next = match (rt.queued.remove(&agent_id), rt.grill_queued.remove(&agent_id)) {
                    (Some(q), g) if !q.is_empty() => {
                        // User work first; grill follow-ups wait for the run after it.
                        if let Some(g) = g {
                            rt.grill_queued.insert(agent_id.clone(), g);
                        }
                        let u = rt.queued_for_user.remove(&agent_id);
                        take_delivery_queue(&mut rt, &agent_id);
                        Some((q.join("\n\n"), "user", u, false))
                    }
                    (_, Some(g)) if !g.is_empty() => Some((g.join("\n\n"), "grill", false, false)),
                    // A Redraw waits for everything else.
                    _ if rt.redraw_queued.remove(&agent_id) => {
                        rt.redrawing.insert(agent_id.clone());
                        Some((REDRAW_PROMPT.to_string(), "avatar", false, true))
                    }
                    _ => {
                        // Saved before the run stops counting as running, so a restart keeps the idle time.
                        let _ = state.db.update_agent(&agent_id, |a| a.last_run_ended_at = Some(chrono::Utc::now().to_rfc3339()));
                        rt.running.remove(&agent_id);
                        None
                    }
                };
                let push = delivered || (drew && next.is_some());
                (next, push)
            };
            // Delivered agent messages leave `queued_messages`; a Redraw that ended with more work queued after it
            // turns `redrawing` false now, not when the next run ends (with nothing queued, the event after the loop says it).
            if push {
                emit_run(&state, &agent_id);
            }
            match next {
                Some((p, t, u, w)) => {
                    prompt = p;
                    trigger = t;
                    for_user = u;
                    throwaway = w;
                }
                None => break,
            }
        }
        if let Ok(Some(a)) = state.db.get_agent(&agent_id) {
            emit_agent(&state, &a);
        }
    });
}

/// The run that takes the agent's queue takes the Delivery steps it was woken for.
pub(crate) fn take_delivery_queue(rt: &mut crate::state::Runtime, agent_id: &str) {
    if let Some(w) = rt.delivery_queued.remove(agent_id) {
        rt.delivery_running.entry(agent_id.to_string()).or_default().extend(w.into_iter().map(|(w, _)| w));
    }
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

/// Where claude reads one of an agent's files from: its system prompt (`md`), MCP config (`mcp.json`), settings
/// (`settings.json`) and the script each Bash command runs first (`env.sh`, see processes.rs). None of them is on claude's command line: every agent's prompt holds phrases like
/// `vite --host` and its MCP config the user's servers (`npx @playwright/mcp`), so a `pkill -f` with one of them in
/// any agent's command would match, and stop, every agent at once.
fn agent_file(state: &AppState, agent_id: &str, kind: &str) -> PathBuf {
    state.cfg.storage_dir.join(AGENT_FILES).join(format!("{agent_id}.{kind}"))
}

pub(crate) const AGENT_FILES: &str = ".agent-files";
const AGENT_FILE_KINDS: [&str; 4] = ["md", "mcp.json", "settings.json", "env.sh"];
/// Where the agents' files were kept before they held more than the prompt.
const OLD_AGENT_FILES: &str = ".system-prompts";

/// Writes one of the agent's files owner-only (it may hold MCP tokens) and returns the path claude reads it at
/// from the workspace, `../.agent-files/<id>.<kind>`: an absolute one would put the user's folders on argv.
fn write_agent_file(state: &AppState, agent_id: &str, kind: &str, text: &str) -> Result<PathBuf> {
    let path = agent_file(state, agent_id, kind);
    write_private(&path, text)?;
    Ok(PathBuf::from("..").join(AGENT_FILES).join(path.file_name().unwrap()))
}

/// Writes `text` to `path` readable by the user only, in a folder only they can open.
pub fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    std::fs::write(path, text)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Deletes the folder older Loft builds kept the agents' files in, with any MCP tokens they left there.
pub fn remove_old_agent_files(storage_dir: &Path) {
    let _ = std::fs::remove_dir_all(storage_dir.join(OLD_AGENT_FILES));
}

/// Removes the files claude was started with, when the agent is deleted.
pub fn remove_agent_files(state: &AppState, agent_id: &str) {
    for kind in AGENT_FILE_KINDS {
        let _ = std::fs::remove_file(agent_file(state, agent_id, kind));
    }
}

/// This binary, which claude runs as the Bash hook and as the agent's `kill`, `pkill` and `killall`. Tests run as
/// their own binary, so they use the loft binary cargo builds next to it.
fn loft_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if cfg!(test) {
        let next_to_tests = exe.parent().and_then(Path::parent).map(|d| d.join("loft"));
        if let Some(bin) = next_to_tests.filter(|b| b.exists()) {
            return Ok(bin);
        }
        // With `CARGO_BUILD_BUILD_DIR` the test binary is built elsewhere; the loft binary stays in the target dir.
        let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target"));
        return Ok(target.join("debug/loft"));
    }
    Ok(exe)
}

/// The claude command for a run. Writing its files fails the run rather than putting them on argv.
fn build_command(state: &AppState, a: &Agent, resume: bool) -> Result<Command> {
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

    // `Kill(<path>)` rules are Loft's own (see processes.rs), not claude tools.
    let mut allowed: Vec<String> = a.allowed_tools.iter().filter(|r| !r.starts_with("Kill(")).cloned().collect();
    allowed.push("mcp__loft".into());
    let mut disallowed = a.disallowed_tools.clone();
    if a.max_subagents == Some(0) {
        disallowed.push("Agent".into());
    }
    // The tool lists hold the user's own rules (`Bash(npm *)`), so they go in the settings file too; `--settings`
    // applies despite `--setting-sources ""`. So does the hook that refuses kills Loft can't scope to the agent's
    // own processes (`/bin/kill`, `sudo kill`, a `kill` in `sh -c`).
    let exe = loft_exe()?;
    let settings = json!({
        "permissions": { "allow": allowed, "deny": disallowed },
        "hooks": crate::kill_guard::hooks(&exe),
    });

    let ports = crate::ports::range(state, &a.id)?;
    let mut cmd = Command::new(state.settings.get().claude_bin);
    cmd.current_dir(workspace(state, &a.id))
        .arg("-p")
        .args(["--output-format", "stream-json", "--verbose"])
        .arg("--append-system-prompt-file")
        .arg(write_agent_file(state, &a.id, "md", &system_prompt(state, a))?)
        .args(["--system-prompt-snapshot", "off"])
        .arg("--mcp-config")
        .arg(write_agent_file(state, &a.id, "mcp.json", &mcp_config)?)
        .arg("--strict-mcp-config")
        // Isolate agents from the user's own Claude Code settings, hooks and plugins.
        .args(["--setting-sources", ""])
        .arg("--settings")
        .arg(write_agent_file(state, &a.id, "settings.json", &settings.to_string())?)
        .args(["--permission-prompt-tool", "mcp__loft__approval_prompt"])
        .args(["--permission-mode", &a.permission_mode])
        .args(["--name", &a.name])
        .env("MCP_TOOL_TIMEOUT", TOOL_WAIT.as_millis().to_string())
        // The kill of a process the agent doesn't own waits that long for the user, inside a Bash call.
        .env("BASH_MAX_TIMEOUT_MS", TOOL_WAIT.as_millis().to_string())
        .env("CLAUDE_ENV_FILE", agent_file(state, &a.id, "env.sh"))
        .env(crate::processes::URL_VAR, format!("http://127.0.0.1:{}/kill/{}", state.cfg.port, a.id))
        .env(crate::kill_guard::URL_VAR, format!("http://127.0.0.1:{}/guard/{}", state.cfg.port, a.id))
        .env("CLAUDE_CODE_ENTRYPOINT", "loft")
        // The agent's port range, for its dev servers: `PORT` (which many servers read) is its first port.
        .env("PORT", ports.start().to_string())
        .env(crate::ports::RANGE_VAR, crate::ports::describe(&ports))
        // Cargo keeps compiled dependencies here, shared by all the agent's clones; binaries stay in each clone's `target/`.
        .env("CARGO_BUILD_BUILD_DIR", workspace(state, &a.id).join(".cargo-build"))
        // Vite rejects requests for hostnames it doesn't know; let dev servers answer on the tailnet name.
        .env("__VITE_ADDITIONAL_SERVER_ALLOWED_HOSTS", crate::tailnet::host().unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Its own Simulator device, once it has one (an earlier run asked for it).
    match state.db.agent_device(&a.id)? {
        Some(udid) => cmd.env(crate::simulator::DEVICE_VAR, udid),
        None => cmd.env_remove(crate::simulator::DEVICE_VAR),
    };
    // Run in each Bash command's shell: records its process session as the agent's, and sends kills through Loft.
    let storage = &state.cfg.storage_dir;
    write_agent_file(state, &a.id, "env.sh", &crate::processes::env_script(storage, &a.id, &exe))?;
    crate::processes::write_shims(storage, &exe)?;
    // Its own process session: Loft knows the run's processes by it, and a group kill (`kill 0`) in the agent's
    // commands can't reach Loft.
    unsafe {
        cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) });
    }
    if !a.model.is_empty() {
        cmd.args(["--model", &a.model]);
    }
    if !a.effort.is_empty() {
        cmd.args(["--effort", &a.effort]);
    }
    if !a.subagent_model.is_empty() {
        // FORCE makes the built-in Explore and Plan subagents follow it too.
        cmd.env("CLAUDE_CODE_SUBAGENT_MODEL", &a.subagent_model).env("CLAUDE_CODE_SUBAGENT_MODEL_FORCE", "1");
    }
    if let Some(n @ 1..) = a.max_subagents {
        cmd.env("CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS", n.to_string());
    }
    if resume
        && let Some(sid) = &a.session_id {
            cmd.args(["--resume", sid]);
        }
    Ok(cmd)
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
/// Whether the run failed: claude reported an error, or exited without a result when nobody stopped it.
/// `throwaway`: run on a new session that isn't saved, leaving the agent's own Session as it was.
#[allow(clippy::too_many_arguments)]
async fn run_once(state: &AppState, agent_id: &str, prompt: &str, trigger: &str, for_user: bool, throwaway: bool, kill: Arc<Notify>, delivered: &mut bool) -> Result<bool> {
    let mut resume = !throwaway;
    loop {
        match run_process(state, agent_id, prompt, trigger, for_user, throwaway, kill.clone(), resume, delivered).await? {
            RunOutcome::Done { failed } => return Ok(failed),
            RunOutcome::StaleSession if trigger == "compact" => anyhow::bail!("Nothing to compact: the session is gone."),
            RunOutcome::StaleSession if resume => {
                *delivered = false;
                // The stored Claude session is gone; start fresh (memory survives).
                if let Ok(Some(a)) = state.db.update_agent(agent_id, reset_session_fields) {
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
    Done { failed: bool },
    StaleSession,
}

#[allow(clippy::too_many_arguments)]
async fn run_process(
    state: &AppState,
    agent_id: &str,
    prompt: &str,
    trigger: &str,
    for_user: bool,
    throwaway: bool,
    kill: Arc<Notify>,
    resume: bool,
    delivered: &mut bool,
) -> Result<RunOutcome> {
    let mut agent = state.db.get_agent(agent_id)?.ok_or_else(|| anyhow::anyhow!("agent not found"))?;
    // A throwaway run leaves the frozen memories to the agent's Session (its prompt builds them if there are none).
    if !throwaway && (!resume || agent.session_id.is_none() || agent.memory_index.is_none()) {
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

    let image_line = crate::uploads::stdin_line(state, agent_id, prompt).await;
    let mut cmd = build_command(state, &agent, resume)?;
    if throwaway {
        // Not saved, so it isn't listed among the agent's past Sessions either.
        cmd.arg("--no-session-persistence");
    }
    if image_line.is_some() {
        cmd.args(["--input-format", "stream-json"]);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to start `{}`: {e}", state.settings.get().claude_bin))?;
    // Its session id is its PID.
    if let Some(pid) = child.id()
        && let Err(e) = crate::processes::record(&state.cfg.storage_dir, agent_id, pid) {
            tracing::warn!("agent {agent_id}: could not record its process session: {e}");
        }
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
    // Loft sent claude a SIGTERM itself (stop, new work over background tasks, or a claude that wouldn't exit).
    let mut loft_stopped = false;
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
                tracing::info!("agent {agent_id}: stopping run {run_id}: stop or restart asked");
                terminate(&mut child).await;
                loft_stopped = true;
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
                tracing::info!("agent {agent_id}: stopping run {run_id}: new messages arrived while only background work was left");
                terminate(&mut child).await;
                loft_stopped = true;
                None
            }
            _ = async { tokio::time::sleep_until(exit_by.unwrap()).await }, if exit_by.is_some() => {
                // Replied with nothing left in the background, yet still alive: don't hold the queue for it.
                tracing::info!("agent {agent_id}: stopping run {run_id}: claude didn't exit after its reply");
                terminate(&mut child).await;
                loft_stopped = true;
                None
            }
        };
        let Some(line) = line else { break };
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        got_any = true;
        {
            let mut rt = state.rt.lock().unwrap();
            if let Some(i) = rt.runs.get_mut(agent_id) {
                i.last_output_at = now();
                // Tool calls still waiting on their result: a long build in the foreground is quiet, not stuck.
                for b in ev["message"]["content"].as_array().into_iter().flatten() {
                    match (ev["type"].as_str(), b["type"].as_str()) {
                        (Some("assistant"), Some("tool_use")) => i.tools_open.extend(b["id"].as_str().map(String::from)),
                        (Some("user"), Some("tool_result")) => i.tools_open.retain(|t| Some(t.as_str()) != b["tool_use_id"].as_str()),
                        _ => {}
                    }
                }
            }
            // A finished turn is real progress: automatic restarts start over. (Any output isn't: a restarted
            // run always says something, and one that then hangs the same way must still reach the limit.)
            if ev["type"] == "result" && ev["num_turns"].as_u64() != Some(0) {
                rt.auto_restarts.remove(agent_id);
            }
        }
        let sub = ev["parent_tool_use_id"].as_str().map(|s| s.to_string());
        match ev["type"].as_str() {
            Some("system") => match ev["subtype"].as_str() {
                Some("init") => {
                    if let Some(sid) = ev["session_id"].as_str().filter(|_| !throwaway) {
                        set_session(state, agent_id, sid)?;
                    }
                }
                // Manual /compact and Claude Code's own auto-compaction both land here.
                Some("compact_boundary") if !throwaway => {
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
                if sub.is_none() && u.is_object() && !throwaway {
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
                            // The command shows in its tool row only; the status line keeps the agent's own words.
                            if name.starts_with("mcp__loft__grill_") {
                                set_status_text(state, agent_id, "Preparing the next question…");
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
                        if let Some((name, input)) = tool_calls.get(&use_id)
                            && ok && crate::prs::creates_pr(name, input) {
                                for (slug, n) in crate::github::extract_pr_links(&full) {
                                    let (state, agent_id, use_id) = (state.clone(), agent_id.to_string(), use_id.clone());
                                    tokio::spawn(async move {
                                        let _ = crate::prs::register(&state, &agent_id, &slug, n, Some(use_id)).await;
                                    });
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
                if throwaway {
                    // A session of its own: all of it is this run's, and only the agent's total cost moves.
                    state.db.update_agent(agent_id, |a| a.total_cost_usd += total)?;
                } else if let Some(sid) = ev["session_id"].as_str() {
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
            tracing::info!("agent {agent_id}: stopping run {run_id}: claude didn't exit after its output ended");
            terminate(&mut child).await;
            loft_stopped = true;
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
            // A SIGTERM Loft didn't send came from another process. Not SIGKILL: macOS ends a process out of memory with it.
            use std::os::unix::process::ExitStatusExt;
            let outside = if !loft_stopped && (status.signal() == Some(libc::SIGTERM) || status.code() == Some(143)) {
                " It was stopped by a signal from outside Loft, for example a `pkill` or `kill` in some agent's command."
            } else {
                ""
            };
            tracing::warn!("agent {agent_id}: run {run_id}: claude exited ({status}) without a result{outside}");
            let text = format!("claude exited ({status}) without a result.{outside}\n{tail}");
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
        let what = match restarted {
            Some("user") => "Restarted by user.",
            Some(_) => "Restarted by Loft: no output for a while.",
            None => "Stopped by user.",
        };
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
    Ok(RunOutcome::Done { failed: run_error.is_some() })
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
        assert!(p.contains("Start servers with `nohup … &`, never with a daemon option (`--daemonize`, `--detach`, `pg_ctl start`)."));
        assert!(!p.contains("start them in the background"));
    }

    #[test]
    fn the_identity_line_names_the_role_label() {
        let st = crate::tasks::tests::state();
        let mut a = crate::tasks::tests::agent(&st, "Checker");
        a.role = "code_reviewer".into();
        assert!(system_prompt(&st, &a).contains("# Identity\nYou are **Checker** (Code reviewer), an autonomous agent"));
        a.role = "custom".into();
        assert!(system_prompt(&st, &a).contains("You are **Checker** (Custom), an autonomous agent"));
        a.role = String::new();
        assert!(system_prompt(&st, &a).contains("You are **Checker** (Custom), an autonomous agent"), "no role yet reads as Custom");
    }

    #[test]
    fn prompt_keeps_the_clone_rule_and_asks_to_reuse_one_clone_per_project() {
        let st = crate::tasks::tests::state();
        let p = system_prompt(&st, &crate::tasks::tests::agent(&st, "Worker"));
        assert!(p.contains("Never edit them in place. To work on one, clone it into your workspace (`git clone "));
        assert!(p.contains("or clone from its remote) and work on the clone."));
        for key in ["reuse it for every ticket", "pull/<N>/head", "--detach", "Never clone a project again", "if it's gone (Loft deletes a clone when everything in it is pushed), clone the project again first"] {
            assert!(p.contains(key), "{key}");
        }
    }

    #[test]
    fn prompt_splits_the_workspace_into_memory_notes_and_scratch() {
        let st = crate::tasks::tests::state();
        let p = system_prompt(&st, &crate::tasks::tests::agent(&st, "Worker"));
        for key in ["Knowledge (decisions, how things work, gotchas worth keeping) goes in your memory", "Reports (QA or security write-ups, plans, audits) go in `notes/`, kept small", "Debug and test output", "`scratch/<ticket number>/`", "Loft deletes a ticket's folder once the ticket is done"] {
            assert!(p.contains(key), "{key}");
        }
        assert!(!p.contains("a `notes/` folder for longer documents is a good idea"));
    }

    #[test]
    fn prompt_asks_to_fetch_and_branch_from_the_remote_base() {
        let st = crate::tasks::tests::state();
        let p = system_prompt(&st, &crate::tasks::tests::agent(&st, "Worker"));
        for key in ["`git fetch origin`", "`git switch -c <branch> origin/<base>`", "`origin/HEAD`", "`git pull --ff-only`"] {
            assert!(p.contains(key), "{key}");
        }
        // Before handing work in: merge the base (never rebase), and fetch from GitHub, not a local checkout.
        for key in ["`git merge origin/<base>`", "merge, not rebase", "re-run the tests", "`git remote set-url origin <the project's GitHub URL>`"] {
            assert!(p.contains(key), "{key}");
        }
        // After the merge: prune, then a safe delete only.
        for key in ["`git fetch --prune origin`", "`git branch -d <branch>`", "`git worktree remove <path>` without `--force`"] {
            assert!(p.contains(key), "{key}");
        }
        assert!(!p.contains("git branch -D") && !p.contains("git worktree remove --force"));
    }

    #[test]
    fn the_kill_timeout_the_prompt_names_is_the_one_claude_allows() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let p = system_prompt(&st, &a);
        let named = p.split("give that Bash call a `timeout` of ").nth(1).and_then(|r| r.split(':').next()).expect("a timeout in # Runs");
        let cmd = build_command(&st, &a, false).unwrap();
        let env = |k: &str| cmd.as_std().get_envs().find(|(n, _)| *n == k).and_then(|(_, v)| v).map(|v| v.to_string_lossy().into_owned());
        assert_eq!(env("BASH_MAX_TIMEOUT_MS").as_deref(), Some(named));
        assert_eq!(env("MCP_TOOL_TIMEOUT").as_deref(), Some(named));
        assert!(TOOL_WAIT > PERMISSION_TIMEOUT);
    }

    #[test]
    fn ios_builds_use_xcodes_derived_data_not_the_workspace() {
        let st = crate::tasks::tests::state();
        let p = system_prompt(&st, &crate::tasks::tests::agent(&st, "Worker"));
        assert!(p.contains("-showBuildSettings") && p.contains("-IDEBuildLocationStyle=Unique") && p.contains("$BUILT_PRODUCTS_DIR/$FULL_PRODUCT_NAME"));
        assert!(!p.contains("-derivedDataPath build"));
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
    fn the_prompt_names_the_agent_s_group_and_every_project_main_first() {
        let st = crate::tasks::tests::state();
        let mut a = crate::tasks::tests::agent(&st, "Web");
        (a.group, a.project, a.projects) = ("Acme".into(), "b2c".into(), vec!["b2c".into(), "b2b".into(), "apps".into()]);
        let p = system_prompt(&st, &a);
        assert!(p.contains("- You belong to the group **Acme**."), "{p}");
        assert!(p.contains("- Your main project is `b2c`: the issues you create go there unless you pass another `project`. You also work in `b2b`, `apps`. You can only create issues in your own projects; when you plan (grill, spec, tickets), say which one with `project`."), "{p}");
        (a.group, a.project, a.projects) = (String::new(), String::new(), vec![]);
        let p = system_prompt(&st, &a);
        assert!(!p.contains("You belong to the group") && !p.contains("Your main project"));
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
                "6. When a needs-action message hands over a ticket or is about one, name it in `ticket`, as the issue, not its pull request: the recipient's context may be cleared between tickets, never within one.",
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
    /// A Redraw costs 0.25 on session "s-redraw" and waits for a `go` file before it replies.
    /// "bash-cmd" runs `cmd.sh` from the workspace the way Claude Code runs a Bash command, in a process session of
    /// its own after `CLAUDE_ENV_FILE`, and appends what it printed and its exit status to `bash.out`.
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
  case "$1" in --append-system-prompt|--allowedTools|--disallowedTools) echo "$1 on argv" >&2; exit 1;; esac
  [ "$1" = --mcp-config ] && [ ! -f "$2" ] && echo 'mcp config on argv' >&2 && exit 1
  [ "$1" = --append-system-prompt-file ] && { cat "$2"; printf '\n---\n'; } >> system.log
  [ "$1" = --settings ] && cat "$2" > settings.log
  printf '%s\n' "$1" >> argv.log
  [ "$1" = --resume ] && echo "$2" >> resumed.log
  shift
done
printf '%s %s\n' "$PORT" "$LOFT_PORT_RANGE" >> ports.log
printf '%s\n' "${LOFT_SIMULATOR_UDID-none}" >> device.log
printf '%s\n' "${CARGO_BUILD_BUILD_DIR-none}" >> cargo-build.log
cost=$(printf '%s' "$prompt" | sed -n 's/.*cost=\([0-9.]*\).*/\1/p')
sid=$(printf '%s' "$prompt" | sed -n 's/.*sid=\([a-z0-9-]*\).*/\1/p')
case "$prompt" in *"redraw your sticker"*) cost=0.25 sid=s-redraw;; esac
: "${cost:=0}" "${sid:=s1}"
echo '{"type":"system","subtype":"init","session_id":"'"$sid"'"}'
case "$prompt" in *"redraw your sticker"*) while [ ! -f go ]; do sleep 0.05; done;; esac
case "$prompt" in /compact*)
  echo '{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual","pre_tokens":9000,"post_tokens":900}}';;
esac
case "$prompt" in *crash*) exit 1;; esac
case "$prompt" in *sigterm*) kill -TERM $$;; esac
case "$prompt" in *exit-143*) exit 143;; esac
case "$prompt" in *sigkill*) kill -KILL $$;; esac
case "$prompt" in *exit-137*) exit 137;; esac
case "$prompt" in *stuck-pipes*) exec >&-; sleep 3; exit 0;; esac
case "$prompt" in *garbled*) printf '\377\n'; exit 0;; esac
case "$prompt" in *chatty*)
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"On it."}]}}';;
esac
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
case "$prompt" in *sharepr*)
  echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s1","name":"mcp__loft__share_pull_request","input":{"url":"https://github.com/octo/share-run/pull/84"}}]}}'
  echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s1","content":[{"type":"text","text":"The user can now see a card for https://github.com/octo/share-run/pull/84."}]}]}}';;
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
case "$prompt" in *tool-status*)
  echo '{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"plan"},{"type":"tool_use","id":"u1","name":"Bash","input":{"command":"cargo test","description":"Run the tests"}}]}}'
  while [ ! -f go ]; do sleep 0.05; done
  echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"u2","name":"Read","input":{"file_path":"/x/config.toml"}}]}}'
  while [ ! -f go2 ]; do sleep 0.05; done
  echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"u3","name":"mcp__loft__grill_round","input":{}}]}}'
  while [ ! -f go3 ]; do sleep 0.05; done;;
esac
case "$prompt" in *bg-agent*) ;; *linger*)
  echo '{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b1","description":"Run the QA suite"}]}';;
esac
case "$prompt" in *bash-cmd*)
  perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV' /bin/zsh -c '. "$CLAUDE_ENV_FILE"; . ./cmd.sh' >> bash.out 2>&1
  echo "exit $?" >> bash.out;;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn the_system_prompt_is_read_from_a_file_not_claude_s_command_line() {
        let (st, a) = fake_claude_state();
        run(&st, &a.id, "hello", 1).await;
        // A `pkill -f "vite --host"` (or any phrase of the prompt) in some agent's command must not match claude.
        let argv = std::fs::read_to_string(workspace(&st, &a.id).join("argv.log")).unwrap();
        assert!(argv.contains("--append-system-prompt-file"), "{argv}");
        for phrase in ["vite --host", "# Identity", "Standing instructions"] {
            assert!(!argv.contains(phrase), "{phrase:?} is on claude's command line");
        }
        // Claude still gets the whole prompt, from an owner-only file.
        let system = std::fs::read_to_string(workspace(&st, &a.id).join("system.log")).unwrap();
        assert!(system.contains("# Identity") && system.contains("vite --host"), "{system}");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(agent_file(&st, &a.id, "md")).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(system.contains("Stop your own processes any way (`kill $pid`, `pkill -f vite`, `killall node`)"), "agents may stop their own processes any way");
        assert!(!system.contains("never with `pkill -f`"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_s_end_is_saved_so_the_idle_time_survives_a_restart() {
        let (st, a) = fake_claude_state();
        assert!(st.db.get_agent(&a.id).unwrap().unwrap().last_run_ended_at.is_none());
        run(&st, &a.id, "hello", 1).await;
        let saved = st.db.get_agent(&a.id).unwrap().unwrap();
        assert!(saved.last_run_ended_at.is_some());
        // Started long ago, ended just now: not idle, even with nothing in memory.
        st.db.update_agent(&a.id, |x| x.last_run_at = Some((chrono::Utc::now() - chrono::Duration::hours(9)).to_rfc3339())).unwrap();
        let saved = st.db.get_agent(&a.id).unwrap().unwrap();
        assert!(!super::idle(&st, &saved, std::time::SystemTime::now()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn claude_s_files_are_private_relative_and_hold_the_tool_lists_and_the_kill_guard() {
        let (st, mut a) = fake_claude_state();
        a = st.db.update_agent(&a.id, |a| {
            a.allowed_tools = vec!["Bash(npm *)".into()];
            a.disallowed_tools = vec!["WebFetch".into()];
            a.max_subagents = Some(0);
        }).unwrap().unwrap();
        run(&st, &a.id, "hello", 1).await;
        // No path into the user's folders, and none of the user's rules, for a `pkill -f` to match.
        let argv = std::fs::read_to_string(workspace(&st, &a.id).join("argv.log")).unwrap();
        let storage = st.cfg.storage_dir.to_string_lossy().to_string();
        for text in [storage.as_str(), "agent-local-storage", "npm", "WebFetch", "loft ·"] {
            assert!(!argv.contains(text), "{text:?} is on claude's command line:\n{argv}");
        }
        assert!(argv.contains(&format!("../.agent-files/{}.settings.json", a.id)), "{argv}");
        // The fake read the settings file from that relative path.
        let settings: Value = serde_json::from_str(&std::fs::read_to_string(workspace(&st, &a.id).join("settings.log")).unwrap()).unwrap();
        assert_eq!(settings["permissions"], json!({ "allow": ["Bash(npm *)", "mcp__loft"], "deny": ["WebFetch", "Agent"] }));
        let hook = &settings["hooks"]["PreToolUse"][0];
        assert_eq!(hook["matcher"], "Bash");
        assert!(hook["hooks"][0]["command"].as_str().unwrap().ends_with("' kill-guard"), "{settings}");
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        for kind in AGENT_FILE_KINDS {
            assert_eq!(mode(&agent_file(&st, &a.id, kind)), 0o600, "{kind}");
        }
        assert_eq!(mode(agent_file(&st, &a.id, "md").parent().unwrap()), 0o700);

        // Deleting the agent removes them, even when its workspace stays.
        let uri: axum::http::Uri = "/?purge_files=false".parse().unwrap();
        let opts = axum::extract::Query::try_from_uri(&uri).unwrap();
        let _ = crate::api::delete_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone()), opts).await.ok().unwrap();
        for kind in AGENT_FILE_KINDS {
            assert!(!agent_file(&st, &a.id, kind).exists(), "{kind} is left behind");
        }
        assert!(workspace(&st, &a.id).exists());
    }

    /// The `PORT` and `LOFT_PORT_RANGE` each of the agent's runs was started with, in order.
    fn ports_seen(st: &AppState, id: &str) -> Vec<String> {
        std::fs::read_to_string(workspace(st, id).join("ports.log")).unwrap_or_default().lines().map(String::from).collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_agent_serves_on_its_own_port_range_on_every_run_until_it_is_deleted() {
        let (st, a) = fake_claude_state();
        let b = crate::tasks::tests::agent(&st, "Other");
        run(&st, &a.id, "hello", 1).await;
        run(&st, &b.id, "hello", 1).await;
        run(&st, &a.id, "again", 2).await;
        assert_eq!(ports_seen(&st, &a.id), ["20000 20000-20009", "20000 20000-20009"], "the same range on every run");
        assert_eq!(ports_seen(&st, &b.id), ["20010 20010-20019"]);
        let system = std::fs::read_to_string(workspace(&st, &a.id).join("system.log")).unwrap();
        assert!(system.contains("only on your own ports, 20000–20009 (`$PORT` is the first, `$LOFT_PORT_RANGE` the whole range)"), "{system}");

        // Deleted through the API, its range goes to the next new agent.
        let uri: axum::http::Uri = "/?purge_files=false".parse().unwrap();
        let opts = axum::extract::Query::try_from_uri(&uri).unwrap();
        let _ = crate::api::delete_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone()), opts).await.ok().unwrap();
        let c = crate::tasks::tests::agent(&st, "Newcomer");
        run(&st, &c.id, "hello", 1).await;
        assert_eq!(ports_seen(&st, &c.id), ["20000 20000-20009"]);
        run(&st, &b.id, "again", 2).await;
        assert_eq!(ports_seen(&st, &b.id)[1], "20010 20010-20019");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_agent_builds_cargo_dependencies_in_its_own_workspace() {
        let (st, a) = fake_claude_state();
        let b = crate::tasks::tests::agent(&st, "Other");
        run(&st, &a.id, "hello", 1).await;
        run(&st, &b.id, "hello", 1).await;
        let seen = |id: &str| std::fs::read_to_string(workspace(&st, id).join("cargo-build.log")).unwrap().trim().to_string();
        assert_eq!(seen(&a.id), workspace(&st, &a.id).join(".cargo-build").to_string_lossy());
        assert_eq!(seen(&b.id), workspace(&st, &b.id).join(".cargo-build").to_string_lossy());
        assert_ne!(seen(&a.id), seen(&b.id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_runs_on_its_own_simulator_device_from_the_run_after_it_asks_and_it_goes_with_the_agent() {
        use crate::simulator::tests::{ask_for_device, boot, fake_devices, fake_xcrun, simctl_log};
        let (mut st, a) = fake_claude_state();
        fake_xcrun(&mut st);
        let other = crate::tasks::tests::agent(&st, "Other");
        let devices_seen = |id: &str| std::fs::read_to_string(workspace(&st, id).join("device.log")).unwrap_or_default();
        let last_system = |id: &str| {
            let log = std::fs::read_to_string(workspace(&st, id).join("system.log")).unwrap();
            log.rsplit("\n---\n").find(|p| !p.trim().is_empty()).unwrap().to_string()
        };

        run(&st, &a.id, "hello", 1).await;
        assert_eq!(devices_seen(&a.id), "none\n");
        let system = last_system(&a.id);
        assert!(system.contains("You have none yet: call `mcp__loft__simulator_device`"), "{system}");
        assert!(!system.contains("booted device") && !system.contains("simctl install booted"), "{system}");
        assert!(simctl_log(&st).is_empty(), "no device until the agent needs one");

        ask_for_device(&st, &a.id).await;
        ask_for_device(&st, &other.id).await;
        let udid = st.db.agent_device(&a.id).unwrap().unwrap();
        run(&st, &a.id, "again", 2).await;
        run(&st, &a.id, "and again", 3).await;
        assert_eq!(devices_seen(&a.id), format!("none\n{udid}\n{udid}\n"), "the same device on every later run");
        let system = last_system(&a.id);
        assert!(system.contains(&format!("Your device's UDID is `{udid}` (also in `$LOFT_SIMULATOR_UDID`)")), "{system}");
        assert!(system.contains("xcrun simctl install <udid>") && !system.contains("booted device"), "{system}");

        // Deleted through the API (booted, as an agent leaves it), its device is deleted; the other agent's stays.
        boot(&st, &udid);
        let uri: axum::http::Uri = "/?purge_files=false".parse().unwrap();
        let opts = axum::extract::Query::try_from_uri(&uri).unwrap();
        let _ = crate::api::delete_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone()), opts).await.ok().unwrap();
        let left: Vec<String> = fake_devices(&st).into_iter().map(|d| d.2).collect();
        assert_eq!(left, ["Other (Loft)"]);
        assert!(simctl_log(&st).contains(&format!("simctl delete {udid}")));
        assert_eq!(st.db.agent_device(&a.id).unwrap(), None);
    }

    fn error_text(st: &AppState, id: &str) -> Option<String> {
        st.db.list_messages(id, 50, None).unwrap().into_iter().find(|m| m.kind == "error").map(|m| m.content)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_sigterm_from_outside_loft_is_named_in_the_error() {
        for prompt in ["sigterm", "exit-143"] {
            let (st, a) = fake_claude_state();
            enqueue(&st, &a.id, Some(prompt.into()), "user");
            until("the run to fail", 10, || !is_running(&st, &a.id) && error_text(&st, &a.id).is_some()).await;
            assert!(error_text(&st, &a.id).unwrap().contains("from outside Loft"), "{prompt}: {:?}", error_text(&st, &a.id));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_sigkill_is_not_blamed_on_the_outside() {
        // macOS ends a process that runs out of memory with SIGKILL, so it isn't named as some agent's kill.
        for prompt in ["sigkill", "exit-137"] {
            let (st, a) = fake_claude_state();
            enqueue(&st, &a.id, Some(prompt.into()), "user");
            until("the run to fail", 10, || !is_running(&st, &a.id) && error_text(&st, &a.id).is_some()).await;
            let err = error_text(&st, &a.id).unwrap();
            assert!(err.contains("without a result") && !err.contains("outside Loft"), "{prompt}: {err}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_file_write_fails_the_run_with_nothing_on_argv() {
        let (st, a) = fake_claude_state();
        // A file where the folder should be: none of claude's files can be written.
        std::fs::create_dir_all(&st.cfg.storage_dir).unwrap();
        std::fs::write(st.cfg.storage_dir.join(AGENT_FILES), "").unwrap();
        enqueue(&st, &a.id, Some("hello".into()), "user");
        until("the run to fail", 10, || !is_running(&st, &a.id) && error_text(&st, &a.id).is_some()).await;
        assert!(error_text(&st, &a.id).unwrap().contains("File exists"));
        // claude never started: no prompt on argv instead.
        assert!(!workspace(&st, &a.id).join("argv.log").exists());
        assert!(!workspace(&st, &a.id).join("prompts.log").exists());
    }

    #[test]
    fn the_old_agent_files_folder_is_deleted() {
        let dir = crate::testdir::for_this_test("old-files");
        std::fs::create_dir_all(dir.join(OLD_AGENT_FILES)).unwrap();
        std::fs::write(dir.join(OLD_AGENT_FILES).join("a.mcp.json"), "{\"token\":\"x\"}").unwrap();
        std::fs::create_dir_all(dir.join(AGENT_FILES)).unwrap();
        remove_old_agent_files(&dir);
        assert!(!dir.join(OLD_AGENT_FILES).exists());
        assert!(dir.join(AGENT_FILES).exists(), "the current folder stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loft_s_own_stops_are_not_blamed_on_the_outside() {
        let (st, a) = fake_claude_state();
        // claude closed its output without a result and stayed alive: Loft stops it.
        enqueue(&st, &a.id, Some("stuck-pipes".into()), "user");
        until("the run to fail", 10, || !is_running(&st, &a.id) && error_text(&st, &a.id).is_some()).await;
        let err = error_text(&st, &a.id).unwrap();
        assert!(err.contains("without a result") && !err.contains("outside Loft"), "{err}");

        // Stop and restart.
        let (st, a) = fake_claude_state();
        enqueue(&st, &a.id, Some("busy".into()), "user");
        until("the run to start", 10, || is_running(&st, &a.id)).await;
        stop(&st, &a.id);
        until("the stop", 10, || !is_running(&st, &a.id)).await;
        enqueue(&st, &a.id, Some("busy".into()), "user");
        until("the run to start", 10, || is_running(&st, &a.id)).await;
        restart(&st, &a.id).await;
        until("the restart", 10, || !is_running(&st, &a.id)).await;
        let msgs = st.db.list_messages(&a.id, 50, None).unwrap();
        assert!(msgs.iter().all(|m| !m.content.contains("outside Loft")), "{msgs:?}");
    }

    async fn run(st: &AppState, id: &str, text: &str, runs: usize) {
        enqueue(st, id, Some(text.into()), "user");
        until("the run", 10, || !is_running(st, id) && memory_parts(st, id).len() == runs).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pr_shared_in_a_run_is_tracked_for_the_agent_that_shared_it() {
        let (st, a) = fake_claude_state();
        let fake = crate::github::tests::own_fake(&st);
        fake.route("GET", "/repos/octo/share-run/pulls/84", 200, json!({ "number": 84 }));
        run(&st, &a.id, "sharepr", 1).await;
        until("the PR is tracked", 10, || st.db.find_pull_request(&a.id, "octo/share-run", 84).unwrap().is_some()).await;
        let pr = st.db.find_pull_request(&a.id, "octo/share-run", 84).unwrap().unwrap();
        assert_eq!(pr.tool_use_id.as_deref(), Some("s1"), "its card sits on the tool call");
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
        // Each finished run also adds itself and its cost to the All-time spend.
        let (runs, cost) = st.db.all_time_spend().unwrap();
        assert_eq!(runs, 7);
        assert!((cost - 0.020).abs() < 1e-9, "{cost}");

        // A grill round saves no run end, yet it is a finished, paid run all the same.
        st.db.update_agent(&a.id, |a| a.grill_briefed = true).unwrap(); // a later round: no planning brief in front
        wake(&st, &a.id, format!("[Loft: grill] next round sid={s1} cost=0.005"), "grill");
        until("the grill run", 10, || !is_running(&st, &a.id) && memory_parts(&st, &a.id).len() == 8).await;
        assert_eq!(costs().len(), 7, "a quiet run's end stays out of the chat");
        assert!((st.db.get_agent(&a.id).unwrap().unwrap().total_cost_usd - 0.023).abs() < 1e-9);
        let (runs, cost) = st.db.all_time_spend().unwrap();
        assert_eq!(runs, 8);
        assert!((cost - 0.023).abs() < 1e-9, "{cost}");
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

    /// A ticket boundary waiting in the queue is dropped with the queue: the Session the user picked (or the
    /// fresh one they asked for) isn't cleared by the next run.
    #[tokio::test]
    async fn clear_and_resume_drop_a_pending_ticket_boundary() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        st.db.update_agent(&a.id, |a| (a.fresh_context, a.session_id) = ("always".into(), Some("s-now".into()))).unwrap();
        let pending = |st: &AppState| st.rt.lock().unwrap().fresh_before.contains(&a.id);
        st.rt.lock().unwrap().fresh_before.insert(a.id.clone());
        let (dir, past) = past_session();
        let _ = crate::api::resume_from(&st, &a.id, dir, past.clone()).await.unwrap();
        assert!(!pending(&st), "resume drops it");
        fresh_boundary(&st, &a.id);
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().session_id, Some(past), "the resumed Session stays");

        st.rt.lock().unwrap().fresh_before.insert(a.id.clone());
        let _ = crate::api::reset_session(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await.unwrap();
        assert!(!pending(&st), "Clear context drops it");
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
        assert_eq!(fields(&st, &a.id), json!({ "running": false, "last_output_at": null, "waiting_on": [], "waiting_since": null, "subagents": [], "queued_messages": [], "redrawing": false }));
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
    async fn a_tool_call_leaves_the_status_text_alone() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        enqueue(&st, &a.id, Some("tool-status".into()), "user");
        let tool_rows = || st.db.list_messages(&a.id, 50, None).unwrap().iter().filter(|m| m.kind == "tool_use").count();
        until("the tool row", 10, || tool_rows() == 1).await;
        assert_eq!(status_text(&st, &a.id), "Thinking…", "the command shows in its tool row only");
        // The agent's own line (set_status) stays through the next tool call.
        set_status_text(&st, &a.id, "Reading the config");
        std::fs::write(ws.join("go"), "").unwrap();
        until("the second tool row", 10, || tool_rows() == 2).await;
        assert_eq!(status_text(&st, &a.id), "Reading the config");
        std::fs::write(ws.join("go2"), "").unwrap();
        until("Loft's own state", 10, || status_text(&st, &a.id) == "Preparing the next question…").await;
        std::fs::write(ws.join("go3"), "").unwrap();
        until("the run to end", 10, || !is_running(&st, &a.id)).await;
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

    /// A Delivery wake's real run: one that ends without the step's call stalls the Delivery (failed or not),
    /// and a Restart carries the wake over to the run it starts instead of stalling it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_delivery_runs_end_without_its_call_stalls_it() {
        let (st, a) = fake_claude_state();
        let save = |spec: i64| {
            let mut d: crate::db::Delivery = serde_json::from_value(json!({ "repo": "o/r", "spec": spec, "state": "running", "current": spec + 1, "dev": a.id, "created_at": "" })).unwrap();
            d.tickets.push(crate::db::DeliveryTicket::new(spec + 1, "T", vec![], "started"));
            st.db.save_delivery(&d).unwrap();
        };
        let wake = |spec: i64, prompt: &str| {
            let w = crate::delivery::Wake { repo: "o/r".into(), spec, step: crate::delivery::Step::Build { ticket: spec + 1, round: 0 } };
            wake_on_ticket(&st, &a.id, &format!("o/r#{}", spec + 1), true, prompt.into(), w);
        };
        let stalled = |spec: i64| st.db.get_delivery("o/r", spec).unwrap().unwrap().stall_reason;
        for (spec, prompt, why) in [(40, "build it", "no_call"), (50, "build it and fail", "run_failed")] {
            save(spec);
            wake(spec, prompt);
            until("the Delivery to stall", 10, || !stalled(spec).is_empty()).await;
            assert_eq!(stalled(spec), why);
            idle(&st, &a.id).await;
        }

        save(60);
        wake(60, "build it while busy");
        until("the run to start", 10, || std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap_or_default().contains("while busy")).await;
        // The restarted run gets an FYI saying "busy", so it keeps going too.
        st.db.add_pending_fyi(&a.id, "stay busy").unwrap();
        assert!(crate::api::restart_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await.is_ok());
        let prompts = || std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap_or_default();
        until("the restarted run to start", 10, || prompts().contains("restarted your last run")).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(stalled(60), "", "the restarted run carries on with the step");
        // Stopped from the agent's Stop without the call: now it stalls.
        assert!(stop(&st, &a.id));
        until("the Delivery to stall", 10, || !stalled(60).is_empty()).await;
        assert_eq!(stalled(60), "no_call");
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

        // Another group's agent is reachable, but doesn't shadow this group's agent of the same name.
        let mut far = crate::tasks::tests::agent(&st, "Web Dev");
        far.group = "imole".into();
        st.db.save_agent(&far).unwrap();
        let r = send("@Web Dev again").await.ok().unwrap().0;
        assert_eq!(r["mentioned"], json!([{ "id": dev.id, "name": "Web Dev" }]));
        let mut isa = crate::tasks::tests::agent(&st, "Ida");
        isa.group = "imole".into();
        st.db.save_agent(&isa).unwrap();
        let r = send("@Ida have a look").await.ok().unwrap().0;
        assert_eq!(r["mentioned"], json!([{ "id": isa.id, "name": "Ida" }]));
    }

    #[test]
    fn at_paths_start_at_the_projects_folder_when_there_is_one() {
        let st = crate::tasks::tests::state();
        let dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        let projects = dir.join("projects");
        std::fs::create_dir_all(projects.join("work/notes")).unwrap();
        std::fs::write(dir.join("settings.json"), json!({ "projects_dir": projects }).to_string()).unwrap();
        let st = AppState { settings: std::sync::Arc::new(crate::settings::Store::load(&dir).unwrap()), ..st };
        let mut a = crate::tasks::tests::agent(&st, "Dev");
        a.project = "work/notes".into();
        assert_eq!(at_base(&st, &a), projects.join("work/notes").canonicalize().unwrap());
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

    fn redraw(st: &AppState, id: &str) -> impl std::future::Future<Output = crate::api::ApiResult<Value>> {
        crate::api::redraw_avatar(axum::extract::State(st.clone()), axum::extract::Path(id.to_string()))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redraw_draws_in_a_quiet_run_on_a_throwaway_session() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        st.db
            .update_agent(&a.id, |a| {
                a.session_id = Some("s-own".into());
                (a.session_cost_usd, a.total_cost_usd) = (1.0, 1.0);
                a.context_tokens = Some(5000);
                a.memory_index = Some("- #1 frozen".into());
            })
            .unwrap();
        let (runs, spend) = st.db.all_time_spend().unwrap();
        assert_eq!(redraw(&st, &a.id).await.ok().unwrap().0, json!({ "ok": true }));
        until("the avatar run", 10, || status_text(&st, &a.id) == "Drawing its avatar…").await;
        std::fs::write(ws.join("go"), "").unwrap();
        idle(&st, &a.id).await;

        let prompts = std::fs::read_to_string(ws.join("prompts.log")).unwrap();
        assert!(prompts.contains("main project's README") && prompts.contains("mcp__loft__set_avatar"), "{prompts}");
        assert!(!prompts.contains("just created"), "{prompts}");
        assert!(!ws.join("resumed.log").exists(), "it doesn't resume the agent's Session");
        assert!(std::fs::read_to_string(ws.join("argv.log")).unwrap().contains("--no-session-persistence"));
        let now = st.db.get_agent(&a.id).unwrap().unwrap();
        assert_eq!(now.session_id.as_deref(), Some("s-own"), "the saved session stays");
        assert_eq!((now.session_cost_usd, now.context_tokens, now.memory_index.as_deref()), (1.0, Some(5000), Some("- #1 frozen")));
        assert!((now.total_cost_usd - 1.25).abs() < 1e-9, "{}", now.total_cost_usd);
        let (runs_now, spend_now) = st.db.all_time_spend().unwrap();
        assert_eq!(runs_now, runs + 1);
        assert!((spend_now - spend - 0.25).abs() < 1e-9, "the run counts in All-time spend");
        assert!(st.db.list_messages(&a.id, 50, None).unwrap().is_empty(), "only errors reach the chat");
        assert!(!unread(&st, &a.id));

        // The next run resumes the agent's own Session (the Redraw's system prompt is the first).
        run(&st, &a.id, "hello", 2).await;
        assert_eq!(std::fs::read_to_string(ws.join("resumed.log")).unwrap(), "s-own\n");
    }

    async fn redrawing(st: &AppState, id: &str) -> bool {
        let got = crate::api::get_agent(axum::extract::State(st.clone()), axum::extract::Path(id.to_string())).await.ok().unwrap().0;
        got["redrawing"].as_bool().unwrap()
    }

    /// The last `redrawing` an agent event for `id` carried, of those received so far.
    fn heard_redrawing(events: &mut tokio::sync::broadcast::Receiver<crate::state::Event>, id: &str) -> Option<bool> {
        let mut last = None;
        while let Ok(ev) = events.try_recv() {
            if let crate::state::Event::AgentUpdated { agent } = ev
                && agent["id"] == id {
                    last = agent["redrawing"].as_bool();
                }
        }
        last
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_agent_says_redrawing_while_a_redraw_waits_or_draws() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        run(&st, &a.id, "hello", 1).await;
        assert!(!redrawing(&st, &a.id).await);

        // Queued behind a busy run: the agent JSON and an event say so straight away.
        let _ = send(&st, &a.id, "busy").await.ok().unwrap();
        until("the busy run", 10, || std::fs::read_to_string(ws.join("prompts.log")).unwrap_or_default().contains("busy")).await;
        let mut events = st.events.subscribe();
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        assert!(redrawing(&st, &a.id).await, "a queued Redraw counts");
        assert_eq!(heard_redrawing(&mut events, &a.id), Some(true), "other tabs hear it's queued");

        // Drawing: still true.
        stop(&st, &a.id);
        until("the avatar run", 10, || status_text(&st, &a.id) == "Drawing its avatar…").await;
        assert!(redrawing(&st, &a.id).await, "a Redraw drawing counts");
        std::fs::write(ws.join("go"), "").unwrap();
        idle(&st, &a.id).await;
        assert!(!redrawing(&st, &a.id).await, "false once it has drawn");
        assert_eq!(heard_redrawing(&mut events, &a.id), Some(false), "the last event says it's done");

        // A Redraw that ends without a sticker (stopped) clears it too.
        std::fs::remove_file(ws.join("go")).unwrap();
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        until("the avatar run", 10, || status_text(&st, &a.id) == "Drawing its avatar…").await;
        assert!(redrawing(&st, &a.id).await);
        stop(&st, &a.id);
        idle(&st, &a.id).await;
        assert!(!redrawing(&st, &a.id).await, "false once a stopped Redraw ends");
        assert_eq!(heard_redrawing(&mut events, &a.id), Some(false));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_redraw_ending_with_work_queued_after_it_says_so_before_that_work_ends() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        until("the avatar run", 10, || status_text(&st, &a.id) == "Drawing its avatar…").await;
        let _ = send(&st, &a.id, "after").await.ok().unwrap();
        let mut events = st.events.subscribe();
        std::fs::write(ws.join("go"), "").unwrap();
        until("the next run", 10, || std::fs::read_to_string(ws.join("prompts.log")).unwrap_or_default().contains("after")).await;
        assert!(!redrawing(&st, &a.id).await);
        assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|ev| matches!(&ev, crate::state::Event::AgentUpdated { agent } if agent["id"] == a.id && agent["redrawing"] == false)));
        idle(&st, &a.id).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redraw_on_a_busy_agent_waits_for_its_run() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        run(&st, &a.id, "hello", 1).await;
        let _ = send(&st, &a.id, "busy").await.ok().unwrap();
        until("the busy run", 10, || std::fs::read_to_string(ws.join("prompts.log")).unwrap_or_default().contains("busy")).await;
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!std::fs::read_to_string(ws.join("prompts.log")).unwrap().contains("redraw your sticker"), "it waits for the run");
        assert_ne!(status_text(&st, &a.id), "Drawing its avatar…");

        stop(&st, &a.id);
        until("the avatar run", 10, || status_text(&st, &a.id) == "Drawing its avatar…").await;
        std::fs::write(ws.join("go"), "").unwrap();
        idle(&st, &a.id).await;
        let prompts = std::fs::read_to_string(ws.join("prompts.log")).unwrap();
        assert!(prompts.find("busy").unwrap() < prompts.find("redraw your sticker").unwrap(), "{prompts}");
        assert_eq!(std::fs::read_to_string(ws.join("resumed.log")).unwrap(), "s1\n", "only the busy run resumed the Session");
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().session_id.as_deref(), Some("s1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redraw_does_not_stop_background_work_the_agent_waits_on() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        enqueue(&st, &a.id, Some("start the suite, linger".into()), "user");
        until("the reply", 10, || status_text(&st, &a.id).starts_with("Waiting on: Run the QA suite")).await;
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        tokio::time::sleep(EXIT_GRACE * 3).await;
        assert!(is_running(&st, &a.id) && !ws.join("terminated").exists(), "the background work keeps going");
        assert_eq!(prompts(&st, &a.id).matches("\n---\n").count(), 1, "no run starts before the busy one ends");
        assert!(status_text(&st, &a.id).starts_with("Waiting on: Run the QA suite"));

        stop(&st, &a.id);
        until("the avatar run", 10, || status_text(&st, &a.id) == "Drawing its avatar…").await;
        std::fs::write(ws.join("go"), "").unwrap();
        idle(&st, &a.id).await;
        let log = prompts(&st, &a.id);
        assert_eq!(log.matches("\n---\n").count(), 2, "only the Redraw ran after it: {log}");
        assert!(!log.contains("was stopped when new messages arrived"), "{log}");
        let msgs = st.db.list_messages(&a.id, 50, None).unwrap();
        assert!(!msgs.iter().any(|m| m.content.contains("Stopped background work")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn clicking_redraw_again_while_it_draws_draws_once() {
        let (st, a) = fake_claude_state();
        let ws = workspace(&st, &a.id);
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        until("the avatar run", 10, || status_text(&st, &a.id) == "Drawing its avatar…").await;
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        std::fs::write(ws.join("go"), "").unwrap();
        idle(&st, &a.id).await;
        assert_eq!(prompts(&st, &a.id).matches("redraw your sticker").count(), 1);

        // Once it has drawn, Redraw draws again.
        let _ = redraw(&st, &a.id).await.ok().unwrap();
        idle(&st, &a.id).await;
        assert_eq!(prompts(&st, &a.id).matches("redraw your sticker").count(), 2);
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
        // Running as far as Restart can tell: the run has registered whether it answers the user.
        let running = |st: &AppState, id: &str| st.rt.lock().unwrap().runs.contains_key(id);
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

    /// Makes the agent's current run look quiet for `minutes`, as a dead API connection does.
    fn go_quiet(st: &AppState, id: &str, minutes: i64) {
        st.rt.lock().unwrap().runs.get_mut(id).unwrap().last_output_at = (Utc::now() - chrono::Duration::minutes(minutes)).to_rfc3339();
    }

    fn prompts(st: &AppState, id: &str) -> String {
        std::fs::read_to_string(workspace(st, id).join("prompts.log")).unwrap_or_default()
    }

    /// The agent's `n`th run has started and its first event (the session) has come in, so the quiet
    /// clock it sets can be moved back.
    async fn started(st: &AppState, id: &str, n: usize) {
        until("the run to start", 10, || prompts(st, id).matches("\n---\n").count() == n && st.rt.lock().unwrap().runs.contains_key(id)).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_quiet_run_with_nothing_to_wait_on_is_restarted_like_the_restart_button() {
        let (st, a) = fake_claude_state();
        let _ = send(&st, &a.id, "busy").await.ok().unwrap();
        started(&st, &a.id, 1).await;

        // Waiting on background work (another agent, so its run's own handling of that stays out of the way).
        let b = crate::tasks::tests::agent(&st, "Builder");
        let _ = send(&st, &b.id, "busy").await.ok().unwrap();
        started(&st, &b.id, 1).await;
        st.rt.lock().unwrap().runs.get_mut(&b.id).unwrap().waiting_on = vec!["npm test".into()];
        go_quiet(&st, &b.id, 25);
        // Not quiet long enough, in a tool call that hasn't returned (a long build), or waiting on a
        // permission: left alone.
        go_quiet(&st, &a.id, 19);
        restart_quiet(&st).await;
        go_quiet(&st, &a.id, 25);
        st.rt.lock().unwrap().runs.get_mut(&a.id).unwrap().tools_open = vec!["toolu_build".into()];
        restart_quiet(&st).await;
        st.rt.lock().unwrap().runs.get_mut(&a.id).unwrap().tools_open.clear();
        let mut ask = crate::db::PermissionRequest {
            id: "p-quiet".into(), agent_id: a.id.clone(), source: "prompt".into(), tool_name: "Bash".into(), input: json!({ "command": "ls" }),
            reason: String::new(), rule: "Bash(ls)".into(), status: "pending".into(), created_at: crate::db::now(),
        };
        st.db.save_permission(&ask).unwrap();
        restart_quiet(&st).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(is_running(&st, &a.id) && !prompts(&st, &a.id).contains("restarted your last run"), "{}", prompts(&st, &a.id));
        assert!(!prompts(&st, &b.id).contains("restarted your last run"), "waiting on background work");
        let _ = crate::api::stop_agent(axum::extract::State(st.clone()), axum::extract::Path(b.id.clone())).await;
        ask.status = "allow_once".into();
        st.db.save_permission(&ask).unwrap();

        // Quiet 20 minutes with nothing to wait on: restarted, told so, and its reply is Unread as before.
        restart_quiet(&st).await;
        until("the restarted run", 10, || prompts(&st, &a.id).contains("[Loft: Loft restarted your last run (no output for 25 min)")).await;
        idle(&st, &a.id).await;
        assert!(unread(&st, &a.id), "the restarted run's reply");
        assert!(st.db.list_messages(&a.id, 50, None).unwrap().iter().any(|m| m.content.starts_with("Restarted by Loft")));
        // It answered: the count starts over.
        assert!(!st.rt.lock().unwrap().auto_restarts.contains_key(&a.id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_that_stays_quiet_is_restarted_twice_then_left_to_the_user() {
        let (st, a) = fake_claude_state();
        let runs = |st: &AppState| prompts(st, &a.id).matches("busy").count();
        let _ = send(&st, &a.id, "busy 1").await.ok().unwrap();
        started(&st, &a.id, 1).await;
        // Each restarted run hangs too: it runs what's queued, says something, and never finishes its turn.
        for n in 2..=3 {
            let _ = send(&st, &a.id, &format!("chatty busy {n}")).await.ok().unwrap();
            go_quiet(&st, &a.id, 21);
            restart_quiet(&st).await;
            started(&st, &a.id, n).await;
        }
        assert_eq!(st.rt.lock().unwrap().auto_restarts.get(&a.id), Some(&2));

        // A third quiet spell: not restarted; Needs you says why.
        go_quiet(&st, &a.id, 21);
        restart_quiet(&st).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(runs(&st), 3);
        assert!(is_running(&st, &a.id));
        let snap = crate::needs_you::snapshot(&st).unwrap();
        let stuck = snap["items"].as_array().unwrap().iter().find(|i| i["type"] == "stuck").unwrap();
        assert!(stuck["reason"].as_str().unwrap().contains("after 2 automatic restarts"), "{stuck}");
        let _ = crate::api::stop_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone())).await;
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

    fn count(st: &AppState, id: &str, kind: &str) -> usize {
        st.db.list_messages(id, 400, None).unwrap().iter().filter(|m| m.kind == kind).count()
    }

    fn set(st: &AppState, id: &str, f: impl FnOnce(&mut Agent)) {
        st.db.update_agent(id, f).unwrap().unwrap();
    }

    /// A Delivery wake on `key`, run to its end.
    async fn on_ticket(st: &AppState, id: &str, key: &str, builder: bool) {
        let runs = count(st, id, "run_end");
        wake_on_ticket(st, id, key, builder, format!("work on {key}"), crate::delivery::Wake { repo: "o/r".into(), spec: 1, step: crate::delivery::Step::FinalReview { round: 0 } });
        until("the ticket's run", 10, || !is_running(st, id) && count(st, id, "run_end") == runs + 1).await;
    }

    fn resumed(st: &AppState, id: &str) -> usize {
        std::fs::read_to_string(workspace(st, id).join("resumed.log")).unwrap_or_default().lines().count()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fresh_context_clears_before_a_new_delivery_ticket_as_set() {
        let (st, a) = fake_claude_state();
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().fresh_context, "above_80k", "an agent saved before the setting");
        st.db.add_memory(&a.id, "Deploys go through staging", "x", "").unwrap();
        on_ticket(&st, &a.id, "o/r#1/2", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 0, "no session yet: nothing to clear");

        // Above 80k: below the warning stays, above it clears, and never within the same ticket.
        set(&st, &a.id, |a| a.context_tokens = Some(CONTEXT_WARN_AT - 1));
        on_ticket(&st, &a.id, "o/r#1/3", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 0, "below 80k");
        set(&st, &a.id, |a| a.context_tokens = Some(120_000));
        on_ticket(&st, &a.id, "o/r#1/3", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 0, "a changes-requested round on the same ticket");
        let before = resumed(&st, &a.id);
        on_ticket(&st, &a.id, "o/r#1/4", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 1, "a new ticket above 80k");
        assert_eq!(resumed(&st, &a.id), before, "the ticket's run started a new session");
        let m = st.db.list_messages(&a.id, 400, None).unwrap().into_iter().find(|m| m.kind == "clear").unwrap();
        assert_eq!(m.content, "Context cleared for a new ticket. Long-term memory carries over.");
        assert_eq!(st.db.list_memories(&a.id).unwrap().len(), 1, "long-term memory carries over");

        // Always: every new ticket, small context or not. Off: never.
        set(&st, &a.id, |a| (a.fresh_context, a.context_tokens) = ("always".into(), Some(10)));
        on_ticket(&st, &a.id, "o/r#1/4", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 1, "same ticket");
        on_ticket(&st, &a.id, "o/r#1/final", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 2, "the final review is its own boundary");
        set(&st, &a.id, |a| (a.fresh_context, a.context_tokens) = ("off".into(), Some(500_000)));
        on_ticket(&st, &a.id, "o/r#2/5", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 2, "off");

        // Above 150k: 120k stays, 150k clears.
        set(&st, &a.id, |a| (a.fresh_context, a.context_tokens) = ("above_150k".into(), Some(120_000)));
        on_ticket(&st, &a.id, "o/r#2/6", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 2, "below 150k");
        set(&st, &a.id, |a| a.context_tokens = Some(150_000));
        on_ticket(&st, &a.id, "o/r#2/7", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 3, "a new ticket at 150k");
    }

    #[test]
    fn fresh_threshold_reads_the_setting() {
        assert_eq!(fresh_threshold("above_80k"), 80_000);
        assert_eq!(fresh_threshold("above_200k"), 200_000);
        assert_eq!(fresh_threshold("above_xk"), CONTEXT_WARN_AT);
        assert!(FRESH_CONTEXT.iter().filter(|f| f.starts_with("above_")).all(|f| fresh_threshold(f) >= CONTEXT_WARN_AT));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_lead_is_cleared_by_delivery_only_as_the_builder() {
        let (st, a) = fake_claude_state();
        set(&st, &a.id, |a| (a.role, a.group, a.fresh_context) = ("lead".into(), "team".into(), "always".into()));
        let lead = st.db.get_agent(&a.id).unwrap().unwrap();
        assert!(crate::delivery::fresh_context_applies(&st, &lead), "no Dev: the Lead builds the tickets");
        on_ticket(&st, &a.id, "o/r#1/2", true).await;
        on_ticket(&st, &a.id, "o/r#1/3", false).await;
        on_ticket(&st, &a.id, "o/r#1/final", false).await;
        assert_eq!(count(&st, &a.id, "clear"), 0, "coordinating");
        on_ticket(&st, &a.id, "o/r#1/4", true).await;
        assert_eq!(count(&st, &a.id, "clear"), 1, "building a new ticket");

        let dev = crate::tasks::tests::agent(&st, "Dev");
        set(&st, &dev.id, |d| (d.role, d.group) = ("dev".into(), "team".into()));
        assert!(!crate::delivery::fresh_context_applies(&st, &lead), "a Lead that only coordinates");
        assert!(crate::delivery::fresh_context_applies(&st, &st.db.get_agent(&dev.id).unwrap().unwrap()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_ticket_queued_behind_a_run_clears_only_when_that_run_ends() {
        let (st, a) = fake_claude_state();
        on_ticket(&st, &a.id, "o/r#1/2", true).await;
        set(&st, &a.id, |a| a.fresh_context = "always".into());
        enqueue(&st, &a.id, Some("delegate".into()), "user");
        until("the run to start", 10, || st.db.list_messages(&a.id, 50, None).unwrap().iter().any(|m| m.content.contains("Nested"))).await;
        wake_on_ticket(&st, &a.id, "o/r#1/3", true, "work on #3".into(), crate::delivery::Wake { repo: "o/r".into(), spec: 1, step: crate::delivery::Step::FinalReview { round: 0 } });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(count(&st, &a.id, "clear"), 0, "never during a run");
        let ws = workspace(&st, &a.id);
        std::fs::write(ws.join("go"), "").unwrap();
        std::fs::write(ws.join("go2"), "").unwrap();
        until("the queued ticket's run", 10, || !is_running(&st, &a.id) && count(&st, &a.id, "run_end") == 3).await;
        let mut msgs = st.db.list_messages(&a.id, 400, None).unwrap();
        msgs.sort_by_key(|m| m.id);
        let kinds: Vec<&str> = msgs.iter().filter(|m| matches!(m.kind.as_str(), "run_end" | "clear")).map(|m| m.kind.as_str()).collect();
        assert_eq!(kinds, ["run_end", "run_end", "clear", "run_end"], "cleared between the run and the ticket's");
    }
}
