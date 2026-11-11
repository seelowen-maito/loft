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

/// Writes `config.json` with the Loft address.
pub fn save(url: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir())?;
    std::fs::write(dir().join("config.json"), serde_json::to_string_pretty(&serde_json::json!({ "url": url }))? + "\n")
}

/// Drops the `token` key an older `config.json` holds from the pairing Loft no longer has. The rest stays.
pub fn forget_token() {
    forget_token_in(&dir().join("config.json"));
}

fn forget_token_in(file: &std::path::Path) {
    let Ok(raw) = std::fs::read_to_string(file) else { return };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) else { return };
    if v.as_object_mut().and_then(|o| o.remove("token")).is_some() {
        let _ = std::fs::write(file, serde_json::to_string_pretty(&v).unwrap_or_default() + "\n");
    }
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
    fn an_old_token_is_dropped_and_the_address_kept() {
        let file = std::env::temp_dir().join(format!("loft-herdr-config-{}.json", std::process::id()));
        std::fs::write(&file, r#"{ "url": "http://x:1", "token": "abc" }"#).unwrap();
        forget_token_in(&file);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v, serde_json::json!({ "url": "http://x:1" }));
        forget_token_in(&file);
        forget_token_in(&file.with_extension("missing"));
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn normalizes_addresses() {
        assert_eq!(normalize(" https://homebase.ts.net/ "), "https://homebase.ts.net");
        assert_eq!(normalize("localhost:7420"), "http://localhost:7420");
    }
}
