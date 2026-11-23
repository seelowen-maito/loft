//! Self-update: with auto-update on in Settings, poll the checkout's upstream branch and,
//! once no agent is running, pull, rebuild and restart into the new binary.
//!
//! On Windows, Loft restarts by exiting with `RESTART_CODE` (it can't replace a running process with `exec`): whatever
//! runs it starts it again, the logon task's scripts/run-service.sh or start.sh's scripts/run-backend.sh. Both export
//! `LOFT_RESTART_BY_EXIT=1`, and under them Loft exits that way on every system. Elsewhere it `exec`s the build on
//! disk, as before: an older start.sh ran `exec cargo run`, with no loop to start Loft again, and launchd's service
//! keeps its process.
//!
//! Windows can't write over a running binary either, but it can rename one: before building, an update sets the
//! running build aside as `loft.old.exe` (always that name, replaced by the next update), so cargo can write the new
//! one. The new Loft removes it once it's up, and the daily sweep tries again if it was still in use.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Result, bail};
use serde::Serialize;
use tokio::process::Command;

use crate::state::{AppState, Event};

const CHECK_EVERY: Duration = Duration::from_secs(60);
const IDLE_POLL: Duration = Duration::from_secs(5);
/// The exit code asking whatever runs Loft to start it again. Keep in step with scripts/run-backend.sh.
pub const RESTART_CODE: i32 = 75;
/// How soon turning auto-update on in Settings takes effect.
const OFF_POLL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Disabled,
    UpToDate,
    /// A newer commit exists; waiting for running agents to finish.
    Waiting,
    Updating,
    /// A newer build is on disk than the one running; a restart starts it.
    Restart,
    Error,
    /// The checkout can't be fast-forwarded (detached, no upstream, local commits, failed fetch); `why` says so.
    Blocked,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UpdateStatus {
    pub phase: Phase,
    /// Short sha of the running checkout.
    pub current: String,
    /// Short sha of the upstream commit, when it is ahead.
    pub latest: Option<String>,
    pub running_agents: usize,
    pub error: Option<String>,
    /// Why a restart hasn't happened, e.g. "2 agents are running", or why the checkout can't update.
    pub why: Option<String>,
}

/// The commit this binary was built from (empty when it was built outside a git checkout).
pub const BUILT_FROM: &str = env!("LOFT_COMMIT");

/// Whether the build on disk was made after this process started, so it isn't the one running.
pub fn newer_build_on_disk(started: SystemTime) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    std::fs::metadata(crate::signing::build_of(&exe)).and_then(|m| m.modified()).is_ok_and(|t| t > started)
}

/// True while an update is queued, being installed or about to be restarted into; the scheduler holds
/// autonomous wake-ups then.
pub fn pending(state: &AppState) -> bool {
    matches!(state.update.lock().unwrap().phase, Phase::Waiting | Phase::Updating | Phase::Restart)
}

fn set(state: &AppState, f: impl FnOnce(&mut UpdateStatus)) {
    let snapshot = {
        let mut s = state.update.lock().unwrap();
        let before = s.clone();
        f(&mut s);
        if *s == before {
            return;
        }
        s.clone()
    };
    state.emit(Event::Update { status: serde_json::to_value(snapshot).unwrap_or_default() });
}

fn running_agents(state: &AppState) -> usize {
    state.rt.lock().unwrap().running.len()
}

async fn run(dir: &Path, program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program).args(args).current_dir(dir).env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0").output().await?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = err.lines().collect();
        // Lead with the first error (cargo ends with hints); else show the tail.
        let start = lines.iter().position(|l| l.starts_with("error")).unwrap_or(lines.len().saturating_sub(6));
        bail!("`{program} {}` failed: {}", args.join(" "), lines[start..].iter().take(6).copied().collect::<Vec<_>>().join("\n"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Upstream commit when it is strictly ahead of the running build and fast-forwardable.
/// Compares against `running` rather than HEAD: a failed build leaves HEAD pulled but the old binary live.
/// `Err` is why the checkout can't be updated, worded for the Update status.
async fn check(root: &Path, running: &str) -> Result<Option<String>, String> {
    let Ok(branch) = run(root, "git", &["symbolic-ref", "--quiet", "--short", "HEAD"]).await else {
        let at = run(root, "git", &["rev-parse", "--short=8", "HEAD"]).await.unwrap_or_default();
        let main = run(root, "git", &["rev-parse", "--abbrev-ref", "origin/HEAD"]).await;
        let main = main.as_deref().ok().and_then(|m| m.strip_prefix("origin/")).unwrap_or("main");
        return Err(format!("Loft's checkout is on commit {at}, not on {main}. Switch it back to {main} to get updates."));
    };
    let Ok(upstream_name) = run(root, "git", &["rev-parse", "--abbrev-ref", "@{u}"]).await else {
        return Err(format!("Loft's checkout is on branch {branch}, which has no upstream. Set one (git branch -u origin/{branch}) or switch back to main to get updates."));
    };
    if let Err(e) = run(root, "git", &["fetch", "--quiet"]).await {
        return Err(format!("Couldn't fetch updates: {e}"));
    }
    let upstream = run(root, "git", &["rev-parse", "@{u}"]).await.map_err(|e| e.to_string())?;
    let local = run(root, "git", &["rev-list", "--count", "@{u}..HEAD"]).await.map_err(|e| e.to_string())?;
    if local != "0" {
        let s = if local == "1" { "" } else { "s" };
        return Err(format!("Loft's checkout has {local} commit{s} on {branch} that aren't on {upstream_name}. Push or drop them to get updates."));
    }
    if upstream == running {
        return Ok(None);
    }
    if run(root, "git", &["merge-base", "--is-ancestor", running, &upstream]).await.is_err() {
        // The running build isn't on the upstream: nothing to fast-forward to.
        return Ok(None);
    }
    Ok(Some(upstream))
}

/// The status while the checkout can't be updated. It clears on the first check that passes.
fn blocked(state: &AppState, head: &str, why: String) {
    set(state, |s| *s = UpdateStatus { phase: Phase::Blocked, current: short(head), why: Some(why), ..Default::default() });
}

/// The pinned pnpm (`packageManager` in frontend/package.json): an installed pnpm, else corepack's, else fetched
/// through `npm exec`. Never plain `npm install`: that would resolve the dependencies fresh, with no lockfile.
/// Keep in step with scripts/frontend-pm.sh.
async fn pnpm(dir: &Path) -> Result<(&'static str, Vec<String>)> {
    for (program, pre) in [("pnpm", &[][..]), ("corepack", &["pnpm"][..])] {
        if run(dir, program, &[pre, &["--version"]].concat()).await.is_ok() {
            return Ok((program, pre.iter().map(|s| s.to_string()).collect()));
        }
    }
    let pkg: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("package.json"))?)?;
    let version = pkg["packageManager"].as_str().and_then(|p| p.strip_prefix("pnpm@")).and_then(|p| p.split('+').next());
    let Some(version) = version.filter(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.')) else {
        bail!("frontend/package.json has no pinned pnpm in `packageManager`");
    };
    Ok(("npm", ["exec", "--yes", &format!("pnpm@{version}"), "--"].map(String::from).to_vec()))
}

/// node_modules made by npm: pnpm leaves a `.modules.yaml` there, npm doesn't. pnpm won't reuse npm's without a
/// terminal, and the updater has none, so it starts over.
fn stale_modules(has_modules_yaml: bool) -> bool {
    !has_modules_yaml
}

async fn install(root: &Path) -> Result<()> {
    run(root, "git", &["pull", "--ff-only", "--quiet"]).await?;
    // Backend first: if it fails, the live frontend bundle stays in step with the running binary.
    let mut cargo = vec!["build"];
    if !cfg!(debug_assertions) {
        cargo.push("--release");
    }
    let aside = match running_build() {
        Some(exe) if cfg!(windows) => set_aside(&exe).map_err(|e| anyhow::anyhow!("Couldn't set the running build aside: {e}"))?,
        _ => SetAside::default(),
    };
    let built = run(&root.join("backend"), "cargo", &cargo).await;
    // A failed build leaves the running build where a restart finds it.
    aside.restore();
    built?;
    let frontend = root.join("frontend");
    let (pm, pre) = pnpm(&frontend).await?;
    let pre: Vec<&str> = pre.iter().map(String::as_str).collect();
    let modules = frontend.join("node_modules");
    if modules.is_dir() && stale_modules(modules.join(".modules.yaml").is_file()) {
        std::fs::remove_dir_all(&modules)?;
    }
    run(&frontend, pm, &[&pre[..], &["install", "--frozen-lockfile", "--ignore-workspace"]].concat()).await?;
    run(&frontend, pm, &[&pre[..], &["run", "build"]].concat()).await?;
    // The Herdr UI client, once it was set up: rebuilt so new chat tabs run the new version.
    let herdr = root.join("herdr");
    if herdr.join("target").join("release").join("loft-herdr").is_file()
        && let Err(e) = run(&herdr, "cargo", &["build", "--release", "--quiet"]).await
    {
        tracing::warn!("herdr ui client: {e}");
    }
    // The voice transcriber (spec #493), so `POST /api/voice/transcribe` runs the new version.
    if cfg!(target_os = "macos")
        && let Err(e) = run(&root.join("ios/LoftKit"), "swift", &["build", "-c", "release", "--product", "loft-transcribe"]).await
    {
        tracing::warn!("voice transcriber: {e}");
    }
    Ok(())
}

/// The build this process runs (the signed copy's build on the Mac): what an update replaces.
fn running_build() -> Option<PathBuf> {
    std::env::current_exe().ok().map(|exe| crate::signing::build_of(&exe))
}

/// Where an update sets `exe` aside: `loft.old.exe` beside `loft.exe`.
fn old_of(exe: &Path) -> PathBuf {
    let mut name = exe.file_stem().unwrap_or_default().to_os_string();
    name.push(".old");
    if let Some(ext) = exe.extension() {
        name.push(".");
        name.push(ext);
    }
    exe.with_file_name(name)
}

/// Cargo's other name for `exe`: the file in `deps/` it was hard-linked from (`deps/loft-<hash>.exe`), which the
/// linker writes. Each moves to its own `loft.old.exe`, both names of the one set-aside file.
fn names_of(exe: &Path) -> Vec<PathBuf> {
    let mut names = vec![exe.to_path_buf()];
    let (Some(dir), Some(stem)) = (exe.parent(), exe.file_stem()) else { return names };
    let prefix = format!("{}-", stem.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(dir.join("deps")) else { return names };
    names.extend(
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension() == exe.extension() && p.file_stem().is_some_and(|s| s.to_string_lossy().starts_with(&prefix)))
            .filter(|p| same_file(p, exe)),
    );
    names
}

/// Whether `a` and `b` are names of one file (hard links).
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => (a.dev(), a.ino()) == (b.dev(), b.ino()),
        _ => false,
    }
}

#[cfg(windows)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    let id = |p: &Path| {
        let f = std::fs::File::open(p).ok()?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        (unsafe { GetFileInformationByHandle(f.as_raw_handle(), &mut info) } != 0).then_some((info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
    };
    id(a).is_some_and(|a| id(b) == Some(a))
}

/// The names an update moved aside, as (name, set-aside name).
#[derive(Debug, Default)]
struct SetAside(Vec<(PathBuf, PathBuf)>);

impl SetAside {
    /// Moves back each name the build didn't write again, so a failed build leaves the running build in place.
    fn restore(self) {
        for (name, old) in self.0.into_iter().rev() {
            if !name.exists()
                && let Err(e) = std::fs::rename(&old, &name)
            {
                tracing::warn!("couldn't put {} back: {e}", name.display());
            }
        }
    }
}

/// Renames the running build `exe`, under each of its names, to `loft.old.exe`, replacing what an earlier update
/// left there, so cargo can write the new build. Windows refuses to write or delete a running binary, not to
/// rename it. On an error, what moved is moved back.
fn set_aside(exe: &Path) -> std::io::Result<SetAside> {
    let (mut moved, old_name) = (SetAside::default(), old_of(exe).file_name().unwrap_or_default().to_os_string());
    for name in names_of(exe) {
        let old = name.with_file_name(&old_name);
        if let Err(e) = std::fs::rename(&name, &old) {
            moved.restore();
            return Err(e);
        }
        moved.0.push((name, old));
    }
    Ok(moved)
}

/// Removes what an update set aside beside `exe`. False while a name is still there (in use: Windows won't delete
/// a running binary).
fn remove_set_aside(exe: &Path) -> bool {
    let deps = exe.parent().map(|d| d.join("deps").join(old_of(exe).file_name().unwrap_or_default()));
    let mut gone = true;
    for old in std::iter::once(old_of(exe)).chain(deps) {
        match std::fs::remove_file(&old) {
            Ok(()) => tracing::info!("removed {}", old.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!("couldn't remove {}: {e}", old.display());
                gone = false;
            }
        }
    }
    gone
}

/// Removes the build the last update set aside. Run as Loft starts: the old process has exited by then, but a
/// virus scanner may still hold the file, so it tries for a minute; the daily sweep tries again after that.
pub async fn clear_set_aside() {
    let Some(exe) = running_build() else { return };
    for _ in 0..12 {
        if remove_set_aside(&exe) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// The daily sweep's try at a set-aside build that was still in use when Loft started.
pub fn sweep_set_aside() {
    if let Some(exe) = running_build() {
        remove_set_aside(&exe);
    }
}

/// Wait until no agent runs. False if auto-update was turned off meanwhile.
async fn wait_for_idle(state: &AppState) -> bool {
    loop {
        if !state.settings.get().auto_update {
            return false;
        }
        let n = running_agents(state);
        set(state, |s| s.running_agents = n);
        if n == 0 {
            return true;
        }
        tokio::time::sleep(IDLE_POLL).await;
    }
}

/// Once the update is built: waits for agents started by hand during the build (the new build is on disk, so
/// Loft restarts into it even if auto-update was turned off meanwhile), then sweeps.
async fn before_restart(state: &AppState) {
    while running_agents(state) > 0 {
        tokio::time::sleep(IDLE_POLL).await;
    }
    crate::sweep::run(state, Some("Sweep before update"), None, crate::sweep::IDLE).await;
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

pub async fn updater(state: AppState, root: PathBuf) {
    let started = SystemTime::now();
    // The commit this process was built from, not the checkout's HEAD: a pull or a build the restart never
    // followed leaves HEAD ahead of the running binary.
    let built = if BUILT_FROM.is_empty() { run(&root, "git", &["rev-parse", "HEAD"]).await.map_err(|e| e.to_string()) } else { Ok(BUILT_FROM.to_string()) };
    // A commit whose build failed, and why; skipped until upstream moves on.
    let mut failed: Option<(String, String)> = None;
    loop {
        if !state.settings.get().auto_update {
            set(&state, |s| *s = UpdateStatus::default());
            tokio::time::sleep(OFF_POLL).await;
            continue;
        }
        let head = match &built {
            Ok(h) => h.clone(),
            Err(e) => {
                set(&state, |s| {
                    s.phase = Phase::Error;
                    s.error = Some(e.clone());
                });
                tokio::time::sleep(CHECK_EVERY).await;
                continue;
            }
        };
        if newer_build_on_disk(started) {
            let n = running_agents(&state);
            set(&state, |s| *s = restart_status(&head, n));
            // Restart by itself once agents are idle, as an update does.
            if n == 0 {
                before_restart(&state).await;
                restart();
            }
            tokio::time::sleep(IDLE_POLL).await;
            continue;
        }
        if let Some(up) = checked(&state, &head, check(&root, &head).await, failed.as_ref()) {
            if !wait_for_idle(&state).await {
                continue;
            }
            set(&state, |s| s.phase = Phase::Updating);
            tracing::info!("updating {} -> {}", short(&head), short(&up));
            match install(&root).await {
                Ok(()) => {
                    before_restart(&state).await;
                    restart();
                }
                Err(e) => {
                    tracing::error!("update failed: {e:#}");
                    set(&state, |s| {
                        s.phase = Phase::Error;
                        s.error = Some(e.to_string());
                    });
                    failed = Some((up, e.to_string()));
                }
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

/// Sets the status for one check's result, and returns the commit to install. A commit whose build failed isn't
/// installed again: its error shows again, also after the checkout was blocked meanwhile.
fn checked(state: &AppState, head: &str, result: Result<Option<String>, String>, failed: Option<&(String, String)>) -> Option<String> {
    match result {
        Err(why) => blocked(state, head, why),
        Ok(None) => set(state, |s| *s = UpdateStatus { phase: Phase::UpToDate, current: short(head), ..Default::default() }),
        Ok(Some(up)) => match failed.filter(|(f, _)| *f == up) {
            Some((_, error)) => set(state, |s| {
                *s = UpdateStatus { phase: Phase::Error, current: short(head), latest: Some(short(&up)), error: Some(error.clone()), ..Default::default() }
            }),
            None => {
                set(state, |s| *s = UpdateStatus { phase: Phase::Waiting, current: short(head), latest: Some(short(&up)), ..Default::default() });
                return Some(up);
            }
        },
    }
    None
}

/// The status while a newer build waits on disk. The loop doesn't check upstream in this phase (the commit it
/// would compare is the old one, so it would rebuild what's already on disk): it restarts when idle, and the new
/// process checks.
fn restart_status(head: &str, running_agents: usize) -> UpdateStatus {
    let why = (running_agents > 0).then(|| format!("{running_agents} agent{} running; restarts when they finish", if running_agents == 1 { " is" } else { "s are" }));
    UpdateStatus { phase: Phase::Restart, current: short(head), running_agents, why, ..Default::default() }
}

/// Start the build on disk in this process's place: after an update, the new build, which signs its own copy again
/// (a stale signed copy runs the build first, see signing.rs). Exits with `RESTART_CODE` for a script that runs Loft
/// again, and otherwise `exec`s the build with the same arguments.
pub fn restart() -> ! {
    if !restarts_by_exit(cfg!(windows), std::env::var_os("LOFT_RESTART_BY_EXIT").as_deref()) {
        tracing::info!("restarting: replacing this process with the build on disk");
        match std::env::current_exe() {
            Ok(exe) => {
                let err = crate::sys::exec(std::process::Command::new(crate::signing::build_of(&exe)).args(std::env::args_os().skip(1)));
                tracing::error!("restart failed: {err}");
            }
            Err(err) => tracing::error!("restart failed: {err}"),
        }
    }
    tracing::info!("restarting: exiting with code {RESTART_CODE}");
    std::process::exit(RESTART_CODE)
}

/// Whether a restart exits with `RESTART_CODE` rather than `exec`ing: always on Windows, and wherever
/// scripts/run-backend.sh or scripts/run-service.sh run Loft (they set `LOFT_RESTART_BY_EXIT=1`).
fn restarts_by_exit(windows: bool, var: Option<&std::ffi::OsStr>) -> bool {
    windows || var.is_some_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_running_commit_is_the_one_built_in() {
        if super::BUILT_FROM.is_empty() {
            return;
        }
        // Built from a checkout, so the embedded commit is a full sha; it's what `current` shows (7 chars).
        assert_eq!(super::BUILT_FROM.len(), 40, "{:?}", super::BUILT_FROM);
        assert_eq!(super::short(super::BUILT_FROM).len(), 7);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wake_ups_are_held_until_the_restart() {
        let st = crate::tasks::tests::state();
        let phase = |p: super::Phase, why: Option<&str>| {
            *st.update.lock().unwrap() = super::UpdateStatus { phase: p, why: why.map(Into::into), ..Default::default() };
            super::pending(&st)
        };
        assert!(phase(super::Phase::Restart, None));
        assert!(phase(super::Phase::Restart, Some("1 agent is running; restarts when they finish")));
        assert!(!phase(super::Phase::UpToDate, None));
    }

    #[test]
    fn the_restart_status_says_what_it_waits_for() {
        assert_eq!(super::restart_status("abcdef012", 0).why, None);
        assert_eq!(super::restart_status("abcdef012", 1).why.unwrap(), "1 agent is running; restarts when they finish");
        assert!(super::restart_status("abcdef012", 2).why.unwrap().starts_with("2 agents are"));
    }

    #[test]
    fn a_restart_execs_unless_windows_or_a_script_starts_loft_again() {
        use std::ffi::OsStr;
        assert!(!super::restarts_by_exit(false, None));
        assert!(!super::restarts_by_exit(false, Some(OsStr::new("0"))));
        assert!(super::restarts_by_exit(false, Some(OsStr::new("1"))));
        assert!(super::restarts_by_exit(true, None));
        assert!(super::restarts_by_exit(true, Some(OsStr::new("1"))));
    }

    /// Runs `restart` in a copy of this test binary, since it ends the process.
    #[test]
    fn a_restart_exits_with_the_restart_code() {
        if std::env::var_os("LOFT_TEST_RESTART").is_some() {
            super::restart();
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "updater::tests::a_restart_exits_with_the_restart_code", "--test-threads=1"])
            .env("LOFT_TEST_RESTART", "1")
            .env("LOFT_RESTART_BY_EXIT", "1")
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(super::RESTART_CODE));
    }

    /// start.sh's loop with a fake cargo that exits with the given codes in turn: how many times it ran, the
    /// `LOFT_RESTART_BY_EXIT` and args it got, and the script's own exit code.
    #[cfg(unix)]
    fn run_backend(codes: &[i32]) -> (usize, String, Option<i32>) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("loft-run-backend-{}-{}", std::process::id(), codes.len()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let codes = codes.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
        let cargo = dir.join("cargo");
        std::fs::write(
            &cargo,
            format!("#!/bin/sh\necho \"$LOFT_RESTART_BY_EXIT $*\" >> '{d}/runs'\nn=$(wc -l < '{d}/runs')\nset -- {codes}\nshift $((n - 1))\nexit \"$1\"\n", d = dir.display()),
        )
        .unwrap();
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/run-backend.sh");
        let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default());
        let status = std::process::Command::new("bash").arg(script).arg("--release").env("PATH", path).env_remove("LOFT_RESTART_BY_EXIT").stderr(std::process::Stdio::null()).status().unwrap();
        let runs = std::fs::read_to_string(dir.join("runs")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        (runs.lines().count(), runs.lines().next().unwrap_or_default().to_string(), status.code())
    }

    #[cfg(unix)]
    #[test]
    fn start_sh_runs_loft_again_after_a_restart_and_ends_on_any_other_exit() {
        let code = super::RESTART_CODE;
        assert_eq!(run_backend(&[code, code, 0]), (3, "1 run --release".into(), Some(0)));
        assert_eq!(run_backend(&[1]), (1, "1 run --release".into(), Some(1)), "an error ends it instead of restarting");
        assert_eq!(run_backend(&[code, 101, 0, 0]), (2, "1 run --release".into(), Some(101)));
    }

    /// A cargo output folder: `release/loft.exe`, hard-linked from `release/deps/loft-abc.exe` as cargo leaves it, and
    /// another build's `deps/loft-def.exe`.
    fn cargo_output(tmp: &std::path::Path) -> std::path::PathBuf {
        let release = tmp.join("release");
        std::fs::create_dir_all(release.join("deps")).unwrap();
        std::fs::write(release.join("deps/loft-abc.exe"), "running").unwrap();
        std::fs::hard_link(release.join("deps/loft-abc.exe"), release.join("loft.exe")).unwrap();
        std::fs::write(release.join("deps/loft-def.exe"), "other build").unwrap();
        std::fs::write(release.join("deps/libloft-abc.rlib"), "lib").unwrap();
        release.join("loft.exe")
    }

    fn read(p: std::path::PathBuf) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    #[test]
    fn an_update_sets_the_running_build_aside_as_loft_old_under_both_its_names() {
        let tmp = crate::testdir::TempDir::new("set-aside");
        let exe = cargo_output(&tmp);
        let release = exe.parent().unwrap().to_path_buf();
        // What the last update set aside, if it was never removed: replaced, so there's only ever one.
        std::fs::write(release.join("loft.old.exe"), "older").unwrap();
        let aside = super::set_aside(&exe).unwrap();
        assert!(!exe.exists() && !release.join("deps/loft-abc.exe").exists(), "cargo can write both names");
        assert_eq!(read(release.join("loft.old.exe")), "running");
        assert_eq!(read(release.join("deps/loft.old.exe")), "running");
        assert_eq!(read(release.join("deps/loft-def.exe")), "other build", "another build's binary stays");
        assert_eq!(read(release.join("deps/libloft-abc.rlib")), "lib");
        // A build that worked wrote both names again: nothing goes back.
        std::fs::write(release.join("deps/loft-abc.exe"), "new").unwrap();
        std::fs::write(&exe, "new").unwrap();
        aside.restore();
        assert_eq!(read(exe.clone()), "new");
        assert_eq!(read(release.join("loft.old.exe")), "running");
        let olds = std::fs::read_dir(&release).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains(".old")).count();
        assert_eq!(olds, 1);
    }

    #[test]
    fn a_failed_build_puts_the_running_build_back() {
        let tmp = crate::testdir::TempDir::new("set-aside");
        let exe = cargo_output(&tmp);
        let release = exe.parent().unwrap().to_path_buf();
        super::set_aside(&exe).unwrap().restore();
        assert_eq!(read(exe.clone()), "running", "a restart runs the build that was running");
        assert_eq!(read(release.join("deps/loft-abc.exe")), "running");
        assert!(super::same_file(&exe, &release.join("deps/loft-abc.exe")));
        assert!(!release.join("loft.old.exe").exists() && !release.join("deps/loft.old.exe").exists());
    }

    #[test]
    fn the_new_loft_removes_the_set_aside_build() {
        let tmp = crate::testdir::TempDir::new("set-aside");
        let exe = cargo_output(&tmp);
        let release = exe.parent().unwrap().to_path_buf();
        super::set_aside(&exe).unwrap();
        std::fs::write(&exe, "new").unwrap();
        assert!(super::remove_set_aside(&exe));
        assert!(!release.join("loft.old.exe").exists() && !release.join("deps/loft.old.exe").exists());
        assert_eq!(read(exe.clone()), "new");
        assert!(super::remove_set_aside(&exe), "nothing left to remove");
    }

    /// Stands in for a running Loft in the Windows test below.
    #[test]
    fn a_running_build() {
        if std::env::var_os("LOFT_TEST_RUNNING_BUILD").is_some() {
            std::thread::sleep(std::time::Duration::from_secs(120));
        }
    }

    /// Windows itself: a running binary can't be written over or deleted, but set aside it can, and it's removed
    /// once it has exited.
    #[cfg(windows)]
    #[test]
    fn on_windows_a_running_build_is_set_aside_written_over_and_removed_once_it_exits() {
        let tmp = crate::testdir::TempDir::new("set-aside-running");
        let exe = cargo_output(&tmp);
        let release = exe.parent().unwrap().to_path_buf();
        std::fs::remove_file(&exe).unwrap();
        std::fs::remove_file(release.join("deps/loft-abc.exe")).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), release.join("deps/loft-abc.exe")).unwrap();
        std::fs::hard_link(release.join("deps/loft-abc.exe"), &exe).unwrap();
        let mut child = std::process::Command::new(&exe)
            .args(["--exact", "updater::tests::a_running_build", "--test-threads=1"])
            .env("LOFT_TEST_RUNNING_BUILD", "1")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(1));
        assert!(std::fs::write(release.join("deps/loft-abc.exe"), "new").is_err(), "Windows refuses to write a running binary");
        super::set_aside(&exe).unwrap();
        std::fs::write(release.join("deps/loft-abc.exe"), "new").unwrap();
        std::fs::write(&exe, "new").unwrap();
        assert!(!super::remove_set_aside(&exe), "still running: left for the sweep");
        // Windows may let one name of the running file go, never both.
        assert!(release.join("loft.old.exe").exists() || release.join("deps/loft.old.exe").exists());
        child.kill().unwrap();
        child.wait().unwrap();
        // The process can take a moment to let go of its file.
        let gone = (0..20).any(|_| super::remove_set_aside(&exe) || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            false
        });
        assert!(gone);
        assert!(!release.join("loft.old.exe").exists() && !release.join("deps/loft.old.exe").exists());
        assert_eq!(read(exe), "new");
    }

    #[test]
    fn a_build_made_after_start_is_a_newer_build() {
        use std::time::{Duration, SystemTime};
        assert!(!super::newer_build_on_disk(SystemTime::now() + Duration::from_secs(3600)));
        assert!(super::newer_build_on_disk(SystemTime::UNIX_EPOCH));
    }

    #[test]
    fn node_modules_made_by_npm_is_stale() {
        assert!(super::stale_modules(false));
        assert!(!super::stale_modules(true));
    }

    use std::time::{Duration, SystemTime};

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_sweeps_before_restarting() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let ws = crate::agents::workspace(&st, &a.id);
        // A real clone: the sweep asks git whether it tracks the folder.
        std::fs::create_dir_all(ws.join("loft")).unwrap();
        assert!(std::process::Command::new("git").arg("-C").arg(ws.join("loft")).args(["init", "-q"]).env("GIT_CONFIG_GLOBAL", "/dev/null").status().unwrap().success());
        std::fs::write(ws.join("loft/Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(ws.join("loft/target/debug")).unwrap();
        std::fs::write(ws.join("loft/target/debug/app"), "x").unwrap();
        // Untouched for 4 days.
        let then = SystemTime::now() - Duration::from_secs(4 * 24 * 60 * 60);
        for p in ["loft/Cargo.toml", "loft/target/debug/app", "loft/target", "loft/.git", "loft"] {
            crate::testdir::set_modified(&ws.join(p), then);
        }
        super::before_restart(&st).await;
        assert!(!ws.join("loft/target").exists(), "the update restarted with its build output");
        assert!(st.rt.lock().unwrap().clean.at.is_some());
    }

    /// Runs git in `dir` without the user's config; returns stdout.
    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"]).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A bare origin with one commit on main and a clone of it; returns (tempdir, clone, head sha).
    fn checkout() -> (crate::testdir::TempDir, std::path::PathBuf, String) {
        let tmp = crate::testdir::TempDir::new("updater");
        let (origin, clone) = (tmp.join("origin.git"), tmp.join("loft"));
        git(&tmp, &["init", "-q", "--bare", origin.to_str().unwrap()]);
        git(&tmp, &["clone", "-q", origin.to_str().unwrap(), clone.to_str().unwrap()]);
        git(&clone, &["commit", "-q", "--allow-empty", "-m", "one"]);
        git(&clone, &["push", "-q", "-u", "origin", "main"]);
        git(&clone, &["remote", "set-head", "origin", "main"]);
        let head = git(&clone, &["rev-parse", "HEAD"]);
        (tmp, clone, head)
    }

    /// One check of `clone`, applied as the loop applies it, read back through GET /api/update.
    async fn status(clone: &std::path::Path, head: &str) -> serde_json::Value {
        let st = crate::tasks::tests::state();
        status_in(&st, clone, head, None).await
    }

    async fn status_in(st: &crate::state::AppState, clone: &std::path::Path, head: &str, failed: Option<&(String, String)>) -> serde_json::Value {
        super::checked(st, head, super::check(clone, head).await, failed);
        crate::api::update_status(axum::extract::State(st.clone())).await.0
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_build_shows_its_error_again_once_the_checkout_is_back_on_main() {
        let (tmp, clone, head) = checkout();
        let other = tmp.join("other");
        git(&tmp, &["clone", "-q", tmp.join("origin.git").to_str().unwrap(), other.to_str().unwrap()]);
        git(&other, &["commit", "-q", "--allow-empty", "-m", "two"]);
        git(&other, &["push", "-q"]);
        // The build of the upstream commit failed; then the checkout went detached, then back on main.
        let failed = (git(&other, &["rev-parse", "HEAD"]), "`cargo build` failed: error[E0425]".to_string());
        let st = crate::tasks::tests::state();
        git(&clone, &["checkout", "-q", "--detach", &head]);
        assert_eq!(status_in(&st, &clone, &head, Some(&failed)).await["phase"], "blocked");
        git(&clone, &["switch", "-q", "main"]);
        let s = status_in(&st, &clone, &head, Some(&failed)).await;
        assert_eq!(s["phase"], "error", "{s}");
        assert_eq!(s["error"], failed.1);
        assert_eq!(s["why"], serde_json::Value::Null);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_detached_checkout_says_which_commit_its_on_and_to_switch_back_to_main() {
        let (_tmp, clone, head) = checkout();
        git(&clone, &["checkout", "-q", "--detach", &head]);
        let s = status(&clone, &head).await;
        assert_eq!(s["phase"], "blocked");
        assert_eq!(s["why"], format!("Loft's checkout is on commit {}, not on main. Switch it back to main to get updates.", &head[..8]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_branch_with_no_upstream_says_so() {
        let (_tmp, clone, head) = checkout();
        git(&clone, &["switch", "-q", "-c", "scratch"]);
        let s = status(&clone, &head).await;
        assert_eq!(s["phase"], "blocked");
        assert!(s["why"].as_str().unwrap().starts_with("Loft's checkout is on branch scratch, which has no upstream."), "{s}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_commits_not_on_the_upstream_say_how_many() {
        let (_tmp, clone, head) = checkout();
        git(&clone, &["commit", "-q", "--allow-empty", "-m", "local"]);
        git(&clone, &["commit", "-q", "--allow-empty", "-m", "local 2"]);
        let s = status(&clone, &head).await;
        assert_eq!(s["phase"], "blocked");
        assert_eq!(s["why"], "Loft's checkout has 2 commits on main that aren't on origin/main. Push or drop them to get updates.");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_fetch_says_it_couldnt_fetch() {
        let (tmp, clone, head) = checkout();
        std::fs::rename(tmp.join("origin.git"), tmp.join("gone.git")).unwrap();
        let s = status(&clone, &head).await;
        assert_eq!(s["phase"], "blocked");
        assert!(s["why"].as_str().unwrap().starts_with("Couldn't fetch updates:"), "{s}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn updates_resume_once_the_checkout_is_back_on_main() {
        let (tmp, clone, head) = checkout();
        git(&clone, &["checkout", "-q", "--detach", &head]);
        assert_eq!(status(&clone, &head).await["phase"], "blocked");
        git(&clone, &["switch", "-q", "main"]);
        assert_eq!(status(&clone, &head).await["phase"], "up_to_date");
        // A newer commit on origin is picked up again.
        let other = tmp.join("other");
        git(&tmp, &["clone", "-q", tmp.join("origin.git").to_str().unwrap(), other.to_str().unwrap()]);
        git(&other, &["commit", "-q", "--allow-empty", "-m", "two"]);
        git(&other, &["push", "-q"]);
        let s = status(&clone, &head).await;
        assert_eq!(s["phase"], "waiting");
        assert_eq!(s["latest"], git(&other, &["rev-parse", "--short=7", "HEAD"]));
    }
}
