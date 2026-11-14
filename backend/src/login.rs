//! Whether the `claude` CLI is logged in, read from the same stored login the usage numbers use.
//! Logging in needs a browser and a terminal, so Loft only shows the status and, on the Mac, opens Terminal to do it.
use axum::Json;
use serde_json::{json, Value};

use crate::api::{bad, ApiResult};

pub const STEPS: &str = "Log in from a terminal on the Mac: run `claude auth login` and finish in the browser, or press Open Terminal in Settings › Claude login.";

/// The status from the stored credentials (Err: none found) and the account's email, if the CLI's config has it.
pub fn status_from(creds: Result<Value, String>, email: Option<String>) -> Value {
    let oauth = match &creds {
        Ok(c) => &c["claudeAiOauth"],
        Err(_) => &Value::Null,
    };
    let Some(_) = oauth["accessToken"].as_str() else {
        let why = match creds {
            Err(e) => format!("{}.", e.trim_end_matches('.')),
            Ok(_) => "Claude Code isn't logged in with a Claude plan.".into(),
        };
        return json!({ "logged_in": false, "account": null, "plan": null, "note": why, "steps": STEPS });
    };
    // An expired token refreshes by itself on the next run, so it still counts as logged in.
    json!({ "logged_in": true, "account": email, "plan": oauth["subscriptionType"], "note": null, "steps": STEPS })
}

fn email() -> Option<String> {
    let dir = std::env::var("CLAUDE_CONFIG_DIR").ok().map(std::path::PathBuf::from);
    let path = match dir {
        Some(d) => d.join(".claude.json"),
        None => dirs::home_dir()?.join(".claude.json"),
    };
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    v["oauthAccount"]["emailAddress"].as_str().map(String::from)
}

pub async fn status() -> Value {
    status_from(crate::usage::credentials().await.map_err(|e| e.to_string()), email())
}

pub async fn get() -> Json<Value> {
    Json(status().await)
}

/// A run's error text, with the fix added when the CLI says it isn't logged in.
pub fn with_fix(msg: &str) -> String {
    let m = msg.to_lowercase();
    if m.contains("not logged in") || m.contains("please run /login") {
        format!("{msg}\nLoft can't log in for you. {STEPS}")
    } else {
        msg.to_string()
    }
}

/// The status as chat text, for `/login`.
pub async fn text() -> String {
    let s = status().await;
    if s["logged_in"] == true {
        let who = s["account"].as_str().map_or(String::new(), |a| format!(" as {a}"));
        let plan = s["plan"].as_str().map_or(String::new(), |p| format!(" ({p} plan)"));
        format!("Claude is logged in{who}{plan}. To switch accounts: {STEPS}")
    } else {
        format!("Claude is not logged in. {STEPS}")
    }
}

/// Opens Terminal on the Mac running `claude auth login`.
pub async fn open_terminal() -> ApiResult<Value> {
    if !cfg!(target_os = "macos") {
        return Err(bad("Open Terminal works on the Mac only. Run `claude auth login` in a terminal."));
    }
    let out = tokio::process::Command::new("osascript")
        .args(["-e", "tell application \"Terminal\" to do script \"claude auth login\"", "-e", "tell application \"Terminal\" to activate"])
        .output()
        .await
        .map_err(|e| bad(format!("Couldn't open Terminal: {e}")))?;
    if !out.status.success() {
        return Err(bad(format!("Couldn't open Terminal: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reads_a_fake_credential_store() {
        let ok = status_from(Ok(json!({ "claudeAiOauth": { "accessToken": "t", "subscriptionType": "max", "expiresAt": 1 } })), Some("a@b.c".into()));
        assert_eq!((ok["logged_in"].clone(), ok["account"].clone(), ok["plan"].clone()), (json!(true), json!("a@b.c"), json!("max")), "an expired token still refreshes by itself");
        let none = status_from(Err("no Claude Code login found".into()), None);
        assert_eq!((none["logged_in"].clone(), none["note"].clone()), (json!(false), json!("no Claude Code login found.")));
        assert!(none["steps"].as_str().unwrap().contains("claude auth login"));
        let api_key = status_from(Ok(json!({})), None);
        assert_eq!(api_key["logged_in"], false);
        assert!(with_fix("Not logged in · Please run /login").contains("claude auth login"));
        assert_eq!(with_fix("boom"), "boom");
    }
}
