//! Self-update: with auto-update on in Settings, poll the checkout's upstream branch and,
//! once no agent is running, pull, rebuild and restart into the new binary.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;
use tokio::process::Command;

use crate::state::{AppState, Event};

const CHECK_EVERY: Duration = Duration::from_secs(60);
const IDLE_POLL: Duration = Duration::from_secs(5);
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
}

/// True while an update is queued or being installed; the scheduler holds autonomous wake-ups then.
pub fn pending(state: &AppState) -> bool {
    matches!(state.update.lock().unwrap().phase, Phase::Waiting | Phase::Updating)
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
    let out = Command::new(program).args(args).current_dir(dir).output().await?;
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

async fn install(root: &Path) -> Result<()> {
    run(root, "git", &["pull", "--ff-only", "--quiet"]).await?;
    // Backend first: if it fails, the live frontend bundle stays in step with the running binary.
    let mut cargo = vec!["build"];
    if !cfg!(debug_assertions) {
        cargo.push("--release");
    }
    run(&root.join("backend"), "cargo", &cargo).await?;
    let frontend = root.join("frontend");
    run(&frontend, "npm", &["install", "--no-audit", "--no-fund"]).await?;
    run(&frontend, "npm", &["run", "build"]).await?;
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

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

pub async fn updater(state: AppState, root: PathBuf) {
    // The commit this process was built from.
    let built = run(&root, "git", &["rev-parse", "HEAD"]).await.map_err(|e| e.to_string());
    // A commit whose build failed; skipped until upstream moves on.
    let mut failed: Option<String> = None;
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
                        // Agents may have been started by hand during the build. The new build is
                        // on disk, so restart into it even if auto-update was turned off meanwhile.
                        while running_agents(&state) > 0 {
                            tokio::time::sleep(IDLE_POLL).await;
                        }
                        crate::notify::send_now(&state, crate::notify::Notice::Update, &format!("loft updated to {}", short(&up))).await;
                        restart();
                    }
                    Err(e) => {
                        tracing::error!("update failed: {e:#}");
                        crate::notify::send(&state, crate::notify::Notice::Update, format!("loft update to {} failed: {}", short(&up), crate::notify::clip(&e.to_string(), 300)));
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

/// Replace this process with the binary on disk (same path, args and env): after an update, the new build.
/// If exec fails, exit and leave the restart to the supervisor (e.g. launchd KeepAlive).
pub fn restart() -> ! {
    if let Ok(exe) = std::env::current_exe() {
        let err = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
        tracing::error!("restart: exec failed: {err}");
    }
    std::process::exit(0)
}
