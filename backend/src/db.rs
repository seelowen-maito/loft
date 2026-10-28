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
    pub created_at: String,
    /// Loft's own agent: always present, and the user can't delete it.
    #[serde(default)]
    pub builtin: bool,
    /// The agent that created this one, if an agent did. None when the user did.
    #[serde(default)]
    pub created_by: Option<String>,
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
}

/// A unit of work: a personal todo, or work handed to an agent. Agents report
/// progress on the tasks they own through MCP; the user owns the rest.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
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
    created_at TEXT NOT NULL
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
"#;

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
        let c = self.0.lock().unwrap();
        let data: Option<String> = c
            .query_row("SELECT data FROM agents WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        Ok(match data {
            Some(d) => Some(serde_json::from_str(&d)?),
            None => None,
        })
    }

    pub fn save_agent(&self, a: &Agent) -> Result<()> {
        let c = self.0.lock().unwrap();
        c.execute(
            "INSERT INTO agents (id, data, created_at, position)
             VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position), 0) + 1 FROM agents))
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            params![a.id, serde_json::to_string(a)?, a.created_at],
        )?;
        Ok(())
    }

    /// Read-modify-write an agent under the connection lock's ordering.
    pub fn update_agent(&self, id: &str, f: impl FnOnce(&mut Agent)) -> Result<Option<Agent>> {
        let Some(mut a) = self.get_agent(id)? else {
            return Ok(None);
        };
        f(&mut a);
        self.save_agent(&a)?;
        Ok(Some(a))
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

    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let c = self.0.lock().unwrap();
        let data: Option<String> = c.query_row("SELECT data FROM tasks WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(data.map(|d| serde_json::from_str(&d)).transpose()?)
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
        })
    }

    pub fn list_memories(&self, agent_id: &str) -> Result<Vec<Memory>> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare(
            "SELECT id, agent_id, title, content, tags, created_at FROM memories WHERE agent_id = ?1 ORDER BY id",
        )?;
        let rows = st.query_map([agent_id], memory_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn search_memories(&self, agent_id: &str, query: &str) -> Result<Vec<Memory>> {
        let c = self.0.lock().unwrap();
        let like = format!("%{}%", query.to_lowercase());
        let mut st = c.prepare(
            "SELECT id, agent_id, title, content, tags, created_at FROM memories
             WHERE agent_id = ?1 AND (lower(title) LIKE ?2 OR lower(content) LIKE ?2 OR lower(tags) LIKE ?2) ORDER BY id",
        )?;
        let rows = st.query_map(params![agent_id, like], memory_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
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
