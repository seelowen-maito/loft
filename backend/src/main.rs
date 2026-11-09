mod agents;
mod apns;
mod api;
mod avatar;
mod backup;
mod db;
mod delivery;
mod file_search;
mod devices;
mod github;
mod github_inbox;
mod greenfield;
mod grill;
mod needs_you;
mod planning;
mod ports;
mod guard;
mod kill_guard;
mod mac_access;
mod mcp;
mod mentions;
mod mirror;
mod processes;
mod prs;
mod push;
mod signin;
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

fn main() -> anyhow::Result<()> {
    // As the agents' Bash hook, claude runs this binary before every Bash call: answer before any setup.
    match std::env::args().nth(1).as_deref() {
        Some(kill_guard::ARG) => kill_guard::hook(),
        // And as their `kill`, `pkill` and `killall`.
        Some(processes::ARG) => processes::cli(),
        _ => {}
    }
    serve()
}

#[tokio::main]
async fn serve() -> anyhow::Result<()> {
    // Loft takes no arguments. Refuse any before opening the db or binding a port: an older binary run as a newer
    // Loft's hook (`loft kill-guard`, or a later subcommand) must not start a whole server on every call.
    if let Some(arg) = std::env::args().nth(1) {
        anyhow::bail!("unknown argument {arg:?}: loft takes no arguments");
    }
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
        xcrun: "xcrun".into(),
        xcode: state::XcodeLookup::system(),
    };
    std::fs::create_dir_all(&cfg.storage_dir)?;
    std::fs::create_dir_all(&data_dir)?;
    agents::remove_old_agent_files(&cfg.storage_dir);

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
        gh_budget: Arc::new(Default::default()),
        update: Arc::new(Mutex::new(updater::UpdateStatus::default())),
        settings: Arc::new(settings::Store::load(&data_dir)?),
        usage: Arc::new(usage::Cache::default()),
        system: Arc::new(system::Cache::default()),
        grill_locks: Arc::new(grill::Locks::default()),
        publish_locks: Arc::new(planning::Locks::default()),
        publish_known: Arc::new(planning::Known::default()),
        mirror: Arc::new(Default::default()),
        listings: Arc::new(Default::default()),
        xcode: Arc::new(Default::default()),
    };
    setup::ensure_loft(&state);
    {
        let with_repo: Vec<String> = repos::list(&state, false).await.into_iter().filter(|r| r.github.is_some()).map(|r| r.name).collect();
        if let Err(e) = state.settings.migrate_github_settings(&with_repo) {
            tracing::warn!("GitHub settings migration: {e}");
        }
    }
    // Before any run starts: the runs Deliveries were waiting on were lost with the last process.
    delivery::stall_lost_runs(&state);
    tokio::spawn(agents::scheduler(state.clone()));
    tokio::spawn(prs::poller(state.clone()));
    tokio::spawn(github_inbox::poller(state.clone()));
    tokio::spawn(prs::backfill(state.clone()));
    tokio::spawn(delivery::track_open_prs(state.clone()));
    tokio::spawn(agents::backfill_context(state.clone()));
    tokio::spawn(updater::updater(state.clone(), root.clone()));
    tokio::spawn(system::sampler(state.clone()));
    tokio::spawn(grill::sweeper(state.clone()));
    tokio::spawn(needs_you::watcher(state.clone()));
    tokio::spawn(agents::quiet_watcher(state.clone()));
    tokio::spawn(processes::pruner(state.cfg.storage_dir.clone()));
    tokio::spawn(mac_access::watcher(state.clone()));
    tokio::spawn(mirror::poller(state.clone()));
    tokio::spawn(backup::scheduler(state.clone()));

    let api = Router::new()
        .route("/info", get(api::info))
        .route("/update", get(api::update_status))
        .route("/usage", get(api::usage))
        .route("/system", get(system::get))
        .route("/system/mac-access", post(mac_access::check))
        .route("/settings", get(api::get_settings).put(api::put_settings))
        .route("/backups", get(backup::get).post(backup::now))
        .route("/restart", post(api::restart))
        .route("/events", get(api::events))
        .route("/repos", get(api::list_repos))
        .route("/repos/github", get(api::github))
        .route("/repos/{name}/commits", get(api::repo_commits))
        .route("/agents", get(api::list_agents).post(api::create_agent))
        .route("/agents/summary", get(api::agents_summary))
        .route("/agents/order", put(api::reorder_agents))
        .route("/agents/{id}", get(api::get_agent).patch(api::update_agent).delete(api::delete_agent))
        .route("/agents/{id}/messages", get(api::list_messages).post(api::send_message).delete(api::clear_messages))
        .route("/agents/{id}/messages/{mid}/team-setup", post(planning::send_team_setup))
        .route("/agents/{id}/messages/{mid}/start-delivery", post(delivery::start_from_card))
        .route("/deliveries/state", get(delivery::state_route))
        .route("/deliveries/summary", get(delivery::summary_route))
        .route("/deliveries/merge", post(delivery::merge_route))
        .route("/deliveries/resume", post(delivery::resume_route))
        .route("/agents/{id}/uploads", post(uploads::upload).layer(DefaultBodyLimit::max(uploads::MAX_BYTES + 1)))
        .route("/agents/{id}/uploads/{file}", get(uploads::get))
        .route("/agents/{id}/run", post(api::run_now))
        .route("/agents/{id}/stop", post(api::stop_agent))
        .route("/agents/{id}/read", post(api::mark_read))
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
        .route("/agents/{id}/find", get(api::find_files))
        .route("/agents/{id}/open-editor", post(api::open_editor))
        .route("/simulator", get(simulator::booted))
        .route("/simulator/screenshot", get(simulator::screenshot))
        .route("/simulator/stream", get(simulator::stream))
        .route("/simulator/input", post(simulator::input))
        .route("/agents/{id}/grill", get(grill::get_active))
        .route("/grill/commands", get(grill::command_catalog))
        .route("/grill/{id}", get(grill::get))
        .route("/grill/{id}/command", post(grill::command))
        .nest("/issues", tasks::routes())
        // The old name, kept until the iOS app moves to `/issues`.
        .nest("/tasks", tasks::routes())
        .route("/activity", get(tasks::activity))
        .route("/needs-you", get(needs_you::list))
        .route("/needs-you/{id}/dismiss", post(needs_you::dismiss_item))
        .route("/voice/clean", post(voice::clean))
        .route("/voice/brief", post(voice::brief))
        .route("/devices", get(devices::list).post(devices::register))
        .route("/devices/{token}", axum::routing::patch(devices::update).delete(devices::remove))
        .route("/devices/{token}/test", post(devices::test))
        .route("/push", get(devices::status).put(devices::configure))
        .route("/push/signin", post(signin::start))
        .route("/permissions", get(api::list_permissions))
        .route("/permissions/{id}", post(api::decide_permission))
        .route("/pull-requests", get(api::list_pull_requests))
        .route("/pull-requests/{id}/refresh", post(api::refresh_pull_request))
        .route("/pull-requests/{id}/merge", post(api::merge_pull_request))
        .route("/github/budget", get(github::budget))
        // GitHub requests a person's page or click makes count as `user` (a poller's have their own source).
        .layer(axum::middleware::from_fn(|req, next: axum::middleware::Next| github::as_source("user", next.run(req))))
        .layer(axum::middleware::from_fn(guard::same_origin_writes));

    // Serve the built frontend when present (production mode).
    let dist = root.join("frontend").join("dist");
    let app = Router::new()
        .nest("/api", api)
        .route("/mcp/{agent_id}", post(mcp::handle))
        // Agents' `kill`, `pkill` and `killall` (processes.rs). JSON only, so a web page can't send one.
        .route("/kill/{agent_id}", post(processes::handle))
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
