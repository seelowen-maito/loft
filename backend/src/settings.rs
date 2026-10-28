//! User settings, edited on the Settings page and stored in `.loft/settings.json`.
//! Most apply immediately; `RESTART_FIELDS` take effect on the next start.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Folder holding the user's git repos.
    pub projects_dir: PathBuf,
    /// Claude Code binary the agents run.
    pub claude_bin: String,
    /// SSH host other machines use for this one; `/code` through a proxy then opens VS Code Remote-SSH. Empty = off.
    pub ssh_host: String,
    /// Explicit GitHub token; empty falls back to `gh auth token`.
    pub github_token: String,
    /// Pull, rebuild and restart when the upstream branch moves (see updater.rs).
    pub auto_update: bool,
    /// Show the Simulator tab in the web navbar. Off by default: only useful for iOS work on this Mac.
    pub show_simulator: bool,
    /// Projects whose new Loft issues are published to GitHub as they're made. Off (empty) by default.
    pub auto_publish: Vec<String>,
    pub push: Push,
}

/// APNs push. Not secret: the key itself is a file only the user can read (see apns.rs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Push {
    /// The auth key's id (10 characters, shown next to the key in the Apple Developer portal).
    pub key_id: String,
    /// The Apple Developer team id.
    pub team_id: String,
    /// The app's bundle id, used as the APNs topic.
    pub bundle_id: String,
}

impl Default for Push {
    /// The team and bundle id start from LOFT_TEAM_ID and LOFT_BUNDLE_ID, as for the iOS build.
    fn default() -> Self {
        let env = |k: &str| std::env::var(k).map(|v| v.trim().to_string()).unwrap_or_default();
        Self { key_id: String::new(), team_id: env("LOFT_TEAM_ID"), bundle_id: env("LOFT_BUNDLE_ID") }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            projects_dir: dirs::home_dir().unwrap_or_default().join("projects"),
            claude_bin: "claude".into(),
            ssh_host: String::new(),
            github_token: String::new(),
            auto_update: false,
            show_simulator: false,
            auto_publish: vec![],
            push: Push::default(),
        }
    }
}

/// Settings read once at startup; changing them asks for a restart.
pub const RESTART_FIELDS: &[&str] = &["projects_dir"];

pub struct Store {
    path: PathBuf,
    current: RwLock<Settings>,
    /// What this process started with, to tell which saved changes still need a restart.
    boot: Settings,
}

impl Store {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("settings.json");
        let s: Settings = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, current: RwLock::new(s.clone()), boot: s })
    }

    pub fn get(&self) -> Settings {
        self.current.read().unwrap().clone()
    }

    /// Values in effect for restart-only fields.
    pub fn boot(&self) -> &Settings {
        &self.boot
    }

    pub fn save(&self, s: Settings) -> Result<()> {
        if !s.projects_dir.is_dir() {
            bail!("Projects folder {} doesn't exist", s.projects_dir.display());
        }
        if s.claude_bin.trim().is_empty() {
            bail!("Claude command can't be empty");
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&s)?)?;
        std::fs::rename(&tmp, &self.path)?;
        *self.current.write().unwrap() = s;
        Ok(())
    }

    /// Restart-only fields whose saved value differs from the running one.
    pub fn pending_restart(&self) -> Vec<&'static str> {
        let s = self.current.read().unwrap();
        let mut pending = Vec::new();
        if s.projects_dir != self.boot.projects_dir {
            pending.push("projects_dir");
        }
        pending
    }
}
