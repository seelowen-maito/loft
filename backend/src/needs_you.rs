//! "Needs you": everything waiting on the user, gathered from permissions, tasks, chats, grills, pull
//! requests and runs, for the apps' inbox and badge (`GET /api/needs-you`).
//!
//! The server only collects and orders, by fixed urgency and then age; any ranking or summarising with
//! a model happens on the phone. Item text is plain text (it may contain markdown characters; clients
//! must not render it as HTML), clipped, with anything that looks like a secret redacted.

use std::collections::HashSet;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::api::{ApiError, not_found};
use crate::db::{Agent, Message, Task};
use crate::state::{AppState, Event};

/// A run quiet this long counts as possibly stuck (the clients' watchdog uses the same).
const QUIET_MINUTES: i64 = 10;
/// How long finished tasks you created stay here for review, and how far back unanswered questions go.
const REVIEW_HOURS: i64 = 48;
const QUESTION_DAYS: i64 = 7;
const MAX_TITLE: usize = 120;
const MAX_REASON: usize = 160;
const MAX_TEXT: usize = 2000;

pub(crate) struct Item {
    id: String,
    pub(crate) kind: &'static str,
    /// 3 blocks an agent now · 2 waiting for your answer · 1 worth a look · 0 to review
    urgency: u8,
    agent: Option<(String, String)>,
    pub(crate) task_id: Option<String>,
    title: String,
    reason: String,
    text: String,
    created_at: String,
    links: Vec<Value>,
    actions: Vec<Value>,
    /// What a push says: names and titles only, never tool input or message text.
    push: (String, String),
    /// Grouping key: the GitHub repo's name for PR items, else the task's or agent's project.
    project: Option<String>,
    /// For PR items: repo, number, title, draft, checks, as separate fields.
    pr: Option<Value>,
}

fn age_secs(ts: &str) -> i64 {
    DateTime::parse_from_rfc3339(ts).map(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds().max(0)).unwrap_or(0)
}

fn within(ts: &str, d: chrono::Duration) -> bool {
    DateTime::parse_from_rfc3339(ts).is_ok_and(|t| Utc::now() - t.with_timezone(&Utc) <= d)
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end())
}

/// The first line, without markdown emphasis or heading marks.
fn first_line(s: &str) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    line.trim_start_matches(['#', '>', '-', '*', ' ']).replace("**", "").replace('`', "")
}

/// Hides values that look like credentials: known token prefixes, long JWTs, `Bearer <token>`, and
/// `password=…` / `token: …` style pairs. Deliberately broad: a hidden word costs little here.
pub fn redact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut hide_next = false;
    for piece in s.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end();
        let ws = &piece[word.len()..];
        let (w, hide) = redact_word(word, hide_next);
        out.push_str(&w);
        out.push_str(ws);
        hide_next = hide;
    }
    out
}

const PREFIXES: &[&str] = &["ghp_", "gho_", "ghu_", "ghs_", "github_pat_", "glpat-", "sk-", "sk_live_", "sk_test_", "rk_live_", "xoxb-", "xoxp-", "xoxa-", "AKIA", "ASIA", "AIza"];
const KEYS: &[&str] = &["password", "passwd", "secret", "token", "api_key", "apikey", "api-key", "access_key", "private_key", "authorization"];

/// Known token prefixes anywhere in a word (`GH=ghp_…`, `https://x:ghp_…@github.com`): the token
/// (letters, digits, `_`, `-`) is replaced when it's long enough to be one.
fn redact_embedded(word: &str) -> String {
    let mut out = String::new();
    let mut rest = word;
    'scan: while !rest.is_empty() {
        for (i, _) in rest.char_indices() {
            let at_boundary = i == 0 || !rest[..i].chars().last().is_some_and(|c| c.is_ascii_alphanumeric());
            if at_boundary && let Some(p) = PREFIXES.iter().find(|p| rest[i..].starts_with(**p)) {
                let len = rest[i..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(rest.len() - i);
                if len >= 16 && len > p.len() {
                    out.push_str(&rest[..i]);
                    out.push_str("[redacted]");
                    rest = &rest[i + len..];
                    continue 'scan;
                }
            }
        }
        out.push_str(rest);
        break;
    }
    out
}

/// One word; `hide` says the previous word announced a value ("Bearer", "password:"). Returns the
/// word as shown and whether the next word is a value to hide.
fn redact_word(word: &str, hide: bool) -> (String, bool) {
    let bare = word.trim_matches(|c: char| "\"'`()[]{}<>,;".contains(c));
    let lower = bare.to_lowercase();
    if bare.is_empty() {
        return (word.to_string(), hide);
    }
    // "Authorization: Bearer abc": the scheme word is kept and the value after it hidden.
    if lower == "bearer" || lower == "basic" || lower == "token" {
        return (word.to_string(), true);
    }
    if hide {
        return ("[redacted]".into(), false);
    }
    if bare.starts_with("eyJ") && bare.len() >= 30 && bare.matches('.').count() == 2 {
        return ("[redacted]".into(), false);
    }
    if let Some((k, _)) = bare.split_once(['=', ':']).filter(|(k, v)| !v.is_empty() && !v.starts_with("//") && KEYS.iter().any(|key| k.to_lowercase().trim_start_matches('-').ends_with(key))) {
        return (format!("{k}{}[redacted]", &bare[k.len()..k.len() + 1]), false);
    }
    let next = KEYS.iter().any(|k| lower == format!("{k}:") || lower == format!("{k}="));
    (redact_embedded(word), next)
}

fn text(s: &str, max: usize) -> String {
    clip(&redact(s), max)
}

fn action(kind: &str, label: &str, method: &str, path: String, body: Value) -> Value {
    json!({ "kind": kind, "label": label, "method": method, "path": path, "body": body })
}

fn open_chat(agent_id: &str) -> Value {
    json!({ "kind": "open", "label": "Open chat", "target": { "screen": "chat", "agent_id": agent_id } })
}

fn open_task(task_id: &str) -> Value {
    json!({ "kind": "open", "label": "Open task", "target": { "screen": "task", "task_id": task_id } })
}

fn dismiss(id: &str) -> Value {
    action("dismiss", "Dismiss", "POST", format!("/api/needs-you/{}/dismiss", urlencode(id)), json!({}))
}

fn urlencode(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

/// What a permission prompt is asking to run, in words a person can check.
fn permission_text(tool: &str, input: &Value) -> String {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    match tool {
        "Bash" => {
            let d = s("description");
            if d.is_empty() { format!("$ {}", s("command")) } else { format!("{d}\n$ {}", s("command")) }
        }
        "Edit" | "Write" | "MultiEdit" | "Read" => s("file_path"),
        "WebFetch" => s("url"),
        _ => serde_json::to_string_pretty(input).unwrap_or_default(),
    }
}

pub(crate) fn collect(state: &AppState) -> Result<Vec<Item>, String> {
    let e = |e: anyhow::Error| e.to_string();
    let agents: Vec<Agent> = state.db.list_agents().map_err(e)?;
    let name = |id: &str| agents.iter().find(|a| a.id == id).map(|a| (a.id.clone(), a.name.clone()));
    let agent_project = |id: &str| agents.iter().find(|a| a.id == id).map(|a| a.project.clone()).filter(|p| !p.is_empty());
    let who_name = |id: &str| name(id).map_or_else(|| "An agent".to_string(), |n| n.1);
    let tasks: Vec<Task> = state.db.list_tasks().map_err(e)?;
    let mut items = vec![];

    // Permission prompts: an agent is blocked until you decide.
    for p in state.db.list_permissions(None).map_err(e)?.into_iter().filter(|p| p.status == "pending") {
        let who = name(&p.agent_id);
        let base = format!("/api/permissions/{}", p.id);
        let mut actions = vec![
            action("approve", "Allow once", "POST", base.clone(), json!({ "decision": "allow_once" })),
            action("approve_always", "Always allow", "POST", base.clone(), json!({ "decision": "allow_always", "rule": p.rule })),
            action("deny", "Deny", "POST", base, json!({ "decision": "deny" })),
        ];
        actions.push(open_chat(&p.agent_id));
        items.push(Item {
            id: format!("permission:{}", p.id),
            kind: "permission",
            urgency: 3,
            title: text(&format!("{} wants to use {}", who.as_ref().map_or("An agent", |w| w.1.as_str()), p.tool_name), MAX_TITLE),
            reason: text(if p.reason.trim().is_empty() { "Waiting for your approval." } else { &p.reason }, MAX_REASON),
            text: text(&permission_text(&p.tool_name, &p.input), MAX_TEXT),
            agent: who,
            task_id: None,
            created_at: p.created_at.clone(),
            links: vec![],
            actions,
            push: (format!("{} needs your approval", who_name(&p.agent_id)), format!("Wants to use {}", p.tool_name)),
            project: agent_project(&p.agent_id),
            pr: None,
        });
    }

    // Tasks waiting for you, blocked or failed.
    for t in &tasks {
        let (urgency, reason) = match t.status.as_str() {
            "waiting_approval" => (3, if t.note.is_empty() { "Waiting for your approval.".to_string() } else { t.note.clone() }),
            "blocked" => (2, if t.error.is_empty() { "Blocked: needs your input.".to_string() } else { format!("Blocked: {}", first_line(&t.error)) }),
            "failed" => (1, if t.error.is_empty() { "Failed.".to_string() } else { format!("Failed: {}", first_line(&t.error)) }),
            _ => continue,
        };
        let body = [&t.error, &t.note, &t.summary].iter().find(|s| !s.trim().is_empty()).map(|s| s.to_string()).unwrap_or_default();
        let mut actions = vec![action("answer", "Reply on the task", "POST", format!("/api/tasks/{}/comments", t.id), json!({ "text": "", "send": true }))];
        if t.status == "failed" && t.agent_id.is_some() {
            actions.push(action("retry", "Reopen and send", "POST", format!("/api/tasks/{}/comments", t.id), json!({ "text": "", "send": true, "reopen": true })));
        }
        actions.push(open_task(&t.id));
        actions.push(dismiss(&format!("task:{}:{}", t.id, t.status)));
        items.push(Item {
            id: format!("task:{}:{}", t.id, t.status),
            kind: match t.status.as_str() { "waiting_approval" => "task_approval", "blocked" => "task_blocked", _ => "task_failed" },
            urgency,
            agent: t.agent_id.as_deref().and_then(name),
            task_id: Some(t.id.clone()),
            title: text(&t.title, MAX_TITLE),
            reason: text(&reason, MAX_REASON),
            text: text(&body, MAX_TEXT),
            created_at: t.updated_at.clone(),
            links: vec![json!({ "label": "Task", "path": format!("/tasks/{}", t.id) })],
            actions,
            push: (
                match t.status.as_str() {
                    "waiting_approval" => format!("{} needs your approval", t.agent_id.as_deref().map_or("A task".to_string(), who_name)),
                    "blocked" => format!("{} is blocked", t.agent_id.as_deref().map_or("A task".to_string(), who_name)),
                    _ => "A task failed".to_string(),
                },
                t.title.clone(),
            ),
            project: (!t.project.is_empty()).then(|| t.project.clone()),
            pr: None,
        });
    }

    // Comments on mirrored issues waiting to go to GitHub. Nobody is blocked, so it's worth a look, not urgent.
    // Auto-approved ones are on their way and need nobody.
    for c in state.db.task_comments_on_github("pending").map_err(e)?.into_iter().filter(|c| c.meta["github"]["auto"] != true) {
        let Some(t) = tasks.iter().find(|t| t.id == c.task_id) else { continue };
        let Some(g) = &t.github else { continue };
        let who = if c.author == "user" { "Your".to_string() } else { format!("{}'s", c.meta["author_name"].as_str().unwrap_or("An agent")) };
        let base = format!("/api/issues/{}/comments/{}", t.id, c.id);
        items.push(Item {
            id: format!("comment:{}", c.id),
            kind: "comment_approval",
            urgency: 1,
            agent: (c.author != "user").then(|| name(&c.author)).flatten(),
            task_id: Some(t.id.clone()),
            title: text(&format!("{who} comment on {}", t.title), MAX_TITLE),
            reason: text(&format!("Not on GitHub yet: approve to post it on {}#{}.", g.repo, g.number), MAX_REASON),
            text: text(&c.text, MAX_TEXT),
            created_at: c.created_at.clone(),
            links: vec![json!({ "label": "Task", "path": format!("/tasks/{}", t.id) })],
            actions: vec![
                action("approve", "Approve", "POST", format!("{base}/approve"), json!({})),
                action("answer", "Edit and approve", "POST", format!("{base}/approve"), json!({ "text": c.text })),
                action("deny", "Keep in Loft", "POST", format!("{base}/keep"), json!({})),
                open_task(&t.id),
            ],
            push: ("A comment waits for your approval".to_string(), t.title.clone()),
            project: (!t.project.is_empty()).then(|| t.project.clone()),
            pr: None,
        });
    }

    // Tasks you created that finished recently: a result to look at.
    for t in tasks.iter().filter(|t| t.status == "done" && t.created_by == "user") {
        let done_at = t.completed_at.clone().unwrap_or_else(|| t.updated_at.clone());
        if !within(&done_at, chrono::Duration::hours(REVIEW_HOURS)) {
            continue;
        }
        let id = format!("done:{}:{}", t.id, done_at);
        items.push(Item {
            kind: "task_done",
            urgency: 0,
            agent: t.agent_id.as_deref().and_then(name),
            task_id: Some(t.id.clone()),
            title: text(&t.title, MAX_TITLE),
            reason: text(&if t.summary.is_empty() { "Done.".to_string() } else { format!("Done: {}", first_line(&t.summary)) }, MAX_REASON),
            text: text(&t.summary, MAX_TEXT),
            created_at: done_at,
            links: vec![json!({ "label": "Task", "path": format!("/tasks/{}", t.id) })],
            actions: vec![
                action("answer", "Follow up", "POST", format!("/api/tasks/{}/comments", t.id), json!({ "text": "", "send": true, "reopen": true })),
                open_task(&t.id),
                dismiss(&id),
            ],
            push: ("Task done".to_string(), t.title.clone()),
            project: (!t.project.is_empty()).then(|| t.project.clone()),
            pr: None,
            id,
        });
    }

    for a in &agents {
        let mut run = json!({});
        crate::agents::run_fields(state, &a.id, &mut run);
        let running = run["running"].as_bool().unwrap_or(false);

        // A grill round waiting for the user's answers.
        if let Some(s) = crate::grill::active(state, &a.id)
            && let Some(r) = crate::grill::open_round(&s)
        {
            let paused = s.status == "paused";
            let answered = r.questions.iter().filter(|q| q.state != "open").count();
            items.push(Item {
                id: format!("grill:{}:{}", s.id, r.index),
                kind: "grill_round",
                urgency: if paused { 1 } else { 2 },
                agent: Some((a.id.clone(), a.name.clone())),
                task_id: None,
                title: text(&format!("{} is interviewing you: {}", a.name, s.topic), MAX_TITLE),
                reason: text(&format!("Round {}: {answered} of {} answered{}", r.index, r.questions.len(), if paused { " (interview paused)" } else { "" }), MAX_REASON),
                text: text(&r.questions.iter().map(|q| format!("• {}", q.text)).collect::<Vec<_>>().join("\n"), MAX_TEXT),
                created_at: r.posted_at.clone(),
                links: vec![],
                actions: vec![open_chat(&a.id)],
                push: (format!("{}: round {}", a.name, r.index), format!("Interview: {}", s.topic)),
                project: agent_project(&a.id),
                pr: None,
            });
        }

        // A run that's stuck: waiting on background work, or quiet for a while.
        if running {
            let waiting: Vec<String> = run["waiting_on"].as_array().map(|w| w.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let last = run["last_output_at"].as_str().unwrap_or("").to_string();
            let quiet = age_secs(&last) / 60;
            let since = run["waiting_since"].as_str().map(String::from).unwrap_or(last.clone());
            let reason = if !waiting.is_empty() && age_secs(&since) / 60 >= QUIET_MINUTES {
                Some(format!("Waiting on a background task for {} min: {}", age_secs(&since) / 60, waiting.join(", ")))
            } else if waiting.is_empty() && !last.is_empty() && quiet >= QUIET_MINUTES {
                Some(format!("No activity for {quiet} min."))
            } else {
                None
            };
            if let Some(reason) = reason {
                let id = format!("stuck:{}:{}", a.id, a.last_run_at.clone().unwrap_or_default());
                items.push(Item {
                    kind: "stuck",
                    urgency: 2,
                    agent: Some((a.id.clone(), a.name.clone())),
                    task_id: None,
                    title: text(&format!("{} may be stuck", a.name), MAX_TITLE),
                    reason: text(&reason, MAX_REASON),
                    text: text(&a.status_text, MAX_TEXT),
                    created_at: since,
                    links: vec![],
                    actions: vec![
                        action("restart", "Restart (keeps queued messages)", "POST", format!("/api/agents/{}/restart", a.id), json!({})),
                        open_chat(&a.id),
                        dismiss(&id),
                    ],
                    push: (
                        format!("{} may be stuck", a.name),
                        if waiting.is_empty() { format!("No activity for {quiet} min") } else { "Waiting on a background task".to_string() },
                    ),
                    project: agent_project(&a.id),
                    pr: None,
                    id,
                });
            }
            continue;
        }

        // An agent that ended its reply to you with a question (and you haven't written since): like an
        // Unread reply, only a run you started (a message or a schedule) counts, and only your own
        // message answers it, not another agent's.
        let msgs = state.db.list_messages(&a.id, 30, None).map_err(e)?;
        let last_turn = msgs.iter().rev().find(|m| {
            m.meta["subagent"].is_null() && m.meta.get("grill").is_none() && ((m.role == "assistant" && m.kind == "text") || (m.role == "user" && m.kind == "text" && m.meta["from_agent"].is_null()))
        });
        // The reply's run marker is usually among the messages already loaded.
        let for_user = |m: &Message| {
            let Some(r) = m.run_id.as_deref() else { return false };
            let loaded = msgs.iter().find(|s| s.kind == "run_start" && s.run_id.as_deref() == Some(r)).cloned();
            loaded.or_else(|| state.db.run_start(&a.id, r, m.id).ok().flatten()).is_some_and(|s| s.meta["for_user"] == json!(true))
        };
        if let Some(m) = last_turn.filter(|m| m.role == "assistant" && within(&m.created_at, chrono::Duration::days(QUESTION_DAYS)) && for_user(m)) {
            let ends_with_question = m.content.trim_end().trim_end_matches(['*', '_', ')']).ends_with('?');
            if ends_with_question {
                let last_q = m.content.lines().rev().map(str::trim).find(|l| l.ends_with('?') || l.ends_with("?**")).unwrap_or("");
                let id = format!("question:{}", m.id);
                items.push(Item {
                    kind: "question",
                    urgency: 2,
                    agent: Some((a.id.clone(), a.name.clone())),
                    task_id: m.meta["task_id"].as_str().map(String::from),
                    title: text(&format!("{} asked you something", a.name), MAX_TITLE),
                    reason: text(&first_line(last_q), MAX_REASON),
                    text: text(&m.content, MAX_TEXT),
                    created_at: m.created_at.clone(),
                    links: vec![],
                    actions: vec![
                        action("answer", "Reply", "POST", format!("/api/agents/{}/messages", a.id), json!({ "text": "" })),
                        open_chat(&a.id),
                        dismiss(&id),
                    ],
                    push: (format!("{} asked you something", a.name), "Open the chat to answer".to_string()),
                    project: agent_project(&a.id),
                    pr: None,
                    id,
                });
            }
        }
    }

    // Pull requests ready to merge: open, not a draft, mergeable, checks green. First the ones agents
    // opened here (they can be merged from the app), then the user's other ready PRs and review
    // requests found on GitHub (github_inbox).
    let mut seen_prs: HashSet<(String, i64)> = HashSet::new();
    for p in state.db.list_pull_requests(None).map_err(e)? {
        let checks = &p.checks;
        let n = |k: &str| checks[k].as_i64().unwrap_or(0);
        let ready = p.state == "open" && !p.draft && p.mergeable == Some(true) && ["clean", "unstable", "has_hooks"].contains(&p.mergeable_state.as_str()) && n("failed") == 0 && n("pending") == 0;
        if !ready {
            continue;
        }
        seen_prs.insert((p.slug.to_lowercase(), p.number));
        let id = format!("pr:{}:{}", p.id, p.head_sha);
        let who = name(&p.agent_id);
        let method = p.merge_methods.first().cloned().unwrap_or_else(|| "squash".into());
        let repo_name = p.slug.rsplit('/').next().unwrap_or(&p.slug).to_string();
        items.push(Item {
            kind: "pr_ready",
            urgency: 1,
            title: text(&format!("{}#{}: {}", p.slug, p.number, p.title), MAX_TITLE),
            reason: text(&format!("Ready to merge{}.", if n("total") > 0 { format!(", {} checks passed", n("passed")) } else { String::new() }), MAX_REASON),
            text: text(&format!("{} → {} · +{} −{} in {} files", p.head_ref, p.base_ref, p.additions, p.deletions, p.changed_files), MAX_TEXT),
            agent: who,
            task_id: None,
            created_at: p.created_at.clone(),
            links: vec![json!({ "label": "Pull request", "url": p.url })],
            actions: vec![
                action("merge", "Merge", "POST", format!("/api/pull-requests/{}/merge", p.id), json!({ "method": method })),
                json!({ "kind": "open", "label": "Open on GitHub", "target": { "url": p.url } }),
                dismiss(&id),
            ],
            push: ("Ready to merge".to_string(), format!("{}#{}: {}", p.slug, p.number, p.title)),
            project: Some(repo_name.clone()),
            pr: Some(pr_fields(&p.slug, &repo_name, p.number, &p.title, &p.url, &p.author, if n("total") > 0 { "passing" } else { "none" }, false)),
            id,
        });
    }
    let inbox = state.rt.lock().unwrap().github_inbox.clone();
    for g in inbox.into_iter().filter(|g| !seen_prs.contains(&(g.repo.to_lowercase(), g.number))) {
        let id = format!("{}:{}#{}:{}", if g.review_requested { "review" } else { "ghpr" }, g.repo, g.number, g.head_sha);
        let repo_name = g.repo.rsplit('/').next().unwrap_or(&g.repo).to_string();
        let checks = if g.checks.is_empty() { "none" } else { "passing" };
        items.push(Item {
            kind: if g.review_requested { "pr_review" } else { "pr_ready" },
            urgency: 1,
            title: text(&format!("{}#{}: {}", g.repo, g.number, g.title), MAX_TITLE),
            reason: text(if g.review_requested { "Your review is requested." } else if checks == "passing" { "Ready to merge, checks passed." } else { "Ready to merge." }, MAX_REASON),
            text: text(&format!("by {}", g.author), MAX_TEXT),
            agent: None,
            task_id: None,
            created_at: g.created_at.clone(),
            links: vec![json!({ "label": "Pull request", "url": g.url })],
            actions: vec![json!({ "kind": "open", "label": "Open on GitHub", "target": { "url": g.url } }), dismiss(&id)],
            push: (if g.review_requested { "Review requested" } else { "Ready to merge" }.to_string(), format!("{}#{}: {}", g.repo, g.number, g.title)),
            project: Some(repo_name.clone()),
            pr: Some(pr_fields(&g.repo, &repo_name, g.number, &g.title, &g.url, &g.author, checks, g.review_requested)),
            id,
        });
    }

    let dismissed: HashSet<String> = state.db.dismissed_needs_you().map_err(e)?.into_iter().collect();
    items.retain(|i| !dismissed.contains(&i.id));
    // Most urgent first; within the same urgency, the longest waiting first.
    items.sort_by(|a, b| b.urgency.cmp(&a.urgency).then_with(|| a.created_at.cmp(&b.created_at)));
    Ok(items)
}

/// The separate PR fields on a PR item. Only ready PRs are listed, so `draft` is always false and
/// `checks` is "passing" or "none" (no checks configured).
#[allow(clippy::too_many_arguments)]
fn pr_fields(repo: &str, project: &str, number: i64, title: &str, url: &str, author: &str, checks: &str, review_requested: bool) -> Value {
    json!({
        "repo": repo, "project": project, "number": number, "title": text(title, MAX_TITLE), "url": url, "author": author,
        "draft": false, "checks": checks, "checks_green": true, "review_requested": review_requested,
    })
}

fn to_json(i: &Item) -> Value {
    json!({
        "id": i.id,
        "type": i.kind,
        "urgency": i.urgency,
        "agent": i.agent.as_ref().map(|(id, name)| json!({ "id": id, "name": name })),
        "task_id": i.task_id,
        "title": i.title,
        "reason": i.reason,
        "text": i.text,
        "created_at": i.created_at,
        "age_seconds": age_secs(&i.created_at),
        "links": i.links,
        "actions": i.actions,
        "project": i.project,
        // PR items: the GitHub slug at the top level too, so clients can group without opening `pr`.
        "repo": i.pr.as_ref().map(|p| p["repo"].clone()),
        "pr": i.pr,
    })
}

/// An item as a push notification sees it: names and titles only.
pub struct PushItem {
    pub id: String,
    pub kind: &'static str,
    pub urgency: u8,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub title: String,
    pub body: String,
    /// Where tapping it goes: the same shape as an `open` action's target.
    pub target: Value,
}

pub(crate) fn to_push(i: &Item) -> PushItem {
    let target = i.actions.iter().find(|a| a["kind"] == "open").map(|a| a["target"].clone()).unwrap_or(Value::Null);
    PushItem {
        id: i.id.clone(),
        kind: i.kind,
        urgency: i.urgency,
        agent_id: i.agent.as_ref().map(|a| a.0.clone()),
        task_id: i.task_id.clone(),
        title: text(&i.push.0, MAX_TITLE),
        body: text(&i.push.1, MAX_REASON),
        target,
    }
}

/// The whole list, as `GET /api/needs-you` returns it.
pub fn snapshot(state: &AppState) -> Result<Value, String> {
    collect(state).map(|items| snapshot_of(&items))
}

fn snapshot_of(items: &[Item]) -> Value {
    json!({
        "version": 1,
        "count": items.len(),
        "urgent": items.iter().filter(|i| i.urgency >= 2).count(),
        "generated_at": crate::db::now(),
        "items": items.iter().map(to_json).collect::<Vec<_>>(),
    })
}

/// This holds conversation text, so unlike other reads it is refused to other websites: CORS is
/// permissive, so without this a page open in the user's browser could read it. The iOS app and
/// other non-browser clients don't send `Sec-Fetch-Site`.
fn same_site(headers: &HeaderMap) -> Result<(), ApiError> {
    match headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        None | Some("same-origin" | "none") => Ok(()),
        Some(_) => Err(ApiError(StatusCode::FORBIDDEN, "cross-site request refused".into())),
    }
}

/// `GET /api/needs-you`
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    same_site(&headers)?;
    snapshot(&state).map(Json).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))
}

/// `POST /api/needs-you/{id}/dismiss`: hide an item until it changes (a changed item gets a new id).
/// Permission prompts and open grill rounds can't be dismissed: they block an agent until answered.
pub async fn dismiss_item(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    if id.starts_with("permission:") || id.starts_with("grill:") {
        return Err(ApiError(StatusCode::BAD_REQUEST, "This one needs an answer; it can't be dismissed.".into()));
    }
    let known = collect(&state).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?.iter().any(|i| i.id == id);
    if !known {
        return Err(not_found());
    }
    state.db.dismiss_needs_you(&id)?;
    emit_now(&state);
    Ok(Json(json!({ "ok": true })))
}

/// The ids and count last sent, so the watcher only emits on a change.
fn fingerprint(v: &Value) -> String {
    v["items"].as_array().map(|a| a.iter().filter_map(|i| i["id"].as_str()).collect::<Vec<_>>().join("|")).unwrap_or_default()
}

fn emit(state: &AppState, v: &Value) {
    state.emit(Event::NeedsYou { count: v["count"].as_u64().unwrap_or(0), urgent: v["urgent"].as_u64().unwrap_or(0), ids: v["items"].as_array().map(|a| a.iter().map(|i| i["id"].clone()).collect()).unwrap_or_default() });
}

fn emit_now(state: &AppState) {
    if let Ok(v) = snapshot(state) {
        emit(state, &v);
    }
}

/// One tick's push items and snapshot, from a single read of everything.
fn tick_lists(state: &AppState) -> Result<(Vec<PushItem>, Value), String> {
    let items = collect(state)?;
    Ok((items.iter().map(to_push).collect(), snapshot_of(&items)))
}

/// Tells clients when the list changes (`needs_you` event with the count and ids). Checks every few
/// seconds instead of on every event: messages stream fast, and the list rarely changes. The list is
/// built once per tick; push and the event both come from it.
pub async fn watcher(state: AppState) {
    let mut last = String::new();
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    let mut pusher = crate::push::Pusher::default();
    loop {
        tick.tick().await;
        let Ok((push, v)) = tick_lists(&state) else { continue };
        pusher.tick(&state, &push).await;
        let fp = fingerprint(&v);
        if fp != last {
            emit(&state, &v);
            last = fp;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{PermissionRequest, PullRequest, now};
    use crate::tasks::tests::{agent, state};

    fn ids(v: &Value) -> Vec<String> {
        v["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
    }

    fn item<'a>(v: &'a Value, kind: &str) -> &'a Value {
        v["items"].as_array().unwrap().iter().find(|i| i["type"] == kind).unwrap_or_else(|| panic!("no {kind} item in {v:#}"))
    }

    /// A run's reply as the chat records it: a `run_start` marker saying whether the user started the run, then the text.
    fn reply(st: &AppState, agent_id: &str, for_user: bool, text: &str) {
        let rid = uuid::Uuid::new_v4().to_string();
        crate::agents::add_msg(st, agent_id, Some(&rid), "system", "run_start", "user", json!({ "trigger": "user", "for_user": for_user }));
        crate::agents::add_msg(st, agent_id, Some(&rid), "assistant", "text", text, json!({}));
    }

    #[test]
    fn redacts_things_that_look_like_secrets() {
        let s = "export GH=ghp_abcdefghijklmnopqrstuvwxyz123456 then curl -H 'Authorization: Bearer abc.def-123' \
                 password=hunter2 token: s3cr3tvalue --api-key=XYZ123 api_key=k1 and sk-ant-REDACTED_EXAMPLE_KEY_1234 ok";
        let r = redact(s);
        for leaked in ["ghp_abcdef", "abc.def-123", "hunter2", "s3cr3tvalue", "XYZ123", "k1 ", "sk-ant-"] {
            assert!(!r.contains(leaked), "{leaked:?} leaked: {r}");
        }
        assert!(r.contains("password=[redacted]") && r.contains("--api-key=[redacted]") && r.ends_with(" ok"), "{r}");
        assert_eq!(redact("Monthly pricing, 8 dollars? Yes."), "Monthly pricing, 8 dollars? Yes.");
    }

    #[tokio::test]
    async fn gathers_everything_waiting_on_the_user_in_order() {
        let st = state();
        let (dev, qa, orch) = (agent(&st, "Dev"), agent(&st, "QA"), crate::grill::tests::planner(&st, "Orchestrator"));
        assert_eq!(snapshot(&st).unwrap()["count"], json!(0), "nothing waits on a fresh install");

        // A permission prompt (with a secret in the command).
        st.db.save_permission(&PermissionRequest {
            id: "p1".into(), agent_id: dev.id.clone(), source: "prompt".into(), tool_name: "Bash".into(),
            input: json!({ "command": "git push https://x:ghp_abcdefghijklmnopqrstuvwxyz123456@github.com/o/r", "description": "Push the fix" }),
            reason: String::new(), rule: "Bash(git push *)".into(), status: "pending".into(), created_at: now(),
        }).unwrap();
        // Tasks: blocked, failed, done recently (yours), done (an agent's subtask: not yours to review).
        let mk = |title: &str, status: &str, by: &str, extra: Value| {
            let mut t: Task = serde_json::from_value(json!({ "id": uuid::Uuid::new_v4().to_string(), "title": title, "status": status, "created_by": by,
                "agent_id": qa.id, "created_at": now(), "updated_at": now(), "completed_at": if status == "done" { Some(now()) } else { None } })).unwrap();
            if let Some(e) = extra["error"].as_str() { t.error = e.into(); }
            if let Some(s) = extra["summary"].as_str() { t.summary = s.into(); }
            st.db.save_task(&t).unwrap();
            t
        };
        mk("Run the suite", "blocked", "user", json!({ "error": "Which simulator should I use?" }));
        mk("Fix the build", "failed", "user", json!({ "error": "Xcode 27 missing\nmore lines" }));
        let done = mk("Measure sizes", "done", "user", json!({ "summary": "**15 GB** after cleanup." }));
        mk("Subtask", "done", &orch.id, json!({}));
        // An agent ended its reply with a question; another one was answered after its question.
        reply(&st, &dev.id, true, "I found two options.\n\nShould I use B2 or S3?");
        reply(&st, &orch.id, true, "Want me to start?");
        crate::agents::add_msg(&st, &orch.id, None, "user", "text", "yes", json!({}));
        // A grill question on Orchestrator.
        crate::grill::mcp_call(&st, &orch, "grill_round", &json!({ "topic": "Pricing", "questions": [{ "text": "Who pays?", "speak": "Who pays?", "recommendation": "The team lead" }] })).unwrap().unwrap();
        // A PR ready to merge, and one with failing checks (not ready).
        let pr = |id: &str, failed: i64| PullRequest {
            id: id.into(), agent_id: dev.id.clone(), slug: "o/r".into(), number: 7 + failed, url: "https://github.com/o/r/pull/7".into(), title: "Fix it".into(),
            state: "open".into(), mergeable: Some(true), mergeable_state: "clean".into(), merge_methods: vec!["squash".into()], head_sha: "abc".into(),
            checks: json!({ "total": 3, "passed": 3 - failed, "failed": failed, "pending": 0 }), created_at: now(), ..Default::default()
        };
        st.db.save_pull_request(&pr("pr1", 0)).unwrap();
        st.db.save_pull_request(&pr("pr2", 1)).unwrap();

        // Ready PRs found on GitHub: one already tracked above (shown once), one of yours, one review request.
        let gh = |repo: &str, n: i64, review: bool| crate::github_inbox::GhPr {
            repo: repo.into(), number: n, title: format!("PR {n}"), url: format!("https://github.com/{repo}/pull/{n}"), author: "octocat".into(),
            head_sha: format!("sha{n}"), created_at: now(), checks: if n == 41 { String::new() } else { "SUCCESS".into() }, review_requested: review,
        };
        st.rt.lock().unwrap().github_inbox = vec![gh("o/r", 7, false), gh("acme/shop", 41, false), gh("acme/api", 1800, true)];
        let v = snapshot(&st).unwrap();
        // `NEEDS_YOU_FIXTURE=<path> cargo test needs_you` refreshes the contract's example (docs/needs-you-api.md).
        if let Ok(path) = std::env::var("NEEDS_YOU_FIXTURE") {
            std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        }
        let types: Vec<&str> = v["items"].as_array().unwrap().iter().map(|i| i["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["permission", "task_blocked", "question", "grill_round", "task_failed", "pr_ready", "pr_ready", "pr_review", "task_done"], "{v:#}");
        assert_eq!((v["count"].clone(), v["urgent"].clone(), v["version"].clone()), (json!(9), json!(4), json!(1)));
        // PRs carry their repo as separate fields, for grouping by project.
        let prs: Vec<&Value> = v["items"].as_array().unwrap().iter().filter(|i| i["pr"].is_object()).collect();
        let keys: Vec<(String, String, i64, String, bool)> = prs.iter().map(|i| (i["pr"]["repo"].as_str().unwrap().into(), i["project"].as_str().unwrap().into(), i["pr"]["number"].as_i64().unwrap(), i["pr"]["checks"].as_str().unwrap().into(), i["pr"]["review_requested"].as_bool().unwrap())).collect();
        assert_eq!(keys, [
            ("o/r".to_string(), "r".to_string(), 7, "passing".to_string(), false),
            ("acme/shop".into(), "shop".into(), 41, "none".into(), false),
            ("acme/api".into(), "api".into(), 1800, "passing".into(), true),
        ], "o/r#7 is listed once, from the tracked PR with its Merge action");
        assert_eq!((prs[0]["pr"]["draft"].clone(), prs[0]["pr"]["checks_green"].clone()), (json!(false), json!(true)));
        assert_eq!(prs[0]["actions"][0]["kind"], json!("merge"));
        assert_eq!((prs[1]["repo"].clone(), item(&v, "task_blocked")["repo"].clone()), (json!("acme/shop"), Value::Null));
        assert_eq!(prs[2]["actions"][0]["target"]["url"], json!("https://github.com/acme/api/pull/1800"));
        assert_eq!(item(&v, "task_blocked")["pr"], Value::Null);

        let p = item(&v, "permission");
        assert!(!p.to_string().contains("ghp_abcdef"), "the token is redacted everywhere");
        assert_eq!(p["text"], json!("Push the fix\n$ git push https://x:[redacted]@github.com/o/r"));
        assert_eq!((p["agent"]["name"].clone(), p["actions"][0]["path"].clone(), p["actions"][0]["body"].clone()), (json!("Dev"), json!("/api/permissions/p1"), json!({ "decision": "allow_once" })));
        assert_eq!(item(&v, "question")["reason"], json!("Should I use B2 or S3?"));
        assert_eq!(item(&v, "task_failed")["reason"], json!("Failed: Xcode 27 missing"));
        assert_eq!(item(&v, "task_done")["reason"], json!("Done: 15 GB after cleanup."));
        assert_eq!((item(&v, "grill_round")["text"].clone(), item(&v, "grill_round")["reason"].clone()), (json!("• Who pays?"), json!("Round 1: 0 of 1 answered")));
        let merge = &item(&v, "pr_ready")["actions"][0];
        assert_eq!((merge["path"].clone(), merge["body"]["method"].clone()), (json!("/api/pull-requests/pr1/merge"), json!("squash")));
        assert_eq!(item(&v, "pr_ready")["links"][0]["url"], json!("https://github.com/o/r/pull/7"), "links are kept apart from the text");

        // The watcher reads everything once per tick: its push items and snapshot match separate reads.
        let (push, once) = tick_lists(&st).unwrap();
        let shown = |p: &[PushItem]| p.iter().map(|i| json!([i.id, i.kind, i.urgency, i.agent_id, i.task_id, i.title, i.body, i.target])).collect::<Vec<_>>();
        assert_eq!(shown(&push), shown(&collect(&st).unwrap().iter().map(to_push).collect::<Vec<_>>()));
        assert_eq!(push.len(), 9);
        let without_time = |mut v: Value| {
            v.as_object_mut().unwrap().remove("generated_at");
            v
        };
        assert_eq!(without_time(once), without_time(snapshot(&st).unwrap()));

        // Dismissing hides an item until it changes; permission prompts can't be dismissed.
        let done_id = format!("done:{}:{}", done.id, done.completed_at.clone().unwrap());
        assert!(dismiss_item(State(st.clone()), Path(done_id.clone())).await.is_ok());
        assert!(!ids(&snapshot(&st).unwrap()).contains(&done_id));
        assert!(dismiss_item(State(st.clone()), Path("permission:p1".into())).await.is_err());
        assert!(dismiss_item(State(st.clone()), Path(item(&v, "grill_round")["id"].as_str().unwrap().into())).await.is_err(), "an open round can't be dismissed");
        assert!(dismiss_item(State(st.clone()), Path("question:999999".into())).await.is_err(), "unknown ids are refused");
    }

    #[test]
    fn an_open_round_is_one_item_until_it_is_sent() {
        let st = state();
        let lead = crate::grill::tests::planner(&st, "Lead");
        let qs = json!([{ "text": "Who pays?", "speak": "Who pays?", "recommendation": "The team lead" }, { "text": "Trial?", "speak": "Trial?", "recommendation": "No" }]);
        crate::grill::mcp_call(&st, &lead, "grill_round", &json!({ "topic": "Pricing", "questions": qs })).unwrap().unwrap();
        let rounds = |st: &AppState| snapshot(st).unwrap()["items"].as_array().unwrap().iter().filter(|i| i["type"] == "grill_round").cloned().collect::<Vec<_>>();
        let r = rounds(&st);
        assert_eq!(r.len(), 1, "one item for the round, not one per question");
        assert_eq!((r[0]["text"].clone(), r[0]["urgency"].clone(), r[0]["actions"][0]["kind"].clone()), (json!("• Who pays?\n• Trial?"), json!(2), json!("open")));
        assert!(!r[0]["actions"].as_array().unwrap().iter().any(|a| a["kind"] == "dismiss"));
        // Answering and pausing keep the same item (no new push), with its progress and a lower urgency.
        let id = r[0]["id"].clone();
        let s = crate::grill::active(&st, &lead.id).unwrap();
        crate::grill::on_user_message(&st, &lead, "yes", Some(&json!({ "role": "answer", "session_id": s.id, "question_id": s.rounds[0].questions[0].id })), json!({})).ok().unwrap();
        crate::grill::on_user_message(&st, &lead, "pause", None, json!({})).ok().unwrap();
        let r = rounds(&st);
        assert_eq!((r[0]["id"].clone(), r[0]["urgency"].clone(), r[0]["reason"].clone()), (id, json!(1), json!("Round 1: 1 of 2 answered (interview paused)")));
        // Sent: nothing waits on the user until the next round.
        crate::grill::on_user_message(&st, &lead, "send", None, json!({})).ok().unwrap();
        assert!(rounds(&st).is_empty());
    }

    #[test]
    fn other_websites_cannot_read_it() {
        let h = |site: Option<&str>| {
            let mut h = HeaderMap::new();
            if let Some(s) = site {
                h.insert("sec-fetch-site", s.parse().unwrap());
            }
            h
        };
        assert!(same_site(&h(None)).is_ok(), "the iOS app and curl send no Sec-Fetch-Site");
        assert!(same_site(&h(Some("same-origin"))).is_ok());
        assert!(same_site(&h(Some("cross-site"))).is_err());
        assert!(same_site(&h(Some("same-site"))).is_err());
    }

    #[test]
    fn only_a_run_you_started_asks_you_something_and_only_your_message_answers_it() {
        let st = state();
        let (dev, qa) = (agent(&st, "Dev"), agent(&st, "QA"));
        let questions = |st: &AppState| {
            let push = collect(st).unwrap().iter().map(to_push).filter(|p| p.kind == "question").count();
            let items = snapshot(st).unwrap()["items"].as_array().unwrap().iter().filter(|i| i["type"] == "question").filter_map(|i| i["agent"]["name"].as_str().map(String::from)).collect::<Vec<_>>();
            assert_eq!(push, items.len(), "the push follows the item");
            items
        };

        // Another agent started the run: no item, no push. Neither for a reply outside any run.
        let from_qa = json!({ "from_agent": { "id": qa.id, "name": "QA" } });
        crate::agents::add_msg(&st, &dev.id, None, "user", "text", "Can you check the build?", from_qa.clone());
        reply(&st, &dev.id, false, "Which branch should I build?");
        crate::agents::add_msg(&st, &qa.id, None, "assistant", "text", "Anything else?", json!({}));
        assert!(questions(&st).is_empty());

        // A run the user started (a message or a schedule) still asks.
        reply(&st, &dev.id, true, "Should I use B2 or S3?");
        assert_eq!(questions(&st), ["Dev"]);

        // An agent's message after the question doesn't answer it; the user's does.
        crate::agents::add_msg(&st, &dev.id, None, "user", "text", "Status update: CI is green.", from_qa);
        assert_eq!(questions(&st), ["Dev"]);
        crate::agents::add_msg(&st, &dev.id, None, "user", "text", "S3", json!({}));
        assert!(questions(&st).is_empty());

        // A long run: its marker is past the loaded messages, so it's looked up, walking back from the reply.
        let rid = "long-run";
        let start = crate::agents::add_msg(&st, &dev.id, Some(rid), "system", "run_start", "user", json!({ "for_user": true })).unwrap();
        for i in 0..40 {
            crate::agents::add_msg(&st, &dev.id, Some(rid), "assistant", "tool_use", &format!("step {i}"), json!({}));
        }
        let r = crate::agents::add_msg(&st, &dev.id, Some(rid), "assistant", "text", "Ready to merge?", json!({})).unwrap();
        assert_eq!(questions(&st), ["Dev"]);
        assert_eq!(st.db.run_start(&dev.id, rid, r.id).unwrap().map(|m| m.id), Some(start.id));
        assert!(st.db.run_start(&dev.id, rid, start.id).unwrap().is_none(), "only markers before the reply");
    }
}
