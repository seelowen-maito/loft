//! Claude plan usage, the numbers behind Claude Code's `/usage`: how much of the current
//! 5-hour session and of the weekly limits is used. Read with the same login agents run on.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
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

/// Claude Code's stored login: the macOS keychain, else `.credentials.json` in its config dir.
async fn credentials() -> Result<Value> {
    let out = tokio::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .await;
    let raw = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => {
            let dir = std::env::var("CLAUDE_CONFIG_DIR")
                .map(Into::into)
                .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".claude"));
            std::fs::read_to_string(dir.join(".credentials.json")).context("no Claude Code login found; run `claude` and log in")?
        }
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

/// Usage for the navbar. Failures come back as `error` (and are cached too) rather than retried each poll.
pub async fn get(state: &AppState) -> Value {
    let mut cache = state.usage.0.lock().await;
    if let Some((at, v)) = &*cache {
        if at.elapsed() < TTL {
            return v.clone();
        }
    }
    let v = fetch(state).await.unwrap_or_else(|e| json!({ "windows": [], "error": e.to_string() }));
    *cache = Some((Instant::now(), v.clone()));
    v
}
