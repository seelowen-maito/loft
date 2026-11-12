//! Self-update: with auto-update on in Settings, poll the checkout's upstream branch and,
//! once no agent is running, pull, rebuild and restart into the new binary.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Result, bail};
use serde::Serialize;
use tokio::process::Command;

use crate::state::{AppState, Event};

const CHECK_EVERY: Duration = Duration::from_secs(60);
const IDLE_POLL: Duration = Duration::from_secs(5);
const RESTART_BACKOFF: Duration = Duration::from_secs(30);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(600);
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
    /// Why a restart hasn't happened, e.g. "2 agents are running".
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
/// autonomous wake-ups then. Not after a failed exec: that would hold them for as long as the retries fail.
pub fn pending(state: &AppState) -> bool {
    let s = state.update.lock().unwrap();
    match s.phase {
        Phase::Waiting | Phase::Updating => true,
        Phase::Restart => !s.why.as_deref().is_some_and(|w| w.starts_with("restart failed")),
        _ => false,
    }
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
async fn check(root: &Path, running: &str) -> Result<Option<String>> {
    run(root, "git", &["fetch", "--quiet"]).await?;
    let upstream = run(root, "git", &["rev-parse", "@{u}"]).await?;
    if upstream == running {
        return Ok(None);
    }
    if run(root, "git", &["merge-base", "--is-ancestor", running, &upstream]).await.is_err() {
        // Local commits not on the upstream: nothing to fast-forward to.
        return Ok(None);
    }
    Ok(Some(upstream))
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
    run(&root.join("backend"), "cargo", &cargo).await?;
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
    Ok(())
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
    // A commit whose build failed; skipped until upstream moves on.
    let mut failed: Option<String> = None;
    // After a failed exec: when to try again, and how long to wait after the next failure.
    let mut retry_at: Option<std::time::Instant> = None;
    let mut backoff = RESTART_BACKOFF;
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
            set(&state, |s| *s = restart_status(&head, n, s.why.take()));
            // Restart by itself once agents are idle, as an update does; `restart` only returns when exec failed.
            let now = std::time::Instant::now();
            if should_restart(n, now, retry_at) {
                before_restart(&state).await;
                restart(&state);
                retry_at = Some(std::time::Instant::now() + backoff);
                backoff = next_backoff(backoff);
            }
            tokio::time::sleep(IDLE_POLL).await;
            continue;
        }
        match check(&root, &head).await {
            Err(e) => set(&state, |s| {
                s.phase = Phase::Error;
                s.current = short(&head);
                s.error = Some(e.to_string());
            }),
            Ok(None) => set(&state, |s| {
                *s = UpdateStatus { phase: Phase::UpToDate, current: short(&head), ..Default::default() };
            }),
            Ok(Some(up)) if failed.as_deref() == Some(up.as_str()) => {}
            Ok(Some(up)) => {
                set(&state, |s| {
                    *s = UpdateStatus { phase: Phase::Waiting, current: short(&head), latest: Some(short(&up)), ..Default::default() };
                });
                if !wait_for_idle(&state).await {
                    continue;
                }
                set(&state, |s| s.phase = Phase::Updating);
                tracing::info!("updating {} -> {}", short(&head), short(&up));
                match install(&root).await {
                    Ok(()) => {
                        before_restart(&state).await;
                        restart(&state);
                    }
                    Err(e) => {
                        tracing::error!("update failed: {e:#}");
                        failed = Some(up);
                        set(&state, |s| {
                            s.phase = Phase::Error;
                            s.error = Some(e.to_string());
                        });
                    }
                }
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

fn next_backoff(b: Duration) -> Duration {
    (b * 2).min(RESTART_BACKOFF_MAX)
}

/// Restart now: no agent runs, and a failed exec's backoff is over.
fn should_restart(running_agents: usize, now: std::time::Instant, retry_at: Option<std::time::Instant>) -> bool {
    running_agents == 0 && retry_at.is_none_or(|t| now >= t)
}

/// The status while a newer build waits on disk. A failed exec's reason stays until a restart works. The loop
/// doesn't check upstream in this phase (the commit it would compare is the old one, so it would rebuild what's
/// already on disk): it restarts when idle, and the new process checks.
fn restart_status(head: &str, running_agents: usize, prev_why: Option<String>) -> UpdateStatus {
    let why = prev_why.filter(|w| w.starts_with("restart failed")).or_else(|| {
        (running_agents > 0).then(|| format!("{running_agents} agent{} running; restarts when they finish", if running_agents == 1 { " is" } else { "s are" }))
    });
    UpdateStatus { phase: Phase::Restart, current: short(head), running_agents, why, ..Default::default() }
}

/// Replace this process with the build on disk (same args and env): after an update, the new build, which
/// signs its own copy again. If exec fails, the old build keeps running and the status says why.
pub fn restart(state: &AppState) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => return restart_failed(state, err),
    };
    let err = std::process::Command::new(crate::signing::build_of(&exe)).args(std::env::args_os().skip(1)).exec();
    restart_failed(state, err)
}

fn restart_failed(state: &AppState, err: std::io::Error) {
    tracing::error!("restart: exec failed: {err}");
    // Stay up on the running build and say why it didn't switch.
    set(state, |s| {
        s.phase = Phase::Restart;
        s.why = Some(format!("restart failed: {err}"));
    });
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
    async fn wake_ups_are_held_until_the_restart_unless_it_failed() {
        let st = crate::tasks::tests::state();
        let phase = |p: super::Phase, why: Option<&str>| {
            *st.update.lock().unwrap() = super::UpdateStatus { phase: p, why: why.map(Into::into), ..Default::default() };
            super::pending(&st)
        };
        assert!(phase(super::Phase::Restart, None));
        assert!(phase(super::Phase::Restart, Some("1 agent is running; restarts when they finish")));
        assert!(!phase(super::Phase::Restart, Some("restart failed: boom")));
        assert!(!phase(super::Phase::UpToDate, None));
    }

    #[test]
    fn a_restart_waits_for_idle_and_for_its_backoff() {
        use std::time::Instant;
        let now = Instant::now();
        assert!(!super::should_restart(1, now, None));
        assert!(super::should_restart(0, now, None));
        assert!(!super::should_restart(0, now, Some(now + Duration::from_secs(5))));
        assert!(super::should_restart(0, now, Some(now)));
        assert!(!super::should_restart(2, now, Some(now)));
    }

    #[test]
    fn the_restart_backoff_doubles_up_to_its_cap() {
        let mut b = super::RESTART_BACKOFF;
        let mut seen = vec![b.as_secs()];
        for _ in 0..6 {
            b = super::next_backoff(b);
            seen.push(b.as_secs());
        }
        assert_eq!(seen, [30, 60, 120, 240, 480, 600, 600]);
    }

    #[test]
    fn a_failed_restart_keeps_its_reason_across_loop_ticks() {
        let failed = Some("restart failed: boom".to_string());
        assert_eq!(super::restart_status("abcdef012", 2, failed.clone()).why, failed);
        assert_eq!(super::restart_status("abcdef012", 0, None).why, None);
        assert!(super::restart_status("abcdef012", 1, Some("1 agent is running".into())).why.unwrap().starts_with("1 agent is"));
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
            std::fs::File::open(ws.join(p)).unwrap().set_modified(then).unwrap();
        }
        super::before_restart(&st).await;
        assert!(!ws.join("loft/target").exists(), "the update restarted with its build output");
        assert!(st.rt.lock().unwrap().clean.at.is_some());
    }
}
