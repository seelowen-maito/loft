//! This machine's Tailscale name, so agents can hand out dev-server links that work from the user's other devices.

use std::process::Command;
use std::sync::OnceLock;

/// The CLI may not be on a background service's PATH; the Mac app bundles its own copy.
const BINS: &[&str] = &["tailscale", "/usr/local/bin/tailscale", "/Applications/Tailscale.app/Contents/MacOS/Tailscale"];

/// MagicDNS name, e.g. `my-mac.tail1234.ts.net`. None when Tailscale isn't installed or running; retried until found.
pub fn host() -> Option<String> {
    static HOST: OnceLock<String> = OnceLock::new();
    if let Some(h) = HOST.get() {
        return Some(h.clone());
    }
    let h = BINS.iter().find_map(|bin| {
        let out = Command::new(bin).args(["status", "--json"]).output().ok().filter(|o| o.status.success())?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let name = v["Self"]["DNSName"].as_str()?.trim_end_matches('.');
        (!name.is_empty()).then(|| name.to_string())
    })?;
    Some(HOST.get_or_init(|| h).clone())
}

/// Loft's https address on the tailnet (`https://<name>`), when `tailscale serve` proxies to Loft's `port`, as last
/// checked (`check_https`). Browsers allow the microphone only over https.
static HTTPS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Asks Tailscale whether it serves Loft over https, and keeps the answer for `web_url`. When Tailscale doesn't
/// answer (tailscaled restarting), the last answer stays. Runs a process: call it off the async threads.
pub fn check_https(port: u16) -> Option<String> {
    let status = BINS.iter().find_map(|bin| {
        let out = Command::new(bin).args(["serve", "status", "--json"]).output().ok().filter(|o| o.status.success())?;
        serde_json::from_slice::<serde_json::Value>(&out.stdout).ok()
    });
    keep(&HTTPS, status, port)
}

/// Stores the https address read from `status` in `slot`; no status (Tailscale didn't answer) keeps what's there.
fn keep(slot: &std::sync::Mutex<Option<String>>, status: Option<serde_json::Value>, port: u16) -> Option<String> {
    let mut https = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(status) = status {
        *https = serve_url(&status, port);
    }
    https.clone()
}

/// The https address in `tailscale serve status --json` whose root proxies to Loft's `port` on this Mac.
fn serve_url(status: &serde_json::Value, port: u16) -> Option<String> {
    let targets = [format!("http://127.0.0.1:{port}"), format!("http://localhost:{port}"), format!("localhost:{port}"), port.to_string()];
    status["Web"].as_object()?.iter().find_map(|(host_port, web)| {
        let proxy = web["Handlers"]["/"]["Proxy"].as_str()?.trim_end_matches('/');
        targets.iter().any(|t| t == proxy).then(|| format!("https://{}", host_port.strip_suffix(":443").unwrap_or(host_port)))
    })
}

/// Where another device opens Loft: its https address when Tailscale serves it, else `http://<tailnet name>:<port>`.
/// The https address is as last checked, at startup and each time Settings opens: if `tailscale serve` is turned off
/// in between, links keep the https address until the next check.
pub fn web_url(port: u16) -> Option<String> {
    HTTPS.lock().unwrap_or_else(|e| e.into_inner()).clone().or_else(|| host().map(|h| format!("http://{h}:{port}")))
}

/// `GET /api/system/https`: checks Tailscale now. `{ url }` is Loft's https address, or null with the `command` that
/// turns it on.
pub async fn https(axum::extract::State(state): axum::extract::State<crate::state::AppState>) -> axum::Json<serde_json::Value> {
    let port = state.cfg.port;
    let url = tokio::task::spawn_blocking(move || check_https(port)).await.ok().flatten();
    axum::Json(serde_json::json!({ "url": url, "host": host(), "command": format!("tailscale serve --bg {port}") }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_https_address_that_serves_loft() {
        let web = |host: &str, proxy: &str| json!({ "Web": { host: { "Handlers": { "/": { "Proxy": proxy } } } } });
        assert_eq!(serve_url(&web("mac.tail1.ts.net:443", "http://127.0.0.1:7420"), 7420).as_deref(), Some("https://mac.tail1.ts.net"));
        assert_eq!(serve_url(&web("mac.tail1.ts.net:8443", "http://localhost:7420/"), 7420).as_deref(), Some("https://mac.tail1.ts.net:8443"));
        // Another app served there, or nothing served.
        assert_eq!(serve_url(&web("mac.tail1.ts.net:443", "http://127.0.0.1:3000"), 7420), None);
        assert_eq!(serve_url(&json!({}), 7420), None);
    }

    #[test]
    fn keeps_the_https_address_when_tailscale_does_not_answer() {
        let slot = std::sync::Mutex::new(None);
        let serving = json!({ "Web": { "mac.tail1.ts.net:443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:7420" } } } } });
        assert_eq!(keep(&slot, Some(serving), 7420).as_deref(), Some("https://mac.tail1.ts.net"));
        // tailscaled restarting: no answer, so the address stays.
        assert_eq!(keep(&slot, None, 7420).as_deref(), Some("https://mac.tail1.ts.net"));
        // Tailscale answers and no longer serves Loft.
        assert_eq!(keep(&slot, Some(json!({})), 7420), None);
    }
}
