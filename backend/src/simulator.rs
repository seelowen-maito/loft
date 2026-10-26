//! The iOS Simulator running on this Mac, viewable and controllable from other devices.
//! `simctl` can only take screenshots; live video and touch input go through AXe (github.com/cameroncooke/AXe).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

use axum::Json;
use axum::body::Body;
use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

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

async fn simctl_json(args: &[&str]) -> Value {
    match Command::new("xcrun").arg("simctl").args(args).arg("-j").output().await {
        Ok(o) if o.status.success() => serde_json::from_slice(&o.stdout).unwrap_or_default(),
        _ => Value::Null,
    }
}

/// Booted simulators and whether they can be controlled: `{ interactive, devices: [{ udid, name, runtime }] }`.
pub async fn booted() -> Response {
    let v = simctl_json(&["list", "devices", "booted"]).await;
    let mut list = Vec::new();
    if let Some(runtimes) = v["devices"].as_object() {
        for (runtime, devices) in runtimes {
            // com.apple.CoreSimulator.SimRuntime.iOS-27-0 -> iOS 27.0
            let runtime = runtime.rsplit('.').next().unwrap_or(runtime).replacen('-', " ", 1).replace('-', ".");
            for d in devices.as_array().into_iter().flatten() {
                list.push(json!({ "udid": d["udid"], "name": d["name"], "runtime": runtime }));
            }
        }
    }
    Json(json!({ "interactive": axe().is_some(), "devices": list })).into_response()
}

#[derive(Deserialize)]
pub struct Which {
    /// Device UDID; defaults to the first booted one.
    pub udid: Option<String>,
}

/// A JPEG of the simulator's screen. The fallback view when AXe isn't installed.
pub async fn screenshot(Query(q): Query<Which>) -> Response {
    let device = q.udid.filter(|u| valid_udid(u)).unwrap_or_else(|| "booted".into());
    let path = std::env::temp_dir().join(format!("loft-sim-{}.jpg", uuid::Uuid::new_v4()));
    let out = Command::new("xcrun").args(["simctl", "io", &device, "screenshot", "--type=jpeg"]).arg(&path).output().await;
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
        (Err(e), _) => err(StatusCode::NOT_FOUND, format!("Couldn't run xcrun: {e}")),
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
async fn screen_points(udid: &str) -> Option<(f64, f64)> {
    static CACHE: OnceLock<Mutex<HashMap<String, (f64, f64)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(s) = cache.lock().unwrap().get(udid) {
        return Some(*s);
    }
    let devices = simctl_json(&["list", "devices"]).await;
    let type_id = devices["devices"].as_object()?.values().flat_map(|d| d.as_array().into_iter().flatten()).find(|d| d["udid"] == udid)?
        ["deviceTypeIdentifier"]
        .as_str()?
        .to_string();
    let types = simctl_json(&["list", "devicetypes"]).await;
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
pub async fn input(Json(req): Json<InputReq>) -> Response {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    if !valid_udid(&req.udid) {
        return err(StatusCode::BAD_REQUEST, "invalid udid");
    }
    let Some(bin) = axe() else { return err(StatusCode::NOT_FOUND, "AXe isn't installed") };
    let pt = |fx: f64, fy: f64, (w, h): (f64, f64)| (format!("{:.1}", fx.clamp(0.0, 1.0) * w), format!("{:.1}", fy.clamp(0.0, 1.0) * h));
    let size = match req.input {
        Input::Tap { .. } | Input::Swipe { .. } => match screen_points(&req.udid).await {
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
