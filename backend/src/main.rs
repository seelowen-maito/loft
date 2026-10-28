mod agents;
mod apns;
mod api;
mod avatar;
mod db;
mod devices;
mod github;
mod github_inbox;
mod greenfield;
mod grill;
mod issues;
mod needs_you;
mod planning;
mod guard;
mod mcp;
mod mentions;
mod prs;
mod push;
mod repos;
mod schedule;
mod sessions;
mod settings;
mod setup;
mod signing;
mod simulator;
mod state;
mod system;
mod tailnet;
mod tasks;
mod updater;
mod uploads;
mod voice;
mod usage;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};
use tokio::sync::broadcast;
use tower_http::services::{ServeDir, ServeFile};

use crate::state::{AppState, Config, Runtime};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    signing::ensure();

    // The repo root is the parent of the backend crate, unless overridden.
    let root = std::env::var("LOFT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf());
    let data_dir = root.join(".loft");
    let cfg = Config {
        port: std::env::var("LOFT_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7420),
        storage_dir: root.join("agent-local-storage"),
    };
    std::fs::create_dir_all(&cfg.storage_dir)?;
    std::fs::create_dir_all(&data_dir)?;

    let (events, _) = broadcast::channel(1024);
    let state = AppState {
        db: db::Db::open(&data_dir.join("loft.db"))?,
        cfg: Arc::new(cfg),
        events,
        rt: Arc::new(Mutex::new(Runtime::default())),
        repos: Arc::new(repos::RepoCache::default()),
        http: reqwest::Client::new(),
        gh_token: Arc::new(github::TokenCache::default()),
        gh_pace: Arc::new(github::Pace::default()),
        update: Arc::new(Mutex::new(updater::UpdateStatus::default())),
        settings: Arc::new(settings::Store::load(&data_dir)?),
        usage: Arc::new(usage::Cache::default()),
        system: Arc::new(system::Cache::default()),
        grill_locks: Arc::new(grill::Locks::default()),
        publish_locks: Arc::new(planning::Locks::default()),
        publish_known: Arc::new(planning::Known::default()),
    };
    setup::ensure_loft(&state);
    tokio::spawn(agents::scheduler(state.clone()));
    tokio::spawn(prs::poller(state.clone()));
    tokio::spawn(github_inbox::poller(state.clone()));
    tokio::spawn(prs::backfill(state.clone()));
    tokio::spawn(agents::backfill_context(state.clone()));
    tokio::spawn(updater::updater(state.clone(), root.clone()));
    tokio::spawn(system::sampler(state.clone()));
    tokio::spawn(grill::sweeper(state.clone()));
    tokio::spawn(needs_you::watcher(state.clone()));

    let api = Router::new()
        .route("/info", get(api::info))
        .route("/update", get(api::update_status))
        .route("/usage", get(api::usage))
        .route("/system", get(system::get))
        .route("/settings", get(api::get_settings).put(api::put_settings))
        .route("/restart", post(api::restart))
        .route("/events", get(api::events))
        .route("/repos", get(api::list_repos))
        .route("/repos/github", get(api::github))
        .route("/repos/{name}/commits", get(api::repo_commits))
        .route("/issues", get(issues::list))
        .route("/issues/{number}", get(issues::get))
        .route("/issues/{number}/send", post(issues::send))
        .route("/agents", get(api::list_agents).post(api::create_agent))
        .route("/agents/summary", get(api::agents_summary))
        .route("/agents/order", put(api::reorder_agents))
        .route("/agents/{id}", get(api::get_agent).patch(api::update_agent).delete(api::delete_agent))
        .route("/agents/{id}/messages", get(api::list_messages).post(api::send_message).delete(api::clear_messages))
        .route("/agents/{id}/messages/{mid}/team-setup", post(planning::send_team_setup))
        .route("/agents/{id}/uploads", post(uploads::upload).layer(DefaultBodyLimit::max(uploads::MAX_BYTES + 1)))
        .route("/agents/{id}/uploads/{file}", get(uploads::get))
        .route("/agents/{id}/run", post(api::run_now))
        .route("/agents/{id}/stop", post(api::stop_agent))
        .route("/agents/{id}/restart", post(api::restart_agent))
        .route("/agents/{id}/reset-session", post(api::reset_session))
        .route("/agents/{id}/compact", post(api::compact_session))
        .route("/agents/{id}/sessions", get(api::list_sessions))
        .route("/agents/{id}/resume-session", post(api::resume_session))
        .route("/agents/{id}/avatar/randomize", post(api::randomize_avatar))
        .route("/agents/{id}/memories", get(api::list_memories).post(api::add_memory))
        .route("/agents/{id}/memories/{mid}", delete(api::delete_memory))
        .route("/agents/{id}/files", get(api::list_files))
        .route("/agents/{id}/ls", get(api::list_dir))
        .route("/agents/{id}/open-editor", post(api::open_editor))
        .route("/simulator", get(simulator::booted))
        .route("/simulator/screenshot", get(simulator::screenshot))
        .route("/simulator/stream", get(simulator::stream))
        .route("/simulator/input", post(simulator::input))
        .route("/agents/{id}/grill", get(grill::get_active))
        .route("/grill/commands", get(grill::command_catalog))
        .route("/grill/{id}", get(grill::get))
        .route("/grill/{id}/command", post(grill::command))
        .route("/tasks", get(tasks::list).post(tasks::create_task))
        .route("/tasks/{id}", get(tasks::get).patch(tasks::update).delete(tasks::delete))
        .route("/tasks/{id}/execute", post(tasks::execute_task))
        .route("/tasks/{id}/cancel", post(tasks::cancel))
        .route("/tasks/{id}/comments", get(tasks::list_comments).post(tasks::add_comment))
        .route("/tasks/{id}/plan", get(tasks::plan))
        .route("/activity", get(tasks::activity))
        .route("/needs-you", get(needs_you::list))
        .route("/needs-you/{id}/dismiss", post(needs_you::dismiss_item))
        .route("/voice/clean", post(voice::clean))
        .route("/voice/brief", post(voice::brief))
        .route("/devices", get(devices::list).post(devices::register))
        .route("/devices/{token}", axum::routing::patch(devices::update).delete(devices::remove))
        .route("/devices/{token}/test", post(devices::test))
        .route("/push", get(devices::status).put(devices::configure))
        .route("/permissions", get(api::list_permissions))
        .route("/permissions/{id}", post(api::decide_permission))
        .route("/pull-requests", get(api::list_pull_requests))
        .route("/pull-requests/{id}/refresh", post(api::refresh_pull_request))
        .route("/pull-requests/{id}/merge", post(api::merge_pull_request))
        .layer(axum::middleware::from_fn(guard::same_origin_writes));

    // Serve the built frontend when present (production mode).
    let dist = root.join("frontend").join("dist");
    let app = Router::new()
        .nest("/api", api)
        .route("/mcp/{agent_id}", post(mcp::handle))
        // No CORS layer: every client is same-origin (the built frontend, the Vite proxy, `tailscale serve`)
        // or not a browser (the iOS app, agents' MCP calls). A permissive policy let any web page the user
        // visits read and drive the API from their browser.
        .fallback_service(ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html"))))
        .with_state(state.clone());

    let addr = format!("127.0.0.1:{}", state.cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("loft listening on http://{addr}");
    println!("  projects: {}", state.settings.boot().projects_dir.display());
    println!("  agents:   {}", state.cfg.storage_dir.display());
    println!("  github:   {}", if github::token(&state).is_some() { "token found" } else { "no token (public repos only, 60 req/h)" });
    axum::serve(listener, app).await?;
    Ok(())
}
