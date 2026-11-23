//! Herdr UI: Loft's agents as agents in the Herdr terminal multiplexer.
//!
//! The Herdr side is a plugin in `herdr/` at the repo root: a small terminal client (`loft-herdr`)
//! that shows one agent's chat per Herdr tab and talks to this API like the web app does.
//!
//! - Set up on this machine (`POST /api/herdr/setup`): builds the client, points it at this Loft,
//!   links the plugin into the local Herdr and opens the agents' tabs.
//! - Set up on another machine: Settings shows a one-line command that runs `install.sh` there. It
//!   downloads the plugin files and, when that machine has the same OS and CPU, the built client.
//! - Open an agent in Herdr (`POST /api/herdr/focus/{agent_id}`): for the sandboxed Mac app, which can't
//!   reach Herdr's socket itself. Focuses the agent's pane and brings the terminal running Herdr forward.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::api::{ApiError, ApiResult, bad};
use crate::state::AppState;

const PLUGIN_ID: &str = "loft";

fn root(state: &AppState) -> PathBuf {
    state.cfg.storage_dir.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn plugin_dir(state: &AppState) -> PathBuf {
    root(state).join("herdr")
}

fn built(state: &AppState) -> PathBuf {
    plugin_dir(state).join("target").join("release").join("loft-herdr")
}

/// This machine's `os-arch`, as `install.sh` names it: `macos-aarch64`, `linux-x86_64`.
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// A command on PATH, else in the usual install folders (Loft may run as a service with a short PATH).
fn find(cmd: &str) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(cmd)).find(|p| p.is_file()));
    on_path.or_else(|| {
        [home().join(".local/bin"), home().join(".cargo/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]
            .into_iter()
            .map(|d| d.join(cmd))
            .find(|p| p.is_file())
    })
}

async fn run(program: &Path, args: &[&str], dir: Option<&Path>) -> Result<String, String> {
    let mut c = Command::new(program);
    c.args(args).kill_on_drop(true);
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let out = tokio::time::timeout(Duration::from_secs(600), c.output())
        .await
        .map_err(|_| format!("`{}` took too long", program.display()))?
        .map_err(|e| format!("couldn't run {}: {e}", program.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = err.lines().collect();
        let start = lines.iter().position(|l| l.starts_with("error")).unwrap_or(lines.len().saturating_sub(6));
        Err(lines[start..].iter().take(6).copied().collect::<Vec<_>>().join("\n"))
    }
}

/// One build at a time: Settings and a remote install may ask together.
static BUILD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn build(state: &AppState) -> Result<PathBuf, String> {
    let _one = BUILD.lock().await;
    let cargo = find("cargo").ok_or("cargo isn't installed, so the Herdr UI client can't be built")?;
    run(&cargo, &["build", "--release", "--quiet"], Some(&plugin_dir(state))).await.map_err(|e| format!("building the Herdr UI client failed: {e}"))?;
    Ok(built(state))
}

/// The plugin Herdr has linked under Loft's id, if any.
async fn linked(herdr: &Path) -> Option<Value> {
    let out = run(herdr, &["plugin", "list", "--json"], None).await.ok()?;
    let v: Value = serde_json::from_str(&out).ok()?;
    v.pointer("/result/plugins")?.as_array()?.iter().find(|p| p.get("plugin_id").and_then(|i| i.as_str()) == Some(PLUGIN_ID)).cloned()
}

/// `GET /api/herdr`: whether Herdr is here and what is set up.
pub async fn status(State(state): State<AppState>) -> ApiResult<Value> {
    let herdr = find("herdr");
    let (version, plugin) = match &herdr {
        Some(h) => (run(h, &["--version"], None).await.ok(), linked(h).await),
        None => (None, None),
    };
    let config = std::fs::read_to_string(home().join(".config/loft-herdr/config.json")).ok().and_then(|c| serde_json::from_str::<Value>(&c).ok());
    Ok(Json(json!({
        "herdr": herdr.map(|p| json!({ "path": p, "version": version })),
        "linked": plugin.as_ref().map(|p| json!({ "root": p.get("plugin_root"), "enabled": p.get("enabled") })),
        "built": built(&state).is_file(),
        "platform": platform(),
        "url": config.and_then(|c| c.get("url").cloned()),
    })))
}

/// `POST /api/herdr/setup`: everything needed for this machine's Herdr to show Loft's agents.
pub async fn setup_local(State(state): State<AppState>) -> ApiResult<Value> {
    let herdr = find("herdr").ok_or_else(|| bad("Herdr isn't installed on this machine. Install it from herdr.dev, then try again."))?;
    let mut done = vec![];
    let bin = build(&state).await.map_err(bad)?;
    done.push("Built the Herdr UI client".to_string());

    let url = format!("http://localhost:{}", state.cfg.port);
    let config = home().join(".config/loft-herdr");
    std::fs::create_dir_all(&config).map_err(|e| bad(e.to_string()))?;
    std::fs::write(config.join("config.json"), format!("{}\n", serde_json::to_string_pretty(&json!({ "url": url })).unwrap())).map_err(|e| bad(e.to_string()))?;
    done.push(format!("Pointed it at {url}"));

    // A plain command on PATH, so Herdr can start a chat again after it restarts.
    #[cfg(unix)]
    {
        let link = home().join(".local/bin/loft-herdr");
        if std::fs::symlink_metadata(&link).map(|m| m.file_type().is_symlink()).unwrap_or(true) {
            let _ = std::fs::create_dir_all(link.parent().unwrap());
            let _ = std::fs::remove_file(&link);
            if std::os::unix::fs::symlink(&bin, &link).is_ok() {
                done.push(format!("Linked {}", link.display()));
            }
        }
    }

    let dir = plugin_dir(&state);
    let current = linked(&herdr).await;
    let here = current.as_ref().and_then(|p| p.get("plugin_root")).and_then(|r| r.as_str()).map(PathBuf::from);
    if here.as_deref() != Some(dir.as_path()) {
        if current.is_some() {
            let _ = run(&herdr, &["plugin", "unlink", PLUGIN_ID], None).await;
        }
        run(&herdr, &["plugin", "link", &dir.to_string_lossy()], None).await.map_err(|e| bad(format!("Herdr didn't link the plugin: {e}")))?;
        done.push("Added the Loft plugin to Herdr".to_string());
    } else if current.as_ref().and_then(|p| p.get("enabled")).and_then(|e| e.as_bool()) == Some(false) {
        let _ = run(&herdr, &["plugin", "enable", PLUGIN_ID], None).await;
        done.push("Turned the Loft plugin back on".to_string());
    }

    let message = match run(&bin, &["sync"], None).await {
        Ok(out) => out,
        Err(_) => "Herdr isn't running. Loft's agents appear in a Loft workspace the next time you start it.".to_string(),
    };
    Ok(Json(json!({ "ok": true, "done": done, "message": message })))
}

/// Herdr's socket: the pane metadata token each Loft chat pane reports its agent id under (herdr/src/herdr.rs).
const TOKEN: &str = "loft_agent";

fn socket() -> PathBuf {
    std::env::var_os("HERDR_SOCKET_PATH").filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".config/herdr/herdr.sock"))
}

/// One request over Herdr's socket API (newline-delimited JSON); `None` when Herdr isn't running or fails.
#[cfg(unix)]
async fn call(socket: &Path, method: &str, params: Value) -> Option<Value> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let talk = async {
        let mut s = tokio::net::UnixStream::connect(socket).await.ok()?;
        let line = format!("{}\n", json!({ "id": "loft_focus", "method": method, "params": params }));
        s.write_all(line.as_bytes()).await.ok()?;
        let mut reply = String::new();
        BufReader::new(s).read_line(&mut reply).await.ok()?;
        let v: Value = serde_json::from_str(&reply).ok()?;
        if v.get("error").is_some() { None } else { Some(v.get("result").cloned().unwrap_or(Value::Null)) }
    };
    tokio::time::timeout(Duration::from_secs(10), talk).await.ok().flatten()
}

/// Herdr's socket is a Unix one: on Windows, Herdr never answers.
#[cfg(not(unix))]
async fn call(_socket: &Path, _method: &str, _params: Value) -> Option<Value> {
    None
}

/// The pane showing this Loft agent's chat in a `pane.list` reply, if Herdr has one.
fn pane_in(list: &Value, agent_id: &str) -> Option<String> {
    list.get("panes")?.as_array()?.iter().find(|p| p.pointer(&format!("/tokens/{TOKEN}")).and_then(|v| v.as_str()) == Some(agent_id))?.get("pane_id")?.as_str().map(str::to_string)
}

/// Focuses the agent's pane; when it has none, runs `sync` (if given) to open its tab and looks again.
async fn focus_pane(socket: &Path, agent_id: &str, sync: Option<&Path>) -> Result<(), ApiError> {
    let not_running = || ApiError(StatusCode::CONFLICT, "Herdr isn't running.".into());
    let list = call(socket, "pane.list", json!({})).await.ok_or_else(not_running)?;
    let mut pane = pane_in(&list, agent_id);
    if pane.is_none()
        && let Some(bin) = sync
    {
        let _ = run(bin, &["sync"], None).await;
        pane = call(socket, "pane.list", json!({})).await.and_then(|l| pane_in(&l, agent_id));
    }
    let pane = pane.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "This agent has no tab in Herdr.".into()))?;
    call(socket, "pane.focus", json!({ "pane_id": pane })).await.ok_or_else(not_running)?;
    Ok(())
}

/// The app a Herdr client runs in, from `ps -axo pid=,ppid=,ruid=,args=` (only processes of user `uid` count; the real uid, since setuid `login` between the shell and the terminal shows uid 0): the nearest ancestor of a `herdr` client
/// process (not `herdr server`) that runs from an app bundle: the client's pid and that bundle's path.
#[cfg_attr(not(unix), allow(dead_code))]
fn terminal_app(ps: &str, uid: u32) -> Option<(u32, String)> {
    let procs: Vec<(u32, u32, &str)> = ps
        .lines()
        .filter_map(|l| {
            let (pid, rest) = l.trim_start().split_once(char::is_whitespace)?;
            let (ppid, rest) = rest.trim_start().split_once(char::is_whitespace)?;
            let (owner, args) = rest.trim_start().split_once(char::is_whitespace).unwrap_or((rest.trim_start(), ""));
            (owner.parse::<u32>().ok()? == uid).then_some((pid.parse().ok()?, ppid.parse().ok()?, args.trim_start()))
        })
        .collect();
    let is_client = |args: &str| {
        let mut words = args.split_whitespace();
        words.next().is_some_and(|cmd| cmd.rsplit('/').next() == Some("herdr")) && !words.any(|w| w == "server")
    };
    procs.iter().filter(|p| is_client(p.2)).find_map(|&(client, mut ppid, _)| {
        for _ in 0..32 {
            let &(_, up, args) = procs.iter().find(|p| p.0 == ppid)?;
            if let Some(end) = args.find(".app/Contents/MacOS/") {
                return Some((client, args[..end + 4].to_string()));
            }
            ppid = up;
        }
        None
    })
}

/// The real path of the program process `pid` runs, from the kernel (`ps -o comm=` gives only the name it was
/// started with, or a path cut short).
#[cfg(all(unix, not(target_os = "macos")))]
fn exe_of(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[cfg(target_os = "macos")]
fn exe_of(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0u8; 4096];
    // SAFETY: proc_pidpath writes at most `buf.len()` bytes into `buf` and returns how many.
    let n = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..n as usize])))
}

/// Whether `exe` is the `herdr` Loft found, by real path.
#[cfg_attr(not(unix), allow(dead_code))]
fn is_herdr_exe(exe: &Path, herdr: &Path) -> bool {
    match (std::fs::canonicalize(exe), std::fs::canonicalize(herdr)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Brings the terminal running Herdr to the front; best effort (Herdr may run in tmux or over ssh). Only this
/// user's processes count, and the client must really run the `herdr` executable Loft found (its path from the
/// system, not the process's own argv[0]), so another process can't name an app for Loft to open.
#[cfg(unix)]
async fn raise_terminal() {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    let Ok(ps) = run(Path::new("/bin/ps"), &["-axo", "pid=,ppid=,ruid=,args="], None).await else { return };
    let Some((client, app)) = terminal_app(&ps, uid) else { return };
    let (Some(herdr), Some(exe)) = (find("herdr"), exe_of(client)) else { return };
    if is_herdr_exe(&exe, &herdr) {
        let _ = run(Path::new("/usr/bin/open"), &["-a", &app], None).await;
    }
}

/// Windows has no Herdr.
#[cfg(not(unix))]
async fn raise_terminal() {}

/// `POST /api/herdr/focus/{agent_id}`: shows the agent's tab in Herdr and brings its terminal forward.
/// 409 when Herdr isn't running, 404 when the agent has no tab even after a sync.
pub async fn focus(State(state): State<AppState>, UrlPath(agent_id): UrlPath<String>) -> ApiResult<Value> {
    // Only Loft's agents: an unknown id would start `loft-herdr sync` on every call.
    state.db.get_agent(&agent_id)?.ok_or_else(crate::api::not_found)?;
    let sync = Some(built(&state)).filter(|b| b.is_file()).or_else(|| find("loft-herdr"));
    focus_pane(&socket(), &agent_id, sync.as_deref()).await?;
    raise_terminal().await;
    Ok(Json(json!({ "ok": true })))
}

fn text(body: String, kind: &'static str) -> Response {
    ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "no-store")], body).into_response()
}

/// `GET /api/herdr/install.sh`: sets up Herdr UI on another machine (`curl … | sh -s -- <loft-url>`).
pub async fn install_script() -> Response {
    text(include_str!("../../herdr/install.sh").to_string(), "text/x-shellscript; charset=utf-8")
}

/// `GET /api/herdr/files/{name}`: the plugin's manifest and launcher, for `install.sh`.
pub async fn file(State(state): State<AppState>, UrlPath(name): UrlPath<String>) -> Result<Response, ApiError> {
    let path = match name.as_str() {
        "herdr-plugin.toml" => plugin_dir(&state).join("herdr-plugin.toml"),
        "loft-herdr" => plugin_dir(&state).join("bin").join("loft-herdr"),
        _ => return Err(crate::api::not_found()),
    };
    let body = tokio::fs::read_to_string(&path).await.map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(text(body, "text/plain; charset=utf-8"))
}

/// `GET /api/herdr/bin/{platform}`: the built client, for a machine with this one's OS and CPU.
pub async fn binary(State(state): State<AppState>, headers: axum::http::HeaderMap, UrlPath(wanted): UrlPath<String>) -> Result<Response, ApiError> {
    // It builds when there is no binary yet: not for a page on another site.
    crate::guard::same_site_read(&headers)?;
    if wanted != platform() {
        return Err(ApiError(StatusCode::NOT_FOUND, format!("this Loft has a {} build only", platform())));
    }
    let path = if built(&state).is_file() { built(&state) } else { build(&state).await.map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, e))? };
    let bytes = tokio::fs::read(&path).await.map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream"), (header::CACHE_CONTROL, "no-store")], bytes).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty folder for a socket, unique per test.
    fn temp() -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("loft-herdr-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A fake Herdr on a socket in a temp folder: answers `pane.list` with these panes and records `pane.focus`.
    #[cfg(unix)]
    async fn fake_herdr(panes: Value) -> (PathBuf, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let (path, focused, _) = fake_herdr_counting(panes).await;
        (path, focused)
    }

    /// The same, also counting the `pane.list` calls.
    #[cfg(unix)]
    async fn fake_herdr_counting(panes: Value) -> (PathBuf, std::sync::Arc<std::sync::Mutex<Vec<String>>>, std::sync::Arc<std::sync::Mutex<u32>>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let path = temp().join("herdr.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let focused = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let seen = focused.clone();
        let lists = std::sync::Arc::new(std::sync::Mutex::new(0));
        let counted = lists.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = listener.accept().await {
                let (r, mut w) = s.into_split();
                let mut line = String::new();
                BufReader::new(r).read_line(&mut line).await.unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let result = match req["method"].as_str().unwrap() {
                    "pane.list" => {
                        *counted.lock().unwrap() += 1;
                        json!({ "panes": panes })
                    }
                    "pane.focus" => {
                        seen.lock().unwrap().push(req["params"]["pane_id"].as_str().unwrap().to_string());
                        json!({})
                    }
                    _ => json!(null),
                };
                w.write_all(format!("{}\n", json!({ "id": req["id"], "result": result })).as_bytes()).await.unwrap();
            }
        });
        (path, focused, lists)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn focuses_the_agents_pane() {
        let panes = json!([{ "pane_id": "w1:p1", "tokens": { "loft_agent": "a1" } }, { "pane_id": "w1:p2", "tokens": { "loft_agent": "a2" } }]);
        let (sock, focused) = fake_herdr(panes).await;
        focus_pane(&sock, "a2", None).await.unwrap();
        assert_eq!(*focused.lock().unwrap(), vec!["w1:p2".to_string()]);
    }

    #[tokio::test]
    async fn an_id_that_is_not_an_agent_is_refused_before_anything_runs() {
        let st = crate::tasks::tests::state();
        let err = focus(State(st), UrlPath("nobody".into())).await.unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn asks_herdr_for_its_panes_once() {
        let panes = json!([{ "pane_id": "w1:p1", "tokens": { "loft_agent": "a1" } }]);
        let (sock, _, lists) = fake_herdr_counting(panes).await;
        focus_pane(&sock, "a1", None).await.unwrap();
        assert_eq!(*lists.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn says_when_herdr_isnt_running() {
        let err = focus_pane(&temp().join("herdr.sock"), "a1", None).await.unwrap_err();
        assert_eq!((err.0, err.1.as_str()), (StatusCode::CONFLICT, "Herdr isn't running."));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn says_when_the_agent_has_no_tab() {
        let (sock, focused) = fake_herdr(json!([{ "pane_id": "w1:p1", "tokens": { "loft_agent": "a1" } }, { "pane_id": "w1:p3" }])).await;
        let err = focus_pane(&sock, "a9", None).await.unwrap_err();
        assert_eq!((err.0, err.1.as_str()), (StatusCode::NOT_FOUND, "This agent has no tab in Herdr."));
        assert!(focused.lock().unwrap().is_empty());
    }

    #[test]
    fn finds_the_terminal_app_running_the_herdr_client() {
        // `login` is setuid root: `ps -o uid=` shows 0 for it, `ruid=` (read here) the user, 501.
        let ps = "\
    1     0     0 /sbin/launchd
  500     1   501 /Applications/Ghostty.app/Contents/MacOS/ghostty
  501   500   501 /usr/bin/login -flp gustavo /bin/zsh
  502   501   501 -zsh
  503   502   501 herdr
  600     1   501 /Users/gustavo/.local/bin/herdr server
  700     1   501 /System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal
";
        assert_eq!(terminal_app(ps, 501), Some((503, "/Applications/Ghostty.app".to_string())));
        assert_eq!(terminal_app("  600     1   501 /Users/me/.local/bin/herdr server\n", 501), None, "the server has no terminal");
    }

    #[test]
    fn only_this_users_processes_count() {
        // Another user's herdr client, and this user's client under another user's terminal.
        let theirs = "  500     1   502 /Applications/Evil.app/Contents/MacOS/evil\n  503   500   502 herdr\n";
        assert_eq!(terminal_app(theirs, 501), None);
        let mixed = "  500     1   502 /Applications/Evil.app/Contents/MacOS/evil\n  503   500   501 herdr\n";
        assert_eq!(terminal_app(mixed, 501), None, "its terminal belongs to someone else");
        assert_eq!(terminal_app(theirs, 502), Some((503, "/Applications/Evil.app".to_string())));
    }

    #[cfg(unix)]
    #[test]
    fn the_herdr_exe_is_checked_by_real_path() {
        let dir = temp();
        let real = dir.join("herdr");
        std::fs::write(&real, "").unwrap();
        let link = dir.join("herdr-link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let other = dir.join("other");
        std::fs::write(&other, "").unwrap();
        assert!(is_herdr_exe(&real, &link), "through a symlink");
        assert!(!is_herdr_exe(&other, &real), "another program doesn't count, whatever it's called");
        // What `ps -o comm=` would give: a bare name or a path cut to 16 characters. Neither is a path to trust.
        assert!(!is_herdr_exe(Path::new("herdr"), &real));
        assert!(!is_herdr_exe(Path::new(&real.to_string_lossy()[..16]), &real));
    }

    #[cfg(unix)]
    #[test]
    fn the_exe_comes_from_the_kernel_not_the_process_name() {
        let me = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(std::fs::canonicalize(exe_of(std::process::id()).unwrap()).unwrap(), me);
        assert_eq!(exe_of(u32::MAX - 1), None);
    }

    #[test]
    fn names_the_platform_like_install_sh() {
        let p = platform();
        assert!(p.starts_with(std::env::consts::OS) && p.contains('-'), "{p}");
    }

    #[test]
    fn the_install_script_and_plugin_files_ship_with_loft() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("herdr");
        for f in ["herdr-plugin.toml", "bin/loft-herdr", "install.sh", "Cargo.toml"] {
            assert!(dir.join(f).is_file(), "missing herdr/{f}");
        }
        let script = include_str!("../../herdr/install.sh");
        assert!(script.contains("/api/herdr/bin/"));
        assert!(script.contains("plugin link"));
    }
}
