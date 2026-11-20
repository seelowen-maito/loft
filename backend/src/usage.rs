//! Claude plan usage, the numbers behind Claude Code's `/usage`: how much of the current
//! 5-hour session and of the weekly limits is used. Read with the same login agents run on.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::state::AppState;

/// Last answer, shared by every open tab so the endpoint isn't hit more than once per TTL.
#[derive(Default)]
pub struct Cache(Mutex<Option<(Instant, Value)>>);

const TTL: Duration = Duration::from_secs(120);

/// Limits in display order: (key in the API response, short label).
const WINDOWS: &[(&str, &str)] = &[
    ("five_hour", "session"),
    ("seven_day", "week"),
    ("seven_day_opus", "week · Opus"),
    ("seven_day_sonnet", "week · Sonnet"),
];

/// The keychain item the CLI keeps its login in: "Claude Code-credentials", with the first 8 hex digits of the
/// SHA-256 of the config folder added after a dash when `CLAUDE_CONFIG_DIR` is set (read from the CLI's code, v2.1.323).
pub fn keychain_service(config_dir: Option<&std::path::Path>) -> String {
    use sha2::{Digest, Sha256};
    match config_dir {
        None => "Claude Code-credentials".into(),
        Some(d) => {
            let hash: String = Sha256::digest(d.to_string_lossy().as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
            format!("Claude Code-credentials-{}", &hash[..8])
        }
    }
}

/// Claude Code's stored login, where the CLI keeps it: the macOS keychain (an item named for `CLAUDE_CONFIG_DIR`
/// when that is set), else `.credentials.json` in the config folder.
pub async fn credentials() -> Result<Value> {
    let dir = crate::sessions::config_dir();
    let out = tokio::process::Command::new("security").args(["find-generic-password", "-s", &keychain_service(dir.as_deref()), "-w"]).output().await;
    let raw = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => std::fs::read_to_string(crate::sessions::claude_dir().join(".credentials.json")).context("no Claude Code login found; run `claude` and log in")?,
    };
    serde_json::from_str(&raw).context("unreadable Claude Code credentials")
}

async fn fetch(state: &AppState) -> Result<Value> {
    let creds = credentials().await?;
    let oauth = &creds["claudeAiOauth"];
    let token = oauth["accessToken"].as_str().ok_or_else(|| anyhow!("Claude Code isn't logged in with a Claude plan"))?;
    if oauth["expiresAt"].as_i64().is_some_and(|ms| ms <= chrono::Utc::now().timestamp_millis()) {
        bail!("Claude login expired; it refreshes the next time Claude Code runs");
    }
    let resp = state
        .http
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "loft")
        .send()
        .await?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("usage {}: {}", status.as_u16(), body["error"]["message"].as_str().unwrap_or("request failed"));
    }
    let windows: Vec<Value> = WINDOWS
        .iter()
        .filter_map(|(key, label)| {
            let w = &body[*key];
            let percent = w["utilization"].as_f64()?;
            Some(json!({ "key": key, "label": label, "percent": percent, "resets_at": w["resets_at"] }))
        })
        .collect();
    Ok(json!({ "windows": windows, "error": null }))
}

/// The current Usage week, as (start, reset): the weekly limit's `resets_at` minus 7 days, until
/// that reset. None when the plan usage couldn't be read or has no weekly reset.
pub fn usage_week(usage: &Value) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let w = usage["windows"].as_array()?.iter().find(|w| w["key"] == "seven_day")?;
    let reset = DateTime::parse_from_rfc3339(w["resets_at"].as_str()?).ok()?.with_timezone(&Utc);
    Some((reset - chrono::Duration::days(7), reset))
}

/// Usage for the navbar. Failures come back as `error` (and are cached too) rather than retried each poll.
pub async fn get(state: &AppState) -> Value {
    let mut cache = state.usage.0.lock().await;
    if let Some((at, v)) = &*cache
        && at.elapsed() < TTL {
            return v.clone();
        }
    let v = fetch(state).await.unwrap_or_else(|e| json!({ "windows": [], "error": e.to_string() }));
    *cache = Some((Instant::now(), v.clone()));
    v
}

/// Make `get` answer `v`, as if just read from the API.
#[cfg(test)]
pub async fn fake(state: &AppState, v: Value) {
    *state.usage.0.lock().await = Some((Instant::now(), v));
}

#[cfg(test)]
mod keychain_tests {
    use super::keychain_service;

    #[test]
    fn the_keychain_item_is_named_for_the_config_folder_like_the_clis() {
        assert_eq!(keychain_service(None), "Claude Code-credentials");
        // sha256("x") starts 2d711642.
        assert_eq!(keychain_service(Some(std::path::Path::new("x"))), "Claude Code-credentials-2d711642");
    }
}
