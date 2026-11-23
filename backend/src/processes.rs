//! Agent processes: which agent owns which process, and the kills agents run, which Loft scopes to them.
//!
//! Ownership is by process session. Each run's claude starts in a session of its own, and Claude Code starts each
//! Bash command in another one, so the script Loft gives claude as `CLAUDE_ENV_FILE` (claude runs it in each
//! command's shell, before the command) records that shell's session for the agent. A detached server
//! (`nohup … &`) keeps its session after the shell exits and is reparented, so a process is the agent's when its
//! session id is one of the agent's. The records are files, `.agent-files/<id>.sessions/<sid>`: pruned once no
//! process of the session is left, and removed with the agent, whose processes are stopped then.
//!
//! A daemon (`pg_ctl start`, `redis-server --daemonize yes`, `celery --detach`) leaves the session for one of its
//! own, so a process of the user's in no agent's session is also the agent's whose workspace holds its working
//! folder. Not one with a terminal: that's the user's own shell, or what they run in it. Nor one under Loft (its git
//! checks and size scans in a clone) or under a GUI app (an editor's language servers, when the user opens a clone):
//! a daemon leaves its parent and runs under launchd.
//!
//! The same script makes `kill`, `pkill` and `killall` shell functions, and puts commands of those names first on
//! PATH (for `xargs kill`, `find -exec kill`), that run `loft kill-as <tool> <args>`. That asks Loft
//! (`POST /kill/<agent id>`), which finds the targets the way the tool would, signals the agent's own by PID and
//! sends any other (another agent's, the user's apps and MCP servers, Loft itself) to the user as a permission
//! request. Only a caller running in one of the agent's sessions has any own: from anywhere else (a command holding
//! another agent's URL), every target goes to the user. Unanswered, it is refused. Kills that skip the functions (by path, through `sudo` or `builtin`, or a
//! `kill` in another shell) are the Bash hook's (kill_guard.rs): it refuses them and says to kill by name.
//!
//! On Windows the agent's processes are those in its Job Object (job.rs), and the caller speaks for the agent when it
//! is in the agent's job. The script, in Git Bash, also makes `taskkill` a function and a command first on PATH, and
//! turns the Git Bash PIDs `kill` is given (`$!`) into Windows ones. Windows has no `pgrep` or `killall`, so Loft finds
//! what `pkill`, `killall` and `taskkill` name itself (by_name.rs).

// Windows uses it; every OS tests it.
#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code))]
mod by_name;

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path as UrlPath, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::Agent;
use crate::state::AppState;
use crate::sys::Canonical;

/// The argument that makes this binary run as an agent's `kill`, `pkill` or `killall`.
pub const ARG: &str = "kill-as";
/// Where `loft kill-as` asks Loft: `http://127.0.0.1:<port>/kill/<agent id>`, in claude's environment.
pub const URL_VAR: &str = "LOFT_KILL_URL";

/// The tools Loft scopes, and the real command each stands for.
#[cfg(unix)]
const TOOLS: &[(&str, &str)] = &[("kill", "/bin/kill"), ("pkill", "/usr/bin/pkill"), ("killall", "/usr/bin/killall")];
/// On Windows only taskkill is a real command (in System32): Git Bash's kill is its builtin, and it has no pkill or
/// killall, which Loft emulates.
#[cfg(windows)]
const TOOLS: &[(&str, &str)] = &[("kill", ""), ("pkill", ""), ("killall", ""), ("taskkill", "taskkill.exe")];

/// How long a kill of someone else's process waits for the user.
#[cfg(not(test))]
const ASK_TIMEOUT: Duration = crate::agents::PERMISSION_TIMEOUT;
#[cfg(test)]
const ASK_TIMEOUT: Duration = Duration::from_secs(3);

/// How long a deleted agent's processes get to exit after SIGTERM, before SIGKILL.
#[cfg(not(test))]
const STOP_GRACE: Duration = Duration::from_secs(5);
#[cfg(test)]
const STOP_GRACE: Duration = Duration::from_millis(300);

/// A session recorded this recently is kept even with no process yet: its shell may not show in the table taken.
const PRUNE_AFTER: Duration = Duration::from_secs(60);

// ---- ownership --------------------------------------------------------

fn sessions_dir(storage: &Path, agent_id: &str) -> PathBuf {
    storage.join(crate::agents::AGENT_FILES).join(format!("{agent_id}.sessions"))
}

/// Records process session `sid` as the agent's.
#[cfg(unix)]
pub fn record(storage: &Path, agent_id: &str, sid: u32) -> anyhow::Result<()> {
    let dir = sessions_dir(storage, agent_id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(sid.to_string()), "")?;
    Ok(())
}

/// The process sessions the agent's runs used.
pub fn sessions(storage: &Path, agent_id: &str) -> HashSet<i32> {
    let Ok(dir) = std::fs::read_dir(sessions_dir(storage, agent_id)) else { return HashSet::new() };
    dir.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect()
}

/// Every recorded session, and the agent it belongs to.
fn session_owners(storage: &Path) -> HashMap<i32, String> {
    let mut owners = HashMap::new();
    let Ok(dir) = std::fs::read_dir(storage.join(crate::agents::AGENT_FILES)) else { return owners };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(id) = name.strip_suffix(".sessions") {
            owners.extend(sessions(storage, id).into_iter().map(|sid| (sid, id.to_string())));
        }
    }
    owners
}

/// Who owns a process: the agent whose session it runs in, else the agent of the nearest process it runs under that
/// is in an agent's session, else the agent whose workspace holds its working folder. On Windows, the agent whose
/// job it is in (see job.rs).
#[cfg_attr(windows, allow(dead_code))]
struct Owners {
    sessions: HashMap<i32, String>,
    /// Each agent's workspace, resolved (a process's working folder comes resolved), and its id.
    workspaces: Vec<(PathBuf, String)>,
    uid: u32,
    /// The processes in each agent's job, and the agent.
    #[cfg(windows)]
    jobs: HashMap<i32, String>,
}

impl Owners {
    /// The recorded sessions, and the workspaces of `agents`: every agent's, or only the one whose processes count.
    /// On Windows, every agent's job.
    fn load(storage: &Path, agents: &[String]) -> Self {
        let workspaces = agents.iter().filter_map(|id| Some((storage.join(id).canonical().ok()?, id.clone()))).collect();
        #[cfg(windows)]
        let jobs = crate::job::agents(storage).into_iter().flat_map(|id| crate::job::pids(storage, &id).into_iter().map(move |pid| (pid as i32, id.clone()))).collect();
        Owners {
            sessions: session_owners(storage),
            workspaces,
            uid: crate::sys::uid(),
            #[cfg(windows)]
            jobs,
        }
    }

    /// The agent `p` is: the one of its session, else the one of the nearest session above it before Loft or a GUI
    /// app (a child that left its parent's session, like node's `detached` or Python's `start_new_session`, while
    /// that parent runs), else, for a process of the user's with no terminal that runs under neither Loft nor a GUI
    /// app, the one whose workspace holds its working folder.
    fn of(&self, p: &Proc) -> Option<&String> {
        #[cfg(windows)]
        return self.jobs.get(&p.pid);
        #[cfg(not(windows))]
        {
            let sid = session_of(p.pid)?;
            if let Some(id) = std::iter::once(&sid).chain(&p.sessions_above).find_map(|s| self.sessions.get(s)) {
                return Some(id);
            }
            if self.workspaces.is_empty() || p.uid != self.uid || p.tty || p.under_app {
                return None;
            }
            let cwd = cwd_of(p.pid)?;
            self.workspaces.iter().find(|(w, _)| cwd.starts_with(w)).map(|(_, id)| id)
        }
    }
}

/// The working folder of `pid`, if Loft may read it (a process of the user's).
#[cfg(target_os = "macos")]
fn cwd_of(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, (&raw mut info).cast(), size) };
    if got != size {
        return None;
    }
    let path = unsafe { std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    (!path.is_empty()).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn cwd_of(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

/// Xcode's helper daemons, which `xcrun` and `xcodebuild` start and which never exit. They run in the agent's
/// session but aren't its processes: Clean doesn't list them, and the idle stop and Stop them leave them alone.
#[cfg(unix)]
const HELPERS: [&str; 4] = ["DTServiceHub", "DTDeviceKit", "XCBBuildService", "SWBBuildService"];

/// Whether `comm`, a process's executable path with no arguments (`ps -o comm=`), is one of the helpers inside an
/// app bundle: decided from the executable, not from the arguments. `comm` is argv[0] on macOS, so an agent can
/// still spoof the name (`exec -a`, a symlink) and hide a process from Clean; that low risk is accepted.
#[cfg(unix)]
fn is_helper(comm: &str) -> bool {
    comm.contains(".app/") && HELPERS.contains(&comm.rsplit('/').next().unwrap_or_default())
}

/// Whether `comm`, a process's executable path, is a test browser: one of Playwright's (in its cache,
/// `ms-playwright`) or any Chrome for Testing, with their helpers. Never the user's own Google Chrome.
fn is_test_browser(comm: &str) -> bool {
    comm.contains("/ms-playwright/") || comm.rsplit('/').next().unwrap_or_default().starts_with("Google Chrome for Testing")
}

/// A process an agent left alive, for Clean to list.
#[derive(Debug, PartialEq)]
pub struct Live {
    pub pid: i32,
    pub command: String,
    pub age_secs: u64,
}

/// The live processes of `agents`, like a detached dev server or a daemon in the workspace, by agent id (Xcode's
/// helpers aside).
pub fn live_processes(storage: &Path, agents: &[String]) -> HashMap<String, Vec<Live>> {
    let owners = Owners::load(storage, agents);
    let me = std::process::id() as i32;
    let mut live: HashMap<String, Vec<Live>> = HashMap::new();
    for p in table().into_iter().filter(|p| p.pid != me && !p.helper) {
        if let Some(id) = owners.of(&p) {
            live.entry(id.clone()).or_default().push(Live { pid: p.pid, command: p.command, age_secs: p.age_secs });
        }
    }
    live
}

/// The ids of every agent.
fn agent_ids(state: &AppState) -> Vec<String> {
    state.db.list_agents().unwrap_or_default().into_iter().map(|a| a.id).collect()
}

/// The process session of `pid`, if it is alive. Windows has no sessions: none there.
fn session_of(pid: i32) -> Option<i32> {
    #[cfg(unix)]
    {
        let sid = unsafe { libc::getsid(pid) };
        (sid > 0).then_some(sid)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

/// The process group of `pid`, if it is alive. None on Windows.
fn group_of(pid: i32) -> Option<i32> {
    #[cfg(unix)]
    {
        let pgid = unsafe { libc::getpgid(pid) };
        (pgid >= 0).then_some(pgid)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

/// Sends `signal` to `pid`. Windows has no signals: there signal 0 checks the process is alive, and any other ends it.
fn signal(pid: i32, signal: i32) -> std::io::Result<()> {
    #[cfg(unix)]
    return if unsafe { libc::kill(pid, signal) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) };
    #[cfg(windows)]
    {
        let pid = u32::try_from(pid).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        if signal != 0 {
            return crate::job::kill(pid);
        }
        if crate::job::alive(pid) { Ok(()) } else { Err(std::io::Error::from(std::io::ErrorKind::NotFound)) }
    }
}

/// The script claude runs in each Bash command's shell before the command (`CLAUDE_ENV_FILE`): it records the
/// shell's session (Claude Code makes the shell a session leader, so `$$` is its id) and sends kills through Loft.
/// A job (`kill %1`) is the shell's own child, so a call naming only jobs (after the signal option) stays with the
/// builtin; one that mixes jobs and PIDs is refused, or the builtin would signal the PIDs too. On Windows the shell is
/// already in the agent's job (see job.rs), so nothing is recorded, the paths are Git Bash's, `taskkill` goes through
/// Loft too, and Git Bash leaves the arguments as they are (`MSYS_NO_PATHCONV`: it would read `/PID` as a path).
pub fn env_script(storage: &Path, agent_id: &str, exe: &Path) -> String {
    env_script_on(storage, agent_id, exe, cfg!(windows))
}

/// On Windows, the shell code that turns each Git Bash PID among the arguments (`$!`, what Git Bash's `ps` shows
/// first) into the Windows PID Loft knows, the way Git Bash's own kill reads a PID: a number that is a Git Bash PID is
/// that process. `local` declares its variables, in a function.
fn windows_pids(local: &str) -> String {
    format!(
        r#"{local}p= w= n=$#
for a in "$@"; do
  case "$p:$a" in
    -s:*|-n:*|*:*[!0-9]*|*:) ;;
    *) [ -r "/proc/$a/winpid" ] && read -r w < "/proc/$a/winpid" && a=$w ;;
  esac
  set -- "$@" "$a"; p=$a
done
shift "$n"
"#
    )
}

fn env_script_on(storage: &Path, agent_id: &str, exe: &Path, windows: bool) -> String {
    let loft = quote(&crate::sys::bash_path(exe));
    let (record, run, to_windows, taskkill) = if windows {
        let run = format!("MSYS_NO_PATHCONV=1 {loft}");
        let pids = windows_pids("local a ").lines().map(|l| format!("    {l}\n")).collect::<String>();
        let taskkill = format!("taskkill() {{ {run} {ARG} taskkill \"$@\"; }}\n");
        ("# Loft: sends kills through Loft.".to_string(), run, pids, taskkill)
    } else {
        let record = format!("# Loft: records this shell's process session as the agent's, and sends kills through Loft.\n: > {}/\"$$\" 2>/dev/null", quote(&sessions_dir(storage, agent_id).to_string_lossy()));
        (record, loft, String::new(), String::new())
    };
    format!(
        r#"{record}
kill() {{
  case " $* " in *" %"*) ;; *)
{to_windows}    {run} {ARG} kill "$@"; return ;;
  esac
  local a s=f
  for a in "$@"; do
    case "$s:$a" in
      f:-s|f:-n) s=v ;;
      v:*) s=o ;;
      f:--|o:--) s=j ;;
      f:%*|o:%*|j:%*) s=j ;;
      f:-*) s=o ;;
      *) echo "kill: Loft refused this: name jobs (%1) and process IDs in separate kill commands." >&2; return 2 ;;
    esac
  done
  builtin kill "$@"
}}
pkill() {{ {run} {ARG} pkill "$@"; }}
killall() {{ {run} {ARG} killall "$@"; }}
{taskkill}PATH={bin}:"$PATH"
export PATH
"#,
        bin = quote(&crate::sys::bash_path(&shims_dir(storage))),
    )
}

fn shims_dir(storage: &Path) -> PathBuf {
    storage.join(crate::agents::AGENT_FILES).join("bin")
}

/// Writes the commands that stand in for `kill`, `pkill` and `killall` (and on Windows `taskkill`) where the shell
/// functions don't reach (`xargs kill`, `find -exec kill`), and returns their folder.
pub fn write_shims(storage: &Path, exe: &Path) -> anyhow::Result<PathBuf> {
    let dir = shims_dir(storage);
    std::fs::create_dir_all(&dir)?;
    for (tool, _) in TOOLS {
        let path = dir.join(tool);
        let text = shim(tool, exe, cfg!(windows));
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text)?;
        }
        crate::owner_only::executable(&path)?;
    }
    Ok(dir)
}

/// The shim for `tool`: on Windows it turns Git Bash PIDs into Windows ones for `kill`, and keeps Git Bash from
/// reading the arguments as paths (see [`env_script`]).
fn shim(tool: &str, exe: &Path, windows: bool) -> String {
    let loft = quote(&crate::sys::bash_path(exe));
    if !windows {
        return format!("#!/bin/sh\nexec {loft} {ARG} {tool} \"$@\"\n");
    }
    let pids = if tool == "kill" { windows_pids("") } else { String::new() };
    format!("#!/bin/sh\n{pids}MSYS_NO_PATHCONV=1\nexport MSYS_NO_PATHCONV\nexec {loft} {ARG} {tool} \"$@\"\n")
}

/// `s` as one shell word: quoted when it holds anything but plain characters.
fn shell_word(s: &str) -> String {
    let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain { s.to_string() } else { quote(s) }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Drops the records of sessions with no process left, so a session id the system hands out again later isn't
/// taken for the agent's.
pub fn prune(storage: &Path) {
    let alive: HashSet<i32> = table().iter().filter_map(|p| session_of(p.pid)).collect();
    let Ok(dir) = std::fs::read_dir(storage.join(crate::agents::AGENT_FILES)) else { return };
    for agent in dir.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".sessions")) {
        for e in std::fs::read_dir(agent.path()).into_iter().flatten().flatten() {
            let sid: Option<i32> = e.file_name().to_str().and_then(|n| n.parse().ok());
            let recent = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default() < PRUNE_AFTER);
            if sid.is_none_or(|sid| !alive.contains(&sid)) && !recent {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Prunes the session records once a minute, and on Windows records the agents' processes.
pub async fn pruner(storage: PathBuf) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let storage = storage.clone();
        let _ = tokio::task::spawn_blocking(move || {
            prune(&storage);
            // On Windows, what a Loft that restarts knows of each agent's job (see job.rs).
            #[cfg(windows)]
            crate::job::remember_all(&storage);
        })
        .await;
    }
}

/// Stops every process the agent owns, when it is deleted (see [`stop`]): on Windows its whole job ends. Its session
/// records go now.
pub async fn stop_all(storage: &Path, agent_id: &str) {
    stop(storage, agent_id, || true, false).await;
    #[cfg(windows)]
    {
        crate::job::terminate(storage, agent_id);
        crate::job::forget(storage, agent_id);
    }
    let _ = tokio::fs::remove_dir_all(sessions_dir(storage, agent_id)).await;
}

/// Stops the test browsers the agent left running (see [`is_test_browser`]), when its run ends: a script that
/// never closed its browser, crashed or timed out. Its detached servers keep running. Returns how many browsers:
/// the stopped processes not run by another one stopped (a browser's renderers and GPU process are its own).
pub async fn stop_test_browsers(storage: &Path, agent_id: &str) -> usize {
    let stopped = stop(storage, agent_id, || true, true).await;
    let pids: HashSet<i32> = stopped.iter().map(|(pid, _, _)| *pid).collect();
    stopped.iter().filter(|(_, ppid, _)| !pids.contains(ppid)).count()
}

/// Stops every process the agent owns (see [`Owners`]), or with `browsers_only` its test browsers in its sessions
/// (never by working folder: a script's browser runs in the script's session): SIGTERM, then SIGKILL for any still there after a grace period. Returns
/// each one's PID, parent's PID and command. `proceed` is asked again right before the first signal, and nothing is stopped
/// when it says no.
async fn stop(storage: &Path, agent_id: &str, proceed: impl FnOnce() -> bool + Send + 'static, browsers_only: bool) -> Vec<(i32, i32, String)> {
    let (storage, id) = (storage.to_path_buf(), agent_id.to_string());
    let targets = tokio::task::spawn_blocking(move || {
        if !proceed() {
            return vec![];
        }
        let owners = Owners::load(&storage, if browsers_only { &[] } else { std::slice::from_ref(&id) });
        let me = std::process::id() as i32;
        let targets: Vec<(i32, i32, i32, String)> = table()
            .into_iter()
            .filter(|p| p.pid != me && !p.helper && (p.browser || !browsers_only) && owners.of(p) == Some(&id))
            // On Windows the first stop ends the process: no session to check before a second.
            .filter_map(|p| Some((p.pid, if cfg!(windows) { 0 } else { session_of(p.pid)? }, p.ppid, p.command)))
            .collect();
        for (pid, _, _, _) in &targets {
            let _ = signal(*pid, SIGTERM);
        }
        targets
    })
    .await
    .unwrap_or_default();
    let stopped = targets.iter().map(|(pid, _, ppid, command)| (*pid, *ppid, command.clone())).collect();
    if !targets.is_empty() {
        tokio::spawn(async move {
            tokio::time::sleep(STOP_GRACE).await;
            // Only a process still in the session: the PID may belong to another one by now.
            for (pid, sid, _, _) in targets {
                if session_of(pid) == Some(sid) {
                    let _ = signal(pid, SIGKILL);
                }
            }
        });
    }
    stopped
}

/// How often idle agents' processes are looked for.
const STOP_IDLE_EVERY: Duration = Duration::from_secs(10 * 60);

/// Stops the processes of agents idle past the timeout (see [`crate::agents::idle`]), like a dev server left
/// running; the agent's next run can start them again. Returns a line per process, for the clean log.
pub async fn stop_idle(state: &AppState) -> Vec<crate::sweep::Line> {
    let (storage, agents) = (state.cfg.storage_dir.clone(), agent_ids(state));
    let live: HashSet<String> = tokio::task::spawn_blocking(move || live_processes(&storage, &agents).into_keys().collect()).await.unwrap_or_default();
    let mut lines = vec![];
    for a in state.db.list_agents().unwrap_or_default().into_iter().filter(|a| live.contains(&a.id)) {
        if !crate::agents::idle(state, &a, std::time::SystemTime::now()) {
            continue;
        }
        // Asked again right before the signals, so a run that just started keeps its processes.
        let (st, agent) = (state.clone(), a.clone());
        let still_idle = move || st.db.get_agent(&agent.id).ok().flatten().is_some_and(|a| crate::agents::idle(&st, &a, std::time::SystemTime::now()));
        for (pid, _, command) in stop(&state.cfg.storage_dir, &a.id, still_idle, false).await {
            let (path, rel) = (PathBuf::from(format!("pid {pid}")), clip(&command, 200));
            lines.push(crate::sweep::Line { agent: a.id.clone(), path, rel, bytes: 0, error: None, command: format!("kill {pid}"), done: "Stopped process".into() });
        }
    }
    lines
}

/// Stops idle agents' processes every few minutes; each run that stopped any is the clean log's.
pub async fn idle_stopper(state: AppState) {
    let mut tick = tokio::time::interval(STOP_IDLE_EVERY);
    loop {
        tick.tick().await;
        let lines = stop_idle(&state).await;
        if !lines.is_empty() {
            crate::sweep::record(&state, "Idle processes", lines).await;
        }
    }
}

/// `GET /api/workspaces/processes`: what each idle agent left running, `{ <agent id>: [{ pid, command, age_secs }] }`,
/// for Clean to list. An agent in a run has none listed: its processes are its own.
pub async fn left_running(State(state): State<AppState>) -> Json<Value> {
    let (storage, agents) = (state.cfg.storage_dir.clone(), agent_ids(&state));
    let live = tokio::task::spawn_blocking(move || live_processes(&storage, &agents)).await.unwrap_or_default();
    let running = state.rt.lock().unwrap().running.clone();
    let list: serde_json::Map<String, Value> = live
        .into_iter()
        .filter(|(id, _)| !running.contains_key(id))
        .map(|(id, ps)| (id, json!(ps.iter().map(|p| json!({ "pid": p.pid, "command": clip(&p.command, 200), "age_secs": p.age_secs })).collect::<Vec<_>>())))
        .collect();
    Json(Value::Object(list))
}

/// `POST /api/workspaces/{id}/stop-processes`: Clean's Stop them, which stops what an agent not in a run left
/// running, the way the idle stop does (see [`stop`]). A run that starts meanwhile keeps its processes. The stops
/// are the clean log's. 409 for an agent in a run.
pub async fn stop_left_running(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> crate::api::ApiResult<Value> {
    // The id names a real agent: a path like `./<id>` must not reach its sessions.
    let id = state.db.get_agent(&id)?.ok_or_else(crate::api::not_found)?.id;
    let st = state.clone();
    let agent = id.clone();
    let not_running = move || !st.rt.lock().unwrap().running.contains_key(&agent);
    if !not_running() {
        return Err(crate::api::ApiError(axum::http::StatusCode::CONFLICT, "the agent is working".into()));
    }
    let stopped = stop(&state.cfg.storage_dir, &id, not_running, false).await;
    let lines: Vec<crate::sweep::Line> = stopped
        .iter()
        .map(|(pid, _, command)| crate::sweep::Line { agent: id.clone(), path: PathBuf::from(format!("pid {pid}")), rel: clip(command, 200), bytes: 0, error: None, command: format!("kill {pid}"), done: "Stopped process".into() })
        .collect();
    if !lines.is_empty() {
        crate::sweep::record(&state, "Stopped processes", lines).await;
    }
    Ok(Json(json!({ "stopped": stopped.len() })))
}

// ---- the process table ------------------------------------------------

#[cfg_attr(windows, allow(dead_code))]
struct Proc {
    pid: i32,
    ppid: i32,
    uid: u32,
    /// Seconds since it started.
    age_secs: u64,
    command: String,
    /// One of Xcode's helpers, by its executable.
    helper: bool,
    /// A test browser, by its executable (see [`is_test_browser`]).
    browser: bool,
    /// It has a terminal: the user's shell, or something run in it.
    tty: bool,
    /// It, or a process it runs under, is Loft or a GUI app: never an agent's by its working folder.
    under_app: bool,
    /// The sessions of the processes it runs under, nearest first, up to Loft or a GUI app (or launchd).
    sessions_above: Vec<i32>,
    /// Its executable's path (`command` is its command line).
    #[cfg(windows)]
    exe: String,
}

/// `ps`'s elapsed time, `[[dd-]hh:]mm:ss`, in seconds.
#[cfg(unix)]
fn etime(s: &str) -> Option<u64> {
    let (days, rest) = s.split_once('-').map_or((0, s), |(d, r)| (d.parse().unwrap_or(0), r));
    let parts: Vec<u64> = rest.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    Some(days * 86400 + parts.iter().fold(0, |t, x| t * 60 + x))
}

/// Every process on Windows, from the system's snapshot: no terminals, user ids or sessions there, and ownership is by
/// job. Its command is its command line, or its executable when Loft may not read that.
#[cfg(windows)]
fn table() -> Vec<Proc> {
    crate::job::table()
        .into_iter()
        .map(|e| {
            let browser = is_test_browser(&e.exe.replace('\\', "/"));
            let command = e.command.filter(|c| !c.trim().is_empty()).unwrap_or_else(|| e.exe.clone());
            Proc { pid: e.pid as i32, ppid: e.ppid as i32, uid: 0, age_secs: e.age_secs, command, helper: false, browser, tty: false, under_app: false, sessions_above: vec![], exe: e.exe }
        })
        .collect()
}

/// Every process on the Mac, from `ps`.
#[cfg(unix)]
fn table() -> Vec<Proc> {
    let Ok(out) = std::process::Command::new("/bin/ps").args(["-axww", "-o", "pid=,ppid=,uid=,etime=,tty=,command="]).output() else {
        return vec![];
    };
    // The executable on its own: `command` mixes it with arguments the agent chooses.
    let (mut helpers, mut apps, mut browsers) = (HashSet::new(), HashSet::new(), HashSet::new());
    if let Ok(o) = std::process::Command::new("/bin/ps").args(["-axww", "-o", "pid=,comm="]).output() {
        for l in String::from_utf8_lossy(&o.stdout).lines() {
            let (pid, comm) = field(l);
            let Ok(pid) = pid.parse::<i32>() else { continue };
            if is_helper(comm) {
                helpers.insert(pid);
            }
            if is_gui_app(comm) {
                apps.insert(pid);
            }
            if is_test_browser(comm) {
                browsers.insert(pid);
            }
        }
    }
    apps.insert(std::process::id() as i32);
    let mut procs: Vec<Proc> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, rest) = field(line);
            let (ppid, rest) = field(rest);
            let (uid, rest) = field(rest);
            let (age, rest) = field(rest);
            let (tty, command) = field(rest);
            let pid: i32 = pid.parse().ok()?;
            let tty = !matches!(tty, "??" | "?" | "-");
            Some(Proc { pid, ppid: ppid.parse().ok()?, uid: uid.parse().ok()?, age_secs: etime(age).unwrap_or(0), command: command.to_string(), helper: helpers.contains(&pid), browser: browsers.contains(&pid), tty, under_app: false, sessions_above: vec![] })
        })
        .collect();
    mark_under_app(&mut procs, &apps, session_of);
    procs
}

/// Whether `comm`, a process's executable path, is a GUI app's (the app's own or a helper app's, like VS Code's
/// `Code Helper (Plugin)`), installed in /Applications or ~/Applications (where JetBrains Toolbox puts its IDEs).
/// Postgres.app's `postgres` isn't: it's in `bin`, not `MacOS`.
#[cfg(unix)]
fn is_gui_app(comm: &str) -> bool {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty()).map(|h| format!("{}/Applications/", h.trim_end_matches('/')));
    (comm.starts_with("/Applications/") || home.is_some_and(|h| comm.starts_with(&h))) && comm.contains(".app/Contents/MacOS/")
}

/// Marks the processes that are, or run under, one of `apps` (Loft and the GUI apps), and lists the sessions
/// (`sid_of`) of the processes each one runs under, up to the first app.
#[cfg(unix)]
fn mark_under_app(procs: &mut [Proc], apps: &HashSet<i32>, sid_of: impl Fn(i32) -> Option<i32>) {
    let parents: HashMap<i32, i32> = procs.iter().map(|p| (p.pid, p.ppid)).collect();
    let mut sids: HashMap<i32, Option<i32>> = HashMap::new();
    for p in procs.iter_mut() {
        let (mut pid, mut seen) = (p.pid, HashSet::new());
        while pid > 1 && seen.insert(pid) {
            if apps.contains(&pid) {
                p.under_app = true;
                break;
            }
            if pid != p.pid {
                let sid = *sids.entry(pid).or_insert_with(|| sid_of(pid));
                if let Some(sid) = sid.filter(|s| p.sessions_above.last() != Some(s)) {
                    p.sessions_above.push(sid);
                }
            }
            pid = parents.get(&pid).copied().unwrap_or(0);
        }
    }
}

/// The first whitespace-separated field of `s`, and the rest.
fn field(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    (&s[..end], s[end..].trim_start())
}

/// `pid` and the processes it runs under, up to launchd.
fn ancestors(procs: &[Proc], mut pid: i32) -> HashSet<i32> {
    let parents: HashMap<i32, i32> = procs.iter().map(|p| (p.pid, p.ppid)).collect();
    let mut seen = HashSet::new();
    while pid > 1 && seen.insert(pid) {
        pid = parents.get(&pid).copied().unwrap_or(0);
    }
    seen
}

// ---- signals ----------------------------------------------------------

#[cfg(unix)]
const SIGTERM: i32 = libc::SIGTERM;
#[cfg(unix)]
const SIGKILL: i32 = libc::SIGKILL;
#[cfg(not(unix))]
const SIGTERM: i32 = 15;
#[cfg(not(unix))]
const SIGKILL: i32 = 9;

#[cfg(unix)]
const SIGNALS: &[(&str, i32)] = &[
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("ILL", libc::SIGILL),
    ("TRAP", libc::SIGTRAP),
    ("ABRT", libc::SIGABRT),
    ("FPE", libc::SIGFPE),
    ("KILL", libc::SIGKILL),
    ("BUS", libc::SIGBUS),
    ("SEGV", libc::SIGSEGV),
    ("SYS", libc::SIGSYS),
    ("PIPE", libc::SIGPIPE),
    ("ALRM", libc::SIGALRM),
    ("TERM", libc::SIGTERM),
    ("URG", libc::SIGURG),
    ("STOP", libc::SIGSTOP),
    ("TSTP", libc::SIGTSTP),
    ("CONT", libc::SIGCONT),
    ("CHLD", libc::SIGCHLD),
    ("TTIN", libc::SIGTTIN),
    ("TTOU", libc::SIGTTOU),
    ("IO", libc::SIGIO),
    ("XCPU", libc::SIGXCPU),
    ("XFSZ", libc::SIGXFSZ),
    ("VTALRM", libc::SIGVTALRM),
    ("PROF", libc::SIGPROF),
    ("WINCH", libc::SIGWINCH),
    ("USR1", libc::SIGUSR1),
    ("USR2", libc::SIGUSR2),
];

/// Windows has no signals: the names kill takes, with the usual numbers.
#[cfg(not(unix))]
const SIGNALS: &[(&str, i32)] = &[("HUP", 1), ("INT", 2), ("QUIT", 3), ("KILL", SIGKILL), ("TERM", SIGTERM)];

/// A signal by number (`9`) or name (`KILL`, `SIGKILL`, `term`).
fn signal_of(s: &str) -> Option<i32> {
    if let Ok(n) = s.parse::<i32>() {
        return (0..32).contains(&n).then_some(n);
    }
    let up = s.to_ascii_uppercase();
    let name = up.strip_prefix("SIG").unwrap_or(&up);
    SIGNALS.iter().find(|(n, _)| *n == name).map(|(_, sig)| *sig)
}

// ---- the agent's side: `loft kill-as <tool> <args>` --------------------

/// Runs as an agent's `kill`, `pkill`, `killall` (or on Windows `taskkill`): asks Loft, prints its answer and exits with
/// its code. Outside an agent (no `LOFT_KILL_URL`) it is the real command.
pub fn cli() -> ! {
    let mut argv = std::env::args().skip(2);
    let tool = argv.next().unwrap_or_default();
    let args: Vec<String> = argv.collect();
    let Some((_, real)) = TOOLS.iter().find(|(t, _)| *t == tool) else {
        eprintln!("loft {ARG}: expected {}", TOOLS.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", "));
        std::process::exit(2);
    };
    let Ok(url) = std::env::var(URL_VAR) else { real_command(&tool, real, &args) };
    // Where the command runs: a relative path in an option (`pkill -F server.pid`) is read from there.
    let cwd = std::env::current_dir().ok();
    let body = json!({ "tool": tool, "args": args, "caller": std::process::id(), "cwd": cwd });
    match post(&url, &body.to_string()) {
        Ok(v) => {
            print!("{}", v["stdout"].as_str().unwrap_or_default());
            eprint!("{}", v["stderr"].as_str().unwrap_or_default());
            std::process::exit(v["code"].as_i64().unwrap_or(1) as i32);
        }
        Err(e) => {
            eprintln!("{tool}: nothing was stopped: Loft, which checks that the processes are yours, didn't answer ({e}).");
            std::process::exit(1);
        }
    }
}

/// Runs the real command, outside an agent.
#[cfg(unix)]
fn real_command(tool: &str, real: &str, args: &[String]) -> ! {
    let e = crate::sys::exec(std::process::Command::new(real).args(args));
    eprintln!("{tool}: {e}");
    std::process::exit(126);
}

/// Runs the real command, outside an agent: Windows' taskkill, from System32 (not the shim of that name on PATH). There
/// is none for the others.
#[cfg(windows)]
fn real_command(tool: &str, real: &str, args: &[String]) -> ! {
    if real.is_empty() {
        eprintln!("{tool}: Loft's {tool} works only in an agent's command.");
        std::process::exit(126);
    }
    let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    match std::process::Command::new(root.join("System32").join(real)).args(args).status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("{tool}: {e}");
            std::process::exit(126);
        }
    }
}

/// A plain HTTP/1.1 POST to Loft on localhost, with no runtime to start: this runs inside an agent's command (and
/// in the Bash hook). No read timeout: a kill of someone else's process, or a change to a user's checkout, waits for
/// the user.
pub fn post(url: &str, body: &str) -> anyhow::Result<Value> {
    let rest = url.strip_prefix("http://").ok_or_else(|| anyhow::anyhow!("bad Loft URL"))?;
    let (host, path) = rest.split_once('/').ok_or_else(|| anyhow::anyhow!("bad Loft URL"))?;
    let mut s = std::net::TcpStream::connect(host)?;
    write!(
        s,
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut resp = String::new();
    s.read_to_string(&mut resp)?;
    let (head, body) = resp.split_once("\r\n\r\n").ok_or_else(|| anyhow::anyhow!("no answer"))?;
    let status = head.lines().next().unwrap_or_default();
    anyhow::ensure!(status.split_whitespace().nth(1) == Some("200"), "{status}");
    Ok(serde_json::from_str(body)?)
}

// ---- Kill rules -------------------------------------------------------

/// The real executable path of `pid` from the kernel, not `ps`'s argv[0], which a process can set to anything.
#[cfg(target_os = "macos")]
fn exe_of(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}
#[cfg(not(target_os = "macos"))]
fn exe_of(_pid: i32) -> Option<String> {
    None
}

/// A folder Xcode names `<Name>-<hash>` in DerivedData: 28 lowercase letters after the dash.
fn is_build_folder(part: &str) -> bool {
    part.rsplit_once('-').is_some_and(|(name, hash)| !name.is_empty() && hash.len() == 28 && hash.bytes().all(|b| b.is_ascii_lowercase()))
}

/// Programs that run whatever script they are given: a rule for one would allow stopping anything they run.
const INTERPRETERS: &[&str] = &["node", "python", "ruby", "perl", "php", "bash", "sh", "zsh", "fish", "deno", "bun", "java", "osascript"];

/// The `Kill(...)` rule for stopping the program at `exe`: its path, with each per-build folder as `*` so a rebuild
/// still matches. `None` for a path that can't make a rule that names one program.
fn kill_rule(exe: &str) -> Option<String> {
    if !exe.starts_with('/') || exe.contains(['*', '(', ')', '\n']) {
        return None;
    }
    let spec = exe.split('/').map(|p| if is_build_folder(p) { "*" } else { p }).collect::<Vec<_>>().join("/");
    kill_spec_ok(&spec).then(|| format!("Kill({spec})"))
}

fn is_interpreter(file: &str) -> bool {
    INTERPRETERS.contains(&file.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.'))
}

/// A path pattern names a program only if it ends in a literal file name that isn't an interpreter (`python3.12` and
/// `node22` count), so `*` and `*/` never match everything.
fn kill_spec_ok(spec: &str) -> bool {
    let file = spec.rsplit('/').next().unwrap_or_default();
    spec.starts_with('/') && !file.is_empty() && !file.contains('*') && !spec.contains(')') && !is_interpreter(file)
}

/// The pattern inside a saved `Kill(...)` rule. A bare `Kill` or a rule that would match every program has none.
pub fn kill_spec(rule: &str) -> Option<&str> {
    let spec = rule.trim().strip_prefix("Kill(")?.strip_suffix(')')?;
    kill_spec_ok(spec).then_some(spec)
}

/// `*` matches any run of characters inside one path folder or file name, never a `/`.
fn glob(pat: &str, s: &str) -> bool {
    match pat.split_once('*') {
        None => pat == s,
        Some((head, rest)) => {
            let Some(s) = s.strip_prefix(head) else { return false };
            let reach = s.find('/').unwrap_or(s.len());
            (0..=reach).filter(|&i| s.is_char_boundary(i)).any(|i| glob(rest, &s[i..]))
        }
    }
}

fn rule_allows(rules: &[String], exe: &str) -> bool {
    rules.iter().filter_map(|r| kill_spec(r)).any(|spec| glob(spec, exe))
}

// ---- Loft's side: `POST /kill/<agent id>` -----------------------------

#[derive(Deserialize)]
pub struct KillRequest {
    tool: String,
    #[serde(default)]
    args: Vec<String>,
    /// The `loft kill-as` process, which runs in the agent's command.
    #[serde(default)]
    caller: i32,
    /// The caller's working directory, where `pgrep` and `killall` run so relative paths read as the agent meant.
    #[serde(default)]
    cwd: Option<PathBuf>,
}

/// What the agent's command prints and exits with.
#[derive(Default, Debug)]
struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Output {
    fn error(code: i32, stderr: String) -> Self {
        Output { code, stderr, ..Default::default() }
    }
}

pub async fn handle(State(state): State<AppState>, UrlPath(agent_id): UrlPath<String>, Json(req): Json<KillRequest>) -> Json<Value> {
    let out = match state.db.get_agent(&agent_id) {
        Ok(Some(a)) => kill(&state, &a, req).await,
        _ => Output::error(1, format!("{}: nothing was stopped: Loft has no agent {agent_id}.\n", req.tool)),
    };
    Json(json!({ "code": out.code, "stdout": out.stdout, "stderr": out.stderr }))
}

/// What a kill command comes to.
enum Plan {
    /// Send `signal` to these processes. `out` holds what the tool says about operands it couldn't use.
    Signal { signal: i32, pids: Vec<i32>, out: Output },
    /// Nothing to send: the tool's answer as it is (`kill -l`, a usage error, no process matched).
    Done(Output),
}

async fn kill(state: &AppState, a: &Agent, req: KillRequest) -> Output {
    let (storage, agents) = (state.cfg.storage_dir.clone(), agent_ids(state));
    let (tool, caller) = (req.tool.clone(), req.caller);
    // As the agent typed it, near enough: `pkill -f 'sleep 120'` mustn't read as a PID 120.
    let command = std::iter::once(req.tool.clone()).chain(req.args.iter().map(|a| shell_word(a))).collect::<Vec<_>>().join(" ");
    let Ok((plan, owners, procs)) = tokio::task::spawn_blocking(move || {
        let procs = table();
        (plan(&req, &procs), Owners::load(&storage, &agents), procs)
    })
    .await
    else {
        return Output::error(1, format!("{tool}: nothing was stopped: Loft couldn't read the process table.\n"));
    };
    let (signal, pids, mut out) = match plan {
        Plan::Done(out) => return out,
        Plan::Signal { signal, pids, out } => (signal, pids, out),
    };
    if pids.is_empty() {
        out.code = out.code.max(1);
        return out;
    }
    let me = std::process::id() as i32;
    let by_pid: HashMap<i32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let owner = |pid: i32| by_pid.get(&pid).and_then(|p| owners.of(p));
    // The URL names the agent, but only a caller in one of its sessions (on Windows, in its job) speaks for it: a
    // command holding another agent's kill URL (a stale environment, a hand-written curl, a shell in its workspace)
    // gets none of that agent's processes as its own.
    let inside = caller > 0 && caller_agent(&owners, caller) == Some(&a.id);
    // Signal 0 only checks the process exists. A PID with no process gets the tool's own error.
    let (own, foreign): (Vec<i32>, Vec<i32>) =
        pids.into_iter().partition(|&pid| signal == 0 || gone(pid, &by_pid) || (inside && pid != me && owner(pid) == Some(&a.id)));
    send(&own, signal, &tool, &mut out);
    if foreign.is_empty() {
        return out;
    }

    let names: HashMap<String, String> = state.db.list_agents().unwrap_or_default().into_iter().map(|a| (a.id, a.name)).collect();
    let commands: HashMap<i32, &str> = procs.iter().map(|p| (p.pid, p.command.as_str())).collect();
    let whose = |pid: i32| match owner(pid) {
        _ if pid == me => "Loft".to_string(),
        Some(id) => format!("{}'s", names.get(id).map(String::as_str).unwrap_or("another agent")),
        None => "not an agent's".to_string(),
    };
    const SHOWN: usize = 5;
    let mut list: Vec<String> = foreign
        .iter()
        .take(SHOWN)
        .map(|&pid| format!("{pid} ({}: {})", whose(pid), clip(commands.get(&pid).copied().unwrap_or("?"), 80)))
        .collect();
    if foreign.len() > SHOWN {
        list.push(format!("and {} more", foreign.len() - SHOWN));
    }
    // Their programs, unless one is another agent's live process, Loft's own, or not asked about as this agent's run.
    let exes: Option<Vec<String>> = (inside && foreign.iter().all(|&pid| pid != me && owner(pid).is_none())).then(|| foreign.iter().filter_map(|&pid| exe_of(pid)).collect()).filter(|e: &Vec<String>| e.len() == foreign.len());
    if exes.as_ref().is_some_and(|e| e.iter().all(|x| rule_allows(&a.allowed_tools, x))) {
        send(&foreign, signal, &tool, &mut out);
        return out;
    }
    // Offered only when saving it would stop the question: one program, a path that makes a rule.
    let rule = exes
        .and_then(|e| e.iter().map(|x| kill_rule(x)).collect::<Option<HashSet<_>>>())
        .and_then(|r| if r.len() == 1 { r.into_iter().next() } else { None })
        .unwrap_or_default();
    let n = foreign.len();
    let es = if n == 1 { "" } else { "es" };
    let reason = if inside {
        format!("Stops {n} process{es} {} didn't start and that run outside its workspace: {}.", a.name, list.join(", "))
    } else {
        format!("A command outside {}'s runs (PID {caller}) asks to stop {n} process{es}: {}.", a.name, list.join(", "))
    };
    let input = json!({ "command": command, "pids": foreign });
    let d = crate::agents::ask_permission_for(state, &a.id, "prompt", "Kill", input, &reason, rule, ASK_TIMEOUT).await;
    if d.allow {
        send(&foreign, signal, &tool, &mut out);
    } else {
        let pids = foreign.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" ");
        let why = d.message.unwrap_or_else(|| "The user denied it.".into());
        let because = if inside {
            format!("your runs didn't start {} and {} outside your workspace", if n == 1 { "it" } else { "them" }, if n == 1 { "it runs" } else { "they run" })
        } else {
            "this command doesn't run in one of your runs".to_string()
        };
        out.stderr += &format!("{tool}: Loft didn't stop {pids}: {because}, so it asked the user. {why} Stop only your own processes.\n");
        out.code = 1;
    }
    out
}

/// The agent a kill's caller runs for: the one whose session it is in, on Windows whose job.
fn caller_agent(owners: &Owners, caller: i32) -> Option<&String> {
    #[cfg(windows)]
    return owners.jobs.get(&caller);
    #[cfg(not(windows))]
    session_of(caller).and_then(|sid| owners.sessions.get(&sid))
}

/// Whether no process has the PID. On Windows, one that isn't in the table taken and isn't running either (it may
/// have started since).
fn gone(pid: i32, table: &HashMap<i32, &Proc>) -> bool {
    #[cfg(windows)]
    return !table.contains_key(&pid) && signal(pid, 0).is_err();
    #[cfg(not(windows))]
    {
        let _ = table;
        session_of(pid).is_none()
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

/// Sends `signal` to each PID, noting failures the way kill does, and the way taskkill does its successes too.
fn send(pids: &[i32], signal: i32, tool: &str, out: &mut Output) {
    for &pid in pids {
        let Err(e) = self::signal(pid, signal) else {
            if tool == "taskkill" {
                out.stdout += &format!("SUCCESS: The process with PID {pid} has been terminated.\n");
            }
            continue;
        };
        let why = match e.raw_os_error() {
            #[cfg(unix)]
            Some(libc::ESRCH) => "No such process".to_string(),
            #[cfg(unix)]
            Some(libc::EPERM) => "Operation not permitted".to_string(),
            // ERROR_INVALID_PARAMETER: OpenProcess has no such process. ERROR_ACCESS_DENIED.
            #[cfg(windows)]
            Some(87) => "No such process".to_string(),
            #[cfg(windows)]
            Some(5) => "Operation not permitted".to_string(),
            _ => e.to_string(),
        };
        out.stderr += &if tool == "taskkill" { format!("ERROR: The process with PID {pid} could not be terminated.\nReason: {why}.\n") } else { format!("{tool}: {pid}: {why}\n") };
        out.code = 1;
    }
}

fn plan(req: &KillRequest, procs: &[Proc]) -> Plan {
    let caller_side = ancestors(procs, req.caller);
    // Without a directory that exists, Loft's own, as before.
    let cwd = req.cwd.as_deref().filter(|d| d.is_absolute() && d.is_dir());
    #[cfg(windows)]
    {
        let _ = cwd;
        let named: Vec<by_name::Named> = procs
            .iter()
            .map(|p| by_name::Named { pid: p.pid, ppid: p.ppid, age_secs: p.age_secs, name: p.exe.rsplit(['\\', '/']).next().unwrap_or_default(), command: &p.command })
            .collect();
        match req.tool.as_str() {
            "kill" => plan_kill(&req.args, req.caller, procs),
            "pkill" => by_name::pkill(&req.args, &named, &caller_side),
            "killall" => by_name::killall(&req.args, &named, &caller_side),
            "taskkill" => by_name::taskkill(&req.args, &named, &caller_side),
            t => Plan::Done(Output::error(2, format!("{t}: not a kill command\n"))),
        }
    }
    #[cfg(not(windows))]
    match req.tool.as_str() {
        "kill" => plan_kill(&req.args, req.caller, procs),
        "pkill" => plan_pkill(&req.args, &caller_side, cwd),
        "killall" => plan_killall(&req.args, &caller_side, cwd),
        t => Plan::Done(Output::error(2, format!("{t}: not a kill command\n"))),
    }
}

const KILL_USAGE: &str = "usage: kill [-s signal_name] pid ...\n       kill -l [exit_status]\n       kill -signal_name pid ...\n       kill -signal_number pid ...\n";

/// `kill [-s sig | -n num | -sig] [--] pid...`: a PID, `0` (the caller's process group), `-1` (every process the
/// user owns but the caller) or `-<pgid>` (a process group).
fn plan_kill(args: &[String], caller: i32, procs: &[Proc]) -> Plan {
    let bad_signal = |s: &str| Plan::Done(Output::error(2, format!("kill: unknown signal {s}; valid signals:\n{}\n", names())));
    let mut signal = SIGTERM;
    let mut rest = args;
    match args.first().map(String::as_str) {
        None => return Plan::Done(Output::error(2, KILL_USAGE.into())),
        // Windows has no /bin/kill to list its signals: Loft's few.
        Some("-l" | "-L") if cfg!(windows) => return Plan::Done(Output { stdout: format!("{}\n", names()), ..Default::default() }),
        Some("-l" | "-L") => return Plan::Done(run("/bin/kill", args, None)),
        Some("-s" | "-n") => {
            let Some(s) = args.get(1) else { return Plan::Done(Output::error(2, KILL_USAGE.into())) };
            let Some(sig) = signal_of(s) else { return bad_signal(s) };
            (signal, rest) = (sig, &args[2..]);
        }
        Some(a) if a.starts_with('-') && a != "--" => {
            let Some(sig) = signal_of(&a[1..]) else { return bad_signal(&a[1..]) };
            (signal, rest) = (sig, &args[1..]);
        }
        _ => {}
    }
    if rest.first().is_some_and(|a| a == "--") {
        rest = &rest[1..];
    }
    if rest.is_empty() {
        return Plan::Done(Output::error(2, KILL_USAGE.into()));
    }
    let mut out = Output::default();
    let mut pids = vec![];
    let group = |pgid: Option<i32>| -> Vec<i32> { procs.iter().map(|p| p.pid).filter(|&p| pgid.is_some() && group_of(p) == pgid).collect() };
    let uid = crate::sys::uid();
    for a in rest {
        let found = match a.parse::<i32>() {
            Ok(pid) if pid > 0 => vec![pid],
            // Windows has no process groups, and every process there would count as the user's.
            Ok(_) if cfg!(windows) => {
                out.stderr += &format!("kill: {a}: Windows has no process groups: name each process by its PID\n");
                out.code = 1;
                continue;
            }
            Ok(0) => group(group_of(caller)),
            Ok(-1) => procs.iter().filter(|p| p.uid == uid && p.pid != caller && p.pid > 1).map(|p| p.pid).collect(),
            Ok(n) => group(Some(-n)),
            Err(_) => {
                out.stderr += &format!("kill: illegal process id: {a}\n");
                out.code = 1;
                continue;
            }
        };
        if found.is_empty() {
            out.stderr += &format!("kill: {a}: No such process\n");
            out.code = 1;
        }
        pids.extend(found.into_iter().filter(|p| !pids.contains(p)).collect::<Vec<_>>());
    }
    Plan::Signal { signal, pids, out }
}

fn names() -> String {
    SIGNALS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" ")
}

/// `pkill [-signal] <pgrep options> pattern`: the processes `pgrep` finds with the same options, but the caller and
/// what it runs under, which pkill leaves out too.
#[cfg_attr(windows, allow(dead_code))]
fn plan_pkill(args: &[String], caller_side: &HashSet<i32>, cwd: Option<&Path>) -> Plan {
    let (signal, rest) = match args.first().and_then(|a| signal_of(a.strip_prefix('-')?)) {
        Some(sig) => (sig, &args[1..]),
        None => (SIGTERM, args),
    };
    // `-I` asks before each kill, which no one can answer here.
    let rest: Vec<String> = rest.iter().filter(|a| *a != "-I").cloned().collect();
    let found = run("/usr/bin/pgrep", &rest, cwd);
    if found.code != 0 {
        return Plan::Done(Output { stdout: String::new(), stderr: found.stderr.replace("pgrep", "pkill"), code: found.code });
    }
    let pids = found.stdout.lines().filter_map(|l| field(l).0.parse().ok()).filter(|p| !caller_side.contains(p)).collect();
    Plan::Signal { signal, pids, out: Output::default() }
}

/// `killall [-signal] [options] name...`: what `killall -s` (show only) lists with the same options, but the caller
/// and what it runs under. `-s` and `-l` themselves only print.
#[cfg_attr(windows, allow(dead_code))]
fn plan_killall(args: &[String], caller_side: &HashSet<i32>, cwd: Option<&Path>) -> Plan {
    if args.iter().any(|a| a == "-s" || a == "-l") {
        return Plan::Done(run("/usr/bin/killall", args, cwd));
    }
    let shown = run("/usr/bin/killall", &[&["-s".to_string()], args].concat(), cwd);
    if shown.code != 0 {
        return Plan::Done(shown);
    }
    // One `kill -<signal> <pid>` line a process.
    let mut signal = SIGTERM;
    let mut pids = vec![];
    for line in shown.stdout.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let ["kill", sig, pid] = words[..]
            && let (Some(s), Ok(p)) = (signal_of(sig.trim_start_matches('-')), pid.parse::<i32>()) {
                signal = s;
                if !caller_side.contains(&p) {
                    pids.push(p);
                }
            }
    }
    Plan::Signal { signal, pids, out: Output::default() }
}

/// Runs a command that only looks processes up or prints (`pgrep`, `killall -s`, `kill -l`), in `cwd` if given.
fn run(bin: &str, args: &[String], cwd: Option<&Path>) -> Output {
    let mut cmd = std::process::Command::new(bin);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    match cmd.args(args).output() {
        Ok(o) => Output {
            code: o.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&o.stdout).into(),
            stderr: String::from_utf8_lossy(&o.stderr).into(),
        },
        Err(e) => Output::error(1, format!("{bin}: {e}\n")),
    }
}

// Unix only: they start processes in sessions of their own and signal them.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::agents::tests::{fake_claude_state, until};
    use crate::agents::{enqueue, is_running, workspace};
    use std::os::unix::process::CommandExt;
    use std::sync::Arc;

    // Every process these tests signal is one they start themselves, by PID.

    /// A `sleep` the test starts in a process session of its own, stopped by PID when dropped.
    /// A sleep the test started: its child, or an orphan (a daemon) the test only knows by PID.
    struct Sleeper(Option<std::sync::Mutex<std::process::Child>>, i32);

    impl Sleeper {
        fn start(own_session: bool) -> Self {
            Self::start_as(Path::new("/bin/sleep"), own_session)
        }

        /// From `path`, a link to sleep: killall finds a process by that name.
        fn start_as(path: &Path, own_session: bool) -> Self {
            Self::spawn(path, None, own_session)
        }

        /// A daemon: a session of its own that no run recorded, working in `dir`, left by the shell that started
        /// it, so it runs under launchd and not under Loft (here the test).
        fn daemon_in(dir: &Path) -> Self {
            Self::daemon_as(Path::new("/bin/sleep"), dir)
        }

        /// [`Self::daemon_in`], from `path`, a link to sleep.
        fn daemon_as(path: &Path, dir: &Path) -> Self {
            std::fs::create_dir_all(dir).unwrap();
            let mut cmd = std::process::Command::new("/bin/sh");
            cmd.args(["-c", "\"$0\" 60 >/dev/null 2>&1 & echo $!"]).arg(path).current_dir(dir);
            unsafe { cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) }) };
            let out = cmd.output().unwrap();
            let pid = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
            // Until the shell's exit reaches the table, the sleep still runs under it.
            for _ in 0..100 {
                if table().iter().any(|p| p.pid == pid && !p.under_app) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Sleeper(None, pid)
        }

        fn spawn(path: &Path, dir: Option<&Path>, own_session: bool) -> Self {
            let mut cmd = std::process::Command::new(path);
            cmd.arg("60");
            if let Some(dir) = dir {
                cmd.current_dir(dir);
            }
            if own_session {
                unsafe { cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) }) };
            }
            let child = cmd.spawn().unwrap();
            let pid = child.id() as i32;
            Sleeper(Some(std::sync::Mutex::new(child)), pid)
        }

        fn pid(&self) -> i32 {
            self.1
        }

        fn alive(&self) -> bool {
            match &self.0 {
                Some(child) => child.lock().unwrap().try_wait().unwrap().is_none(),
                None => alive(self.1),
            }
        }

        fn stop(&self) {
            match &self.0 {
                Some(child) => {
                    let mut child = child.lock().unwrap();
                    let _ = child.kill();
                    let _ = child.wait();
                }
                None if self.alive() => unsafe {
                    libc::kill(self.1, libc::SIGKILL);
                },
                None => {}
            }
        }
    }

    impl Drop for Sleeper {
        fn drop(&mut self) {
            self.stop();
        }
    }

    /// Whether `pid` is a process (not one the test has to reap).
    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_idle_agent_s_processes_are_stopped_after_the_timeout_never_during_a_run_and_only_its_own() {
        let st = crate::tasks::tests::state();
        let storage = st.cfg.storage_dir.clone();
        let a = crate::tasks::tests::agent(&st, "Builder");
        let server = Sleeper::start(true);
        record(&storage, &a.id, server.pid() as u32).unwrap();
        let untracked = Sleeper::start(true);
        let ago = |h: i64| Some((chrono::Utc::now() - chrono::Duration::hours(h)).to_rfc3339());

        // Started 3 hours ago: under the 4-hour default.
        st.db.update_agent(&a.id, |x| x.last_run_at = ago(3)).unwrap();
        assert!(stop_idle(&st).await.is_empty());
        // Started 5 hours ago, but still running.
        st.db.update_agent(&a.id, |x| x.last_run_at = ago(5)).unwrap();
        st.rt.lock().unwrap().running.insert(a.id.clone(), Default::default());
        assert!(stop_idle(&st).await.is_empty(), "never during a run");
        assert!(server.alive());

        st.rt.lock().unwrap().running.remove(&a.id);
        let lines = stop_idle(&st).await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        let pid = server.pid();
        assert_eq!((lines[0].agent.as_str(), lines[0].done.as_str(), lines[0].command.clone()), (a.id.as_str(), "Stopped process", format!("kill {pid}")));
        assert!(lines[0].rel.contains("sleep 60"), "{}", lines[0].rel);
        until("the server stops", 5, || !server.alive()).await;
        assert!(untracked.alive(), "a process Loft didn't track for the agent stays");

        crate::sweep::record(&st, "Idle processes", lines).await;
        let Json(log) = crate::sweep::log(State(st.clone())).await;
        assert_eq!((log.what.as_str(), log.removed, log.lines[0].done.as_str()), ("Idle processes", 1, "Stopped process"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_daemon_in_an_idle_agent_s_workspace_is_stopped_after_the_timeout_never_during_a_run_and_clean_lists_it() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Builder");
        let b = crate::tasks::tests::agent(&st, "Other");
        // Deep in the workspace, like `pg_ctl -D data start` from a clone.
        let daemon = Sleeper::daemon_in(&crate::agents::workspace(&st, &a.id).join("app/pgdata"));
        let outside = Sleeper::daemon_in(&crate::testdir::for_this_test("outside"));
        // In the other agent's session, though it works in this one's workspace: the session's agent owns it.
        let borrowed = Sleeper::daemon_in(&crate::agents::workspace(&st, &a.id));
        record(&st.cfg.storage_dir, &b.id, session_of(borrowed.pid()).unwrap() as u32).unwrap();
        let ago = |h: i64| Some((chrono::Utc::now() - chrono::Duration::hours(h)).to_rfc3339());

        // Clean lists it as the idle agent's, and the other agent's process as the other's.
        let Json(list) = left_running(State(st.clone())).await;
        let pids = |id: &str| list.get(id).and_then(Value::as_array).map(|ps| ps.iter().map(|p| p["pid"].as_i64().unwrap() as i32).collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!((pids(&a.id), pids(&b.id)), (vec![daemon.pid()], vec![borrowed.pid()]));
        let rows = crate::sweep::sizes(State(st.clone())).await.unwrap().0;
        let row = rows["agents"].as_array().unwrap().iter().find(|r| r["id"] == json!(a.id)).unwrap().clone();
        assert_eq!((&row["busy"], &row["processes"]), (&json!(false), &json!(1)));

        // In a run past the timeout: busy, and nothing stops.
        st.db.update_agent(&a.id, |x| x.last_run_at = ago(5)).unwrap();
        st.db.update_agent(&b.id, |x| x.last_run_at = ago(1)).unwrap();
        st.rt.lock().unwrap().running.insert(a.id.clone(), Default::default());
        let rows = crate::sweep::sizes(State(st.clone())).await.unwrap().0;
        assert_eq!(rows["agents"].as_array().unwrap().iter().find(|r| r["id"] == json!(a.id)).unwrap()["busy"], json!(true));
        assert!(stop_idle(&st).await.is_empty(), "never during a run");
        assert!(daemon.alive());

        // Idle past the timeout: stopped, and in the clean log.
        st.rt.lock().unwrap().running.remove(&a.id);
        let lines = stop_idle(&st).await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!((lines[0].agent.as_str(), lines[0].command.clone()), (a.id.as_str(), format!("kill {}", daemon.pid())));
        until("the daemon stops", 5, || !daemon.alive()).await;
        crate::sweep::record(&st, "Idle processes", lines).await;
        let Json(log) = crate::sweep::log(State(st.clone())).await;
        assert_eq!((log.what.as_str(), log.removed), ("Idle processes", 1));
        assert!(outside.alive(), "a process outside every workspace isn't an agent's");
        assert!(borrowed.alive(), "the other agent isn't idle");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_process_with_a_terminal_in_the_workspace_is_the_user_s() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Builder");
        let daemon = Sleeper::daemon_in(&crate::agents::workspace(&st, &a.id));
        let owners = Owners::load(&st.cfg.storage_dir, std::slice::from_ref(&a.id));
        let p = table().into_iter().find(|p| p.pid == daemon.pid()).unwrap();
        assert_eq!(owners.of(&p), Some(&a.id));
        assert_eq!(owners.of(&Proc { tty: true, ..p }), None, "the user's shell, cd'ed into the workspace");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loft_s_own_child_in_the_workspace_is_not_the_agent_s_but_a_daemon_the_agent_started_is() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Builder");
        let ws = crate::agents::workspace(&st, &a.id);
        std::fs::create_dir_all(&ws).unwrap();
        // A git check or size scan: Loft's child (here the test's), in a session of its own, working in the clone.
        let check = Sleeper::spawn(Path::new("/bin/sleep"), Some(&ws), true);
        let daemon = Sleeper::daemon_in(&ws);
        let live = live_processes(&st.cfg.storage_dir, std::slice::from_ref(&a.id));
        let pids: Vec<i32> = live.get(&a.id).into_iter().flatten().map(|l| l.pid).collect();
        assert_eq!(pids, vec![daemon.pid()]);
        assert!(check.alive());
    }

    #[test]
    fn a_process_under_loft_or_a_gui_app_runs_under_an_app() {
        use super::{is_gui_app, mark_under_app};
        let code = "/Applications/Visual Studio Code.app/Contents/MacOS/Electron";
        let plugin = "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper (Plugin).app/Contents/MacOS/Code Helper (Plugin)";
        assert!(is_gui_app(code) && is_gui_app(plugin) && is_gui_app("/Applications/Cursor.app/Contents/MacOS/Cursor"));
        assert!(!is_gui_app("/Applications/Postgres.app/Contents/Versions/16/bin/postgres"), "a daemon from an app bundle");
        assert!(!is_gui_app("/usr/local/bin/node") && !is_gui_app("/opt/homebrew/bin/redis-server"));
        let home = std::env::var("HOME").unwrap();
        assert!(is_gui_app(&format!("{home}/Applications/IntelliJ IDEA Ultimate.app/Contents/MacOS/idea")), "a JetBrains Toolbox IDE");
        assert!(is_gui_app(&format!("{home}/Applications/RustRover.app/Contents/MacOS/fsnotifier")));
        assert!(!is_gui_app(&format!("{home}/Documents/Applications/Fake.app/Contents/MacOS/x")), "only ~/Applications itself");

        let p = |pid, ppid| Proc { pid, ppid, uid: 501, age_secs: 0, command: String::new(), helper: false, browser: false, tty: false, under_app: false, sessions_above: vec![] };
        // Loft 100 runs git 101; VS Code 200 runs its extension host 201, which runs a language server 202; a
        // daemon 300 runs under launchd and has a child 301.
        let mut procs = vec![p(100, 1), p(101, 100), p(200, 1), p(201, 200), p(202, 201), p(300, 1), p(301, 300)];
        mark_under_app(&mut procs, &HashSet::from([100, 200, 201]), Some);
        let under: Vec<i32> = procs.iter().filter(|p| p.under_app).map(|p| p.pid).collect();
        assert_eq!(under, vec![100, 101, 200, 201, 202]);
    }

    #[test]
    fn a_child_that_left_an_agent_s_session_is_the_agent_s() {
        use super::mark_under_app;
        let p = |pid, ppid| Proc { pid, ppid, uid: 501, age_secs: 0, command: String::new(), helper: false, browser: false, tty: false, under_app: false, sessions_above: vec![] };
        // Loft 100 runs claude 110, whose Bash shell 120 (session 120, recorded) runs a dev server 121; the dev server
        // starts a worker 130 in a session of its own (node `detached`), and the worker runs 131. Loft's git 101 is
        // in Loft's session.
        let mut procs = vec![p(100, 1), p(101, 100), p(110, 100), p(120, 110), p(121, 120), p(130, 121), p(131, 130)];
        let sid = |pid| Some(match pid { 120 | 121 => 120, 130 | 131 => 130, _ => 100 });
        mark_under_app(&mut procs, &HashSet::from([100]), sid);
        let above = |pid| procs.iter().find(|p| p.pid == pid).unwrap().sessions_above.clone();
        assert_eq!(above(131), vec![130, 120, 100], "nearest first, up to Loft");
        assert_eq!(above(130), vec![120, 100]);
        assert_eq!(above(101), Vec::<i32>::new());
        assert!(procs.iter().all(|p| p.under_app), "all of them run under Loft");

        // The live process (this test) stands in for the worker: its own session isn't recorded, one above is.
        let me = std::process::id() as i32;
        let owners = Owners { sessions: HashMap::from([(120, "a".to_string())]), workspaces: vec![], uid: unsafe { libc::getuid() } };
        let worker = Proc { pid: me, under_app: true, sessions_above: above(130), ..p(me, 1) };
        assert_eq!(owners.of(&worker), Some(&"a".to_string()), "the agent's, though it runs under Loft");
        let git = Proc { pid: me, under_app: true, sessions_above: above(101), ..p(me, 1) };
        assert_eq!(owners.of(&git), None, "Loft's own child is no one's");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_editor_helper_working_in_the_workspace_is_not_the_agent_s() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Builder");
        let daemon = Sleeper::daemon_in(&crate::agents::workspace(&st, &a.id));
        let owners = Owners::load(&st.cfg.storage_dir, std::slice::from_ref(&a.id));
        let p = table().into_iter().find(|p| p.pid == daemon.pid()).unwrap();
        assert_eq!(owners.of(&p), Some(&a.id));
        assert_eq!(owners.of(&Proc { under_app: true, ..p }), None, "a language server VS Code runs in the clone");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_stops_a_daemon_in_its_workspace_without_asking_and_deleting_it_stops_them() {
        let (st, a) = fake_claude_state();
        let ws = crate::agents::workspace(&st, &a.id);
        let daemon = Sleeper::daemon_in(&ws.join("redis"));
        let v = ask(&st, &a.id, "kill", &[&daemon.pid().to_string()]).await;
        assert_eq!(v["code"], 0, "{v}");
        until("the daemon to stop", 5, || !daemon.alive()).await;
        assert!(st.db.list_permissions(Some(&a.id)).unwrap().is_empty(), "it asked the user");

        // A shell in the workspace isn't in the agent's runs: it doesn't speak for the agent.
        let daemon = Sleeper::daemon_in(&ws.join("redis"));
        let shell = Sleeper::daemon_in(&ws);
        let asking = tokio::spawn({
            let (st, id, pid, caller) = (st.clone(), a.id.clone(), daemon.pid().to_string(), shell.pid());
            async move { ask_as(&st, &id, caller, "kill", &[&pid]).await }
        });
        until("a permission request", 5, || pending(&st, &a.id).len() == 1).await;
        let req = pending(&st, &a.id).remove(0);
        crate::agents::decide(&st, &req.id, "deny", None, None, None).unwrap();
        assert_eq!(asking.await.unwrap()["code"], 1);
        assert!(daemon.alive());

        let outside = Sleeper::daemon_in(&crate::testdir::for_this_test("outside"));
        let uri: axum::http::Uri = "/?purge_files=false".parse().unwrap();
        let opts = axum::extract::Query::try_from_uri(&uri).unwrap();
        let _ = crate::api::delete_agent(State(st.clone()), UrlPath(a.id.clone()), opts).await.ok().unwrap();
        until("its daemons to stop", 5, || !daemon.alive() && !shell.alive()).await;
        assert!(outside.alive(), "it stopped a process outside the workspace");
    }

    #[test]
    fn a_test_browser_is_playwright_s_or_chrome_for_testing_never_the_user_s_chrome() {
        let cache = "/Users/u/Library/Caches/ms-playwright";
        let cft = "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";
        assert!(is_test_browser(&format!("{cache}/chromium-1234/chrome-mac-arm64/{cft}")));
        assert!(is_test_browser(&format!("{cache}/chromium_headless_shell-1234/chrome-headless-shell-mac-arm64/chrome-headless-shell")));
        let helper = "Google Chrome for Testing Helper (Renderer).app/Contents/MacOS/Google Chrome for Testing Helper (Renderer)";
        assert!(is_test_browser(&format!("/Users/u/.cache/puppeteer/chrome/mac_arm-131/Frameworks/{helper}")));
        assert!(!is_test_browser("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
        assert!(!is_test_browser("/Applications/Google Chrome.app/Contents/Frameworks/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)"));
        assert!(!is_test_browser("/usr/local/bin/node"));
    }

    // macOS's ps, killall and pkill: Linux's `ps -o comm` shows no path and its killall has no `-s` to only show.
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_s_end_stops_only_its_test_browsers_and_the_next_run_hears_of_it() {
        let (st, a) = fake_claude_state();
        let b = crate::tasks::tests::agent(&st, "Other");
        let root = st.cfg.root.as_path().to_path_buf();
        let link = |rel: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink("/bin/sleep", &path).unwrap();
            path
        };
        let cft = link("ms-playwright/chromium-1234/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing");
        let shell = link("ms-playwright/chromium_headless_shell-1234/chrome-headless-shell-mac-arm64/chrome-headless-shell");
        let chrome = link("Google Chrome.app/Contents/MacOS/Google Chrome");
        // A Chrome for Testing that runs a renderer helper: `sh` under the browser's name, starting a `sleep` under
        // the helper's.
        let sh = root.join("ms-playwright/chromium-1234/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/sh");
        std::os::unix::fs::symlink("/bin/sh", &sh).unwrap();
        let renderer = link("ms-playwright/chromium-1234/chrome-mac-arm64/Google Chrome for Testing Helper (Renderer)");
        let mut cmd = std::process::Command::new(&sh);
        cmd.arg0(&cft).arg("-c").arg("\"$0\" 60 & wait").arg(&renderer);
        unsafe { cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) }) };
        let child = cmd.spawn().unwrap();
        let pid = child.id() as i32;
        let browser = Sleeper(Some(std::sync::Mutex::new(child)), pid);
        until("the renderer to start", 5, || table().iter().any(|p| p.ppid == browser.pid() && p.browser)).await;
        let helper = table().into_iter().find(|p| p.ppid == browser.pid()).unwrap().pid;
        // The agent's: two test browsers its scripts left open (one with its renderer), and a `nohup` dev server.
        let (headless, server) = (Sleeper::start_as(&shell, true), Sleeper::start(true));
        for p in [&browser, &headless, &server] {
            record(&st.cfg.storage_dir, &a.id, p.pid() as u32).unwrap();
        }
        // The user's own Chrome, a test browser in no agent's session though it works in the agent's workspace,
        // and another agent's test browser in a run still going.
        let users = Sleeper::start_as(&chrome, true);
        let stray = Sleeper::daemon_as(&cft, &workspace(&st, &a.id));
        let others = Sleeper::start_as(&cft, true);
        record(&st.cfg.storage_dir, &b.id, others.pid() as u32).unwrap();
        st.rt.lock().unwrap().running.insert(b.id.clone(), Arc::new(tokio::sync::Notify::new()));

        enqueue(&st, &a.id, Some("hello".into()), "user");
        until("the run to end", 30, || !is_running(&st, &a.id)).await;
        until("its test browsers to stop", 5, || !browser.alive() && !headless.alive() && !alive(helper)).await;
        assert!(server.alive(), "it stopped the dev server");
        assert!(users.alive(), "it stopped the user's Chrome");
        assert!(stray.alive(), "it stopped a browser in no session of the agent's");
        assert!(others.alive(), "it stopped another agent's browser");

        enqueue(&st, &a.id, Some("again".into()), "user");
        until("the next run to end", 30, || !is_running(&st, &a.id)).await;
        let prompts = std::fs::read_to_string(workspace(&st, &a.id).join("prompts.log")).unwrap();
        let second = prompts.split("\n---\n").nth(1).unwrap();
        assert!(second.contains("Loft closed 2 test browsers left from your last run"), "{second}");
        st.rt.lock().unwrap().running.remove(&b.id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_leaves_the_processes_when_the_agent_is_no_longer_idle_by_then() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Builder");
        let server = Sleeper::start(true);
        record(&st.cfg.storage_dir, &a.id, server.pid() as u32).unwrap();
        assert!(stop(&st.cfg.storage_dir, &a.id, || false, false).await.is_empty());
        assert!(server.alive());
        assert_eq!(stop(&st.cfg.storage_dir, &a.id, || true, false).await.len(), 1);
        until("the server stops", 5, || !server.alive()).await;
    }

    #[test]
    fn a_name_in_the_arguments_does_not_make_a_helper() {
        // Security's probes: the helper's name and `.app/` only after the executable.
        let mut kids: Vec<_> = ["x.app/DTServiceHub", "--tag=.app/DTServiceHub"]
            .iter()
            .map(|probe| std::process::Command::new("/bin/sh").args(["-c", "sleep 30; :", probe]).spawn().unwrap())
            .collect();
        let t = super::table();
        for k in &kids {
            let p = t.iter().find(|p| p.pid == k.id() as i32).expect("in the table");
            assert!(p.command.contains("DTServiceHub") && !p.helper, "{}", p.command);
        }
        for k in &mut kids {
            let _ = k.kill();
            let _ = k.wait();
        }
    }

    #[test]
    fn a_helper_is_its_executable_in_an_app_never_the_arguments() {
        use super::is_helper;
        assert!(is_helper("/Applications/Xcode.app/Contents/SharedFrameworks/DTServiceHub"));
        assert!(is_helper("/Applications/Xcode beta.app/Contents/Developer/DTServiceHub"));
        assert!(!is_helper("/usr/bin/DTServiceHub"));
        assert!(!is_helper("/bin/sleep"));
        assert!(!is_helper("/Applications/Xcode.app/Contents/bin/node"));
    }

    #[test]
    fn ps_elapsed_time_is_read_in_seconds() {
        assert_eq!([etime("00:07"), etime("12:34"), etime("3:04:05"), etime("2-03:04:05"), etime("x")], [Some(7), Some(754), Some(11045), Some(183845), None]);
    }

    // macOS's ps, killall and pkill: Linux's `ps -o comm` shows no path and its killall has no `-s` to only show.
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread")]
    async fn clean_locks_only_an_agent_in_a_run_lists_what_an_idle_one_left_and_stops_it_but_never_xcode_s_helpers() {
        let st = crate::tasks::tests::state();
        let storage = st.cfg.storage_dir.clone();
        let a = crate::tasks::tests::agent(&st, "Builder");
        let server = Sleeper::start(true);
        record(&storage, &a.id, server.pid() as u32).unwrap();
        // Xcode's DTServiceHub, in the agent's session too.
        let dir = crate::testdir::for_this_test("helpers").join("Xcode beta.app");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink("/bin/sleep", dir.join("DTServiceHub")).unwrap();
        let helper = Sleeper::start_as(&dir.join("DTServiceHub"), true);
        record(&storage, &a.id, helper.pid() as u32).unwrap();
        let row = |v: &Value| v["agents"].as_array().unwrap().iter().find(|r| r["id"] == json!(a.id)).unwrap().clone();
        let sizes = || async { crate::sweep::sizes(State(st.clone())).await.unwrap().0 };

        // Idle, with a server and a helper: not busy, and only the server counts.
        let r = row(&sizes().await);
        assert_eq!((&r["busy"], &r["processes"]), (&json!(false), &json!(1)));
        let Json(list) = left_running(State(st.clone())).await;
        let ps = list[&a.id].as_array().unwrap();
        assert_eq!((ps.len(), &ps[0]["pid"], ps[0]["command"].as_str().unwrap().contains("sleep 60")), (1, &json!(server.pid()), true));

        // In a run: busy, nothing listed, and Stop them is refused.
        st.rt.lock().unwrap().running.insert(a.id.clone(), Default::default());
        let r = row(&sizes().await);
        assert_eq!((&r["busy"], &r["processes"]), (&json!(true), &json!(0)));
        assert!(left_running(State(st.clone())).await.0.as_object().unwrap().is_empty());
        let err = stop_left_running(State(st.clone()), UrlPath(a.id.clone())).await.unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::CONFLICT);
        assert!(server.alive());
        // A path that only resolves to the agent's id is no agent: 404, and nothing stops.
        for odd in [format!("./{}", a.id), format!("bin/../{}", a.id)] {
            st.rt.lock().unwrap().running.remove(&a.id);
            let err = stop_left_running(State(st.clone()), UrlPath(odd)).await.unwrap_err();
            assert_eq!(err.0, axum::http::StatusCode::NOT_FOUND);
            st.rt.lock().unwrap().running.insert(a.id.clone(), Default::default());
        }
        assert!(server.alive());

        // Idle again: Stop them stops the server, logs it, and leaves the helper.
        st.rt.lock().unwrap().running.remove(&a.id);
        let Json(done) = stop_left_running(State(st.clone()), UrlPath(a.id.clone())).await.unwrap();
        assert_eq!(done, json!({ "stopped": 1 }));
        until("the server stops", 5, || !server.alive()).await;
        assert!(helper.alive(), "Xcode's helper stays");
        let axum::Json(log) = crate::sweep::log(State(st.clone())).await;
        assert_eq!((log.what.as_str(), log.removed), ("Stopped processes", 1));
        assert!(row(&sizes().await)["processes"] == json!(0));
    }

    /// A kill from a command in one of the agent's sessions, as `loft kill-as` sends it.
    async fn ask(st: &AppState, agent_id: &str, tool: &str, args: &[&str]) -> Value {
        let caller = Sleeper::start(true);
        record(&st.cfg.storage_dir, agent_id, caller.pid() as u32).unwrap();
        ask_as(st, agent_id, caller.pid(), tool, args).await
    }

    async fn ask_as(st: &AppState, agent_id: &str, caller: i32, tool: &str, args: &[&str]) -> Value {
        ask_in(st, agent_id, caller, None, tool, args).await
    }

    async fn ask_in(st: &AppState, agent_id: &str, caller: i32, cwd: Option<&Path>, tool: &str, args: &[&str]) -> Value {
        let req = KillRequest { tool: tool.into(), args: args.iter().map(|a| a.to_string()).collect(), caller, cwd: cwd.map(Path::to_path_buf) };
        handle(State(st.clone()), UrlPath(agent_id.into()), Json(req)).await.0
    }

    fn pending(st: &AppState, agent_id: &str) -> Vec<crate::db::PermissionRequest> {
        st.db.list_permissions(Some(agent_id)).unwrap().into_iter().filter(|p| p.status == "pending").collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_run_gets_its_own_process_session_and_the_agent_records_it() {
        let (st, a) = fake_claude_state();
        let b = crate::tasks::tests::agent(&st, "Other");
        for id in [&a.id, &b.id] {
            enqueue(&st, id, Some("busy".into()), "user");
        }
        let storage = st.cfg.storage_dir.clone();
        until("both runs to record a session", 60, || sessions(&storage, &a.id).len() == 1 && sessions(&storage, &b.id).len() == 1).await;
        let (sa, sb) = (*sessions(&storage, &a.id).iter().next().unwrap(), *sessions(&storage, &b.id).iter().next().unwrap());
        assert_ne!(sa, sb);
        for sid in [sa, sb] {
            // Claude leads the session: its PID is the id, and it left Loft's process group, so a group kill
            // (`kill 0`, `kill -- -<pgid>`) from an agent's command can't reach Loft.
            assert_eq!(session_of(sid), Some(sid));
            assert_eq!(unsafe { libc::getpgid(sid) }, sid);
            assert_ne!(unsafe { libc::getpgid(sid) }, unsafe { libc::getpgid(0) });
            assert_ne!(session_of(sid), session_of(std::process::id() as i32));
        }
        // A second run of the same agent adds its own.
        crate::agents::stop(&st, &a.id);
        until("the run to stop", 60, || !is_running(&st, &a.id)).await;
        enqueue(&st, &a.id, Some("busy".into()), "user");
        until("the second session", 60, || sessions(&storage, &a.id).len() == 2).await;
        crate::agents::stop(&st, &a.id);
        crate::agents::stop(&st, &b.id);
        until("the runs to stop", 60, || !is_running(&st, &a.id) && !is_running(&st, &b.id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_stops_its_own_processes_without_asking() {
        let (st, a) = fake_claude_state();
        let server = Sleeper::start(true);
        record(&st.cfg.storage_dir, &a.id, server.pid() as u32).unwrap();
        // Checking it is there sends nothing.
        let v = ask(&st, &a.id, "kill", &["-0", &server.pid().to_string()]).await;
        assert_eq!(v["code"], 0, "{v}");
        assert!(server.alive());
        let v = ask(&st, &a.id, "kill", &["-s", "KILL", &server.pid().to_string()]).await;
        assert_eq!(v["code"], 0, "{v}");
        until("the server to stop", 5, || !server.alive()).await;
        assert!(st.db.list_permissions(Some(&a.id)).unwrap().is_empty(), "it asked the user");
        // A PID with no process gets kill's own error, and no question either.
        let v = ask(&st, &a.id, "kill", &[&server.pid().to_string()]).await;
        assert_eq!(v["code"], 1);
        assert!(v["stderr"].as_str().unwrap().contains("No such process"), "{v}");
        assert!(st.db.list_permissions(Some(&a.id)).unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_kill_from_outside_the_agents_sessions_asks_even_about_its_own_processes() {
        let (st, a) = fake_claude_state();
        let b = crate::tasks::tests::agent(&st, "Other");
        let server = Sleeper::start(true);
        record(&st.cfg.storage_dir, &a.id, server.pid() as u32).unwrap();
        // A command of another agent's holding this one's kill URL, then one of no agent's (Loft's own session).
        let elsewhere = Sleeper::start(true);
        record(&st.cfg.storage_dir, &b.id, elsewhere.pid() as u32).unwrap();
        let pid = server.pid().to_string();
        for caller in [elsewhere.pid(), std::process::id() as i32, 0] {
            let asking = tokio::spawn({
                let (st, id, pid) = (st.clone(), a.id.clone(), pid.clone());
                async move { ask_as(&st, &id, caller, "kill", &["-9", &pid]).await }
            });
            until("a permission request", 5, || pending(&st, &a.id).len() == 1).await;
            let req = pending(&st, &a.id).remove(0);
            assert!(req.reason.contains(&format!("A command outside {}'s runs (PID {caller})", a.name)), "{}", req.reason);
            assert!(req.reason.contains(&format!("{pid} ({}'s:", a.name)), "{}", req.reason);
            assert!(server.alive(), "it was stopped before the user answered");
            crate::agents::decide(&st, &req.id, "deny", None, None, None).unwrap();
            let v = asking.await.unwrap();
            assert_eq!(v["code"], 1, "{v}");
            assert!(v["stderr"].as_str().unwrap().contains("doesn't run in one of your runs"), "{v}");
            assert!(server.alive(), "caller {caller} stopped the agent's server");
        }
        // From its own session, the same kill needs no answer.
        let v = ask(&st, &a.id, "kill", &["-9", &pid]).await;
        assert_eq!(v["code"], 0, "{v}");
        until("the server to stop", 5, || !server.alive()).await;
    }

    // macOS's ps, killall and pkill: Linux's `ps -o comm` shows no path and its killall has no `-s` to only show.
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_kill_of_a_process_the_agent_does_not_own_runs_once_the_user_approves() {
        let (st, a) = fake_claude_state();
        let other = Sleeper::start(false);
        let pid = other.pid().to_string();
        let asking = tokio::spawn({
            let (st, id, pid) = (st.clone(), a.id.clone(), pid.clone());
            async move { ask(&st, &id, "kill", &[&pid]).await }
        });
        until("a permission request", 5, || pending(&st, &a.id).len() == 1).await;
        let req = pending(&st, &a.id).remove(0);
        assert_eq!(req.tool_name, "Kill");
        assert_eq!(req.input["command"], format!("kill {pid}"));
        assert!(req.reason.contains(&format!("{pid} (not an agent's: /bin/sleep 60)")), "{}", req.reason);
        assert!(other.alive(), "it was stopped before the user answered");
        // "Always allow" saves the program, not these processes.
        assert_eq!(req.rule, "Kill(/bin/sleep)");
        crate::agents::decide(&st, &req.id, "allow_always", None, None, None).unwrap();
        let v = asking.await.unwrap();
        assert_eq!(v["code"], 0, "{v}");
        until("the process to stop", 5, || !other.alive()).await;
        assert_eq!(st.db.get_permission(&req.id).unwrap().unwrap().status, "allowed_always");
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().allowed_tools, vec!["Kill(/bin/sleep)".to_string()]);
        // The same program, a new process: no question.
        let again = Sleeper::start(false);
        let v = ask(&st, &a.id, "kill", &[&again.pid().to_string()]).await;
        assert_eq!(v["code"], 0, "{v}");
        until("the second process to stop", 5, || !again.alive()).await;
        assert!(pending(&st, &a.id).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_saved_kill_rule_does_not_cover_another_program_or_another_agents_live_process() {
        let (st, a) = fake_claude_state();
        let b = crate::tasks::tests::agent(&st, "Reviewer");
        st.db.update_agent(&a.id, |a| a.allowed_tools = vec!["Kill(/usr/bin/cat)".into(), "Kill".into(), "Kill(*)".into()]).unwrap();
        // /bin/sleep isn't in the rules, and a bare `Kill` or `Kill(*)` covers nothing.
        let other = Sleeper::start(false);
        let pid = other.pid().to_string();
        let asking = tokio::spawn({
            let (st, id, pid) = (st.clone(), a.id.clone(), pid.clone());
            async move { ask(&st, &id, "kill", &[&pid]).await }
        });
        until("a permission request", 5, || pending(&st, &a.id).len() == 1).await;
        assert!(other.alive());
        crate::agents::decide(&st, &pending(&st, &a.id)[0].id, "deny", None, None, None).unwrap();
        assert_eq!(asking.await.unwrap()["code"], 1);
        // A process another agent runs right now asks even when the rule matches it.
        st.db.update_agent(&a.id, |a| a.allowed_tools = vec!["Kill(/bin/sleep)".into()]).unwrap();
        let theirs = Sleeper::start(true);
        record(&st.cfg.storage_dir, &b.id, theirs.pid() as u32).unwrap();
        let asking = tokio::spawn({
            let (st, id, pid) = (st.clone(), a.id.clone(), theirs.pid().to_string());
            async move { ask(&st, &id, "kill", &[&pid]).await }
        });
        until("a second permission request", 5, || pending(&st, &a.id).len() == 1).await;
        let req = pending(&st, &a.id).remove(0);
        assert!(theirs.alive());
        // It offers no Always: saving the rule wouldn't stop the question.
        assert_eq!(req.rule, "");
        assert!(crate::agents::decide(&st, &req.id, "allow_always", Some("Kill(*)".into()), None, None).is_err());
        // A rule posted with the answer doesn't give it one.
        assert!(crate::agents::decide(&st, &req.id, "allow_always", Some("Kill(/bin/sleep)".into()), None, None).is_err());
        assert!(st.db.get_agent(&a.id).unwrap().unwrap().allowed_tools == vec!["Kill(/bin/sleep)".to_string()], "nothing was added");
        crate::agents::decide(&st, &req.id, "deny", None, None, None).unwrap();
        assert_eq!(asking.await.unwrap()["code"], 1);
    }

    #[test]
    fn a_kill_rule_names_the_program_with_the_build_folder_wildcarded() {
        let exe = "/Users/me/Library/Developer/Xcode/DerivedData/Loft-bfxjtqwnvmpyeoaksvduhzgrlcnk/Build/Products/Debug/Loft.app/Contents/MacOS/Loft";
        let rule = kill_rule(exe).unwrap();
        assert_eq!(rule, "Kill(/Users/me/Library/Developer/Xcode/DerivedData/*/Build/Products/Debug/Loft.app/Contents/MacOS/Loft)");
        let rules = vec![rule];
        // A rebuild is another folder; a different program or build type is not covered.
        assert!(rule_allows(&rules, &exe.replace("bfxjtqwnvmpyeoaksvduhzgrlcnk", "zzzzzzzzzzzzzzzzzzzzzzzzzzzz")));
        assert!(rule_allows(&rules, exe));
        assert!(!rule_allows(&rules, &exe.replace("Debug", "Release")));
        // `*` is one folder: it doesn't reach across `/`.
        assert!(!rule_allows(&rules, &exe.replace("Loft-bfxjtqwnvmpyeoaksvduhzgrlcnk", "x/y")));
        assert!(rule_allows(&["Kill(/a/*/c)".to_string()], "/a/b/c") && !rule_allows(&["Kill(/a/*/c)".to_string()], "/a/b/b/c"));
        assert!(!rule_allows(&rules, "/Applications/Loft.app/Contents/MacOS/Loft"));
        assert_eq!(kill_rule("/bin/sleep").unwrap(), "Kill(/bin/sleep)");
        // No rule for an interpreter, which runs whatever it is given.
        assert_eq!(kill_rule(&exe.replace("MacOS/Loft", "MacOS/node")), None);
        assert_eq!(kill_rule("/usr/local/bin/node"), None);
        assert!(!rule_allows(&["Kill(/a/*/node)".to_string()], "/a/b/node"));
        for v in ["python3.12", "node22", "ruby3.3", "python3"] {
            assert_eq!(kill_rule(&format!("/usr/bin/{v}")), None, "{v}");
        }
        assert!(kill_rule("/usr/bin/nodemon").is_some());
        // A folder is a build folder with exactly 28 lowercase letters after the dash.
        let l28 = "abcdefghijklmnopqrstuvwxyz".to_string() + "ab";
        assert_eq!(l28.len(), 28);
        assert_eq!(kill_rule(&format!("/d/Loft-{l28}/x")).unwrap(), "Kill(/d/*/x)");
        for hash in [&l28[..27], &(l28.clone() + "a"), &(l28[..27].to_string() + "1")] {
            let p = format!("/d/Loft-{hash}/x");
            assert_eq!(kill_rule(&p).unwrap(), format!("Kill({p})"), "{hash}");
        }
        // No rule from a path that isn't absolute or holds rule syntax.
        for bad in ["", "sleep", "/tmp/a*b", "/tmp/a)b", "/tmp/"] {
            assert_eq!(kill_rule(bad), None, "{bad}");
        }
        // Rules that would match every program match none.
        for bad in ["Kill", "Kill()", "Kill(*)", "Kill(*/*)", "Kill(/bin/*)", "Bash(/bin/sleep)"] {
            assert!(!rule_allows(&[bad.to_string()], "/bin/sleep"), "{bad}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unanswered_kill_of_another_agents_claude_is_refused_with_the_reason() {
        let (st, a) = fake_claude_state();
        let b = crate::tasks::tests::agent(&st, "Reviewer");
        enqueue(&st, &b.id, Some("busy".into()), "user");
        let storage = st.cfg.storage_dir.clone();
        until("the other agent's run", 10, || sessions(&storage, &b.id).len() == 1).await;
        let claude = *sessions(&storage, &b.id).iter().next().unwrap();
        // By PID (`kill <pid>` of another agent's claude was allowed before), and by a pattern only it matches.
        let bin = st.settings.get().claude_bin;
        for (tool, args) in [("kill", vec!["-9".to_string(), claude.to_string()]), ("pkill", vec!["-f".into(), bin])] {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let v = ask(&st, &a.id, tool, &args).await;
            assert_eq!(v["code"], 1, "{v}");
            let err = v["stderr"].as_str().unwrap();
            assert!(err.contains("Loft didn't stop") && err.contains(&claude.to_string()), "{err}");
            assert!(err.contains("did not respond") && err.contains("Stop only your own processes"), "{err}");
            assert!(alive(claude) && is_running(&st, &b.id), "{tool} stopped the other agent");
            let req = st.db.list_permissions(Some(&a.id)).unwrap().into_iter().find(|p| p.input["command"].as_str().unwrap().starts_with(tool)).unwrap();
            assert_eq!(req.status, "expired");
            assert!(req.reason.contains(&format!("{claude} (Reviewer's:")), "{}", req.reason);
        }
        crate::agents::stop(&st, &b.id);
        until("the run to stop", 10, || !is_running(&st, &b.id)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_kill_whose_command_stops_waiting_expires_its_request() {
        let (st, a) = fake_claude_state();
        let other = Sleeper::start(false);
        let asking = tokio::spawn({
            let (st, id, pid) = (st.clone(), a.id.clone(), other.pid().to_string());
            async move { ask(&st, &id, "kill", &[&pid]).await }
        });
        until("a permission request", 5, || pending(&st, &a.id).len() == 1).await;
        let id = pending(&st, &a.id)[0].id.clone();
        // The Bash call timed out: the connection, and the handler with it, goes away.
        asking.abort();
        until("the request to expire", 5, || st.db.get_permission(&id).unwrap().unwrap().status == "expired").await;
        assert!(crate::agents::decide(&st, &id, "allow_once", None, None, None).is_err());
        assert!(other.alive());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loft_itself_is_never_the_agents_own() {
        let (st, a) = fake_claude_state();
        // Even when Loft runs in a session of the agent's (a test instance it started).
        let me = std::process::id();
        record(&st.cfg.storage_dir, &a.id, session_of(me as i32).unwrap() as u32).unwrap();
        let v = ask(&st, &a.id, "kill", &["-0", &me.to_string()]).await;
        assert_eq!(v["code"], 0, "a check needs no answer");
        let asking = tokio::spawn({
            let (st, id) = (st.clone(), a.id.clone());
            async move { ask(&st, &id, "kill", &["-CONT", &me.to_string()]).await }
        });
        until("a permission request", 5, || pending(&st, &a.id).len() == 1).await;
        assert!(pending(&st, &a.id)[0].reason.contains(&format!("{me} (Loft:")));
        crate::agents::decide(&st, &pending(&st, &a.id)[0].id, "deny", None, None, None).unwrap();
        let v = asking.await.unwrap();
        assert_eq!(v["code"], 1);
        assert!(v["stderr"].as_str().unwrap().contains("The user denied it."), "{v}");
    }

    /// A link to sleep with a name no process outside this test has, for killall.
    #[cfg(target_os = "macos")]
    fn unique_sleep(st: &AppState) -> (PathBuf, String) {
        let name = format!("ks{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let path = st.cfg.root.as_path().join(&name);
        std::os::unix::fs::symlink("/bin/sleep", &path).unwrap();
        (path, name)
    }

    // macOS's ps, killall and pkill: Linux's `ps -o comm` shows no path and its killall has no `-s` to only show.
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread")]
    async fn killall_stops_the_agents_own_process_by_name_and_asks_about_another() {
        let (st, a) = fake_claude_state();
        let (path, name) = unique_sleep(&st);
        let server = Sleeper::start_as(&path, true);
        record(&st.cfg.storage_dir, &a.id, server.pid() as u32).unwrap();
        let v = ask(&st, &a.id, "killall", &["-KILL", &name]).await;
        assert_eq!(v["code"], 0, "{v}");
        until("the server to stop", 5, || !server.alive()).await;
        assert!(st.db.list_permissions(Some(&a.id)).unwrap().is_empty(), "it asked the user");

        // The same name, started by someone else: killall asks, and nobody answers.
        let (path, name) = unique_sleep(&st);
        let other = Sleeper::start_as(&path, false);
        assert!(other.alive());
        let v = ask(&st, &a.id, "killall", &[&name]).await;
        assert_eq!(v["code"], 1, "{v}");
        assert!(v["stderr"].as_str().unwrap().contains(&format!("Loft didn't stop {}", other.pid())), "{v}");
        assert!(other.alive());
        let req = &st.db.list_permissions(Some(&a.id)).unwrap()[0];
        assert_eq!((req.input["command"].as_str(), req.status.as_str()), (Some(format!("killall {name}").as_str()), "expired"));
        assert!(req.reason.contains(&format!("{} (not an agent's: ", other.pid())), "{}", req.reason);

        // Nothing by that name: killall's own answer.
        let v = ask(&st, &a.id, "killall", &["ks-no-such-process"]).await;
        assert_eq!(v["code"], 1, "{v}");
    }

    // macOS's ps, killall and pkill: Linux's `ps -o comm` shows no path and its killall has no `-s` to only show.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn pkill_reads_a_relative_pidfile_from_the_callers_folder_or_else_loft_s() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let caller = Sleeper::start(true);
        record(&st.cfg.storage_dir, &a.id, caller.pid() as u32).unwrap();
        let server = Sleeper::start(true);
        record(&st.cfg.storage_dir, &a.id, server.pid() as u32).unwrap();
        let dir = st.cfg.storage_dir.join("ws");
        std::fs::create_dir_all(&dir).unwrap();
        // A name Loft's own folder doesn't have.
        let file = format!("{}.pid", uuid::Uuid::new_v4().simple());
        std::fs::write(dir.join(&file), format!("{}\n", server.pid())).unwrap();

        // No folder, or one that doesn't exist: pgrep looks in Loft's, as before, and finds no such file.
        for cwd in [None, Some(dir.join("gone").as_path())] {
            let v = ask_in(&st, &a.id, caller.pid(), cwd, "pkill", &["-F", &file]).await;
            assert!(v["stderr"].as_str().unwrap().contains("pkill: Cannot open pidfile") && v["code"] != 0, "{cwd:?}: {v}");
            assert!(server.alive());
        }
        let v = ask_in(&st, &a.id, caller.pid(), Some(&dir), "pkill", &["-F", &file]).await;
        assert_eq!(v["code"], 0, "{v}");
        until("the server to stop", 5, || !server.alive()).await;
        assert!(pending(&st, &a.id).is_empty());
    }

    #[test]
    fn the_request_shows_the_command_with_its_quoting() {
        assert_eq!(shell_word("-f"), "-f");
        assert_eq!(shell_word("sleep 120"), "'sleep 120'");
        assert_eq!(shell_word("it's"), r"'it'\''s'");
        assert_eq!(shell_word("vite|next"), "'vite|next'");
        assert_eq!(shell_word(""), "''");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deleting_an_agent_stops_the_processes_it_owns() {
        let (st, a) = fake_claude_state();
        let server = Sleeper::start(true);
        let other = Sleeper::start(true);
        record(&st.cfg.storage_dir, &a.id, server.pid() as u32).unwrap();
        let uri: axum::http::Uri = "/?purge_files=false".parse().unwrap();
        let opts = axum::extract::Query::try_from_uri(&uri).unwrap();
        let _ = crate::api::delete_agent(State(st.clone()), UrlPath(a.id.clone()), opts).await.ok().unwrap();
        until("its server to stop", 5, || !server.alive()).await;
        assert!(other.alive(), "it stopped a process that wasn't the agent's");
        assert!(sessions(&st.cfg.storage_dir, &a.id).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sessions_with_no_process_left_are_pruned() {
        let (st, a) = fake_claude_state();
        let server = Sleeper::start(true);
        let gone = Sleeper::start(true);
        let gone_pid = gone.pid();
        gone.stop();
        let storage = &st.cfg.storage_dir;
        record(storage, &a.id, server.pid() as u32).unwrap();
        record(storage, &a.id, gone_pid as u32).unwrap();
        // Just recorded: its shell may not have shown in the process table yet.
        prune(storage);
        assert_eq!(sessions(storage, &a.id), HashSet::from([server.pid(), gone_pid]));
        let old = std::time::SystemTime::now() - PRUNE_AFTER * 2;
        for sid in [server.pid(), gone_pid] {
            let f = std::fs::File::options().write(true).open(sessions_dir(storage, &a.id).join(sid.to_string())).unwrap();
            f.set_modified(old).unwrap();
        }
        prune(storage);
        assert_eq!(sessions(storage, &a.id), HashSet::from([server.pid()]));
    }

    #[test]
    fn kill_reads_its_arguments_like_kill() {
        let signal = |args: &[&str]| match plan_kill(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>(), 0, &[]) {
            Plan::Signal { signal, pids, out } => Ok((signal, pids, out.code)),
            Plan::Done(out) => Err(out.code),
        };
        assert_eq!(signal(&["12"]), Ok((libc::SIGTERM, vec![12], 0)));
        assert_eq!(signal(&["-9", "12", "34"]), Ok((libc::SIGKILL, vec![12, 34], 0)));
        assert_eq!(signal(&["-KILL", "12"]), Ok((libc::SIGKILL, vec![12], 0)));
        assert_eq!(signal(&["-SIGHUP", "--", "12"]), Ok((libc::SIGHUP, vec![12], 0)));
        assert_eq!(signal(&["-s", "term", "12"]), Ok((libc::SIGTERM, vec![12], 0)));
        assert_eq!(signal(&["-n", "0", "12"]), Ok((0, vec![12], 0)));
        // A first `-1` is the signal.
        assert_eq!(signal(&["-1", "12"]), Ok((libc::SIGHUP, vec![12], 0)));
        assert_eq!(signal(&["12", "abc"]), Ok((libc::SIGTERM, vec![12], 1)));
        assert_eq!(signal(&["-FOO", "12"]), Err(2));
        assert_eq!(signal(&[]), Err(2));
        assert_eq!(signal(&["-9"]), Err(2));
        assert!(signal(&["-l"]).is_err());
    }

    #[test]
    fn pkill_and_killall_signals() {
        assert_eq!(signal_of("9"), Some(libc::SIGKILL));
        assert_eq!(signal_of("term"), Some(libc::SIGTERM));
        assert_eq!(signal_of("SIGUSR1"), Some(libc::SIGUSR1));
        // pkill's options aren't signals.
        for flag in ["f", "x", "i", "l", "n", "o", "v", "I", "L", "u", "U", "g", "F"] {
            assert_eq!(signal_of(flag), None, "{flag}");
        }
    }

    /// Serves `POST /kill/<agent>` for the agents' `loft kill-as`, as Loft does.
    async fn serve_kills(st: &mut AppState) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        st.cfg = Arc::new(crate::state::Config { port, storage_dir: st.cfg.storage_dir.clone(), xcrun: st.cfg.xcrun.clone(), xcode: st.cfg.xcode.clone(), root: st.cfg.root.clone() });
        let app = axum::Router::new().route("/kill/{agent_id}", axum::routing::post(handle)).with_state(st.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    }

    /// One run whose only Bash command is `cmd`; what it printed.
    async fn bash(st: &AppState, id: &str, cmd: &str) -> String {
        let ws = workspace(st, id);
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("cmd.sh"), cmd).unwrap();
        let _ = std::fs::remove_file(ws.join("bash.out"));
        enqueue(st, id, Some("bash-cmd".into()), "user");
        let out = || std::fs::read_to_string(ws.join("bash.out")).unwrap_or_default();
        until("the command", 60, || !is_running(st, id) && out().contains("exit ")).await;
        out()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_stops_its_detached_server_from_an_earlier_run_by_name_and_by_pid() {
        let (mut st, a) = fake_claude_state();
        serve_kills(&mut st).await;
        let ws = workspace(&st, &a.id);
        std::fs::create_dir_all(&ws).unwrap();
        // A name no process outside this test has, for the `pkill -f`.
        let name = format!("srv-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        std::fs::write(ws.join(format!("{name}.sh")), "trap 'kill $!; exit 0' TERM\nsleep 60 &\nwait\n").unwrap();
        let start = format!("nohup /bin/sh ./{name}.sh > /dev/null 2>&1 &\necho $! > server.pid\n");
        let server = || std::fs::read_to_string(ws.join("server.pid")).unwrap().trim().parse::<i32>().unwrap();

        // `pkill -F` reads the pidfile from the command's folder, the workspace, not Loft's.
        for stop in [format!("pkill -f {name}"), "pkill -F server.pid".into(), "kill $(cat server.pid)".into(), "echo $(cat server.pid) | xargs kill".to_string()] {
            let out = bash(&st, &a.id, &start).await;
            assert!(out.contains("exit 0"), "{out}");
            let pid = server();
            // Started in the run's Bash command, whose session the agent recorded; that run is over.
            let sid = session_of(pid).unwrap();
            assert!(sessions(&st.cfg.storage_dir, &a.id).contains(&sid));
            assert!(alive(pid));
            let out = bash(&st, &a.id, &stop).await;
            assert!(out.contains("exit 0"), "{stop}: {out}");
            until("the server to stop", 30, || !alive(pid)).await;
            assert!(st.db.list_permissions(Some(&a.id)).unwrap().is_empty(), "{stop} asked the user");
        }

        // A job of the command's own shell goes to the shell's builtin; a PID named with it doesn't come along.
        let other = Sleeper::start(false);
        let out = bash(&st, &a.id, &format!("/bin/sleep 60 &\nkill %+ {}; echo \"mixed $?\"\nkill -9 %+; echo \"job $?\"\nwait\n", other.pid())).await;
        assert!(out.contains("separate kill commands") && out.contains("mixed 2") && out.contains("job 0"), "{out}");
        assert!(other.alive());
        assert!(st.db.list_permissions(Some(&a.id)).unwrap().is_empty());

        // A process it didn't start: refused once nobody answers, with the reason in the command's output.
        let out = bash(&st, &a.id, &format!("kill {}", other.pid())).await;
        assert!(out.contains("Loft didn't stop") && out.contains("did not respond") && out.contains("exit 1"), "{out}");
        assert!(other.alive());
    }
}

// On every OS: the env script, and on Windows the agents' jobs.
#[cfg(test)]
mod job_tests {
    use super::*;

    #[test]
    fn on_windows_the_env_script_records_no_session() {
        let (storage, exe) = (Path::new("/data/storage"), Path::new("/opt/loft/loft"));
        let unix = env_script_on(storage, "a1", exe, false);
        assert!(unix.contains(&format!(": > '{}'/\"$$\"", sessions_dir(storage, "a1").display())), "{unix}");
        assert!(unix.contains("pkill() { '/opt/loft/loft' kill-as pkill \"$@\"; }") && !unix.contains("taskkill"), "{unix}");
        let windows = env_script_on(storage, "a1", exe, true);
        assert!(!windows.contains("sessions") && !windows.contains("$$"), "{windows}");
        // The kills still go through Loft, taskkill too, with their arguments as written (`//PID`, `/IM`).
        for tool in ["pkill", "killall", "taskkill"] {
            assert!(windows.contains(&format!("{tool}() {{ MSYS_NO_PATHCONV=1 '/opt/loft/loft' kill-as {tool} \"$@\"; }}")), "{windows}");
        }
        // kill turns Git Bash PIDs into Windows ones.
        assert!(windows.contains("/proc/$a/winpid"), "{windows}");
    }

    /// Runs `script` with `args` in a POSIX shell, and returns what it prints.
    #[cfg(unix)]
    fn sh(script: &str, args: &[&str]) -> String {
        let out = std::process::Command::new("/bin/sh").arg("-c").arg(script).arg("sh").args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn git_bash_pids_become_windows_pids_but_signals_and_other_words_stay() {
        // Git Bash's /proc/<pid>/winpid, faked in a folder of the test's.
        let dir = crate::testdir::for_this_test("winpids");
        std::fs::create_dir_all(dir.join("1234")).unwrap();
        std::fs::write(dir.join("1234/winpid"), "9876\n").unwrap();
        std::fs::create_dir_all(dir.join("9")).unwrap();
        std::fs::write(dir.join("9/winpid"), "5555\n").unwrap();
        let script = windows_pids("").replace("/proc/", &format!("{}/", dir.display())) + "printf '%s,' \"$@\"";
        assert_eq!(sh(&script, &["-9", "1234", "77"]), "-9,9876,77,", "a number with no Git Bash process stays");
        assert_eq!(sh(&script, &["-s", "9", "1234"]), "-s,9,9876,", "the signal after -s is no PID");
        assert_eq!(sh(&script, &["-n", "9", "--", "9", ""]), "-n,9,--,5555,,");
        assert_eq!(sh(&script, &["-l"]), "-l,");

        // The whole kill function, with a fake loft that prints what it gets.
        let fake = dir.join("loft");
        std::fs::write(&fake, "#!/bin/sh\nprintf '%s,' \"$MSYS_NO_PATHCONV\" \"$@\"\n").unwrap();
        crate::owner_only::executable(&fake).unwrap();
        let env = env_script_on(Path::new("/data/storage"), "a1", &fake, true).replace("/proc/", &format!("{}/", dir.display()));
        let out = std::process::Command::new("/bin/bash").arg("-c").arg(format!("{env}\nkill -9 1234 77; taskkill //PID 1234")).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "1,kill-as,kill,-9,9876,77,1,kill-as,taskkill,//PID,1234,", "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn on_windows_the_shims_keep_their_arguments_and_kill_s_turns_git_bash_pids() {
        let exe = Path::new("/c/loft/loft.exe");
        assert_eq!(shim("pkill", exe, false), "#!/bin/sh\nexec '/c/loft/loft.exe' kill-as pkill \"$@\"\n");
        let taskkill = shim("taskkill", exe, true);
        assert!(taskkill.ends_with("MSYS_NO_PATHCONV=1\nexport MSYS_NO_PATHCONV\nexec '/c/loft/loft.exe' kill-as taskkill \"$@\"\n"), "{taskkill}");
        assert!(!taskkill.contains("winpid"));
        assert!(shim("kill", exe, true).contains("/proc/$a/winpid"));
    }

    /// On Windows: the agent's own processes are those in its job, and a kill stops them without asking, while any other
    /// waits for the user. The processes are pings (a console program every Windows has) that wait 30 s, each at a
    /// loopback address no other test uses, which makes its command line unique.
    #[cfg(windows)]
    mod windows {
        use super::*;
        use crate::agents::tests::until;
        use std::os::windows::process::CommandExt;
        use std::sync::atomic::{AtomicU32, Ordering};

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        fn ping() -> PathBuf {
            PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("System32").join("PING.EXE")
        }

        fn unique_ip() -> String {
            static N: AtomicU32 = AtomicU32::new(1);
            let n = N.fetch_add(1, Ordering::SeqCst).wrapping_add(std::process::id().wrapping_mul(7919));
            format!("127.{}.{}.{}", (n >> 16) & 255, (n >> 8) & 255, (n & 255).max(1))
        }

        /// A ping in no agent's job, ended when dropped.
        struct Outsider(std::process::Child);

        impl Outsider {
            fn start(exe: &Path, ip: &str) -> Self {
                let child = std::process::Command::new(exe).args(["-n", "30", ip]).stdout(std::process::Stdio::null()).creation_flags(CREATE_NO_WINDOW).spawn().unwrap();
                Outsider(child)
            }

            fn pid(&self) -> i32 {
                self.0.id() as i32
            }
        }

        impl Drop for Outsider {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        /// A ping in the agent's job, as a run's command starts one.
        fn in_job(storage: &Path, agent_id: &str, exe: &Path, ip: &str) -> tokio::process::Child {
            let mut cmd = tokio::process::Command::new(exe);
            cmd.args(["-n", "30", ip]).stdout(std::process::Stdio::null()).creation_flags(crate::job::CREATION_FLAGS).kill_on_drop(true);
            let child = cmd.spawn().unwrap();
            crate::job::join(storage, agent_id, &child).unwrap();
            child
        }

        fn pid_of(child: &tokio::process::Child) -> i32 {
            child.id().unwrap() as i32
        }

        fn alive(pid: i32) -> bool {
            crate::job::alive(pid as u32)
        }

        async fn ask(st: &AppState, agent_id: &str, caller: i32, tool: &str, args: &[&str]) -> Value {
            let req = KillRequest { tool: tool.into(), args: args.iter().map(|a| a.to_string()).collect(), caller, cwd: None };
            handle(State(st.clone()), UrlPath(agent_id.into()), Json(req)).await.0
        }

        fn pending(st: &AppState, agent_id: &str) -> Vec<crate::db::PermissionRequest> {
            st.db.list_permissions(Some(agent_id)).unwrap().into_iter().filter(|p| p.status == "pending").collect()
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn an_agent_stops_what_its_job_holds_without_asking_and_another_process_once_the_user_allows() {
            let st = crate::tasks::tests::state();
            let a = crate::tasks::tests::agent(&st, "Builder");
            let storage = st.cfg.storage_dir.clone();
            // Where `loft kill-as` runs: the agent's command, in its job.
            let shell = in_job(&storage, &a.id, &ping(), &unique_ip());
            let caller = pid_of(&shell);
            let server = in_job(&storage, &a.id, &ping(), &unique_ip());
            let pid = pid_of(&server).to_string();
            // Checking it is there stops nothing.
            let v = ask(&st, &a.id, caller, "kill", &["-0", &pid]).await;
            assert_eq!(v["code"], 0, "{v}");
            assert!(alive(pid_of(&server)));
            let v = ask(&st, &a.id, caller, "kill", &["-9", &pid]).await;
            assert_eq!(v["code"], 0, "{v}");
            until("the server to end", 10, || !alive(pid_of(&server))).await;
            assert!(st.db.list_permissions(Some(&a.id)).unwrap().is_empty(), "it asked the user");
            // A PID with no process gets kill's own error; a process group isn't a thing on Windows.
            let v = ask(&st, &a.id, caller, "kill", &[&pid]).await;
            assert!(v["code"] == 1 && v["stderr"].as_str().unwrap().contains("No such process"), "{v}");
            for group in ["0", "-1", &format!("-{caller}")] {
                let v = ask(&st, &a.id, caller, "kill", &["--", group]).await;
                assert!(v["code"] == 1 && v["stderr"].as_str().unwrap().contains("no process groups"), "{group}: {v}");
            }
            assert!(alive(caller));

            // A process no agent started: asked about, and stopped only once allowed.
            let other = Outsider::start(&ping(), &unique_ip());
            let pid = other.pid().to_string();
            for answer in ["deny", "allow"] {
                let asking = tokio::spawn({
                    let (st, id, pid) = (st.clone(), a.id.clone(), pid.clone());
                    async move { ask(&st, &id, caller, "kill", &[&pid]).await }
                });
                until("a permission request", 10, || pending(&st, &a.id).len() == 1).await;
                let req = pending(&st, &a.id).remove(0);
                assert_eq!((req.tool_name.as_str(), req.input["command"].as_str()), ("Kill", Some(format!("kill {pid}").as_str())));
                assert!(req.reason.contains(&format!("{pid} (not an agent's: ")) && req.reason.contains("PING.EXE"), "{}", req.reason);
                assert!(alive(other.pid()), "it was stopped before the user answered");
                crate::agents::decide(&st, &req.id, answer, None, None, None).unwrap();
                let v = asking.await.unwrap();
                if answer == "deny" {
                    assert!(v["code"] == 1 && v["stderr"].as_str().unwrap().contains(&format!("Loft didn't stop {pid}")), "{v}");
                    assert!(alive(other.pid()));
                } else {
                    assert_eq!(v["code"], 0, "{v}");
                    until("the other process to end", 10, || !alive(other.pid())).await;
                }
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_kill_from_outside_the_agents_job_asks_even_about_its_own_processes() {
            let st = crate::tasks::tests::state();
            let a = crate::tasks::tests::agent(&st, "Builder");
            let b = crate::tasks::tests::agent(&st, "Other");
            let storage = st.cfg.storage_dir.clone();
            let server = in_job(&storage, &a.id, &ping(), &unique_ip());
            let pid = pid_of(&server).to_string();
            // A command in another agent's job holding this one's kill URL, Loft itself, and no caller.
            let elsewhere = in_job(&storage, &b.id, &ping(), &unique_ip());
            for caller in [pid_of(&elsewhere), std::process::id() as i32, 0] {
                let asking = tokio::spawn({
                    let (st, id, pid) = (st.clone(), a.id.clone(), pid.clone());
                    async move { ask(&st, &id, caller, "taskkill", &["//F", "//PID", &pid]).await }
                });
                until("a permission request", 10, || pending(&st, &a.id).len() == 1).await;
                let req = pending(&st, &a.id).remove(0);
                assert!(req.reason.contains(&format!("A command outside {}'s runs (PID {caller})", a.name)), "{}", req.reason);
                assert!(req.reason.contains(&format!("{pid} ({}'s:", a.name)), "{}", req.reason);
                crate::agents::decide(&st, &req.id, "deny", None, None, None).unwrap();
                let v = asking.await.unwrap();
                assert!(v["code"] == 1 && v["stderr"].as_str().unwrap().contains("doesn't run in one of your runs"), "{v}");
                assert!(alive(pid_of(&server)), "caller {caller} stopped the agent's server");
            }
            // From its own job, taskkill stops it unasked and says so the way taskkill does.
            let shell = in_job(&storage, &a.id, &ping(), &unique_ip());
            let v = ask(&st, &a.id, pid_of(&shell), "taskkill", &["//F", "//PID", &pid]).await;
            assert_eq!(v["code"], 0, "{v}");
            assert_eq!(v["stdout"], format!("SUCCESS: The process with PID {pid} has been terminated.\n"));
            until("the server to end", 10, || !alive(pid_of(&server))).await;
            crate::job::terminate(&storage, &b.id);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn pkill_and_killall_find_by_name_or_command_line_and_stop_unasked_only_the_agents_own() {
            let st = crate::tasks::tests::state();
            let a = crate::tasks::tests::agent(&st, "Builder");
            let storage = st.cfg.storage_dir.clone();
            // A program by a name no other process has: a copy of ping.
            let dir = crate::testdir::for_this_test("windows-by-name");
            std::fs::create_dir_all(&dir).unwrap();
            let name = format!("ks{}", unique_ip().replace('.', ""));
            let exe = dir.join(format!("{name}.exe"));
            std::fs::copy(ping(), &exe).unwrap();
            let shell = in_job(&storage, &a.id, &ping(), &unique_ip());
            let caller = pid_of(&shell);

            // pkill -f by the command line: the agent's, unasked; the other's, left alone.
            let (mine, theirs) = (unique_ip(), unique_ip());
            let server = in_job(&storage, &a.id, &exe, &mine);
            let other = Outsider::start(&exe, &theirs);
            let v = ask(&st, &a.id, caller, "pkill", &["-f", &mine.replace('.', "\\.")]).await;
            assert_eq!(v["code"], 0, "{v}");
            until("the server to end", 10, || !alive(pid_of(&server))).await;
            assert!(alive(other.pid()) && st.db.list_permissions(Some(&a.id)).unwrap().is_empty());
            // Nothing matches: pkill's own answer.
            let v = ask(&st, &a.id, caller, "pkill", &["-f", "ks-no-such-process"]).await;
            assert_eq!(v["code"], 1, "{v}");

            // killall by the name, with or without .exe: the agent's stops unasked, the other's waits for the user, who
            // doesn't answer.
            let again = in_job(&storage, &a.id, &exe, &unique_ip());
            let v = ask(&st, &a.id, caller, "killall", &[&format!("{name}.EXE")]).await;
            assert_eq!(v["code"], 1, "{v}");
            assert!(v["stderr"].as_str().unwrap().contains(&format!("Loft didn't stop {}", other.pid())), "{v}");
            until("the agent's copy to end", 10, || !alive(pid_of(&again))).await;
            assert!(alive(other.pid()));
            let req = &st.db.list_permissions(Some(&a.id)).unwrap()[0];
            assert_eq!(req.input["pids"], json!([other.pid()]), "only the other's process was asked about");
            // Never the caller's own command.
            assert!(alive(caller));
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_process_in_an_agent_s_job_is_the_agent_s_and_deleting_the_agent_ends_its_tree() {
        let storage = crate::testdir::for_this_test("processes-job");
        let _child = crate::job::tests::spawn_tree(&storage, "a1");
        let pids = crate::job::tests::wait_for(&storage, "a1", 3);
        let ids = ["a1".to_string(), "b2".to_string()];
        let live = live_processes(&storage, &ids);
        let mine: HashSet<i32> = live.get("a1").into_iter().flatten().map(|p| p.pid).collect();
        assert!(pids.iter().all(|p| mine.contains(&(*p as i32))), "{pids:?} vs {mine:?}");
        assert!(!live.contains_key("b2"), "another agent owns none of them");
        // Loft itself is no agent's.
        assert!(!mine.contains(&(std::process::id() as i32)));
        stop_all(&storage, "a1").await;
        crate::agents::tests::until("the job's processes to end", 10, || pids.iter().all(|p| !crate::job::alive(*p))).await;
        assert!(crate::job::agents(&storage).is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn an_idle_stop_ends_the_agent_s_processes_and_signal_zero_checks_one() {
        let storage = crate::testdir::for_this_test("processes-job-stop");
        let _child = crate::job::tests::spawn_tree(&storage, "a1");
        let pids = crate::job::tests::wait_for(&storage, "a1", 3);
        assert!(signal(pids[0] as i32, 0).is_ok());
        let stopped = stop(&storage, "a1", || true, false).await;
        assert!(pids.iter().all(|p| stopped.iter().any(|(s, _, _)| *s == *p as i32)), "{pids:?} vs {stopped:?}");
        crate::agents::tests::until("the job's processes to end", 10, || pids.iter().all(|p| !crate::job::alive(*p))).await;
        assert!(signal(pids[0] as i32, 0).is_err());
    }
}
