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
    /// Loft published issues on a project's repo.
    Issues { project: String },
    /// A Delivery changed: its state, its tickets or who has them.
    Delivery { repo: String, spec: i64 },
    /// The "needs you" list changed: its size, how many are urgent, how many of each the user hasn't seen
    /// (`new`, `new_urgent`: the counter), and the item ids.
    NeedsYou { count: u64, urgent: u64, new: u64, new_urgent: u64, ids: Vec<serde_json::Value> },
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
    /// `xcrun`, which runs `simctl` (simulator.rs). Tests point it at a fake.
    pub xcrun: PathBuf,
    /// How `xcrun` finds Xcode (simulator.rs). Tests fake it.
    pub xcode: XcodeLookup,
}

/// Where Xcode is looked for when `xcode-select` points at the Command Line Tools, which have no `simctl`.
#[derive(Clone)]
pub struct XcodeLookup {
    /// `DEVELOPER_DIR` as Loft was started with it: used as is when set.
    pub developer_dir: Option<PathBuf>,
    pub xcode_select: PathBuf,
    /// Where `/Applications` and `/Volumes` are: `/`, or a temp dir in tests.
    pub root: PathBuf,
}

impl XcodeLookup {
    pub fn system() -> Self {
        let developer_dir = std::env::var_os("DEVELOPER_DIR").filter(|d| !d.is_empty()).map(PathBuf::from);
        Self { developer_dir, xcode_select: "xcode-select".into(), root: "/".into() }
    }
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
    /// Tool calls (ids) whose result hasn't come back yet.
    pub tools_open: Vec<String>,
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
    /// agent id -> the agent messages (their ids in its chat) waiting in `queued` for the current run to end
    pub queued_messages: HashMap<String, Vec<i64>>,
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
    /// agents whose next run starts a Delivery ticket they haven't worked on: Loft first clears their
    /// context if their Fresh context setting says so
    pub fresh_before: std::collections::HashSet<String>,
    /// agent id -> the Delivery steps its queued work was woken for, each with the queued prompt that wakes
    /// for it (None for a step moved back from an ended or restarted run): the run that takes the queue takes
    /// them, and Pause drops a paused Delivery's (delivery::pause)
    pub delivery_queued: HashMap<String, Vec<(crate::delivery::Wake, Option<String>)>>,
    /// agent id -> the Delivery steps its current run was woken for, checked when it ends (delivery::run_ended)
    pub delivery_running: HashMap<String, Vec<crate::delivery::Wake>>,
    /// Needs you item ids that became news at this start-up (Deliveries stalled by the restart): the pusher's
    /// first look pushes them, unlike the items already waiting
    pub news_at_start: std::collections::HashSet<String>,
    /// When push was found off with a phone registered (RFC 3339), until it's back on: the `push_off` item's id
    /// and age, so a Dismiss lasts for that off period only
    pub push_off_since: Option<String>,
    /// The disk is almost full: Needs you shows `disk_low`, until free space is back over LOW_DISK + LOW_DISK_MARGIN
    pub disk_low: bool,
    /// agent id -> who restarted its run ("user" or "Loft"); the stop notice says so
    pub restarting: HashMap<String, &'static str>,
    /// agent id -> automatic restarts in a row with no output since (agents::restart_quiet)
    pub auto_restarts: HashMap<String, u32>,
    /// agent id -> chats it was @-mentioned in, waiting for the run that answers the mention
    pub mention_pending: HashMap<String, Vec<String>>,
    /// Ready PRs on GitHub that wait on the user (their own, or their review requested), from github_inbox
    pub github_inbox: Vec<crate::github_inbox::GhPr>,
    /// macOS folder requests in Loft's name that wait on an answer (mac_access.rs)
    pub tcc: crate::mac_access::Requests,
    /// The backups folder, its newest backup and the last attempt's error (backup.rs)
    pub backup: crate::backup::Status,
    /// The running or last clean of agents' build output: its log (sweep.rs)
    pub clean: crate::sweep::Log,
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
    /// What's left of GitHub's rate limit, and Loft's requests by source.
    pub gh_budget: Arc<crate::github::Budget>,
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
    /// What `@` file search last listed under each base.
    pub listings: Arc<crate::file_search::Listings>,
    /// The Xcode `simctl` last found to run with.
    pub xcode: Arc<crate::simulator::XcodeCache>,
    /// What Needs you last listed and last told clients (needs_you.rs).
    pub needs_you: Arc<crate::needs_you::Watch>,
}

impl AppState {
    pub fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
    }
}
