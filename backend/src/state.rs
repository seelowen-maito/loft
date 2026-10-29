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
    /// Loft published issues on a project's repo; the Issues tab refetches.
    Issues { project: String },
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
    /// Subagents the agent itself started (Agent tool calls) that haven't returned yet.
    pub subagents: Vec<Subagent>,
    /// The run answers the user, so a Restart carries that over to the run it starts.
    pub for_user: bool,
}

/// A running subagent, as clients show it above the composer.
#[derive(Clone, serde::Serialize)]
pub struct Subagent {
    /// The Agent tool call's id.
    pub id: String,
    pub description: String,
    /// The subagent type, e.g. Explore or general-purpose.
    #[serde(rename = "type")]
    pub kind: String,
    pub started_at: String,
    /// Claude Code's task id for it, from its task_started.
    #[serde(skip)]
    pub task_id: Option<String>,
    /// Runs in the background: its tool result only says it launched.
    #[serde(skip)]
    pub background: bool,
    #[serde(skip)]
    pub launched: bool,
}

#[derive(Default)]
pub struct Runtime {
    /// agent id -> kill switch for the currently running claude process
    pub running: HashMap<String, Arc<Notify>>,
    /// agent id -> user messages that arrived while a run was in progress
    pub queued: HashMap<String, Vec<String>>,
    /// agents whose queued work includes the user's own message or a schedule: the run it starts counts
    /// for an Unread reply
    pub queued_for_user: std::collections::HashSet<String>,
    /// agent id -> grill prompts waiting for the current run; run quietly after user work
    pub grill_queued: HashMap<String, Vec<String>>,
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
    /// Spaces out write requests to GitHub.
    pub gh_pace: Arc<crate::github::Pace>,
    pub update: Arc<Mutex<crate::updater::UpdateStatus>>,
    pub settings: Arc<crate::settings::Store>,
    /// Last Claude plan usage reading.
    pub usage: Arc<crate::usage::Cache>,
    /// Latest CPU temperature reading.
    pub system: Arc<crate::system::Cache>,
    /// Serializes writes to each agent's grill session.
    pub grill_locks: Arc<crate::grill::Locks>,
    /// Serializes publishing issues to each GitHub repo.
    pub publish_locks: Arc<crate::planning::Locks>,
    /// What publishing already looked up on GitHub: the token's login, repos whose labels exist.
    pub publish_known: Arc<crate::planning::Known>,
    /// Where mirroring each GitHub repo stands.
    pub mirror: Arc<crate::mirror::Mirror>,
}

impl AppState {
    pub fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
    }
}
