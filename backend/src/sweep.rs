//! The sweep: deletes rebuildable build output (`target/`, `node_modules/`, `.build/`, `ios/build/`,
//! `DerivedData/`, `.cargo-build/`) from the clones in agents' workspaces that haven't changed in 3 days, so
//! they don't fill the disk. Source, git state and notes stay.
//!
//! It runs once a day, before Loft restarts into an update, and on a workspace whose agent was deleted with its
//! files kept. It skips running agents and agents with a live process (a dev server), never follows a symlink,
//! and never removes anything outside the agents' folder (`agent-local-storage/`), so Loft's own checkout stays.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::state::AppState;

/// A clone untouched this long loses its build output.
const IDLE: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// How deep in a workspace clones are looked for (`<workspace>/a/b/<clone>`).
const DEPTH: usize = 3;
/// How often the day's sweep runs.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long a measured size is reused: Settings and the delete dialog reload often, and a walk is slow.
const SIZE_FOR: Duration = Duration::from_secs(60);

/// What a sweep removed and how much it freed.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub at: Option<DateTime<Local>>,
    pub removed: Vec<PathBuf>,
    pub freed_bytes: u64,
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

/// `GET /api/workspaces`: every agent's workspace size, the disk's free space and what the last sweep freed.
pub async fn sizes(axum::extract::State(state): axum::extract::State<AppState>) -> crate::api::ApiResult<serde_json::Value> {
    let agents = state.db.list_agents()?;
    let storage = state.cfg.storage_dir.clone();
    let dirs: Vec<(String, String, PathBuf)> = agents.iter().map(|a| (a.id.clone(), a.name.clone(), crate::agents::workspace(&state, &a.id))).collect();
    let (rows, free) = tokio::task::spawn_blocking(move || {
        let rows: Vec<_> = dirs.into_iter().map(|(id, name, d)| serde_json::json!({ "id": id, "name": name, "bytes": size(&d) })).collect();
        (rows, free_bytes(&storage))
    })
    .await
    .unwrap_or_default();
    let last = state.rt.lock().unwrap().sweep.clone();
    Ok(axum::Json(serde_json::json!({ "agents": rows, "free_bytes": free, "low_disk_bytes": LOW_DISK, "last_sweep": last })))
}

/// `POST /api/workspaces/clean`: Clean now. Runs the sweep and answers what it freed.
pub async fn clean(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<Report> {
    axum::Json(run(&state, None).await)
}

/// One sweep at a time.
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Sweeps every workspace but those of running agents and agents with a live process, or just `only`'s.
/// Returns what it removed, and records it as the last sweep when it swept every workspace.
pub async fn run(state: &AppState, only: Option<&str>) -> Report {
    let _one = RUNNING.lock().await;
    let (st, all) = (state.clone(), only.is_none());
    let only = only.map(str::to_string);
    let r = tokio::task::spawn_blocking(move || {
        let storage = &st.cfg.storage_dir;
        let live = crate::processes::with_live_processes(storage);
        // Asked again for each clone, so a run that starts mid-sweep keeps its build output.
        let busy = |id: &str| live.contains(id) || st.rt.lock().unwrap().running.contains_key(id);
        sweep(storage, only.as_deref(), &busy, SystemTime::now())
    })
    .await
    .unwrap_or_default();
    tracing::info!("sweep freed {} bytes in {} folders: {:?}", r.freed_bytes, r.removed.len(), r.removed);
    *SIZES.lock().unwrap() = None;
    // One deleted agent's sweep isn't what Settings shows as the last one.
    if all {
        state.rt.lock().unwrap().sweep = r.clone();
    }
    r
}

/// The day's sweep, the first right after Loft starts.
pub async fn scheduler(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        run(&state, None).await;
    }
}

/// Sweeps the workspaces in `storage` (or only `only`'s) as of `now`, skipping the agents `busy` names.
pub fn sweep(storage: &Path, only: Option<&str>, busy: &dyn Fn(&str) -> bool, now: SystemTime) -> Report {
    let mut r = Report { at: Some(Local::now()), ..Default::default() };
    let Ok(root) = storage.canonicalize() else { return r };
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let id = e.file_name().to_string_lossy().to_string();
        // `.agent-files` and the like are Loft's; a symlinked workspace leads outside.
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if id.starts_with('.') || !is_dir || only.is_some_and(|o| o != id) || busy(&id) {
            continue;
        }
        workspace(&root, &e.path(), now, &|| busy(&id), &mut r);
    }
    r
}

fn workspace(root: &Path, ws: &Path, now: SystemTime, busy: &dyn Fn() -> bool, r: &mut Report) {
    let mut clones = vec![];
    find_clones(ws, DEPTH, &mut clones);
    let mut all_idle = true;
    for clone in clones {
        let mut builds = vec![];
        let mut newest = git_time(&clone);
        walk(&clone, &mut builds, &mut newest);
        if now.duration_since(newest).unwrap_or_default() < IDLE || busy() {
            all_idle = false;
            continue;
        }
        for b in builds {
            remove(root, &b, r);
        }
    }
    // Compiled dependencies all the workspace's clones share: kept while any of them is in use.
    if all_idle && !busy() {
        remove(root, &ws.join(".cargo-build"), r);
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

/// Collects the clone's build folders and the newest change to anything else in it, stopping at nested clones.
fn walk(dir: &Path, builds: &mut Vec<PathBuf>, newest: &mut SystemTime) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // Not followed: a symlink's own metadata.
        let Ok(meta) = e.metadata() else { continue };
        *newest = (*newest).max(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        if !meta.is_dir() || name == ".git" {
            continue;
        }
        let path = e.path();
        if name == ".build" || name == ".cargo-build" || is_build(dir, &name) {
            builds.push(path);
        } else if path.join(".git").symlink_metadata().is_err() {
            walk(&path, builds, newest);
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

/// Removes `path` when it is a real directory inside `root`, adding it and its size to the report.
fn remove(root: &Path, path: &Path, r: &mut Report) {
    if !path.symlink_metadata().is_ok_and(|m| m.is_dir()) || !path.canonicalize().is_ok_and(|p| p.starts_with(root)) {
        return;
    }
    let size = du(path);
    // Removes symlinks inside, not what they point at.
    match std::fs::remove_dir_all(path) {
        Ok(()) => {
            r.removed.push(path.to_path_buf());
            r.freed_bytes += size;
        }
        Err(e) => tracing::warn!("sweep: can't remove {}: {e}", path.display()),
    }
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

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_lists_workspace_sizes_and_free_disk_and_clean_now_reports_what_it_freed() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let ws = crate::agents::workspace(&st, &a.id);
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("big.bin"), vec![0u8; 5000]).unwrap();
        let axum::Json(v) = sizes(axum::extract::State(st.clone())).await.unwrap();
        let row = v["agents"].as_array().unwrap().iter().find(|r| r["id"] == a.id.as_str()).unwrap();
        assert!(row["bytes"].as_u64().unwrap() >= 5000, "{row}");
        assert!(v["free_bytes"].as_u64().unwrap() > 0);
        // A clone built just now isn't idle: nothing freed, and the report is what Settings shows after.
        clone(&ws.join("loft"));
        let axum::Json(r) = clean(axum::extract::State(st.clone())).await;
        assert_eq!(r.freed_bytes, 0);
        assert!(r.at.is_some());
        let axum::Json(v) = sizes(axum::extract::State(st)).await.unwrap();
        assert!(v["last_sweep"]["at"].is_string());
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

        let r = sweep(&storage, None, &|_| false, later());

        for gone in ["loft/target", "loft/frontend/node_modules", "loft/ios/build", "loft/ios/DerivedData", "loft/Pkg/.build", ".cargo-build"] {
            assert!(!ws.join(gone).exists(), "{gone} stays");
            assert!(r.removed.contains(&ws.join(gone)), "{gone} not reported");
        }
        for kept in ["loft/.git/HEAD", "loft/Cargo.toml", "loft/src/main.rs", "loft/ios/App/App.swift", "loft/docs/target/keep.md", "notes/plan.md", "scratch/node_modules/keep.js"] {
            assert!(ws.join(kept).exists(), "{kept} removed");
        }
        assert_eq!(r.removed.len(), 6);
        assert!(r.freed_bytes > 0);
    }

    /// Dates everything in `dir` 4 days back.
    fn age(dir: &Path) {
        let then = SystemTime::now() - Duration::from_secs(4 * 24 * 60 * 60);
        for e in fs::read_dir(dir).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                age(&e.path());
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

        let r = sweep(&storage, None, &|id| id == "running", SystemTime::now());

        assert!(!storage.join("idle/old/target").exists(), "an idle clone keeps its build output");
        assert!(storage.join("idle/fresh/target").exists(), "a fresh clone lost its build output");
        assert!(storage.join("running/loft/target").exists(), "a running agent's build output was removed");
        assert!(r.removed.iter().all(|p| p.starts_with(storage.join("idle/old"))), "{:?}", r.removed);
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
        let r = sweep(&storage, None, &busy, later());
        assert!(r.removed.is_empty(), "{:?}", r.removed);
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

        run(&st, None).await;
        let _ = dev.kill();
        let _ = dev.wait();

        assert!(crate::agents::workspace(&st, &server.id).join("loft/target").exists(), "a dev server's agent lost its build output");
        assert!(!crate::agents::workspace(&st, &idle.id).join("loft/target").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_agent_s_sweep_leaves_the_last_sweep_report_alone() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Gone");
        let ws = crate::agents::workspace(&st, &a.id);
        clone(&ws.join("loft"));
        age(&ws);
        let one = run(&st, Some(&a.id)).await;
        assert!(!one.removed.is_empty());
        assert!(st.rt.lock().unwrap().sweep.at.is_none(), "the last sweep is one agent's");
        run(&st, None).await;
        assert!(st.rt.lock().unwrap().sweep.at.is_some());
    }

    #[test]
    fn sizes_are_disk_blocks_like_the_freed_bytes() {
        let storage = temp();
        clone(&storage.join("a/loft"));
        let target = storage.join("a/loft/target");
        let before = size(&target);
        let mut r = Report::default();
        remove(&storage, &target, &mut r);
        assert_eq!(r.freed_bytes, before);
        assert!(before >= 4096, "a 1-byte file takes a block: {before}");
    }

    #[test]
    fn one_workspace_alone_is_swept_when_named() {
        let storage = temp();
        clone(&storage.join("a/loft"));
        clone(&storage.join("b/loft"));
        sweep(&storage, Some("a"), &|_| false, later());
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

        let r = sweep(&storage, None, &|_| false, later());

        assert!(!ws.join("app/target").exists(), "the real target/ is removed");
        assert!(ws.join("app/web/node_modules").symlink_metadata().is_ok(), "a symlinked node_modules is left alone");
        for kept in ["target/debug/app", "frontend/node_modules/vite/index.js", "ios/build/App.app/App"] {
            assert!(loft.join(kept).exists(), "Loft's checkout lost {kept}");
        }
        assert!(outside.join("lib/index.js").exists(), "a symlink target was removed");
        assert!(r.removed.iter().all(|p| p.starts_with(&storage)), "{:?}", r.removed);
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
