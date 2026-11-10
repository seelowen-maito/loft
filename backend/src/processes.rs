//! Agent processes: which agent owns which process, and the kills agents run, which Loft scopes to them.
//!
//! Ownership is by process session. Each run's claude starts in a session of its own, and Claude Code starts each
//! Bash command in another one, so the script Loft gives claude as `CLAUDE_ENV_FILE` (claude runs it in each
//! command's shell, before the command) records that shell's session for the agent. A detached server
//! (`nohup … &`) keeps its session after the shell exits and is reparented, so a process is the agent's when its
//! session id is one of the agent's. The records are files, `.agent-files/<id>.sessions/<sid>`: pruned once no
//! process of the session is left, and removed with the agent, whose processes are stopped then.
//!
//! The same script makes `kill`, `pkill` and `killall` shell functions, and puts commands of those names first on
//! PATH (for `xargs kill`, `find -exec kill`), that run `loft kill-as <tool> <args>`. That asks Loft
//! (`POST /kill/<agent id>`), which finds the targets the way the tool would, signals the agent's own by PID and
//! sends any other (another agent's, the user's apps and MCP servers, Loft itself) to the user as a permission
//! request. Only a caller running in one of the agent's sessions has any own: from anywhere else (a command holding
//! another agent's URL), every target goes to the user. Unanswered, it is refused. Kills that skip the functions (by path, through `sudo` or `builtin`, or a
//! `kill` in another shell) are the Bash hook's (kill_guard.rs): it refuses them and says to kill by name.

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

/// The argument that makes this binary run as an agent's `kill`, `pkill` or `killall`.
pub const ARG: &str = "kill-as";
/// Where `loft kill-as` asks Loft: `http://127.0.0.1:<port>/kill/<agent id>`, in claude's environment.
pub const URL_VAR: &str = "LOFT_KILL_URL";

/// The tools Loft scopes, and the real command each stands for.
const TOOLS: [(&str, &str); 3] = [("kill", "/bin/kill"), ("pkill", "/usr/bin/pkill"), ("killall", "/usr/bin/killall")];

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
fn owners(storage: &Path) -> HashMap<i32, String> {
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

/// The agents with a process still alive, like a detached dev server.
pub fn with_live_processes(storage: &Path) -> HashSet<String> {
    let owners = owners(storage);
    table().iter().filter_map(|p| owners.get(&session_of(p.pid)?).cloned()).collect()
}

/// The process session of `pid`, if it is alive.
fn session_of(pid: i32) -> Option<i32> {
    let sid = unsafe { libc::getsid(pid) };
    (sid > 0).then_some(sid)
}

/// The script claude runs in each Bash command's shell before the command (`CLAUDE_ENV_FILE`): it records the
/// shell's session (Claude Code makes the shell a session leader, so `$$` is its id) and sends kills through Loft.
/// A job (`kill %1`) is the shell's own child, so a call naming only jobs (after the signal option) stays with the
/// builtin; one that mixes jobs and PIDs is refused, or the builtin would signal the PIDs too.
pub fn env_script(storage: &Path, agent_id: &str, exe: &Path) -> String {
    let loft = quote(&exe.to_string_lossy());
    format!(
        r#"# Loft: records this shell's process session as the agent's, and sends kills through Loft.
: > {dir}/"$$" 2>/dev/null
kill() {{
  case " $* " in *" %"*) ;; *) {loft} {ARG} kill "$@"; return ;; esac
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
pkill() {{ {loft} {ARG} pkill "$@"; }}
killall() {{ {loft} {ARG} killall "$@"; }}
PATH={bin}:"$PATH"
export PATH
"#,
        dir = quote(&sessions_dir(storage, agent_id).to_string_lossy()),
        bin = quote(&shims_dir(storage).to_string_lossy()),
    )
}

fn shims_dir(storage: &Path) -> PathBuf {
    storage.join(crate::agents::AGENT_FILES).join("bin")
}

/// Writes the commands that stand in for `kill`, `pkill` and `killall` where the shell functions don't reach
/// (`xargs kill`, `find -exec kill`), and returns their folder.
pub fn write_shims(storage: &Path, exe: &Path) -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = shims_dir(storage);
    std::fs::create_dir_all(&dir)?;
    for (tool, _) in TOOLS {
        let path = dir.join(tool);
        let text = format!("#!/bin/sh\nexec {} {ARG} {tool} \"$@\"\n", quote(&exe.to_string_lossy()));
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text)?;
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
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

/// Prunes the session records once a minute.
pub async fn pruner(storage: PathBuf) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let storage = storage.clone();
        let _ = tokio::task::spawn_blocking(move || prune(&storage)).await;
    }
}

/// Stops every process the agent owns, when it is deleted: SIGTERM, then SIGKILL for any still there after a
/// grace period. Its session records go now.
pub async fn stop_all(storage: &Path, agent_id: &str) {
    let (storage, id) = (storage.to_path_buf(), agent_id.to_string());
    let targets = tokio::task::spawn_blocking(move || {
        let mine = sessions(&storage, &id);
        let me = std::process::id() as i32;
        let targets: Vec<(i32, i32)> = table()
            .iter()
            .filter(|p| p.pid != me)
            .filter_map(|p| Some((p.pid, session_of(p.pid).filter(|s| mine.contains(s))?)))
            .collect();
        for (pid, _) in &targets {
            unsafe { libc::kill(*pid, libc::SIGTERM) };
        }
        let _ = std::fs::remove_dir_all(sessions_dir(&storage, &id));
        targets
    })
    .await
    .unwrap_or_default();
    if targets.is_empty() {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(STOP_GRACE).await;
        // Only a process still in the session: the PID may belong to another one by now.
        for (pid, sid) in targets {
            if session_of(pid) == Some(sid) {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    });
}

// ---- the process table ------------------------------------------------

struct Proc {
    pid: i32,
    ppid: i32,
    uid: u32,
    command: String,
}

/// Every process on the Mac, from `ps`.
fn table() -> Vec<Proc> {
    let Ok(out) = std::process::Command::new("/bin/ps").args(["-axww", "-o", "pid=,ppid=,uid=,command="]).output() else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, rest) = field(line);
            let (ppid, rest) = field(rest);
            let (uid, command) = field(rest);
            Some(Proc { pid: pid.parse().ok()?, ppid: ppid.parse().ok()?, uid: uid.parse().ok()?, command: command.to_string() })
        })
        .collect()
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

/// Runs as an agent's `kill`, `pkill` or `killall`: asks Loft, prints its answer and exits with its code. Outside an
/// agent (no `LOFT_KILL_URL`) it is the real command.
pub fn cli() -> ! {
    let mut argv = std::env::args().skip(2);
    let tool = argv.next().unwrap_or_default();
    let args: Vec<String> = argv.collect();
    let Some((_, real)) = TOOLS.iter().find(|(t, _)| *t == tool) else {
        eprintln!("loft {ARG}: expected kill, pkill or killall");
        std::process::exit(2);
    };
    let Ok(url) = std::env::var(URL_VAR) else {
        use std::os::unix::process::CommandExt;
        let e = std::process::Command::new(real).args(&args).exec();
        eprintln!("{tool}: {e}");
        std::process::exit(126);
    };
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

/// A plain HTTP/1.1 POST to Loft on localhost, with no runtime to start: this runs inside an agent's command.
/// No read timeout: a kill of someone else's process waits for the user.
fn post(url: &str, body: &str) -> anyhow::Result<Value> {
    let rest = url.strip_prefix("http://").ok_or_else(|| anyhow::anyhow!("bad {URL_VAR}"))?;
    let (host, path) = rest.split_once('/').ok_or_else(|| anyhow::anyhow!("bad {URL_VAR}"))?;
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
    let storage = state.cfg.storage_dir.clone();
    let (tool, caller) = (req.tool.clone(), req.caller);
    // As the agent typed it, near enough: `pkill -f 'sleep 120'` mustn't read as a PID 120.
    let command = std::iter::once(req.tool.clone()).chain(req.args.iter().map(|a| shell_word(a))).collect::<Vec<_>>().join(" ");
    let Ok((plan, owners, procs)) = tokio::task::spawn_blocking(move || {
        let procs = table();
        (plan(&req, &procs), owners(&storage), procs)
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
    let owner = |pid: i32| session_of(pid).and_then(|sid| owners.get(&sid));
    // The URL names the agent, but only a caller in one of its sessions speaks for it: a command holding another
    // agent's kill URL (a stale environment, a hand-written curl) gets none of that agent's processes as its own.
    let inside = caller > 0 && owner(caller) == Some(&a.id);
    // Signal 0 only checks the process exists. A PID with no process gets the tool's own error.
    let (own, foreign): (Vec<i32>, Vec<i32>) = pids
        .into_iter()
        .partition(|&pid| signal == 0 || session_of(pid).is_none() || (inside && pid != me && owner(pid) == Some(&a.id)));
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
    let n = foreign.len();
    let es = if n == 1 { "" } else { "es" };
    let reason = if inside {
        format!("Stops {n} process{es} {} didn't start: {}.", a.name, list.join(", "))
    } else {
        format!("A command outside {}'s runs (PID {caller}) asks to stop {n} process{es}: {}.", a.name, list.join(", "))
    };
    let input = json!({ "command": command, "pids": foreign });
    let d = crate::agents::ask_permission_for(state, &a.id, "prompt", "Kill", input, &reason, "Kill".into(), ASK_TIMEOUT).await;
    if d.allow {
        send(&foreign, signal, &tool, &mut out);
    } else {
        let pids = foreign.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" ");
        let why = d.message.unwrap_or_else(|| "The user denied it.".into());
        let because = if inside {
            format!("your runs didn't start {}", if n == 1 { "it" } else { "them" })
        } else {
            "this command doesn't run in one of your runs".to_string()
        };
        out.stderr += &format!("{tool}: Loft didn't stop {pids}: {because}, so it asked the user. {why} Stop only your own processes.\n");
        out.code = 1;
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

/// Sends `signal` to each PID, noting failures the way kill does.
fn send(pids: &[i32], signal: i32, tool: &str, out: &mut Output) {
    for &pid in pids {
        if unsafe { libc::kill(pid, signal) } != 0 {
            let e = std::io::Error::last_os_error();
            let why = match e.raw_os_error() {
                Some(libc::ESRCH) => "No such process".to_string(),
                Some(libc::EPERM) => "Operation not permitted".to_string(),
                _ => e.to_string(),
            };
            out.stderr += &format!("{tool}: {pid}: {why}\n");
            out.code = 1;
        }
    }
}

fn plan(req: &KillRequest, procs: &[Proc]) -> Plan {
    let caller_side = ancestors(procs, req.caller);
    // Without a directory that exists, Loft's own, as before.
    let cwd = req.cwd.as_deref().filter(|d| d.is_absolute() && d.is_dir());
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
    let mut signal = libc::SIGTERM;
    let mut rest = args;
    match args.first().map(String::as_str) {
        None => return Plan::Done(Output::error(2, KILL_USAGE.into())),
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
    let group = |pgid: i32| -> Vec<i32> { procs.iter().map(|p| p.pid).filter(|&p| unsafe { libc::getpgid(p) } == pgid).collect() };
    let uid = unsafe { libc::getuid() };
    for a in rest {
        let found = match a.parse::<i32>() {
            Ok(pid) if pid > 0 => vec![pid],
            Ok(0) => group(unsafe { libc::getpgid(caller) }),
            Ok(-1) => procs.iter().filter(|p| p.uid == uid && p.pid != caller && p.pid > 1).map(|p| p.pid).collect(),
            Ok(n) => group(-n),
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
fn plan_pkill(args: &[String], caller_side: &HashSet<i32>, cwd: Option<&Path>) -> Plan {
    let (signal, rest) = match args.first().and_then(|a| signal_of(a.strip_prefix('-')?)) {
        Some(sig) => (sig, &args[1..]),
        None => (libc::SIGTERM, args),
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
fn plan_killall(args: &[String], caller_side: &HashSet<i32>, cwd: Option<&Path>) -> Plan {
    if args.iter().any(|a| a == "-s" || a == "-l") {
        return Plan::Done(run("/usr/bin/killall", args, cwd));
    }
    let shown = run("/usr/bin/killall", &[&["-s".to_string()], args].concat(), cwd);
    if shown.code != 0 {
        return Plan::Done(shown);
    }
    // One `kill -<signal> <pid>` line a process.
    let mut signal = libc::SIGTERM;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::{fake_claude_state, until};
    use crate::agents::{enqueue, is_running, workspace};
    use std::os::unix::process::CommandExt;
    use std::sync::Arc;

    // Every process these tests signal is one they start themselves, by PID.

    /// A `sleep` the test starts in a process session of its own, stopped by PID when dropped.
    struct Sleeper(std::sync::Mutex<std::process::Child>);

    impl Sleeper {
        fn start(own_session: bool) -> Self {
            Self::start_as(Path::new("/bin/sleep"), own_session)
        }

        /// From `path`, a link to sleep: killall finds a process by that name.
        fn start_as(path: &Path, own_session: bool) -> Self {
            let mut cmd = std::process::Command::new(path);
            cmd.arg("60");
            if own_session {
                unsafe { cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) }) };
            }
            Sleeper(std::sync::Mutex::new(cmd.spawn().unwrap()))
        }

        fn pid(&self) -> i32 {
            self.0.lock().unwrap().id() as i32
        }

        fn alive(&self) -> bool {
            self.0.lock().unwrap().try_wait().unwrap().is_none()
        }

        fn stop(&self) {
            let mut child = self.0.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
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
        until("both runs to record a session", 10, || sessions(&storage, &a.id).len() == 1 && sessions(&storage, &b.id).len() == 1).await;
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
        until("the run to stop", 10, || !is_running(&st, &a.id)).await;
        enqueue(&st, &a.id, Some("busy".into()), "user");
        until("the second session", 10, || sessions(&storage, &a.id).len() == 2).await;
        crate::agents::stop(&st, &a.id);
        crate::agents::stop(&st, &b.id);
        until("the runs to stop", 10, || !is_running(&st, &a.id) && !is_running(&st, &b.id)).await;
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
            crate::agents::decide(&st, &req.id, "deny", None, None).unwrap();
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
        // "Always allow" is about these processes only: no rule is saved.
        crate::agents::decide(&st, &req.id, "allow_always", None, None).unwrap();
        let v = asking.await.unwrap();
        assert_eq!(v["code"], 0, "{v}");
        until("the process to stop", 5, || !other.alive()).await;
        assert_eq!(st.db.get_permission(&req.id).unwrap().unwrap().status, "allowed_once");
        assert!(st.db.get_agent(&a.id).unwrap().unwrap().allowed_tools.is_empty());
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
        assert!(crate::agents::decide(&st, &id, "allow_once", None, None).is_err());
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
        crate::agents::decide(&st, &pending(&st, &a.id)[0].id, "deny", None, None).unwrap();
        let v = asking.await.unwrap();
        assert_eq!(v["code"], 1);
        assert!(v["stderr"].as_str().unwrap().contains("The user denied it."), "{v}");
    }

    /// A link to sleep with a name no process outside this test has, for killall.
    fn unique_sleep(st: &AppState) -> (PathBuf, String) {
        let name = format!("ks{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let path = st.cfg.storage_dir.parent().unwrap().join(&name);
        std::os::unix::fs::symlink("/bin/sleep", &path).unwrap();
        (path, name)
    }

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
        st.cfg = Arc::new(crate::state::Config { port, storage_dir: st.cfg.storage_dir.clone(), xcrun: st.cfg.xcrun.clone(), xcode: st.cfg.xcode.clone() });
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
        until("the command", 15, || !is_running(st, id) && out().contains("exit ")).await;
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
            until("the server to stop", 5, || !alive(pid)).await;
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
