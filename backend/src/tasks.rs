//! Tasks: personal todos and work handed to agents. The user owns a task's
//! description and assignment; the agent working on it owns its execution
//! status, which it reports through the `update_task` MCP tool.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agents::{self, is_running};
use crate::api::{ApiError, bad, not_found};
use crate::db::{Agent, Task, TaskComment, now};
use crate::state::{AppState, Event};

const PRIORITIES: &[&str] = &["low", "normal", "high", "urgent"];
/// Statuses the user may set directly. The rest come from the agent doing the work, or from execute/cancel.
const USER_STATUSES: &[&str] = &["inbox", "ready", "done"];
/// Statuses an agent may report on a task it owns.
const AGENT_STATUSES: &[&str] = &["in_progress", "blocked", "waiting_approval", "done", "failed"];

fn emit(state: &AppState, t: &Task) {
    state.emit(Event::Task { task: json!(t) });
}

/// Longest comment or note kept in a task's thread.
const MAX_COMMENT: usize = 4000;

/// "you" for the user, else the agent's name.
fn who(state: &AppState, author: &str) -> String {
    if author == "user" {
        return "you".into();
    }
    state.db.get_agent(author).ok().flatten().map(|a| a.name).unwrap_or_else(|| "an agent".into())
}

/// Adds an entry to the task's thread and tells open clients. Failures are logged, not fatal:
/// the change it describes has already happened.
fn log(state: &AppState, t: &Task, author: &str, kind: &str, text: &str, mut meta: Value) -> Option<TaskComment> {
    if author != "user" {
        meta["author_name"] = json!(who(state, author));
    }
    match state.db.add_task_comment(&t.id, author, kind, &clip(text, MAX_COMMENT), meta) {
        Ok(c) => {
            state.emit(Event::TaskComment { comment: json!(c) });
            Some(c)
        }
        Err(e) => {
            tracing::warn!("task {}: couldn't add to the thread: {e}", t.id);
            None
        }
    }
}

/// A status entry: "Started by Web Dev", "Marked done by you".
fn log_status(state: &AppState, t: &Task, author: &str, text: &str) {
    log(state, t, author, "status", text, json!({ "status": t.status }));
}

fn is_active(t: &Task) -> bool {
    matches!(t.status.as_str(), "in_progress" | "blocked" | "waiting_approval")
}

fn is_closed(t: &Task) -> bool {
    matches!(t.status.as_str(), "done" | "cancelled")
}

fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// A message's first line without the `[Task <id>]` tag task briefs and reports start with.
fn headline(s: &str) -> &str {
    let line = s.lines().next().unwrap_or("").trim();
    match line.strip_prefix("[Task ").and_then(|r| r.split_once(']')) {
        Some((_, rest)) => rest.trim(),
        None => line,
    }
}

/// Validate the fields a task creator or editor controls, from a JSON object, onto `t`.
fn apply_fields(state: &AppState, t: &mut Task, p: &Value) -> Result<(), String> {
    let str_field = |k: &str| p.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    if let Some(v) = str_field("title") {
        let v = v.trim();
        if v.is_empty() {
            return Err("title cannot be empty".into());
        }
        t.title = clip(v, 200);
    }
    if let Some(v) = str_field("description") { t.description = clip(&v, 20_000); }
    if let Some(v) = str_field("project") { t.project = clip(&v, 100); }
    if let Some(v) = str_field("priority") {
        if !PRIORITIES.contains(&v.as_str()) {
            return Err(format!("priority must be one of {}", PRIORITIES.join(" | ")));
        }
        t.priority = v;
    }
    if let Some(v) = str_field("kind") {
        if !["todo", "work"].contains(&v.as_str()) {
            return Err("kind must be todo | work".into());
        }
        t.kind = v;
    }
    if let Some(v) = p.get("due_at") {
        t.due_at = match v {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            Value::String(s) => Some(
                chrono::DateTime::parse_from_rfc3339(s)
                    .map_err(|_| "due_at must be an RFC 3339 timestamp".to_string())?
                    .with_timezone(&chrono::Utc)
                    .to_rfc3339(),
            ),
            _ => return Err("due_at must be a string or null".into()),
        };
    }
    if let Some(v) = p.get("agent_id") {
        t.agent_id = match v {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            Value::String(s) => {
                state.db.get_agent(s).map_err(|e| e.to_string())?.ok_or("unknown agent_id")?;
                Some(s.clone())
            }
            _ => return Err("agent_id must be a string or null".into()),
        };
    }
    if let Some(v) = p.get("parent_id") {
        t.parent_id = match v {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            Value::String(s) => {
                if *s == t.id {
                    return Err("a task can't be its own parent".into());
                }
                // Walk up from the new parent so the tree stays a tree.
                let mut cur = Some(s.clone());
                while let Some(id) = cur {
                    let parent = state.db.get_task(&id).map_err(|e| e.to_string())?.ok_or("unknown parent_id")?;
                    if parent.id == t.id {
                        return Err("that parent is a subtask of this task".into());
                    }
                    cur = parent.parent_id;
                }
                Some(s.clone())
            }
            _ => return Err("parent_id must be a string or null".into()),
        };
    }
    if let Some(v) = p.get("depends_on") {
        let ids: Vec<String> = serde_json::from_value(v.clone()).map_err(|_| "depends_on must be a list of task ids".to_string())?;
        for id in &ids {
            if *id == t.id {
                return Err("a task can't depend on itself".into());
            }
            state.db.get_task(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("unknown task {id} in depends_on"))?;
        }
        t.depends_on = ids;
    }
    Ok(())
}

/// Create a task. `p.id` (a UUID chosen by the client) makes the call idempotent:
/// sending the same id again returns the task already created.
pub fn create(state: &AppState, p: &Value, created_by: &str) -> Result<(Task, bool), String> {
    let id = match p.get("id").and_then(|v| v.as_str()) {
        Some(id) => {
            let id = uuid::Uuid::parse_str(id).map_err(|_| "id must be a UUID".to_string())?.to_string();
            if let Some(existing) = state.db.get_task(&id).map_err(|e| e.to_string())? {
                return Ok((existing, false));
            }
            id
        }
        None => uuid::Uuid::new_v4().to_string(),
    };
    let ts = now();
    let mut t = Task {
        id,
        kind: "todo".into(),
        priority: "normal".into(),
        created_by: created_by.into(),
        created_at: ts.clone(),
        updated_at: ts,
        ..Default::default()
    };
    if p.get("title").and_then(|v| v.as_str()).is_none_or(|s| s.trim().is_empty()) {
        return Err("title is required".into());
    }
    apply_fields(state, &mut t, p)?;
    if t.agent_id.is_some() {
        t.kind = "work".into();
    }
    t.status = match p.get("status").and_then(|v| v.as_str()) {
        Some(s) if ["inbox", "ready"].contains(&s) => s.into(),
        Some(_) => return Err("a new task's status must be inbox | ready".into()),
        None if t.agent_id.is_some() => "ready".into(),
        None => "inbox".into(),
    };
    state.db.save_task(&t).map_err(|e| e.to_string())?;
    emit(state, &t);
    log_status(state, &t, created_by, &format!("Created by {}", who(state, created_by)));
    Ok((t, true))
}

/// The brief an agent receives when a task is handed to it.
fn brief(state: &AppState, t: &Task, note: &str) -> String {
    let mut s = format!("[Task {}] {}", t.id, t.title);
    if !t.description.trim().is_empty() {
        s += &format!("\n\n{}", t.description.trim());
    }
    let parent = t.parent_id.as_deref().and_then(|id| state.db.get_task(id).ok().flatten());
    if let Some(parent) = &parent {
        s += &format!("\n\nThis is part of “{}”.", parent.title);
    }
    let deps: Vec<String> = t
        .depends_on
        .iter()
        .filter_map(|id| state.db.get_task(id).ok().flatten())
        .map(|d| format!("- {} ({})", d.title, d.status.replace('_', " ")))
        .collect();
    if !deps.is_empty() {
        s += &format!("\n\nDepends on:\n{}", deps.join("\n"));
    }
    if !note.trim().is_empty() {
        s += &format!("\n\n{}", note.trim());
    }
    // Ids go in the trailing note, which the apps hide; the agent needs them for the task tools.
    let mut ids = format!("task id {}", t.id);
    if let Some(p) = &parent {
        ids += &format!(", parent task id {}", p.id);
    }
    if !t.depends_on.is_empty() {
        ids += &format!(", depends on {}", t.depends_on.join(", "));
    }
    s += &format!("\n\n[Report progress on this task with `mcp__loft__update_task` ({ids}): notes as you go, then `done` with a summary or `failed`/`blocked` with the reason.]");
    s
}

/// Hand a task to an agent and start it (or queue it behind the agent's current run).
/// `by` is the agent delegating it, or None when the user started it.
pub fn execute(state: &AppState, id: &str, agent_id: Option<&str>, note: &str, by: Option<&Agent>) -> Result<(Task, bool), ApiError> {
    let mut t = state.db.get_task(id)?.ok_or_else(not_found)?;
    if is_active(&t) {
        return Err(ApiError(StatusCode::CONFLICT, "This task is already running. Cancel it first to restart it.".into()));
    }
    if is_closed(&t) {
        return Err(ApiError(StatusCode::CONFLICT, format!("This task is {}. Reopen it before running it again.", t.status)));
    }
    let agent_id = agent_id.map(str::to_string).or(t.agent_id.clone()).ok_or_else(|| bad("Pick an agent to run this task."))?;
    let agent = state.db.get_agent(&agent_id)?.ok_or_else(|| bad("unknown agent_id"))?;
    let ts = now();
    t.agent_id = Some(agent.id.clone());
    t.kind = "work".into();
    t.status = "in_progress".into();
    t.error = String::new();
    t.started_at = Some(ts.clone());
    t.completed_at = None;
    t.updated_at = ts;
    state.db.save_task(&t)?;
    emit(state, &t);
    let by_id = by.map_or("user", |a| a.id.as_str());
    log_status(state, &t, by_id, &format!("Sent to {} by {}", agent.name, who(state, by_id)));
    if !note.trim().is_empty() {
        log(state, &t, by_id, "comment", note.trim(), json!({ "to_agent": agent.id }));
    }
    let queued = is_running(state, &agent.id);
    let text = brief(state, &t, note);
    match by {
        // `task_role` lets apps show a compact task row instead of the full brief.
        Some(from) => agents::message_from_agent_with(state, from, &agent, &text, json!({ "task_id": t.id, "task_role": "brief" })),
        None => {
            agents::enqueue_with(state, &agent.id, Some(text), json!({ "task_id": t.id, "task_role": "brief" }), "user");
        }
    }
    Ok((t, queued))
}

/// Report a subtask's outcome to the agent that delegated it.
fn tell_delegator(state: &AppState, t: &Task, from: &Agent) {
    if t.created_by == "user" || t.created_by == from.id {
        return;
    }
    let Ok(Some(delegator)) = state.db.get_agent(&t.created_by) else { return };
    let detail = match t.status.as_str() {
        "done" if !t.summary.is_empty() => format!(": {}", t.summary),
        "failed" | "blocked" if !t.error.is_empty() => format!(": {}", t.error),
        _ => String::new(),
    };
    let text = format!(
        "[Task {}] “{}”, which you delegated to me, is now {}{}",
        t.id,
        t.title,
        t.status.replace('_', " "),
        detail
    );
    agents::message_from_agent_with(state, from, &delegator, &text, json!({ "task_id": t.id, "task_role": "report", "task_status": t.status }));
}

// ---- REST ---------------------------------------------------------------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    agent_id: Option<String>,
    parent_id: Option<String>,
    project: Option<String>,
}

pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let tasks: Vec<Task> = state
        .db
        .list_tasks()?
        .into_iter()
        .filter(|t| q.status.as_deref().is_none_or(|s| s.split(',').any(|s| s == t.status)))
        .filter(|t| q.agent_id.as_deref().is_none_or(|a| t.agent_id.as_deref() == Some(a)))
        .filter(|t| q.parent_id.as_deref().is_none_or(|p| t.parent_id.as_deref() == Some(p)))
        .filter(|t| q.project.as_deref().is_none_or(|p| t.project == p))
        .collect();
    Ok(Json(json!(tasks)))
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(state.db.get_task(&id)?.ok_or_else(not_found)?)))
}

pub async fn create_task(State(state): State<AppState>, Json(p): Json<Value>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (t, created) = create(&state, &p, "user").map_err(bad)?;
    Ok((if created { StatusCode::CREATED } else { StatusCode::OK }, Json(json!(t))))
}

pub async fn update(State(state): State<AppState>, Path(id): Path<String>, Json(p): Json<Value>) -> Result<Json<Value>, ApiError> {
    let mut t = state.db.get_task(&id)?.ok_or_else(not_found)?;
    let reassigning = p.get("agent_id").is_some_and(|v| v.as_str() != t.agent_id.as_deref());
    if reassigning && is_active(&t) {
        return Err(ApiError(StatusCode::CONFLICT, "Cancel this task before reassigning it: its agent is working on it.".into()));
    }
    let before = t.clone();
    apply_fields(&state, &mut t, &p).map_err(bad)?;
    if let Some(s) = p.get("status").and_then(|v| v.as_str())
        && s != t.status
    {
        if !USER_STATUSES.contains(&s) {
            return Err(bad(format!("status can be set to {} here; use execute or cancel for the rest", USER_STATUSES.join(" | "))));
        }
        if is_active(&t) {
            return Err(ApiError(StatusCode::CONFLICT, "An agent is working on this task. Cancel it first.".into()));
        }
        t.status = s.into();
        t.completed_at = (s == "done").then(now);
    }
    t.updated_at = now();
    state.db.save_task(&t)?;
    emit(&state, &t);
    if t.status != before.status {
        let text = match t.status.as_str() {
            "done" => "Marked done by you".to_string(),
            "inbox" if is_closed(&before) => "Reopened by you".to_string(),
            s => format!("Moved to {} by you", s.replace('_', " ")),
        };
        log_status(&state, &t, "user", &text);
    }
    if t.agent_id != before.agent_id {
        let text = match t.agent_id.as_deref().and_then(|id| state.db.get_agent(id).ok().flatten()) {
            Some(a) => format!("Assigned to {} by you", a.name),
            None => "Unassigned by you".to_string(),
        };
        log_status(&state, &t, "user", &text);
    }
    Ok(Json(json!(t)))
}

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let t = state.db.get_task(&id)?.ok_or_else(not_found)?;
    if is_active(&t) {
        return Err(ApiError(StatusCode::CONFLICT, "An agent is working on this task. Cancel it before deleting it.".into()));
    }
    // Subtasks and dependents outlive it; drop the dangling references.
    for mut other in state.db.list_tasks()? {
        let before = (other.parent_id.clone(), other.depends_on.len());
        if other.parent_id.as_deref() == Some(&id) {
            other.parent_id = None;
        }
        other.depends_on.retain(|d| *d != id);
        if before != (other.parent_id.clone(), other.depends_on.len()) {
            other.updated_at = now();
            state.db.save_task(&other)?;
            emit(&state, &other);
        }
    }
    state.db.delete_task(&id)?;
    state.emit(Event::TaskDeleted { task_id: id });
    Ok(Json(json!({ "ok": true })))
}

// ---- task references in messages ----------------------------------------

/// Task ids a message points at: the one attached (`meta.task_id`) first, then task links in the
/// text (`…/tasks/<id>` or `loft://task/<id>`), existing tasks only, at most 5.
pub fn references(state: &AppState, attached: Option<&str>, text: &str) -> Vec<String> {
    let mut ids: Vec<String> = attached.into_iter().map(|s| s.trim().to_lowercase()).collect();
    for marker in ["/tasks/", "loft://task/"] {
        let mut rest = text;
        while let Some(i) = rest.find(marker) {
            let after = &rest[i + marker.len()..];
            let cand: String = after.chars().take(36).collect();
            if uuid::Uuid::parse_str(&cand).is_ok() {
                ids.push(cand.to_lowercase());
            }
            rest = &rest[i + marker.len()..];
        }
    }
    let mut out: Vec<String> = vec![];
    for id in ids {
        if !out.contains(&id) && state.db.get_task(&id).ok().flatten().is_some() {
            out.push(id);
        }
        if out.len() == 5 {
            break;
        }
    }
    out
}

/// The short header an agent gets for each task a message refers to.
pub fn reference_header(state: &AppState, ids: &[String]) -> String {
    ids.iter()
        .filter_map(|id| state.db.get_task(id).ok().flatten())
        .map(|t| {
            let mut s = format!("[The user refers to task {} “{}” (status: {}", t.id, t.title, t.status.replace('_', " "));
            if let Some(a) = t.agent_id.as_deref().and_then(|a| state.db.get_agent(a).ok().flatten()) {
                s += &format!(", agent: {}", a.name);
            }
            s += ")";
            for (k, v) in [("latest note", &t.note), ("summary", &t.summary), ("problem", &t.error)] {
                if !v.trim().is_empty() {
                    s += &format!(". {}: {}", capitalize(k), clip(v.lines().next().unwrap_or(""), 300));
                }
            }
            s + ". `mcp__loft__get_task` shows its details and thread.]"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- plans ----------------------------------------------------------------

/// A task and its subtasks as one plan: who's on what, how far along, and the PRs opened for each
/// (`GET /api/tasks/{id}/plan`, and the plan card in the delegating agent's chat).
pub fn plan_view(state: &AppState, id: &str) -> Result<Value, ApiError> {
    let parent = state.db.get_task(id)?.ok_or_else(not_found)?;
    let mut subs: Vec<Task> = state.db.list_tasks()?.into_iter().filter(|t| t.parent_id.as_deref() == Some(id)).collect();
    subs.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let agents = state.db.list_agents()?;
    let prs = state.db.list_pull_requests(None)?;
    let agent = |id: &Option<String>| id.as_deref().and_then(|id| agents.iter().find(|a| a.id == id)).map(|a| json!({ "id": a.id, "name": a.name }));
    let rows: Vec<Value> = subs
        .iter()
        .map(|t| {
            // PRs its agent opened while working on it: the closest link a PR has to a task.
            let from = t.started_at.clone().unwrap_or_else(|| t.created_at.clone());
            let to = t.completed_at.clone().unwrap_or_else(|| "9999".into());
            let task_prs: Vec<Value> = prs
                .iter()
                .filter(|p| t.agent_id.as_deref() == Some(p.agent_id.as_str()) && p.created_at >= from && p.created_at <= to)
                .map(|p| json!({ "id": p.id, "slug": p.slug, "number": p.number, "title": p.title, "url": p.url, "state": p.state }))
                .collect();
            json!({
                "id": t.id, "title": t.title, "status": t.status, "agent": agent(&t.agent_id),
                "note": t.note, "summary": t.summary, "error": t.error,
                "updated_at": t.updated_at, "started_at": t.started_at, "completed_at": t.completed_at,
                "depends_on": t.depends_on, "prs": task_prs,
            })
        })
        .collect();
    let count = |st: &[&str]| subs.iter().filter(|t| st.contains(&t.status.as_str())).count();
    let total = subs.iter().filter(|t| t.status != "cancelled").count();
    let done = count(&["done"]);
    Ok(json!({
        "task": { "id": parent.id, "title": parent.title, "status": parent.status, "agent": agent(&parent.agent_id), "summary": parent.summary },
        "subtasks": rows,
        "progress": {
            "total": total, "done": done,
            "in_progress": count(&["in_progress"]), "waiting": count(&["waiting_approval", "blocked"]),
            "failed": count(&["failed"]), "todo": count(&["inbox", "ready"]), "cancelled": count(&["cancelled"]),
            "percent": (done * 100).checked_div(total).unwrap_or(0),
        },
    }))
}

/// `GET /api/tasks/{id}/plan`
pub async fn plan(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    plan_view(&state, &id).map(Json)
}

/// The first time an agent splits a task into subtasks, one plan card goes into its chat; clients keep
/// it live from the plan endpoint and task events. A `notice` row, so older clients show a plain line.
fn ensure_plan_card(state: &AppState, me: &Agent, parent_id: &str) {
    let Ok(Some(parent)) = state.db.get_task(parent_id) else { return };
    let recent = state.db.list_messages(&me.id, 300, None).unwrap_or_default();
    if recent.iter().any(|m| m.meta["plan_task_id"] == json!(parent_id)) {
        return;
    }
    agents::add_msg(state, &me.id, None, "system", "notice", &format!("Plan: {}", parent.title), json!({ "plan_task_id": parent_id }));
}

#[derive(Deserialize)]
pub struct CommentsQuery {
    limit: Option<i64>,
    /// A comment id; returns older entries, for paging.
    before: Option<i64>,
}

/// `GET /api/tasks/{id}/comments`: the task's thread, oldest first (the newest `limit`, default 100).
pub async fn list_comments(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<CommentsQuery>) -> Result<Json<Value>, ApiError> {
    state.db.get_task(&id)?.ok_or_else(not_found)?;
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    Ok(Json(json!(state.db.list_task_comments(&id, limit, q.before)?)))
}

#[derive(Deserialize)]
pub struct CommentBody {
    text: String,
    /// Set by clients so a retried post isn't added twice.
    client_id: Option<String>,
    /// Deliver it to the task's agent (default). False keeps it as a private note on the task.
    #[serde(default = "yes")]
    send: bool,
    /// On a finished task: reopen it and run it again with this comment as the instruction.
    #[serde(default)]
    reopen: bool,
}

fn yes() -> bool {
    true
}

/// `POST /api/tasks/{id}/comments`: the user comments on a task.
pub async fn add_comment(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<CommentBody>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let t = state.db.get_task(&id)?.ok_or_else(not_found)?;
    let text = b.text.trim();
    if text.is_empty() {
        return Err(bad("A comment can't be empty."));
    }
    let client_id = b.client_id.map(|c| c.trim().chars().take(64).collect::<String>()).filter(|c| !c.is_empty());
    if let Some(c) = &client_id
        && let Some(existing) = state.db.find_task_comment_by_client(&id, c)?
    {
        let found = state.db.list_task_comments(&id, 1, Some(existing + 1))?;
        return Ok((StatusCode::OK, Json(json!({ "comment": found.first(), "duplicate": true }))));
    }
    let agent = t.agent_id.as_deref().and_then(|a| state.db.get_agent(a).ok().flatten());
    if b.reopen {
        // Reopening runs the task again with the comment as its instruction; `execute` keeps it in the thread.
        if !(is_closed(&t) || t.status == "failed") {
            return Err(ApiError(StatusCode::CONFLICT, "Only a finished task can be reopened.".into()));
        }
        if agent.is_none() {
            return Err(bad("This task has no agent. Run it with an agent instead."));
        }
        let mut reopened = t.clone();
        reopened.status = "ready".into();
        state.db.save_task(&reopened)?;
        log_status(&state, &reopened, "user", "Reopened by you");
        let (t, queued) = execute(&state, &t.id, None, text, None)?;
        let c = state.db.list_task_comments(&t.id, 1, None)?.pop();
        return Ok((StatusCode::CREATED, Json(json!({ "comment": c, "task": t, "queued": queued, "delivered_to": t.agent_id }))));
    }
    let deliver = b.send && agent.is_some();
    let mut meta = client_id.map_or(json!({}), |c| json!({ "client_id": c }));
    meta["private"] = json!(!deliver);
    if let (true, Some(a)) = (deliver, &agent) {
        meta["to_agent"] = json!(a.id);
    }
    let c = log(&state, &t, "user", "comment", text, meta).ok_or_else(|| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "couldn't save the comment".into()))?;
    let mut queued = false;
    if let (true, Some(a)) = (deliver, &agent) {
        queued = is_running(&state, &a.id);
        // The chat keeps what you wrote, as a task row (title, then your comment); the agent's prompt adds
        // how to answer. Queued behind a running run like any message.
        agents::add_msg(&state, &a.id, None, "user", "text", &format!("[Task {}] {}\n{}", t.id, t.title, text), json!({ "task_id": t.id, "task_role": "comment", "task_comment_id": c.id }));
        agents::wake(&state, &a.id, comment_prompt(&t, text), "user");
    }
    Ok((StatusCode::CREATED, Json(json!({ "comment": c, "queued": queued, "delivered_to": deliver.then(|| t.agent_id.clone()).flatten() }))))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ExecuteBody {
    agent_id: Option<String>,
    note: String,
}

pub async fn execute_task(State(state): State<AppState>, Path(id): Path<String>, body: Option<Json<ExecuteBody>>) -> Result<Json<Value>, ApiError> {
    let b = body.map(|b| b.0).unwrap_or_default();
    let (t, queued) = execute(&state, &id, b.agent_id.as_deref(), &b.note, None)?;
    Ok(Json(json!({ "task": t, "queued": queued })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CancelBody {
    /// Also stop the agent's current run. Off by default: the run may be doing other work too.
    stop_run: bool,
}

/// Mark a task cancelled. The agent is told if it is busy, so it drops the task when it reads its queue;
/// its run is only stopped when `stop_run` is set.
pub async fn cancel(State(state): State<AppState>, Path(id): Path<String>, body: Option<Json<CancelBody>>) -> Result<Json<Value>, ApiError> {
    let b = body.map(|b| b.0).unwrap_or_default();
    let mut t = state.db.get_task(&id)?.ok_or_else(not_found)?;
    if is_closed(&t) {
        return Err(ApiError(StatusCode::CONFLICT, format!("This task is already {}.", t.status)));
    }
    let was_active = is_active(&t);
    let ts = now();
    t.status = "cancelled".into();
    t.completed_at = Some(ts.clone());
    t.updated_at = ts;
    state.db.save_task(&t)?;
    emit(&state, &t);
    log_status(&state, &t, "user", "Cancelled by you");
    let mut stopped = false;
    if let (true, Some(agent_id)) = (was_active, t.agent_id.as_deref()) {
        if b.stop_run {
            stopped = agents::stop(&state, agent_id);
        } else if is_running(&state, agent_id) {
            agents::enqueue(&state, agent_id, Some(format!("[Task {}] {} was cancelled by the user. Stop working on it.", t.id, t.title)), "user");
        }
    }
    Ok(Json(json!({ "task": t, "stopped": stopped })))
}

// ---- activity -----------------------------------------------------------

#[derive(Deserialize)]
pub struct ActivityQuery {
    limit: Option<i64>,
    /// RFC 3339; returns items strictly older, for paging.
    before: Option<String>,
}

/// The cross-agent activity feed: runs, failures, approvals, agent-to-agent
/// messages and task milestones, newest first. Derived from stored history, so
/// it covers whatever happened while no client was connected.
pub async fn activity(State(state): State<AppState>, Query(q): Query<ActivityQuery>) -> Result<Json<Value>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let before = q.before.as_deref();
    let names: std::collections::HashMap<String, String> = state.db.list_agents()?.into_iter().map(|a| (a.id, a.name)).collect();
    let name = |id: &str| names.get(id).cloned().unwrap_or_else(|| "A deleted agent".into());
    let mut items: Vec<Value> = vec![];
    for m in state.db.activity_messages(limit, before)? {
        let who = name(&m.agent_id);
        let (kind, summary, severity) = match m.kind.as_str() {
            "run_start" => {
                let why = match m.content.as_str() {
                    "schedule" => " (scheduled)",
                    "agent" => " (asked by another agent)",
                    _ => "",
                };
                if m.content == "compact" { continue }
                ("agent.started", format!("{who} started working{why}"), "info")
            }
            "run_end" if m.meta["stopped"].as_bool() == Some(true) => ("agent.stopped", format!("{who} was stopped"), "info"),
            "run_end" if m.meta["is_error"].as_bool() == Some(true) => ("agent.failed", format!("{who}'s run ended with an error"), "error"),
            "run_end" => ("agent.completed", format!("{who} finished"), "success"),
            "error" => ("agent.failed", format!("{who} failed: {}", clip(m.content.lines().next().unwrap_or(""), 160)), "error"),
            "permission" => ("approval.required", format!("{who} asked for approval: {}", clip(&m.content, 80)), "attention"),
            _ => {
                let from = m.meta["from_agent"]["name"].as_str().unwrap_or("An agent");
                ("agent.message", format!("{from} messaged {who}: {}", clip(headline(&m.content), 120)), "info")
            }
        };
        items.push(json!({
            "id": format!("msg-{}", m.id),
            "type": kind,
            "at": m.created_at,
            "agent_id": m.agent_id,
            "task_id": m.meta["task_id"].as_str(),
            "message_id": m.id,
            "summary": summary,
            "severity": severity,
        }));
    }
    for t in state.db.list_tasks()? {
        let mut push = |kind: &str, at: &str, summary: String, severity: &str| {
            if before.is_none_or(|b| at < b) {
                items.push(json!({
                    "id": format!("task-{}-{kind}", t.id),
                    "type": kind,
                    "at": at,
                    "agent_id": t.agent_id,
                    "task_id": t.id,
                    "summary": summary,
                    "severity": severity,
                }));
            }
        };
        let by = if t.created_by == "user" { "You".to_string() } else { name(&t.created_by) };
        let what = if t.parent_id.is_some() { "subtask" } else { "task" };
        push("task.created", &t.created_at, format!("{by} created {what} “{}”", t.title), "info");
        if let Some(at) = &t.completed_at {
            match t.status.as_str() {
                "done" => push("task.completed", at, format!("“{}” is done", t.title), "success"),
                "cancelled" => push("task.cancelled", at, format!("“{}” was cancelled", t.title), "info"),
                _ => {}
            }
        }
        match t.status.as_str() {
            "failed" => push("task.failed", &t.updated_at, format!("“{}” failed", t.title), "error"),
            "blocked" => push("task.blocked", &t.updated_at, format!("“{}” is blocked", t.title), "attention"),
            _ => {}
        }
    }
    items.sort_by(|a, b| b["at"].as_str().cmp(&a["at"].as_str()));
    items.truncate(limit as usize);
    Ok(Json(json!(items)))
}

// ---- MCP tools ------------------------------------------------------------

pub fn mcp_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "list_tasks",
            "description": "List Loft tasks: id, status, title, assignee. By default the open tasks assigned to you or created by you.",
            "inputSchema": { "type": "object", "properties": {
                "all": { "type": "boolean", "description": "Every task, not just yours." },
                "include_closed": { "type": "boolean", "description": "Include done and cancelled tasks." }
            } }
        }),
        json!({
            "name": "create_task",
            "description": "Create a task on the user's task list, optionally a subtask (`parent_id`) assigned to another agent. With `start: true` the agent gets it right away; you're messaged when it is done, failed or blocked.",
            "inputSchema": { "type": "object", "properties": {
                "title": { "type": "string" },
                "description": { "type": "string" },
                "agent": { "type": "string", "description": "Id or name of the agent to do it (you, or another agent)." },
                "parent_id": { "type": "string" },
                "depends_on": { "type": "array", "items": { "type": "string" }, "description": "Ids of tasks that must finish first." },
                "priority": { "type": "string", "enum": PRIORITIES },
                "project": { "type": "string" },
                "start": { "type": "boolean", "description": "Hand it to the agent now." }
            }, "required": ["title"] }
        }),
        json!({
            "name": "update_task",
            "description": "Report progress on a task assigned to you (or that you created). `status`: in_progress | blocked | waiting_approval | done | failed. Add a short `note` for progress, a `summary` when done, an `error` when failed or blocked.",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "string" },
                "status": { "type": "string", "enum": AGENT_STATUSES },
                "note": { "type": "string" },
                "summary": { "type": "string" },
                "error": { "type": "string" }
            }, "required": ["id"] }
        }),
        json!({
            "name": "get_task",
            "description": "One task with its thread: comments from the user and agents, progress notes and status changes, oldest first.",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "string" }
            }, "required": ["id"] }
        }),
        json!({
            "name": "comment_task",
            "description": "Comment on a task you're on, created or delegated: answer a comment the user left on it (a message starting with `[Comment on task <id>`), or ask the user something about it. The user sees it on the task (and in this chat).",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "string" },
                "text": { "type": "string" }
            }, "required": ["id", "text"] }
        }),
    ]
}

/// May `me` comment on `t`: assigned to it, created it, or delegated the task it's part of.
fn involved(state: &AppState, t: &Task, me: &Agent) -> bool {
    t.agent_id.as_deref() == Some(&me.id)
        || t.created_by == me.id
        || t.parent_id.as_deref().and_then(|p| state.db.get_task(p).ok().flatten()).is_some_and(|p| p.agent_id.as_deref() == Some(&me.id) || p.created_by == me.id)
}

/// The prompt an agent gets for a comment on its task.
fn comment_prompt(t: &Task, text: &str) -> String {
    format!(
        "[Comment on task {} “{}” (status: {})]\n{}\n\n[Answer on the task with `mcp__loft__comment_task` (id {}); `mcp__loft__get_task` shows its whole thread. If it changes the work, carry on and report with `update_task` as usual.]",
        t.id,
        t.title,
        t.status.replace('_', " "),
        text,
        t.id
    )
}

/// Handle a task MCP tool call. Ok is the tool's text answer, Err its error text.
pub fn mcp_call(state: &AppState, me: &Agent, name: &str, args: &Value, find_agent: impl Fn(&str) -> Result<Agent, String>) -> Option<Result<String, String>> {
    let s = |k: &str| args[k].as_str().unwrap_or("").trim().to_string();
    let names: std::collections::HashMap<String, String> =
        state.db.list_agents().unwrap_or_default().into_iter().map(|a| (a.id, a.name)).collect();
    let line = |t: &Task| {
        let who = t.agent_id.as_deref().and_then(|id| names.get(id)).map(|n| format!(" → {n}")).unwrap_or_default();
        let parent = t.parent_id.as_deref().map(|p| format!(" (subtask of {p})")).unwrap_or_default();
        format!("- {} [{}] {}{who}{parent}", t.id, t.status, t.title)
    };
    Some(match name {
        "list_tasks" => (|| {
            let all = args["all"].as_bool().unwrap_or(false);
            let closed = args["include_closed"].as_bool().unwrap_or(false);
            let tasks = state.db.list_tasks().map_err(|e| e.to_string())?;
            let lines: Vec<String> = tasks
                .iter()
                .filter(|t| all || t.agent_id.as_deref() == Some(&me.id) || t.created_by == me.id)
                .filter(|t| closed || !is_closed(t))
                .map(line)
                .collect();
            Ok(if lines.is_empty() { "No tasks.".into() } else { lines.join("\n") })
        })(),
        "create_task" => (|| {
            let assignee = match s("agent") {
                a if a.is_empty() => None,
                a => Some(find_agent(&a)?),
            };
            let mut p = json!({
                "title": s("title"),
                "description": s("description"),
                "kind": "work",
                "agent_id": assignee.as_ref().map(|a| a.id.clone()),
                "depends_on": args.get("depends_on").cloned().unwrap_or(json!([])),
            });
            for k in ["parent_id", "priority", "project"] {
                if !s(k).is_empty() {
                    p[k] = json!(s(k));
                }
            }
            if s("project").is_empty() && !me.project.is_empty() {
                p["project"] = json!(me.project);
            }
            let (t, _) = create(state, &p, &me.id)?;
            if let Some(parent) = t.parent_id.as_deref() {
                ensure_plan_card(state, me, parent);
            }
            match assignee {
                Some(a) if args["start"].as_bool() == Some(true) => {
                    let by = (a.id != me.id).then_some(me);
                    let (t, queued) = execute(state, &t.id, Some(&a.id), "", by).map_err(|e| e.1)?;
                    Ok(format!("Created and started task {}. {} {}.", t.id, a.name, if queued { "will pick it up after its current run" } else { "is on it now" }))
                }
                _ => Ok(format!("Created task {}.", t.id)),
            }
        })(),
        "get_task" => (|| {
            let t = state.db.get_task(&s("id")).map_err(|e| e.to_string())?.ok_or("No task with that id. Call list_tasks.")?;
            let mut out = vec![line(&t)];
            for (k, v) in [("Description", &t.description), ("Latest note", &t.note), ("Summary", &t.summary), ("Problem", &t.error)] {
                if !v.trim().is_empty() {
                    out.push(format!("{k}: {}", v.trim()));
                }
            }
            let thread = state.db.list_task_comments(&t.id, 40, None).map_err(|e| e.to_string())?;
            if !thread.is_empty() {
                out.push("Thread:".into());
                for c in thread {
                    let by = if c.author == "user" { "user".to_string() } else { c.meta["author_name"].as_str().unwrap_or("agent").to_string() };
                    out.push(format!("- {} {} ({}): {}", &c.created_at[..16], by, c.kind, c.text));
                }
            }
            Ok(out.join("\n"))
        })(),
        "comment_task" => (|| {
            let t = state.db.get_task(&s("id")).map_err(|e| e.to_string())?.ok_or("No task with that id. Call list_tasks.")?;
            if !involved(state, &t, me) {
                return Err("You can only comment on tasks you're on, created or delegated.".to_string());
            }
            let text = s("text");
            if text.trim().is_empty() {
                return Err("text is empty".to_string());
            }
            log(state, &t, &me.id, "comment", text.trim(), json!({})).ok_or("Couldn't save the comment.")?;
            Ok(format!("Commented on task {}.", t.id))
        })(),
        "update_task" => (|| {
            let mut t = state.db.get_task(&s("id")).map_err(|e| e.to_string())?.ok_or("No task with that id. Call list_tasks.")?;
            if t.agent_id.as_deref() != Some(&me.id) && t.created_by != me.id {
                return Err("That task isn't assigned to you or created by you.".to_string());
            }
            if t.status == "cancelled" {
                return Err("The user cancelled this task. Stop working on it.".to_string());
            }
            let status = s("status");
            let before = t.status.clone();
            if !status.is_empty() {
                if !AGENT_STATUSES.contains(&status.as_str()) {
                    return Err(format!("status must be one of {}", AGENT_STATUSES.join(" | ")));
                }
                if status != t.status {
                    t.status = status.clone();
                    if t.started_at.is_none() {
                        t.started_at = Some(now());
                    }
                    t.completed_at = (status == "done").then(now);
                }
            }
            if !s("note").is_empty() { t.note = clip(&s("note"), 500); }
            if !s("summary").is_empty() { t.summary = clip(&s("summary"), 4000); }
            if !s("error").is_empty() { t.error = clip(&s("error"), 2000); }
            if status == "done" || status == "in_progress" { t.error = String::new(); }
            t.updated_at = now();
            state.db.save_task(&t).map_err(|e| e.to_string())?;
            emit(state, &t);
            // Every note is kept in the thread; `note` on the task is only the latest.
            if !s("note").is_empty() {
                log(state, &t, &me.id, "note", &s("note"), json!({}));
            }
            if t.status != before {
                let text = match t.status.as_str() {
                    "done" => format!("Done, by {}", me.name),
                    "failed" => format!("Failed, reported by {}", me.name),
                    "blocked" => format!("Blocked, reported by {}", me.name),
                    s => format!("{} by {}", capitalize(&s.replace('_', " ")), me.name),
                };
                let mut meta = json!({ "status": t.status });
                for k in ["summary", "error"] {
                    if !s(k).is_empty() {
                        meta[k] = json!(clip(&s(k), MAX_COMMENT));
                    }
                }
                log(state, &t, &me.id, "status", &text, meta);
            }
            if ["done", "failed", "blocked"].contains(&status.as_str()) {
                tell_delegator(state, &t, me);
            }
            Ok(format!("Task {} is {}.", t.id, t.status))
        })(),
        _ => return None,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    pub(crate) fn state() -> AppState {
        let dir = std::env::temp_dir().join(format!("loft-test-{}", uuid::Uuid::new_v4()));
        // Boot with a temp projects folder so no test writes into the real ~/projects.
        let projects = dir.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(dir.join("settings.json"), json!({ "projects_dir": projects }).to_string()).unwrap();
        AppState {
            db: crate::db::Db::open(&dir.join("loft.db")).unwrap(),
            cfg: Arc::new(crate::state::Config { port: 0, storage_dir: dir.join("agents") }),
            events: tokio::sync::broadcast::channel(64).0,
            rt: Arc::new(Mutex::new(Default::default())),
            repos: Arc::new(Default::default()),
            http: reqwest::Client::new(),
            gh_token: Arc::new(Default::default()),
            update: Arc::new(Mutex::new(Default::default())),
            settings: Arc::new(crate::settings::Store::load(&dir).unwrap()),
            usage: Arc::new(Default::default()),
            system: Arc::new(Default::default()),
        }
    }

    /// A saved agent with this name.
    pub(crate) fn agent(st: &AppState, name: &str) -> Agent {
        let a: Agent = serde_json::from_value(json!({
            "id": uuid::Uuid::new_v4().to_string(), "name": name, "title": "", "personality": "", "prompt": "",
            "model": "haiku", "effort": "", "interval_minutes": 0, "allowed_tools": [], "disallowed_tools": [],
            "mcp_servers": {}, "permission_mode": "auto", "avatar": { "svg": "<svg/>" }, "status": "idle",
            "status_text": "", "session_id": null, "last_run_at": null, "next_run_at": null, "total_cost_usd": 0.0,
            "created_at": now(),
        }))
        .unwrap();
        st.db.save_agent(&a).unwrap();
        a
    }

    #[tokio::test]
    async fn the_thread_keeps_every_note_comment_and_status_change() {
        let st = state();
        let a = agent(&st, "Worker");
        let (t, _) = create(&st, &json!({ "title": "Fix it" }), "user").unwrap();
        // Busy, so the brief is queued instead of starting a real claude run.
        st.rt.lock().unwrap().running.insert(a.id.clone(), std::sync::Arc::new(tokio::sync::Notify::new()));
        // Run it with a note: the instruction is kept as your comment.
        execute(&st, &t.id, Some(&a.id), "Start with the login screen", None).unwrap();
        let find = |k: &str| Err::<Agent, _>(k.to_string());
        for note in ["looked at it", "halfway"] {
            mcp_call(&st, &a, "update_task", &json!({ "id": t.id, "note": note }), find).unwrap().unwrap();
        }
        mcp_call(&st, &a, "update_task", &json!({ "id": t.id, "status": "done", "summary": "Fixed." }), find).unwrap().unwrap();
        let (_, Json(c)) = add_comment(State(st.clone()), Path(t.id.clone()), Json(CommentBody { text: "Thanks!".into(), client_id: Some("c1".into()), send: false, reopen: false })).await.unwrap();
        let c = c["comment"].clone();
        // A retried post isn't added twice.
        let (code, Json(again)) = add_comment(State(st.clone()), Path(t.id.clone()), Json(CommentBody { text: "Thanks!".into(), client_id: Some("c1".into()), send: false, reopen: false })).await.unwrap();
        assert_eq!((code, again["comment"]["id"].clone()), (StatusCode::OK, c["id"].clone()));
        assert!(add_comment(State(st.clone()), Path(t.id.clone()), Json(CommentBody { text: "  ".into(), client_id: None, send: true, reopen: false })).await.is_err());

        let Json(thread) = list_comments(State(st.clone()), Path(t.id.clone()), Query(CommentsQuery { limit: None, before: None })).await.unwrap();
        let rows: Vec<(String, String)> = thread.as_array().unwrap().iter().map(|c| (c["kind"].as_str().unwrap().into(), c["text"].as_str().unwrap().into())).collect();
        let want: Vec<(&str, &str)> = vec![
            ("status", "Created by you"),
            ("status", "Sent to Worker by you"),
            ("comment", "Start with the login screen"),
            ("note", "looked at it"),
            ("note", "halfway"),
            ("status", "Done, by Worker"),
            ("comment", "Thanks!"),
        ];
        assert_eq!(rows, want.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>());
        assert_eq!(thread[3]["author"], json!(a.id));
        assert_eq!(thread[3]["meta"]["author_name"], json!("Worker"));
        assert_eq!(thread[5]["meta"]["summary"], json!("Fixed."));
        // `note` on the task is still the latest one.
        assert_eq!(st.db.get_task(&t.id).unwrap().unwrap().note, "halfway");
        // Paging: the two before the last.
        let Json(page) = list_comments(State(st.clone()), Path(t.id.clone()), Query(CommentsQuery { limit: Some(2), before: thread[6]["id"].as_i64() })).await.unwrap();
        assert_eq!(page.as_array().unwrap().iter().map(|c| c["text"].as_str().unwrap()).collect::<Vec<_>>(), ["halfway", "Done, by Worker"]);
    }

    fn comment(text: &str, send: bool, reopen: bool) -> Json<CommentBody> {
        Json(CommentBody { text: text.into(), client_id: None, send, reopen })
    }

    /// An agent marked busy, so whatever reaches it is queued instead of starting a real claude run.
    fn busy(st: &AppState, a: &Agent) {
        st.rt.lock().unwrap().running.insert(a.id.clone(), std::sync::Arc::new(tokio::sync::Notify::new()));
    }

    fn queued(st: &AppState, a: &Agent) -> String {
        st.rt.lock().unwrap().queued.get(&a.id).map(|q| q.join("\n---\n")).unwrap_or_default()
    }

    #[tokio::test]
    async fn a_comment_reaches_the_tasks_agent_tagged_with_the_task() {
        let st = state();
        let a = agent(&st, "Worker");
        busy(&st, &a);
        let (t, _) = create(&st, &json!({ "title": "Fix login", "agent_id": a.id }), "user").unwrap();
        execute(&st, &t.id, None, "", None).unwrap();
        st.rt.lock().unwrap().queued.remove(&a.id);
        let (_, Json(r)) = add_comment(State(st.clone()), Path(t.id.clone()), comment("Also check the iPad layout", true, false)).await.unwrap();
        assert_eq!((r["queued"].clone(), r["delivered_to"].clone()), (json!(true), json!(a.id)));
        let q = queued(&st, &a);
        assert!(q.starts_with(&format!("[Comment on task {} “Fix login” (status: in progress)]\nAlso check the iPad layout", t.id)), "{q}");
        assert!(q.contains("mcp__loft__comment_task"));
        // In the agent's chat it's a task row with what you wrote, not the instructions for the agent.
        let m = st.db.list_messages(&a.id, 10, None).unwrap().into_iter().last().unwrap();
        assert_eq!((m.meta["task_id"].clone(), m.meta["task_role"].clone()), (json!(t.id), json!("comment")));
        assert_eq!(m.content, format!("[Task {}] Fix login\nAlso check the iPad layout", t.id));
        let c = st.db.list_task_comments(&t.id, 50, None).unwrap().pop().unwrap();
        assert_eq!((c.text.as_str(), c.meta["to_agent"].clone(), c.meta["private"].clone()), ("Also check the iPad layout", json!(a.id), json!(false)));

        // A private note stays on the task.
        let (_, Json(r)) = add_comment(State(st.clone()), Path(t.id.clone()), comment("Remember to ask Tom", false, false)).await.unwrap();
        assert_eq!(r["delivered_to"], Value::Null);
        assert!(!queued(&st, &a).contains("Remember to ask Tom"));
        assert_eq!(st.db.list_task_comments(&t.id, 50, None).unwrap().pop().unwrap().meta["private"], json!(true));
    }

    #[tokio::test]
    async fn the_agent_answers_on_the_task_and_strangers_cannot() {
        let st = state();
        let (lead, worker, stranger) = (agent(&st, "Lead"), agent(&st, "Worker"), agent(&st, "Stranger"));
        let (parent, _) = create(&st, &json!({ "title": "Ship it", "agent_id": lead.id }), "user").unwrap();
        let (t, _) = create(&st, &json!({ "title": "Tests", "agent_id": worker.id, "parent_id": parent.id }), &lead.id).unwrap();
        let find = |k: &str| Err::<Agent, _>(k.to_string());
        mcp_call(&st, &worker, "comment_task", &json!({ "id": t.id, "text": "Done with the iPad layout too." }), find).unwrap().unwrap();
        mcp_call(&st, &lead, "comment_task", &json!({ "id": t.id, "text": "Thanks" }), find).unwrap().unwrap();
        assert!(mcp_call(&st, &stranger, "comment_task", &json!({ "id": t.id, "text": "hi" }), find).unwrap().is_err());
        assert!(mcp_call(&st, &worker, "comment_task", &json!({ "id": t.id, "text": " " }), find).unwrap().is_err());
        let thread = st.db.list_task_comments(&t.id, 50, None).unwrap();
        let last: Vec<_> = thread.iter().rev().take(2).map(|c| (c.author.clone(), c.text.clone())).collect();
        assert_eq!(last, [(lead.id.clone(), "Thanks".to_string()), (worker.id.clone(), "Done with the iPad layout too.".to_string())]);
        let view = mcp_call(&st, &stranger, "get_task", &json!({ "id": t.id }), find).unwrap().unwrap();
        assert!(view.contains("Worker (comment): Done with the iPad layout too.") && view.contains("Created by Lead"), "{view}");
    }

    #[tokio::test]
    async fn reopen_and_send_runs_a_finished_task_again_with_the_comment() {
        let st = state();
        let a = agent(&st, "Worker");
        busy(&st, &a);
        let (t, _) = create(&st, &json!({ "title": "Fix login", "agent_id": a.id }), "user").unwrap();
        // Not finished yet: nothing to reopen.
        assert!(add_comment(State(st.clone()), Path(t.id.clone()), comment("again", true, true)).await.is_err());
        let _ = update(State(st.clone()), Path(t.id.clone()), Json(json!({ "status": "done" }))).await.unwrap();
        let (_, Json(r)) = add_comment(State(st.clone()), Path(t.id.clone()), comment("The fix broke sign-up, please look", true, true)).await.unwrap();
        assert_eq!(r["task"]["status"], json!("in_progress"));
        assert!(queued(&st, &a).contains("The fix broke sign-up, please look"), "the brief carries the comment");
        let texts: Vec<String> = st.db.list_task_comments(&t.id, 50, None).unwrap().into_iter().map(|c| c.text).collect();
        assert_eq!(texts[texts.len() - 3..], ["Reopened by you", "Sent to Worker by you", "The fix broke sign-up, please look"]);
        // A task with nobody on it can't be reopened that way.
        let (u, _) = create(&st, &json!({ "title": "Solo" }), "user").unwrap();
        let _ = update(State(st.clone()), Path(u.id.clone()), Json(json!({ "status": "done" }))).await.unwrap();
        assert!(add_comment(State(st.clone()), Path(u.id.clone()), comment("again", true, true)).await.is_err());
        assert_eq!(st.db.get_task(&u.id).unwrap().unwrap().status, "done", "unchanged");
    }

    #[tokio::test]
    async fn delegating_posts_one_live_plan_card_with_progress_and_prs() {
        let st = state();
        let (orch, dev, qa) = (agent(&st, "Orchestrator"), agent(&st, "Dev"), agent(&st, "QA"));
        for a in [&dev, &qa] {
            busy(&st, a);
        }
        let (parent, _) = create(&st, &json!({ "title": "Ship settings", "agent_id": orch.id }), "user").unwrap();
        let find = |k: &str| st.db.list_agents().unwrap().into_iter().find(|a| a.name == k).ok_or("none".to_string());
        let mut ids = vec![];
        for (title, who) in [("Backend endpoints", "Dev"), ("iOS screen", "Dev"), ("Test it", "QA")] {
            let r = mcp_call(&st, &orch, "create_task", &json!({ "title": title, "agent": who, "parent_id": parent.id, "start": true }), find).unwrap().unwrap();
            ids.push(r.split_whitespace().find(|w| w.len() == 37 || w.len() == 36).unwrap().trim_end_matches('.').to_string());
        }
        let cards: Vec<_> = st.db.list_messages(&orch.id, 50, None).unwrap().into_iter().filter(|m| m.meta["plan_task_id"] == json!(parent.id)).collect();
        assert_eq!(cards.len(), 1, "one card per plan, however many subtasks");
        assert_eq!((cards[0].kind.as_str(), cards[0].content.as_str()), ("notice", "Plan: Ship settings"));

        // Dev finishes the first one after opening a PR; QA is blocked.
        let pr = crate::db::PullRequest { id: "pr1".into(), agent_id: dev.id.clone(), slug: "o/r".into(), number: 9, url: "https://github.com/o/r/pull/9".into(), title: "Endpoints".into(), state: "open".into(), created_at: now(), ..Default::default() };
        st.db.save_pull_request(&pr).unwrap();
        let none = |k: &str| Err::<Agent, _>(k.to_string());
        mcp_call(&st, &dev, "update_task", &json!({ "id": ids[0], "status": "done", "summary": "4 endpoints" }), none).unwrap().unwrap();
        mcp_call(&st, &qa, "update_task", &json!({ "id": ids[2], "status": "blocked", "error": "Needs the build" }), none).unwrap().unwrap();

        let Json(v) = plan(State(st.clone()), Path(parent.id.clone())).await.unwrap();
        assert_eq!(v["progress"], json!({ "total": 3, "done": 1, "in_progress": 1, "waiting": 1, "failed": 0, "todo": 0, "cancelled": 0, "percent": 33 }));
        let rows = v["subtasks"].as_array().unwrap();
        assert_eq!(rows.iter().map(|r| r["title"].as_str().unwrap()).collect::<Vec<_>>(), ["Backend endpoints", "iOS screen", "Test it"]);
        assert_eq!(rows[0]["prs"][0]["number"], json!(9), "the PR opened while it was being worked on");
        assert_eq!(rows[2]["agent"]["name"], json!("QA"));
        assert_eq!(rows[2]["error"], json!("Needs the build"));
        assert!(plan(State(st.clone()), Path("nope".into())).await.is_err());
    }

    #[tokio::test]
    async fn an_attached_task_or_task_link_gives_the_agent_a_header() {
        let st = state();
        let a = agent(&st, "Worker");
        busy(&st, &a);
        let (t, _) = create(&st, &json!({ "title": "Fix login", "agent_id": a.id }), "user").unwrap();
        let find = |k: &str| Err::<Agent, _>(k.to_string());
        mcp_call(&st, &a, "update_task", &json!({ "id": t.id, "status": "in_progress", "note": "halfway" }), find).unwrap().unwrap();
        let send = |text: &str, task: Option<&str>| crate::api::send_message(
            State(st.clone()),
            Path(a.id.clone()),
            Json(serde_json::from_value(json!({ "text": text, "meta": task.map(|t| json!({ "task_id": t })) })).unwrap()),
        );
        // Attached: stored on the message, and the agent's prompt carries the header.
        let Json(r) = send("Can you look at this one?", Some(&t.id.to_uppercase())).await.ok().unwrap();
        assert_eq!((r["message"]["meta"]["task_id"].clone(), r["message"]["meta"]["task_refs"].clone()), (json!(t.id), json!([t.id])));
        let q = queued(&st, &a);
        assert!(q.contains(&format!("[The user refers to task {} “Fix login” (status: in progress, agent: Worker). Latest note: halfway.", t.id)), "{q}");
        // A link in the text works too; an unknown attached task is refused; unknown links are ignored.
        st.rt.lock().unwrap().queued.remove(&a.id);
        let Json(r) = send(&format!("see http://mac:7420/tasks/{} and loft://task/{}", t.id, uuid::Uuid::new_v4()), None).await.ok().unwrap();
        assert_eq!((r["message"]["meta"]["task_refs"].clone(), r["message"]["meta"]["task_id"].clone()), (json!([t.id]), Value::Null));
        assert!(send("hi", Some(&uuid::Uuid::new_v4().to_string())).await.is_err());
        let Json(r) = send("plain message", None).await.ok().unwrap();
        assert_eq!(r["message"]["meta"]["task_refs"], Value::Null);
    }

    #[tokio::test]
    async fn user_changes_are_in_the_thread_and_deleting_the_task_clears_it() {
        let st = state();
        let a = agent(&st, "Worker");
        let (t, _) = create(&st, &json!({ "title": "Tidy" }), "user").unwrap();
        let _ = update(State(st.clone()), Path(t.id.clone()), Json(json!({ "agent_id": a.id }))).await.unwrap();
        let _ = update(State(st.clone()), Path(t.id.clone()), Json(json!({ "status": "done" }))).await.unwrap();
        let _ = update(State(st.clone()), Path(t.id.clone()), Json(json!({ "status": "inbox" }))).await.unwrap();
        let texts: Vec<String> = st.db.list_task_comments(&t.id, 50, None).unwrap().into_iter().map(|c| c.text).collect();
        assert_eq!(texts, ["Created by you", "Assigned to Worker by you", "Marked done by you", "Reopened by you"]);
        let _ = cancel(State(st.clone()), Path(t.id.clone()), None).await.unwrap();
        assert_eq!(st.db.list_task_comments(&t.id, 50, None).unwrap().last().unwrap().text, "Cancelled by you");
        let _ = delete(State(st.clone()), Path(t.id.clone())).await.unwrap();
        assert!(st.db.list_task_comments(&t.id, 50, None).unwrap().is_empty());
    }

    #[test]
    fn headline_strips_task_tag() {
        assert_eq!(headline("[Task 123e4567-e89b-12d3-a456-426614174000] Fix it is now done\nmore"), "Fix it is now done");
        assert_eq!(headline("plain text\nsecond"), "plain text");
    }

    #[test]
    fn create_defaults_and_validation() {
        let st = state();
        let (t, created) = create(&st, &json!({ "title": "  Buy milk " }), "user").unwrap();
        assert!(created);
        assert_eq!((t.title.as_str(), t.status.as_str(), t.kind.as_str(), t.priority.as_str()), ("Buy milk", "inbox", "todo", "normal"));
        assert!(create(&st, &json!({ "title": "" }), "user").is_err());
        assert!(create(&st, &json!({ "title": "x", "priority": "asap" }), "user").is_err());
        assert!(create(&st, &json!({ "title": "x", "status": "done" }), "user").is_err());
        assert!(create(&st, &json!({ "title": "x", "agent_id": "nope" }), "user").is_err());
        assert!(create(&st, &json!({ "title": "x", "parent_id": "nope" }), "user").is_err());
        assert!(create(&st, &json!({ "title": "x", "due_at": "tomorrow" }), "user").is_err());
    }

    #[test]
    fn create_is_idempotent_by_client_id() {
        let st = state();
        let id = uuid::Uuid::new_v4().to_string();
        let (a, first) = create(&st, &json!({ "id": id, "title": "Once" }), "user").unwrap();
        let (b, second) = create(&st, &json!({ "id": id, "title": "Twice" }), "user").unwrap();
        assert!(first && !second);
        assert_eq!((a.id, b.title.as_str()), (b.id.clone(), "Once"));
        assert_eq!(st.db.list_tasks().unwrap().len(), 1);
        assert!(create(&st, &json!({ "id": "not-a-uuid", "title": "x" }), "user").is_err());
    }

    #[test]
    fn parent_cycles_are_rejected() {
        let st = state();
        let (a, _) = create(&st, &json!({ "title": "A" }), "user").unwrap();
        let (b, _) = create(&st, &json!({ "title": "B", "parent_id": a.id }), "user").unwrap();
        let mut a2 = a.clone();
        assert!(apply_fields(&st, &mut a2, &json!({ "parent_id": b.id })).is_err());
        assert!(apply_fields(&st, &mut a2, &json!({ "parent_id": a.id })).is_err());
        assert!(apply_fields(&st, &mut a2, &json!({ "depends_on": [a.id] })).is_err());
        assert!(apply_fields(&st, &mut a2, &json!({ "depends_on": [b.id] })).is_ok());
    }

    #[test]
    fn agents_report_only_on_their_own_tasks() {
        let st = state();
        let mk = |name: &str| {
            let mut a: Agent = serde_json::from_value(json!({
                "id": uuid::Uuid::new_v4().to_string(), "name": name, "title": "", "personality": "", "prompt": "",
                "model": "haiku", "effort": "", "interval_minutes": 0, "allowed_tools": [], "disallowed_tools": [],
                "mcp_servers": {}, "permission_mode": "auto", "avatar": { "svg": "<svg/>" }, "status": "idle",
                "status_text": "", "session_id": null, "last_run_at": null, "next_run_at": null, "total_cost_usd": 0.0,
                "created_at": now(),
            }))
            .unwrap();
            a.project = "p".into();
            st.db.save_agent(&a).unwrap();
            a
        };
        let (me, other) = (mk("Me"), mk("Other"));
        let find = |k: &str| st.db.list_agents().unwrap().into_iter().find(|a| a.name == k || a.id == k).ok_or("none".to_string());
        // Created without starting, so no agent process is launched.
        let r = mcp_call(&st, &me, "create_task", &json!({ "title": "Sub", "agent": "Other" }), find).unwrap().unwrap();
        let id = r.split_whitespace().nth(2).unwrap().trim_end_matches('.').to_string();
        let t = st.db.get_task(&id).unwrap().unwrap();
        assert_eq!((t.created_by.as_str(), t.agent_id.as_deref(), t.status.as_str(), t.project.as_str()), (me.id.as_str(), Some(other.id.as_str()), "ready", "p"));

        let stranger = mk("Stranger");
        let find = |k: &str| Err::<Agent, _>(k.to_string());
        assert!(mcp_call(&st, &stranger, "update_task", &json!({ "id": id, "status": "done" }), find).unwrap().is_err());
        assert!(mcp_call(&st, &other, "update_task", &json!({ "id": id, "status": "cancelled" }), find).unwrap().is_err());
        assert!(mcp_call(&st, &other, "update_task", &json!({ "id": id, "status": "in_progress", "note": "halfway" }), find).unwrap().is_ok());
        let t = st.db.get_task(&id).unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.note.as_str()), ("in_progress", "halfway"));
        assert!(t.started_at.is_some());
        assert!(mcp_call(&st, &other, "nope", &json!({}), find).is_none());

        // Finishing a delegated subtask reports back to the delegator, tagged with the task.
        // Mark the delegator busy so the report is queued rather than starting a real run.
        st.rt.lock().unwrap().running.insert(me.id.clone(), std::sync::Arc::new(tokio::sync::Notify::new()));
        assert!(mcp_call(&st, &other, "update_task", &json!({ "id": id, "status": "done", "summary": "ok" }), find).unwrap().is_ok());
        let report = st.db.list_messages(&me.id, 10, None).unwrap().into_iter().last().unwrap();
        assert_eq!(report.meta["task_id"], json!(id));
        assert_eq!(report.meta["task_role"], json!("report"));
        assert_eq!(report.meta["task_status"], json!("done"));
        assert_eq!(report.meta["from_agent"]["id"], json!(other.id));
    }
}
