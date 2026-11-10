//! The sweep: deletes rebuildable build output (`target/`, `node_modules/`, `.build/`, `ios/build/`,
//! `DerivedData/`, `.cargo-build/`) from the clones in agents' workspaces that haven't changed in 3 days, so
//! they don't fill the disk. Source, git state and notes stay.
//!
//! It runs once a day, before Loft restarts into an update, and on a workspace whose agent was deleted with its
//! files kept. It skips running agents and agents with a live process (a dev server), never follows a symlink,
//! and never removes anything outside the agents' folder (`agent-local-storage/`), so Loft's own checkout stays.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::state::AppState;

/// A clone untouched this long loses its build output.
const IDLE: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// How deep in a workspace clones are looked for (`<workspace>/a/b/<clone>`).
const DEPTH: usize = 3;
/// How often the day's sweep runs.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// What a sweep removed and how much it freed.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub at: Option<DateTime<Local>>,
    pub removed: Vec<PathBuf>,
    pub freed_bytes: u64,
}

/// One sweep at a time.
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Sweeps every workspace but those of running agents and agents with a live process, or just `only`'s.
/// Records and returns what it removed.
pub async fn run(state: &AppState, only: Option<&str>) -> Report {
    let _one = RUNNING.lock().await;
    let storage = state.cfg.storage_dir.clone();
    let running: HashSet<String> = state.rt.lock().unwrap().running.keys().cloned().collect();
    let only = only.map(str::to_string);
    let r = tokio::task::spawn_blocking(move || {
        let mut skip = running;
        skip.extend(crate::processes::with_live_processes(&storage));
        sweep(&storage, only.as_deref(), &skip, SystemTime::now())
    })
    .await
    .unwrap_or_default();
    tracing::info!("sweep freed {} bytes in {} folders: {:?}", r.freed_bytes, r.removed.len(), r.removed);
    state.rt.lock().unwrap().sweep = r.clone();
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

/// Sweeps the workspaces in `storage` (or only `only`'s), skipping the agent ids in `skip`, as of `now`.
pub fn sweep(storage: &Path, only: Option<&str>, skip: &HashSet<String>, now: SystemTime) -> Report {
    let mut r = Report { at: Some(Local::now()), ..Default::default() };
    let Ok(root) = storage.canonicalize() else { return r };
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let id = e.file_name().to_string_lossy().to_string();
        // `.agent-files` and the like are Loft's; a symlinked workspace leads outside.
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if id.starts_with('.') || !is_dir || skip.contains(&id) || only.is_some_and(|o| o != id) {
            continue;
        }
        workspace(&root, &e.path(), now, &mut r);
    }
    r
}

fn workspace(root: &Path, ws: &Path, now: SystemTime, r: &mut Report) {
    let mut clones = vec![];
    find_clones(ws, DEPTH, &mut clones);
    let mut all_idle = true;
    for clone in clones {
        let mut builds = vec![];
        let mut newest = git_time(&clone);
        walk(&clone, &mut builds, &mut newest);
        if now.duration_since(newest).unwrap_or_default() < IDLE {
            all_idle = false;
            continue;
        }
        for b in builds {
            remove(root, &b, r);
        }
    }
    // Compiled dependencies all the workspace's clones share: kept while any of them is in use.
    if all_idle {
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

        let r = sweep(&storage, None, &HashSet::new(), later());

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

        let r = sweep(&storage, None, &HashSet::from(["running".to_string()]), SystemTime::now());

        assert!(!storage.join("idle/old/target").exists(), "an idle clone keeps its build output");
        assert!(storage.join("idle/fresh/target").exists(), "a fresh clone lost its build output");
        assert!(storage.join("running/loft/target").exists(), "a running agent's build output was removed");
        assert!(r.removed.iter().all(|p| p.starts_with(storage.join("idle/old"))), "{:?}", r.removed);
    }

    #[test]
    fn one_workspace_alone_is_swept_when_named() {
        let storage = temp();
        clone(&storage.join("a/loft"));
        clone(&storage.join("b/loft"));
        sweep(&storage, Some("a"), &HashSet::new(), later());
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

        let r = sweep(&storage, None, &HashSet::new(), later());

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
