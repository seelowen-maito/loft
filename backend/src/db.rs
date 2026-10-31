use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::avatar::Avatar;

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub title: String,
    /// Free-form project or tag; agents sharing one are grouped in the UI.
    #[serde(default)]
    pub project: String,
    pub personality: String,
    pub prompt: String,
    pub model: String,
    pub effort: String,
    /// Minutes between autonomous runs. 0 disables the heartbeat.
    pub interval_minutes: i64,
    /// Windows when autonomous wake-ups are held (e.g. working hours). Empty runs around the clock.
    #[serde(default)]
    pub quiet_hours: Vec<QuietWindow>,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    /// Map of MCP server name -> Claude Code MCP server config.
    pub mcp_servers: Value,
    pub permission_mode: String,
    /// Model for the subagents it spawns, Explore included. Empty uses Claude Code's default.
    #[serde(default)]
    pub subagent_model: String,
    /// Subagents allowed to run at once. None is Claude Code's default; 0 turns subagents off.
    #[serde(default)]
    pub max_subagents: Option<u32>,
    /// The grill instructions are in its Claude session's context. Cleared when the session is replaced
    /// or compacted, so the next grill run sends them again.
    #[serde(default)]
    pub grill_briefed: bool,
    /// The memory part of its system prompt, built when its Session starts and resent unchanged by every
    /// run of that Session, so saving a memory doesn't break the prompt cache. Clear and Compact drop it.
    #[serde(default)]
    pub memory_index: Option<String>,
    /// Leads its team (the agents sharing its project): at most one per project.
    #[serde(default)]
    pub lead: bool,
    /// Its part in the team check at the end of planning: "dev", "reviewer" or "" (neither). Set from
    /// the template it came from; agents saved before roles existed get one inferred once at startup.
    #[serde(default)]
    pub role: String,
    pub avatar: Avatar,
    pub status: String,
    pub status_text: String,
    pub session_id: Option<String>,
    /// Tokens in the Claude session's context as of its last reply. None until a run reports one.
    #[serde(default)]
    pub context_tokens: Option<u64>,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub total_cost_usd: f64,
    /// The running total Claude Code last reported for `session_id`, so a run's cost is what it added.
    #[serde(default)]
    pub session_cost_usd: f64,
    pub created_at: String,
    /// Loft's own agent: always present, and the user can't delete it.
    #[serde(default)]
    pub builtin: bool,
    /// The agent that created this one, if an agent did. None when the user did.
    #[serde(default)]
    pub created_by: Option<String>,
    /// When the last run the user started (by a message or a schedule) ended with a reply or an error.
    #[serde(default)]
    pub replied_at: Option<String>,
    /// The user's read mark: they have seen its chat up to here, in any client.
    #[serde(default)]
    pub read_at: Option<String>,
}

impl Agent {
    /// An Unread reply: a run the user started ended after their read mark.
    pub fn unread_reply(&self) -> bool {
        let at = |t: &Option<String>| t.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok());
        match (at(&self.replied_at), at(&self.read_at)) {
            (Some(r), Some(read)) => r > read,
            (r, _) => r.is_some(),
        }
    }
}

/// A weekly window with no autonomous wake-ups, in this Mac's local time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuietWindow {
    /// Days the window starts on, 0 = Monday … 6 = Sunday.
    pub days: Vec<u8>,
    /// "HH:MM". An end at or before the start runs past midnight; equal times cover the whole day.
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub agent_id: String,
    pub run_id: Option<String>,
    /// user | assistant | system
    pub role: String,
    /// text | thinking | tool_use | tool_result | permission | error | run_start | run_end
    pub kind: String,
    pub content: String,
    pub meta: Value,
    pub created_at: String,
}

/// One entry in a task's thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskComment {
    pub id: i64,
    pub task_id: String,
    /// "user", or the id of the agent that wrote it.
    pub author: String,
    /// comment (someone wrote it) | note (an agent's progress note) | status (a change: started, done, reassigned…)
    pub kind: String,
    pub text: String,
    pub meta: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub agent_id: String,
    /// One line, listed in the agent's system prompt.
    pub title: String,
    /// The full text, read with `recall`.
    pub content: String,
    pub tags: String,
    pub created_at: String,
    /// When `update` last rewrote it; memories never updated (or saved before this existed) have none.
    pub updated_at: Option<String>,
}

/// A title for a memory saved before titles existed: its first line, cut at about 80 characters.
pub fn title_from(content: &str) -> String {
    let line = content.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() <= 80 {
        return line.to_string();
    }
    let cut: String = line.chars().take(80).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head).trim_end();
    format!("{cut}…")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub agent_id: String,
    /// prompt (blocked tool call) | request (agent asked explicitly)
    pub source: String,
    pub tool_name: String,
    pub input: Value,
    pub reason: String,
    /// Suggested rule to add to allowed_tools when granted "always".
    pub rule: String,
    /// pending | allowed_once | allowed_always | denied | expired
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PullRequest {
    pub id: String,
    pub agent_id: String,
    /// owner/repo
    pub slug: String,
    pub number: i64,
    pub url: String,
    /// The tool call that opened it; the chat renders the card right after it.
    pub tool_use_id: Option<String>,
    pub title: String,
    /// open | closed | merged
    pub state: String,
    pub draft: bool,
    /// None while GitHub is still computing it.
    pub mergeable: Option<bool>,
    /// clean | dirty | blocked | behind | unstable | has_hooks | draft | unknown
    pub mergeable_state: String,
    pub merge_methods: Vec<String>,
    pub head_ref: String,
    pub head_sha: String,
    pub base_ref: String,
    pub author: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub commits: i64,
    pub comments: i64,
    /// { total, passed, failed, pending }
    pub checks: Value,
    pub merged_by: Option<String>,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
    pub error: Option<String>,
    pub checked_at: Option<String>,
    pub created_at: String,
    /// GitHub's ETag for the PR, so an unchanged one costs a free 304.
    pub etag: Option<String>,
    /// The head commit `checks` were read for; settled checks there aren't read again.
    pub checks_sha: String,
    /// GitHub answered 404/410: the PR (or its repo) is gone, so it's no longer polled.
    pub gone: bool,
}

/// A Loft issue (stored as a task): a personal todo, or work handed to an agent. It is shaped like a
/// GitHub issue (title, markdown description, labels, owner, open or closed) plus what Loft adds: an
/// agent, an execution status and a thread. Agents report progress on the tasks they own through MCP;
/// the user owns the rest.
///
/// Serialized by hand (below) so the API also carries `state`, derived from `status`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Task {
    pub id: String,
    pub title: String,
    /// Markdown.
    pub description: String,
    /// Label names, as on GitHub. Items stored before labels existed load with none.
    pub labels: Vec<String>,
    /// The GitHub login of the person who owns it, or empty. Always a person, never an agent.
    pub owner: String,
    /// The GitHub issue it mirrors, if any. None: a Loft-only issue.
    pub github: Option<GithubLink>,
    /// todo (personal; never runs by itself) | work (meant for an agent)
    pub kind: String,
    /// inbox | ready | in_progress | blocked | waiting_approval | done | failed | cancelled
    pub status: String,
    /// low | normal | high | urgent
    pub priority: String,
    pub project: String,
    pub agent_id: Option<String>,
    pub parent_id: Option<String>,
    pub depends_on: Vec<String>,
    pub due_at: Option<String>,
    /// "user", or the id of the agent that created it (a delegated subtask).
    pub created_by: String,
    /// Latest progress note from the agent working on it.
    pub note: String,
    pub summary: String,
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

fn read_agent(c: &Connection, id: &str) -> Result<Option<Agent>> {
    let data: Option<String> = c.query_row("SELECT data FROM agents WHERE id = ?1", [id], |r| r.get(0)).optional()?;
    Ok(match data {
        Some(d) => Some(serde_json::from_str(&d)?),
        None => None,
    })
}

fn write_agent(c: &Connection, a: &Agent) -> Result<()> {
    c.execute(
        "INSERT INTO agents (id, data, created_at, position)
         VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position), 0) + 1 FROM agents))
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        params![a.id, serde_json::to_string(a)?, a.created_at],
    )?;
    Ok(())
}

/// Where a mirrored Loft issue lives on GitHub, and what Loft last read of it there.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct GithubLink {
    /// owner/name
    pub repo: String,
    pub number: i64,
    /// GitHub's database id of the issue, which sub-issue and dependency links take.
    pub id: i64,
    pub url: String,
    /// open | closed, as GitHub last said.
    pub state: String,
    /// GitHub's `updated_at` for the issue when Loft last read it.
    pub updated_at: String,
    /// GitHub's own count of its sub-issues (`sub_issues_summary`): all, and the closed ones. Zero when it has none.
    pub sub_total: i64,
    pub sub_done: i64,
}

impl Task {
    /// `open` or `closed`, as on GitHub: done and cancelled are closed, everything else is open.
    pub fn state(&self) -> &'static str {
        if matches!(self.status.as_str(), "done" | "cancelled") { "closed" } else { "open" }
    }
}

impl Serialize for Task {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut m = s.serialize_struct("Task", 26)?;
        m.serialize_field("id", &self.id)?;
        m.serialize_field("title", &self.title)?;
        m.serialize_field("description", &self.description)?;
        m.serialize_field("labels", &self.labels)?;
        m.serialize_field("owner", &self.owner)?;
        m.serialize_field("state", self.state())?;
        m.serialize_field("github", &self.github)?;
        m.serialize_field("kind", &self.kind)?;
        m.serialize_field("status", &self.status)?;
        m.serialize_field("priority", &self.priority)?;
        m.serialize_field("project", &self.project)?;
        m.serialize_field("agent_id", &self.agent_id)?;
        m.serialize_field("parent_id", &self.parent_id)?;
        m.serialize_field("depends_on", &self.depends_on)?;
        m.serialize_field("due_at", &self.due_at)?;
        m.serialize_field("created_by", &self.created_by)?;
        m.serialize_field("note", &self.note)?;
        m.serialize_field("summary", &self.summary)?;
        m.serialize_field("error", &self.error)?;
        m.serialize_field("created_at", &self.created_at)?;
        m.serialize_field("updated_at", &self.updated_at)?;
        m.serialize_field("started_at", &self.started_at)?;
        m.serialize_field("completed_at", &self.completed_at)?;
        m.end()
    }
}

pub fn now() -> String {
    Utc::now().to_rfc3339()
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    run_id TEXT,
    role TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    meta TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_agent ON messages(agent_id, id);
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT
);
CREATE INDEX IF NOT EXISTS memories_agent ON memories(agent_id, id);
CREATE TABLE IF NOT EXISTS permissions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pull_requests (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    number INTEGER NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(agent_id, slug, number)
);
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS grill_sessions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS grill_sessions_agent ON grill_sessions(agent_id, created_at);
CREATE TABLE IF NOT EXISTS push_seen (
    item_id TEXT PRIMARY KEY,
    seen_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS devices (
    token TEXT PRIMARY KEY,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS task_comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    author TEXT NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    meta TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS task_comments_task ON task_comments(task_id, id);
CREATE TABLE IF NOT EXISTS needs_you_dismissed (
    item_id TEXT PRIMARY KEY,
    dismissed_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS published_issues (
    slug TEXT NOT NULL,
    kind TEXT NOT NULL,
    key TEXT NOT NULL,
    number INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(slug, kind, key)
);
CREATE TABLE IF NOT EXISTS pending_fyis (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    prompt TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS pending_fyis_agent ON pending_fyis(agent_id, id);
"#;

/// Run ends saved before #164 hold Claude Code's running total for the session, not the run's own cost.
/// Each one becomes its difference from the previous run end of the same session, keeping the reported
/// total as `session_cost_usd`, which also marks it done; the agent's lifetime total is then recomputed.
/// Old run ends don't name their session: a Resume marker does, and a cleared or fresh session runs
/// unnamed until the next one, except the last, which is the agent's current session.
fn per_run_costs(conn: &Connection) -> Result<()> {
    let agents: Vec<(String, String)> = {
        let mut st = conn.prepare(
            "SELECT id, data FROM agents WHERE id IN (SELECT agent_id FROM messages WHERE kind = 'run_end'
               AND json_extract(meta, '$.cost_usd') IS NOT NULL AND json_extract(meta, '$.session_cost_usd') IS NULL)",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (agent_id, data) in agents {
        let current = serde_json::from_str::<Value>(&data).ok().and_then(|d| d["session_id"].as_str().map(String::from));
        let tx = conn.unchecked_transaction()?;
        let rows: Vec<(i64, String, Value)> = {
            let mut st = tx.prepare(
                "SELECT id, kind, meta FROM messages WHERE agent_id = ?1 AND kind IN ('run_start', 'run_end', 'clear', 'resume') ORDER BY id",
            )?;
            let rows = st
                .query_map([&agent_id], |r| Ok((r.get(0)?, r.get(1)?, serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null))))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        // The session the runs belong to (None while unnamed), its last total, and each named session's last total.
        let mut sid: Option<String> = None;
        let mut prev = 0.0;
        let mut totals: HashMap<String, f64> = HashMap::new();
        let mut unnamed: Vec<(i64, Value)> = vec![];
        for (id, kind, mut meta) in rows {
            match kind.as_str() {
                "clear" => (sid, prev, unnamed) = (None, 0.0, vec![]),
                "run_start" if meta["resumed"] == Value::Bool(false) => (sid, prev, unnamed) = (None, 0.0, vec![]),
                "resume" => {
                    sid = meta["session_id"].as_str().map(String::from);
                    prev = sid.as_ref().and_then(|s| totals.get(s)).copied().unwrap_or(0.0);
                    unnamed.clear();
                }
                "run_end" => {
                    // A stopped run reported nothing.
                    let Some(total) = meta["cost_usd"].as_f64() else { continue };
                    if let Some(total) = meta["session_cost_usd"].as_f64() {
                        prev = total;
                    } else {
                        meta["cost_usd"] = run_cost(total, prev).into();
                        meta["session_cost_usd"] = total.into();
                        match &sid {
                            Some(s) => meta["session_id"] = s.as_str().into(),
                            None => unnamed.push((id, meta.clone())),
                        }
                        tx.execute("UPDATE messages SET meta = ?1 WHERE id = ?2", params![meta.to_string(), id])?;
                        prev = total;
                    }
                    if let Some(s) = &sid {
                        totals.insert(s.clone(), prev);
                    }
                }
                _ => {}
            }
        }
        if let (None, Some(cur)) = (&sid, &current) {
            for (id, mut meta) in unnamed {
                meta["session_id"] = cur.as_str().into();
                tx.execute("UPDATE messages SET meta = ?1 WHERE id = ?2", params![meta.to_string(), id])?;
            }
        }
        tx.execute(
            "UPDATE agents SET data = json_set(data, '$.total_cost_usd',
                 (SELECT COALESCE(sum(json_extract(meta, '$.cost_usd')), 0.0) FROM messages WHERE agent_id = ?1 AND kind = 'run_end'),
                 '$.session_cost_usd', ?2)
             WHERE id = ?1",
            params![agent_id, prev],
        )?;
        tx.commit()?;
    }
    Ok(())
}

/// What a run added to its session, given the running total it reported and the one before it.
/// A total that went down is a fresh session, so all of it is this run's.
pub fn run_cost(total: f64, prev: f64) -> f64 {
    if total >= prev { total - prev } else { total }
}

fn message_from_row(r: &Row) -> rusqlite::Result<Message> {
    let meta: String = r.get(6)?;
    Ok(Message {
        id: r.get(0)?,
        agent_id: r.get(1)?,
        run_id: r.get(2)?,
        role: r.get(3)?,
        kind: r.get(4)?,
        content: r.get(5)?,
        meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
        created_at: r.get(7)?,
    })
}

fn memory_from_row(r: &Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: r.get(0)?,
        agent_id: r.get(1)?,
        title: r.get(2)?,
        content: r.get(3)?,
        tags: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        // Roster order the user sets by dragging; kept out of `data` so status saves can't clobber it.
        let has_position: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('agents') WHERE name = 'position'", [], |r| r.get(0))?;
        if !has_position {
            conn.execute("ALTER TABLE agents ADD COLUMN position INTEGER NOT NULL DEFAULT 0", [])?;
        }
        // A grill's status as its own column, so finding the open ones is an indexed lookup instead of
        // parsing every session. Sessions saved before it get theirs from `data`, once.
        let has_status: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('grill_sessions') WHERE name = 'status'", [], |r| r.get(0))?;
        if !has_status {
            conn.execute("ALTER TABLE grill_sessions ADD COLUMN status TEXT NOT NULL DEFAULT ''", [])?;
            conn.execute("UPDATE grill_sessions SET status = COALESCE(json_extract(data, '$.status'), '')", [])?;
        }
        // Memories saved before titles existed get their first line as title, once; the text stays the body.
        let has_title: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'title'", [], |r| r.get(0))?;
        if !has_title {
            conn.execute("ALTER TABLE memories ADD COLUMN title TEXT NOT NULL DEFAULT ''", [])?;
            let old: Vec<(i64, String)> = {
                let mut st = conn.prepare("SELECT id, content FROM memories")?;
                let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            for (id, content) in old {
                conn.execute("UPDATE memories SET title = ?1 WHERE id = ?2", params![title_from(&content), id])?;
            }
        }
        // Memories saved before updates were timed have none, so they keep their creation order.
        let has_updated: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'updated_at'", [], |r| r.get(0))?;
        if !has_updated {
            conn.execute("ALTER TABLE memories ADD COLUMN updated_at TEXT", [])?;
        }
        // FYIs used to wait in the agent's `data`; move any still there to their table, once.
        conn.execute_batch(
            "BEGIN;
             INSERT INTO pending_fyis (agent_id, prompt, created_at)
                 SELECT a.id, j.value, a.created_at FROM agents a, json_each(a.data, '$.pending_fyis') j ORDER BY a.id, j.key;
             UPDATE agents SET data = json_remove(data, '$.pending_fyis') WHERE json_type(data, '$.pending_fyis') IS NOT NULL;
             COMMIT;",
        )?;
        per_run_costs(&conn)?;
        conn.execute("CREATE INDEX IF NOT EXISTS grill_sessions_open ON grill_sessions(agent_id, created_at) WHERE status <> 'ended'", [])?;
        // Agents saved before roles existed get one from their name and title, once; the user can change it.
        let unroled: Vec<String> = {
            let mut st = conn.prepare("SELECT data FROM agents")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter().filter(|d| serde_json::from_str::<Value>(d).is_ok_and(|v| v.get("role").is_none())).collect()
        };
        // Anything left "running" from a previous backend process is stale.
        let db = Db(Arc::new(Mutex::new(conn)));
        for d in unroled {
            if let Ok(mut a) = serde_json::from_str::<Agent>(&d) {
                a.role = crate::planning::infer_role(&a.name, &a.title).into();
                db.save_agent(&a)?;
            }
        }
        for mut a in db.list_agents()? {
            if a.status != "idle" {
                a.status = "idle".into();
                a.status_text = String::new();
                db.save_agent(&a)?;
            }
        }
        for mut p in db.list_permissions(None)? {
            if p.status == "pending" {
                p.status = "expired".into();
                db.save_permission(&p)?;
            }
        }
        Ok(db)
    }

    // ---- agents -------------------------------------------------------

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT data FROM agents ORDER BY position, created_at")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for r in rows {
            out.push(serde_json::from_str(&r?)?);
        }
        Ok(out)
    }

    pub fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        read_agent(&self.0.lock().unwrap(), id)
    }

    pub fn save_agent(&self, a: &Agent) -> Result<()> {
        write_agent(&self.0.lock().unwrap(), a)
    }

    /// Read-modify-write an agent under one hold of the connection lock, so concurrent updates
    /// (a status write and a context-token write, say) can't overwrite each other. `f` must not use the db.
    pub fn update_agent(&self, id: &str, f: impl FnOnce(&mut Agent)) -> Result<Option<Agent>> {
        let c = self.0.lock().unwrap();
        let Some(mut a) = read_agent(&c, id)? else {
            return Ok(None);
        };
        f(&mut a);
        write_agent(&c, &a)?;
        Ok(Some(a))
    }

    /// The running total Claude Code last reported for one of the agent's sessions; 0 when none of its runs was recorded.
    pub fn session_total(&self, agent_id: &str, session_id: &str) -> Result<f64> {
        let c = self.0.lock().unwrap();
        Ok(c.query_row(
            "SELECT json_extract(meta, '$.session_cost_usd') FROM messages
             WHERE agent_id = ?1 AND kind = 'run_end' AND json_extract(meta, '$.session_id') = ?2
             ORDER BY id DESC LIMIT 1",
            params![agent_id, session_id],
            |r| r.get::<_, Option<f64>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or(0.0))
    }

    /// Queue an FYI's prompt text for the agent's next run.
    pub fn add_pending_fyi(&self, agent_id: &str, prompt: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("INSERT INTO pending_fyis (agent_id, prompt, created_at) VALUES (?1, ?2, ?3)", params![agent_id, prompt, now()])?;
        Ok(())
    }

    /// Remove and return the agent's pending FYIs, oldest first, as (id, prompt).
    pub fn take_pending_fyis(&self, agent_id: &str) -> Result<Vec<(i64, String)>> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        let fyis = {
            let mut st = tx.prepare("SELECT id, prompt FROM pending_fyis WHERE agent_id = ?1 ORDER BY id")?;
            let rows = st.query_map([agent_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.execute("DELETE FROM pending_fyis WHERE agent_id = ?1", [agent_id])?;
        tx.commit()?;
        Ok(fyis)
    }

    /// Put taken FYIs back under their old ids, so they stay ahead of any that arrived since.
    pub fn restore_pending_fyis(&self, agent_id: &str, fyis: &[(i64, String)]) -> Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        for (id, prompt) in fyis {
            tx.execute("INSERT OR IGNORE INTO pending_fyis (id, agent_id, prompt, created_at) VALUES (?1, ?2, ?3, ?4)", params![id, agent_id, prompt, now()])?;
        }
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn count_pending_fyis(&self, agent_id: &str) -> Result<usize> {
        let c = self.0.lock().unwrap();
        Ok(c.query_row("SELECT COUNT(*) FROM pending_fyis WHERE agent_id = ?1", [agent_id], |r| r.get::<_, i64>(0))? as usize)
    }

    /// Store the roster order. Ids not listed keep their position.
    pub fn reorder_agents(&self, ids: &[String]) -> Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute("UPDATE agents SET position = ?1 WHERE id = ?2", params![i as i64, id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_agent(&self, id: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("DELETE FROM agents WHERE id = ?1", [id])?;
        c.execute("DELETE FROM messages WHERE agent_id = ?1", [id])?;
        c.execute("DELETE FROM memories WHERE agent_id = ?1", [id])?;
        c.execute("DELETE FROM permissions WHERE agent_id = ?1", [id])?;
        c.execute("DELETE FROM pull_requests WHERE agent_id = ?1", [id])?;
        c.execute("DELETE FROM pending_fyis WHERE agent_id = ?1", [id])?;
        Ok(())
    }

    // ---- tasks --------------------------------------------------------

    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT data FROM tasks ORDER BY created_at DESC")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for r in rows {
            out.push(serde_json::from_str(&r?)?);
        }
        Ok(out)
    }

    /// An agent's grill sessions, newest first.
    pub fn list_grills(&self, agent_id: &str) -> Result<Vec<crate::grill::Session>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT data FROM grill_sessions WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT 20")?;
        let rows = st.query_map([agent_id], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for r in rows {
            out.push(serde_json::from_str(&r?)?);
        }
        Ok(out)
    }

    /// An agent's newest grill session that hasn't ended.
    pub fn active_grill(&self, agent_id: &str) -> Result<Option<crate::grill::Session>> {
        let c = self.0.lock().unwrap();
        let data: Option<String> = c
            .query_row("SELECT data FROM grill_sessions WHERE agent_id = ?1 AND status <> 'ended' ORDER BY created_at DESC LIMIT 1", [agent_id], |r| r.get(0))
            .optional()?;
        Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
    }

    /// Grill sessions that haven't ended, for the idle timeouts.
    pub fn open_grills(&self) -> Result<Vec<crate::grill::Session>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT data FROM grill_sessions WHERE status <> 'ended'")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for r in rows {
            out.push(serde_json::from_str(&r?)?);
        }
        Ok(out)
    }

    pub fn get_grill(&self, id: &str) -> Result<Option<crate::grill::Session>> {
        let c = self.0.lock().unwrap();
        let data: Option<String> = c.query_row("SELECT data FROM grill_sessions WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
    }

    pub fn save_grill(&self, s: &crate::grill::Session) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT INTO grill_sessions (id, agent_id, data, created_at, status) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, status = excluded.status",
            params![s.id, s.agent_id, serde_json::to_string(s)?, s.created_at, s.status],
        )?;
        Ok(())
    }

    /// The issue Loft already published on `slug` for this kind (`spec`, …) and key, if any.
    pub fn published_issue(&self, slug: &str, kind: &str, key: &str) -> Result<Option<i64>> {
        let c = self.0.lock().unwrap();
        Ok(c.query_row("SELECT number FROM published_issues WHERE slug = ?1 AND kind = ?2 AND key = ?3", params![slug, kind, key], |r| r.get(0)).optional()?)
    }

    pub fn record_published_issue(&self, slug: &str, kind: &str, key: &str, number: i64) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT OR REPLACE INTO published_issues (slug, kind, key, number, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![slug, kind, key, number, now()],
        )?;
        Ok(())
    }

    pub fn forget_published_issue(&self, slug: &str, kind: &str, key: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("DELETE FROM published_issues WHERE slug = ?1 AND kind = ?2 AND key = ?3", params![slug, kind, key])?;
        Ok(())
    }

    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let c = self.0.lock().unwrap();
        let data: Option<String> = c.query_row("SELECT data FROM tasks WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
    }

    /// A task row exactly as given, for tests that start from data an older Loft wrote.
    #[cfg(test)]
    pub(crate) fn insert_raw_task(&self, id: &str, data: &str, created_at: &str) {
        let c = self.0.lock().unwrap();
        c.execute("INSERT INTO tasks (id, data, created_at) VALUES (?1, ?2, ?3)", params![id, data, created_at]).unwrap();
    }

    pub fn save_task(&self, t: &Task) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT INTO tasks (id, data, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            params![t.id, serde_json::to_string(t)?, t.created_at],
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        c.execute("DELETE FROM task_comments WHERE task_id = ?1", [id])?;
        Ok(())
    }

    pub fn add_task_comment(&self, task_id: &str, author: &str, kind: &str, text: &str, meta: Value) -> Result<TaskComment> {
        let c = self.0.lock().unwrap();
        let created_at = now();
        c.execute(
            "INSERT INTO task_comments (task_id, author, kind, text, meta, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![task_id, author, kind, text, meta.to_string(), created_at],
        )?;
        Ok(TaskComment { id: c.last_insert_rowid(), task_id: task_id.into(), author: author.into(), kind: kind.into(), text: text.into(), meta, created_at })
    }

    /// Thread entries by a query on their columns: `WHERE {filter}` with `args`, oldest first.
    fn task_comments_where(&self, filter: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<TaskComment>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(&format!("SELECT id, task_id, author, kind, text, meta, created_at FROM task_comments WHERE {filter} ORDER BY id"))?;
        let rows = st
            .query_map(args, |r| {
                let meta: String = r.get(5)?;
                Ok(TaskComment { id: r.get(0)?, task_id: r.get(1)?, author: r.get(2)?, kind: r.get(3)?, text: r.get(4)?, meta: serde_json::from_str(&meta).unwrap_or(Value::Null), created_at: r.get(6)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_task_comment(&self, id: i64) -> Result<Option<TaskComment>> {
        Ok(self.task_comments_where("id = ?1", &[&id])?.pop())
    }

    /// Comments whose GitHub state (`meta.github.state`) is this, across every thread: one read.
    pub fn task_comments_on_github(&self, github_state: &str) -> Result<Vec<TaskComment>> {
        self.task_comments_where("json_extract(meta, '$.github.state') = ?1", &[&github_state])
    }

    /// GitHub comment ids already in a thread, so a GitHub comment is added once and Loft's own aren't echoed.
    pub fn github_comment_ids(&self, task_id: &str) -> Result<Vec<i64>> {
        Ok(self.task_comments_where("task_id = ?1 AND json_extract(meta, '$.github.comment_id') IS NOT NULL", &[&task_id])?.iter().filter_map(|c| c.meta["github"]["comment_id"].as_i64()).collect())
    }

    pub fn update_task_comment(&self, id: i64, text: &str, meta: &Value) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("UPDATE task_comments SET text = ?2, meta = ?3 WHERE id = ?1", params![id, text, meta.to_string()])?;
        Ok(())
    }

    /// The newest `limit` entries (older than `before`, an id, when paging), oldest first.
    pub fn list_task_comments(&self, task_id: &str, limit: i64, before: Option<i64>) -> Result<Vec<TaskComment>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(
            "SELECT id, task_id, author, kind, text, meta, created_at FROM task_comments
             WHERE task_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let mut rows = st
            .query_map(params![task_id, before.unwrap_or(i64::MAX), limit], |r| {
                let meta: String = r.get(5)?;
                Ok(TaskComment {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    author: r.get(2)?,
                    kind: r.get(3)?,
                    text: r.get(4)?,
                    meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
                    created_at: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.reverse();
        Ok(rows)
    }

    /// Which of these "needs you" item ids were already considered for push.
    pub fn push_seen_among(&self, ids: &[&str]) -> Result<std::collections::HashSet<String>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let c = self.0.lock().unwrap();
        let marks = vec!["?"; ids.len()].join(",");
        let mut st = c.prepare(&format!("SELECT item_id FROM push_seen WHERE item_id IN ({marks})"))?;
        let rows = st.query_map(rusqlite::params_from_iter(ids), |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows.into_iter().collect())
    }

    /// Forgets push marks older than 30 days; returns how many.
    pub fn prune_push_seen(&self) -> Result<usize> {
        let c = self.0.lock().unwrap();
        let cutoff = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        Ok(c.execute("DELETE FROM push_seen WHERE seen_at < ?1", [&cutoff])?)
    }

    /// Raw access for tests elsewhere in the crate that need to set up rows directly.
    #[cfg(test)]
    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().unwrap()
    }

    pub fn mark_push_seen(&self, item_id: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("INSERT OR IGNORE INTO push_seen (item_id, seen_at) VALUES (?1, ?2)", params![item_id, now()])?;
        Ok(())
    }

    pub fn list_devices(&self) -> Result<Vec<crate::apns::Device>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT data FROM devices ORDER BY created_at")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter_map(|d| serde_json::from_str(&d).ok()).collect())
    }

    pub fn get_device(&self, token: &str) -> Result<Option<crate::apns::Device>> {
        let c = self.0.lock().unwrap();
        let data: Option<String> = c.query_row("SELECT data FROM devices WHERE token = ?1", [token], |r| r.get(0)).optional()?;
        Ok(data.and_then(|d| serde_json::from_str(&d).ok()))
    }

    pub fn save_device(&self, d: &crate::apns::Device) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT INTO devices (token, data, created_at) VALUES (?1, ?2, ?3) ON CONFLICT(token) DO UPDATE SET data = excluded.data",
            params![d.token, serde_json::to_string(d)?, d.created_at],
        )?;
        Ok(())
    }

    pub fn delete_device(&self, token: &str) -> Result<bool> {
        let c = self.0.lock().unwrap();
        Ok(c.execute("DELETE FROM devices WHERE token = ?1", [token])? > 0)
    }


    /// "Needs you" items the user dismissed in the last 30 days (older ones are forgotten).
    pub fn dismissed_needs_you(&self) -> Result<Vec<String>> {
        let c = self.0.lock().unwrap();
        let cutoff = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        c.execute("DELETE FROM needs_you_dismissed WHERE dismissed_at < ?1", [&cutoff])?;
        let mut st = c.prepare("SELECT item_id FROM needs_you_dismissed")?;
        let rows = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    pub fn dismiss_needs_you(&self, item_id: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("INSERT OR REPLACE INTO needs_you_dismissed (item_id, dismissed_at) VALUES (?1, ?2)", params![item_id, now()])?;
        Ok(())
    }

    /// A comment previously posted with this client id, so a retried post isn't added twice.
    pub fn find_task_comment_by_client(&self, task_id: &str, client_id: &str) -> Result<Option<i64>> {
        let c = self.0.lock().unwrap();
        Ok(c
            .query_row(
                "SELECT id FROM task_comments WHERE task_id = ?1 AND json_extract(meta, '$.client_id') = ?2",
                params![task_id, client_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// A user message previously sent with this client id, so a retried send isn't delivered twice.
    pub fn find_client_message(&self, agent_id: &str, client_id: &str) -> Result<Option<Message>> {
        let c = self.0.lock().unwrap();
        Ok(c.query_row(
            "SELECT id, agent_id, run_id, role, kind, content, meta, created_at FROM messages
             WHERE agent_id = ?1 AND role = 'user' AND json_extract(meta, '$.client_id') = ?2 LIMIT 1",
            params![agent_id, client_id],
            message_from_row,
        )
        .optional()?)
    }

    /// Messages across all agents that make up the activity feed, newest first.
    pub fn activity_messages(&self, limit: i64, before: Option<&str>) -> Result<Vec<Message>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(
            "SELECT id, agent_id, run_id, role, kind, content, meta, created_at FROM messages
             WHERE created_at < ?1
               AND (kind IN ('run_start', 'run_end', 'error', 'permission')
                    OR (role = 'user' AND kind = 'text' AND json_extract(meta, '$.from_agent') IS NOT NULL))
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![before.unwrap_or("9999"), limit], message_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // ---- messages -----------------------------------------------------

    pub fn add_message(
        &self,
        agent_id: &str,
        run_id: Option<&str>,
        role: &str,
        kind: &str,
        content: &str,
        meta: Value,
    ) -> Result<Message> {
        let c = self.0.lock().unwrap();
        let created_at = now();
        c.execute(
            "INSERT INTO messages (agent_id, run_id, role, kind, content, meta, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![agent_id, run_id, role, kind, content, meta.to_string(), created_at],
        )?;
        Ok(Message {
            id: c.last_insert_rowid(),
            agent_id: agent_id.into(),
            run_id: run_id.map(Into::into),
            role: role.into(),
            kind: kind.into(),
            content: content.into(),
            meta,
            created_at,
        })
    }

    pub fn get_message(&self, agent_id: &str, id: i64) -> Result<Option<Message>> {
        let c = self.0.lock().unwrap();
        Ok(c.query_row(
            "SELECT id, agent_id, run_id, role, kind, content, meta, created_at FROM messages WHERE agent_id = ?1 AND id = ?2",
            params![agent_id, id],
            message_from_row,
        )
        .optional()?)
    }

    /// The `run_start` marker of run `run_id`, before message `before`: its meta says what started the run.
    /// Walks back from `before`, so it costs the run's length, not the agent's history.
    pub fn run_start(&self, agent_id: &str, run_id: &str, before: i64) -> Result<Option<Message>> {
        let c = self.0.lock().unwrap();
        Ok(c.query_row(
            "SELECT id, agent_id, run_id, role, kind, content, meta, created_at FROM messages
             WHERE agent_id = ?1 AND id < ?3 AND run_id = ?2 AND kind = 'run_start' ORDER BY id DESC LIMIT 1",
            params![agent_id, run_id, before],
            message_from_row,
        )
        .optional()?)
    }

    /// Marks an end-of-planning card's team setup as sent, once: false when the card has no setup or
    /// it was already sent, so two clicks can't both send it.
    pub fn mark_team_setup_sent(&self, agent_id: &str, id: i64) -> Result<bool> {
        let c = self.0.lock().unwrap();
        let n = c.execute(
            "UPDATE messages SET meta = json_set(meta, '$.grill.end_card.setup.sent_at', ?3)
             WHERE agent_id = ?1 AND id = ?2 AND json_type(meta, '$.grill.end_card.setup') = 'object'
               AND json_extract(meta, '$.grill.end_card.setup.sent_at') IS NULL",
            params![agent_id, id, now()],
        )?;
        Ok(n == 1)
    }

    /// Most recent `limit` messages, returned oldest-first. `before` pages backwards.
    pub fn list_messages(&self, agent_id: &str, limit: i64, before: Option<i64>) -> Result<Vec<Message>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(
            "SELECT id, agent_id, run_id, role, kind, content, meta, created_at FROM messages
             WHERE agent_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = st.query_map(params![agent_id, before.unwrap_or(i64::MAX), limit], message_from_row)?;
        let mut out: Vec<Message> = rows.collect::<Result<_, _>>()?;
        out.reverse();
        Ok(out)
    }

    pub fn clear_messages(&self, agent_id: &str) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("DELETE FROM messages WHERE agent_id = ?1", [agent_id])?;
        Ok(())
    }

    // ---- memories -----------------------------------------------------

    pub fn add_memory(&self, agent_id: &str, title: &str, content: &str, tags: &str) -> Result<Memory> {
        let c = self.0.lock().unwrap();
        let created_at = now();
        c.execute(
            "INSERT INTO memories (agent_id, title, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![agent_id, title, content, tags, created_at],
        )?;
        Ok(Memory {
            id: c.last_insert_rowid(),
            agent_id: agent_id.into(),
            title: title.into(),
            content: content.into(),
            tags: tags.into(),
            created_at,
            updated_at: None,
        })
    }

    /// Oldest first by when each was last saved or updated, so an updated memory counts as new.
    pub fn list_memories(&self, agent_id: &str) -> Result<Vec<Memory>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(
            "SELECT id, agent_id, title, content, tags, created_at, updated_at FROM memories WHERE agent_id = ?1 ORDER BY coalesce(updated_at, created_at), id",
        )?;
        let rows = st.query_map([agent_id], memory_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn search_memories(&self, agent_id: &str, query: &str) -> Result<Vec<Memory>> {
        let c = self.0.lock().unwrap();
        let like = format!("%{}%", query.to_lowercase());
        let mut st = c.prepare(
            "SELECT id, agent_id, title, content, tags, created_at, updated_at FROM memories
             WHERE agent_id = ?1 AND (lower(title) LIKE ?2 OR lower(content) LIKE ?2 OR lower(tags) LIKE ?2) ORDER BY coalesce(updated_at, created_at), id",
        )?;
        let rows = st.query_map(params![agent_id, like], memory_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Rewrites the title and/or body of one of the agent's memories; `None` keeps that part.
    /// Returns `None` when the agent has no memory with that id.
    pub fn update_memory(&self, agent_id: &str, id: i64, title: Option<&str>, content: Option<&str>) -> Result<Option<Memory>> {
        let c = self.0.lock().unwrap();
        let n = c.execute(
            "UPDATE memories SET title = coalesce(?3, title), content = coalesce(?4, content), updated_at = ?5 WHERE agent_id = ?1 AND id = ?2",
            params![agent_id, id, title, content, now()],
        )?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(c.query_row(
            "SELECT id, agent_id, title, content, tags, created_at, updated_at FROM memories WHERE id = ?1",
            [id],
            memory_from_row,
        )?))
    }

    pub fn delete_memory(&self, agent_id: &str, id: i64) -> Result<bool> {
        let c = self.0.lock().unwrap();
        Ok(c.execute("DELETE FROM memories WHERE agent_id = ?1 AND id = ?2", params![agent_id, id])? > 0)
    }

    // ---- permissions --------------------------------------------------

    pub fn save_permission(&self, p: &PermissionRequest) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT INTO permissions (id, agent_id, data, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            params![p.id, p.agent_id, serde_json::to_string(p)?, p.created_at],
        )?;
        Ok(())
    }

    pub fn get_permission(&self, id: &str) -> Result<Option<PermissionRequest>> {
        let c = self.0.lock().unwrap();
        let data: Option<String> = c
            .query_row("SELECT data FROM permissions WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        Ok(match data {
            Some(d) => Some(serde_json::from_str(&d)?),
            None => None,
        })
    }

    pub fn list_permissions(&self, agent_id: Option<&str>) -> Result<Vec<PermissionRequest>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(
            "SELECT data FROM permissions WHERE (?1 IS NULL OR agent_id = ?1) ORDER BY created_at DESC LIMIT 200",
        )?;
        let rows = st.query_map([agent_id], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for r in rows {
            out.push(serde_json::from_str(&r?)?);
        }
        Ok(out)
    }

    // ---- pull requests ------------------------------------------------

    pub fn save_pull_request(&self, p: &PullRequest) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT INTO pull_requests (id, agent_id, slug, number, data, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            params![p.id, p.agent_id, p.slug, p.number, serde_json::to_string(p)?, p.created_at],
        )?;
        Ok(())
    }

    pub fn get_pull_request(&self, id: &str) -> Result<Option<PullRequest>> {
        let c = self.0.lock().unwrap();
        let data: Option<String> = c
            .query_row("SELECT data FROM pull_requests WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
    }

    pub fn find_pull_request(&self, agent_id: &str, slug: &str, number: i64) -> Result<Option<PullRequest>> {
        let c = self.0.lock().unwrap();
        let data: Option<String> = c
            .query_row(
                "SELECT data FROM pull_requests WHERE agent_id = ?1 AND slug = ?2 AND number = ?3",
                params![agent_id, slug, number],
                |r| r.get(0),
            )
            .optional()?;
        Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
    }

    pub fn list_pull_requests(&self, agent_id: Option<&str>) -> Result<Vec<PullRequest>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(
            "SELECT data FROM pull_requests WHERE (?1 IS NULL OR agent_id = ?1) ORDER BY created_at DESC",
        )?;
        let rows = st.query_map([agent_id], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for r in rows {
            out.push(serde_json::from_str(&r?)?);
        }
        Ok(out)
    }

    /// Tool calls that may have opened a PR, paired with their results.
    pub fn pr_tool_uses(&self) -> Result<Vec<(Message, Message)>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(
            "SELECT u.id, u.agent_id, u.run_id, u.role, u.kind, u.content, u.meta, u.created_at,
                    r.id, r.agent_id, r.run_id, r.role, r.kind, r.content, r.meta, r.created_at
             FROM messages u JOIN messages r
               ON r.agent_id = u.agent_id AND r.kind = 'tool_result'
              AND json_extract(r.meta, '$.tool_use_id') = json_extract(u.meta, '$.id')
             WHERE u.kind = 'tool_use'
               AND (u.meta LIKE '%gh pr create%' OR u.meta LIKE '%/pulls%' OR u.meta LIKE '%pull_request%')
             ORDER BY u.id",
        )?;
        let rows = st.query_map([], |r| {
            let m = |o: usize| -> rusqlite::Result<Message> {
                let meta: String = r.get(o + 6)?;
                Ok(Message {
                    id: r.get(o)?,
                    agent_id: r.get(o + 1)?,
                    run_id: r.get(o + 2)?,
                    role: r.get(o + 3)?,
                    kind: r.get(o + 4)?,
                    content: r.get(o + 5)?,
                    meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
                    created_at: r.get(o + 7)?,
                })
            };
            Ok((m(0)?, m(8)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }


    // ---- activity summary ---------------------------------------------

    /// Raw activity for one agent since `since` (RFC 3339), for the dashboard.
    pub fn agent_activity(&self, agent_id: &str, since: &str) -> Result<Value> {
        let c = self.0.lock().unwrap();
        let mut runs = vec![];
        {
            let mut st = c.prepare(
                "SELECT created_at, COALESCE(json_extract(meta, '$.trigger'), 'user') FROM messages
                 WHERE agent_id = ?1 AND kind = 'run_start' AND created_at >= ?2",
            )?;
            for r in st.query_map(params![agent_id, since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (at, trigger) = r?;
                runs.push(serde_json::json!({ "at": at, "trigger": trigger }));
            }
        }
        let count = |kind: &str| -> rusqlite::Result<i64> {
            c.query_row(
                "SELECT count(*) FROM messages WHERE agent_id = ?1 AND kind = ?2 AND created_at >= ?3",
                params![agent_id, kind, since],
                |r| r.get(0),
            )
        };
        let tool_calls = count("tool_use")?;
        let user_messages: i64 = c.query_row(
            "SELECT count(*) FROM messages WHERE agent_id = ?1 AND role = 'user' AND json_extract(meta, '$.from_agent') IS NULL AND created_at >= ?2",
            params![agent_id, since],
            |r| r.get(0),
        )?;
        let cost: f64 = c.query_row(
            "SELECT COALESCE(sum(json_extract(meta, '$.cost_usd')), 0) FROM messages
             WHERE agent_id = ?1 AND kind = 'run_end' AND created_at >= ?2",
            params![agent_id, since],
            |r| r.get(0),
        )?;
        let mut top_tools = vec![];
        {
            let mut st = c.prepare(
                "SELECT json_extract(meta, '$.name') AS n, count(*) AS k FROM messages
                 WHERE agent_id = ?1 AND kind = 'tool_use' AND created_at >= ?2
                   AND n IS NOT NULL AND n != 'ToolSearch'
                 GROUP BY n ORDER BY k DESC LIMIT 4",
            )?;
            for r in st.query_map(params![agent_id, since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (n, k) = r?;
                top_tools.push(serde_json::json!([n, k]));
            }
        }
        let last_reply: Option<(String, String)> = c
            .query_row(
                "SELECT content, created_at FROM messages
                 WHERE agent_id = ?1 AND role = 'assistant' AND kind = 'text'
                 ORDER BY id DESC LIMIT 1",
                [agent_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        // The latest message (ignoring run bookkeeping), if it was an error.
        let last_error: Option<String> = c
            .query_row(
                "SELECT kind, content FROM messages
                 WHERE agent_id = ?1 AND kind NOT IN ('run_start', 'run_end')
                 ORDER BY id DESC LIMIT 1",
                [agent_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
            .and_then(|(kind, content)| (kind == "error").then_some(content));
        let memories: i64 = c.query_row("SELECT count(*) FROM memories WHERE agent_id = ?1", [agent_id], |r| r.get(0))?;
        Ok(serde_json::json!({
            "runs": runs,
            "tool_calls": tool_calls,
            "last_error": last_error,
            "user_messages": user_messages,
            "cost_usd": cost,
            "top_tools": top_tools,
            "last_reply": last_reply.map(|(text, at)| serde_json::json!({ "text": text, "at": at })),
            "memories": memories,
        }))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "title": "", "personality": "", "prompt": "", "model": "sonnet", "effort": "",
            "interval_minutes": 0, "allowed_tools": [], "disallowed_tools": [], "mcp_servers": {},
            "permission_mode": "auto", "avatar": { "svg": "" }, "status": "idle", "status_text": "",
            "session_id": null, "last_run_at": null, "next_run_at": null, "total_cost_usd": 0.0, "created_at": now(),
        }))
        .unwrap()
    }

    #[test]
    fn stored_running_totals_become_per_run_costs_once() {
        let path = std::env::temp_dir().join(format!("loft-costs-{}.db", uuid::Uuid::new_v4()));
        {
            let db = Db::open(&path).unwrap();
            let mut a = agent("a");
            a.session_id = Some("Z".into());
            db.save_agent(&a).unwrap();
            // As runs before #164 stored them: each run end holds the session's running total.
            let run = |resumed: bool, cost: Option<f64>| {
                db.add_message("a", None, "system", "run_start", "user", serde_json::json!({ "resumed": resumed })).unwrap();
                let end = cost.map_or(serde_json::json!({ "stopped": true }), |c| serde_json::json!({ "cost_usd": c }));
                db.add_message("a", None, "system", "run_end", "", end).unwrap();
            };
            let marker = |kind: &str, sid: Option<&str>| {
                db.add_message("a", None, "system", kind, "", serde_json::json!({ "session_id": sid })).unwrap();
            };
            run(false, Some(0.005));
            run(true, Some(0.006));
            run(true, None);
            run(true, Some(0.007));
            run(false, Some(0.002)); // a fresh session
            marker("clear", None);
            run(false, Some(0.004));
            marker("resume", Some("X")); // never run here: its whole total counts
            run(true, Some(0.010));
            run(true, Some(0.012));
            marker("resume", Some("Y"));
            run(true, Some(0.003));
            marker("resume", Some("X")); // back to X: from its last total
            run(true, Some(0.015));
            marker("clear", None);
            run(false, Some(0.002)); // the agent's current session, Z
        }
        let costs = |db: &Db| -> Vec<f64> {
            let mut m = db.list_messages("a", 100, None).unwrap();
            m.sort_by_key(|m| m.id);
            m.iter().filter_map(|m| (m.kind == "run_end").then(|| m.meta["cost_usd"].as_f64()).flatten()).map(|c| (c * 1e6).round() / 1e6).collect()
        };
        let want = vec![0.005, 0.001, 0.001, 0.002, 0.004, 0.010, 0.002, 0.003, 0.003, 0.002];
        let db = Db::open(&path).unwrap();
        assert_eq!(costs(&db), want);
        let a = db.get_agent("a").unwrap().unwrap();
        assert!((a.total_cost_usd - 0.033).abs() < 1e-9, "{}", a.total_cost_usd);
        assert_eq!(a.session_cost_usd, 0.002, "the next run of the session counts from its last total");
        // Named sessions keep their last total for a later Resume.
        assert_eq!(db.session_total("a", "X").unwrap(), 0.015);
        assert_eq!(db.session_total("a", "Z").unwrap(), 0.002);
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(costs(&db), want, "running it again changes nothing");
        assert!((db.get_agent("a").unwrap().unwrap().total_cost_usd - 0.033).abs() < 1e-9);
    }

    #[test]
    fn fyis_kept_in_agent_data_move_to_their_table_in_order() {
        let path = std::env::temp_dir().join(format!("loft-fyis-{}.db", uuid::Uuid::new_v4()));
        {
            let db = Db::open(&path).unwrap();
            db.save_agent(&agent("a")).unwrap();
            db.save_agent(&agent("b")).unwrap();
            // As the branch before #117 stored them.
            let c = db.0.lock().unwrap();
            c.execute("UPDATE agents SET data = json_set(data, '$.pending_fyis', json('[\"first\",\"second\"]')) WHERE id = 'a'", []).unwrap();
        }
        let db = Db::open(&path).unwrap();
        db.add_pending_fyi("a", "third").unwrap();
        let prompts = |v: Vec<(i64, String)>| v.into_iter().map(|(_, p)| p).collect::<Vec<_>>();
        assert_eq!(prompts(db.take_pending_fyis("a").unwrap()), ["first", "second", "third"]);
        assert!(db.take_pending_fyis("b").unwrap().is_empty());
        let data: String = db.0.lock().unwrap().query_row("SELECT data FROM agents WHERE id = 'a'", [], |r| r.get(0)).unwrap();
        assert!(!data.contains("pending_fyis"), "{data}");
        // Opening again moves nothing twice.
        drop(db);
        assert_eq!(Db::open(&path).unwrap().count_pending_fyis("a").unwrap(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn restored_fyis_go_back_ahead_of_newer_ones_and_leave_with_their_agent() {
        let path = std::env::temp_dir().join(format!("loft-fyis-{}.db", uuid::Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        db.save_agent(&agent("a")).unwrap();
        db.add_pending_fyi("a", "first").unwrap();
        db.add_pending_fyi("a", "second").unwrap();
        let taken = db.take_pending_fyis("a").unwrap();
        // One arrives while the run that took them fails.
        db.add_pending_fyi("a", "third").unwrap();
        db.restore_pending_fyis("a", &taken).unwrap();
        let prompts = |v: Vec<(i64, String)>| v.into_iter().map(|(_, p)| p).collect::<Vec<_>>();
        assert_eq!(prompts(db.take_pending_fyis("a").unwrap()), ["first", "second", "third"]);

        db.add_pending_fyi("a", "fourth").unwrap();
        db.delete_agent("a").unwrap();
        assert_eq!(db.count_pending_fyis("a").unwrap(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn grill_status_is_its_own_column_and_older_sessions_get_theirs() {
        let path = std::env::temp_dir().join(format!("loft-grills-{}.db", uuid::Uuid::new_v4()));
        let session = |id: &str, agent: &str, status: &str, at: &str| {
            serde_json::json!({ "id": id, "agent_id": agent, "topic": id, "status": status, "created_at": at, "updated_at": at })
        };
        // A database from before the column: sessions only have their status in `data`.
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE grill_sessions (id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, data TEXT NOT NULL, created_at TEXT NOT NULL);").unwrap();
            for (id, agent, status, at) in [("old", "a", "paused", "2026-01-01"), ("done", "a", "ended", "2026-01-02"), ("b1", "b", "active", "2026-01-01"), ("b0", "b", "ended", "2026-01-03")] {
                c.execute("INSERT INTO grill_sessions (id, agent_id, data, created_at) VALUES (?1, ?2, ?3, ?4)", params![id, agent, session(id, agent, status, at).to_string(), at]).unwrap();
            }
        }
        let db = Db::open(&path).unwrap();
        let statuses: Vec<(String, String)> = {
            let c = db.0.lock().unwrap();
            let mut st = c.prepare("SELECT id, status FROM grill_sessions ORDER BY id").unwrap();
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap()
        };
        let pairs = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        assert_eq!(statuses, pairs(&[("b0", "ended"), ("b1", "active"), ("done", "ended"), ("old", "paused")]));
        // The newest session that hasn't ended, even behind a newer ended one.
        assert_eq!(db.active_grill("a").unwrap().unwrap().id, "old");
        assert_eq!(db.active_grill("b").unwrap().unwrap().id, "b1");
        assert!(db.active_grill("nobody").unwrap().is_none());
        let mut open: Vec<String> = db.open_grills().unwrap().into_iter().map(|s| s.id).collect();
        open.sort();
        assert_eq!(open, ["b1", "old"]);
        // Saving keeps the column in step with `data`; opening again changes nothing.
        let mut s = db.active_grill("a").unwrap().unwrap();
        s.status = "ended".into();
        db.save_grill(&s).unwrap();
        assert!(db.active_grill("a").unwrap().is_none());
        let new: crate::grill::Session = serde_json::from_value(session("new", "a", "preparing", "2026-02-01")).unwrap();
        db.save_grill(&new).unwrap();
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.active_grill("a").unwrap().unwrap().id, "new");
        assert_eq!(db.open_grills().unwrap().len(), 2);
        drop(db);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    #[test]
    fn older_memories_get_their_first_line_as_title_and_keep_their_text() {
        let path = std::env::temp_dir().join(format!("loft-memories-{}.db", uuid::Uuid::new_v4()));
        let long = "Deploys go through staging first and only then to production, after the smoke tests pass on it\nmore";
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE memories (id INTEGER PRIMARY KEY AUTOINCREMENT, agent_id TEXT NOT NULL, content TEXT NOT NULL, tags TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL);").unwrap();
            for content in ["User prefers short replies", "\n  Two lines  \nThe second one", long] {
                c.execute("INSERT INTO memories (agent_id, content, created_at) VALUES ('a', ?1, '2026-01-01')", [content]).unwrap();
            }
        }
        let db = Db::open(&path).unwrap();
        let ms = db.list_memories("a").unwrap();
        let titles: Vec<&str> = ms.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(titles, ["User prefers short replies", "Two lines", "Deploys go through staging first and only then to production, after the smoke…"]);
        assert!(titles[2].chars().count() <= 81);
        assert_eq!(ms[2].content, long, "the whole text stays the body");
        assert!(ms.iter().all(|m| m.updated_at.is_none()), "never updated, so they keep their creation order");
        // Opening again changes nothing; new memories keep the title they were given.
        drop(db);
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_memories("a").unwrap()[1].title, "Two lines");
        let m = db.add_memory("a", "Release on Fridays", "Releases ship on Friday mornings.", "").unwrap();
        assert_eq!(db.search_memories("a", "release").unwrap()[0].id, m.id, "matches the title");
        assert_eq!(db.search_memories("a", "MORNINGS").unwrap()[0].id, m.id, "matches the body");
        drop(db);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    #[test]
    fn roster_order() {
        let db = Db::open(Path::new(":memory:")).unwrap();
        for id in ["a", "b", "c"] {
            db.save_agent(&agent(id)).unwrap();
        }
        let ids = |db: &Db| db.list_agents().unwrap().into_iter().map(|a| a.id).collect::<Vec<_>>();
        assert_eq!(ids(&db), ["a", "b", "c"]);
        db.reorder_agents(&["c".into(), "a".into(), "b".into()]).unwrap();
        // A later save (e.g. a status change) keeps the order.
        db.save_agent(&agent("a")).unwrap();
        assert_eq!(ids(&db), ["c", "a", "b"]);
        // New agents land at the bottom.
        db.save_agent(&agent("d")).unwrap();
        assert_eq!(ids(&db), ["c", "a", "b", "d"]);
    }

    #[test]
    fn push_marks_are_looked_up_by_id_and_pruned_after_30_days() {
        let db = Db::open(Path::new(":memory:")).unwrap();
        for id in ["a", "b", "old"] {
            db.mark_push_seen(id).unwrap();
        }
        let old = (Utc::now() - chrono::Duration::days(31)).to_rfc3339();
        db.0.lock().unwrap().execute("UPDATE push_seen SET seen_at = ?1 WHERE item_id = 'old'", [&old]).unwrap();
        let among = |ids: &[&str]| { let mut v: Vec<String> = db.push_seen_among(ids).unwrap().into_iter().collect(); v.sort(); v };
        assert_eq!(among(&["a", "c", "old"]), ["a", "old"], "only the ids asked about");
        assert!(among(&[]).is_empty());
        // Marking again keeps the first time seen.
        db.mark_push_seen("a").unwrap();
        assert_eq!(db.prune_push_seen().unwrap(), 1);
        assert_eq!(among(&["a", "b", "old"]), ["a", "b"]);
        assert_eq!(db.prune_push_seen().unwrap(), 0);
    }
}
