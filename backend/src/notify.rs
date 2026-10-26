//! iMessage notifications via the Messages app (macOS). What gets sent, and to whom, is set on the Settings page.

use std::time::Duration;

use anyhow::{Result, bail};
use tokio::process::Command;

use crate::state::AppState;

/// The text is passed as an argument, never spliced into the script.
const SCRIPT: &str = r#"on run argv
    tell application "Messages" to send (item 1 of argv) to participant (item 2 of argv) of (1st account whose service type = iMessage)
end run"#;

#[derive(Clone, Copy)]
pub enum Notice {
    Permission,
    ScheduledRun,
    ManualRun,
    PrOpened,
    PrMerged,
    Update,
}

/// The recipient, when iMessage is on and this kind of notice is wanted.
fn recipient(state: &AppState, notice: Notice) -> Option<String> {
    let m = state.settings.get().imessage;
    let wanted = match notice {
        Notice::Permission => m.permission,
        Notice::ScheduledRun => m.scheduled_run,
        Notice::ManualRun => m.manual_run,
        Notice::PrOpened => m.pr_opened,
        Notice::PrMerged => m.pr_merged,
        Notice::Update => m.update,
    };
    (m.enabled && wanted && !m.to.trim().is_empty()).then(|| m.to.trim().to_string())
}

/// Fire and forget; failures are logged.
pub fn send(state: &AppState, notice: Notice, text: impl Into<String>) {
    let Some(to) = recipient(state, notice) else { return };
    let text = text.into();
    tokio::spawn(async move {
        if let Err(e) = deliver(&to, &text).await {
            tracing::error!("imessage: {e:#}");
        }
    });
}

/// Send and wait, for callers about to exit (e.g. a self-update restart).
pub async fn send_now(state: &AppState, notice: Notice, text: &str) {
    let Some(to) = recipient(state, notice) else { return };
    if let Err(e) = deliver(&to, text).await {
        tracing::error!("imessage: {e:#}");
    }
}

/// A reply in the iMessage conversation; sent whenever iMessage is on, regardless of the notice toggles.
pub fn send_reply(state: &AppState, text: impl Into<String>) {
    let m = state.settings.get().imessage;
    if !m.enabled || m.to.trim().is_empty() {
        return;
    }
    let (to, text) = (m.to.trim().to_string(), text.into());
    tokio::spawn(async move {
        if let Err(e) = deliver(&to, &text).await {
            tracing::error!("imessage: {e:#}");
        }
    });
}

pub async fn deliver(to: &str, text: &str) -> Result<()> {
    let mut cmd = Command::new("osascript");
    cmd.arg("-e").arg(SCRIPT).arg(text).arg(to).kill_on_drop(true);
    // The first send blocks on macOS's "allow loft to control Messages" prompt; don't hang on it.
    let out = match tokio::time::timeout(Duration::from_secs(30), cmd.output()).await {
        Ok(out) => out?,
        Err(_) => bail!("Timed out. Is macOS asking to let loft control Messages on this Mac's screen?"),
    };
    if !out.status.success() {
        bail!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// One line of at most `max` chars, for message previews.
pub fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    format!("{}…", flat.chars().take(max - 1).collect::<String>())
}
