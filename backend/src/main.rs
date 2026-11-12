mod agents;
mod apns;
mod api;
mod avatar;
mod backup;
mod db;
mod delivery;
mod file_search;
mod devices;
mod git_check;
mod github;
mod github_inbox;
mod greenfield;
mod grill;
mod needs_you;
mod planning;
mod ports;
mod guard;
mod herdr;
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
mod sweep;
mod system;
mod tailnet;
mod tags;
mod tasks;
#[cfg(test)]
mod testdir;
mod updater;
mod uploads;
mod voice;
mod usage;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
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

/// Everything under `/api`.
fn api_routes() -> Router<AppState> {
    Router::new()
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
        .route("/agents/{id}/workspace-size", get(api::workspace_size))
        .route("/workspaces", get(sweep::sizes))
        .route("/workspaces/clean", post(sweep::clean))
        .route("/workspaces/clean/preview", get(sweep::preview))
        .route("/workspaces/clean/folders", get(sweep::folders))
        .route("/workspaces/clean/check", post(git_check::check))
        .route("/workspaces/log", get(sweep::log))
        .route("/workspaces/processes", get(processes::left_running))
        .route("/workspaces/{id}/stop-processes", post(processes::stop_left_running))
        .route("/agents/{id}/messages", get(api::list_messages).post(api::send_message).delete(api::clear_messages))
        .route("/agents/{id}/messages/{mid}/team-setup", post(planning::send_team_setup))
        .route("/agents/{id}/messages/{mid}/start-delivery", post(delivery::start_from_card))
        .route("/deliveries/state", get(delivery::state_route))
        .route("/deliveries/summary", get(delivery::summary_route))
        .route("/deliveries/merge", post(delivery::merge_route))
        .route("/deliveries/resume", post(delivery::resume_route))
        .route("/deliveries/reopen", post(delivery::reopen_route))
        .route("/agents/{id}/uploads", post(uploads::upload))
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
        .route("/simulator/devices", get(simulator::agent_devices))
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
        .route("/needs-you/seen", post(needs_you::mark_seen))
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
        // Herdr UI: Loft's agents in the Herdr terminal multiplexer (herdr.rs, herdr/ at the root).
        .route("/herdr", get(herdr::status))
        .route("/herdr/setup", post(herdr::setup_local))
        .route("/herdr/install.sh", get(herdr::install_script))
        .route("/herdr/files/{name}", get(herdr::file))
        .route("/herdr/bin/{platform}", get(herdr::binary))
        // GitHub requests a person's page or click makes count as `user` (a poller's have their own source).
        .layer(axum::middleware::from_fn(|req, next: axum::middleware::Next| github::as_source("user", next.run(req))))
        .layer(axum::middleware::from_fn(guard::same_origin_writes))
        .layer(axum::middleware::from_fn(guard::local_host))
}

/// The whole server. `dist` is the built frontend, served when present (production mode).
fn app(state: &AppState, dist: &std::path::Path) -> Router {
    Router::new()
        .nest("/api", api_routes())
        .route("/mcp/{agent_id}", post(mcp::handle).layer(axum::middleware::from_fn(guard::local_host)))
        // Agents' `kill`, `pkill` and `killall` (processes.rs). JSON only, so a web page can't send one.
        .route("/kill/{agent_id}", post(processes::handle).layer(axum::middleware::from_fn(guard::local_host)))
        // No CORS layer: every client is same-origin (the built frontend, the Vite proxy, `tailscale serve`)
        // or not a browser (the iOS app, agents' MCP calls). A permissive policy let any web page the user
        // visits read and drive the API from their browser.
        .fallback_service(ServeDir::new(dist).fallback(ServeFile::new(dist.join("index.html"))))
        .layer(compression())
        .with_state(state.clone())
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
        needs_you: Arc::new(Default::default()),
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
    tokio::spawn(delivery::ci_poller(state.clone()));
    tokio::spawn(backup::scheduler(state.clone()));
    tokio::spawn(sweep::scheduler(state.clone()));
    tokio::spawn(simulator::tidier(state.clone()));
    tokio::spawn(processes::idle_stopper(state.clone()));

    // Serve the built frontend when present (production mode).
    let app = app(&state, &root.join("frontend").join("dist"));

    let addr = format!("127.0.0.1:{}", state.cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("loft listening on http://{addr}");
    println!("  projects: {}", state.settings.boot().projects_dir.display());
    println!("  agents:   {}", state.cfg.storage_dir.display());
    println!("  github:   {}", if github::token(&state).is_some() { "token found" } else { "no token (public repos only, 60 req/h)" });
    axum::serve(listener, app).await?;
    Ok(())
}

/// Gzip or brotli, as the client asks, for text bodies of 1 KB and up: JSON, JS, CSS, HTML and SVG. Images,
/// video and event streams go as they are, so live updates aren't held back to fill a compressed block.
fn compression() -> tower_http::compression::CompressionLayer<impl tower_http::compression::Predicate> {
    use tower_http::compression::{CompressionLayer, CompressionLevel, Predicate, predicate::SizeAbove};
    let text = |_: axum::http::StatusCode, _: axum::http::Version, h: &axum::http::HeaderMap, _: &axum::http::Extensions| {
        let ty = h.get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
        let ty = ty.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        matches!(ty.as_str(), "application/json" | "text/javascript" | "application/javascript" | "text/css" | "text/html" | "image/svg+xml")
    };
    // Brotli's default (its best) takes seconds on a large list; 4 compresses about as well as gzip's best, fast.
    CompressionLayer::new().quality(CompressionLevel::Precise(4)).compress_when(SizeAbove::new(1024).and(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test(flavor = "multi_thread")]
    async fn the_api_and_mcp_answer_only_to_this_macs_host_and_the_herdr_build_to_same_site_reads() {
        let st = tasks::tests::state();
        let dist = crate::testdir::for_this_test("dist-host");
        std::fs::create_dir_all(&dist).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = app(&st, &dist);
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let http = reqwest::Client::new();
        let status = |req: reqwest::RequestBuilder| async move { req.send().await.unwrap().status().as_u16() };
        // The cookie of the removed pairing is told to go.
        let r = http.get(format!("{base}/api/info")).header("cookie", "__Host-loft=old").send().await.unwrap();
        assert!(r.headers().get("set-cookie").unwrap().to_str().unwrap().starts_with("__Host-loft=; Max-Age=0"));
        assert!(http.get(format!("{base}/api/info")).send().await.unwrap().headers().get("set-cookie").is_none());
        // A page rebound to this Mac still sends its own name as Host.
        assert_eq!(status(http.get(format!("{base}/api/info"))).await, 200, "the address as typed");
        assert_eq!(status(http.get(format!("{base}/api/info")).header("host", "localhost:7420")).await, 200);
        assert_eq!(status(http.get(format!("{base}/api/info")).header("host", "evil.example:7420")).await, 403);
        assert_eq!(status(http.get(format!("{base}/api/info")).header("host", "127.0.0.1.evil.example")).await, 403);
        assert_eq!(status(http.post(format!("{base}/mcp/nobody")).header("host", "evil.example").body("{}")).await, 403);
        assert_ne!(status(http.post(format!("{base}/mcp/nobody")).body("{}")).await, 403);
        // A LAN address can't be rebound, so it is served; `/kill` is checked like the rest.
        assert_eq!(status(http.get(format!("{base}/api/info")).header("host", "192.168.1.20:7420")).await, 200);
        let kill = format!("{base}/kill/nobody");
        assert_eq!(status(http.post(&kill).header("host", "evil.example").json(&json!({}))).await, 403);
        assert_ne!(status(http.post(&kill).json(&json!({}))).await, 403);
        // The herdr build is for same-site reads only; the platform here is wrong so no build starts.
        let bin = format!("{base}/api/herdr/bin/nowhere");
        assert_eq!(status(http.get(&bin).header("sec-fetch-site", "cross-site")).await, 403);
        assert_eq!(status(http.get(&bin).header("sec-fetch-site", "same-origin")).await, 404);
        assert_eq!(status(http.get(&bin)).await, 404);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn text_is_compressed_when_asked_and_images_video_small_bodies_and_events_are_not() {
        let st = tasks::tests::state();
        for i in 0..40 {
            tasks::create(&st, &json!({ "title": format!("Issue number {i}"), "body": "Some words that repeat. ".repeat(20) }), "user").unwrap();
        }
        let dist = crate::testdir::for_this_test("dist");
        std::fs::create_dir_all(&dist).unwrap();
        for f in ["app.js", "app.css", "pic.png", "clip.mp4"] {
            std::fs::write(dist.join(f), "x".repeat(4096)).unwrap();
        }
        let app = Router::new()
            .nest("/api/issues", tasks::routes())
            .route("/api/info", get(api::info))
            .route("/api/events", get(api::events))
            .fallback_service(ServeDir::new(&dist))
            .layer(compression())
            .with_state(st.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        // Built without reqwest's decompression: bodies arrive as sent.
        let http = reqwest::Client::new();
        let get = |path: &str, accept: &str| {
            let req = http.get(format!("{base}{path}"));
            if accept.is_empty() { req } else { req.header("accept-encoding", accept) }
        };
        let encoding = |r: &reqwest::Response| r.headers().get("content-encoding").map(|v| v.to_str().unwrap().to_string());

        let plain = get("/api/issues", "").send().await.unwrap();
        assert_eq!(encoding(&plain), None);
        let plain = plain.bytes().await.unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&plain).unwrap().as_array().unwrap().len() == 40);
        for enc in ["gzip", "br"] {
            let r = get("/api/issues", enc).send().await.unwrap();
            assert_eq!(encoding(&r).as_deref(), Some(enc));
            let n = r.bytes().await.unwrap().len();
            assert!(n * 4 < plain.len(), "{enc}: {n} of {} bytes", plain.len());
        }
        for path in ["/app.js", "/app.css"] {
            assert_eq!(encoding(&get(path, "gzip, br").send().await.unwrap()).as_deref(), Some("br"), "{path}");
        }
        for path in ["/pic.png", "/clip.mp4", "/api/info"] {
            let r = get(path, "gzip, br").send().await.unwrap();
            assert!(r.status().is_success(), "{path}: {}", r.status());
            assert_eq!(encoding(&r), None, "{path}");
        }
        assert!(get("/api/info", "").send().await.unwrap().content_length().unwrap() < 1024, "/api/info isn't small any more");

        // The event stream's first message arrives right away, uncompressed.
        let mut r = get("/api/events", "gzip, br").send().await.unwrap();
        assert_eq!(encoding(&r), None);
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), r.chunk()).await.expect("the stream is held back").unwrap().unwrap();
        assert!(String::from_utf8_lossy(&first).contains("connected"), "{first:?}");
    }
}
