//! Where Loft is. One small file that Loft's Settings (or the remote install script) writes:
//! `~/.config/loft-herdr/config.json`, `{ "url": "https://homebase.example.ts.net" }`.
//!
//! It lives outside Herdr's plugin config folder on purpose: after a Herdr restart a chat pane is
//! started again by its resume command, as a plain pane without the plugin's environment.

use std::path::PathBuf;

pub const DEFAULT_URL: &str = "http://localhost:7420";

pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("LOFT_HERDR_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    home().join(".config").join("loft-herdr")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// The Loft address: `LOFT_URL`, else the config file, else Loft on this machine.
pub fn url() -> String {
    let from_env = std::env::var("LOFT_URL").ok().filter(|u| !u.trim().is_empty());
    let from_file = || {
        let raw = std::fs::read_to_string(dir().join("config.json")).ok()?;
        let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
        v.get("url")?.as_str().map(str::to_string).filter(|u| !u.trim().is_empty())
    };
    normalize(&from_env.or_else(from_file).unwrap_or_else(|| DEFAULT_URL.to_string()))
}

/// The paired client's token, from `config.json` (written by `loft-herdr setup`, see ADR 0004).
pub fn token() -> Option<String> {
    let raw = std::fs::read_to_string(dir().join("config.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("token")?.as_str().map(str::to_string).filter(|t| !t.is_empty())
}

/// Writes `config.json` with the Loft address and, if there is one, the token: readable by this user only.
pub fn save(url: &str, token: Option<&str>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::create_dir_all(dir())?;
    let mut v = serde_json::json!({ "url": url });
    if let Some(t) = token {
        v["token"] = t.into();
    }
    let path = dir().join("config.json");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    writeln!(f, "{}", serde_json::to_string_pretty(&v)?)
}

/// Trims spaces and trailing slashes, and adds `http://` to a bare host.
pub fn normalize(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    if u.contains("://") { u.to_string() } else { format!("http://{u}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_the_token_readable_by_this_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("loft-herdr-test-{}", std::process::id()));
        // SAFETY: the only test that sets it.
        unsafe { std::env::set_var("LOFT_HERDR_HOME", &tmp) };
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("config.json"), "{}").unwrap();
        std::fs::set_permissions(tmp.join("config.json"), std::fs::Permissions::from_mode(0o644)).unwrap();
        save("http://x:1", Some("abc")).unwrap();
        assert_eq!(token().as_deref(), Some("abc"));
        assert_eq!(std::fs::metadata(tmp.join("config.json")).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn normalizes_addresses() {
        assert_eq!(normalize(" https://homebase.ts.net/ "), "https://homebase.ts.net");
        assert_eq!(normalize("localhost:7420"), "http://localhost:7420");
    }
}
