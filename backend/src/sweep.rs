//! Workspace disk space: Settings' Clean, and the daily sweep.
//!
//! Clean (one agent) and Clean all list each workspace's top-level items (`GET /api/workspaces/clean/preview`)
//! and remove the ones the user ticks (`POST /api/workspaces/clean`). Each project's main clone, `notes/` and
//! `uploads/` start unticked, and so do all the clones of an agent with no project; everything else starts ticked. A main clone opens up to its own folders, one level
//! at a time (`GET /api/workspaces/clean/folders`), so the user can remove just `backend/target`; its `.git`
//! never goes, and neither does the clone itself. A working agent's items can't be removed, and each path is
//! checked again just before it goes.
//!
//! The sweep deletes rebuildable build output (the table of `RULES`: Rust, Node, .NET, Gradle, Pods, Flutter,
//! Python and the like, each only beside its marker file) whose newest file is 3 days old, so idle workspaces don't fill the disk.
//! An agent that works often never leaves its `.cargo-build/` that old, so there its `*/incremental` caches go
//! once together they pass 2 GB, and `deps/` stays (rebuilding incremental takes seconds, deps about 30 s). It
//! runs once a day, before Loft restarts into an update, and on a workspace whose agent was deleted with its
//! files kept, and skips running agents and agents with a live process (a dev server).
//!
//! Neither follows a symlink or removes anything outside the agent's own workspace in `agent-local-storage/`,
//! so Loft's own checkout stays. Each run writes a log of what it removed, skipped and why, and the last one
//! stays readable (`GET /api/workspaces/log`).

use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Build output whose newest file is this old goes in the daily sweep.
pub const IDLE: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// How deep in a workspace clones are looked for (`<workspace>/a/b/<clone>`).
const DEPTH: usize = 3;
/// How often the day's sweep runs.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long a measured size is reused: Settings and the delete dialog reload often, and a walk is slow.
const SIZE_FOR: Duration = Duration::from_secs(60);
/// The last run's log, in Loft's data folder next to its database: outside the agents' workspaces, not out of
/// their reach (they run as the same Mac user; see ADR 0004's threat model).
const LOG_FILE: &str = "clean-log.json";
/// Past this, a young `.cargo-build/`'s `*/incremental` caches go.
const INCREMENTAL_CAP: u64 = 2_000_000_000;

#[cfg(test)]
thread_local!(static FAKE_CAP: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) });

fn incremental_cap() -> u64 {
    #[cfg(test)]
    if let Some(c) = FAKE_CAP.with(|c| c.get()) {
        return c;
    }
    INCREMENTAL_CAP
}

/// Under a main clone in Clean: what keeping it costs.
const KEPT: &str = "Kept: this is the agent's working copy. Its own build stays (e.g. backend/target or node_modules/), usually 1–5 GB, and the agent reuses it on its next build.";

/// A build folder the sweep would remove.
#[derive(Clone, Debug, Serialize)]
pub struct Folder {
    pub agent: String,
    pub path: PathBuf,
    /// Inside the agent's workspace, like `loft/backend/target`.
    pub rel: String,
    pub bytes: u64,
    /// Its newest file's change, what the age rule goes by.
    pub changed: DateTime<Local>,
    /// The shell command equivalent to its removal; Loft removes by path.
    pub command: String,
}

/// What a run leaves, and why.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Skip {
    pub agent: String,
    /// Inside the agent's workspace; empty for the whole workspace.
    pub rel: String,
    pub reason: String,
}

/// What the sweep would remove and what it would skip.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Plan {
    pub folders: Vec<Folder>,
    pub skipped: Vec<Skip>,
}

/// One thing Clean lists: a workspace's top-level item, or a folder inside a main clone.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub agent: String,
    pub path: PathBuf,
    /// Inside the agent's workspace, like `loft` or `loft/backend/target`.
    pub rel: String,
    pub name: String,
    /// "clone", "worktree", "folder", "file" or "symlink".
    pub kind: &'static str,
    pub bytes: u64,
    /// Its newest file's change.
    pub changed: DateTime<Local>,
    /// The shell command equivalent to its removal; Loft removes by path. Empty for `.git`, which never goes.
    pub command: String,
    /// Why it starts unticked, like "main clone of loft"; empty otherwise.
    pub label: String,
    /// Said under it, like what keeping a main clone costs; empty otherwise.
    pub note: String,
    /// What removing it risks, like "may contain unpushed work"; empty otherwise.
    pub warning: String,
    pub ticked: bool,
    /// Why it can't be ticked ("Working", "symlink", "git history: never deleted"); empty when it can.
    pub blocked: String,
    /// It opens up to its own folders (`GET /api/workspaces/clean/folders`).
    pub opens: bool,
}

/// `GET /api/workspaces/clean/preview` and `.../folders`.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Listing {
    pub items: Vec<Item>,
}

/// One removal: the folder and what it freed, or why it failed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Line {
    pub agent: String,
    pub path: PathBuf,
    pub rel: String,
    pub bytes: u64,
    pub error: Option<String>,
    #[serde(default)]
    pub command: String,
    /// What was done to `rel` when it isn't a removal, like "Shut down Simulator device"; empty for a removal.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub done: String,
}

/// A run's log: each removal as it happens, what was skipped, and the totals.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Log {
    pub at: Option<DateTime<Local>>,
    /// "Clean", "Clean all", "Daily sweep" or "Sweep before update".
    pub what: String,
    /// What to call it: "Cleaning Loft Tester…" while running, then "Cleaned Loft Tester", "Cleaned all agents"
    /// or "Daily sweep". Empty in a log saved before titles.
    #[serde(default)]
    pub title: String,
    pub running: bool,
    pub lines: Vec<Line>,
    pub skipped: Vec<Skip>,
    pub freed_bytes: u64,
    pub removed: usize,
    pub failed: usize,
}

impl Log {
    /// Adds a removal, its path and `rel` on one line whatever the folder's name.
    fn push(&mut self, mut l: Line) {
        (l.path, l.rel) = (one_line(&l.path.to_string_lossy()).into(), one_line(&l.rel));
        if l.error.is_some() {
            self.failed += 1;
        } else {
            self.removed += 1;
            self.freed_bytes += l.bytes;
        }
        self.lines.push(l);
    }
}

/// Under this much free disk, Needs you says the disk is almost full.
pub const LOW_DISK: u64 = 20_000_000_000;

#[cfg(test)]
thread_local!(pub(crate) static FAKE_FREE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) });

/// Bytes free on the disk that holds `path` (or its nearest existing parent).
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(test)]
    if let Some(f) = FAKE_FREE.with(|f| f.get()) {
        return Some(f);
    }
    let dir = path.ancestors().find(|p| p.exists())?;
    let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0).then(|| s.f_bavail as u64 * s.f_frsize as u64)
}

/// `GET /api/workspaces`: every agent's workspace size and whether it is busy (running or with a live process,
/// so Clean can't remove its items), and the disk's free space.
pub async fn sizes(axum::extract::State(state): axum::extract::State<AppState>) -> crate::api::ApiResult<serde_json::Value> {
    let agents = state.db.list_agents()?;
    let storage = state.cfg.storage_dir.clone();
    let running: Vec<String> = state.rt.lock().unwrap().running.keys().cloned().collect();
    let dirs: Vec<(String, String, PathBuf)> = agents.iter().map(|a| (a.id.clone(), a.name.clone(), crate::agents::workspace(&state, &a.id))).collect();
    let (rows, free) = tokio::task::spawn_blocking(move || {
        let live = crate::processes::with_live_processes(&storage);
        let rows: Vec<_> = dirs
            .into_iter()
            .map(|(id, name, d)| {
                let busy = live.contains(&id) || running.contains(&id);
                serde_json::json!({ "id": id, "name": name, "bytes": size(&d), "busy": busy })
            })
            .collect();
        (rows, free_bytes(&storage))
    })
    .await
    .unwrap_or_default();
    Ok(axum::Json(serde_json::json!({ "agents": rows, "free_bytes": free, "low_disk_bytes": LOW_DISK })))
}

/// Each agent's projects by id, main first.
pub(crate) fn projects(state: &AppState) -> HashMap<String, Vec<String>> {
    let agents = state.db.list_agents().unwrap_or_default();
    agents.into_iter().map(|a| (a.id, if a.projects.is_empty() { vec![a.project].into_iter().filter(|p| !p.is_empty()).collect() } else { a.projects })).collect()
}

/// Whether an agent is running or has a live process. Asked again before each removal.
fn busy_check(state: &AppState) -> impl Fn(&str) -> bool + '_ {
    let live = crate::processes::with_live_processes(&state.cfg.storage_dir);
    move |id: &str| live.contains(id) || state.rt.lock().unwrap().running.contains_key(id)
}

#[derive(Deserialize)]
pub struct PreviewOpts {
    agent: Option<String>,
}

/// `GET /api/workspaces/clean/preview[?agent=<id>]`: what Clean on that agent, or Clean all, lists.
pub async fn preview(axum::extract::State(state): axum::extract::State<AppState>, axum::extract::Query(q): axum::extract::Query<PreviewOpts>) -> axum::Json<Listing> {
    let items = tokio::task::spawn_blocking(move || {
        let projects = projects(&state);
        let of = |id: &str| projects.get(id).cloned().unwrap_or_default();
        listing(&state.cfg.storage_dir, q.agent.as_deref(), &of, &busy_check(&state))
    })
    .await
    .unwrap_or_default();
    axum::Json(Listing { items })
}

#[derive(Deserialize)]
pub struct FoldersOpts {
    agent: String,
    rel: String,
}

/// `GET /api/workspaces/clean/folders?agent=<id>&rel=<loft/backend>`: the folders inside a main clone's `rel`.
pub async fn folders(axum::extract::State(state): axum::extract::State<AppState>, axum::extract::Query(q): axum::extract::Query<FoldersOpts>) -> crate::api::ApiResult<Listing> {
    let r = tokio::task::spawn_blocking(move || {
        let projects = projects(&state).remove(&q.agent).unwrap_or_default();
        let busy = busy_check(&state)(&q.agent);
        inside(&state.cfg.storage_dir, &q.agent, &q.rel, &projects, busy)
    })
    .await
    .unwrap_or_else(|_| Err("can't list it".into()));
    r.map(|items| axum::Json(Listing { items })).map_err(|e| crate::api::ApiError(axum::http::StatusCode::BAD_REQUEST, e))
}

#[derive(Deserialize)]
pub struct CleanBody {
    agent: Option<String>,
    /// The listed items the user ticked.
    paths: Vec<PathBuf>,
}

/// `POST /api/workspaces/clean`: removes the items the user ticked in Clean (`agent`) or Clean all, each checked
/// again against the workspace as it is now, and answers the log.
pub async fn clean(axum::extract::State(state): axum::extract::State<AppState>, axum::Json(b): axum::Json<CleanBody>) -> axum::Json<Log> {
    let _one = RUNNING.lock().await;
    let names: HashMap<String, String> = state.db.list_agents().unwrap_or_default().into_iter().map(|a| (a.id, a.name)).collect();
    let (what, running, done) = match &b.agent {
        Some(id) => {
            let n = names.get(id).cloned().unwrap_or_else(|| format!("deleted agent {}", id.chars().take(8).collect::<String>()));
            ("Clean", format!("Cleaning {n}…"), format!("Cleaned {n}"))
        }
        None => ("Clean all", "Cleaning all agents…".into(), "Cleaned all agents".into()),
    };
    let st = state.clone();
    let log = tokio::task::spawn_blocking(move || {
        let storage = &st.cfg.storage_dir;
        let root = storage.canonicalize().unwrap_or_default();
        let projects = projects(&st);
        let mains = |id: &str| main_clones(&root.join(id), projects.get(id).map(Vec::as_slice).unwrap_or_default());
        let busy = busy_check(&st);
        let mut log = Log { at: Some(Local::now()), what: what.into(), title: running, running: true, ..Default::default() };
        show(&st, &log);
        for path in b.paths {
            match check(&root, &path, &mains) {
                Err(reason) => {
                    let (agent, rel) = split(&root, &path);
                    log.skipped.push(skip(&agent, &rel, reason));
                }
                Ok((id, _)) if b.agent.as_ref().is_some_and(|a| *a != id) => log.skipped.push(skip(&id, "", "another agent's")),
                Ok((id, rel)) => {
                    let bytes = size(&path);
                    let command = rm_command(&path);
                    let error = if busy(&id) { Some("agent started working".into()) } else { remove_item(&root, &path).err() };
                    log.push(Line { agent: id, path, rel, bytes: if error.is_none() { bytes } else { 0 }, error, command, done: String::new() });
                }
            }
            show(&st, &log);
        }
        log.running = false;
        log.title = done;
        show(&st, &log);
        save(&st, &log);
        log
    })
    .await
    .unwrap_or_default();
    *SIZES.lock().unwrap() = None;
    axum::Json(log)
}

/// The log Settings shows, live.
fn show(st: &AppState, log: &Log) {
    st.rt.lock().unwrap().clean = log.clone();
}

fn save(st: &AppState, log: &Log) {
    if let Err(e) = serde_json::to_vec(log).map_err(std::io::Error::other).and_then(|b| std::fs::write(st.settings.dir().join(LOG_FILE), b)) {
        tracing::warn!("sweep: can't save the log: {e}");
    }
    // Where it was kept before, which agents can write.
    let _ = std::fs::remove_file(st.cfg.storage_dir.join(".clean-log.json"));
}

/// `GET /api/workspaces/log`: the running or last run's log, kept across restarts.
pub async fn log(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<Log> {
    let l = state.rt.lock().unwrap().clean.clone();
    if l.at.is_some() {
        return axum::Json(l);
    }
    let file = state.settings.dir().join(LOG_FILE);
    let saved = tokio::fs::read(file).await.ok().and_then(|b| serde_json::from_slice(&b).ok());
    axum::Json(saved.unwrap_or_default())
}

/// One sweep or clean at a time.
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Removes the build output untouched for `idle` from every workspace but those of running agents and agents
/// with a live process, or just from `only`'s. With `what`, the run's log is the one Settings shows, live and
/// saved; without, it is only returned.
pub async fn run(state: &AppState, what: Option<&'static str>, only: Option<&str>, idle: Duration) -> Log {
    let _one = RUNNING.lock().await;
    let (st, only) = (state.clone(), only.map(str::to_string));
    let log = tokio::task::spawn_blocking(move || {
        let storage = &st.cfg.storage_dir;
        // Asked again before each removal, so a run that starts mid-sweep keeps its build output.
        let busy = busy_check(&st);
        let p = plan(storage, only.as_deref(), &busy, SystemTime::now(), idle);
        let what = what.unwrap_or_default();
        let mut log = Log { at: Some(Local::now()), what: what.into(), title: format!("{what}…"), running: true, skipped: p.skipped, ..Default::default() };
        let show = |log: &Log| {
            if !what.is_empty() {
                show(&st, log);
            }
        };
        show(&log);
        let root = storage.canonicalize().unwrap_or_default();
        apply(&root, p.folders, &busy, &mut |l| {
            log.push(l);
            show(&log);
        });
        log.running = false;
        log.title = what.into();
        show(&log);
        if !what.is_empty() {
            save(&st, &log);
        }
        log
    })
    .await
    .unwrap_or_default();
    tracing::info!("sweep freed {} bytes in {} folders, {} failed", log.freed_bytes, log.removed, log.failed);
    *SIZES.lock().unwrap() = None;
    log
}

/// Saves `lines` as a run of its own (`what`, like "Simulator cleanup"), the log Settings shows, after any sweep or
/// clean in progress.
pub async fn record(state: &AppState, what: &'static str, lines: Vec<Line>) {
    let _one = RUNNING.lock().await;
    let mut log = Log { at: Some(Local::now()), what: what.into(), title: what.into(), ..Default::default() };
    for l in lines {
        log.push(l);
    }
    show(state, &log);
    save(state, &log);
}

/// The day's sweep, the first right after Loft starts.
pub async fn scheduler(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        run(&state, Some("Daily sweep"), None, IDLE).await;
    }
}

/// The workspaces' top-level items in `storage` (or only `only`'s), for Clean: biggest first within each.
pub fn listing(storage: &Path, only: Option<&str>, projects: &dyn Fn(&str) -> Vec<String>, busy: &dyn Fn(&str) -> bool) -> Vec<Item> {
    let Ok(root) = storage.canonicalize() else { return vec![] };
    let mut out = vec![];
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let id = e.file_name().to_string_lossy().to_string();
        // `.agent-files` and the like are Loft's; a symlinked workspace leads outside; no agent's id has a
        // control character, and one would break the `# <agent>` line of Copy commands.
        if id.starts_with('.') || id.contains(char::is_control) || only.is_some_and(|o| o != id) || !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let ws = root.join(&id);
        let projects = projects(&id);
        let mains = main_clones(&ws, &projects);
        let working = busy(&id);
        let mut items: Vec<Item> = std::fs::read_dir(&ws)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let mut i = item(&id, name.clone(), e.path());
                let dir = i.kind == "folder";
                if let Some(project) = mains.get(&name) {
                    (i.label, i.note, i.warning, i.ticked, i.opens) = (format!("main clone of {project}"), KEPT.into(), String::new(), false, true);
                } else if projects.is_empty() && matches!(i.kind, "clone" | "worktree") {
                    // Any of them may be its working copy.
                    (i.label, i.ticked) = ("agent has no project: can't tell its main clone".into(), false);
                } else if dir && name == "notes" {
                    (i.label, i.ticked) = ("reports and notes".into(), false);
                } else if dir && name == "uploads" {
                    (i.label, i.ticked) = ("chat attachments".into(), false);
                }
                if working {
                    (i.blocked, i.ticked) = ("Working".into(), false);
                }
                i
            })
            .collect();
        items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        out.extend(items);
    }
    out
}

/// The folders in `rel` inside one of agent `id`'s main clones, all unticked, biggest first.
fn inside(storage: &Path, id: &str, rel: &str, projects: &[String], busy: bool) -> Result<Vec<Item>, String> {
    let root = storage.canonicalize().map_err(|e| e.to_string())?;
    let dir = root.join(id).join(rel);
    let mains = main_clones(&root.join(id), projects);
    if !mains.contains_key(rel.split('/').next().unwrap_or_default()) {
        return Err("not inside a main clone".into());
    }
    match check(&root, &dir, &|_| mains.clone()) {
        Ok(_) | Err("main clone") => {}
        Err(e) => return Err(e.into()),
    }
    if !dir.symlink_metadata().is_ok_and(|m| m.is_dir()) {
        return Err("not a folder".into());
    }
    let mut items: Vec<Item> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()))
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let mut i = item(id, format!("{rel}/{name}"), e.path());
            (i.ticked, i.warning, i.opens) = (false, String::new(), i.blocked.is_empty());
            if name == ".git" {
                (i.blocked, i.opens, i.command) = ("git history: never deleted".into(), false, String::new());
            } else if busy {
                i.blocked = "Working".into();
            }
            i
        })
        .collect();
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    Ok(items)
}

/// What Clean shows for `path`, ticked unless it is a symlink.
fn item(agent: &str, rel: String, path: PathBuf) -> Item {
    let meta = path.symlink_metadata().ok();
    let link = meta.as_ref().is_some_and(|m| m.is_symlink());
    let kind = match &meta {
        _ if link => "symlink",
        Some(m) if m.is_dir() => match path.join(".git").symlink_metadata() {
            Ok(g) if g.is_dir() => "clone",
            Ok(_) => "worktree",
            Err(_) => "folder",
        },
        _ => "file",
    };
    let (bytes, changed) = if link { (0, meta.and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH)) } else { measure(&path) };
    let warning = if matches!(kind, "clone" | "worktree") { "may contain unpushed work" } else { "" };
    Item {
        agent: agent.into(),
        name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        command: rm_command(&path),
        path,
        rel,
        kind,
        bytes,
        changed: changed.into(),
        label: String::new(),
        note: String::new(),
        warning: warning.into(),
        ticked: !link,
        blocked: if link { "symlink".into() } else { String::new() },
        opens: false,
    }
}

/// The main clone of each of `projects` in `ws`, as folder name → project: a clone whose `.git/config` (read as a
/// file, no git run) names a remote of that project. Of several, the one named after the project.
pub(crate) fn main_clones(ws: &Path, projects: &[String]) -> HashMap<String, String> {
    let last = |p: &str| p.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
    let mut found: Vec<(String, String)> = vec![];
    for (name, dir) in subdirs(ws) {
        let Ok(config) = std::fs::read_to_string(dir.join(".git/config")) else { continue };
        let urls = config.lines().filter_map(|l| l.trim().strip_prefix("url")?.trim().strip_prefix('=').map(str::trim));
        let repos: Vec<String> = urls.map(|u| last(u.trim_end_matches('/').trim_end_matches(".git")).rsplit(':').next().unwrap_or_default().to_string()).collect();
        if let Some(p) = projects.iter().find(|p| repos.contains(&last(p))) {
            found.push((name, p.clone()));
        }
    }
    let named = |p: &str| found.iter().any(|(d, q)| q == p && *d == last(p));
    found.iter().filter(|(d, p)| !named(p) || *d == last(p)).cloned().collect()
}

/// Whether Clean may remove `path`, as the workspace is now: a top-level item of an agent's workspace that isn't
/// a main clone, or a folder inside a main clone, never `.git` and never through a symlink. Its agent and `rel`.
fn check(root: &Path, path: &Path, mains: &dyn Fn(&str) -> HashMap<String, String>) -> Result<(String, String), &'static str> {
    let rest = path.strip_prefix(root).map_err(|_| "outside agent-local-storage")?;
    let parts = rest
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str().ok_or("not a plain path"),
            _ => Err("not a plain path"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.len() < 2 || parts[0].starts_with('.') || parts[0].contains(char::is_control) {
        return Err("not in a workspace");
    }
    let mut p = root.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        p.push(part);
        let m = p.symlink_metadata().map_err(|_| "no longer there")?;
        if m.is_symlink() {
            return Err("symlink");
        }
        if i + 1 < parts.len() && !m.is_dir() {
            return Err("no longer there");
        }
    }
    // APFS finds a name whatever its case or Unicode form (`.GIT` is `.git`, `LOFT` is `loft`), so the names
    // below are compared only once the path is spelled as it is on disk.
    if p.canonicalize().ok().as_deref() != Some(p.as_path()) {
        return Err("not the name on disk");
    }
    if parts.contains(&".git") {
        return Err("git history: never deleted");
    }
    let main = mains(parts[0]).contains_key(parts[1]);
    match (parts.len(), main) {
        (2, true) => Err("main clone"),
        (2, false) => Ok(()),
        (_, true) if p.is_dir() => Ok(()),
        (_, true) => Err("not a folder"),
        (_, false) => Err("not inside a main clone"),
    }?;
    Ok((parts[0].into(), parts[1..].join("/")))
}

/// The agent and `rel` a path names, for the log, whatever it is.
fn split(root: &Path, path: &Path) -> (String, String) {
    let rest = path.strip_prefix(root).unwrap_or(path).to_string_lossy().to_string();
    let (a, r) = rest.split_once('/').unwrap_or((&rest, ""));
    (a.into(), r.into())
}

/// Removes a checked item: a folder and all in it (symlinks inside, not what they point at), or a file.
fn remove_item(root: &Path, path: &Path) -> Result<(), String> {
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() || m.is_file() => remove_under(root, path),
        _ => Err("no longer there".into()),
    }
}

/// What a sweep of the workspaces in `storage` (or only `only`'s) would remove as of `now`: build output whose
/// newest file is `idle` old, in workspaces of agents `busy` doesn't name.
pub fn plan(storage: &Path, only: Option<&str>, busy: &dyn Fn(&str) -> bool, now: SystemTime, idle: Duration) -> Plan {
    let mut p = Plan::default();
    let Ok(root) = storage.canonicalize() else { return p };
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let id = e.file_name().to_string_lossy().to_string();
        // `.agent-files` and the like are Loft's; no agent's id has a control character.
        if id.starts_with('.') || id.contains(char::is_control) || only.is_some_and(|o| o != id) {
            continue;
        }
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            // It leads outside.
            p.skipped.push(skip(&id, "", "symlink"));
        } else if t.is_dir() && busy(&id) {
            p.skipped.push(skip(&id, "", "agent is working"));
        } else if t.is_dir() {
            workspace(&root, &id, &e.path(), now, idle, &mut p);
        }
    }
    p
}

fn skip(agent: &str, rel: &str, reason: &str) -> Skip {
    Skip { agent: one_line(agent), rel: one_line(rel), reason: reason.into() }
}

/// `s` with its control characters (a newline in a folder name) written as escapes, so it stays on one line.
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' => "\\n".into(),
            '\r' => "\\r".into(),
            '\t' => "\\t".into(),
            c if c.is_control() && (c as u32) < 0x80 => format!("\\x{:02x}", c as u32),
            c if c.is_control() => format!("\\u{:04x}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

fn workspace(root: &Path, id: &str, ws: &Path, now: SystemTime, idle: Duration, p: &mut Plan) {
    let rel = |path: &Path| path.strip_prefix(ws).unwrap_or(path).to_string_lossy().to_string();
    let mut clones = vec![];
    find_clones(ws, DEPTH, &mut clones);
    let shared = ws.join(".cargo-build");
    let shared_dir = shared.symlink_metadata().is_ok_and(|m| m.is_dir());
    let mut builds = vec![];
    for clone in clones {
        let mut links = vec![];
        walk(&clone, &mut builds, &mut links);
        p.skipped.extend(links.iter().map(|l| skip(id, &rel(l), "symlink")));
    }
    if shared_dir {
        builds.push(shared.clone());
    }
    for b in builds {
        let (_, changed) = measure(&b);
        if now.duration_since(changed).unwrap_or_default() >= idle {
            folder(root, id, &rel(&b), b, changed, p);
            continue;
        }
        p.skipped.push(skip(id, &rel(&b), "younger than 3 days"));
        // An agent that works often keeps it young, so its incremental caches are capped instead.
        if shared_dir && b == shared {
            let caches: Vec<_> = subdirs(&b).map(|(_, profile)| profile.join("incremental")).filter(|d| d.symlink_metadata().is_ok_and(|m| m.is_dir())).collect();
            if caches.iter().map(|d| size(d)).sum::<u64>() > incremental_cap() {
                for d in caches {
                    let (_, changed) = measure(&d);
                    folder(root, id, &rel(&d), d, changed, p);
                }
            }
        }
        // With the workspace's `.cargo-build/` holding the build, what a Cargo `target/` still has besides
        // binaries (`<profile>/deps`, `incremental` and `build`) is left over from before, and goes whatever its age.
        if shared_dir && b.file_name().is_some_and(|n| n == "target") {
            for (_, profile) in subdirs(&b) {
                for (name, dir) in subdirs(&profile) {
                    if ["deps", "incremental", "build"].contains(&name.as_str()) {
                        let (_, changed) = measure(&dir);
                        folder(root, id, &rel(&dir), dir, changed, p);
                    }
                }
            }
        }
    }
}

/// Adds `path` to the plan when it is a real directory inside `root`.
fn folder(root: &Path, id: &str, rel: &str, path: PathBuf, changed: SystemTime, p: &mut Plan) {
    if !path.canonicalize().is_ok_and(|c| c.starts_with(root)) {
        p.skipped.push(skip(id, rel, "outside agent-local-storage"));
        return;
    }
    let bytes = size(&path);
    let command = rm_command(&path);
    p.folders.push(Folder { agent: id.into(), path, rel: rel.into(), bytes, changed: changed.into(), command });
}

/// `rm -rf <path>`, quoted for the shell when it needs to be, and on one line: control characters are written
/// as escapes inside `$'…'`.
fn rm_command(path: &Path) -> String {
    let p = path.to_string_lossy();
    if p.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@".contains(c)) {
        format!("rm -rf {p}")
    } else if p.chars().any(char::is_control) {
        format!("rm -rf $'{}'", one_line(&p.replace('\\', "\\\\").replace('\'', "\\'")))
    } else {
        format!("rm -rf '{}'", p.replace('\'', "'\\''"))
    }
}

/// Removes the folders, reporting each as it goes; an agent that started working keeps its.
fn apply(root: &Path, folders: Vec<Folder>, busy: &dyn Fn(&str) -> bool, line: &mut dyn FnMut(Line)) {
    for f in folders {
        let error = if busy(&f.agent) { Some("agent started working".into()) } else { remove(root, &f.path).err() };
        let bytes = if error.is_none() { f.bytes } else { 0 };
        line(Line { agent: f.agent, path: f.path, rel: f.rel, bytes, error, command: f.command, done: String::new() });
    }
}

/// Real (not symlinked) directories, never a dotfile one.
fn subdirs(dir: &Path) -> impl Iterator<Item = (String, PathBuf)> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        (e.file_type().ok()?.is_dir() && !name.starts_with('.')).then(|| (name, e.path()))
    })
}

/// The git clones in `dir`, down to `depth` levels, nested ones included.
fn find_clones(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if dir.join(".git").symlink_metadata().is_ok() {
        out.push(dir.to_path_buf());
    }
    if depth == 0 {
        return;
    }
    for (_, path) in subdirs(dir) {
        if !is_build(&path) {
            find_clones(&path, depth - 1, out);
        }
    }
}

/// One kind of build output: a `folder` (a name, or a path like `ios/build`) and the `markers` that must sit
/// beside it, in the folder's parent (`*.csproj` matches any file with that ending). A folder is build output
/// only when one marker is there, so a `build/` or `bin/` that is somebody's real files stays. No markers means
/// the name never holds anything but rebuildable files.
struct Rule {
    folder: &'static str,
    markers: &'static [&'static str],
}

const fn rule(folder: &'static str, markers: &'static [&'static str]) -> Rule {
    Rule { folder, markers }
}

const GRADLE: &[&str] = &["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"];
const DOTNET: &[&str] = &["*.csproj", "*.fsproj", "*.vbproj"];

/// Everything the sweep and Clean's preview count as build output. To teach them a new stack, add a row here and
/// a case to `every_rule_is_swept_beside_its_marker_and_kept_without_it`; see docs/sweep-rules.md.
const RULES: &[Rule] = &[
    // Rust
    rule("target", &["Cargo.toml"]),
    rule(".cargo-build", &[]),
    // Node, Next.js, Nuxt, Expo
    rule("node_modules", &[]),
    rule(".next", &["package.json"]),
    rule(".nuxt", &["package.json"]),
    rule(".expo", &["app.json", "package.json"]),
    // .NET
    rule("bin", DOTNET),
    rule("obj", DOTNET),
    // Java (Maven)
    rule("target", &["pom.xml"]),
    // Gradle: Java, Kotlin, Android (also React Native's and Flutter's android/)
    rule("build", GRADLE),
    rule(".gradle", GRADLE),
    rule(".cxx", GRADLE),
    // iOS: CocoaPods, Carthage, Xcode, Swift
    rule("Pods", &["Podfile"]),
    rule("Carthage/Build", &["Cartfile"]),
    rule("DerivedData", &[]),
    rule("ios/build", &[]),
    rule(".build", &[]),
    // Flutter
    rule("build", &["pubspec.yaml"]),
    rule(".dart_tool", &["pubspec.yaml"]),
    // Python
    rule(".venv", &[]),
    rule("__pycache__", &[]),
    rule(".pytest_cache", &[]),
    rule(".mypy_cache", &[]),
    rule(".ruff_cache", &[]),
];

impl Rule {
    /// Whether `path` is the folder this rule names, with a marker beside it.
    fn matches(&self, path: &Path) -> bool {
        let Some(base) = path.ancestors().nth(self.folder.split('/').count()) else { return false };
        path.ends_with(self.folder) && (self.markers.is_empty() || self.markers.iter().any(|m| has_marker(base, m)))
    }
}

fn has_marker(dir: &Path, marker: &str) -> bool {
    match marker.strip_prefix('*') {
        Some(ending) => std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| e.file_name().to_string_lossy().ends_with(ending)),
        None => dir.join(marker).exists(),
    }
}

/// Whether `path` is build output.
fn is_build(path: &Path) -> bool {
    RULES.iter().any(|r| r.matches(path))
}

/// Collects the clone's build folders and the symlinked ones it leaves, stopping at nested clones.
fn walk(dir: &Path, builds: &mut Vec<PathBuf>, links: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // Not followed: a symlink's own metadata.
        let Ok(meta) = e.metadata() else { continue };
        let path = e.path();
        let build = is_build(&path);
        if meta.is_symlink() && build {
            links.push(path.clone());
        }
        if !meta.is_dir() || name == ".git" {
            continue;
        }
        if build {
            builds.push(path);
        } else if path.join(".git").symlink_metadata().is_err() {
            walk(&path, builds, links);
        }
    }
}

/// Removes `path` when it is still a real directory inside `root`.
fn remove(root: &Path, path: &Path) -> Result<(), String> {
    if !path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
        return Err("no longer a folder".into());
    }
    if !path.canonicalize().is_ok_and(|p| p.starts_with(root)) {
        return Err("outside agent-local-storage".into());
    }
    remove_under(root, path)
}

/// Removes `path`, a file or a folder and all in it, through folder handles opened from `root` down, none of
/// them through a symlink: a folder on the way swapped for a symlink after the checks fails the removal instead
/// of leading it outside `root`. Symlinks inside go, not what they point at.
fn remove_under(root: &Path, path: &Path) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let rest = path.strip_prefix(root).map_err(|_| "outside agent-local-storage")?;
    let names = rest
        .components()
        .map(|c| match c {
            Component::Normal(s) => CString::new(s.as_bytes()).map_err(|_| "not a plain path"),
            _ => Err("not a plain path"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (last, parents) = names.split_last().ok_or("not in a workspace")?;
    let root = CString::new(root.as_os_str().as_bytes()).map_err(|_| "not a plain path")?;
    let mut dir = open_dir(libc::AT_FDCWD, &root)?;
    for n in parents {
        dir = open_dir(dir.as_raw_fd(), n)?;
    }
    remove_at(&dir, last)
}

/// The folder `name` in `dir`, opened without following a symlink.
fn open_dir(dir: libc::c_int, name: &std::ffi::CStr) -> Result<OwnedFd, String> {
    // SAFETY: a valid C string; the fd is owned from here on.
    let fd = unsafe { libc::openat(dir, name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Removes `name` in `dir`: a folder with all in it, or anything else as it is.
fn remove_at(dir: &OwnedFd, name: &std::ffi::CStr) -> Result<(), String> {
    let last = || Err(std::io::Error::last_os_error().to_string());
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid fd, C string and out pointer.
    if unsafe { libc::fstatat(dir.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return last();
    }
    if st.st_mode & libc::S_IFMT == libc::S_IFDIR {
        let sub = open_dir(dir.as_raw_fd(), name)?;
        for n in entries(&sub)? {
            remove_at(&sub, &n)?;
        }
    }
    let flags = if st.st_mode & libc::S_IFMT == libc::S_IFDIR { libc::AT_REMOVEDIR } else { 0 };
    // SAFETY: valid fd and C string.
    if unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), flags) } != 0 {
        return last();
    }
    Ok(())
}

/// The names in an open folder, but `.` and `..`.
fn entries(dir: &OwnedFd) -> Result<Vec<std::ffi::CString>, String> {
    // SAFETY: `fdopendir` takes its own copy of the fd, closed by `closedir`.
    let d = unsafe { libc::fdopendir(libc::dup(dir.as_raw_fd())) };
    if d.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut out = Vec::new();
    loop {
        // SAFETY: `d` is open until `closedir` below.
        let e = unsafe { libc::readdir(d) };
        if e.is_null() {
            break;
        }
        let n = unsafe { std::ffi::CStr::from_ptr((*e).d_name.as_ptr()) };
        if n.to_bytes() != b"." && n.to_bytes() != b".." {
            out.push(n.to_owned());
        }
    }
    unsafe { libc::closedir(d) };
    Ok(out)
}

/// A measure of a path: when it was taken, how big, and the newest file's change.
type Measured = (Instant, u64, SystemTime);
/// Measured sizes, by path.
static SIZES: Mutex<Option<HashMap<PathBuf, Measured>>> = Mutex::new(None);

/// Disk space `path` takes, as `du` measures it (the same measure as a sweep's freed bytes), measured at most
/// once a minute; 0 when it doesn't exist. Blocking.
pub fn size(path: &Path) -> u64 {
    measure(path).0
}

/// Disk space `path` takes and its newest file's change (its own, with no files in it), measured at most once
/// a minute. Blocking.
fn measure(path: &Path) -> (u64, SystemTime) {
    if let Some(&(at, n, t)) = SIZES.lock().unwrap().get_or_insert_default().get(path)
        && at.elapsed() < SIZE_FOR
    {
        return (n, t);
    }
    let (n, newest) = du(path);
    let t = newest.or_else(|| path.symlink_metadata().ok()?.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH);
    SIZES.lock().unwrap().get_or_insert_default().insert(path.to_path_buf(), (Instant::now(), n, t));
    (n, t)
}

/// Disk space `path` takes and its newest file's change, symlinks not followed.
fn du(path: &Path) -> (u64, Option<SystemTime>) {
    let Ok(meta) = path.symlink_metadata() else { return (0, None) };
    let n = meta.blocks() * 512;
    if !meta.is_dir() {
        return (n, meta.modified().ok());
    }
    std::fs::read_dir(path).into_iter().flatten().flatten().fold((n, None), |(n, newest), e| {
        let (b, t) = du(&e.path());
        (n + b, newest.max(t))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp() -> PathBuf {
        let d = crate::testdir::for_this_test("sweep");
        fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn file(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
    }

    /// A clone with Rust, web and iOS build output.
    fn clone(dir: &Path) {
        for f in [".git/HEAD", ".git/index", "Cargo.toml", "src/main.rs", "ios/App/App.swift", "frontend/package.json"] {
            file(&dir.join(f));
        }
        for f in ["target/debug/app", "frontend/node_modules/vite/index.js", "ios/build/App.app/App", "ios/DerivedData/x", "Pkg/.build/x"] {
            file(&dir.join(f));
        }
    }

    /// A clone of `remote`, as `.git/config` names it.
    fn clone_of(dir: &Path, remote: &str) {
        clone(dir);
        fs::write(dir.join(".git/config"), format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {remote}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n")).unwrap();
    }

    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(4 * 24 * 60 * 60)
    }

    /// Plans and removes everything, as the daily sweep does.
    fn sweep(storage: &Path, only: Option<&str>, busy: &dyn Fn(&str) -> bool, now: SystemTime, idle: Duration) -> Log {
        *SIZES.lock().unwrap() = None;
        let p = plan(storage, only, busy, now, idle);
        let mut log = Log { skipped: p.skipped, ..Default::default() };
        apply(&storage.canonicalize().unwrap(), p.folders, busy, &mut |l| log.push(l));
        log
    }

    impl Log {
        fn removed_paths(&self) -> Vec<PathBuf> {
            self.lines.iter().filter(|l| l.error.is_none()).map(|l| l.path.clone()).collect()
        }
    }

    /// One case per rule and marker: (folder, marker beside it, `None` for a rule with no marker). To teach the
    /// sweep a new stack, add its row to `RULES` and a case here (docs/sweep-rules.md).
    const CASES: &[(&str, Option<&str>)] = &[
        ("target", Some("Cargo.toml")),
        (".cargo-build", None),
        ("node_modules", None),
        (".next", Some("package.json")),
        (".nuxt", Some("package.json")),
        (".expo", Some("app.json")),
        (".expo", Some("package.json")),
        ("bin", Some("App.csproj")),
        ("bin", Some("App.fsproj")),
        ("bin", Some("App.vbproj")),
        ("obj", Some("App.csproj")),
        ("target", Some("pom.xml")),
        ("build", Some("build.gradle")),
        ("build", Some("build.gradle.kts")),
        ("build", Some("settings.gradle")),
        ("build", Some("settings.gradle.kts")),
        (".gradle", Some("build.gradle")),
        (".cxx", Some("build.gradle")),
        ("Pods", Some("Podfile")),
        ("Carthage/Build", Some("Cartfile")),
        ("DerivedData", None),
        ("ios/build", None),
        (".build", None),
        ("build", Some("pubspec.yaml")),
        (".dart_tool", Some("pubspec.yaml")),
        (".venv", None),
        ("__pycache__", None),
        (".pytest_cache", None),
        (".mypy_cache", None),
        (".ruff_cache", None),
    ];

    /// A clone holding `folder` (with a file in it) and, beside it, `marker`; whether a sweep removes the folder.
    fn swept(folder: &str, marker: Option<&str>) -> bool {
        let storage = temp();
        let clone = storage.join("agent/app");
        file(&clone.join(".git/HEAD"));
        file(&clone.join("src/Program.cs"));
        file(&clone.join(folder).join("out.bin"));
        if let Some(m) = marker {
            let base = clone.join(folder).ancestors().nth(folder.split('/').count()).unwrap().to_path_buf();
            file(&base.join(m));
        }
        let log = sweep(&storage, None, &|_| false, later(), IDLE);
        assert!(clone.join("src/Program.cs").exists(), "source files stay");
        log.removed_paths().contains(&clone.join(folder))
    }

    #[test]
    fn every_rule_is_swept_beside_its_marker_and_kept_without_it() {
        for rule in RULES {
            assert!(CASES.iter().any(|(f, _)| *f == rule.folder), "{} has no case in CASES", rule.folder);
        }
        for (folder, marker) in CASES {
            assert!(swept(folder, *marker), "{folder} beside {marker:?} should be swept");
            if marker.is_some() {
                assert!(!swept(folder, None), "{folder} with no marker should stay");
            }
        }
    }

    #[test]
    fn a_bare_build_dist_or_bin_with_no_marker_is_never_swept() {
        for folder in ["build", "dist", "bin", "obj", "out", "target"] {
            assert!(!swept(folder, None), "{folder}");
        }
        // The wrong stack's marker doesn't count.
        assert!(!swept("bin", Some("Cargo.toml")));
        assert!(!swept("build", Some("package.json")));
    }

    #[test]
    fn a_dotnet_projects_stale_bin_and_obj_are_planned_and_go_with_the_sweep() {
        let storage = temp();
        let ws = storage.join("agent");
        for f in ["Api/Api.csproj", "Api/Program.cs", "Api/bin/Debug/Api.dll", "Api/obj/project.assets.json"] {
            file(&ws.join("app").join(f));
        }
        file(&ws.join("app/.git/HEAD"));
        let planned = plan(&storage, None, &|_| false, later(), IDLE);
        let rels: Vec<&str> = planned.folders.iter().map(|f| f.rel.as_str()).collect();
        assert!(rels.contains(&"app/Api/bin") && rels.contains(&"app/Api/obj"), "{rels:?}");
        let log = sweep(&storage, None, &|_| false, later(), IDLE);
        assert!(!ws.join("app/Api/bin").exists() && !ws.join("app/Api/obj").exists());
        assert!(ws.join("app/Api/Program.cs").exists() && ws.join("app/Api/Api.csproj").exists());
        assert!(log.removed_paths().len() >= 2);
    }

    async fn preview_of(st: &AppState, agent: Option<&str>) -> Vec<Item> {
        let q = agent.map(|a| format!("/?agent={a}")).unwrap_or("/".into());
        let axum::Json(l) = preview(axum::extract::State(st.clone()), axum::extract::Query::try_from_uri(&q.parse().unwrap()).unwrap()).await;
        l.items
    }

    async fn folders_of(st: &AppState, agent: &str, rel: &str) -> Result<Vec<Item>, axum::http::StatusCode> {
        let q: axum::http::Uri = format!("/?agent={agent}&rel={rel}").parse().unwrap();
        folders(axum::extract::State(st.clone()), axum::extract::Query::try_from_uri(&q).unwrap()).await.map(|j| j.0.items).map_err(|e| e.0)
    }

    async fn clean_paths(st: &AppState, agent: Option<&str>, paths: Vec<PathBuf>) -> Log {
        let axum::Json(l) = clean(axum::extract::State(st.clone()), axum::Json(CleanBody { agent: agent.map(Into::into), paths })).await;
        l
    }

    /// An agent named `name` working in `loft`, its workspace with a main clone of loft, an older second clone of
    /// it, a worktree, notes, uploads, `.cargo-build/`, a log file and scratch.
    fn workspace_with_everything(st: &AppState, name: &str) -> (crate::db::Agent, PathBuf) {
        let a = crate::tasks::tests::agent(st, name);
        let a = st.db.update_agent(&a.id, |a| (a.project, a.projects) = ("loft".into(), vec!["loft".into()])).unwrap().unwrap();
        let ws = crate::agents::workspace(st, &a.id);
        clone_of(&ws.join("loft"), "/Users/me/dev/loft");
        clone_of(&ws.join("loft-old"), "git@github.com:me/loft.git");
        file(&ws.join("wt/src/main.rs"));
        fs::write(ws.join("wt/.git"), "gitdir: ../loft/.git/worktrees/wt\n").unwrap();
        file(&ws.join("notes/plan.md"));
        file(&ws.join("uploads/1.png"));
        file(&ws.join(".cargo-build/debug/deps/x.rlib"));
        fs::write(ws.join("dev.log"), vec![b'x'; 5000]).unwrap();
        file(&ws.join("scratch/out.json"));
        (a, ws.canonicalize().unwrap())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn clean_lists_a_workspace_s_top_level_items_with_kind_size_change_and_command() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        age_by(&ws.join("scratch"), Duration::from_secs(5 * 60 * 60));
        let items = preview_of(&st, Some(&a.id)).await;
        let by = |n: &str| items.iter().find(|i| i.name == n).unwrap_or_else(|| panic!("{n} not listed: {items:?}"));
        let mut names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
        names.sort();
        assert_eq!(names, [".cargo-build", "dev.log", "loft", "loft-old", "notes", "scratch", "uploads", "wt"]);
        // Kinds, and the main clone found from .git/config: of two clones of loft, the one named after it.
        let kinds: Vec<_> = ["loft", "loft-old", "wt", "notes", "dev.log"].iter().map(|n| by(n).kind).collect();
        assert_eq!(kinds, ["clone", "clone", "worktree", "folder", "file"]);
        let main = by("loft");
        assert_eq!((main.label.as_str(), main.ticked, main.opens, main.note.as_str()), ("main clone of loft", false, true, KEPT));
        assert!(main.warning.is_empty());
        assert_eq!((by("notes").label.as_str(), by("notes").ticked), ("reports and notes", false));
        assert_eq!((by("uploads").label.as_str(), by("uploads").ticked), ("chat attachments", false));
        // Everything else starts ticked; other clones and worktrees warn.
        for n in ["loft-old", "wt", ".cargo-build", "dev.log", "scratch"] {
            assert!(by(n).ticked && by(n).label.is_empty() && by(n).blocked.is_empty() && !by(n).opens, "{:?}", by(n));
        }
        assert_eq!(by("loft-old").warning, "may contain unpushed work");
        assert_eq!(by("wt").warning, "may contain unpushed work");
        // Size, last change (the newest file), path and command.
        let log = by("dev.log");
        assert!(log.bytes >= 5000 && log.path == ws.join("dev.log") && log.rel == "dev.log", "{log:?}");
        assert_eq!(log.command, format!("rm -rf {}", ws.join("dev.log").display()));
        let age = Local::now() - by("scratch").changed;
        assert!(age.num_minutes() > 4 * 60 && age.num_minutes() < 6 * 60, "{age}");
        // Biggest first.
        assert!(items.windows(2).all(|w| w[0].bytes >= w[1].bytes));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_with_no_project_starts_with_all_its_clones_unticked() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Dineout Security");
        let a = st.db.update_agent(&a.id, |a| (a.project, a.projects) = (String::new(), vec![])).unwrap().unwrap();
        let ws = crate::agents::workspace(&st, &a.id);
        clone_of(&ws.join("api"), "git@github.com:dineout/api.git");
        clone_of(&ws.join("api-old"), "git@github.com:dineout/api.git");
        file(&ws.join("wt1895/src/x.cs"));
        fs::write(ws.join("wt1895/.git"), "gitdir: ../api/.git/worktrees/wt1895\n").unwrap();
        file(&ws.join("scratch/out.json"));
        let ws = ws.canonicalize().unwrap();

        let items = preview_of(&st, Some(&a.id)).await;
        let by = |n: &str| items.iter().find(|i| i.name == n).unwrap();
        for n in ["api", "api-old", "wt1895"] {
            let i = by(n);
            assert_eq!((i.label.as_str(), i.ticked, i.blocked.as_str(), i.opens), ("agent has no project: can't tell its main clone", false, "", false), "{i:?}");
        }
        assert!(by("scratch").ticked && by("scratch").label.is_empty());
        // Still the user's to tick.
        let l = clean_paths(&st, Some(&a.id), vec![ws.join("api-old")]).await;
        assert_eq!(l.removed, 1, "{l:?}");
        assert!(!ws.join("api-old").exists() && ws.join("api/src/main.rs").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_ticked_items_go_and_kept_ones_stay() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        let items = preview_of(&st, Some(&a.id)).await;
        let ticked: Vec<_> = items.iter().filter(|i| i.ticked).map(|i| i.path.clone()).collect();
        let l = clean_paths(&st, Some(&a.id), ticked).await;
        assert_eq!((l.removed, l.failed, l.running), (5, 0, false), "{:?}", l);
        for gone in ["loft-old", "wt", ".cargo-build", "dev.log", "scratch"] {
            assert!(!ws.join(gone).exists(), "{gone} stays");
        }
        for kept in ["loft/src/main.rs", "loft/target/debug/app", "notes/plan.md", "uploads/1.png"] {
            assert!(ws.join(kept).exists(), "{kept} removed");
        }
        assert!(l.freed_bytes >= 5000);
        assert!(l.lines.iter().any(|x| x.rel == "dev.log" && x.command == format!("rm -rf {}", ws.join("dev.log").display())));
        // Unticked items can be ticked: notes goes when asked.
        let l = clean_paths(&st, Some(&a.id), vec![ws.join("notes")]).await;
        assert_eq!(l.removed, 1);
        assert!(!ws.join("notes").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_main_clone_opens_level_by_level_and_removing_backend_target_keeps_the_rest() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        file(&ws.join("loft/backend/target/debug/loft"));
        file(&ws.join("loft/backend/src/main.rs"));

        let top = folders_of(&st, &a.id, "loft").await.unwrap();
        let git = top.iter().find(|i| i.name == ".git").unwrap();
        assert_eq!((git.blocked.as_str(), git.ticked, git.opens, git.command.as_str()), ("git history: never deleted", false, false, ""));
        let backend = top.iter().find(|i| i.name == "backend").unwrap();
        assert_eq!((backend.rel.as_str(), backend.ticked, backend.opens), ("loft/backend", false, true));
        assert!(top.iter().all(|i| !i.ticked && i.kind != "file"), "{top:?}");
        let inner = folders_of(&st, &a.id, "loft/backend").await.unwrap();
        let target = inner.iter().find(|i| i.name == "target").unwrap();
        assert!(target.bytes > 0 && target.command == format!("rm -rf {}", ws.join("loft/backend/target").display()));

        // Only a main clone opens.
        for rel in ["notes", "loft-old", "loft/.git", "../x", "loft/../notes"] {
            assert_eq!(folders_of(&st, &a.id, rel).await.err(), Some(axum::http::StatusCode::BAD_REQUEST), "{rel} opened");
        }

        let l = clean_paths(&st, Some(&a.id), vec![target.path.clone(), ws.join("loft"), ws.join("loft/.git"), ws.join("loft/.git/HEAD")]).await;
        assert_eq!(l.removed, 1, "{l:?}");
        assert!(!ws.join("loft/backend/target").exists());
        for kept in ["loft/backend/src/main.rs", "loft/.git/HEAD", "loft/target/debug/app", "loft/src/main.rs"] {
            assert!(ws.join(kept).exists(), "{kept} removed");
        }
        let reasons: Vec<_> = l.skipped.iter().map(|s| (s.rel.as_str(), s.reason.as_str())).collect();
        assert_eq!(reasons, [("loft", "main clone"), ("loft/.git", "git history: never deleted"), ("loft/.git/HEAD", "git history: never deleted")]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn clean_refuses_paths_outside_the_workspace_through_symlinks_or_of_another_agent() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        let (b, other) = workspace_with_everything(&st, "Other");
        let outside = temp().join("keep");
        file(&outside.join("x"));
        std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();
        file(&ws.join("loft/real/target/x"));
        std::os::unix::fs::symlink(ws.join("loft/real"), ws.join("loft/linked")).unwrap();

        let items = preview_of(&st, Some(&a.id)).await;
        let link = items.iter().find(|i| i.name == "link").unwrap();
        assert_eq!((link.kind, link.blocked.as_str(), link.ticked), ("symlink", "symlink", false));

        let paths = vec![ws.join("link"), ws.join("link/x"), ws.join("loft/linked/target"), outside.clone(), ws.join("scratch/../notes"), other.join("scratch"), ws.clone()];
        let l = clean_paths(&st, Some(&a.id), paths).await;
        assert_eq!(l.removed, 0, "{l:?}");
        assert!(outside.join("x").exists() && ws.join("loft/real/target/x").exists() && ws.join("notes/plan.md").exists() && other.join("scratch").exists());
        let reasons: Vec<_> = l.skipped.iter().map(|s| s.reason.as_str()).collect();
        assert_eq!(reasons, ["symlink", "symlink", "symlink", "outside agent-local-storage", "not a plain path", "another agent's", "not in a workspace"]);
        // Clean all takes any agent's.
        let l = clean_paths(&st, None, vec![other.join("scratch")]).await;
        assert_eq!((l.removed, l.lines[0].agent.as_str()), (1, b.id.as_str()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_path_spelled_unlike_the_disk_is_refused_whatever_its_case_or_unicode_form() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        let nfc = "caf\u{e9}";
        file(&ws.join(nfc).join("x"));
        let root = ws.parent().unwrap().to_path_buf();
        let upper = root.join(a.id.to_uppercase());
        let paths = vec![ws.join("loft/.GIT"), ws.join("loft/.Git/objects"), ws.join("LOFT"), ws.join("Loft/frontend"), ws.join("cafe\u{301}"), upper.join("loft"), upper.join("scratch")];
        // Clean on the agent, and Clean all while it runs: the uppercase id names it too on APFS.
        let l = clean_paths(&st, Some(&a.id), paths.clone()).await;
        assert_eq!(l.removed, 0, "{l:?}");
        st.rt.lock().unwrap().running.insert(a.id.clone(), Default::default());
        let l = clean_paths(&st, None, paths).await;
        assert_eq!(l.removed, 0, "{l:?}");
        st.rt.lock().unwrap().running.remove(&a.id);
        for kept in ["loft/.git/HEAD", "loft/src/main.rs", "loft/frontend/node_modules/vite/index.js", "scratch/out.json", "loft-old/src/main.rs"] {
            assert!(ws.join(kept).exists(), "{kept} removed");
        }
        assert!(ws.join(nfc).join("x").exists());
        for rel in ["loft/.GIT", "LOFT", "Loft/frontend"] {
            assert_eq!(folders_of(&st, &a.id, rel).await.err(), Some(axum::http::StatusCode::BAD_REQUEST), "{rel} opened");
        }
        assert_eq!(folders_of(&st, &a.id.to_uppercase(), "loft").await.err(), Some(axum::http::StatusCode::BAD_REQUEST));
        // The on-disk spelling still goes.
        let l = clean_paths(&st, Some(&a.id), vec![ws.join(nfc)]).await;
        assert_eq!(l.removed, 1, "{l:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_working_agent_s_items_cant_be_ticked_and_are_checked_again_at_removal() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        let idle = preview_of(&st, Some(&a.id)).await;
        st.rt.lock().unwrap().running.insert(a.id.clone(), Default::default());
        let items = preview_of(&st, None).await;
        assert!(items.iter().filter(|i| i.agent == a.id).all(|i| i.blocked == "Working" && !i.ticked), "{items:?}");
        assert!(folders_of(&st, &a.id, "loft").await.unwrap().iter().all(|i| !i.blocked.is_empty()));
        // A clean confirmed from before it started leaves its items.
        let l = clean_paths(&st, Some(&a.id), idle.iter().filter(|i| i.ticked).map(|i| i.path.clone()).collect()).await;
        assert_eq!((l.removed, l.failed), (0, 5));
        assert!(l.lines.iter().all(|x| x.error.as_deref() == Some("agent started working") && x.bytes == 0));
        assert!(ws.join("scratch").exists() && ws.join("wt").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_log_says_what_happened_in_the_past_tense_with_the_agent_s_name() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        let l = clean_paths(&st, Some(&a.id), vec![ws.join("scratch")]).await;
        assert_eq!((l.what.as_str(), l.title.as_str()), ("Clean", "Cleaned Loft Tester"));
        let l = clean_paths(&st, None, vec![]).await;
        assert_eq!((l.what.as_str(), l.title.as_str()), ("Clean all", "Cleaned all agents"));
        let l = run(&st, Some("Daily sweep"), None, IDLE).await;
        assert_eq!(l.title, "Daily sweep");
        // A log saved before titles still reads.
        let old: Log = serde_json::from_str(r#"{"at":null,"what":"Clean","running":false,"lines":[],"skipped":[],"freed_bytes":0,"removed":0,"failed":0}"#).unwrap();
        assert!(old.title.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_lists_workspace_sizes_and_free_disk_and_they_refresh_after_a_clean() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Worker");
        let axum::Json(v) = sizes(axum::extract::State(st.clone())).await.unwrap();
        let row = |v: &serde_json::Value| v["agents"].as_array().unwrap().iter().find(|r| r["id"] == a.id.as_str()).unwrap().clone();
        let before = row(&v)["bytes"].as_u64().unwrap();
        assert!(before >= 5000, "{}", row(&v));
        assert_eq!(row(&v)["busy"], false);
        assert!(v["free_bytes"].as_u64().unwrap() > 0);
        let r = clean_paths(&st, Some(&a.id), vec![ws.join("dev.log")]).await;
        let axum::Json(v) = sizes(axum::extract::State(st.clone())).await.unwrap();
        assert_eq!(row(&v)["bytes"].as_u64().unwrap(), before - r.freed_bytes, "sizes weren't refreshed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_last_run_s_log_stays_readable_after_a_restart_the_daily_sweep_s_too() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Idle");
        let ws = crate::agents::workspace(&st, &a.id);
        clone(&ws.join("loft"));
        age(&ws);
        let axum::Json(empty) = log(axum::extract::State(st.clone())).await;
        assert!(empty.at.is_none());

        run(&st, Some("Daily sweep"), None, IDLE).await;
        let axum::Json(l) = log(axum::extract::State(st.clone())).await;
        assert_eq!((l.what.as_str(), l.removed, l.running), ("Daily sweep", 5, false));
        let target = ws.canonicalize().unwrap().join("loft/target");
        assert!(l.lines.iter().any(|x| x.path == target && x.bytes > 0 && x.error.is_none()), "{:?}", l.lines);

        // A restart forgets the in-memory log; the saved one is read back.
        st.rt.lock().unwrap().clean = Log::default();
        let axum::Json(saved) = log(axum::extract::State(st.clone())).await;
        assert_eq!((saved.what.as_str(), saved.removed, saved.freed_bytes), ("Daily sweep", 5, l.freed_bytes));
        assert_eq!(saved.lines.len(), 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_log_is_kept_in_loft_s_data_folder_outside_the_agents_workspaces() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        // One left where it used to be, which an agent could have written.
        fs::write(st.cfg.storage_dir.join(".clean-log.json"), r#"{"what":"forged","running":false,"lines":[],"skipped":[],"freed_bytes":0,"removed":0,"failed":0}"#).unwrap();
        clean_paths(&st, Some(&a.id), vec![ws.join("scratch")]).await;
        let saved = st.settings.dir().join(LOG_FILE);
        assert!(saved.exists() && !saved.starts_with(&st.cfg.storage_dir));
        assert!(!st.cfg.storage_dir.join(".clean-log.json").exists());
        st.rt.lock().unwrap().clean = Log::default();
        let axum::Json(l) = log(axum::extract::State(st.clone())).await;
        assert_eq!((l.what.as_str(), l.removed), ("Clean", 1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_name_with_a_newline_stays_on_one_line_in_the_log_and_in_copy_commands() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        let name = "x\nrm -rf ~\t\u{7}";
        file(&ws.join(name).join("f"));
        let escaped = format!("{}/x\\nrm -rf ~\\t\\x07", ws.display());

        let items = preview_of(&st, Some(&a.id)).await;
        let i = items.iter().find(|i| i.name == name).unwrap();
        assert_eq!(i.command, format!("rm -rf $'{escaped}'"));

        let l = clean_paths(&st, Some(&a.id), vec![ws.join(name), ws.join("gone\nrm -rf ~")]).await;
        assert_eq!(l.removed, 1, "{l:?}");
        let line = &l.lines[0];
        assert_eq!((line.rel.as_str(), line.path.to_string_lossy().as_ref(), line.command.as_str()), ("x\\nrm -rf ~\\t\\x07", escaped.as_str(), format!("rm -rf $'{escaped}'").as_str()));
        assert_eq!((l.skipped[0].rel.as_str(), l.skipped[0].reason.as_str()), ("gone\\nrm -rf ~", "no longer there"));
        assert!(l.lines.iter().map(|x| format!("{}{}{}", x.rel, x.path.display(), x.command)).chain(l.skipped.iter().map(|x| x.rel.clone())).all(|t| !t.contains('\n')));
        assert_eq!(rm_command(Path::new("/w/it's\n\\x")), r"rm -rf $'/w/it\'s\n\\x'");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_workspace_folder_with_a_newline_in_its_name_is_neither_listed_swept_nor_cleaned() {
        let st = crate::tasks::tests::state();
        let (a, ws) = workspace_with_everything(&st, "Loft Tester");
        let root = ws.parent().unwrap().to_path_buf();
        let odd = root.join("\nrm -r ~");
        clone(&odd.join("app"));
        file(&odd.join("scratch/x"));
        age(&odd);

        let items = preview_of(&st, None).await;
        assert!(items.iter().all(|i| i.agent == a.id), "{items:?}");
        let p = plan(&root, None, &|_| false, later(), IDLE);
        assert!(p.folders.iter().all(|f| f.agent == a.id) && p.skipped.iter().all(|s| !s.agent.contains('\n')), "{p:?}");
        let l = clean_paths(&st, None, vec![odd.join("scratch"), odd.join("app/target")]).await;
        assert_eq!(l.removed, 0, "{l:?}");
        assert!(l.skipped.iter().all(|s| s.reason == "not in a workspace" && !s.agent.contains('\n')), "{l:?}");
        assert!(odd.join("scratch/x").exists() && odd.join("app/target/debug/app").exists());
    }

    #[test]
    fn a_folder_swapped_for_a_symlink_after_the_checks_leads_nowhere_outside() {
        let root = temp();
        let storage = root.join("agent-local-storage");
        let outside = root.join("outside");
        clone(&outside);
        file(&outside.join("scratch/keep"));
        let ws = storage.join("a");
        clone(&ws.join("app"));
        file(&ws.join("scratch/x"));
        let p = plan(&storage, None, &|_| false, later(), IDLE);
        assert!(p.folders.iter().any(|f| f.rel == "app/target"));

        // Between the checks and the removal, the clone and `scratch` become symlinks to folders outside.
        fs::rename(ws.join("app"), root.join("app-moved")).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("app")).unwrap();
        fs::remove_dir_all(ws.join("scratch")).unwrap();
        std::os::unix::fs::symlink(outside.join("scratch"), ws.join("scratch")).unwrap();
        let storage = storage.canonicalize().unwrap();
        let mut lines = vec![];
        for f in p.folders {
            lines.push(remove_under(&storage, &f.path));
        }
        assert!(lines.iter().all(Result::is_err), "{lines:?}");
        assert!(remove_item(&storage, &storage.join("a/scratch/keep")).is_err());
        assert!(remove_under(&storage, &storage.join("a/scratch/keep")).is_err());
        for kept in ["target/debug/app", "frontend/node_modules/vite/index.js", "ios/build/App.app/App", "scratch/keep"] {
            assert!(outside.join(kept).exists(), "{kept} outside was removed");
        }
        // A real folder still goes, with the symlinks inside it but not what they point at.
        std::os::unix::fs::symlink(&outside, ws.join("app-moved-link")).unwrap();
        file(&storage.join("a/real/deep/er/f"));
        std::os::unix::fs::symlink(&outside, storage.join("a/real/deep/out")).unwrap();
        remove_under(&storage, &storage.join("a/real")).unwrap();
        assert!(!storage.join("a/real").exists() && outside.join("scratch/keep").exists());
    }

    #[test]
    fn needs_you_says_disk_almost_full_under_20_gb_free() {
        let st = crate::tasks::tests::state();
        let items = |st: &AppState| crate::needs_you::snapshot(st).unwrap()["items"].as_array().unwrap().iter().filter(|i| i["type"] == "disk_low").cloned().collect::<Vec<_>>();
        FAKE_FREE.with(|f| f.set(Some(19_000_000_000)));
        let low = items(&st);
        FAKE_FREE.with(|f| f.set(Some(21_000_000_000)));
        let ok = items(&st);
        FAKE_FREE.with(|f| f.set(None));
        assert_eq!(low.len(), 1);
        assert_eq!(low[0]["title"], "Disk almost full");
        assert!(ok.is_empty());
    }

    #[test]
    fn an_idle_clone_loses_its_build_output_and_keeps_source_git_and_notes() {
        let storage = temp();
        let ws = storage.join("a");
        clone(&ws.join("loft"));
        file(&ws.join("notes/plan.md"));
        file(&ws.join(".cargo-build/debug/deps/x"));
        // A folder named `target` that isn't Cargo's, and a non-git folder's node_modules.
        file(&ws.join("loft/docs/target/keep.md"));
        file(&ws.join("scratch/node_modules/keep.js"));

        let r = sweep(&storage, None, &|_| false, later(), IDLE);

        for gone in ["loft/target", "loft/frontend/node_modules", "loft/ios/build", "loft/ios/DerivedData", "loft/Pkg/.build", ".cargo-build"] {
            assert!(!ws.join(gone).exists(), "{gone} stays");
            assert!(r.removed_paths().contains(&ws.join(gone)), "{gone} not reported");
        }
        for kept in ["loft/.git/HEAD", "loft/Cargo.toml", "loft/src/main.rs", "loft/ios/App/App.swift", "loft/docs/target/keep.md", "notes/plan.md", "scratch/node_modules/keep.js"] {
            assert!(ws.join(kept).exists(), "{kept} removed");
        }
        assert_eq!(r.removed, 6);
        assert!(r.freed_bytes > 0);
    }

    /// `.cargo-build/` as Cargo leaves it: each profile's deps and incremental cache, freshly built.
    fn cargo_build(ws: &Path) {
        for f in ["debug/deps/libx.rlib", "debug/incremental/x-1/s-1/dep-graph.bin", "release/deps/liby.rlib", "release/incremental/y-1/query-cache.bin"] {
            fs::create_dir_all(ws.join(".cargo-build").join(f).parent().unwrap()).unwrap();
            fs::write(ws.join(".cargo-build").join(f), vec![0u8; 64 * 1024]).unwrap();
        }
    }

    #[test]
    fn over_the_cap_a_working_agent_s_incremental_caches_go_and_deps_stay() {
        let storage = temp();
        cargo_build(&storage.join("a"));
        // 2 GB, scaled down: the two caches take about 128 KB on disk.
        FAKE_CAP.with(|c| c.set(Some(100_000)));

        let l = sweep(&storage, None, &|_| false, SystemTime::now(), IDLE);

        let shared = storage.join("a/.cargo-build");
        assert!(!shared.join("debug/incremental").exists() && !shared.join("release/incremental").exists());
        assert!(shared.join("debug/deps/libx.rlib").exists() && shared.join("release/deps/liby.rlib").exists());
        let mut rels: Vec<_> = l.lines.iter().map(|x| (x.rel.as_str(), x.error.is_none() && x.bytes > 0)).collect();
        rels.sort();
        assert_eq!(rels, [(".cargo-build/debug/incremental", true), (".cargo-build/release/incremental", true)], "the log shows them");
        assert!(l.skipped.contains(&skip("a", ".cargo-build", "younger than 3 days")));
        FAKE_CAP.with(|c| c.set(None));
    }

    #[test]
    fn under_the_cap_or_while_the_agent_runs_the_incremental_caches_stay() {
        let storage = temp();
        cargo_build(&storage.join("a"));
        cargo_build(&storage.join("running"));
        FAKE_CAP.with(|c| c.set(Some(100_000)));
        let l = sweep(&storage, None, &|id| id == "running", SystemTime::now(), IDLE);
        assert!(storage.join("running/.cargo-build/debug/incremental/x-1/s-1/dep-graph.bin").exists(), "removed during a run");
        assert!(l.removed_paths().iter().all(|p| !p.starts_with(storage.join("running"))));

        FAKE_CAP.with(|c| c.set(Some(10_000_000)));
        cargo_build(&storage.join("b"));
        let l = sweep(&storage, Some("b"), &|_| false, SystemTime::now(), IDLE);
        assert_eq!(l.lines.len(), 0, "{:?}", l.lines);
        assert!(storage.join("b/.cargo-build/debug/incremental/x-1/s-1/dep-graph.bin").exists());
        FAKE_CAP.with(|c| c.set(None));
    }

    #[test]
    fn the_sweep_goes_by_the_newest_file_inside_each_build_folder() {
        let storage = temp();
        let ws = storage.join("a");
        clone(&ws.join("loft"));
        file(&ws.join(".cargo-build/debug/deps/x.rlib"));
        file(&ws.join("loft/frontend/node_modules/fresh/index.js"));
        age(&ws);
        // Source and git changed just now, and the folders themselves were touched (a listing, a lock file):
        // none of that is a build.
        file(&ws.join("loft/src/main.rs"));
        file(&ws.join("loft/.git/FETCH_HEAD"));
        for d in [".cargo-build", "loft/target", "loft"] {
            fs::File::open(ws.join(d)).unwrap().set_modified(SystemTime::now()).unwrap();
        }
        // But a file written just now inside node_modules keeps it.
        file(&ws.join("loft/frontend/node_modules/fresh/index.js"));

        let r = sweep(&storage, None, &|_| false, SystemTime::now(), IDLE);
        for gone in [".cargo-build", "loft/target", "loft/ios/build"] {
            assert!(!ws.join(gone).exists(), "{gone} stays: {:?}", r.skipped);
        }
        assert!(ws.join("loft/frontend/node_modules/fresh/index.js").exists());
        assert!(r.skipped.contains(&skip("a", "loft/frontend/node_modules", "younger than 3 days")), "{:?}", r.skipped);
    }

    /// Dates everything in `dir` 4 days back.
    fn age(dir: &Path) {
        age_by(dir, Duration::from_secs(4 * 24 * 60 * 60));
    }

    fn age_by(dir: &Path, by: Duration) {
        let then = SystemTime::now() - by;
        for e in fs::read_dir(dir).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                age_by(&e.path(), by);
            }
            fs::File::open(e.path()).unwrap().set_modified(then).unwrap();
        }
        fs::File::open(dir).unwrap().set_modified(then).unwrap();
    }

    #[test]
    fn a_running_agent_s_workspace_and_a_fresh_clone_are_skipped() {
        let storage = temp();
        clone(&storage.join("running/loft"));
        clone(&storage.join("idle/old"));
        age(&storage.join("running"));
        age(&storage.join("idle/old"));
        clone(&storage.join("idle/fresh"));

        let r = sweep(&storage, None, &|id| id == "running", SystemTime::now(), IDLE);

        assert!(!storage.join("idle/old/target").exists(), "an idle clone keeps its build output");
        assert!(storage.join("idle/fresh/target").exists(), "a fresh clone lost its build output");
        assert!(storage.join("running/loft/target").exists(), "a running agent's build output was removed");
        assert!(r.removed_paths().iter().all(|p| p.starts_with(storage.join("idle/old"))), "{:?}", r.lines);
    }

    #[test]
    fn a_run_that_starts_mid_sweep_keeps_its_build_output() {
        let storage = temp();
        clone(&storage.join("a/loft"));
        clone(&storage.join("a/homebase"));
        // Idle when the sweep reaches the workspace, running by the time it reaches a clone.
        let asked = std::cell::Cell::new(0);
        let busy = |_: &str| {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        let r = sweep(&storage, None, &busy, later(), IDLE);
        assert_eq!(r.removed, 0, "{:?}", r.lines);
        assert!(storage.join("a/loft/target").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_with_a_live_process_is_skipped() {
        use std::os::unix::process::CommandExt;
        let st = crate::tasks::tests::state();
        let (server, idle) = (crate::tasks::tests::agent(&st, "Server"), crate::tasks::tests::agent(&st, "Idle"));
        for a in [&server, &idle] {
            let ws = crate::agents::workspace(&st, &a.id);
            clone(&ws.join("loft"));
            age(&ws);
        }
        // A detached dev server: a process in a session of its own, recorded as the agent's.
        let mut cmd = std::process::Command::new("/bin/sleep");
        cmd.arg("60");
        unsafe { cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) }) };
        let mut dev = cmd.spawn().unwrap();
        crate::processes::record(&st.cfg.storage_dir, &server.id, dev.id()).unwrap();

        run(&st, None, None, IDLE).await;
        let _ = dev.kill();
        let _ = dev.wait();

        assert!(crate::agents::workspace(&st, &server.id).join("loft/target").exists(), "a dev server's agent lost its build output");
        assert!(!crate::agents::workspace(&st, &idle.id).join("loft/target").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_agent_s_sweep_leaves_the_last_log_alone() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Gone");
        let ws = crate::agents::workspace(&st, &a.id);
        clone(&ws.join("loft"));
        age(&ws);
        let one = run(&st, None, Some(&a.id), IDLE).await;
        assert!(one.removed > 0);
        assert!(st.rt.lock().unwrap().clean.at.is_none(), "the last log is a deleted agent's");
        run(&st, Some("Daily sweep"), None, IDLE).await;
        assert!(st.rt.lock().unwrap().clean.at.is_some());
    }

    #[test]
    fn sizes_are_disk_blocks_like_the_freed_bytes() {
        let storage = temp();
        clone(&storage.join("a/loft"));
        let (before, _) = du(&storage.join("a/loft/target"));
        let r = sweep(&storage, None, &|_| false, later(), IDLE);
        let line = r.lines.iter().find(|l| l.path.ends_with("loft/target")).unwrap();
        assert_eq!(line.bytes, before);
        assert!(before >= 4096, "a 1-byte file takes a block: {before}");
    }

    #[test]
    fn with_cargo_build_a_fresh_clone_s_duplicate_cargo_output_goes_and_its_binary_stays() {
        let storage = temp();
        let ws = storage.join("a");
        clone(&ws.join("loft"));
        for f in ["target/debug/deps/x.rlib", "target/debug/incremental/x", "target/debug/build/x", "target/release/deps/y.rlib"] {
            file(&ws.join("loft").join(f));
        }
        file(&ws.join(".cargo-build/debug/deps/x.rlib"));

        // Built just now: only the duplicates go.
        let p = plan(&storage, None, &|_| false, SystemTime::now(), IDLE);
        let mut rels: Vec<_> = p.folders.iter().map(|f| f.rel.as_str()).collect();
        rels.sort();
        assert_eq!(rels, ["loft/target/debug/build", "loft/target/debug/deps", "loft/target/debug/incremental", "loft/target/release/deps"]);
        let r = sweep(&storage, None, &|_| false, SystemTime::now(), IDLE);
        assert_eq!(r.removed, 4);
        assert!(ws.join("loft/target/debug/app").exists(), "the binary stays");
        assert!(!ws.join("loft/target/debug/deps").exists());
        assert!(ws.join(".cargo-build/debug/deps/x.rlib").exists(), "the shared build stays while it's in use");
        assert!(ws.join("loft/frontend/node_modules/vite/index.js").exists(), "fresh build output stays");
    }

    #[test]
    fn without_cargo_build_a_fresh_clone_keeps_its_cargo_output() {
        let storage = temp();
        let ws = storage.join("a");
        clone(&ws.join("loft"));
        file(&ws.join("loft/target/debug/deps/x.rlib"));
        assert!(plan(&storage, None, &|_| false, SystemTime::now(), IDLE).folders.is_empty());
    }

    #[test]
    fn each_folder_carries_its_equivalent_command() {
        let storage = temp();
        clone(&storage.join("a/loft"));
        let p = plan(&storage, None, &|_| false, later(), IDLE);
        let f = p.folders.iter().find(|f| f.rel == "loft/target").unwrap();
        assert!(f.path.is_absolute());
        assert_eq!(f.command, format!("rm -rf {}", f.path.display()));
        assert_eq!(rm_command(Path::new("/w/it's here/target")), r"rm -rf '/w/it'\''s here/target'");
    }

    #[test]
    fn one_workspace_alone_is_swept_when_named() {
        let storage = temp();
        clone(&storage.join("a/loft"));
        clone(&storage.join("b/loft"));
        sweep(&storage, Some("a"), &|_| false, later(), IDLE);
        assert!(!storage.join("a/loft/target").exists());
        assert!(storage.join("b/loft/target").exists());
    }

    #[test]
    fn symlinks_are_not_followed_and_nothing_outside_the_agents_folder_is_touched() {
        let root = temp();
        let storage = root.join("agent-local-storage");
        // Loft's own checkout, next to the agents' folder, and another folder outside it.
        let loft = root.join("loft");
        clone(&loft);
        let outside = root.join("outside");
        file(&outside.join("lib/index.js"));

        let ws = storage.join("a");
        clone(&ws.join("app"));
        std::os::unix::fs::symlink(&loft, ws.join("loft")).unwrap();
        fs::create_dir_all(ws.join("app/web")).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("app/web/node_modules")).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("app/target/linked")).unwrap();
        // A whole workspace that is a symlink to Loft's checkout.
        std::os::unix::fs::symlink(&loft, storage.join("b")).unwrap();

        let r = sweep(&storage, None, &|_| false, later(), IDLE);

        assert!(!ws.join("app/target").exists(), "the real target/ is removed");
        assert!(ws.join("app/web/node_modules").symlink_metadata().is_ok(), "a symlinked node_modules is left alone");
        for kept in ["target/debug/app", "frontend/node_modules/vite/index.js", "ios/build/App.app/App"] {
            assert!(loft.join(kept).exists(), "Loft's checkout lost {kept}");
        }
        assert!(outside.join("lib/index.js").exists(), "a symlink target was removed");
        assert!(r.removed_paths().iter().all(|p| p.starts_with(&storage)), "{:?}", r.lines);
        assert!(r.skipped.contains(&skip("a", "app/web/node_modules", "symlink")), "{:?}", r.skipped);
        assert!(r.skipped.contains(&skip("b", "", "symlink")), "{:?}", r.skipped);
        // Nor does Clean list a symlinked workspace.
        assert!(listing(&storage, None, &|_| vec![], &|_| false).iter().all(|i| i.agent == "a"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deleting_an_agent_but_keeping_its_files_sweeps_its_workspace() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Gone");
        let ws = crate::agents::workspace(&st, &a.id);
        clone(&ws.join("loft"));
        age(&ws);

        let uri: axum::http::Uri = "/?purge_files=false".parse().unwrap();
        let opts = axum::extract::Query::try_from_uri(&uri).unwrap();
        let _ = crate::api::delete_agent(axum::extract::State(st.clone()), axum::extract::Path(a.id.clone()), opts).await.ok().unwrap();

        for _ in 0..50 {
            if !ws.join("loft/target").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!ws.join("loft/target").exists(), "the kept workspace's build output stays");
        assert!(ws.join("loft/src/main.rs").exists());
    }
}
