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
    let mut v = serde_json::to_value(a).unwrap();
    agents::run_fields(state, &a.id, &mut v);
    v["workspace"] = json!(workspace(state, &a.id).display().to_string());
    v["grill_enabled"] = json!(crate::grill::enabled(a));
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
        "features": ["tasks", "activity", "idempotent_messages", "voice_clean", "system", "grill", "run_watchdog", "task_comments", "push", "needs_you", "plans", "mentions", "voice_brief"],
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
    let mut s = state.settings.get();
    let token_set = !s.github_token.trim().is_empty();
    s.github_token = String::new();
    json!({
        "settings": s,
        "github_token_set": token_set,
        "restart_fields": crate::settings::RESTART_FIELDS,
        "pending_restart": state.settings.pending_restart(),
        "running_agents": running_count(state),
        "imessage_inbox_error": *state.inbox_error.lock().unwrap(),
    })
}

pub async fn get_settings(State(state): State<AppState>) -> Json<Value> {
    Json(settings_view(&state))
}

/// Replace the settings. `github_token`: absent or null keeps the saved one, "" clears it.
pub async fn put_settings(State(state): State<AppState>, Json(mut body): Json<Value>) -> ApiResult<Value> {
    let token = body.as_object_mut().and_then(|o| o.remove("github_token"));
    // Push is set on its own endpoint (`PUT /api/push`); clients that don't send it keep it.
    let keep_push = body.get("push").is_none();
    let mut s: crate::settings::Settings = serde_json::from_value(body).map_err(|e| bad(e.to_string()))?;
    if keep_push {
        s.push = state.settings.get().push;
    }
    s.github_token = match token {
        Some(Value::String(t)) => t.trim().to_string(),
        _ => state.settings.get().github_token,
    };
    state.settings.save(s).map_err(|e| bad(e.to_string()))?;
    Ok(Json(settings_view(&state)))
}

#[derive(Deserialize)]
pub struct TestMessage {
    to: String,
}

pub async fn test_imessage(Json(m): Json<TestMessage>) -> ApiResult<Value> {
    if m.to.trim().is_empty() {
        return Err(bad("Enter a phone number or Apple ID email first"));
    }
    crate::notify::deliver(m.to.trim(), "loft: test message. Notifications will arrive here.")
        .await
        .map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
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
    // Let this response go out first.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        crate::updater::restart();
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

const DEFAULT_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "Edit", "Write", "TodoWrite",
    "Bash(git *)", "Bash(ls *)", "Bash(cat *)", "Bash(mkdir *)", "WebSearch",
];

/// Apply editable fields from a JSON patch onto an agent.
fn apply_patch(a: &mut Agent, p: &Value) -> Result<(), ApiError> {
    let str_field = |k: &str| p.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    if let Some(v) = str_field("name") {
        if v.trim().is_empty() {
            return Err(bad("name cannot be empty"));
        }
        a.name = v.trim().into();
    }
    if let Some(v) = str_field("title") { a.title = v; }
    if let Some(v) = str_field("project") { a.project = v.trim().into(); }
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
    if let Some(v) = p.get("grill") {
        a.grill = match v {
            Value::Null => None,
            v => Some(v.as_bool().ok_or_else(|| bad("grill must be true, false or null"))?),
        };
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
    Ok(())
}

pub async fn create_agent(State(state): State<AppState>, Json(p): Json<Value>) -> ApiResult<Value> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut a = Agent {
        id: id.clone(),
        name: "New agent".into(),
        title: String::new(),
        project: String::new(),
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
        grill: None,
        avatar: Avatar::generate(&id),
        status: "idle".into(),
        status_text: String::new(),
        session_id: None,
        context_tokens: None,
        last_run_at: None,
        next_run_at: None,
        total_cost_usd: 0.0,
        created_at: now(),
    };
    apply_patch(&mut a, &p)?;
    std::fs::create_dir_all(workspace(&state, &id)).map_err(anyhow::Error::from)?;
    std::fs::write(
        workspace(&state, &id).join("README.md"),
        format!("# {}'s workspace\n\nPrivate storage for the Loft agent `{}` ({id}).\n", a.name, a.name),
    )
    .map_err(anyhow::Error::from)?;
    state.db.save_agent(&a)?;
    emit_agent(&state, &a);
    Ok(Json(agent_json(&state, &a)))
}

pub async fn update_agent(State(state): State<AppState>, Path(id): Path<String>, Json(p): Json<Value>) -> ApiResult<Value> {
    let mut a = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    apply_patch(&mut a, &p)?;
    state.db.save_agent(&a)?;
    emit_agent(&state, &a);
    Ok(Json(agent_json(&state, &a)))
}

#[derive(Deserialize)]
pub struct Order {
    ids: Vec<String>,
}

pub async fn reorder_agents(State(state): State<AppState>, Json(o): Json<Order>) -> ApiResult<Value> {
    state.db.reorder_agents(&o.ids)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct DeleteOpts {
    #[serde(default)]
    purge_files: bool,
}

pub async fn delete_agent(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<DeleteOpts>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    agents::stop(&state, &id);
    state.db.delete_agent(&id)?;
    crate::uploads::remove_all(&state, &id);
    if q.purge_files {
        let _ = std::fs::remove_dir_all(workspace(&state, &id));
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
    if let Some(c) = &client_id {
        if let Some(prev) = state.db.find_client_message(&id, c)? {
            return Ok(Json(json!({ "ok": true, "queued": false, "duplicate": true, "message": prev })));
        }
    }
    let queued = is_running(&state, &id);
    let meta = client_id.map(|c| json!({ "client_id": c })).unwrap_or(json!({}));
    let agent = state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let grill = m.meta.as_ref().and_then(|v| v.get("grill")).filter(|g| g.is_object());
    let meta = match crate::grill::on_user_message(&state, &agent, m.text.trim(), grill, meta)? {
        crate::grill::Routed::Normal(meta) => meta,
        crate::grill::Routed::Handled { message, wake } => {
            if let Some(p) = wake {
                agents::wake(&state, &id, p.prompt, p.trigger);
            }
            return Ok(Json(json!({ "ok": true, "queued": queued, "message": message })));
        }
    };
    let mut meta = meta;
    if !m.images.is_empty() {
        meta["images"] = json!(m.images);
    }
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
    let all = state.db.list_agents()?;
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
                "[Loft: the user restarted your last run ({why}); anything it still had running was stopped. Pick up where you left off: finish the work, or say what's left.]"
            ));
        }
        rt.restarting.insert(id.clone());
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
        rt.grill_queued.remove(&id);
    }
    let a = state.db.update_agent(&id, |a| {
        a.session_id = None;
        a.context_tokens = None;
    })?.ok_or_else(not_found)?;
    let m = state.db.add_message(&id, None, "system", "clear", "Context cleared. Long-term memory carries over.", json!({}))?;
    state.emit(Event::Message { message: json!(m) });
    emit_agent(&state, &a);
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

pub async fn clear_messages(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    state.db.clear_messages(&id)?;
    crate::uploads::remove_all(&state, &id);
    Ok(Json(json!({ "ok": true })))
}

pub async fn randomize_avatar(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let seed = format!("{id}-{}", now());
    let a = state
        .db
        .update_agent(&id, |a| a.avatar = Avatar::generate(&seed))?
        .ok_or_else(not_found)?;
    emit_agent(&state, &a);
    Ok(Json(agent_json(&state, &a)))
}

// ---- memories ---------------------------------------------------------

pub async fn list_memories(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(json!(state.db.list_memories(&id)?)))
}

#[derive(Deserialize)]
pub struct NewMemory {
    content: String,
    #[serde(default)]
    tags: String,
}

pub async fn add_memory(State(state): State<AppState>, Path(id): Path<String>, Json(m): Json<NewMemory>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    if m.content.trim().is_empty() {
        return Err(bad("empty memory"));
    }
    let mem = state.db.add_memory(&id, m.content.trim(), m.tags.trim())?;
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

// ---- pull requests ----------------------------------------------------

#[derive(Deserialize)]
pub struct PrQuery {
    agent_id: Option<String>,
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

/// Per-agent activity for the home page over the last `DAYS` days: daily runs
/// (local days, split autonomous vs on request), totals, PR outcomes, and
/// anything waiting on the user.
pub async fn agents_summary(State(state): State<AppState>) -> ApiResult<Value> {
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
    Ok(Json(json!(out)))
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
mod tests {
    use super::is_plain_id;

    #[test]
    fn agent_ids_cannot_leave_the_storage_folder() {
        assert!(is_plain_id("3f2b8c1e-5a4d-4e6f-9b7a-1c2d3e4f5a6b"));
        for bad in ["", "../../Users", "..%2F..", "a/b", "a\\b", ".", "..", "x\0y"] {
            assert!(!is_plain_id(bad), "accepted {bad:?}");
        }
    }
}
