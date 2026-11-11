//! Herdr UI: Loft's agents as agents in the Herdr terminal multiplexer.
//!
//! The Herdr side is a plugin in `herdr/` at the repo root: a small terminal client (`loft-herdr`)
//! that shows one agent's chat per Herdr tab and talks to this API like the web app does.
//!
//! - Set up on this machine (`POST /api/herdr/setup`): builds the client, points it at this Loft,
//!   links the plugin into the local Herdr and opens the agents' tabs.
//! - Set up on another machine: Settings shows a one-line command that runs `install.sh` there. It
//!   downloads the plugin files and, when that machine has the same OS and CPU, the built client.

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
    let link = home().join(".local/bin/loft-herdr");
    if std::fs::symlink_metadata(&link).map(|m| m.file_type().is_symlink()).unwrap_or(true) {
        let _ = std::fs::create_dir_all(link.parent().unwrap());
        let _ = std::fs::remove_file(&link);
        if std::os::unix::fs::symlink(&bin, &link).is_ok() {
            done.push(format!("Linked {}", link.display()));
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
