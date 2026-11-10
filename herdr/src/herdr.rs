//! Herdr's socket API: newline-delimited JSON requests over a Unix socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

/// Identifies this integration in Herdr. Never `herdr:`-prefixed (Herdr's own).
pub const SOURCE: &str = "loft";
/// The agent label Herdr keeps for every Loft pane; the visible name is each agent's own.
pub const AGENT: &str = "loft";
pub const PLUGIN_ID: &str = "loft";
/// The pane metadata token holding the Loft agent id a chat pane shows. (Herdr keeps agent
/// session ids only for its own integrations, so this is how `sync` recognizes Loft panes.)
pub const TOKEN: &str = "loft_agent";

pub struct Herdr {
    socket: PathBuf,
}

impl Herdr {
    /// The socket of the Herdr session this process runs in, else the default one.
    pub fn connect() -> Self {
        let socket = std::env::var_os("HERDR_SOCKET_PATH")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::config::home().join(".config/herdr/herdr.sock"));
        Self { socket }
    }

    /// Inside a Herdr pane: the pane id, which state reports need.
    pub fn pane_id() -> Option<String> {
        (std::env::var("HERDR_ENV").ok().as_deref() == Some("1"))
            .then(|| std::env::var("HERDR_PANE_ID").ok())
            .flatten()
            .filter(|p| !p.is_empty())
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut s = UnixStream::connect(&self.socket).map_err(|e| anyhow!("Herdr isn't running ({}): {e}", self.socket.display()))?;
        s.set_read_timeout(Some(Duration::from_secs(15)))?;
        s.set_write_timeout(Some(Duration::from_secs(5)))?;
        let id = format!("loft_{}", now_ms());
        let mut line = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))?;
        line.push('\n');
        s.write_all(line.as_bytes())?;
        let mut reply = String::new();
        BufReader::new(s).read_line(&mut reply)?;
        let v: Value = serde_json::from_str(&reply).map_err(|e| anyhow!("bad reply from Herdr: {e}"))?;
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(|c| c.as_str()).unwrap_or("error");
            let msg = err.get("message").and_then(|c| c.as_str()).unwrap_or("");
            return Err(anyhow!("{method}: {code} {msg}"));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

/// How a Loft agent shows in Herdr: its lifecycle state, why it's blocked, its name and a title.
#[derive(Clone, PartialEq, Debug)]
pub struct Presence {
    pub state: &'static str,
    pub message: Option<String>,
    pub name: String,
    pub title: String,
}

/// `blocked` while a permission (or a grill round) waits for the user, `working` during a run, else `idle`.
pub fn presence(agent: &Value, pending_tool: Option<&str>, grill_open: bool) -> Presence {
    let str_of = |k: &str| agent.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let running = agent.get("running").and_then(|v| v.as_bool()) == Some(true) || str_of("status") == "running";
    let (state, message) = match (pending_tool, grill_open) {
        (Some(tool), _) => ("blocked", Some(format!("Permission: {tool}"))),
        (None, true) if !running => ("blocked", Some("Grill round waiting for your answers".to_string())),
        _ if running => ("working", None),
        _ => ("idle", None),
    };
    let status_text = str_of("status_text");
    Presence { state, message, name: str_of("name"), title: if status_text.trim().is_empty() { str_of("title") } else { status_text } }
}

pub enum Report {
    Presence(Presence),
    Release,
}

/// Sends the latest presence to Herdr on its own thread, dropping stale ones; `Release` ends it.
pub fn reporter(pane: String, agent_id: String) -> (std::sync::mpsc::Sender<Report>, std::thread::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<Report>();
    let handle = std::thread::spawn(move || {
        let herdr = Herdr::connect();
        let resume = on_path("loft-herdr").then(|| json!(["loft-herdr", "chat", agent_id]));
        let mut seq = 0u64;
        let mut named = String::new();
        while let Ok(mut r) = rx.recv() {
            while let Ok(newer) = rx.try_recv() {
                r = newer;
            }
            match r {
                Report::Presence(p) => {
                    let mut params = json!({
                        "pane_id": pane, "source": SOURCE, "agent": AGENT, "state": p.state,
                        "message": p.message, "seq": next_seq(&mut seq), "agent_session_id": agent_id,
                    });
                    if let Some(r) = &resume {
                        params["resume_argv"] = r.clone();
                    }
                    if herdr.call("pane.report_agent", params.clone()).is_err() && resume.is_some() {
                        // An older Herdr without resume commands: report without one.
                        params.as_object_mut().map(|o| o.remove("resume_argv"));
                        params["seq"] = json!(next_seq(&mut seq));
                        let _ = herdr.call("pane.report_agent", params);
                    }
                    let _ = herdr.call(
                        "pane.report_metadata",
                        json!({
                            "pane_id": pane, "source": SOURCE, "display_agent": p.name, "title": p.title,
                            "tokens": { TOKEN: agent_id }, "seq": next_seq(&mut seq),
                        }),
                    );
                    if named != p.name {
                        named = p.name.clone();
                        if let Some(n) = agent_name(&p.name) {
                            let _ = herdr.call("agent.rename", json!({ "target": pane, "name": n }));
                        }
                    }
                }
                Report::Release => {
                    let _ = herdr.call("pane.report_metadata", json!({ "pane_id": pane, "source": SOURCE, "tokens": { TOKEN: null }, "seq": next_seq(&mut seq) }));
                    let _ = herdr.call("pane.release_agent", json!({ "pane_id": pane, "source": SOURCE, "agent": AGENT, "seq": next_seq(&mut seq) }));
                    return;
                }
            }
        }
    });
    (tx, handle)
}

/// Whether a plain command name resolves on PATH (Herdr only accepts such resume commands).
pub fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A strictly increasing sequence number for reports, so Herdr never keeps a stale one.
pub fn next_seq(last: &mut u64) -> u64 {
    *last = (*last + 1).max(now_ms());
    *last
}

/// A Herdr live agent name for a Loft agent: `[a-z][a-z0-9_-]{0,31}`.
pub fn agent_name(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').trim_start_matches(|c: char| !c.is_ascii_lowercase());
    let out: String = out.chars().take(32).collect();
    let out = out.trim_end_matches('-').to_string();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_names_fit_herdr() {
        assert_eq!(agent_name("Findmeal QA").as_deref(), Some("findmeal-qa"));
        assert_eq!(agent_name("Passora Dev Lovable").as_deref(), Some("passora-dev-lovable"));
        assert_eq!(agent_name("2nd reviewer!").as_deref(), Some("nd-reviewer"));
        assert_eq!(agent_name("🚀"), None);
        assert!(agent_name(&"a".repeat(50)).unwrap().len() <= 32);
    }

    #[test]
    fn presence_follows_the_agent() {
        let idle = json!({"name":"lofty","title":"Loft's own","status":"idle","status_text":""});
        let p = presence(&idle, None, false);
        assert_eq!((p.state, p.title.as_str(), p.name.as_str()), ("idle", "Loft's own", "lofty"));
        let running = json!({"name":"lofty","running":true,"status":"running","status_text":"Reviewing #12"});
        let p = presence(&running, None, false);
        assert_eq!((p.state, p.title.as_str()), ("working", "Reviewing #12"));
        let p = presence(&running, Some("Bash"), false);
        assert_eq!((p.state, p.message.as_deref()), ("blocked", Some("Permission: Bash")));
        assert_eq!(presence(&idle, None, true).state, "blocked");
        assert_eq!(presence(&running, None, true).state, "working", "a grill being prepared is work");
    }

    #[test]
    fn seq_always_grows() {
        let mut last = u64::MAX - 10;
        let a = next_seq(&mut last);
        let b = next_seq(&mut last);
        assert!(b > a);
    }
}
