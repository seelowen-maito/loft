//! Mac access: whether macOS lets Loft (and so its agents) into the folders it guards with a privacy
//! prompt (TCC). The grant belongs to the Loft binary: agents' file reads are attributed to it. On a
//! Mac with no screen nobody sees the prompt, and the agent's read waits on it.
//!
//! The check only runs when the user asks, since listing an undecided folder is what shows the prompt.
//! It lists a fixed set of folders in a child process, to /dev/null, and keeps only the exit status:
//! no names or contents are read, and no other path can be asked for.
//!
//! While agents work, `watcher` follows macOS's privacy log for requests made in Loft's name. One that
//! goes unanswered is a prompt nobody sees: Needs you says so, naming the folder only.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::state::AppState;

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

/// `POST /api/system/mac-access`: check each folder now. A folder that shows allowed clears its
/// "macOS is asking" item.
pub async fn check(State(state): State<AppState>) -> Json<Value> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut v = run(&home, "/bin/ls").await;
    state.rt.lock().unwrap().tcc.clear_allowed(&v);
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

/// The folder services macOS asks about: (service, what the item names, the check's id for it). Requests
/// for anything else (another app's, the camera...) are never kept.
const SERVICES: [(&str, &str, Option<&str>); 6] = [
    ("kTCCServiceSystemPolicyDesktopFolder", "Desktop", Some("desktop")),
    ("kTCCServiceSystemPolicyDocumentsFolder", "Documents", Some("documents")),
    ("kTCCServiceSystemPolicyDownloadsFolder", "Downloads", Some("downloads")),
    ("kTCCServiceFileProviderDomain", "iCloud Drive", Some("icloud")),
    ("kTCCServiceSystemPolicyRemovableVolumes", "an external drive", None),
    ("kTCCServiceSystemPolicyNetworkVolumes", "a network drive", None),
];

/// A request still unanswered after this is a prompt waiting on the Mac's screen.
const ASKING_AFTER: Duration = Duration::from_secs(10);
/// Requests kept at most, and for how long, should an answer never be logged.
const MAX_PENDING: usize = 200;
const FORGET_AFTER: Duration = Duration::from_secs(24 * 3600);

/// Only the request, attribution and answer lines of macOS's privacy log.
const PREDICATE: &str = r#"subsystem == "com.apple.TCC" AND (eventMessage BEGINSWITH "AUTHREQ_CTX" OR eventMessage BEGINSWITH "AUTHREQ_ATTRIBUTION" OR eventMessage BEGINSWITH "AUTHREQ_RESULT")"#;

struct Pending {
    service: usize,
    /// Known once its attribution line names the Loft binary.
    loft: bool,
    at: Instant,
    since: String,
}

/// Folder requests made in Loft's name that macOS hasn't answered yet, from its privacy log.
#[derive(Default)]
pub struct Requests {
    pending: HashMap<String, Pending>,
}

/// A folder macOS is asking about: what the item names, the check's id, and since when.
pub(crate) struct Asking {
    pub service: &'static str,
    pub folder: &'static str,
    pub since: String,
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = &line[line.find(key)? + key.len()..];
    Some(&rest[..rest.find([',', '}']).unwrap_or(rest.len())])
}

impl Requests {
    /// One line of the log; `exe` is the Loft binary's path, the only one compared, never kept.
    pub(crate) fn feed(&mut self, line: &str, exe: &str, now: Instant) {
        let Some(id) = field(line, "msgID=") else { return };
        if line.contains("AUTHREQ_CTX:") {
            let service = field(line, "service=").and_then(|s| SERVICES.iter().position(|(name, ..)| *name == s));
            if let (Some(service), true) = (service, line.contains("preflight=no")) {
                self.pending.retain(|_, p| now.duration_since(p.at) < FORGET_AFTER);
                if self.pending.len() < MAX_PENDING {
                    self.pending.insert(id.to_string(), Pending { service, loft: false, at: now, since: crate::db::now() });
                }
            }
        } else if line.contains("AUTHREQ_ATTRIBUTION:") {
            // The first path is the responsible process's: the one macOS asks the user about.
            let ours = field(line, "responsible_path=") == Some(exe);
            match self.pending.get_mut(id) {
                Some(p) if ours => p.loft = true,
                Some(_) => {
                    self.pending.remove(id);
                }
                None => {}
            }
        } else if line.contains("AUTHREQ_RESULT:") {
            self.pending.remove(id);
        }
    }

    /// One per folder: Loft's requests unanswered for a while, the oldest first.
    pub(crate) fn asking(&self, now: Instant) -> Vec<Asking> {
        let mut out: Vec<(Instant, usize, String)> = vec![];
        for p in self.pending.values().filter(|p| p.loft && now.duration_since(p.at) >= ASKING_AFTER) {
            match out.iter_mut().find(|o| o.1 == p.service) {
                Some(o) if p.at < o.0 => *o = (p.at, p.service, p.since.clone()),
                Some(_) => {}
                None => out.push((p.at, p.service, p.since.clone())),
            }
        }
        out.sort_by_key(|o| o.0);
        out.into_iter().map(|(_, s, since)| Asking { service: SERVICES[s].0, folder: SERVICES[s].1, since }).collect()
    }

    /// Forgets the requests for folders a check found allowed: someone answered on the Mac.
    fn clear_allowed(&mut self, check: &Value) {
        let allowed: Vec<&str> = check["items"].as_array().into_iter().flatten().filter(|i| i["state"] == "allowed").filter_map(|i| i["id"].as_str()).collect();
        self.pending.retain(|_, p| SERVICES[p.service].2.is_none_or(|c| !allowed.contains(&c)));
    }
}

/// Follows macOS's privacy log for as long as Loft runs (restarted if `log` stops).
pub async fn watcher(state: AppState) {
    let Some(exe) = std::env::current_exe().ok().map(|p| p.display().to_string()) else { return };
    loop {
        let child = Command::new("/usr/bin/log")
            .args(["stream", "--style", "compact", "--predicate", PREDICATE])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();
        if let Ok(mut child) = child
            && let Some(out) = child.stdout.take()
        {
            let mut lines = tokio::io::BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                state.rt.lock().unwrap().tcc.feed(&line, &exe, Instant::now());
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
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

    /// Lines of macOS's privacy log as `log stream --style compact` prints them, recorded on the Mac mini
    /// (1 Nov 2026); only the paths are changed.
    const EXE: &str = "/Applications/Loft/loft";
    fn ctx(id: &str, service: &str, preflight: &str) -> String {
        format!("2026-11-01 18:51:48.258 Df tccd[1110:270c] [com.apple.TCC:access] AUTHREQ_CTX: msgID={id}, function=TCCAccessRequest, service={service}, preflight={preflight}, query=1, client_dict=(null), daemon_dict=<private>")
    }
    fn attribution(id: &str, responsible: &str) -> String {
        format!("2026-11-01 18:51:48.258 Df tccd[1110:270c] [com.apple.TCC:access] AUTHREQ_ATTRIBUTION: msgID={id}, attribution={{responsible={{TCCDProcess: identifier=loft-c50fb1ccc5eebc97, pid=1343, auid=501, euid=501, responsible_path={responsible}, binary_path={responsible}}}, accessing={{TCCDProcess: identifier=com.apple.dt.xcodebuild, pid=1744, auid=501, euid=501, binary_path=/Applications/Xcode.app/Contents/Developer/usr/bin/xcodebuild}}, requesting={{TCCDProcess: identifier=com.apple.sandboxd, pid=437, auid=0, euid=0, binary_path=/usr/libexec/sandboxd}}, }},")
    }
    fn result(id: &str) -> String {
        format!("2026-11-01 18:51:48.260 Df tccd[1110:270c] [com.apple.TCC:access] AUTHREQ_RESULT: msgID={id}, authValue=2, authReason=2, authVersion=1, desired_auth=0, error=(null),")
    }

    #[tokio::test]
    async fn an_unanswered_folder_request_in_loft_s_name_is_a_needs_you_item_until_answered() {
        let st = crate::tasks::tests::state();
        let feed = |lines: &[String], ago: u64| {
            let at = Instant::now() - Duration::from_secs(ago);
            let mut rt = st.rt.lock().unwrap();
            for l in lines {
                rt.tcc.feed(l, EXE, at);
            }
        };
        let items = || crate::needs_you::collect(&st).unwrap().into_iter().filter(|i| i.kind == "mac_access").collect::<Vec<_>>();
        let push = || items().iter().map(crate::needs_you::to_push).collect::<Vec<_>>();

        // Loft asked for an external drive 11s ago, no answer yet. Ignored: another app's request, Loft's
        // Full Disk Access preflight (never a prompt), a request answered at once, and one only 2s old.
        feed(&[ctx("437.80", "kTCCServiceSystemPolicyRemovableVolumes", "no"), attribution("437.80", EXE)], 11);
        feed(&[ctx("437.73", "kTCCServiceSystemPolicyDownloadsFolder", "no"), attribution("437.73", "/Applications/Raycast.app/Contents/MacOS/Raycast")], 11);
        feed(&[ctx("437.72", "kTCCServiceSystemPolicyAllFiles", "yes"), attribution("437.72", EXE)], 11);
        feed(&[ctx("437.81", "kTCCServiceSystemPolicyDesktopFolder", "no"), attribution("437.81", EXE), result("437.81")], 11);
        feed(&[ctx("437.82", "kTCCServiceSystemPolicyDocumentsFolder", "no"), attribution("437.82", EXE)], 2);
        // A path that only starts like Loft's isn't Loft's.
        feed(&[ctx("437.83", "kTCCServiceSystemPolicyDesktopFolder", "no"), attribution("437.83", &format!("{EXE} copy"))], 11);

        let one = items();
        assert_eq!(one.len(), 1);
        let p = crate::needs_you::to_push(&one[0]);
        assert_eq!(p.title, "macOS is asking for access");
        assert!(p.body.starts_with("an external drive on "), "{}", p.body);
        // Pushed with "stuck", which phones get by default; the same id each tick, so it's pushed once.
        assert_eq!((push()[0].kind, push()[0].id.clone()), ("stuck", p.id.clone()));
        let snap = crate::needs_you::snapshot(&st).unwrap();
        let item = snap["items"].as_array().unwrap().iter().find(|i| i["type"] == "mac_access").unwrap();
        assert!(item["title"].as_str().unwrap().starts_with("macOS is asking for access to an external drive on "), "{item}");
        assert!(!item.to_string().contains("/Applications"), "no path in what's shown: {item}");
        assert_eq!(item["urgency"], json!(3));
        assert!(item["actions"].as_array().unwrap().iter().any(|a| a["label"] == "Check again" && a["path"] == "/api/system/mac-access"), "{item}");

        // Answered on the Mac: gone.
        feed(&[result("437.80")], 0);
        assert!(items().is_empty());

        // Still waiting, then Check again finds Downloads allowed: gone too.
        feed(&[ctx("437.90", "kTCCServiceSystemPolicyDownloadsFolder", "no"), attribution("437.90", EXE)], 11);
        assert_eq!(items().len(), 1);
        let home = std::env::temp_dir().join(format!("loft-mac-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        let check = run(&home, "/bin/ls").await;
        st.rt.lock().unwrap().tcc.clear_allowed(&check);
        assert!(items().is_empty());
        std::fs::remove_dir_all(&home).ok();
    }

}
