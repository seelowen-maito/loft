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
    /// Publish new Loft issues to GitHub as they're made (only projects with a GitHub repo can). Off by default.
    /// `None` until `migrate_github_settings` has turned the old per-project `auto_publish` list into this.
    pub auto_publish_all: Option<bool>,
    /// Old per-project list, no longer read. Kept so clients that still send it keep working.
    pub auto_publish: Vec<String>,
    /// Old per-project modes: folded into `comment_approval_default` by the migration, then empty and no longer read.
    pub comment_approval: std::collections::HashMap<String, String>,
    /// When comments on mirrored issues go to GitHub, for every project: `ask` (every one waits for approval),
    /// `own` (yours go straight, agents' wait) or `all` (everything goes straight). Was the per-project fallback.
    pub comment_approval_default: String,
    pub push: Push,
}

impl Settings {
    /// The comment approval mode; anything unknown asks.
    pub fn approval_mode(&self) -> &str {
        let m = &self.comment_approval_default;
        if matches!(m.as_str(), "own" | "all") { m } else { "ask" }
    }
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
            auto_publish_all: None,
            auto_publish: vec![],
            comment_approval: Default::default(),
            comment_approval_default: "ask".into(),
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

    /// Turns the old per-project settings into the global ones, once (while `auto_publish_all` is unset).
    /// Auto-publish is on only if it was on for every project in `with_repo` (those with a GitHub repo).
    /// The comment mode becomes the strictest of the old default and every per-project override
    /// (ask < own < all), and the overrides are cleared. So nothing goes public that didn't before.
    pub fn migrate_github_settings(&self, with_repo: &[String]) -> Result<()> {
        let mut s = self.get();
        if s.auto_publish_all.is_some() {
            return Ok(());
        }
        s.auto_publish_all = Some(!with_repo.is_empty() && with_repo.iter().all(|p| s.auto_publish.contains(p)));
        let rank = |m: &str| ["ask", "own", "all"].iter().position(|x| *x == m).unwrap_or(0);
        let strictest = s.comment_approval.values().chain([&s.comment_approval_default]).min_by_key(|m| rank(m)).unwrap();
        s.comment_approval_default = ["ask", "own", "all"][rank(strictest)].into();
        s.comment_approval.clear();
        self.save(s)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn store(json: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("loft-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        let json = json.replace("PROJECTS", &dir.join("projects").to_string_lossy());
        std::fs::write(dir.join("settings.json"), json).unwrap();
        Store::load(&dir).unwrap()
    }

    fn migrated(old: &str, with_repo: &[&str]) -> Option<bool> {
        let s = store(&format!(r#"{{ "projects_dir": "PROJECTS", {old} }}"#));
        s.migrate_github_settings(&with_repo.iter().map(|p| p.to_string()).collect::<Vec<_>>()).unwrap();
        s.get().auto_publish_all
    }

    #[test]
    fn auto_publish_migrates_on_only_if_it_was_on_for_every_project_with_a_repo() {
        let all = r#""auto_publish": ["a", "b", "no-repo"]"#;
        assert_eq!(migrated(all, &["a", "b"]), Some(true));
        assert_eq!(migrated(r#""auto_publish": ["a"]"#, &["a", "b"]), Some(false), "off for one: stays off");
        assert_eq!(migrated(r#""auto_publish": ["no-repo"]"#, &["a"]), Some(false), "no-repo projects don't count");
        assert_eq!(migrated(all, &[]), Some(false), "nothing to publish to");
        assert_eq!(migrated(r#""x": 1"#, &["a"]), Some(false), "never set");
    }

    #[test]
    fn the_migration_runs_once_and_the_old_mode_becomes_the_global_one() {
        let s = store(r#"{ "projects_dir": "PROJECTS", "auto_publish": ["a"], "comment_approval_default": "own", "comment_approval": { "a": "all" } }"#);
        s.migrate_github_settings(&["a".into()]).unwrap();
        assert_eq!((s.get().auto_publish_all, s.get().approval_mode()), (Some(true), "own"));
        let mut now = s.get();
        now.auto_publish_all = Some(false);
        s.save(now).unwrap();
        s.migrate_github_settings(&["a".into()]).unwrap();
        assert_eq!(s.get().auto_publish_all, Some(false), "a later choice isn't overwritten");
    }

    #[test]
    fn the_comment_mode_becomes_the_strictest_of_the_old_default_and_every_override() {
        let mode = |default: &str, overrides: &str| {
            let s = store(&format!(r#"{{ "projects_dir": "PROJECTS", "comment_approval_default": "{default}", "comment_approval": {{ {overrides} }} }}"#));
            s.migrate_github_settings(&[]).unwrap();
            assert!(s.get().comment_approval.is_empty(), "the old overrides are cleared");
            s.get().approval_mode().to_string()
        };
        assert_eq!(mode("all", r#""sensitive": "ask""#), "ask", "an ask override can't be widened by an all default");
        assert_eq!(mode("own", r#""sensitive": "ask""#), "ask");
        assert_eq!(mode("all", r#""a": "own", "b": "all""#), "own");
        assert_eq!(mode("ask", r#""a": "all""#), "ask", "an all override doesn't widen an ask default");
        assert_eq!(mode("own", ""), "own", "no overrides: unchanged");
        assert_eq!(mode("all", ""), "all");
        assert_eq!(mode("ask", ""), "ask");
        assert_eq!(mode("all", r#""odd": "sometimes""#), "ask", "an unknown value asks");
    }
}
