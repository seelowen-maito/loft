//! Mac access: whether macOS lets Loft (and so its agents) into the folders it guards with a privacy
//! prompt (TCC). The grant belongs to the Loft binary: agents' file reads are attributed to it. On a
//! Mac with no screen nobody sees the prompt, and the agent's read waits on it.
//!
//! The check only runs when the user asks, since listing an undecided folder is what shows the prompt.
//! It lists a fixed set of folders in a child process, to /dev/null, and keeps only the exit status:
//! no names or contents are read, and no other path can be asked for.

use std::path::Path;
use std::time::Duration;

use axum::Json;
use serde_json::{Value, json};
use tokio::process::Command;

/// Long enough for a plain listing; longer means macOS is showing a prompt on the Mac.
const WAIT: Duration = Duration::from_secs(3);

/// (id, label, folder under the home folder). The last one needs Full Disk Access, which never prompts.
const FOLDERS: [(&str, &str, &str); 5] = [
    ("desktop", "Desktop", "Desktop"),
    ("documents", "Documents", "Documents"),
    ("downloads", "Downloads", "Downloads"),
    ("icloud", "iCloud Drive", "Library/Mobile Documents"),
    ("full_disk", "Full Disk Access", "Library/Application Support/com.apple.TCC"),
];

/// `POST /api/system/mac-access`: check each folder now.
pub async fn check() -> Json<Value> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut v = run(&home, "/bin/ls").await;
    let exe = std::env::current_exe().ok();
    v["binary"] = json!(exe.as_ref().map(|p| p.display().to_string()));
    v["adhoc_signed"] = json!(match &exe {
        Some(p) => adhoc_signed(p).await,
        None => None,
    });
    v["host"] = json!(crate::tailnet::host());
    Json(v)
}

/// Each folder as allowed, blocked, waiting (a prompt is up on the Mac) or missing, listed with `probe`.
pub(crate) async fn run(home: &Path, probe: &str) -> Value {
    let checks = FOLDERS.iter().map(|(id, label, rel)| async move {
        let dir = home.join(rel);
        // Stat-ing the folder itself never prompts; listing it does.
        let state = if std::fs::symlink_metadata(&dir).is_err() {
            "missing"
        } else {
            let mut cmd = Command::new(probe);
            cmd.arg(&dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
            match tokio::time::timeout(WAIT, cmd.status()).await {
                Err(_) => "waiting",
                Ok(Ok(s)) if s.success() => "allowed",
                Ok(_) => "blocked",
            }
        };
        json!({ "id": id, "label": label, "folder": format!("~/{rel}"), "state": state })
    });
    json!({ "items": futures::future::join_all(checks).await })
}

/// Whether the binary is ad-hoc signed: macOS then remembers its grants only until the next build.
async fn adhoc_signed(exe: &Path) -> Option<bool> {
    let out = tokio::time::timeout(WAIT, Command::new("/usr/bin/codesign").arg("-dv").arg(exe).kill_on_drop(true).output()).await.ok()?.ok()?;
    Some(String::from_utf8_lossy(&out.stderr).contains("Signature=adhoc"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn each_folder_reads_as_allowed_blocked_waiting_or_missing_and_a_prompt_never_hangs_the_check() {
        let home = std::env::temp_dir().join(format!("loft-mac-{}", uuid::Uuid::new_v4()));
        for d in ["Desktop", "Documents", "Downloads", "Library/Application Support/com.apple.TCC"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        // A fake `ls`: Desktop waits like a prompt nobody answers, Documents is denied, Full Disk Access
        // works; it records what it was asked to list.
        let probe = home.join("probe.sh");
        let log = home.join("asked.txt");
        std::fs::write(&probe, format!("#!/bin/sh\necho \"$1\" >> '{}'\ncase \"$1\" in\n  */Desktop) exec sleep 30 ;;\n  */Documents) exit 1 ;;\nesac\nexit 0\n", log.display())).unwrap();
        std::fs::set_permissions(&probe, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let started = std::time::Instant::now();
        let v = run(&home, probe.to_str().unwrap()).await;
        assert!(started.elapsed() < WAIT + Duration::from_secs(2), "{:?}", started.elapsed());
        let states: Vec<(String, String)> = v["items"].as_array().unwrap().iter().map(|i| (i["id"].as_str().unwrap().into(), i["state"].as_str().unwrap().into())).collect();
        let want = [("desktop", "waiting"), ("documents", "blocked"), ("downloads", "allowed"), ("icloud", "missing"), ("full_disk", "allowed")];
        assert_eq!(states, want.map(|(a, b)| (a.to_string(), b.to_string())));
        assert_eq!(v["items"][3]["folder"], json!("~/Library/Mobile Documents"));
        // Only the fixed folders are listed, and a missing one isn't listed at all.
        let mut asked: Vec<String> = std::fs::read_to_string(&log).unwrap().lines().map(|l| l.strip_prefix(home.to_str().unwrap()).unwrap().to_string()).collect();
        asked.sort();
        assert_eq!(asked, ["/Desktop", "/Documents", "/Downloads", "/Library/Application Support/com.apple.TCC"]);
        // The waiting probe was killed when the check gave up on it.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let left = std::process::Command::new("pgrep").args(["-f", probe.to_str().unwrap()]).output().unwrap();
        assert!(left.stdout.is_empty(), "probe still running: {}", String::from_utf8_lossy(&left.stdout));
        std::fs::remove_dir_all(&home).ok();
    }
}
