use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::{Notify, broadcast, oneshot};

use crate::db::Db;
use crate::repos::RepoCache;

/// Event pushed to every connected browser over SSE.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    AgentUpdated { agent: serde_json::Value },
    AgentDeleted { agent_id: String },
    Message { message: serde_json::Value },
    Permission { request: serde_json::Value },
    Memory { agent_id: String },
    PullRequest { pr: serde_json::Value },
    Update { status: serde_json::Value },
    Task { task: serde_json::Value },
    TaskDeleted { task_id: String },
    System { system: serde_json::Value },
    Grill { session: serde_json::Value },
    TaskComment { comment: serde_json::Value },
    /// The "needs you" list changed: its size, how many are urgent, and the item ids.
    NeedsYou { count: u64, urgent: u64, ids: Vec<serde_json::Value> },
}

/// Decision sent back to a blocked permission prompt.
#[derive(Debug, Clone)]
pub struct Decision {
    pub allow: bool,
    pub message: Option<String>,
}

pub struct Config {
    pub port: u16,
    pub storage_dir: PathBuf,
}

#[derive(Clone, Default)]
pub struct RunInfo {
    /// Last event from claude (RFC 3339).
    pub last_output_at: String,
    /// Background tasks the agent waits on after its reply, and since when.
    pub waiting_on: Vec<String>,
    pub waiting_since: Option<String>,
}

#[derive(Default)]
pub struct Runtime {
    /// agent id -> kill switch for the currently running claude process
    pub running: HashMap<String, Arc<Notify>>,
    /// agent id -> user messages that arrived while a run was in progress
    pub queued: HashMap<String, Vec<String>>,
    /// agent id -> grill follow-ups (prompt, trigger) waiting for the current run; run quietly after user work
    pub grill_queued: HashMap<String, Vec<(String, &'static str)>>,
    /// agent id -> wake-up for a run that has replied but waits on a Claude Code background task;
    /// new work stops the task so the queue isn't held up behind it
    pub waiting: HashMap<String, Arc<Notify>>,
    /// agent id -> what its current run is doing, for the "waiting" / "no activity" display
    pub runs: HashMap<String, RunInfo>,
    /// agents whose run the user restarted; the stop notice says so
    pub restarting: std::collections::HashSet<String>,
    /// agent id -> chats it was @-mentioned in, waiting for the run that answers the mention
    pub mention_pending: HashMap<String, Vec<String>>,
    /// Ready PRs on GitHub that wait on the user (their own, or their review requested), from github_inbox
    pub github_inbox: Vec<crate::github_inbox::GhPr>,
    /// permission request id -> waiter blocked inside an MCP call
    pub waiters: HashMap<String, oneshot::Sender<Decision>>,
}

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: Db,
    pub events: broadcast::Sender<Event>,
    pub rt: Arc<Mutex<Runtime>>,
    pub repos: Arc<RepoCache>,
    pub http: reqwest::Client,
    pub gh_token: Arc<crate::github::TokenCache>,
    pub update: Arc<Mutex<crate::updater::UpdateStatus>>,
    pub settings: Arc<crate::settings::Store>,
    /// Last Claude plan usage reading.
    pub usage: Arc<crate::usage::Cache>,
    /// Latest CPU temperature reading.
    pub system: Arc<crate::system::Cache>,
}

impl AppState {
    pub fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
    }
}
