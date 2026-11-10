//! The sweep: deletes rebuildable build output (`target/`, `node_modules/`, `.build/`, `ios/build/`,
//! `DerivedData/`, `.cargo-build/`) from the clones in agents' workspaces that haven't changed in 3 days, so
//! they don't fill the disk. Source, git state and notes stay.
//!
//! It runs once a day, before Loft restarts into an update, and on a workspace whose agent was deleted with its
//! files kept. It skips running agents and agents with a live process (a dev server), never follows a symlink,
//! and never removes anything outside the agents' folder (`agent-local-storage/`), so Loft's own checkout stays.
//!
//! Settings' Clean (one agent) and Clean all remove build output older than an hour. Nothing goes blind: they
//! first preview the folders (`GET /api/workspaces/clean/preview`), then remove the ones the user kept ticked
//! (`POST /api/workspaces/clean`). Each run writes a log of what it removed, skipped and why, and the last one
//! stays readable (`GET /api/workspaces/log`), the daily sweep's included.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// A clone untouched this long loses its build output.
pub const IDLE: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// Clean and Clean all leave a clone changed this recently: it is probably mid-build.
pub const RECENT: Duration = Duration::from_secs(60 * 60);
/// How deep in a workspace clones are looked for (`<workspace>/a/b/<clone>`).
const DEPTH: usize = 3;
/// How often the day's sweep runs.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long a measured size is reused: Settings and the delete dialog reload often, and a walk is slow.
const SIZE_FOR: Duration = Duration::from_secs(60);
/// The last run's log, next to the workspaces (the sweep skips dotfiles there).
const LOG_FILE: &str = ".clean-log.json";

/// A build folder a clean would remove.
#[derive(Clone, Debug, Serialize)]
pub struct Folder {
    pub agent: String,
    pub path: PathBuf,
    /// Inside the agent's workspace, like `loft/backend/target`.
    pub rel: String,
    pub bytes: u64,
    /// The newest change to its clone, what the age rule goes by.
    pub changed: DateTime<Local>,
}

/// What a clean leaves, and why.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Skip {
    pub agent: String,
    /// Inside the agent's workspace; empty for the whole workspace.
    pub rel: String,
    pub reason: String,
}

/// `GET /api/workspaces/clean/preview`: what a clean would remove and what it would skip.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Plan {
    pub folders: Vec<Folder>,
    pub skipped: Vec<Skip>,
    pub total_bytes: u64,
}

/// One removal: the folder and what it freed, or why it failed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Line {
    pub agent: String,
    pub path: PathBuf,
    pub rel: String,
    pub bytes: u64,
    pub error: Option<String>,
}

/// A run's log: each removal as it happens, what was skipped, and the totals.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Log {
    pub at: Option<DateTime<Local>>,
    /// "Clean", "Clean all", "Daily sweep" or "Sweep before update".
    pub what: String,
    pub running: bool,
    pub lines: Vec<Line>,
    pub skipped: Vec<Skip>,
    pub freed_bytes: u64,
    pub removed: usize,
    pub failed: usize,
}

impl Log {
    fn push(&mut self, l: Line) {
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
/// so Clean skips it), and the disk's free space.
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

#[derive(Deserialize)]
pub struct PreviewOpts {
    agent: Option<String>,
}

/// `GET /api/workspaces/clean/preview[?agent=<id>]`: what Clean on that agent, or Clean all, would remove.
pub async fn preview(axum::extract::State(state): axum::extract::State<AppState>, axum::extract::Query(q): axum::extract::Query<PreviewOpts>) -> axum::Json<Plan> {
    let p = tokio::task::spawn_blocking(move || {
        let storage = &state.cfg.storage_dir;
        let live = crate::processes::with_live_processes(storage);
        let busy = |id: &str| live.contains(id) || state.rt.lock().unwrap().running.contains_key(id);
        plan(storage, q.agent.as_deref(), &busy, SystemTime::now(), RECENT)
    })
    .await
    .unwrap_or_default();
    axum::Json(p)
}

#[derive(Deserialize)]
pub struct CleanBody {
    agent: Option<String>,
    /// The previewed folders the user kept ticked.
    paths: Vec<PathBuf>,
}

/// `POST /api/workspaces/clean`: removes the folders the user kept ticked in the preview of Clean (`agent`) or
/// Clean all, those still on a fresh preview, and answers the log.
pub async fn clean(axum::extract::State(state): axum::extract::State<AppState>, axum::Json(b): axum::Json<CleanBody>) -> axum::Json<Log> {
    let what = if b.agent.is_some() { "Clean" } else { "Clean all" };
    axum::Json(run(&state, Some(what), b.agent.as_deref(), RECENT, Some(b.paths)).await)
}

/// `GET /api/workspaces/log`: the running or last run's log, kept across restarts.
pub async fn log(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<Log> {
    let l = state.rt.lock().unwrap().clean.clone();
    if l.at.is_some() {
        return axum::Json(l);
    }
    let file = state.cfg.storage_dir.join(LOG_FILE);
    let saved = tokio::fs::read(file).await.ok().and_then(|b| serde_json::from_slice(&b).ok());
    axum::Json(saved.unwrap_or_default())
}

/// One sweep at a time.
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Removes the build output of clones untouched for `idle` from every workspace but those of running agents and
/// agents with a live process, or just from `only`'s; with `chosen`, only those of the folders. With `what`, the
/// run's log is the one Settings shows, live and saved; without, it is only returned.
pub async fn run(state: &AppState, what: Option<&'static str>, only: Option<&str>, idle: Duration, chosen: Option<Vec<PathBuf>>) -> Log {
    let _one = RUNNING.lock().await;
    let (st, only) = (state.clone(), only.map(str::to_string));
    let log = tokio::task::spawn_blocking(move || {
        let storage = &st.cfg.storage_dir;
        let live = crate::processes::with_live_processes(storage);
        // Asked again before each removal, so a run that starts mid-sweep keeps its build output.
        let busy = |id: &str| live.contains(id) || st.rt.lock().unwrap().running.contains_key(id);
        let mut p = plan(storage, only.as_deref(), &busy, SystemTime::now(), idle);
        let mut log = Log { at: Some(Local::now()), what: what.unwrap_or_default().into(), running: true, ..Default::default() };
        if let Some(chosen) = chosen {
            let (keep, unticked): (Vec<_>, Vec<_>) = p.folders.into_iter().partition(|f| chosen.contains(&f.path));
            log.skipped.extend(unticked.into_iter().map(|f| Skip { agent: f.agent, rel: f.rel, reason: "unticked".into() }));
            p.folders = keep;
        }
        log.skipped.extend(p.skipped);
        let show = |log: &Log| {
            if what.is_some() {
                st.rt.lock().unwrap().clean = log.clone();
            }
        };
        show(&log);
        let root = storage.canonicalize().unwrap_or_default();
        apply(&root, p.folders, &busy, &mut |l| {
            log.push(l);
            show(&log);
        });
        log.running = false;
        show(&log);
        if what.is_some()
            && let Err(e) = serde_json::to_vec(&log).map_err(std::io::Error::other).and_then(|b| std::fs::write(storage.join(LOG_FILE), b))
        {
            tracing::warn!("sweep: can't save the log: {e}");
        }
        log
    })
    .await
    .unwrap_or_default();
    tracing::info!("sweep freed {} bytes in {} folders, {} failed", log.freed_bytes, log.removed, log.failed);
    *SIZES.lock().unwrap() = None;
    log
}

/// The day's sweep, the first right after Loft starts.
pub async fn scheduler(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        run(&state, Some("Daily sweep"), None, IDLE, None).await;
    }
}

/// What a clean of the workspaces in `storage` (or only `only`'s) would remove as of `now`: the build output of
/// clones untouched for `idle`, in workspaces of agents `busy` doesn't name.
pub fn plan(storage: &Path, only: Option<&str>, busy: &dyn Fn(&str) -> bool, now: SystemTime, idle: Duration) -> Plan {
    let mut p = Plan::default();
    let Ok(root) = storage.canonicalize() else { return p };
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let id = e.file_name().to_string_lossy().to_string();
        // `.agent-files` and the like are Loft's.
        if id.starts_with('.') || only.is_some_and(|o| o != id) {
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
    p.total_bytes = p.folders.iter().map(|f| f.bytes).sum();
    p
}

fn skip(agent: &str, rel: &str, reason: &str) -> Skip {
    Skip { agent: agent.into(), rel: rel.into(), reason: reason.into() }
}

fn workspace(root: &Path, id: &str, ws: &Path, now: SystemTime, idle: Duration, p: &mut Plan) {
    let rel = |path: &Path| path.strip_prefix(ws).unwrap_or(path).to_string_lossy().to_string();
    let recent = if idle >= IDLE { "younger than 3 days" } else { "changed in the last hour" };
    let mut clones = vec![];
    find_clones(ws, DEPTH, &mut clones);
    let mut all_idle = true;
    for clone in clones {
        let (mut builds, mut links) = (vec![], vec![]);
        let mut newest = git_time(&clone);
        walk(&clone, &mut builds, &mut links, &mut newest);
        p.skipped.extend(links.iter().map(|l| skip(id, &rel(l), "symlink")));
        if now.duration_since(newest).unwrap_or_default() < idle {
            all_idle = false;
            if !builds.is_empty() {
                p.skipped.push(skip(id, &rel(&clone), recent));
            }
            continue;
        }
        for b in builds {
            folder(root, id, &rel(&b), b, newest, p);
        }
    }
    // Compiled dependencies all the workspace's clones share: kept while any of them is in use.
    let shared = ws.join(".cargo-build");
    if let Ok(m) = shared.symlink_metadata()
        && m.is_dir()
    {
        if all_idle {
            folder(root, id, ".cargo-build", shared, m.modified().unwrap_or(now), p);
        } else {
            p.skipped.push(skip(id, ".cargo-build", recent));
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
    p.folders.push(Folder { agent: id.into(), path, rel: rel.into(), bytes, changed: changed.into() });
}

/// Removes the folders, reporting each as it goes; an agent that started working keeps its.
fn apply(root: &Path, folders: Vec<Folder>, busy: &dyn Fn(&str) -> bool, line: &mut dyn FnMut(Line)) {
    for f in folders {
        let error = if busy(&f.agent) { Some("agent started working".into()) } else { remove(root, &f.path).err() };
        let bytes = if error.is_none() { f.bytes } else { 0 };
        line(Line { agent: f.agent, path: f.path, rel: f.rel, bytes, error });
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
    for (name, path) in subdirs(dir) {
        if !is_build(dir, &name) {
            find_clones(&path, depth - 1, out);
        }
    }
}

/// Whether `parent/name` is build output.
fn is_build(parent: &Path, name: &str) -> bool {
    match name {
        "node_modules" | "DerivedData" => true,
        "target" => parent.join("Cargo.toml").exists(),
        "build" => parent.file_name().is_some_and(|n| n == "ios"),
        _ => false,
    }
}
/// Collects the clone's build folders, the symlinked ones it leaves, and the newest change to anything else in
/// it, stopping at nested clones.
fn walk(dir: &Path, builds: &mut Vec<PathBuf>, links: &mut Vec<PathBuf>, newest: &mut SystemTime) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // Not followed: a symlink's own metadata.
        let Ok(meta) = e.metadata() else { continue };
        *newest = (*newest).max(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        let build = name == ".build" || name == ".cargo-build" || is_build(dir, &name);
        if meta.is_symlink() && build {
            links.push(e.path());
        }
        if !meta.is_dir() || name == ".git" {
            continue;
        }
        let path = e.path();
        if build {
            builds.push(path);
        } else if path.join(".git").symlink_metadata().is_err() {
            walk(&path, builds, links, newest);
        }
    }
}

/// When the clone's git state last changed: a commit, checkout, add or fetch.
fn git_time(clone: &Path) -> SystemTime {
    let dot = clone.join(".git");
    // A worktree's `.git` is a file naming its git dir.
    let dir = match std::fs::read_to_string(&dot) {
        Ok(s) => s.trim().strip_prefix("gitdir:").map(|d| clone.join(d.trim())).unwrap_or(dot),
        Err(_) => dot,
    };
    ["index", "HEAD", "logs/HEAD", "FETCH_HEAD"]
        .iter()
        .filter_map(|f| dir.join(f).symlink_metadata().ok()?.modified().ok())
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Removes `path` when it is still a real directory inside `root`.
fn remove(root: &Path, path: &Path) -> Result<(), String> {
    if !path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
        return Err("no longer a folder".into());
    }
    if !path.canonicalize().is_ok_and(|p| p.starts_with(root)) {
        return Err("outside agent-local-storage".into());
    }
    // Removes symlinks inside, not what they point at.
    std::fs::remove_dir_all(path).map_err(|e| e.to_string())
}

/// Measured sizes, by path: when and how big.
static SIZES: Mutex<Option<HashMap<PathBuf, (Instant, u64)>>> = Mutex::new(None);

/// Disk space `path` takes, as `du` measures it (the same measure as a sweep's freed bytes), measured at most
/// once a minute; 0 when it doesn't exist. Blocking.
pub fn size(path: &Path) -> u64 {
    if let Some(&(at, n)) = SIZES.lock().unwrap().get_or_insert_default().get(path)
        && at.elapsed() < SIZE_FOR
    {
        return n;
    }
    let n = du(path);
    SIZES.lock().unwrap().get_or_insert_default().insert(path.to_path_buf(), (Instant::now(), n));
    n
}

/// Disk space `path` takes, symlinks not followed.
fn du(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else { return 0 };
    let mut n = meta.blocks() * 512;
    if meta.is_dir() {
        for e in std::fs::read_dir(path).into_iter().flatten().flatten() {
            n += du(&e.path());
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("loft-sweep-{}", uuid::Uuid::new_v4()));
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

    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(4 * 24 * 60 * 60)
    }

    /// Plans and removes everything, as the daily sweep does.
    fn sweep(storage: &Path, only: Option<&str>, busy: &dyn Fn(&str) -> bool, now: SystemTime, idle: Duration) -> Log {
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

    async fn preview_of(st: &AppState, agent: Option<&str>) -> Plan {
        let q = agent.map(|a| format!("/?agent={a}")).unwrap_or("/".into());
        let axum::Json(p) = preview(axum::extract::State(st.clone()), axum::extract::Query::try_from_uri(&q.parse().unwrap()).unwrap()).await;
        p
    }

    /// Previews Clean (or Clean all) and confirms every folder it shows.
    async fn clean_all_shown(st: &AppState, agent: Option<&str>) -> Log {
        let paths = preview_of(st, agent).await.folders.into_iter().map(|f| f.path).collect();
        let axum::Json(l) = clean(axum::extract::State(st.clone()), axum::Json(CleanBody { agent: agent.map(Into::into), paths })).await;
        l
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_lists_workspace_sizes_and_free_disk_and_clean_removes_build_output_of_any_age() {
        let st = crate::tasks::tests::state();
        let (a, b) = (crate::tasks::tests::agent(&st, "Worker"), crate::tasks::tests::agent(&st, "Other"));
        let ws = crate::agents::workspace(&st, &a.id);
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("big.bin"), vec![0u8; 5000]).unwrap();
        clone(&ws.join("loft"));
        file(&ws.join("notes/plan.md"));
        // Built two hours ago: well under the daily sweep's 3 days.
        age_by(&ws, Duration::from_secs(2 * 60 * 60));
        let other = crate::agents::workspace(&st, &b.id);
        clone(&other.join("loft"));
        age_by(&other, Duration::from_secs(2 * 60 * 60));
        let axum::Json(v) = sizes(axum::extract::State(st.clone())).await.unwrap();
        let row = |v: &serde_json::Value| v["agents"].as_array().unwrap().iter().find(|r| r["id"] == a.id.as_str()).unwrap().clone();
        let before = row(&v)["bytes"].as_u64().unwrap();
        assert!(before >= 5000, "{}", row(&v));
        assert_eq!(row(&v)["busy"], false);
        assert!(v["free_bytes"].as_u64().unwrap() > 0);
        // Clean on one agent: its build output goes; source, git and notes stay, and the other agent's
        // workspace isn't touched.
        let r = clean_all_shown(&st, Some(&a.id)).await;
        assert!(r.freed_bytes > 0);
        assert_eq!((r.removed, r.failed, r.running), (5, 0, false), "{:?}", r.lines);
        assert_eq!(r.what, "Clean");
        assert!(!ws.join("loft/target").exists());
        for kept in ["loft/.git/HEAD", "loft/.git/index", "loft/src/main.rs", "loft/Cargo.toml", "notes/plan.md", "big.bin"] {
            assert!(ws.join(kept).exists(), "{kept} removed");
        }
        assert!(other.join("loft/target").exists(), "another agent's build output was removed");
        // The sizes refresh right away.
        let axum::Json(v) = sizes(axum::extract::State(st.clone())).await.unwrap();
        assert_eq!(row(&v)["bytes"].as_u64().unwrap(), before - r.freed_bytes, "sizes weren't refreshed");
        // Clean all: every agent.
        let r = clean_all_shown(&st, None).await;
        let other_real = other.canonicalize().unwrap();
        assert!(r.removed_paths().iter().all(|p| p.starts_with(&other_real)) && r.removed > 0, "{:?}", r.lines);
        assert_eq!(r.what, "Clean all");
        assert!(!other.join("loft/target").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_preview_lists_folders_and_skips_and_a_clean_removes_only_the_ticked_ones() {
        let st = crate::tasks::tests::state();
        let (a, busy, fresh) = (crate::tasks::tests::agent(&st, "Idle"), crate::tasks::tests::agent(&st, "Busy"), crate::tasks::tests::agent(&st, "Fresh"));
        let ws = crate::agents::workspace(&st, &a.id);
        clone(&ws.join("loft"));
        age_by(&ws, Duration::from_secs(2 * 60 * 60));
        clone(&crate::agents::workspace(&st, &busy.id).join("loft"));
        st.rt.lock().unwrap().running.insert(busy.id.clone(), Default::default());
        // Built just now.
        clone(&crate::agents::workspace(&st, &fresh.id).join("loft"));

        let p = preview_of(&st, None).await;
        let rels: Vec<_> = p.folders.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels.len(), 5, "{rels:?}");
        for r in ["loft/target", "loft/frontend/node_modules", "loft/ios/build", "loft/ios/DerivedData", "loft/Pkg/.build"] {
            assert!(rels.contains(&r), "{r} not previewed: {rels:?}");
        }
        assert!(p.folders.iter().all(|f| f.agent == a.id && f.bytes > 0));
        assert_eq!(p.total_bytes, p.folders.iter().map(|f| f.bytes).sum::<u64>());
        assert!(p.skipped.contains(&Skip { agent: busy.id.clone(), rel: "".into(), reason: "agent is working".into() }), "{:?}", p.skipped);
        assert!(p.skipped.contains(&Skip { agent: fresh.id.clone(), rel: "loft".into(), reason: "changed in the last hour".into() }), "{:?}", p.skipped);
        // The preview removes nothing.
        assert!(ws.join("loft/target").exists());

        // The user unticks target/; a path that wasn't previewed is ignored.
        let mut paths: Vec<_> = p.folders.iter().filter(|f| f.rel != "loft/target").map(|f| f.path.clone()).collect();
        paths.push(ws.join("loft/src"));
        let axum::Json(l) = clean(axum::extract::State(st.clone()), axum::Json(CleanBody { agent: None, paths })).await;
        assert!(ws.join("loft/target").exists(), "an unticked folder was removed");
        assert!(ws.join("loft/src/main.rs").exists(), "a folder outside the preview was removed");
        assert!(!ws.join("loft/frontend/node_modules").exists());
        assert_eq!((l.removed, l.failed), (4, 0));
        let freed: u64 = p.folders.iter().filter(|f| f.rel != "loft/target").map(|f| f.bytes).sum();
        assert_eq!(l.freed_bytes, freed);
        assert!(l.skipped.contains(&Skip { agent: a.id.clone(), rel: "loft/target".into(), reason: "unticked".into() }), "{:?}", l.skipped);
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

        run(&st, Some("Daily sweep"), None, IDLE, None).await;
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
    async fn clean_all_skips_a_running_agent_and_sizes_say_it_is_busy() {
        let st = crate::tasks::tests::state();
        let (running, idle) = (crate::tasks::tests::agent(&st, "Running"), crate::tasks::tests::agent(&st, "Idle"));
        for a in [&running, &idle] {
            let ws = crate::agents::workspace(&st, &a.id);
            clone(&ws.join("loft"));
            age(&ws);
        }
        st.rt.lock().unwrap().running.insert(running.id.clone(), Default::default());

        let axum::Json(v) = sizes(axum::extract::State(st.clone())).await.unwrap();
        let busy = |id: &str| v["agents"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap()["busy"].clone();
        assert_eq!(busy(&running.id), true);
        assert_eq!(busy(&idle.id), false);

        let _ = clean_all_shown(&st, None).await;
        // Even a confirm naming its folders leaves them.
        let theirs = crate::agents::workspace(&st, &running.id).canonicalize().unwrap().join("loft/target");
        let axum::Json(one) = clean(axum::extract::State(st.clone()), axum::Json(CleanBody { agent: Some(running.id.clone()), paths: vec![theirs] })).await;
        assert_eq!(one.removed, 0);
        assert!(crate::agents::workspace(&st, &running.id).join("loft/target").exists(), "a running agent's build output was removed");
        assert!(!crate::agents::workspace(&st, &idle.id).join("loft/target").exists());
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

        run(&st, None, None, IDLE, None).await;
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
        let one = run(&st, None, Some(&a.id), IDLE, None).await;
        assert!(one.removed > 0);
        assert!(st.rt.lock().unwrap().clean.at.is_none(), "the last log is a deleted agent's");
        run(&st, Some("Daily sweep"), None, IDLE, None).await;
        assert!(st.rt.lock().unwrap().clean.at.is_some());
    }

    #[test]
    fn sizes_are_disk_blocks_like_the_freed_bytes() {
        let storage = temp();
        clone(&storage.join("a/loft"));
        let before = du(&storage.join("a/loft/target"));
        let r = sweep(&storage, None, &|_| false, later(), IDLE);
        let line = r.lines.iter().find(|l| l.path.ends_with("loft/target")).unwrap();
        assert_eq!(line.bytes, before);
        assert!(before >= 4096, "a 1-byte file takes a block: {before}");
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
    }

    #[test]
    fn a_worktree_counts_its_git_dir_s_changes() {
        let storage = temp();
        let ws = storage.join("a");
        clone(&ws.join("wt"));
        fs::remove_dir_all(ws.join("wt/.git")).unwrap();
        file(&ws.join("main/.git/worktrees/wt/HEAD"));
        fs::write(ws.join("wt/.git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
        assert!(git_time(&ws.join("wt")) > SystemTime::UNIX_EPOCH);
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
