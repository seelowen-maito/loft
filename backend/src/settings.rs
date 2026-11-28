//! User settings, edited on the Settings page and stored in `.loft/settings.json`.
//! Most apply immediately; `RESTART_FIELDS` take effect on the next start.

use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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
    /// Explicit GitHub token; empty falls back to `gh auth token`. Read from `settings.json` but never
    /// serialized, so no view or log can carry it: only `Store::save` writes it, through `OnDisk`.
    #[serde(skip_serializing)]
    pub github_token: String,
    /// Pull, rebuild and restart when the upstream branch moves (see updater.rs).
    pub auto_update: bool,
    /// Show the Simulator tab in the web navbar. Off by default: only useful for iOS work on this Mac.
    pub show_simulator: bool,
    /// Give every agent Loft's messaging rules (see `agents::MESSAGING_RULES`). On by default.
    pub messaging_rules: bool,
    /// GitHub publishing as Settings kept it before it moved to each project's Project settings (#467): read from
    /// `settings.json` and from older clients, but never written back, so it goes once
    /// `project_settings::migrate_github_settings` has handed it to the projects. The global switch for new Loft
    /// issues (`None`: never set, the older per-project `auto_publish` list then says).
    #[serde(skip_serializing)]
    pub auto_publish_all: Option<bool>,
    /// The per-project list from before the global switch.
    #[serde(skip_serializing)]
    pub auto_publish: Vec<String>,
    /// The per-project comment approval modes from before the global one.
    #[serde(skip_serializing)]
    pub comment_approval: std::collections::HashMap<String, String>,
    /// The comment approval mode for every project: `ask`, `own` or `all`; empty when never set.
    #[serde(skip_serializing)]
    pub comment_approval_default: String,
    pub push: Push,
    /// Where the daily backups go (see backup.rs). Empty: `~/Library/Application Support/Loft/backups`.
    pub backup_dir: String,
    /// Hours with no run after which an agent counts as idle: Loft then shuts its Simulator device down.
    pub idle_timeout_hours: u32,
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
    /// The push relay's address (https), where its portal lives too. Empty: LOFT_RELAY_URL, else the official relay. Only
    /// `PUT /api/push` sets it. A `portal_url` saved by older versions is ignored.
    pub relay_url: String,
}

impl Default for Push {
    /// The team and bundle id start from LOFT_TEAM_ID and LOFT_BUNDLE_ID, as for the iOS build.
    fn default() -> Self {
        let env = |k: &str| std::env::var(k).map(|v| v.trim().to_string()).unwrap_or_default();
        Self { key_id: String::new(), team_id: env("LOFT_TEAM_ID"), bundle_id: env("LOFT_BUNDLE_ID"), relay_url: String::new() }
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
            messaging_rules: true,
            auto_publish: vec![],
            comment_approval: Default::default(),
            comment_approval_default: String::new(),
            push: Push::default(),
            backup_dir: String::new(),
            idle_timeout_hours: 4,
        }
    }
}

/// What `settings.json` holds: the settings plus the token they leave out when serialized.
#[derive(Serialize)]
struct OnDisk<'a> {
    #[serde(flatten)]
    settings: &'a Settings,
    github_token: &'a str,
}

/// Whether `bin` names a file to run: a full path to one, or a bare name found in an absolute PATH folder.
pub fn runnable(bin: &str) -> bool {
    let p = Path::new(bin);
    if p.is_absolute() {
        return p.is_file();
    }
    !bin.is_empty()
        && !bin.contains('/')
        && std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.is_absolute() && d.join(bin).is_file()))
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
        owner_only(&path)?;
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

    /// Loft's data folder, which holds `settings.json` and the database.
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
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
        let backups = Path::new(s.backup_dir.trim());
        if !s.backup_dir.trim().is_empty() && (!backups.is_absolute() || backups.components().any(|c| c == std::path::Component::ParentDir)) {
            bail!("Backups folder must be a full path, like /Volumes/Disk/Loft backups");
        }
        if s.idle_timeout_hours == 0 {
            bail!("Idle timeout must be at least 1 hour");
        }
        let tmp = self.path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&tmp);
        // Owner-only from the first byte: it holds the GitHub token.
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        std::io::Write::write_all(&mut f, serde_json::to_string_pretty(&OnDisk { settings: &s, github_token: &s.github_token })?.as_bytes())?;
        drop(f);
        std::fs::rename(&tmp, &self.path)?;
        owner_only(&self.path)?;
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

/// `.loft` 0700 and `settings.json` (when there is one) 0600: it holds the GitHub token. At start, for
/// files made before, and on every save.
fn owner_only(settings: &Path) -> Result<()> {
    if let Some(dir) = settings.parent().filter(|d| d.is_dir()) {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    if settings.exists() {
        std::fs::set_permissions(settings, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(json: &str) -> Store {
        let dir = crate::testdir::for_this_test("settings");
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        let json = json.replace("PROJECTS", &dir.join("projects").to_string_lossy());
        std::fs::write(dir.join("settings.json"), json).unwrap();
        Store::load(&dir).unwrap()
    }

    #[test]
    fn settings_and_their_folder_are_owner_only_at_start_and_on_save() {
        use std::os::unix::fs::MetadataExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().mode() & 0o777;
        let dir = crate::testdir::for_this_test("settings");
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, format!(r#"{{ "projects_dir": {:?} }}"#, dir.join("projects"))).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let s = Store::load(&dir).unwrap();
        assert_eq!((mode(&dir), mode(&file)), (0o700, 0o600), "made before: tightened at start");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        s.save(s.get()).unwrap();
        assert_eq!(mode(&file), 0o600, "on save");
        // A first start, with no settings yet, and the first save.
        let fresh = crate::testdir::for_this_test("settings");
        std::fs::create_dir_all(&fresh).unwrap();
        let s = Store::load(&fresh).unwrap();
        let mut first = s.get();
        first.projects_dir = dir.join("projects");
        s.save(first).unwrap();
        assert_eq!((mode(&fresh), mode(&fresh.join("settings.json"))), (0o700, 0o600));
    }

    #[test]
    fn the_github_token_is_written_to_settings_json_and_never_serialized_otherwise() {
        let s = store(r#"{ "projects_dir": "PROJECTS", "github_token": "ghp_saved" }"#);
        assert_eq!(s.get().github_token, "ghp_saved", "read back from disk");
        assert!(!serde_json::to_string(&s.get()).unwrap().contains("ghp_saved"));
        assert!(serde_json::to_value(s.get()).unwrap().get("github_token").is_none());
        let mut next = s.get();
        next.github_token = "ghp_new".into();
        s.save(next).unwrap();
        let on_disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&s.path).unwrap()).unwrap();
        assert_eq!(on_disk["github_token"], "ghp_new");
        assert_eq!(Store::load(s.dir()).unwrap().get().github_token, "ghp_new", "survives a restart");
    }

    #[test]
    fn a_runnable_claude_command_is_a_full_path_to_a_file_or_a_name_on_path() {
        let dir = crate::testdir::for_this_test("settings");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(runnable("/bin/sh"));
        assert!(runnable("sh"), "on PATH");
        assert!(!runnable("/nonexistent/claude"));
        assert!(!runnable(&dir.to_string_lossy()), "a folder");
        assert!(!runnable("bin/sh"), "relative");
        assert!(!runnable("./sh"));
        assert!(!runnable("no-such-command-anywhere"));
        assert!(!runnable(""));
    }

    #[test]
    fn the_idle_timeout_defaults_to_4_hours_and_is_at_least_1() {
        let s = store(r#"{ "projects_dir": "PROJECTS" }"#);
        assert_eq!(s.get().idle_timeout_hours, 4, "settings saved before it");
        let mut zero = s.get();
        zero.idle_timeout_hours = 0;
        assert!(s.save(zero).is_err());
    }

    #[test]
    fn the_github_publishing_settings_had_is_read_but_never_written_back() {
        let s = store(r#"{ "projects_dir": "PROJECTS", "auto_publish_all": true, "auto_publish": ["a"], "comment_approval": { "a": "all" }, "comment_approval_default": "own" }"#);
        let got = s.get();
        assert_eq!((got.auto_publish_all, got.comment_approval_default.as_str(), got.comment_approval.len()), (Some(true), "own", 1), "read for the migration");
        s.save(got).unwrap();
        let file = std::fs::read_to_string(&s.path).unwrap();
        for key in ["auto_publish", "comment_approval"] {
            assert!(!file.contains(key), "{key} isn't written back: {file}");
        }
        let fresh = s.get();
        assert_eq!(fresh.auto_publish_all, Some(true), "the running settings keep it until the migration clears it");
    }
}

