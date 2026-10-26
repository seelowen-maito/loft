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
