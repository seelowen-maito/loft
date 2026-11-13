use std::convert::Infallible;
use std::path::Path as FsPath;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::Stream;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::agents::{self, emit_agent, is_running, workspace};
use crate::avatar::Avatar;
use crate::db::{Agent, QuietWindow, now};
use crate::repos;
use crate::state::{AppState, Event};

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

pub fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

pub fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "not found".into())
}

pub type ApiResult<T> = Result<Json<T>, ApiError>;

fn agent_json(state: &AppState, a: &Agent) -> Value {
    let mut v = agents::client_json(a);
    agents::run_fields(state, &a.id, &mut v);
    v["unread_reply"] = json!(a.unread_reply());
    v["fresh_context_applies"] = json!(crate::delivery::fresh_context_applies(state, a));
    v["workspace"] = json!(workspace(state, &a.id).display().to_string());
    // Projects whose folder is gone are problems; a folder of several repos is a hint.
    v["project_problems"] = crate::repos::project_problems(state, a);
    v
}

// ---- meta -------------------------------------------------------------

pub async fn info(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "projects_dir": state.settings.boot().projects_dir,
        "storage_dir": state.cfg.storage_dir,
        "github_token": crate::github::token(&state).is_some(),
        "version": env!("CARGO_PKG_VERSION"),
        "host": crate::tailnet::host(),
        // Lets clients (the iOS app) tell what this server supports.
        "features": ["tasks", "activity", "idempotent_messages", "voice_clean", "system", "grill", "run_watchdog", "task_comments", "push", "needs_you", "plans", "mentions", "voice_brief", "backups", "workspaces", "clean_picker", "agent_devices", "left_processes"],
    }))
}

pub async fn update_status(State(state): State<AppState>) -> Json<Value> {
    Json(json!(*state.update.lock().unwrap()))
}

pub async fn usage(State(state): State<AppState>) -> Json<Value> {
    Json(crate::usage::get(&state).await)
}

// ---- settings ---------------------------------------------------------

fn running_count(state: &AppState) -> usize {
    state.rt.lock().unwrap().running.len()
}

/// Settings for the UI. The GitHub token is write-only: only whether one is set comes back.
fn settings_view(state: &AppState) -> Value {
    let s = state.settings.get();
    let token_set = !s.github_token.trim().is_empty();
    json!({
        "settings": s,
        "github_token_set": token_set,
        "restart_fields": crate::settings::RESTART_FIELDS,
        "pending_restart": state.settings.pending_restart(),
        "running_agents": running_count(state),
    })
}

pub async fn get_settings(State(state): State<AppState>) -> Json<Value> {
    Json(settings_view(&state))
}

/// Replace the settings. `github_token`: absent or null keeps the saved one, "" clears it.
pub async fn put_settings(State(state): State<AppState>, Json(body): Json<Value>) -> ApiResult<Value> {
    let Value::Object(mut body) = body else { return Err(bad("settings must be an object")) };
    let token = body.remove("github_token");
    // Keys a client leaves out keep their saved value, so a client can send only what it changed.
    let mut merged = serde_json::to_value(state.settings.get()).map_err(|e| bad(e.to_string()))?;
    if let Some(o) = merged.as_object_mut() {
        o.extend(body);
    }
    let mut s: crate::settings::Settings = serde_json::from_value(merged).map_err(|e| bad(e.to_string()))?;
    // Where relay keys and push text go is only changed through `PUT /api/push`, with its checks.
    s.push.relay_url = state.settings.get().push.relay_url;
    // Older clients don't know the global auto-publish switch: they keep it.
    if s.auto_publish_all.is_none() {
        s.auto_publish_all = state.settings.get().auto_publish_all;
    }
    // Backups hold every chat and the GitHub token: never where agents or a repo can reach them. Only a
    // new folder is checked, so one that went bad since (a backup reports that) never blocks the rest.
    if s.backup_dir != state.settings.get().backup_dir
        && let Some(why) = crate::backup::refuse(&crate::backup::dir(&s), state.cfg.storage_dir.parent())
    {
        return Err(bad(why));
    }
    // Agents run it: only a file that's there, never a relative path that depends on where a run starts.
    if s.claude_bin != state.settings.get().claude_bin && !crate::settings::runnable(&s.claude_bin) {
        return Err(bad("Claude command must be a full path to a file, or a command on PATH, like claude"));
    }
    s.github_token = match token {
        Some(Value::String(t)) => t.trim().to_string(),
        _ => state.settings.get().github_token,
    };
    state.settings.save(s).map_err(|e| bad(e.to_string()))?;
    Ok(Json(settings_view(&state)))
}

/// Restart into the binary on disk, applying restart-only settings. Refused while agents run.
pub async fn restart(State(state): State<AppState>) -> ApiResult<Value> {
    let n = running_count(&state);
    if n > 0 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            format!("{n} agent{} running. Restart once {} finished.", if n == 1 { " is" } else { "s are" }, if n == 1 { "it has" } else { "they have" }),
        ));
    }
    tracing::info!("restarting on request");
    let st = state.clone();
    // Let this response go out first.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        crate::updater::restart(&st);
    });
    Ok(Json(json!({ "ok": true })))
}

pub async fn events(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = state.events.subscribe();
    let events = BroadcastStream::new(rx).filter_map(|ev: Result<Event, _>| {
        let ev = ev.ok()?;
        Some(Ok(SseEvent::default().data(serde_json::to_string(&ev).ok()?)))
    });
    // An immediate first chunk makes proxies (e.g. Vite's) flush headers right away.
    let hello = tokio_stream::once(Ok(SseEvent::default().comment("connected")));
    let stream = hello.chain(events);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---- repos ------------------------------------------------------------

#[derive(Deserialize)]
pub struct Refresh {
    #[serde(default)]
    refresh: bool,
}

pub async fn list_repos(State(state): State<AppState>, Query(q): Query<Refresh>) -> Json<Value> {
    Json(json!(repos::list(&state, q.refresh).await))
}

pub async fn github(State(state): State<AppState>, Query(q): Query<Refresh>) -> Json<Value> {
    Json(json!(repos::github(&state, q.refresh).await))
}

pub async fn repo_commits(State(state): State<AppState>, Path(name): Path<String>) -> ApiResult<Value> {
    let c = repos::recent_commits(&state, &name, 30).await.map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!(c)))
}

// ---- agents -----------------------------------------------------------

pub async fn list_agents(State(state): State<AppState>) -> ApiResult<Vec<Value>> {
    Ok(Json(state.db.list_agents()?.iter().map(|a| agent_json(&state, a)).collect()))
}

pub async fn get_agent(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let a = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    Ok(Json(agent_json(&state, &a)))
}

/// The most projects one agent can work in.
const MAX_PROJECTS: usize = 20;

pub const DEFAULT_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "Edit", "Write", "TodoWrite",
    "Bash(git *)", "Bash(ls *)", "Bash(cat *)", "Bash(mkdir *)", "WebSearch",
];

/// Refuses a project the agent doesn't have yet when its folder is missing.
/// Projects it already has stay, however they look: they are flagged instead (`project_problems`), from the
/// disk as it is now, so a folder fixed before this save clears its flag in the reply.
fn check_new_projects(state: &AppState, a: &Agent, ps: &[String]) -> Result<(), ApiError> {
    crate::repos::forget_project_problems(state, ps);
    for p in ps.iter().filter(|p| !p.is_empty() && !a.projects.contains(p) && **p != a.project) {
        if let Some(why) = crate::repos::project_problem(state, p) {
            return Err(bad(why));
        }
    }
    Ok(())
}

/// Apply editable fields from a JSON patch onto an agent.
fn apply_patch(state: &AppState, a: &mut Agent, p: &Value) -> Result<(), ApiError> {
    let str_field = |k: &str| p.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    if let Some(v) = str_field("name") {
        if v.trim().is_empty() {
            return Err(bad("name cannot be empty"));
        }
        a.name = v.trim().into();
    }
    if let Some(v) = str_field("group") {
        let g = v.trim();
        if g.chars().count() > 100 || g.chars().any(char::is_control) {
            return Err(bad("group must be at most 100 characters, with no control characters"));
        }
        a.group = g.into();
    }
    if let Some(v) = p.get("projects") {
        let raw: Vec<String> = serde_json::from_value(v.clone()).map_err(|_| bad("projects must be a list of strings"))?;
        let mut ps: Vec<String> = vec![];
        for r in raw {
            let n = crate::repos::project_name(&r).map_err(bad)?;
            if !n.is_empty() && !ps.contains(&n) {
                ps.push(n);
            }
        }
        if ps.len() > MAX_PROJECTS {
            return Err(bad(format!("an agent can have at most {MAX_PROJECTS} projects")));
        }
        check_new_projects(state, a, &ps)?;
        a.project = ps.first().cloned().unwrap_or_default();
        a.projects = ps;
    } else if let Some(v) = str_field("project") {
        // A client that knows one project sets the main one; the others stay.
        let name = crate::repos::project_name(&v).map_err(bad)?;
        check_new_projects(state, a, std::slice::from_ref(&name))?;
        a.project = name;
        a.projects.retain(|p| *p != a.project);
        if a.project.is_empty() {
            a.projects.clear();
        } else {
            a.projects.insert(0, a.project.clone());
        }
    }
    if let Some(v) = str_field("personality") { a.personality = v; }
    if let Some(v) = str_field("prompt") { a.prompt = v; }
    if let Some(v) = str_field("model") { a.model = v; }
    if let Some(v) = str_field("effort") { a.effort = v; }
    if let Some(v) = str_field("subagent_model") { a.subagent_model = v; }
    if let Some(v) = p.get("max_subagents") {
        a.max_subagents = match v {
            Value::Null => None,
            v => Some(v.as_u64().filter(|n| *n <= 20).ok_or_else(|| bad("max_subagents must be null or 0-20"))? as u32),
        };
    }
    if let Some(v) = str_field("fresh_context") {
        if !crate::agents::FRESH_CONTEXT.contains(&v.as_str()) {
            return Err(bad(format!("fresh_context must be one of {}", crate::agents::FRESH_CONTEXT.join(", "))));
        }
        a.fresh_context = v;
    }
    if let Some(v) = p.get("role") {
        a.role = v.as_str().filter(|r| crate::planning::is_role(r)).ok_or_else(|| bad("role must be one of lead, dev, code_reviewer, qa, security or custom"))?.into();
    }
    if let Some(v) = str_field("permission_mode") {
        const MODES: &[&str] = &["default", "acceptEdits", "auto", "bypassPermissions", "dontAsk", "plan", "manual"];
        if !MODES.contains(&v.as_str()) {
            return Err(bad(format!("unknown permission_mode {v}")));
        }
        a.permission_mode = v;
    }
    if let Some(v) = p.get("interval_minutes") {
        let n = v.as_i64().ok_or_else(|| bad("interval_minutes must be an integer"))?;
        if n != a.interval_minutes {
            a.interval_minutes = n.max(0);
            a.next_run_at = None;
        }
    }
    if let Some(v) = p.get("quiet_hours") {
        let w: Vec<QuietWindow> = serde_json::from_value(v.clone()).map_err(|_| bad("quiet_hours must be a list of { days, start, end }"))?;
        let w = crate::schedule::validate(w).map_err(bad)?;
        if w != a.quiet_hours {
            a.quiet_hours = w;
            a.next_run_at = None;
        }
    }
    let list = |k: &str| -> Result<Option<Vec<String>>, ApiError> {
        match p.get(k) {
            None => Ok(None),
            Some(v) => serde_json::from_value::<Vec<String>>(v.clone())
                .map(|l| Some(l.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()))
                .map_err(|_| bad(format!("{k} must be a list of strings"))),
        }
    };
    if let Some(l) = list("allowed_tools")? { a.allowed_tools = l; }
    if let Some(l) = list("disallowed_tools")? { a.disallowed_tools = l; }
    if let Some(v) = p.get("mcp_servers") {
        if !v.is_object() {
            return Err(bad("mcp_servers must be an object of name -> server config"));
        }
        if v.get("loft").is_some() {
            return Err(bad("'loft' is reserved"));
        }
        a.mcp_servers = v.clone();
    }
    if let Some(v) = p.get("avatar") {
        let av: Avatar = serde_json::from_value(v.clone()).map_err(|e| bad(e.to_string()))?;
        a.avatar = av.validate().map_err(|e| bad(e.to_string()))?;
    }
    if a.role == "lead"
        && a.team().is_empty() {
            return Err(bad("Only an agent with a group can be Lead: a Lead leads the agents that share its group."));
        }
    Ok(())
}

/// Makes `a` its group's only Lead (its team: `Agent::team`): any other Lead in the group becomes Custom.
/// Returns the agent the role moved from.
fn take_lead(state: &AppState, a: &Agent) -> Result<Option<Agent>, ApiError> {
    if a.role != "lead" {
        return Ok(None);
    }
    let mut moved = None;
    for mut o in state.db.list_agents()? {
        if o.role == "lead" && o.id != a.id && o.team() == a.team() {
            o.role = "custom".into();
            state.db.save_agent(&o)?;
            emit_agent(state, &o);
            moved = Some(o);
        }
    }
    Ok(moved)
}

/// The agent as JSON, plus a notice naming both agents when the Lead role moved to it.
fn with_lead_notice(mut v: Value, a: &Agent, moved: Option<Agent>) -> Value {
    if let Some(o) = moved {
        v["lead_moved_from"] = json!({ "id": o.id, "name": o.name });
        v["notice"] = json!(format!("Lead moved from {} to {}.", o.name, a.name));
    }
    v
}

pub async fn create_agent(State(state): State<AppState>, Json(p): Json<Value>) -> ApiResult<Value> {
    let (a, moved) = new_agent(&state, uuid::Uuid::new_v4().to_string(), &p, None)?;
    Ok(Json(with_lead_notice(agent_json(&state, &a), &a, moved)))
}

/// Create, save and announce an agent with `id`, defaults overridden by the patch `p`. `created_by` is the agent
/// that asked for it, if one did. Also returns the agent the Lead role moved from, if it took it.
pub fn new_agent(state: &AppState, id: String, p: &Value, created_by: Option<String>) -> Result<(Agent, Option<Agent>), ApiError> {
    let mut a = Agent {
        id: id.clone(),
        name: "New agent".into(),
        project: String::new(),
        group: String::new(),
        projects: vec![],
        personality: String::new(),
        prompt: String::new(),
        model: "sonnet".into(),
        effort: String::new(),
        interval_minutes: 0,
        quiet_hours: vec![],
        allowed_tools: DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect(),
        disallowed_tools: vec![],
        mcp_servers: json!({}),
        // Auto mode: a classifier approves routine actions and blocks risky ones;
        // anything it won't allow still reaches the user as a chat permission card.
        permission_mode: "auto".into(),
        subagent_model: String::new(),
        max_subagents: None,
        grill_briefed: false,
        memory_index: None,
        role: "custom".into(),
        fresh_context: crate::db::NEW_AGENT_FRESH_CONTEXT.into(),
        delivery_ticket: None,
        avatar: Avatar::generate(&id),
        status: "idle".into(),
        status_text: String::new(),
        session_id: None,
        context_tokens: None,
        last_run_at: None,
        last_run_ended_at: None,
        next_run_at: None,
        total_cost_usd: 0.0,
        session_cost_usd: 0.0,
        created_at: now(),
        replied_at: None,
        read_at: None,
        builtin: false,
        created_by,
    };
    apply_patch(state, &mut a, p)?;
    std::fs::create_dir_all(workspace(state, &id)).map_err(anyhow::Error::from)?;
    std::fs::write(
        workspace(state, &id).join("README.md"),
        format!("# {}'s workspace\n\nPrivate storage for the Loft agent `{}` ({id}).\n", a.name, a.name),
    )
    .map_err(anyhow::Error::from)?;
    state.db.save_agent(&a)?;
    emit_agent(state, &a);
    let moved = take_lead(state, &a)?;
    // A description of the look it should have: the agent draws its own sticker. An explicit `avatar` wins.
    match p["avatar_brief"].as_str().map(str::trim) {
        Some(brief) if !brief.is_empty() && p.get("avatar").is_none() => agents::design_avatar(state, &id, brief),
        _ => {}
    }
    Ok((a, moved))
}

pub async fn update_agent(State(state): State<AppState>, Path(id): Path<String>, Json(p): Json<Value>) -> ApiResult<Value> {
    let mut a = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    apply_patch(&state, &mut a, &p)?;
    state.db.save_agent(&a)?;
    emit_agent(&state, &a);
    let moved = take_lead(&state, &a)?;
    Ok(Json(with_lead_notice(agent_json(&state, &a), &a, moved)))
}

#[derive(Deserialize)]
pub struct Order {
    ids: Vec<String>,
}

pub async fn reorder_agents(State(state): State<AppState>, Json(o): Json<Order>) -> ApiResult<Value> {
    state.db.reorder_agents(&o.ids)?;
    Ok(Json(json!({ "ok": true })))
}

/// What deleting the agent with its workspace would free.
pub async fn workspace_size(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let dir = workspace(&state, &id);
    let bytes = tokio::task::spawn_blocking(move || crate::sweep::size(&dir)).await.unwrap_or(0);
    Ok(Json(json!({ "bytes": bytes })))
}

#[derive(Deserialize)]
pub struct DeleteOpts {
    #[serde(default)]
    purge_files: bool,
}

pub async fn delete_agent(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<DeleteOpts>) -> ApiResult<Value> {
    if state.db.get_agent(&id)?.ok_or_else(not_found)?.builtin {
        return Err(ApiError(StatusCode::FORBIDDEN, "Loft's own agent can't be deleted.".into()));
    }
    agents::stop(&state, &id);
    // With the processes it left running (detached servers), so nothing of it keeps running.
    crate::processes::stop_all(&state.cfg.storage_dir, &id).await;
    crate::simulator::delete_agent_device(&state, &id).await;
    state.db.delete_agent(&id)?;
    crate::uploads::remove_all(&state, &id);
    agents::remove_agent_files(&state, &id);
    if q.purge_files {
        let _ = std::fs::remove_dir_all(workspace(&state, &id));
    } else {
        // Its files stay: its clones' build output goes once they're idle.
        let (state, id) = (state.clone(), id.clone());
        tokio::spawn(async move { crate::sweep::run(&state, None, Some(&id), crate::sweep::IDLE).await });
    }
    state.emit(Event::AgentDeleted { agent_id: id });
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct Page {
    limit: Option<i64>,
    before: Option<i64>,
}

pub async fn list_messages(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<Page>) -> ApiResult<Value> {
    let msgs = state.db.list_messages(&id, q.limit.unwrap_or(300).clamp(1, 1000), q.before)?;
    Ok(Json(json!(msgs)))
}

#[derive(Deserialize)]
pub struct SendMessage {
    text: String,
    /// Chosen by the client; a resend with the same id isn't delivered twice.
    client_id: Option<String>,
    /// Only `meta.grill` (a grill answer or start request) and `meta.task_id` (a task attached to the
    /// message) are read; anything else is ignored.
    meta: Option<Value>,
    /// Uploads (from `POST /agents/{id}/uploads`) to send with the text.
    #[serde(default)]
    images: Vec<String>,
}

pub async fn send_message(State(state): State<AppState>, Path(id): Path<String>, Json(m): Json<SendMessage>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    if m.text.trim().is_empty() && m.images.is_empty() {
        return Err(bad("empty message"));
    }
    crate::uploads::validate(&state, &id, &m.images)?;
    let client_id = m.client_id.map(|c| c.trim().chars().take(64).collect::<String>()).filter(|c| !c.is_empty());
    if let Some(c) = &client_id
        && let Some(prev) = state.db.find_client_message(&id, c)? {
            return Ok(Json(json!({ "ok": true, "queued": false, "duplicate": true, "message": prev })));
        }
    let queued = is_running(&state, &id);
    let mut meta = client_id.map(|c| json!({ "client_id": c })).unwrap_or(json!({}));
    // Images go in before routing: a grill answer keeps them too.
    if !m.images.is_empty() {
        meta["images"] = json!(m.images);
    }
    let agent = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let grill = m.meta.as_ref().and_then(|v| v.get("grill")).filter(|g| g.is_object());
    let meta = match crate::grill::on_user_message(&state, &agent, m.text.trim(), grill, meta)? {
        crate::grill::Routed::Normal(meta) => meta,
        crate::grill::Routed::Handled { message, wake } => {
            if let Some(p) = wake {
                agents::wake(&state, &id, p, "grill");
            }
            return Ok(Json(json!({ "ok": true, "queued": queued, "message": message })));
        }
    };
    let mut meta = meta;
    let text = m.text.trim().to_string();
    // A task attached to the message, or task links in it: stored on the message, and the agent gets a short header.
    let attached = m.meta.as_ref().and_then(|v| v["task_id"].as_str()).map(|a| a.trim().to_lowercase());
    let refs = crate::tasks::references(&state, attached.as_deref(), &text);
    if let Some(a) = &attached {
        if refs.first() != Some(a) {
            return Err(bad("The attached task doesn't exist."));
        }
        meta["task_id"] = json!(a);
    }
    if !refs.is_empty() {
        meta["task_refs"] = json!(refs);
    }
    let tags = crate::tags::resolve(&state, &agent, &text);
    if !tags.is_empty() {
        meta["tags"] = json!(tags);
    }
    // Any agent, its own group's first: where two share a name, the "Dev" of this group wins.
    let mut all = state.db.list_agents()?;
    all.sort_by_key(|a| a.team() != agent.team());
    let (mentioned, leading) = crate::mentions::parse(&text, &all, &id);
    if !mentioned.is_empty() {
        meta["mentions"] = crate::mentions::meta(&mentioned);
        meta["mentions_only"] = json!(leading);
    }
    // A message that opens with @-mentions is for them: it stays in this chat without waking its agent.
    let msg = if leading {
        agents::add_msg(&state, &id, None, "user", "text", &text, meta)
    } else {
        agents::enqueue_with(&state, &id, Some(text.clone()), meta, "user")
    };
    crate::mentions::deliver(&state, &agent, &mentioned, &text, msg.as_ref().map(|m| m.id));
    Ok(Json(json!({ "ok": true, "queued": queued && !leading, "message": msg, "mentioned": crate::mentions::meta(&mentioned) })))
}

pub async fn run_now(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    if is_running(&state, &id) {
        return Err(ApiError(StatusCode::CONFLICT, "agent is already running".into()));
    }
    agents::enqueue(&state, &id, None, "manual");
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/agents/{id}/read`: the user has seen this agent's chat; clears its Unread reply everywhere.
pub async fn mark_read(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let a = agents::mark_read(&state, &id)?.ok_or_else(not_found)?;
    Ok(Json(agent_json(&state, &a)))
}

pub async fn stop_agent(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    // Deny any permission prompts that are blocking this agent so the process can exit.
    for p in state.db.list_permissions(Some(&id))?.into_iter().filter(|p| p.status == "pending") {
        let _ = agents::decide(&state, &p.id, "deny", None, Some("Run stopped by user.".into()));
    }
    // Messages queued behind the run are kept: they run next (people press Stop to unstick an agent,
    // and what they sent while it looked stuck must not be lost).
    Ok(Json(json!({ "ok": agents::stop(&state, &id) })))
}

/// Unstick a run that waits on background work or has gone quiet: stop it, keeping the queue. With nothing
/// queued, the agent is told it was restarted and picks up where it left off.
pub async fn restart_agent(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    restart_run(state, id, "user").await
}

/// Restart as `by`: "user" (the Restart button) or "Loft" (a run gone quiet, agents::restart_quiet).
pub(crate) async fn restart_run(state: AppState, id: String, by: &'static str) -> ApiResult<Value> {
    {
        let mut rt = state.rt.lock().unwrap();
        if !rt.running.contains_key(&id) {
            return Err(ApiError(StatusCode::CONFLICT, "Agent isn't running.".into()));
        }
        let info = rt.runs.get(&id).cloned().unwrap_or_default();
        let why = if info.waiting_on.is_empty() {
            let quiet = DateTime::parse_from_rfc3339(&info.last_output_at).map(|t| (Utc::now() - t.with_timezone(&Utc)).num_minutes()).unwrap_or(0);
            format!("no output for {quiet} min")
        } else {
            format!("it was waiting on {}", info.waiting_on.join(", "))
        };
        if rt.queued.get(&id).is_none_or(|q| q.is_empty()) {
            rt.queued.entry(id.clone()).or_default().push(format!(
                "[Loft: {} restarted your last run ({why}); anything it still had running was stopped. Pick up where you left off: finish the work, or say what's left.]",
                if by == "user" { "the user" } else { "Loft" }
            ));
            // The restarted run picks up the same work: if it was answering the user, so is this one.
            // A queue already waiting runs next instead, and counts only if its own messages came from the user.
            if info.for_user {
                rt.queued_for_user.insert(id.clone());
            }
        }
        // The next run picks up the same work, so the Delivery steps it was woken for wait for that run's end.
        if let Some(w) = rt.delivery_running.remove(&id) {
            rt.delivery_queued.entry(id.clone()).or_default().extend(w.into_iter().map(|w| (w, None)));
        }
        rt.restarting.insert(id.clone(), by);
    }
    stop_agent(State(state), Path(id)).await
}

/// `/clear`: drop the Claude session so the next run starts with empty context.
/// Chat history is kept (activity stats and PR cards are built from it); the
/// `clear` marker tells the UI to start the view fresh from here.
pub async fn reset_session(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    if is_running(&state, &id) {
        return Err(ApiError(StatusCode::CONFLICT, "Agent is working. Stop it before clearing.".into()));
    }
    {
        let mut rt = state.rt.lock().unwrap();
        rt.queued.remove(&id);
        rt.queued_for_user.remove(&id);
        rt.grill_queued.remove(&id);
        rt.delivery_queued.remove(&id);
        rt.fresh_before.remove(&id);
    }
    let a = agents::clear_context(&state, &id, "Context cleared. Long-term memory carries over.", json!({})).ok_or_else(not_found)?;
    Ok(Json(agent_json(&state, &a)))
}

/// `/compact [instructions]`: summarise the Claude session in place, so later
/// runs resend a short summary instead of the whole conversation.
pub async fn compact_session(State(state): State<AppState>, Path(id): Path<String>, Json(p): Json<Value>) -> ApiResult<Value> {
    let a = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    if is_running(&state, &id) {
        return Err(ApiError(StatusCode::CONFLICT, "Agent is working. Stop it before compacting.".into()));
    }
    if a.session_id.is_none() {
        return Err(ApiError(StatusCode::CONFLICT, "Nothing to compact: the conversation is empty.".into()));
    }
    agents::compact(&state, &id, p["instructions"].as_str().unwrap_or("").trim());
    Ok(Json(json!({ "ok": true })))
}

/// `/resume`: the agent's past Sessions, from Claude Code's transcripts for its workspace, newest first.
pub async fn list_sessions(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let a = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let dir = crate::sessions::transcripts_dir(&workspace(&state, &id));
    let sessions = tokio::task::spawn_blocking(move || crate::sessions::list(&dir)).await.unwrap_or_default();
    Ok(Json(json!({ "sessions": sessions, "current": a.session_id })))
}

#[derive(Deserialize)]
pub struct ResumeSession {
    session_id: String,
}

/// Makes a past Session the agent's own: its next run resumes it.
pub async fn resume_session(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<ResumeSession>) -> ApiResult<Value> {
    let dir = crate::sessions::transcripts_dir(&workspace(&state, &id));
    resume_from(&state, &id, dir, b.session_id).await
}

pub(crate) async fn resume_from(state: &AppState, id: &str, dir: std::path::PathBuf, sid: String) -> ApiResult<Value> {
    let a = state.db.get_agent(id)?.ok_or_else(not_found)?;
    if is_running(state, id) {
        return Err(ApiError(StatusCode::CONFLICT, "Agent is working. Stop it before resuming a Session.".into()));
    }
    if a.session_id.as_deref() == Some(sid.as_str()) {
        return Err(bad("That's already the agent's Session."));
    }
    let s = tokio::task::spawn_blocking(move || crate::sessions::find(&dir, &sid))
        .await
        .ok()
        .flatten()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "No such Session in this agent's transcripts.".into()))?;
    {
        let mut rt = state.rt.lock().unwrap();
        rt.queued.remove(id);
        rt.grill_queued.remove(id);
        rt.delivery_queued.remove(id);
        rt.fresh_before.remove(id);
    }
    // As for a new Session, the next run builds the memory index afresh. Its next run costs only what it adds.
    let total = state.db.session_total(id, &s.id)?;
    let a = state.db.update_agent(id, |a| {
        a.session_id = Some(s.id.clone());
        a.session_cost_usd = total;
        a.context_tokens = s.context_tokens;
        a.grill_briefed = false;
        a.memory_index = None;
    })?.ok_or_else(not_found)?;
    let day = s.started_at.get(..10).unwrap_or(&s.started_at);
    let m = state.db.add_message(id, None, "system", "resume", &format!("Resumed the Session from {day}."), json!({
        "session_id": s.id, "started_at": s.started_at, "first_message": s.first_message,
    }))?;
    state.emit(Event::Message { message: json!(m) });
    emit_agent(state, &a);
    Ok(Json(agent_json(state, &a)))
}

pub async fn clear_messages(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    state.db.clear_messages(&id)?;
    crate::uploads::remove_all(&state, &id);
    Ok(Json(json!({ "ok": true })))
}

/// Redraw: the agent draws itself a new sticker in a quiet run; the sticker arrives with the agent event.
pub async fn redraw_avatar(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    agents::redraw_avatar(&state, &id);
    Ok(Json(json!({ "ok": true })))
}

// ---- memories ---------------------------------------------------------

pub async fn list_memories(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(json!(state.db.list_memories(&id)?)))
}

#[derive(Deserialize)]
pub struct NewMemory {
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    tags: String,
}

pub async fn add_memory(State(state): State<AppState>, Path(id): Path<String>, Json(m): Json<NewMemory>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let title = m.title.trim();
    if title.is_empty() || title.contains('\n') {
        return Err(bad("a memory needs a one-line title"));
    }
    // A memory added with only a title keeps the title as its body too.
    let content = if m.content.trim().is_empty() { title } else { m.content.trim() };
    let mem = state.db.add_memory(&id, title, content, m.tags.trim())?;
    state.emit(Event::Memory { agent_id: id });
    Ok(Json(json!(mem)))
}

pub async fn delete_memory(State(state): State<AppState>, Path((id, mid)): Path<(String, i64)>) -> ApiResult<Value> {
    state.db.delete_memory(&id, mid)?;
    state.emit(Event::Memory { agent_id: id });
    Ok(Json(json!({ "ok": true })))
}

// ---- permissions ------------------------------------------------------

#[derive(Deserialize)]
pub struct PermQuery {
    agent_id: Option<String>,
}

pub async fn list_permissions(State(state): State<AppState>, Query(q): Query<PermQuery>) -> ApiResult<Value> {
    Ok(Json(json!(state.db.list_permissions(q.agent_id.as_deref())?)))
}

#[derive(Deserialize)]
pub struct DecideBody {
    decision: String,
    rule: Option<String>,
    note: Option<String>,
}

pub async fn decide_permission(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<DecideBody>) -> ApiResult<Value> {
    if !["allow_once", "allow_always", "deny"].contains(&b.decision.as_str()) {
        return Err(bad("decision must be allow_once | allow_always | deny"));
    }
    let r = agents::decide(&state, &id, &b.decision, b.rule, b.note).map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!(r)))
}

// ---- workspace files --------------------------------------------------

fn walk(root: &FsPath, dir: &FsPath, depth: usize, out: &mut Vec<Value>) {
    if out.len() >= 400 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name == ".git" || name == "node_modules" || name == "target" || name == ".DS_Store" {
            continue;
        }
        let meta = e.metadata().ok();
        let is_dir = p.is_dir();
        out.push(json!({
            "path": p.strip_prefix(root).unwrap_or(&p).to_string_lossy(),
            "dir": is_dir,
            "depth": depth,
            "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
            "is_repo": is_dir && p.join(".git").exists(),
        }));
        if is_dir && depth < 2 {
            walk(root, &p, depth + 1, out);
        }
    }
}

/// An agent id is a UUID: letters, digits and dashes. Anything else (in particular a percent-encoded
/// `../`) must never reach a filesystem path.
pub fn is_plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The workspace of an agent named in the URL: the id must be well-formed and the agent must exist.
fn agent_workspace(state: &AppState, id: &str) -> Result<std::path::PathBuf, ApiError> {
    if !is_plain_id(id) {
        return Err(not_found());
    }
    state.db.get_agent(id)?.ok_or_else(not_found)?;
    Ok(workspace(state, id))
}

pub async fn list_files(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let root = agent_workspace(&state, &id)?;
    let mut out = vec![];
    walk(&root, &root, 0, &mut out);
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
pub struct LsQuery {
    path: Option<String>,
}

/// One folder's entries, for completing `@path` in the chat box. `path` works like a shell's from the agent's
/// `@` base (its project's folder, else its workspace), `../` included, and may reach anywhere in the projects
/// folder or the workspace but not above. `dir` is where it landed, relative to the projects folder.
pub async fn list_dir(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<LsQuery>) -> ApiResult<Value> {
    let a = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let ws = workspace(&state, &id);
    let _ = std::fs::create_dir_all(&ws);
    let projects = &state.settings.boot().projects_dir;
    let (dir, entries) = ls(&agents::at_base(&state, &a), &[&ws, projects], q.path.as_deref().unwrap_or("")).map_err(bad)?;
    let shown = projects.canonicalize().ok().and_then(|r| dir.strip_prefix(r).ok().map(|d| d.display().to_string())).unwrap_or_else(|| dir.display().to_string());
    Ok(Json(json!({ "dir": shown, "entries": entries })))
}

#[derive(Deserialize)]
pub struct FindQuery {
    q: Option<String>,
}

/// Up to 20 files and folders anywhere under the agent's `@` base whose path fuzzy-matches `q`, best first,
/// for `@` followed by a name rather than a path. Each is `{ path, dir }`, with `path` relative to the base.
pub async fn find_files(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<FindQuery>) -> ApiResult<Value> {
    let a = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    Ok(Json(json!(crate::file_search::search(&state.listings, &agents::at_base(&state, &a), q.q.as_deref().unwrap_or("")).await)))
}

fn ls(base: &FsPath, roots: &[&FsPath], path: &str) -> Result<(std::path::PathBuf, Vec<Value>), String> {
    // Resolved for real (`..`, symlinks, absolute paths) before checking where it landed.
    let dir = base.join(path).canonicalize().map_err(|_| format!("No folder {path}"))?;
    if !roots.iter().any(|root| root.canonicalize().is_ok_and(|r| dir.starts_with(r))) {
        return Err("That's outside the projects folder".into());
    }
    let mut out: Vec<(bool, String)> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| (e.path().is_dir(), e.file_name().to_string_lossy().into_owned()))
        .filter(|(_, n)| n != ".DS_Store")
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    Ok((dir, out.into_iter().map(|(dir, name)| json!({ "name": name, "dir": dir })).collect()))
}

// ---- pull requests ----------------------------------------------------

#[derive(Deserialize)]
pub struct PrQuery {
    pub agent_id: Option<String>,
}

pub async fn list_pull_requests(State(state): State<AppState>, Query(q): Query<PrQuery>) -> ApiResult<Value> {
    Ok(Json(json!(state.db.list_pull_requests(q.agent_id.as_deref())?)))
}

pub async fn refresh_pull_request(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let pr = crate::prs::refresh(&state, &id).await.map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!(pr)))
}

#[derive(Deserialize)]
pub struct MergeBody {
    method: Option<String>,
}

pub async fn merge_pull_request(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<MergeBody>) -> ApiResult<Value> {
    let method = b.method.unwrap_or_else(|| "squash".into());
    if !["squash", "merge", "rebase"].contains(&method.as_str()) {
        return Err(bad("method must be squash | merge | rebase"));
    }
    let pr = crate::prs::merge(&state, &id, &method).await.map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!(pr)))
}

// ---- dashboard --------------------------------------------------------

#[derive(Deserialize)]
pub struct SummaryQuery {
    totals: Option<bool>,
}

/// Per-agent activity for the home page over the last `DAYS` days: daily runs
/// (local days, split autonomous vs on request), totals, PR outcomes, and
/// anything waiting on the user. With `?totals=true` the list comes as `agents`
/// next to the home line's figures (`period`, `all_time`); without it, a bare list, as the iOS app reads it.
/// `period` counts the current Usage week, or the last 7 days when the plan usage can't be read,
/// with the span just before it as `previous`: the previous week up to the same point in it, or the 7 days before the last 7.
pub async fn agents_summary(State(state): State<AppState>, Query(q): Query<SummaryQuery>) -> ApiResult<Value> {
    const DAYS: i64 = 14;
    let today = chrono::Local::now().date_naive();
    let since14 = (chrono::Utc::now() - chrono::Duration::days(DAYS)).to_rfc3339();
    let prs = state.db.list_pull_requests(None)?;
    let perms = state.db.list_permissions(None)?;
    let mut out = vec![];
    for a in state.db.list_agents()? {
        let act = state.db.agent_activity(&a.id, &since14)?;
        let mut daily_auto = vec![0i64; DAYS as usize];
        let mut daily_user = vec![0i64; DAYS as usize];
        for r in act["runs"].as_array().into_iter().flatten() {
            let Some(at) = r["at"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) else { continue };
            let auto = r["trigger"].as_str() == Some("schedule");
            let age = (today - at.with_timezone(&chrono::Local).date_naive()).num_days();
            if (0..DAYS).contains(&age) {
                let i = (DAYS - 1 - age) as usize;
                if auto { daily_auto[i] += 1 } else { daily_user[i] += 1 }
            }
        }
        let mine: Vec<_> = prs.iter().filter(|p| p.agent_id == a.id).collect();
        out.push(json!({
            "agent": agent_json(&state, &a),
            "daily_auto": daily_auto,
            "daily_user": daily_user,
            "days": DAYS,
            "tool_calls": act["tool_calls"],
            "last_error": act["last_error"],
            "cost_usd": act["cost_usd"],
            "user_messages": act["user_messages"],
            "top_tools": act["top_tools"],
            "last_reply": act["last_reply"],
            "memories": act["memories"],
            "pending_permissions": perms.iter().filter(|p| p.agent_id == a.id && p.status == "pending").count(),
            "prs": {
                "total": mine.len(),
                "open": mine.iter().filter(|p| p.state == "open").count(),
                "merged": mine.iter().filter(|p| p.state == "merged").count(),
            },
        }));
    }
    if !q.totals.unwrap_or(false) {
        return Ok(Json(json!(out)));
    }
    let now = Utc::now();
    let (kind, since, resets_at) = match crate::usage::usage_week(&crate::usage::get(&state).await) {
        Some((start, reset)) => ("usage_week", start, Some(reset)),
        None => ("last_7_days", now - chrono::Duration::days(7), None),
    };
    let (runs, cost) = state.db.spend_between(&since.to_rfc3339(), None)?;
    // The period before, like with like: the previous usage week up to the same point in it,
    // or the 7 days before the last 7 (where that point is the period's end).
    let before = since - chrono::Duration::days(7);
    // Capped at this period's start: right after the week resets, the cached plan usage can still
    // hold the old reset time, putting `now` more than a week past `since`.
    let until = (before + (now - since)).min(since);
    let (prev_runs, prev_cost) = state.db.spend_between(&before.to_rfc3339(), Some(&until.to_rfc3339()))?;
    let period = json!({
        "kind": kind, "since": since.to_rfc3339(), "resets_at": resets_at.map(|r| r.to_rfc3339()), "runs": runs, "cost_usd": cost,
        "previous": { "since": before.to_rfc3339(), "until": until.to_rfc3339(), "runs": prev_runs, "cost_usd": prev_cost },
    });
    let (runs, cost) = state.db.all_time_spend()?;
    Ok(Json(json!({ "agents": out, "period": period, "all_time": { "runs": runs, "cost_usd": cost } })))
}

// ---- editor -----------------------------------------------------------

/// `/code`: open the agent's workspace folder in VS Code. VS Code only: from other machines it's a Remote-SSH link.
pub async fn open_editor(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    let dir = agent_workspace(&state, &id)?;
    std::fs::create_dir_all(&dir).map_err(anyhow::Error::from)?;

    // Requests from another machine arrive through a local reverse proxy (e.g. `tailscale serve`), which marks
    // who it forwards for. Launching an editor here would open it on this machine's screen, so hand back a
    // VS Code Remote-SSH link for the browser to open on the user's machine instead.
    if headers.contains_key("x-forwarded-for") || headers.contains_key("tailscale-user-login") {
        let host = state.settings.get().ssh_host;
        if host.trim().is_empty() {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "This loft runs on another machine. Set its SSH host in Settings to open workspaces with VS Code Remote-SSH.".into(),
            ));
        }
        let host = host.trim();
        let url = format!("vscode://vscode-remote/ssh-remote+{host}{}", dir.display());
        return Ok(Json(json!({ "ok": true, "path": dir, "url": url })));
    }

    let run = |bin: &str, args: Vec<std::ffi::OsString>| {
        let mut cmd = tokio::process::Command::new(bin);
        cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
        cmd
    };
    let launch = |mut cmd: tokio::process::Command| async move {
        // The editor CLI hands off to the app and exits quickly; don't hang on it.
        match tokio::time::timeout(std::time::Duration::from_secs(15), cmd.output()).await {
            Ok(Ok(out)) if out.status.success() => Ok(()),
            Ok(Ok(out)) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Ok(()),
        }
    };

    let mut result = launch(run("code", vec![dir.clone().into_os_string()])).await;
    // `code` not on PATH (shell command not installed): ask macOS to open the app directly.
    if result.is_err() && cfg!(target_os = "macos") {
        result = launch(run("open", vec!["-a".into(), "Visual Studio Code".into(), dir.clone().into_os_string()])).await;
    }
    match result {
        Ok(()) => Ok(Json(json!({ "ok": true, "path": dir }))),
        Err(e) => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Couldn't open {} in VS Code: {e}", dir.display()),
        )),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    /// An agent PATCH of just its project, for other modules' tests.
    pub(crate) fn apply_project(st: &crate::state::AppState, a: &mut crate::db::Agent, project: &str) -> Result<(), super::ApiError> {
        apply_patch(st, a, &serde_json::json!({ "project": project }))
    }

    use super::{apply_patch, is_plain_id};

    /// Run ends of an agent at each time, costing $1, $2, $4, ... in order.
    fn ended_runs_at(st: &crate::state::AppState, agent: &str, at: &[chrono::DateTime<chrono::Utc>]) {
        for (i, t) in at.iter().enumerate() {
            let m = st.db.add_message(agent, None, "system", "run_end", "", serde_json::json!({ "cost_usd": f64::from(1 << i) })).unwrap();
            st.db.conn().execute("UPDATE messages SET created_at = ?1 WHERE id = ?2", rusqlite::params![t.to_rfc3339(), m.id]).unwrap();
            st.db.conn().execute("UPDATE run_ledger SET ended_at = ?1 WHERE rowid = (SELECT max(rowid) FROM run_ledger)", [t.to_rfc3339()]).unwrap();
        }
    }

    async fn summary(st: &crate::state::AppState) -> serde_json::Value {
        use axum::extract::{Query, State};
        super::agents_summary(State(st.clone()), Query(super::SummaryQuery { totals: Some(true) })).await.unwrap().0
    }

    #[tokio::test]
    async fn agents_have_no_title_and_one_stored_with_a_title_still_loads() {
        use axum::{Json, extract::{Path, State}};
        let st = crate::tasks::tests::state();
        let Json(v) = super::create_agent(State(st.clone()), Json(serde_json::json!({ "name": "Ana", "title": "Frontend", "role": "dev" }))).await.ok().unwrap();
        assert!(v.get("title").is_none() && v["role"] == "dev", "{v}");
        let id = v["id"].as_str().unwrap().to_string();
        let Json(v) = super::update_agent(State(st.clone()), Path(id.clone()), Json(serde_json::json!({ "title": "Backend" }))).await.ok().unwrap();
        assert!(v.get("title").is_none(), "a title sent is ignored: {v}");
        // As stored before titles went away.
        let mut raw = serde_json::to_value(st.db.get_agent(&id).unwrap().unwrap()).unwrap();
        raw["title"] = serde_json::json!("Feature developer");
        st.db.conn().execute("UPDATE agents SET data = ?1 WHERE id = ?2", rusqlite::params![raw.to_string(), id]).unwrap();
        let Json(v) = super::get_agent(State(st.clone()), Path(id)).await.ok().unwrap();
        assert!(v.get("title").is_none() && v["name"] == "Ana", "{v}");
    }

    #[tokio::test]
    async fn settings_never_return_the_github_token_and_only_accept_a_runnable_claude_command() {
        use axum::{Json, extract::State};
        let st = crate::tasks::tests::state();
        let put = |body: serde_json::Value| super::put_settings(State(st.clone()), Json(body));
        let Json(v) = put(serde_json::json!({ "github_token": "ghp_secret" })).await.ok().unwrap();
        assert_eq!(st.settings.get().github_token, "ghp_secret");
        assert!(!v.to_string().contains("ghp_secret") && v["settings"].get("github_token").is_none());
        assert_eq!(v["github_token_set"], true);
        for bad in ["/nonexistent/claude", "bin/claude", "./claude", "no-such-command-anywhere", "/tmp"] {
            let err = put(serde_json::json!({ "claude_bin": bad })).await.unwrap_err();
            assert!(err.1.contains("Claude command"), "{bad}: {}", err.1);
        }
        assert!(put(serde_json::json!({ "claude_bin": "/bin/sh" })).await.is_ok());
        assert!(put(serde_json::json!({ "claude_bin": "sh" })).await.is_ok());
        assert_eq!(st.settings.get().claude_bin, "sh");
        assert_eq!(st.settings.get().github_token, "ghp_secret", "kept when not sent");
    }

    #[tokio::test]
    async fn the_home_line_counts_the_usage_week() {
        use chrono::{Duration, Utc};
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "A");
        let now = Utc::now();
        let reset = now + Duration::days(2);
        let start = reset - Duration::days(7);
        // Before the week, its first second, inside it; one 10 days ago, still in the 14-day charts;
        // the previous week's first second and the second before it; and just before and after the same
        // point in the previous week as now in this one (5 days in, so 7 days ago).
        let prev = start - Duration::days(7);
        let same_point = now - Duration::days(7);
        ended_runs_at(&st, &a.id, &[
            start - Duration::seconds(1), start, now - Duration::hours(1), now - Duration::days(10), prev, prev - Duration::seconds(1),
            same_point - Duration::minutes(1), same_point + Duration::minutes(1),
        ]);
        crate::usage::fake(&st, serde_json::json!({ "windows": [
            { "key": "five_hour", "percent": 10.0, "resets_at": (now + Duration::hours(3)).to_rfc3339() },
            { "key": "seven_day", "percent": 40.0, "resets_at": reset.to_rfc3339() },
        ], "error": null })).await;
        let s = summary(&st).await;
        let p = &s["period"];
        assert_eq!(p["kind"], "usage_week");
        assert_eq!(p["since"], start.to_rfc3339());
        assert_eq!(p["resets_at"], reset.to_rfc3339());
        assert_eq!(p["runs"], 2, "the run a second before the week starts isn't counted");
        assert_eq!(p["cost_usd"], 6.0);
        assert_eq!(p["previous"]["since"], prev.to_rfc3339());
        assert_eq!(p["previous"]["runs"], 3, "the previous week counts up to the same point as this one, not to its end");
        assert_eq!(p["previous"]["cost_usd"], 88.0);
        let until = chrono::DateTime::parse_from_rfc3339(p["previous"]["until"].as_str().unwrap()).unwrap();
        assert!((until.with_timezone(&Utc) - same_point).num_seconds().abs() < 5);
        let card = &s["agents"][0];
        assert_eq!(card["days"], 14);
        assert_eq!(card["cost_usd"], 255.0, "the agent card still counts 14 days");
    }

    #[tokio::test]
    async fn deleting_an_agent_with_purge_files_removes_its_workspace_and_without_it_keeps_it() {
        use axum::extract::{Path, Query, State};
        let st = crate::tasks::tests::state();
        let outside = crate::testdir::for_this_test("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("big"), vec![0u8; 1_000_000]).unwrap();
        let (a, b) = (crate::tasks::tests::agent(&st, "A"), crate::tasks::tests::agent(&st, "B"));
        for id in [&a.id, &b.id] {
            let ws = super::workspace(&st, id);
            std::fs::create_dir_all(ws.join("clone/target")).unwrap();
            std::fs::write(ws.join("notes.md"), "hello").unwrap();
            std::fs::write(ws.join("clone/target/lib.rlib"), vec![0u8; 1000]).unwrap();
            std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();
        }
        let size = super::workspace_size(State(st.clone()), Path(a.id.clone())).await.ok().unwrap().0;
        let bytes = size["bytes"].as_u64().unwrap();
        assert!((1005..1_000_000).contains(&bytes), "symlinks aren't followed: {bytes}");

        let del = |id: String, purge: &str| {
            let uri: axum::http::Uri = format!("/?purge_files={purge}").parse().unwrap();
            super::delete_agent(State(st.clone()), Path(id), Query::try_from_uri(&uri).unwrap())
        };
        let _ = del(a.id.clone(), "true").await.ok().unwrap();
        assert!(!super::workspace(&st, &a.id).exists());
        let _ = del(b.id.clone(), "false").await.ok().unwrap();
        assert!(super::workspace(&st, &b.id).join("clone/target/lib.rlib").exists());
        assert_eq!(std::fs::read(outside.join("big")).unwrap().len(), 1_000_000, "a symlink's target is never touched");
    }

    #[tokio::test]
    async fn the_week_keeps_its_runs_when_a_chat_is_cleared_or_an_agent_deleted_and_counts_grill_runs() {
        use chrono::{Duration, Utc};
        let st = crate::tasks::tests::state();
        let (a, b) = (crate::tasks::tests::agent(&st, "A"), crate::tasks::tests::agent(&st, "B"));
        let now = Utc::now();
        ended_runs_at(&st, &a.id, &[now - Duration::hours(2), now - Duration::hours(1)]);
        ended_runs_at(&st, &b.id, &[now - Duration::hours(3)]);
        crate::usage::fake(&st, serde_json::json!({ "windows": [
            { "key": "seven_day", "percent": 40.0, "resets_at": (now + Duration::days(2)).to_rfc3339() },
        ], "error": null })).await;
        let week = |s: &serde_json::Value| (s["period"]["runs"].as_i64().unwrap(), s["period"]["cost_usd"].as_f64().unwrap());
        assert_eq!(week(&summary(&st).await), (3, 4.0));
        // A grill run's run end isn't saved in the chat, but it counts.
        st.db.add_unsaved_run_end(&serde_json::json!({ "cost_usd": 0.5 })).unwrap();
        assert_eq!(week(&summary(&st).await), (4, 4.5));
        st.db.clear_messages(&a.id).unwrap();
        assert_eq!(week(&summary(&st).await), (4, 4.5), "clearing a chat changes nothing");
        st.db.delete_agent(&b.id).unwrap();
        let s = summary(&st).await;
        assert_eq!(week(&s), (4, 4.5), "deleting an agent changes nothing");
        assert_eq!(s["all_time"]["runs"], 4, "the week and the All-time spend agree");
    }

    #[tokio::test]
    async fn the_previous_week_never_ends_after_this_one_starts() {
        use chrono::{Duration, Utc};
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "A");
        let now = Utc::now();
        // The cached plan usage still holds the reset that just passed, so the week it gives started 7 days and an hour ago.
        let stale = now - Duration::hours(1);
        let start = stale - Duration::days(7);
        // One run in the hour just before the stale week's start: it belongs to that week, not the one before.
        ended_runs_at(&st, &a.id, &[start - Duration::days(7) + Duration::hours(1), start - Duration::minutes(30), start + Duration::minutes(1)]);
        crate::usage::fake(&st, serde_json::json!({ "windows": [
            { "key": "seven_day", "percent": 0.0, "resets_at": stale.to_rfc3339() },
        ], "error": null })).await;
        let p = &summary(&st).await["period"];
        assert_eq!(p["since"], start.to_rfc3339());
        assert_eq!(p["previous"]["until"], p["since"], "capped at this week's start");
        assert_eq!(p["runs"], 1);
        assert_eq!(p["previous"]["runs"], 2, "the run 30 minutes before this week's start is last week's only");
        assert_eq!(p["previous"]["cost_usd"], 3.0);
    }

    #[tokio::test]
    async fn without_plan_usage_the_home_line_counts_the_last_7_days() {
        use chrono::{Duration, Utc};
        for usage in [
            serde_json::json!({ "windows": [], "error": "no Claude Code login found; run `claude` and log in" }),
            // Read, but no weekly reset to start the week from.
            serde_json::json!({ "windows": [{ "key": "seven_day", "percent": 0.0, "resets_at": null }], "error": null }),
        ] {
            let st = crate::tasks::tests::state();
            let a = crate::tasks::tests::agent(&st, "A");
            let now = Utc::now();
            // Just over and under 7 days ago, yesterday; just under and over 14 days ago.
            ended_runs_at(&st, &a.id, &[
                now - Duration::days(7) - Duration::minutes(1), now - Duration::days(7) + Duration::minutes(1), now - Duration::days(1),
                now - Duration::days(14) + Duration::minutes(1), now - Duration::days(14) - Duration::minutes(1),
            ]);
            crate::usage::fake(&st, usage).await;
            let s = summary(&st).await;
            let p = &s["period"];
            assert_eq!(p["kind"], "last_7_days");
            assert!(p["resets_at"].is_null());
            let since = chrono::DateTime::parse_from_rfc3339(p["since"].as_str().unwrap()).unwrap();
            assert!((since.with_timezone(&Utc) - (now - Duration::days(7))).num_seconds().abs() < 5);
            assert_eq!(p["runs"], 2, "the run just over 7 days ago isn't counted");
            assert_eq!(p["cost_usd"], 6.0);
            assert_eq!(p["previous"]["runs"], 2, "the 7 days before the last 7, and not a minute more");
            assert_eq!(p["previous"]["cost_usd"], 9.0);
            assert_eq!(p["previous"]["until"], p["since"], "the 7 days before end where the last 7 start");
            assert_eq!(s["agents"][0]["cost_usd"], 15.0, "the agent card still counts 14 days");
            assert_eq!(s["all_time"]["runs"], 5);
        }
    }

    #[test]
    fn every_fresh_context_option_stays_settable_on_a_new_agent() {
        let st = crate::tasks::tests::state();
        let (mut a, _) = super::new_agent(&st, uuid::Uuid::new_v4().to_string(), &serde_json::json!({ "name": "Dev", "role": "dev" }), None).unwrap();
        assert_eq!(a.fresh_context, "always");
        for v in crate::agents::FRESH_CONTEXT {
            apply_patch(&st, &mut a, &serde_json::json!({ "fresh_context": v })).unwrap();
            assert_eq!(a.fresh_context, *v);
        }
        assert!(apply_patch(&st, &mut a, &serde_json::json!({ "fresh_context": "above_90k" })).is_err());
    }

    #[test]
    fn the_role_is_set_through_the_patch() {
        let st = crate::tasks::tests::state();
        let (mut a, _) = super::new_agent(&st, uuid::Uuid::new_v4().to_string(), &serde_json::json!({ "name": "Planner" }), None).unwrap();
        assert_eq!(a.role, "custom", "a new agent is Custom");
        for role in ["dev", "code_reviewer", "qa", "security", "custom"] {
            apply_patch(&st, &mut a, &serde_json::json!({ "role": role })).unwrap();
            assert_eq!(a.role, role);
        }
        for role in ["reviewer", "", "Lead"] {
            assert!(apply_patch(&st, &mut a, &serde_json::json!({ "role": role })).is_err(), "{role:?}");
        }
    }

    async fn patch(st: &crate::state::AppState, id: &str, p: serde_json::Value) -> Result<serde_json::Value, super::ApiError> {
        use axum::extract::{Path, State};
        super::update_agent(State(st.clone()), Path(id.into()), axum::Json(p)).await.map(|j| j.0)
    }

    #[tokio::test]
    async fn one_lead_per_group() {
        use crate::tasks::tests::{agent, state};
        let st = state();
        let mut ids = vec![];
        for (name, group) in [("Dev", "homebase"), ("Lead", "homebase"), ("Other", "imole"), ("Loose", "")] {
            let mut a = agent(&st, name);
            (a.group, a.project) = (group.into(), group.into());
            st.db.save_agent(&a).unwrap();
            ids.push(a.id);
        }
        let get = |i: usize| st.db.get_agent(&ids[i]).unwrap().unwrap();

        let lead = serde_json::json!({ "role": "lead" });
        let v = patch(&st, &ids[0], lead.clone()).await.unwrap();
        assert_eq!((v["role"].clone(), v.get("notice")), ("lead".into(), None));
        patch(&st, &ids[2], lead.clone()).await.unwrap();

        // A second Lead in the same group takes the role: the first becomes Custom. Other groups keep theirs.
        let v = patch(&st, &ids[1], lead.clone()).await.unwrap();
        assert_eq!(v["notice"], "Lead moved from Dev to Lead.");
        assert_eq!(v["lead_moved_from"]["id"], ids[0].as_str());
        assert_eq!(get(0).role, "custom");
        assert_eq!((get(1).role.as_str(), get(2).role.as_str()), ("lead", "lead"));

        // No group, no Lead: neither setting it nor clearing the Lead's group.
        let err = patch(&st, &ids[3], lead.clone()).await.unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
        assert!(err.1.starts_with("Only an agent with a group can be Lead"), "{}", err.1);
        assert_eq!(get(3).role, "custom");
        assert!(patch(&st, &ids[1], serde_json::json!({ "group": "" })).await.is_err());
        patch(&st, &ids[1], serde_json::json!({ "role": "dev", "group": "" })).await.unwrap();
        assert_eq!(get(1).role, "dev");

        // Moving the Lead to another group takes that group's Lead role.
        let v = patch(&st, &ids[0], serde_json::json!({ "role": "lead", "group": "imole" })).await.unwrap();
        assert_eq!(v["notice"], "Lead moved from Other to Dev.");
        assert_eq!(get(2).role, "custom");
    }

    #[tokio::test]
    async fn one_lead_per_group_across_its_projects() {
        use crate::tasks::tests::{agent, state};
        let st = state();
        let mut ids = vec![];
        for (name, group, project) in [("Api Dev", "Acme", "api"), ("Web Dev", "Acme", "b2c"), ("Ida", "", "b2c")] {
            let mut a = agent(&st, name);
            (a.group, a.project) = (group.into(), project.into());
            st.db.save_agent(&a).unwrap();
            ids.push(a.id);
        }
        patch(&st, &ids[0], serde_json::json!({ "role": "lead" })).await.unwrap();
        // Same group, another project: the mark moves. No group: a project alone makes no team.
        let v = patch(&st, &ids[1], serde_json::json!({ "role": "lead" })).await.unwrap();
        assert_eq!(v["notice"], "Lead moved from Api Dev to Web Dev.");
        assert!(patch(&st, &ids[2], serde_json::json!({ "role": "lead" })).await.is_err());
        // A group alone is enough to lead.
        let v = patch(&st, &ids[0], serde_json::json!({ "role": "lead", "project": "", "group": "Imole" })).await.unwrap();
        assert_eq!((v["role"].clone(), v["group"].clone(), v["projects"].clone()), ("lead".into(), "Imole".into(), serde_json::json!([])));
    }

    #[test]
    fn group_and_projects_are_validated_and_the_main_project_follows_them() {
        let st = crate::tasks::tests::state();
        for d in ["b2c", "b2b", "apps", "api", "work/notes"] {
            std::fs::create_dir_all(st.settings.boot().projects_dir.join(d)).unwrap();
        }
        let (mut a, _) = super::new_agent(&st, uuid::Uuid::new_v4().to_string(), &serde_json::json!({ "name": "Web", "group": " Acme ", "projects": ["b2c", " b2b ", "b2c", "", "work/notes"] }), None).unwrap();
        assert_eq!((a.group.as_str(), a.project.as_str()), ("Acme", "b2c"));
        assert_eq!(a.projects, ["b2c", "b2b", "work/notes"], "trimmed, each once, main first");
        let (old, _) = super::new_agent(&st, uuid::Uuid::new_v4().to_string(), &serde_json::json!({ "name": "Old", "project": "api" }), None).unwrap();
        assert_eq!((old.group.as_str(), old.projects.as_slice()), ("", &["api".to_string()][..]), "a client without groups: no group, so no team");

        // The star moves the main project to the front.
        apply_patch(&st, &mut a, &serde_json::json!({ "projects": ["b2b", "b2c"] })).unwrap();
        assert_eq!((a.project.as_str(), a.projects.as_slice()), ("b2b", &["b2b".to_string(), "b2c".into()][..]));

        // A client that only knows `project` changes the main one and keeps the rest.
        apply_patch(&st, &mut a, &serde_json::json!({ "project": "b2c" })).unwrap();
        assert_eq!(a.projects, ["b2c", "b2b"]);
        apply_patch(&st, &mut a, &serde_json::json!({ "project": "apps" })).unwrap();
        assert_eq!(a.projects, ["apps", "b2c", "b2b"]);

        // Paths, long lists and odd groups are refused, and change nothing.
        let before = (a.group.clone(), a.projects.clone());
        for bad in [
            serde_json::json!({ "projects": ["../etc"] }),
            serde_json::json!({ "projects": ["/Users/x"] }),
            serde_json::json!({ "projects": ["~/dev"] }),
            serde_json::json!({ "projects": ["api", "a/../../b"] }),
            serde_json::json!({ "projects": "api" }),
            serde_json::json!({ "projects": (0..21).map(|i| format!("p{i}")).collect::<Vec<_>>() }),
            serde_json::json!({ "project": "../x" }),
            serde_json::json!({ "group": "x".repeat(101) }),
            serde_json::json!({ "group": "Dine\nout" }),
        ] {
            let mut b = a.clone();
            assert!(apply_patch(&st, &mut b, &bad).is_err(), "accepted {bad}");
        }
        assert_eq!((a.group.clone(), a.projects.clone()), before);

        apply_patch(&st, &mut a, &serde_json::json!({ "projects": [] })).unwrap();
        assert_eq!((a.project.as_str(), a.projects.len()), ("", 0));
    }

    #[tokio::test]
    async fn a_missing_folder_is_refused_and_flagged_and_a_folder_of_repos_is_only_a_hint() {
        let st = crate::tasks::tests::state();
        let root = st.settings.boot().projects_dir.clone();
        std::fs::create_dir_all(root.join("Dineout/api/.git")).unwrap();
        std::fs::create_dir_all(root.join("Dineout/web/.git")).unwrap();
        std::fs::create_dir_all(root.join("fresh")).unwrap();
        std::fs::create_dir_all(root.join("solo/.git")).unwrap();

        // A new agent and a patch: a missing folder is refused, naming the project.
        for bad in [serde_json::json!({ "projects": ["Passora"] }), serde_json::json!({ "project": "Passora" })] {
            let e = super::new_agent(&st, "x".into(), &serde_json::json!({ "name": "A" }).as_object().unwrap().iter().chain(bad.as_object().unwrap()).map(|(k, v)| (k.clone(), v.clone())).collect::<serde_json::Value>(), None).unwrap_err();
            assert_eq!(e.1, "There's no folder Passora in the projects folder");
        }
        // No project at all is fine, and so is a folder of several repos.
        super::new_agent(&st, "a0".into(), &serde_json::json!({ "name": "None" }), None).unwrap();
        super::new_agent(&st, "a2".into(), &serde_json::json!({ "name": "Many", "projects": ["Dineout"] }), None).unwrap();
        let (mut a, _) = super::new_agent(&st, "a1".into(), &serde_json::json!({ "name": "A", "projects": ["solo", "fresh"] }), None).unwrap();
        assert!(apply_patch(&st, &mut a.clone(), &serde_json::json!({ "projects": ["solo", "Passora"] })).is_err());
        apply_patch(&st, &mut a.clone(), &serde_json::json!({ "projects": ["solo", "Dineout"] })).unwrap();
        assert_eq!(super::agent_json(&st, &a)["project_problems"], serde_json::json!([]));

        // An agent that already has them keeps them, flagged; saving its other fields still works.
        a.projects = vec!["Dineout".into(), "Passora".into(), "solo".into()];
        a.project = "Dineout".into();
        apply_patch(&st, &mut a, &serde_json::json!({ "name": "Dev", "projects": ["Dineout", "Passora", "solo"] })).unwrap();
        assert_eq!(a.projects, ["Dineout", "Passora", "solo"]);
        let flags = super::agent_json(&st, &a)["project_problems"].clone();
        let flagged: Vec<_> = flags.as_array().unwrap().iter().map(|f| (f["project"].as_str().unwrap(), f["hint"].as_bool().unwrap())).collect();
        assert_eq!(flagged, [("Dineout", true), ("Passora", false)]);
    }

    #[tokio::test]
    async fn the_live_agent_update_carries_its_project_problems() {
        use crate::tasks::tests::{agent, state};
        let st = state();
        let root = st.settings.boot().projects_dir.clone();
        for d in ["web", "dine/api/.git", "dine/pos/.git"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut a = agent(&st, "Old");
        (a.project, a.projects) = ("dine".into(), vec!["dine".into(), "gone".into()]);
        st.db.save_agent(&a).unwrap();
        let problems = super::agent_json(&st, &a)["project_problems"].clone();
        assert_eq!(problems, serde_json::json!([
            { "project": "dine", "problem": "dine holds several repos, so its issues stay in Loft", "hint": true },
            { "project": "gone", "problem": "There's no folder gone in the projects folder", "hint": false },
        ]), "short, without the Mac's paths");

        // The settings panel takes the saved agent from the live event: fixing the projects clears the flag there.
        let mut rx = st.events.subscribe();
        patch(&st, &a.id, serde_json::json!({ "projects": ["web"] })).await.unwrap();
        let agent = loop {
            if let crate::state::Event::AgentUpdated { agent } = rx.try_recv().unwrap() {
                break agent;
            }
        };
        assert_eq!(agent["project_problems"], serde_json::json!([]));
    }

    #[test]
    fn project_problems_read_each_folder_once_in_a_while() {
        use crate::tasks::tests::{agent, state};
        let st = state();
        let mut a = agent(&st, "Busy");
        a.projects = vec!["late".into()];
        let flagged = |a: &crate::db::Agent| super::agent_json(&st, a)["project_problems"].as_array().unwrap().len();
        assert_eq!(flagged(&a), 1, "no folder yet");
        std::fs::create_dir_all(st.settings.boot().projects_dir.join("late")).unwrap();
        assert_eq!(flagged(&a), 1, "the next events reuse the answer instead of reading the disk");
        // Saving still checks the disk itself, and the saved agent's flags are read from it again.
        assert!(apply_patch(&st, &mut a, &serde_json::json!({ "projects": ["late"] })).is_ok());
        assert_eq!(flagged(&a), 0, "re-saving the same project after fixing its folder clears the flag");
    }

    #[test]
    fn agent_ids_cannot_leave_the_storage_folder() {
        assert!(is_plain_id("3f2b8c1e-5a4d-4e6f-9b7a-1c2d3e4f5a6b"));
        for bad in ["", "../../Users", "..%2F..", "a/b", "a\\b", ".", "..", "x\0y"] {
            assert!(!is_plain_id(bad), "accepted {bad:?}");
        }
    }
}

#[cfg(test)]
mod ls_tests {
    use super::ls;

    #[test]
    fn lists_up_to_the_projects_folder_and_no_further() {
        let root = crate::testdir::for_this_test("ls");
        let projects = root.join("projects");
        let ws = projects.join("loft/agent-local-storage/a1");
        std::fs::create_dir_all(ws.join("notes")).unwrap();
        std::fs::create_dir_all(projects.join("imole/brand_docs")).unwrap();
        std::fs::write(projects.join("imole/proposta.md"), "").unwrap();
        std::fs::write(root.join("secret.txt"), "").unwrap();
        let base = projects.join("work/notes");
        std::fs::create_dir_all(base.join("docs")).unwrap();
        let names = |p: &str| ls(&base, &[&ws, &projects], p).map(|(_, v)| v.iter().map(|e| e["name"].as_str().unwrap().to_string()).collect::<Vec<_>>());
        assert_eq!(names("").unwrap(), vec!["docs"], "starts at the project's folder");
        assert_eq!(names("../../imole").unwrap(), vec!["brand_docs", "proposta.md"], "folders first");
        assert!(names("../..").unwrap().contains(&"imole".to_string()));
        assert_eq!(names("../../loft/agent-local-storage/a1").unwrap(), vec!["notes"], "the workspace is reachable too");
        assert!(names("../../..").is_err(), "above the projects folder");
        assert!(names(root.to_str().unwrap()).is_err(), "an absolute path out of bounds");
        std::os::unix::fs::symlink(&root, base.join("escape")).unwrap();
        assert!(names("escape").is_err(), "a symlink out of bounds");
        assert!(names("nope").is_err());
    }
}

#[cfg(test)]
mod find_tests {
    use axum::extract::{Path, Query, State};
    use std::path::Path as FsPath;

    async fn find(st: &crate::state::AppState, id: &str, q: &str) -> Vec<(String, bool)> {
        let v = super::find_files(State(st.clone()), Path(id.to_string()), Query(super::FindQuery { q: Some(q.into()) })).await.unwrap().0;
        v.as_array().unwrap().iter().map(|e| (e["path"].as_str().unwrap().to_string(), e["dir"].as_bool().unwrap())).collect()
    }

    fn write(root: &FsPath, files: &[&str]) {
        for f in files {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
    }

    fn git(dir: &FsPath, args: &[&str]) {
        assert!(std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn searches_what_git_shows_under_the_project_and_nothing_else() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Dev");
        let projects = st.settings.boot().projects_dir.clone();
        // The project is a folder inside a repo; the repo's other folders are outside it.
        let repo = projects.join("mono");
        let web = repo.join("web");
        write(&repo, &["api/chat.rs", "outside-chat.md"]);
        write(&web, &[
            ".gitignore", "frontend/src/pages/agents/Chat.tsx", "chats/log.txt", "docs/c/h/a/t.md", "dist/chat.js", "secret-chat.log",
        ]);
        std::fs::write(web.join(".gitignore"), "dist/\n*.log\n").unwrap();
        write(&web, &(0..30).map(|i| format!("many/item{i}.md")).collect::<Vec<_>>().iter().map(String::as_str).collect::<Vec<_>>());
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        write(&web, &["notes/untracked-chat.md", "frontend/src/gone-chat.ts"]);
        git(&repo, &["add", "web/frontend/src/gone-chat.ts"]);
        std::fs::remove_file(web.join("frontend/src/gone-chat.ts")).unwrap();
        st.db.update_agent(&a.id, |a| a.project = "mono/web".into()).unwrap();

        assert_eq!(find(&st, &a.id, "item").await.len(), super::super::file_search::LIMIT, "at most 20");
        let paths: Vec<String> = find(&st, &a.id, "Chat").await.into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, vec![
            // Name matches: the tightest, then the shortest name. Folders count.
            "chats", "frontend/src/pages/agents/Chat.tsx", "notes/untracked-chat.md",
            // Matches only through the folders above.
            "chats/log.txt", "docs/c/h/a/t.md",
        ]);
        let few = find(&st, &a.id, "pageschat").await;
        assert_eq!(few, vec![("frontend/src/pages/agents/Chat.tsx".to_string(), false)], "a match through the path");
        let dirs = find(&st, &a.id, "agents").await;
        assert_eq!(dirs[0], ("frontend/src/pages/agents".to_string(), true), "folders holding files are found");
        assert!(find(&st, &a.id, "head").await.iter().all(|(p, _)| !p.contains(".git/")), "nothing from .git");
        assert!(find(&st, &a.id, "").await.is_empty());
    }

    #[tokio::test]
    async fn outside_git_it_walks_the_workspace_skipping_the_noise() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Dev");
        let ws = crate::agents::workspace(&st, &a.id);
        write(&ws, &["notes/chat.md", "node_modules/chat/index.js", "target/chat.rs", "app/.DS_Store", "app/src/chatter.ts"]);
        std::os::unix::fs::symlink(crate::testdir::for_this_test("link-target"), ws.join("tmp-link")).unwrap();

        let paths: Vec<String> = find(&st, &a.id, "chat").await.into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, vec!["notes/chat.md", "app/src/chatter.ts"]);
        assert_eq!(find(&st, &a.id, "notes").await[0], ("notes".to_string(), true), "folders too, the name match first");
        assert_eq!(find(&st, &a.id, "store").await, vec![], ".DS_Store is skipped");
    }

    #[tokio::test]
    async fn a_workspace_its_repo_ignores_is_walked_past_the_clones_git_folders() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Dev");
        // Like agent-local-storage/ in the Loft checkout: the workspaces live in a repo that ignores them.
        let checkout = st.cfg.storage_dir.parent().unwrap();
        std::fs::write(checkout.join(".gitignore"), "agents/\n").unwrap();
        git(checkout, &["init", "-q"]);
        let ws = crate::agents::workspace(&st, &a.id);
        write(&ws, &["clone/src/chat.rs"]);
        git(&ws.join("clone"), &["init", "-q"]);
        assert!(ws.join("clone/.git/HEAD").exists());

        assert_eq!(find(&st, &a.id, "chat").await, vec![("clone/src/chat.rs".to_string(), false)], "the walk ran, not git");
        for q in ["head", "config", ".git"] {
            assert!(find(&st, &a.id, q).await.iter().all(|(p, _)| !p.contains(".git")), "nothing from a clone's .git for {q:?}");
        }
    }
}
