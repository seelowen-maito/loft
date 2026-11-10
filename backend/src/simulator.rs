//! The iOS Simulator running on this Mac, viewable and controllable from other devices.
//! `simctl` can only take screenshots; live video and touch input go through AXe (github.com/cameroncooke/AXe).
//!
//! Each agent builds and runs on its own agent device, which Loft creates (named after the agent) the first time the
//! agent asks for it, and deletes with the agent, so two agents never install over each other.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::db::Agent;
use crate::state::{AppState, XcodeLookup};

/// The variable holding the UDID of the agent's device, in each of its runs once it has one.
pub const DEVICE_VAR: &str = "LOFT_SIMULATOR_UDID";

/// Where AXe may be: on PATH, Homebrew, or a manual install in ~/.local/bin (a background service has a short PATH).
pub fn axe() -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let candidates = [home.join(".local/bin/axe"), "/opt/homebrew/bin/axe".into(), "/usr/local/bin/axe".into()];
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).map(|d| d.join("axe")).collect::<Vec<_>>());
    on_path.chain(candidates).find(|p| p.is_file())
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn valid_udid(u: &str) -> bool {
    !u.is_empty() && u.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// What the Simulator page and the `simulator_device` tool say when there is no Xcode to run `simctl` from.
pub const NO_XCODE: &str = "Xcode is missing: xcode-select points at the Command Line Tools, which have no Simulator, and no Xcode was found in /Applications or /Volumes. Install Xcode, or start Loft with DEVELOPER_DIR set to your Xcode's Contents/Developer.";

/// Which Xcode `xcrun` should use.
#[derive(Debug, PartialEq, Clone)]
pub enum Xcode {
    /// The one `DEVELOPER_DIR` or `xcode-select` already picks.
    Selected,
    /// This one (its Contents/Developer), as `DEVELOPER_DIR`: `xcode-select` points at the Command Line Tools.
    At(PathBuf),
    /// `xcode-select` points at the Command Line Tools and there is no Xcode.
    Missing,
}

/// Finds Xcode the way `ios/scripts/test.sh` does: a `DEVELOPER_DIR` already set is used as is; when `xcode-select`
/// points at the Command Line Tools, the first of /Applications/Xcode.app, /Applications/Xcode*.app,
/// /Volumes/*/Xcode.app and /Volumes/*/xcode/Xcode.app.
pub async fn find_xcode(lookup: &XcodeLookup) -> Xcode {
    if lookup.developer_dir.is_some() {
        return Xcode::Selected;
    }
    let selected = match Command::new(&lookup.xcode_select).arg("-p").stdin(Stdio::null()).output().await {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        // No xcode-select to ask: xcrun says what's wrong, if anything.
        _ => return Xcode::Selected,
    };
    if !selected.contains("CommandLineTools") {
        return Xcode::Selected;
    }
    let sorted = |dir: PathBuf| {
        let mut names: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        names.sort();
        names
    };
    let apps = lookup.root.join("Applications");
    let volumes = sorted(lookup.root.join("Volumes"));
    let candidates = std::iter::once(apps.join("Xcode.app"))
        .chain(sorted(apps).into_iter().filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("Xcode") && n.ends_with(".app"))))
        .chain(volumes.iter().map(|v| v.join("Xcode.app")))
        .chain(volumes.iter().map(|v| v.join("xcode/Xcode.app")));
    candidates.map(|x| x.join("Contents/Developer")).find(|d| d.is_dir()).map_or(Xcode::Missing, Xcode::At)
}

/// How long a [`find_xcode`] result is reused: the Simulator page runs simctl about 4 times a second, and a newly
/// installed Xcode is still found within a minute.
const XCODE_TTL: Duration = Duration::from_secs(60);

/// The last [`find_xcode`] result, and when it was found.
#[derive(Default)]
pub struct XcodeCache(tokio::sync::Mutex<Option<(Instant, Xcode)>>);

/// The Xcode `xcrun` should use, looked up at most once per [`XCODE_TTL`].
pub async fn xcode(state: &AppState) -> Xcode {
    let mut cached = state.xcode.0.lock().await;
    if let Some((_, x)) = cached.as_ref().filter(|(at, _)| at.elapsed() < XCODE_TTL) {
        return x.clone();
    }
    let x = find_xcode(&state.cfg.xcode).await;
    *cached = Some((Instant::now(), x.clone()));
    x
}

/// `xcrun simctl`, with the Xcode it needs; `Err(NO_XCODE)` when there is none.
async fn simctl(state: &AppState) -> Result<Command, String> {
    let mut c = Command::new(&state.cfg.xcrun);
    match xcode(state).await {
        Xcode::Selected => {}
        Xcode::At(dir) => {
            c.env("DEVELOPER_DIR", dir);
        }
        Xcode::Missing => return Err(NO_XCODE.into()),
    }
    c.arg("simctl").stdin(Stdio::null());
    Ok(c)
}

/// A `simctl … -j` listing: null when simctl failed, `Err(NO_XCODE)` when there is no Xcode.
async fn simctl_json(state: &AppState, args: &[&str]) -> Result<Value, String> {
    Ok(match simctl(state).await?.args(args).arg("-j").output().await {
        Ok(o) if o.status.success() => serde_json::from_slice(&o.stdout).unwrap_or_default(),
        _ => Value::Null,
    })
}

/// Runs `simctl` with these arguments.
async fn run_simctl<I: IntoIterator<Item = S>, S: AsRef<std::ffi::OsStr>>(state: &AppState, args: I) -> Result<std::process::Output, String> {
    simctl(state).await?.args(args).output().await.map_err(|e| format!("Couldn't run xcrun: {e}"))
}

/// The devices in a `simctl list devices -j`, with the runtime each runs.
fn devices(list: &Value) -> Vec<(&str, &Value)> {
    let Some(runtimes) = list["devices"].as_object() else { return vec![] };
    runtimes.iter().flat_map(|(r, ds)| ds.as_array().into_iter().flatten().map(move |d| (r.as_str(), d))).collect()
}

/// Booted simulators and whether they can be controlled: `{ interactive, devices: [{ udid, name, runtime, agent }] }`.
/// `agent` (`{ id, name }`) is the agent whose device it is, or null for one the user booted; agents' come first.
/// With no Xcode to run simctl from, no devices and an `error` saying so.
pub async fn booted(State(state): State<AppState>) -> Response {
    let v = match simctl_json(&state, &["list", "devices", "booted"]).await {
        Ok(v) => v,
        Err(e) => return Json(json!({ "interactive": false, "devices": [], "error": e })).into_response(),
    };
    let names: HashMap<String, String> = state.db.list_agents().unwrap_or_default().into_iter().map(|a| (a.id, a.name)).collect();
    let owners: HashMap<String, String> = state.db.agent_devices().unwrap_or_default().into_iter().collect();
    let mut list = Vec::new();
    for (runtime, d) in devices(&v) {
        // com.apple.CoreSimulator.SimRuntime.iOS-27-0 -> iOS 27.0
        let runtime = runtime.rsplit('.').next().unwrap_or(runtime).replacen('-', " ", 1).replace('-', ".");
        let agent = d["udid"].as_str().and_then(|u| owners.get(u)).and_then(|id| Some(json!({ "id": id, "name": names.get(id)? })));
        list.push(json!({ "udid": d["udid"], "name": d["name"], "runtime": runtime, "agent": agent }));
    }
    list.sort_by_key(|d| (d["agent"].is_null(), d["agent"]["name"].as_str().unwrap_or("").to_lowercase()));
    Json(json!({ "interactive": axe().is_some(), "devices": list })).into_response()
}

/// What an agent's device is called, in Xcode and on the Simulator page.
fn device_name(agent: &Agent) -> String {
    format!("{} (Loft)", agent.name)
}

/// The agent's own device: the one it has (renamed if the agent was), else a new one, the first time it needs one or
/// after its device was deleted outside Loft. Returns its UDID and name.
pub async fn agent_device(state: &AppState, agent: &Agent) -> Result<(String, String), String> {
    // One at a time, so two calls of one agent don't both create a device.
    static CREATING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = CREATING.lock().await;
    let name = device_name(agent);
    let all = simctl_json(state, &["list", "devices"]).await?;
    if all.is_null() {
        return Err("Couldn't run `xcrun simctl`. Is Xcode installed, and selected with `xcode-select`?".into());
    }
    if let Some(udid) = state.db.agent_device(&agent.id).map_err(|e| e.to_string())?
        && let Some((_, d)) = devices(&all).into_iter().find(|(_, d)| d["udid"] == udid.as_str()) {
            if d["name"] != name.as_str() {
                let _ = run_simctl(state, ["rename", &udid, &name]).await;
            }
            return Ok((udid, name));
        }
    let Some((kind, runtime)) = new_device_kind(state, &all).await else {
        return Err("No iOS Simulator runtime is installed. Add one in Xcode (Settings > Components).".into());
    };
    let out = run_simctl(state, ["create", &name, &kind, &runtime]).await?;
    let udid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !valid_udid(&udid) {
        return Err(format!("Couldn't create a Simulator device: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    state.db.set_agent_device(&agent.id, &udid).map_err(|e| e.to_string())?;
    Ok((udid, name))
}

/// The device type and runtime for a new agent device: those of the iPhone the user used last, else the newest iOS
/// runtime's newest iPhone.
async fn new_device_kind(state: &AppState, all: &Value) -> Option<(String, String)> {
    let last = devices(all)
        .into_iter()
        .filter(|(r, d)| r.contains(".iOS-") && d["isAvailable"] == true && d["deviceTypeIdentifier"].as_str().is_some_and(|t| t.contains(".iPhone")))
        .max_by_key(|(_, d)| d["lastUsedAt"].as_str().unwrap_or("").to_string());
    if let Some((r, d)) = last {
        return Some((d["deviceTypeIdentifier"].as_str()?.to_string(), r.to_string()));
    }
    let runtimes = simctl_json(state, &["list", "runtimes"]).await.ok()?;
    // Runtimes are listed oldest first, and each one's device types newest first.
    runtimes["runtimes"].as_array()?.iter().rev().filter(|r| r["platform"] == "iOS" && r["isAvailable"] == true).find_map(|r| {
        let t = r["supportedDeviceTypes"].as_array()?.iter().find(|t| t["productFamily"] == "iPhone")?;
        Some((t["identifier"].as_str()?.to_string(), r["identifier"].as_str()?.to_string()))
    })
}

/// Deletes the agent's device, if it has one: it goes with the agent.
pub async fn delete_agent_device(state: &AppState, agent_id: &str) {
    let Ok(Some(udid)) = state.db.agent_device(agent_id) else { return };
    // A booted device can't be deleted.
    let _ = run_simctl(state, ["shutdown", &udid]).await;
    match run_simctl(state, ["delete", &udid]).await {
        Ok(o) if o.status.success() => {
            let _ = state.db.forget_agent_device(&udid);
        }
        // Its row stays: the tidy tries again.
        Ok(o) => tracing::warn!("couldn't delete Simulator device {udid}: {}", String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => tracing::warn!("couldn't delete Simulator device {udid}: {e}"),
    }
}

/// How often orphaned and idle agents' devices are looked for.
const TIDY_EVERY: Duration = Duration::from_secs(10 * 60);

/// What a device is to Loft, by its name and UDID: `None` when it isn't an agent's (the user's own, like "iPhone 17"),
/// else the agent it belongs to, or `Some(None)` for an orphan: the device Loft made for an agent since deleted, or
/// one named for a live agent under the old scheme, "<agent name> <8 hex digits>" (like "Loft Dev 93543007"). A
/// "<name> (Loft)" device this Loft didn't make is left alone: another Loft on this Mac (a demo) may own it.
fn owner<'a>(name: &str, udid: &str, agents: &'a [Agent], kept: &HashMap<String, String>) -> Option<Option<&'a Agent>> {
    if let Some(id) = kept.get(udid) {
        return Some(agents.iter().find(|a| a.id == *id));
    }
    let old = name.rsplit_once(' ').is_some_and(|(who, hex)| hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()) && agents.iter().any(|a| a.name == who));
    old.then_some(None)
}

/// Deletes orphaned agent devices and shuts down the booted devices of idle agents (see [`crate::agents::idle`]),
/// leaving every other device alone. Returns a line per deletion or shutdown, for the clean log.
pub async fn tidy(state: &AppState) -> Vec<crate::sweep::Line> {
    let Ok(all) = simctl_json(state, &["list", "devices"]).await else { return vec![] };
    let agents = state.db.list_agents().unwrap_or_default();
    let kept: HashMap<String, String> = state.db.agent_devices().unwrap_or_default().into_iter().collect();
    let mut lines = vec![];
    for (_, d) in devices(&all) {
        let (Some(udid), Some(name)) = (d["udid"].as_str(), d["name"].as_str()) else { continue };
        if !valid_udid(udid) {
            continue;
        }
        let booted = d["state"] == "Booted";
        let delete = match owner(name, udid, &agents, &kept) {
            None => continue,
            Some(None) => true,
            // Asked again right here, so a run that just started keeps its device.
            Some(Some(a)) if booted && crate::agents::idle(state, a, std::time::SystemTime::now()) => false,
            Some(Some(_)) => continue,
        };
        // A booted device can't be deleted.
        let steps: Vec<&str> = [booted.then_some("shutdown"), delete.then_some("delete")].into_iter().flatten().collect();
        let mut error = None;
        for step in &steps {
            if let Err(e) = simctl_ok(state, [step, udid]).await {
                error = Some(e);
                break;
            }
        }
        let command = steps.iter().map(|a| format!("xcrun simctl {a} {udid}")).collect::<Vec<_>>().join(" && ");
        let bytes = if delete && error.is_none() { device_bytes(d) } else { 0 };
        if delete && error.is_none() {
            let _ = state.db.forget_agent_device(udid);
        }
        let done = if delete { "Deleted Simulator device" } else { "Shut down Simulator device" };
        // An old-scheme device goes under the agent it is named for.
        let named = name.rsplit_once(' ').and_then(|(who, _)| agents.iter().find(|a| a.name == who)).map(|a| a.id.clone());
        let agent = kept.get(udid).cloned().or(named).unwrap_or_default();
        let path = d["dataPath"].as_str().and_then(|p| Path::new(p).parent()).map_or_else(|| udid.into(), Path::to_path_buf);
        lines.push(crate::sweep::Line { agent, path, rel: name.to_string(), bytes, error, command, done: done.into() });
    }
    lines
}

/// Runs a `simctl` subcommand, its error output as the error.
async fn simctl_ok<const N: usize>(state: &AppState, args: [&str; N]) -> Result<(), String> {
    match run_simctl(state, args).await? {
        o if o.status.success() => Ok(()),
        o => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
    }
}

/// The disk space a device takes, as simctl reports it.
fn device_bytes(d: &Value) -> u64 {
    d["dataPathSize"].as_u64().unwrap_or(0) + d["logPathSize"].as_u64().unwrap_or(0)
}

/// Tidies agents' devices every few minutes, the first right after Loft starts; each run that did something is
/// the clean log's.
pub async fn tidier(state: AppState) {
    let mut tick = tokio::time::interval(TIDY_EVERY);
    loop {
        tick.tick().await;
        let lines = tidy(&state).await;
        if !lines.is_empty() {
            crate::sweep::record(&state, "Simulator cleanup", lines).await;
        }
    }
}

/// `GET /api/simulator/devices`: agents' devices for Settings → Disk, biggest first:
/// `{ devices: [{ udid, name, state, bytes, agent: { id, name } }] }`, none with no Xcode.
pub async fn agent_devices(State(state): State<AppState>) -> Json<Value> {
    let all = simctl_json(&state, &["list", "devices"]).await.unwrap_or_default();
    let agents = state.db.list_agents().unwrap_or_default();
    let kept: HashMap<String, String> = state.db.agent_devices().unwrap_or_default().into_iter().collect();
    let mut list: Vec<Value> = devices(&all)
        .into_iter()
        .filter_map(|(_, d)| {
            let a = owner(d["name"].as_str()?, d["udid"].as_str()?, &agents, &kept)??;
            Some(json!({ "udid": d["udid"], "name": d["name"], "state": d["state"], "bytes": device_bytes(d), "agent": { "id": a.id, "name": a.name } }))
        })
        .collect();
    list.sort_by_key(|d| std::cmp::Reverse(d["bytes"].as_u64()));
    Json(json!({ "devices": list }))
}

#[derive(Deserialize)]
pub struct Which {
    /// Device UDID; defaults to the first booted one.
    pub udid: Option<String>,
}

/// A JPEG of the simulator's screen. The fallback view when AXe isn't installed.
pub async fn screenshot(State(state): State<AppState>, Query(q): Query<Which>) -> Response {
    let device = q.udid.filter(|u| valid_udid(u)).unwrap_or_else(|| "booted".into());
    let path = std::env::temp_dir().join(format!("loft-sim-{}.jpg", uuid::Uuid::new_v4()));
    let out = async { simctl(&state).await?.args(["io", &device, "screenshot", "--type=jpeg"]).arg(&path).output().await.map_err(|e| format!("Couldn't run xcrun: {e}")) }.await;
    let bytes = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    match (out, bytes) {
        (Ok(o), Ok(b)) if o.status.success() => {
            ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "no-store")], b).into_response()
        }
        (Ok(o), _) => {
            let msg = String::from_utf8_lossy(&o.stderr);
            let msg = msg.lines().filter(|l| !l.starts_with("Note:")).collect::<Vec<_>>().join(" ");
            err(StatusCode::NOT_FOUND, if msg.trim().is_empty() { "No simulator is booted".into() } else { msg })
        }
        (Err(e), _) => err(StatusCode::NOT_FOUND, e),
    }
}

/// Live MJPEG video of the screen, for an `<img>`. The AXe process lives as long as the response.
pub async fn stream(Query(q): Query<Which>) -> Response {
    let Some(udid) = q.udid.filter(|u| valid_udid(u)) else { return err(StatusCode::BAD_REQUEST, "udid required") };
    let Some(bin) = axe() else { return err(StatusCode::NOT_FOUND, "AXe isn't installed") };
    let child = Command::new(bin)
        .args(["stream-video", "--udid", &udid, "--format", "mjpeg", "--fps", "15", "--quality", "60", "--scale", "0.5"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("Couldn't start AXe: {e}")),
    };
    let mut out = child.stdout.take().unwrap();

    // AXe writes an HTTP response of its own; drop its head and pass the multipart body through.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match out.read(&mut byte).await {
            Ok(1) if head.len() < 4096 => head.push(byte[0]),
            _ => return err(StatusCode::NOT_FOUND, "The simulator stream ended. Is the device still booted?"),
        }
    }
    let boundary = String::from_utf8_lossy(&head)
        .split("boundary=")
        .nth(1)
        .and_then(|b| b.lines().next())
        .map(|b| b.trim().trim_start_matches("--").to_string())
        .unwrap_or_else(|| "mjpegstream".into());

    let body = futures::stream::unfold((out, child), |(mut out, child)| async move {
        let mut buf = vec![0u8; 64 * 1024];
        match out.read(&mut buf).await {
            Ok(0) | Err(_) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(buf), (out, child)))
            }
        }
    });
    (
        [
            (header::CONTENT_TYPE, format!("multipart/x-mixed-replace; boundary={boundary}")),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        Body::from_stream(body),
    )
        .into_response()
}

/// Screen size in points (what AXe takes), from the device type's profile. Cached per device.
async fn screen_points(state: &AppState, udid: &str) -> Option<(f64, f64)> {
    static CACHE: OnceLock<Mutex<HashMap<String, (f64, f64)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(s) = cache.lock().unwrap().get(udid) {
        return Some(*s);
    }
    let all = simctl_json(state, &["list", "devices"]).await.ok()?;
    let type_id = devices(&all).into_iter().find(|(_, d)| d["udid"] == udid)?.1["deviceTypeIdentifier"].as_str()?.to_string();
    let types = simctl_json(state, &["list", "devicetypes"]).await.ok()?;
    let bundle = types["devicetypes"].as_array()?.iter().find(|t| t["identifier"] == type_id.as_str())?["bundlePath"].as_str()?.to_string();
    let plist = PathBuf::from(bundle).join("Contents/Resources/capabilities.plist");
    let out = Command::new("plutil").args(["-convert", "json", "-o", "-"]).arg(&plist).output().await.ok()?;
    let caps: Value = serde_json::from_slice(&out.stdout).ok()?;
    let dims = &caps["capabilities"]["ScreenDimensionsCapability"];
    let scale = dims["main-screen-scale"].as_f64()?;
    let size = (dims["main-screen-width"].as_f64()? / scale, dims["main-screen-height"].as_f64()? / scale);
    cache.lock().unwrap().insert(udid.to_string(), size);
    Some(size)
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Input {
    /// Coordinates are fractions of the screen, 0–1, so the client needn't know the device's size.
    Tap { x: f64, y: f64 },
    Swipe { x: f64, y: f64, x2: f64, y2: f64, duration: f64 },
    Type { text: String },
    /// HID keycode, e.g. 40 Return, 42 Delete.
    Key { code: u16 },
    /// home, lock, side-button, siri, apple-pay
    Button { name: String },
}

#[derive(Deserialize)]
pub struct InputReq {
    pub udid: String,
    #[serde(flatten)]
    pub input: Input,
}

/// Touch, type or press a button. Runs one at a time so a tap lands before the text typed after it.
pub async fn input(State(state): State<AppState>, Json(req): Json<InputReq>) -> Response {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    if !valid_udid(&req.udid) {
        return err(StatusCode::BAD_REQUEST, "invalid udid");
    }
    let Some(bin) = axe() else { return err(StatusCode::NOT_FOUND, "AXe isn't installed") };
    let pt = |fx: f64, fy: f64, (w, h): (f64, f64)| (format!("{:.1}", fx.clamp(0.0, 1.0) * w), format!("{:.1}", fy.clamp(0.0, 1.0) * h));
    let size = match req.input {
        Input::Tap { .. } | Input::Swipe { .. } => match screen_points(&state, &req.udid).await {
            Some(s) => s,
            None => return err(StatusCode::NOT_FOUND, "Couldn't find this simulator's screen size"),
        },
        _ => (0.0, 0.0),
    };
    let args: Vec<String> = match req.input {
        Input::Tap { x, y } => {
            let (x, y) = pt(x, y, size);
            // Physical touch down/up: the default synthetic tap is ignored by some SwiftUI controls.
            vec!["tap".into(), "-x".into(), x, "-y".into(), y, "--tap-style".into(), "physical".into()]
        }
        Input::Swipe { x, y, x2, y2, duration } => {
            let (sx, sy) = pt(x, y, size);
            let (ex, ey) = pt(x2, y2, size);
            let duration = format!("{:.2}", duration.clamp(0.05, 3.0));
            vec!["swipe".into(), "--start-x".into(), sx, "--start-y".into(), sy, "--end-x".into(), ex, "--end-y".into(), ey, "--duration".into(), duration]
        }
        Input::Type { text } => vec!["type".into(), text],
        Input::Key { code } => vec!["key".into(), code.to_string()],
        Input::Button { name } => vec!["button".into(), name],
    };
    let _turn = LOCK.lock().await;
    match Command::new(bin).args(&args).args(["--udid", &req.udid]).stdin(Stdio::null()).output().await {
        Ok(o) if o.status.success() => Json(json!({ "ok": true })).into_response(),
        Ok(o) => {
            let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let msg = if msg.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { msg };
            err(StatusCode::INTERNAL_SERVER_ERROR, msg)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("Couldn't run AXe: {e}")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Arc;

    /// Stands in for `xcrun simctl`: each device is a file in `sim/` beside it, named by UDID, holding its state and
    /// name. Every call is logged to `simctl.log`. One iOS runtime, whose iPhone is an iPhone 18 Pro.
    const FAKE_XCRUN: &str = r#"#!/bin/sh
here=$(dirname "$0"); dir="$here/sim"; mkdir -p "$dir"
echo "$*" >> "$here/simctl.log"
shift
rt=com.apple.CoreSimulator.SimRuntime.iOS-27-0
case "$1" in
  list)
    if [ "$2" = runtimes ]; then
      echo '{"runtimes":[{"identifier":"'$rt'","platform":"iOS","isAvailable":true,"supportedDeviceTypes":[{"identifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro","productFamily":"iPhone"}]}]}'
      exit 0
    fi
    printf '{"devices":{"%s":[' $rt; sep=
    for f in "$dir"/*; do
      [ -f "$f" ] || continue
      state=$(sed -n 1p "$f"); name=$(sed -n 2p "$f")
      [ "$3" = booted ] && [ "$state" != Booted ] && continue
      printf '%s{"udid":"%s","name":"%s","state":"%s","isAvailable":true,"deviceTypeIdentifier":"%s","dataPath":"/sim/%s/data","dataPathSize":%s}' "$sep" "$(basename "$f")" "$name" "$state" "$(sed -n 3p "$f")" "$(basename "$f")" "$(sed -n 4p "$f" | grep . || echo 0)"
      sep=,
    done
    echo ']}}';;
  create) udid=$(uuidgen); printf 'Shutdown\n%s\n%s\n' "$2" "$3" > "$dir/$udid"; echo "$udid";;
  boot|shutdown|rename|delete)
    f="$dir/$2"; [ -f "$f" ] || { echo "Invalid device: $2" >&2; exit 1; }
    case "$1" in
      boot) { echo Booted; sed -n 2,4p "$f"; } > "$f.new";;
      shutdown) { echo Shutdown; sed -n 2,4p "$f"; } > "$f.new";;
      rename) { sed -n 1p "$f"; echo "$3"; sed -n 3,4p "$f"; } > "$f.new";;
      delete) [ "$(sed -n 1p "$f")" = Booted ] && { echo "Unable to delete a booted device" >&2; exit 1; }; rm "$f"; exit 0;;
    esac
    mv "$f.new" "$f";;
  *) echo "unknown subcommand $1" >&2; exit 1;;
esac
"#;

    /// Points the state's `xcrun` at a fake of its own.
    pub(crate) fn fake_xcrun(st: &mut AppState) {
        let dir = st.cfg.storage_dir.parent().unwrap().join("xcrun");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("xcrun");
        std::fs::write(&bin, FAKE_XCRUN).unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        st.cfg = Arc::new(crate::state::Config { port: st.cfg.port, storage_dir: st.cfg.storage_dir.clone(), xcrun: bin, xcode: st.cfg.xcode.clone() });
    }

    /// A fake `xcode-select` printing `selected` for `-p` and logging each call, and a temp root for /Applications and
    /// /Volumes, beside the fake `xcrun`, with nothing found yet. Returns the root.
    pub(crate) fn fake_xcode_select(st: &mut AppState, selected: &str, developer_dir: Option<PathBuf>) -> PathBuf {
        let dir = st.cfg.storage_dir.parent().unwrap().join("xcode");
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let bin = dir.join("xcode-select");
        std::fs::write(&bin, format!("#!/bin/sh\necho \"$*\" >> \"$(dirname \"$0\")/xcode-select.log\"\necho '{selected}'\n")).unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let xcode = XcodeLookup { developer_dir, xcode_select: bin, root: root.clone() };
        st.cfg = Arc::new(crate::state::Config { port: st.cfg.port, storage_dir: st.cfg.storage_dir.clone(), xcrun: st.cfg.xcrun.clone(), xcode });
        st.xcode = Default::default();
        root
    }

    /// Ages the Xcode last found past [`XCODE_TTL`], so the next simctl looks again.
    fn expire_xcode(st: &AppState) {
        if let Some((at, _)) = st.xcode.0.try_lock().unwrap().as_mut() {
            *at = Instant::now().checked_sub(XCODE_TTL).unwrap();
        }
    }

    /// How many times `xcode-select -p` was asked.
    fn xcode_lookups(st: &AppState) -> usize {
        std::fs::read_to_string(st.cfg.xcode.xcode_select.parent().unwrap().join("xcode-select.log")).unwrap_or_default().lines().count()
    }

    /// Makes the fake `xcrun` fail as the Command Line Tools' does unless run with `DEVELOPER_DIR=<expected>`.
    fn needs_developer_dir(st: &AppState, expected: &std::path::Path) {
        let here = st.cfg.xcrun.parent().unwrap();
        std::fs::write(here.join("expect"), expected.to_str().unwrap()).unwrap();
        let check = "[ \"$DEVELOPER_DIR\" = \"$(cat \"$(dirname \"$0\")/expect\")\" ] || { echo 'xcrun: error: unable to find utility \"simctl\"' >&2; exit 72; }\n";
        std::fs::write(&st.cfg.xcrun, FAKE_XCRUN.replacen("\n", &format!("\n{check}"), 1)).unwrap();
    }

    fn mkdir(p: PathBuf) -> PathBuf {
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// The fake's devices, as (UDID, state, name, device type).
    pub(crate) fn fake_devices(st: &AppState) -> Vec<(String, String, String, String)> {
        let dir = st.cfg.xcrun.parent().unwrap().join("sim");
        let mut out: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| {
                let text = std::fs::read_to_string(e.path()).unwrap();
                let l: Vec<&str> = text.lines().collect();
                (e.file_name().to_string_lossy().into_owned(), l[0].into(), l[1].into(), l[2].into())
            })
            .collect();
        out.sort();
        out
    }

    pub(crate) fn simctl_log(st: &AppState) -> String {
        std::fs::read_to_string(st.cfg.xcrun.parent().unwrap().join("simctl.log")).unwrap_or_default()
    }

    /// Boots a device the way an agent would, with the fake.
    pub(crate) fn boot(st: &AppState, udid: &str) {
        assert!(std::process::Command::new(&st.cfg.xcrun).args(["simctl", "boot", udid]).status().unwrap().success());
    }

    /// Calls the agent's `simulator_device` tool through Loft's MCP endpoint, as its claude would.
    pub(crate) async fn ask_for_device(st: &AppState, agent_id: &str) -> Value {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "simulator_device", "arguments": {} } });
        let res = crate::mcp::handle(State(st.clone()), axum::extract::Path(agent_id.to_string()), Json(req)).await;
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<Value>(&body).unwrap()["result"].clone()
    }

    async fn listed(st: &AppState) -> Value {
        let body = axum::body::to_bytes(booted(State(st.clone())).await.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    /// A device in the fake as if made outside Loft: its UDID.
    fn device(st: &AppState, name: &str, state: &str, bytes: u64) -> String {
        let udid = uuid::Uuid::new_v4().to_string().to_uppercase();
        let dir = mkdir(st.cfg.xcrun.parent().unwrap().join("sim"));
        std::fs::write(dir.join(&udid), format!("{state}\n{name}\ncom.apple.CoreSimulator.SimDeviceType.iPhone-17\n{bytes}\n")).unwrap();
        udid
    }

    fn state_of(st: &AppState, udid: &str) -> Option<String> {
        fake_devices(st).into_iter().find(|d| d.0 == udid).map(|d| d.1)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn orphaned_agent_devices_are_deleted_and_no_other_device_is_touched() {
        let mut st = crate::tasks::tests::state();
        fake_xcrun(&mut st);
        let a = crate::tasks::tests::agent(&st, "Loft Dev");
        st.rt.lock().unwrap().running.insert(a.id.clone(), Default::default());
        let kept = device(&st, "Loft Dev (Loft)", "Booted", 3_000_000_000);
        st.db.set_agent_device(&a.id, &kept).unwrap();
        let old = device(&st, "Loft Dev 93543007", "Booted", 2_000_000_000);
        let deleted = crate::tasks::tests::agent(&st, "Deleted Agent");
        let gone = device(&st, "Deleted Agent (Loft)", "Shutdown", 1_000_000_000);
        st.db.set_agent_device(&deleted.id, &gone).unwrap();
        st.db.delete_agent(&deleted.id).unwrap();
        let users = [
            // Another Loft's on this Mac (a demo's): not this one's to delete.
            device(&st, "Loft Dev (Loft)", "Shutdown", 5),
            device(&st, "iPhone 17", "Booted", 7),
            device(&st, "QA clone", "Shutdown", 7),
            device(&st, "Someone 93543007", "Shutdown", 7),
            device(&st, "Loft Dev 9354300", "Shutdown", 7),
            device(&st, "Loft Dev 9354300z", "Shutdown", 7),
        ];

        let lines = tidy(&st).await;
        let mut done: Vec<(&str, &str, &str, u64)> = lines.iter().map(|l| (l.rel.as_str(), l.done.as_str(), l.command.as_str(), l.bytes)).collect();
        done.sort();
        let cmd_old = format!("xcrun simctl shutdown {old} && xcrun simctl delete {old}");
        let cmd_gone = format!("xcrun simctl delete {gone}");
        assert_eq!(done, [
            ("Deleted Agent (Loft)", "Deleted Simulator device", cmd_gone.as_str(), 1_000_000_000),
            ("Loft Dev 93543007", "Deleted Simulator device", cmd_old.as_str(), 2_000_000_000),
        ]);
        assert!(lines.iter().all(|l| l.error.is_none() && l.path.ends_with(l.command.rsplit(' ').next().unwrap())), "{lines:?}");
        let left: Vec<String> = fake_devices(&st).into_iter().map(|d| d.0).collect();
        for u in users.iter().chain([&kept]) {
            assert!(left.contains(u), "{u} stays");
        }
        assert_eq!(left.len(), users.len() + 1);
        assert_eq!(state_of(&st, &kept).as_deref(), Some("Booted"), "a running agent's device stays up");
        assert_eq!(state_of(&st, &users[1]).as_deref(), Some("Booted"), "the user's own device stays up");
        assert_eq!(lines.iter().find(|l| l.rel == "Loft Dev 93543007").unwrap().agent, a.id, "under the agent it was named for");
        assert!(tidy(&st).await.is_empty(), "nothing left to do");
        assert_eq!(st.db.agent_devices().unwrap(), [(kept, a.id)], "a deleted device is forgotten");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_idle_agent_s_device_is_shut_down_after_the_timeout_never_during_a_run() {
        let mut st = crate::tasks::tests::state();
        fake_xcrun(&mut st);
        let a = crate::tasks::tests::agent(&st, "Builder");
        let udid = device(&st, "Builder (Loft)", "Booted", 9);
        st.db.set_agent_device(&a.id, &udid).unwrap();
        let ago = |h: i64| Some((Utc::now() - chrono::Duration::hours(h)).to_rfc3339());
        use chrono::Utc;

        // Started 3 hours ago: under the 4-hour default.
        st.db.update_agent(&a.id, |x| x.last_run_at = ago(3)).unwrap();
        assert!(tidy(&st).await.is_empty());
        // Started 5 hours ago, but still running.
        st.db.update_agent(&a.id, |x| x.last_run_at = ago(5)).unwrap();
        st.rt.lock().unwrap().running.insert(a.id.clone(), Default::default());
        assert!(tidy(&st).await.is_empty(), "never during a run");
        // That run ended a minute ago.
        st.rt.lock().unwrap().running.remove(&a.id);
        st.db.update_agent(&a.id, |x| x.last_run_ended_at = ago(0)).unwrap();
        assert!(tidy(&st).await.is_empty(), "the timeout counts from the run's end");
        assert_eq!(state_of(&st, &udid).as_deref(), Some("Booted"));
        // The timeout is a setting.
        let mut s = st.settings.get();
        s.idle_timeout_hours = 1;
        st.db.update_agent(&a.id, |x| x.last_run_ended_at = ago(2)).unwrap();
        st.settings.save(s).unwrap();
        let lines = tidy(&st).await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!((lines[0].agent.as_str(), lines[0].done.as_str(), lines[0].bytes), (a.id.as_str(), "Shut down Simulator device", 0));
        assert_eq!(lines[0].command, format!("xcrun simctl shutdown {udid}"));
        assert_eq!(state_of(&st, &udid).as_deref(), Some("Shutdown"), "shut down, not deleted");
        assert!(tidy(&st).await.is_empty(), "a shut-down device is left alone");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_lists_agents_devices_with_their_size_and_state_and_the_tidy_is_logged() {
        let mut st = crate::tasks::tests::state();
        fake_xcrun(&mut st);
        let a = crate::tasks::tests::agent(&st, "Builder");
        let b = crate::tasks::tests::agent(&st, "Tester");
        let small = device(&st, "Builder (Loft)", "Booted", 100);
        let big = device(&st, "Tester (Loft)", "Shutdown", 200);
        st.db.set_agent_device(&a.id, &small).unwrap();
        st.db.set_agent_device(&b.id, &big).unwrap();
        device(&st, "iPhone 17", "Booted", 300);
        let Json(v) = agent_devices(State(st.clone())).await;
        assert_eq!(v, json!({ "devices": [
            { "udid": big, "name": "Tester (Loft)", "state": "Shutdown", "bytes": 200, "agent": { "id": b.id, "name": "Tester" } },
            { "udid": small, "name": "Builder (Loft)", "state": "Booted", "bytes": 100, "agent": { "id": a.id, "name": "Builder" } },
        ] }));

        // Both never ran: Builder's booted device is shut down, and the log says so.
        let lines = tidy(&st).await;
        crate::sweep::record(&st, "Simulator cleanup", lines).await;
        let axum::Json(log) = crate::sweep::log(State(st.clone())).await;
        assert_eq!((log.what.as_str(), log.lines.len(), log.lines[0].done.as_str(), log.lines[0].rel.as_str()), ("Simulator cleanup", 1, "Shut down Simulator device", "Builder (Loft)"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_gets_its_own_device_once_named_after_it_and_the_simulator_page_lists_agents_devices() {
        let mut st = crate::tasks::tests::state();
        fake_xcrun(&mut st);
        let a = crate::tasks::tests::agent(&st, "Builder");
        let b = crate::tasks::tests::agent(&st, "Tester");
        // The user's own device, used last: agents' devices copy its model.
        let mine = String::from_utf8(std::process::Command::new(&st.cfg.xcrun).args(["simctl", "create", "My iPhone", "com.apple.CoreSimulator.SimDeviceType.iPhone-17"]).output().unwrap().stdout).unwrap();
        let mine = mine.trim().to_string();
        boot(&st, &mine);

        let first = ask_for_device(&st, &a.id).await;
        let udid = st.db.agent_device(&a.id).unwrap().expect("the device is kept for the agent");
        assert!(first["content"][0]["text"].as_str().unwrap().contains(&format!("`Builder (Loft)`, UDID `{udid}`")), "{first}");
        assert_eq!(ask_for_device(&st, &a.id).await, first, "the same device every time");
        assert_eq!(simctl_log(&st).matches("simctl create Builder").count(), 1, "created once:\n{}", simctl_log(&st));
        let devices = fake_devices(&st);
        let made = devices.iter().find(|d| d.0 == udid).unwrap();
        assert_eq!((made.2.as_str(), made.3.as_str()), ("Builder (Loft)", "com.apple.CoreSimulator.SimDeviceType.iPhone-17"));

        ask_for_device(&st, &b.id).await;
        let b_udid = st.db.agent_device(&b.id).unwrap().unwrap();
        assert_ne!(b_udid, udid, "each agent has its own");

        // Only booted devices are listed: the agents' first, each with its agent, then the user's.
        boot(&st, &udid);
        let l = listed(&st).await;
        let rows: Vec<(String, Value)> = l["devices"].as_array().unwrap().iter().map(|d| (d["udid"].as_str().unwrap().to_string(), d["agent"].clone())).collect();
        assert_eq!(rows, [(udid.clone(), json!({ "id": a.id, "name": "Builder" })), (mine.clone(), Value::Null)]);
        boot(&st, &b_udid);
        let l = listed(&st).await;
        let names: Vec<&str> = l["devices"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Builder (Loft)", "Tester (Loft)", "My iPhone"]);

        // A renamed agent's device follows its name; one deleted outside Loft is replaced.
        st.db.update_agent(&a.id, |x| x.name = "Shipper".into()).unwrap();
        ask_for_device(&st, &a.id).await;
        assert!(fake_devices(&st).iter().any(|d| d.0 == udid && d.2 == "Shipper (Loft)"));
        std::fs::remove_file(st.cfg.xcrun.parent().unwrap().join("sim").join(&b_udid)).unwrap();
        let again = ask_for_device(&st, &b.id).await;
        let replaced = st.db.agent_device(&b.id).unwrap().unwrap();
        assert_ne!(replaced, b_udid);
        assert!(again["content"][0]["text"].as_str().unwrap().contains(&replaced));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_no_device_to_copy_the_newest_runtime_s_iphone_is_used_and_without_simctl_the_agent_is_told() {
        let mut st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Builder");
        let none = ask_for_device(&st, &a.id).await;
        assert_eq!(none["isError"], true);
        assert!(none["content"][0]["text"].as_str().unwrap().contains("Couldn't run `xcrun simctl`"), "{none}");
        assert_eq!(st.db.agent_device(&a.id).unwrap(), None);

        fake_xcrun(&mut st);
        ask_for_device(&st, &a.id).await;
        assert_eq!(fake_devices(&st)[0].3, "com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro");
        assert!(simctl_log(&st).contains("create Builder (Loft) com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro com.apple.CoreSimulator.SimRuntime.iOS-27-0"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_xcode_select_on_the_command_line_tools_simctl_runs_with_the_xcode_found_or_says_xcode_is_missing() {
        let mut st = crate::tasks::tests::state();
        fake_xcrun(&mut st);
        let root = fake_xcode_select(&mut st, "/Library/Developer/CommandLineTools", None);
        let a = crate::tasks::tests::agent(&st, "Builder");

        // No Xcode anywhere: the page and the tool say so, and xcrun isn't run.
        let l = listed(&st).await;
        assert_eq!(l["devices"], json!([]));
        assert!(l["error"].as_str().unwrap().starts_with("Xcode is missing"), "{l}");
        let none = ask_for_device(&st, &a.id).await;
        assert_eq!(none["isError"], true);
        assert!(none["content"][0]["text"].as_str().unwrap().starts_with("Xcode is missing"), "{none}");
        assert_eq!(simctl_log(&st), "");

        // An Xcode installed in /Applications: once the last lookup is stale, simctl runs with it as DEVELOPER_DIR, and the
        // agent is told to do the same.
        let beta = mkdir(root.join("Applications/Xcode-beta.app/Contents/Developer"));
        needs_developer_dir(&st, &beta);
        expire_xcode(&st);
        let got = ask_for_device(&st, &a.id).await;
        let text = got["content"][0]["text"].as_str().unwrap();
        let udid = st.db.agent_device(&a.id).unwrap().expect("the device was made");
        assert!(text.contains(&udid) && text.contains(&format!("DEVELOPER_DIR={}", beta.display())), "{got}");
        assert!(std::process::Command::new(&st.cfg.xcrun).args(["simctl", "boot", &udid]).env("DEVELOPER_DIR", &beta).status().unwrap().success());
        let l = listed(&st).await;
        assert_eq!(l["devices"][0]["udid"], udid.as_str(), "{l}");
        assert!(l["error"].is_null());

        // Xcode.app comes before other Xcodes, and /Applications before /Volumes.
        mkdir(root.join("Volumes/Ext/Xcode.app/Contents/Developer"));
        assert_eq!(find_xcode(&st.cfg.xcode).await, Xcode::At(beta.clone()));
        let main = mkdir(root.join("Applications/Xcode.app/Contents/Developer"));
        assert_eq!(find_xcode(&st.cfg.xcode).await, Xcode::At(main));
        std::fs::remove_dir_all(root.join("Applications")).unwrap();
        assert_eq!(find_xcode(&st.cfg.xcode).await, Xcode::At(root.join("Volumes/Ext/Xcode.app/Contents/Developer")));
        std::fs::remove_dir_all(root.join("Volumes/Ext/Xcode.app")).unwrap();
        let nested = mkdir(root.join("Volumes/Ext/xcode/Xcode.app/Contents/Developer"));
        assert_eq!(find_xcode(&st.cfg.xcode).await, Xcode::At(nested));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_developer_dir_already_set_or_an_xcode_already_selected_is_used_as_is() {
        let mut st = crate::tasks::tests::state();
        fake_xcrun(&mut st);
        let set = Some(PathBuf::from("/Volumes/Ext/Xcode.app/Contents/Developer"));
        let root = fake_xcode_select(&mut st, "/Library/Developer/CommandLineTools", set);
        mkdir(root.join("Applications/Xcode.app/Contents/Developer"));
        assert_eq!(find_xcode(&st.cfg.xcode).await, Xcode::Selected);
        let log = st.cfg.xcode.xcode_select.parent().unwrap().join("xcode-select.log");
        assert!(!log.exists(), "xcode-select isn't asked");

        fake_xcode_select(&mut st, "/Applications/Xcode.app/Contents/Developer", None);
        assert_eq!(find_xcode(&st.cfg.xcode).await, Xcode::Selected);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "-p\n");
        let a = crate::tasks::tests::agent(&st, "Builder");
        let got = ask_for_device(&st, &a.id).await;
        assert!(!got["content"][0]["text"].as_str().unwrap().contains("DEVELOPER_DIR"), "{got}");
        assert!(listed(&st).await["error"].is_null());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repeated_simctl_calls_look_xcode_up_once_until_the_lookup_is_stale() {
        let mut st = crate::tasks::tests::state();
        fake_xcrun(&mut st);
        let root = fake_xcode_select(&mut st, "/Library/Developer/CommandLineTools", None);
        let xc = mkdir(root.join("Applications/Xcode.app/Contents/Developer"));
        needs_developer_dir(&st, &xc);
        let a = crate::tasks::tests::agent(&st, "Builder");

        for _ in 0..3 {
            assert!(listed(&st).await["error"].is_null());
        }
        let got = ask_for_device(&st, &a.id).await;
        assert!(got["content"][0]["text"].as_str().unwrap().contains(&format!("DEVELOPER_DIR={}", xc.display())), "{got}");
        assert_eq!(xcode_lookups(&st), 1, "one lookup for every simctl call within the TTL");

        expire_xcode(&st);
        listed(&st).await;
        listed(&st).await;
        assert_eq!(xcode_lookups(&st), 2, "looked up again once stale");
    }
}
