//! Opens a Herdr tab for every Loft agent that doesn't have one yet, in a workspace named "Loft".
//!
//! Each chat pane reports its Loft agent id to Herdr as a pane metadata token, so the pane list
//! tells which agents already have a pane; nothing else is stored.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use serde_json::{Value, json};

use crate::herdr::{Herdr, PLUGIN_ID, TOKEN};
use crate::loft::Loft;

pub const WORKSPACE: &str = "Loft";

/// The Loft agent ids that already have a chat pane.
pub fn open_agents(panes: &[Value]) -> HashSet<String> {
    panes
        .iter()
        .filter_map(|p| p.pointer(&format!("/tokens/{TOKEN}")))
        .filter_map(|v| v.as_str().filter(|v| !v.is_empty()).map(str::to_string))
        .collect()
}

/// One sync at a time: chat panes start one when Loft gets a new agent, and they all see it.
struct Lock(std::path::PathBuf);

impl Lock {
    fn take() -> Option<Lock> {
        let dir = crate::config::dir();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sync.lock");
        if let Ok(meta) = std::fs::metadata(&path) {
            let age = meta.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok()).unwrap_or_default();
            if age < Duration::from_secs(20) {
                return None;
            }
            let _ = std::fs::remove_file(&path);
        }
        OpenOptions::new().write(true).create_new(true).open(&path).ok().map(|_| Lock(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn sync(loft: &Loft, herdr: &Herdr) -> Result<String> {
    let Some(_lock) = Lock::take() else { return Ok("Another sync is already running.".into()) };
    let agents = loft.agents()?;
    let panes = herdr.call("pane.list", json!({}))?.get("panes").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let open = open_agents(&panes);
    let missing: Vec<&Value> = agents.iter().filter(|a| a.get("id").and_then(|i| i.as_str()).is_some_and(|id| !open.contains(id))).collect();
    if missing.is_empty() {
        return Ok(format!("All {} Loft agents are already in Herdr.", agents.len()));
    }

    let workspaces = herdr.call("workspace.list", json!({}))?.get("workspaces").and_then(|w| w.as_array()).cloned().unwrap_or_default();
    let existing = workspaces.iter().find(|w| w.get("label").and_then(|l| l.as_str()) == Some(WORKSPACE));
    let (workspace, scratch_tab) = match existing {
        Some(w) => (w["workspace_id"].as_str().unwrap_or_default().to_string(), None),
        None => {
            let r = herdr.call("workspace.create", json!({ "label": WORKSPACE, "focus": false }))?;
            let ws = r.pointer("/workspace/workspace_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            // The shell tab a new workspace starts with; closed once the agents' tabs are there.
            (ws, r.pointer("/tab/tab_id").and_then(|v| v.as_str()).map(str::to_string))
        }
    };

    let mut opened = 0;
    let mut failed = Vec::new();
    for a in missing {
        let id = a["id"].as_str().unwrap_or_default();
        let name = a.get("name").and_then(|n| n.as_str()).unwrap_or(id);
        let r = herdr.call(
            "plugin.pane.open",
            json!({
                "plugin_id": PLUGIN_ID, "entrypoint": "agent", "placement": "tab", "workspace_id": workspace,
                "focus": false, "env": { "LOFT_AGENT_ID": id, "LOFT_URL": loft.base },
            }),
        );
        match r {
            Ok(r) => {
                opened += 1;
                if let Some(tab) = r.pointer("/plugin_pane/pane/tab_id").and_then(|t| t.as_str()) {
                    let _ = herdr.call("tab.rename", json!({ "tab_id": tab, "label": name }));
                }
            }
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    if opened > 0 {
        if let Some(tab) = scratch_tab {
            let _ = herdr.call("tab.close", json!({ "tab_id": tab }));
        }
        // Hold the lock until the new panes have reported, so a second sync doesn't open them again.
        std::thread::sleep(Duration::from_millis(1500));
    }
    let mut msg = format!("Opened {opened} Loft agent{} in the {WORKSPACE} workspace.", if opened == 1 { "" } else { "s" });
    if !failed.is_empty() {
        msg.push_str(&format!(" Failed: {}", failed.join("; ")));
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_agents_that_have_a_pane() {
        let panes = vec![
            json!({"pane_id":"w1:p1","agent":"loft","tokens":{"loft_agent":"a1"}}),
            json!({"pane_id":"w1:p2","agent":"claude","agent_session":{"source":"herdr:claude","value":"x"}}),
            json!({"pane_id":"w1:p4","tokens":{"loft_agent":""}}),
            json!({"pane_id":"w1:p3"}),
        ];
        assert_eq!(open_agents(&panes), HashSet::from(["a1".to_string()]));
    }
}
