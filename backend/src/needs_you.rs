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
const REVIEW_HOURS: i64 = 24;
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
    json!({ "kind": "open", "label": "Open issue", "target": { "screen": "task", "task_id": task_id } })
}

/// The Loft issue of a Delivery's spec (or standalone ticket).
fn delivery_issue<'a>(tasks: &'a [Task], d: &crate::db::Delivery) -> Option<&'a Task> {
    if d.local {
        tasks.iter().find(|t| t.id == d.spec_id)
    } else {
        tasks.iter().find(|t| t.github.as_ref().is_some_and(|g| g.repo.eq_ignore_ascii_case(&d.repo) && g.number == d.spec))
    }
}

/// Opens a Delivery's spec (or standalone ticket): its Loft issue, else its issue on GitHub.
fn open_delivery(d: &crate::db::Delivery, issue: Option<&Task>) -> Value {
    match issue {
        Some(t) => open_task(&t.id),
        None if !d.local => json!({ "kind": "open", "label": "Open issue", "target": { "url": format!("https://github.com/{}/issues/{}", d.repo, d.spec) } }),
        None => json!({ "kind": "open", "label": "Open Needs you", "target": { "screen": "needs_you" } }),
    }
}

/// Why a spec branch waits for Merge: "Built by A, B." only when the Delivery has builders.
pub(crate) fn merge_reason(state: &AppState, d: &crate::db::Delivery) -> String {
    let built = crate::delivery::builders(state, d);
    let by = if built.is_empty() { String::new() } else { format!(" Built by {}.", built.join(", ")) };
    format!("Its reviews passed: merge `{}` into {}.{by}", d.branch, crate::delivery::merge_into(d))
}

fn dismiss(id: &str) -> Value {
    action("dismiss", "Done", "POST", format!("/api/needs-you/{}/dismiss", urlencode(id)), json!({}))
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
        // A kill with no program to name has no rule to save, so no Always; a change to the user's checkout is only
        // ever allowed once.
        let always = match p.tool_name.as_str() {
            "Kill" => crate::processes::kill_spec(&p.rule).is_some(),
            "Checkout" => false,
            _ => true,
        };
        let mut actions = vec![action("approve", "Allow once", "POST", base.clone(), json!({ "decision": "allow_once" }))];
        if always {
            actions.push(action("approve_always", "Always allow", "POST", base.clone(), json!({ "decision": "allow_always", "rule": p.rule })));
        }
        actions.push(action("deny", "Deny", "POST", base, json!({ "decision": "deny" })));
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

    // Deliveries whose ticket (or final review) waits for the user after too many changes rounds: the Loft
    // issue it blocked offers Resume.
    let mut by_rounds = state.db.deliveries_in_state("running").map_err(e)?;
    by_rounds.extend(state.db.deliveries_in_state("needs_human").map_err(e)?);
    by_rounds.retain(crate::delivery::waits_for_user);

    // Tasks waiting for you, blocked or failed. A blocked or failed one whose GitHub issue was closed is settled
    // there: it waits on nobody.
    let closed_on_github = |t: &Task| t.github.as_ref().is_some_and(|g| g.state == "closed");
    for t in &tasks {
        if matches!(t.status.as_str(), "blocked" | "failed") && closed_on_github(t) {
            continue;
        }
        let (urgency, reason) = match t.status.as_str() {
            "waiting_approval" => (3, if t.note.is_empty() { "Waiting for your approval.".to_string() } else { t.note.clone() }),
            "blocked" => (2, if t.error.is_empty() { "Blocked: needs your input.".to_string() } else { format!("Blocked: {}", first_line(&t.error)) }),
            "failed" => (1, if t.error.is_empty() { "Failed.".to_string() } else { format!("Failed: {}", first_line(&t.error)) }),
            _ => continue,
        };
        let body = [&t.error, &t.note, &t.summary].iter().find(|s| !s.trim().is_empty()).map(|s| s.to_string()).unwrap_or_default();
        // The answer goes to the issue's agent (a failed issue runs again with it); with nobody on it, the client
        // asks for one (`agent_id`) or says it's kept as a note.
        let mut actions = vec![];
        if t.status == "blocked"
            && let Some(d) = by_rounds.iter().find(|d| crate::delivery::blocked_by_rounds(d, t))
        {
            actions.push(action("resume", "Resume", "POST", "/api/deliveries/resume".into(), json!({ "repo": d.repo, "spec": d.spec })));
        }
        actions.push(action("answer", "Reply on the issue", "POST", format!("/api/tasks/{}/comments", t.id), json!({ "text": "", "send": true })));
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
            links: vec![json!({ "label": "Issue", "path": format!("/issues/{}", t.id) })],
            actions,
            push: (
                match t.status.as_str() {
                    "waiting_approval" => format!("{} needs your approval", t.agent_id.as_deref().map_or("An issue".to_string(), who_name)),
                    "blocked" => format!("{} is blocked", t.agent_id.as_deref().map_or("An issue".to_string(), who_name)),
                    _ => "An issue failed".to_string(),
                },
                t.title.clone(),
            ),
            project: (!t.project.is_empty()).then(|| t.project.clone()),
            pr: None,
        });
    }

    // Closed on GitHub while an agent works on it: the user decides. Not a blocked one: nobody works on it.
    for t in tasks.iter().filter(|t| t.state() == "open" && crate::tasks::is_active(t) && t.status != "blocked" && closed_on_github(t)) {
        let g = t.github.as_ref().unwrap();
        let base = format!("/api/issues/{}", t.id);
        items.push(Item {
            id: format!("closed_on_github:{}:{}", t.id, g.updated_at),
            kind: "closed_on_github",
            urgency: 2,
            agent: t.agent_id.as_deref().and_then(name),
            task_id: Some(t.id.clone()),
            title: text(&t.title, MAX_TITLE),
            reason: text(&format!("#{} was closed on GitHub while it's being worked on.", g.number), MAX_REASON),
            text: String::new(),
            created_at: t.updated_at.clone(),
            links: vec![json!({ "label": "Issue", "path": format!("/issues/{}", t.id) })],
            actions: vec![
                action("deny", "Cancel issue", "POST", format!("{base}/cancel"), json!({})),
                action("approve", &format!("Keep working (reopens #{})", g.number), "POST", format!("{base}/keep-working"), json!({})),
                open_task(&t.id),
            ],
            push: (format!("#{} was closed on GitHub", g.number), t.title.clone()),
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
            links: vec![json!({ "label": "Issue", "path": format!("/issues/{}", t.id) })],
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

    // Tasks you created that finished recently: a result to look at, until you open the issue (`issue_opened`).
    for t in &tasks {
        let Some((id, done_at)) = done_item(t) else { continue };
        items.push(Item {
            kind: "task_done",
            urgency: 0,
            agent: t.agent_id.as_deref().and_then(name),
            task_id: Some(t.id.clone()),
            title: text(&t.title, MAX_TITLE),
            reason: text(&if t.summary.is_empty() { "Done.".to_string() } else { format!("Done: {}", first_line(&t.summary)) }, MAX_REASON),
            text: text(&t.summary, MAX_TEXT),
            created_at: done_at,
            links: vec![json!({ "label": "Issue", "path": format!("/issues/{}", t.id) })],
            actions: vec![
                action("answer", "Follow up", "POST", format!("/api/tasks/{}/comments", t.id), json!({ "text": "", "send": true, "reopen": true })),
                open_task(&t.id),
                dismiss(&id),
            ],
            push: ("Issue done".to_string(), t.title.clone()),
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
                let restarts = state.rt.lock().unwrap().auto_restarts.get(&a.id).copied().unwrap_or(0);
                Some(if restarts >= crate::agents::MAX_AUTO_RESTARTS {
                    format!("No activity for {quiet} min, after {restarts} automatic restarts. Loft won't restart it again.")
                } else {
                    format!("No activity for {quiet} min.")
                })
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

        // An agent that ended its reply to you with a question (and nobody has written since): like an Unread
        // reply, only a run you started (a message or a schedule) counts. Your message answers it, and so does
        // another agent's that needs action (the Lead deciding for you); an FYI doesn't.
        let msgs = state.db.list_messages(&a.id, 30, None).map_err(e)?;
        let answers = |m: &Message| m.role == "user" && m.kind == "text" && (m.meta["from_agent"].is_null() || m.meta["fyi"] != json!(true));
        let last_turn = msgs.iter().rev().find(|m| m.meta["subagent"].is_null() && m.meta.get("grill").is_none() && ((m.role == "assistant" && m.kind == "text") || answers(m)));
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

    // Deliveries waiting on a step no agent is working on: only the user can bring them back.
    for d in state.db.deliveries_in_state("stalled").map_err(e)? {
        let id = crate::delivery::stalled_item_id(&d);
        let issue = delivery_issue(&tasks, &d);
        let title = crate::delivery::title_of(&d, issue.map(|t| t.title.as_str()));
        let open = open_delivery(&d, issue);
        // The run that stalled it ended asking the user something: the question leads, and Reply answers the agent
        // (its call carries the Delivery on); Resume still wakes it afresh.
        let ask = d.stall_ask.as_ref().and_then(|a| name(&a.agent).map(|who| (who, a)));
        let (reason, body, push) = match &ask {
            Some(((_, who), a)) => (
                format!("{who} asks: {}", a.question),
                format!("{}\n\n{}", crate::delivery::stalled_reason(state, &d), a.text),
                format!("{who} asks you something"),
            ),
            None => (crate::delivery::stalled_reason(state, &d), String::new(), title.clone()),
        };
        let mut actions = vec![action("resume", "Resume", "POST", "/api/deliveries/resume".into(), json!({ "repo": d.repo, "spec": d.spec }))];
        if let Some(((agent_id, _), _)) = &ask {
            actions.push(action("answer", "Reply", "POST", format!("/api/agents/{agent_id}/messages"), json!({ "text": "" })));
            actions.push(open_chat(agent_id));
        }
        actions.extend([open, dismiss(&id)]);
        items.push(Item {
            kind: "delivery_stalled",
            urgency: 2,
            agent: ask.as_ref().map(|(who, _)| who.clone()),
            task_id: issue.map(|t| t.id.clone()),
            title: text(&format!("Delivery stalled: {title}"), MAX_TITLE),
            reason: text(&reason, MAX_REASON),
            text: text(&body, MAX_TEXT),
            created_at: if d.stalled_at.is_empty() { d.created_at.clone() } else { d.stalled_at.clone() },
            links: issue.map(|t| vec![json!({ "label": "Issue", "path": format!("/issues/{}", t.id) })]).unwrap_or_default(),
            actions,
            push: ("Delivery stalled".to_string(), push),
            project: Some(d.project.clone()).filter(|p| !p.is_empty()),
            pr: None,
            id,
        });
    }

    // Deliveries Loft marked done (the spec PR ready, or a plan kept in Loft ready to Merge), until the PR
    // merges or closes, or the plan is merged.
    let prs = state.db.list_pull_requests(None).map_err(e)?;
    for d in state.db.deliveries_in_state("ready").map_err(e)?.into_iter().filter(|d| !d.done_at.is_empty()) {
        let tracked = d.pr.and_then(|n| prs.iter().find(|p| p.slug.eq_ignore_ascii_case(&d.repo) && p.number == n));
        // Tickets published since it was done: the Reopen item below says so instead.
        if !d.local && tracked.is_some_and(|p| p.state != "open" || p.gone) || crate::delivery::can_reopen(state, &d) {
            continue;
        }
        let id = format!("done:{}:{}", d.repo, d.spec);
        let issue = delivery_issue(&tasks, &d);
        let title = crate::delivery::title_of(&d, issue.map(|t| t.title.as_str()));
        let pr_url = d.pr.map(|n| tracked.map_or_else(|| format!("https://github.com/{}/pull/{n}", d.repo), |p| p.url.clone()));
        let (reason, act, link) = match &pr_url {
            Some(url) if !d.local => (
                format!("Every ticket is closed and its reviews passed: {} #{} is ready for review.", if d.standalone { "PR" } else { "spec PR" }, d.pr.unwrap_or_default()),
                json!({ "kind": "open", "label": "Open PR", "target": { "url": url } }),
                json!({ "label": "Pull request", "url": url }),
            ),
            _ => (
                format!("Every ticket is done and its reviews passed: merge `{}` into {}.", d.branch, crate::delivery::merge_into(&d)),
                action("merge", "Merge", "POST", "/api/deliveries/merge".into(), json!({ "repo": d.repo, "spec": d.spec })),
                Value::Null,
            ),
        };
        let mut links: Vec<Value> = issue.map(|t| vec![json!({ "label": "Issue", "path": format!("/issues/{}", t.id) })]).unwrap_or_default();
        links.extend(Some(link).filter(|l| !l.is_null()));
        items.push(Item {
            kind: "delivery_done",
            urgency: 1,
            agent: None,
            task_id: issue.map(|t| t.id.clone()),
            title: text(&format!("Delivery done: {title}"), MAX_TITLE),
            reason: text(&reason, MAX_REASON),
            text: String::new(),
            created_at: d.done_at.clone(),
            links,
            // The issue first: it's where the push opens.
            actions: vec![open_delivery(&d, issue), act, dismiss(&id)],
            push: ("Delivery done".to_string(), title),
            project: Some(d.project.clone()).filter(|p| !p.is_empty()),
            pr: None,
            id,
        });
    }

    // A done spec Delivery with tickets published since: Reopen delivery builds them (its spec PR goes back to draft).
    // A new ticket is a new item.
    let mut done = state.db.deliveries_in_state("ready").map_err(e)?;
    done.extend(state.db.deliveries_in_state("settled").map_err(e)?);
    for d in done.into_iter().filter(|d| crate::delivery::can_reopen(state, d)) {
        let open: Vec<&crate::db::DeliveryTicket> = d.tickets.iter().filter(|t| t.state == "open").collect();
        let id = format!("reopen:{}:{}:{}", d.repo, d.spec, open.iter().map(|t| t.number.to_string()).collect::<Vec<_>>().join(","));
        let issue = delivery_issue(&tasks, &d);
        let title = crate::delivery::title_of(&d, issue.map(|t| t.title.as_str()));
        let names = open.iter().map(|t| format!("#{}", t.number)).collect::<Vec<_>>();
        let (pr, many) = (d.pr.unwrap_or_default(), open.len() > 1);
        items.push(Item {
            kind: "delivery_reopen",
            urgency: 1,
            agent: None,
            task_id: issue.map(|t| t.id.clone()),
            title: text(&format!("New tickets on a done Delivery: {title}"), MAX_TITLE),
            reason: text(&format!("{} {} open, published after the Delivery was done. Reopen delivery builds {}; spec PR #{pr} goes back to draft until the final review passes again.", names.join(", "), if many { "are" } else { "is" }, if many { "them" } else { "it" }), MAX_REASON),
            text: open.iter().map(|t| format!("#{} {}", t.number, t.title)).collect::<Vec<_>>().join("\n"),
            created_at: if d.done_at.is_empty() { d.created_at.clone() } else { d.done_at.clone() },
            links: issue.map(|t| vec![json!({ "label": "Issue", "path": format!("/issues/{}", t.id) })]).unwrap_or_default(),
            actions: vec![
                action("reopen", "Reopen delivery", "POST", "/api/deliveries/reopen".into(), json!({ "repo": d.repo, "spec": d.spec })),
                open_delivery(&d, issue),
                dismiss(&id),
            ],
            push: ("New tickets on a done Delivery".to_string(), title),
            project: Some(d.project.clone()).filter(|p| !p.is_empty()),
            pr: None,
            id,
        });
    }

    // Plans kept in Loft whose reviews all passed: Merge merges the spec branch into the project's own repo.
    for d in state.db.deliveries_in_state("ready").map_err(e)?.into_iter().filter(|d| d.local) {
        let id = format!("merge:{}:{}", d.repo, d.spec);
        let spec_task = tasks.iter().find(|t| t.id == d.spec_id);
        items.push(Item {
            kind: "delivery_merge",
            urgency: 1,
            agent: name(&d.dev),
            task_id: spec_task.map(|t| t.id.clone()),
            title: text(&format!("Merge \"{}\"", d.title), MAX_TITLE),
            reason: text(&merge_reason(state, &d), MAX_REASON),
            text: String::new(),
            created_at: spec_task.map_or_else(|| d.created_at.clone(), |t| t.updated_at.clone()),
            links: spec_task.map(|t| vec![json!({ "label": "Spec", "path": format!("/issues/{}", t.id) })]).unwrap_or_default(),
            actions: vec![
                action("merge", "Merge", "POST", "/api/deliveries/merge".into(), json!({ "repo": d.repo, "spec": d.spec })),
                dismiss(&id),
            ],
            push: ("Ready to merge".to_string(), d.title.clone()),
            project: Some(d.project.clone()).filter(|p| !p.is_empty()),
            pr: None,
            id,
        });
    }

    // Pull requests ready to merge: open, not a draft, mergeable, checks green. First the ones agents
    // opened here (they can be merged from the app), then the user's other ready PRs and review
    // requests found on GitHub (github_inbox).
    let mut seen_prs: HashSet<(String, i64)> = HashSet::new();
    for p in prs {
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
            reason: text(&format!("{}Ready to merge{}.", if p.ready_sha == p.head_sha && !p.head_sha.is_empty() { "Loft marked it ready: its verdicts are in. " } else { "" }, if n("total") > 0 { format!(", {} checks passed", n("passed")) } else { String::new() }), MAX_REASON),
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

    // macOS asking on a Mac nobody's looking at: an agent's file read waits until someone answers.
    let asking = state.rt.lock().unwrap().tcc.asking(std::time::Instant::now());
    let host = crate::tailnet::host();
    let mac = host.as_deref().and_then(|h| h.split('.').next()).unwrap_or("this Mac").to_string();
    for a in asking {
        let id = format!("mac_access:{}:{}", a.service, a.since);
        let mut actions = vec![];
        if let Some(h) = &host {
            actions.push(json!({ "kind": "open", "label": "Open Mac access", "target": { "url": format!("http://{h}:{}/settings#mac-access", state.cfg.port) } }));
            actions.push(json!({ "kind": "open", "label": "Screen Sharing", "target": { "url": format!("vnc://{h}") } }));
        }
        actions.push(action("check", "Check again", "POST", "/api/system/mac-access".into(), json!({})));
        actions.push(dismiss(&id));
        items.push(Item {
            kind: "mac_access",
            urgency: 3,
            agent: None,
            task_id: None,
            title: text(&format!("macOS is asking for access to {} on {mac}", a.folder), MAX_TITLE),
            reason: text("A prompt is up on the Mac's screen; an agent's work waits until someone answers it.", MAX_REASON),
            text: text("Answer it over Screen Sharing, or give Loft Full Disk Access (Settings → Mac access) so macOS stops asking.", MAX_TEXT),
            created_at: a.since,
            links: vec![],
            actions,
            push: ("macOS is asking for access".to_string(), format!("{} on {mac}", a.folder)),
            project: None,
            pr: None,
            id,
        });
    }

    // Backups failing, with no good one for two days. One item until a good backup.
    let backup = state.rt.lock().unwrap().backup.clone();
    if let Some(err) = &backup.error
        && backup.last.as_ref().is_none_or(|l| chrono::Local::now() - l.at >= chrono::Duration::hours(crate::backup::ALERT_AFTER_HOURS))
    {
        let id = format!("backup:{}", backup.last.as_ref().map(|l| l.at.to_rfc3339()).unwrap_or_default());
        let since = match &backup.last {
            Some(l) => format!("The last good one is from {}.", l.at.format("%b %-d, %H:%M")),
            None => "There is no good backup yet.".into(),
        };
        let mut actions = vec![action("retry", "Back up now", "POST", "/api/backups".into(), json!({}))];
        if let Some(h) = crate::tailnet::host() {
            actions.push(json!({ "kind": "open", "label": "Open Settings", "target": { "url": format!("http://{h}:{}/settings#backups", state.cfg.port) } }));
        }
        actions.push(dismiss(&id));
        items.push(Item {
            kind: "backup_failed",
            urgency: 1,
            agent: None,
            task_id: None,
            title: text("Loft's backups are failing", MAX_TITLE),
            reason: text(&format!("{since} {}", first_line(&err.message)), MAX_REASON),
            text: text(&format!("{}

Backups folder: {}", err.message, backup.dir.display()), MAX_TEXT),
            created_at: err.at.with_timezone(&Utc).to_rfc3339(),
            links: vec![],
            actions,
            push: ("Loft's backups are failing".to_string(), since),
            project: None,
            pr: None,
            id,
        });
    }

    // Phones are paired but push is off (no APNs key, no relay key): every alert is dropped, so the web says so.
    let phones = state.db.list_devices().map_err(e)?.len();
    let off_since = {
        let mut rt = state.rt.lock().unwrap();
        if phones > 0 && crate::apns::route(state) == crate::apns::Route::Off {
            Some(rt.push_off_since.get_or_insert_with(|| Utc::now().to_rfc3339()).clone())
        } else {
            rt.push_off_since = None;
            None
        }
    };
    if let Some(since) = off_since {
        let id = format!("push_off:{since}");
        let mut actions = vec![];
        if let Some(h) = crate::tailnet::host() {
            actions.push(json!({ "kind": "open", "label": "Open Settings", "target": { "url": format!("http://{h}:{}/settings#push", state.cfg.port) } }));
        }
        actions.push(dismiss(&id));
        let phones = if phones == 1 { "your phone".to_string() } else { format!("your {phones} phones") };
        items.push(Item {
            kind: "push_off",
            urgency: 1,
            agent: None,
            task_id: None,
            title: text("Push is off", MAX_TITLE),
            reason: text(&format!("No APNs key and no relay key: Loft drops every alert for {phones}."), MAX_REASON),
            text: text("Settings → Push notifications: Sign in for push (the Loft relay, no Apple developer account needed), or add your own APNs key.", MAX_TEXT),
            created_at: since,
            links: vec![json!({ "label": "Settings → Push notifications", "path": "/settings#push" })],
            actions,
            push: ("Push is off".to_string(), String::new()),
            project: None,
            pr: None,
            id,
        });
    }

    // Under 20 GB free on the disk agents' workspaces live on. One item while it stays low, and until 2 GB more are
    // free: near the line it would come and go, new each time.
    let free = crate::sweep::free_bytes(&state.cfg.storage_dir);
    let disk_low = {
        let mut rt = state.rt.lock().unwrap();
        let line = crate::sweep::LOW_DISK + if rt.disk_low { crate::sweep::LOW_DISK_MARGIN } else { 0 };
        rt.disk_low = free.is_some_and(|f| f < line);
        rt.disk_low
    };
    if let Some(free) = free
        && disk_low
    {
        let id = "disk_low".to_string();
        let gb = free as f64 / 1e9;
        // Nothing is removed blind: Settings previews what a clean removes.
        let mut actions = vec![];
        if let Some(h) = crate::tailnet::host() {
            actions.push(json!({ "kind": "open", "label": "Open Settings", "target": { "url": format!("http://{h}:{}/settings#workspaces", state.cfg.port) } }));
        }
        actions.push(dismiss(&id));
        items.push(Item {
            kind: "disk_low",
            urgency: 1,
            agent: None,
            task_id: None,
            title: text("Disk almost full", MAX_TITLE),
            reason: text(&format!("{gb:.1} GB are free where agents' workspaces live."), MAX_REASON),
            text: text("Settings → Disk shows each workspace's size and cleans build output after a preview. Deleting an agent deletes its workspace by default.", MAX_TEXT),
            created_at: Utc::now().to_rfc3339(),
            links: vec![],
            actions,
            push: ("Disk almost full".to_string(), format!("{gb:.1} GB free")),
            project: None,
            pr: None,
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
        // Phones choose which kinds they get; macOS asking blocks an agent, so it goes with "stuck".
        kind: if i.kind == "mac_access" { "stuck" } else { i.kind },
        urgency: i.urgency,
        agent_id: i.agent.as_ref().map(|a| a.0.clone()),
        task_id: i.task_id.clone(),
        title: text(&i.push.0, MAX_TITLE),
        body: text(&i.push.1, MAX_REASON),
        target,
    }
}

/// What Needs you last listed and last told clients: every read goes through it, so seen marks follow the list and a
/// change is emitted once.
#[derive(Default)]
pub struct Watch(std::sync::Mutex<Last>);

#[derive(Default)]
struct Last {
    /// The ids last listed.
    ids: HashSet<String>,
    /// The fingerprint last emitted.
    sent: String,
}

/// The whole list, as `GET /api/needs-you` returns it.
pub fn snapshot(state: &AppState) -> Result<Value, String> {
    look(state, &collect(state)?)
}

/// The snapshot of these items. An item keeps its id while it lasts (`task:{id}:blocked`, `disk_low`), so the seen
/// marks of items that left the list since the last look are forgotten first: one that comes back (an issue blocked
/// again with a new question) is new again. Only what left while Loft watched: at startup nothing has left, and lists
/// filled later (the GitHub inbox) don't lose their marks.
fn look(state: &AppState, items: &[Item]) -> Result<Value, String> {
    let ids: HashSet<String> = items.iter().map(|i| i.id.clone()).collect();
    {
        let mut last = state.needs_you.0.lock().unwrap();
        let gone: Vec<String> = last.ids.difference(&ids).cloned().collect();
        if !gone.is_empty() {
            state.db.forget_needs_you_seen(&gone).map_err(|e| e.to_string())?;
        }
        last.ids = ids;
    }
    Ok(snapshot_of(items, &seen(state)?))
}

/// The items the user has seen, on any client.
fn seen(state: &AppState) -> Result<HashSet<String>, String> {
    Ok(state.db.seen_needs_you().map_err(|e| e.to_string())?.into_iter().collect())
}

/// `count` and `urgent` are the whole list; `new` and `new_urgent` only what the user hasn't seen (the counter).
fn snapshot_of(items: &[Item], seen: &HashSet<String>) -> Value {
    let new: Vec<&Item> = items.iter().filter(|i| !seen.contains(&i.id)).collect();
    json!({
        "version": 1,
        "count": items.len(),
        "urgent": items.iter().filter(|i| i.urgency >= 2).count(),
        "new": new.len(),
        "new_urgent": new.iter().filter(|i| i.urgency >= 2).count(),
        "generated_at": crate::db::now(),
        "items": items.iter().map(|i| {
            let mut v = to_json(i);
            v["seen"] = json!(seen.contains(&i.id));
            v
        }).collect::<Vec<_>>(),
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
    let mut items = collect(&state).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !items.iter().any(|i| i.id == id) {
        return Err(not_found());
    }
    state.db.dismiss_needs_you(&id)?;
    items.retain(|i| i.id != id);
    if let Ok(v) = look(&state, &items) {
        emit_if_changed(&state, &v);
    }
    Ok(Json(json!({ "ok": true })))
}

/// The `task_done` item of `t`, its id and when it finished: an issue the user created that finished in the last
/// `REVIEW_HOURS`.
fn done_item(t: &Task) -> Option<(String, String)> {
    if t.status != "done" || t.created_by != "user" {
        return None;
    }
    let done_at = t.completed_at.clone().unwrap_or_else(|| t.updated_at.clone());
    within(&done_at, chrono::Duration::hours(REVIEW_HOURS)).then(|| (format!("done:{}:{}", t.id, done_at), done_at))
}

/// `POST /api/issues/{id}/opened`: the user opened the issue on some client. A finished issue's result was there to
/// look at, so its `task_done` item goes, everywhere. Clients call it on every view of a done issue, so it reads the
/// whole list only when that item is there to clear.
pub async fn issue_opened(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let t = state.db.get_task(&id)?.ok_or_else(not_found)?;
    let Some((item, _)) = done_item(&t) else { return Ok(Json(json!({ "ok": true, "cleared": 0 }))) };
    if state.db.needs_you_dismissed(&item)? {
        return Ok(Json(json!({ "ok": true, "cleared": 0 })));
    }
    state.db.dismiss_needs_you(&item)?;
    if let Ok(v) = collect(&state).and_then(|items| look(&state, &items)) {
        emit_if_changed(&state, &v);
    }
    Ok(Json(json!({ "ok": true, "cleared": 1 })))
}

/// `POST /api/needs-you/seen` with `{ "ids": [...] }`: the user saw these items (the list was opened on some client),
/// so the counter stops counting them everywhere. Ids not in the list are ignored. Answers with the new counts.
pub async fn mark_seen(State(state): State<AppState>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let ids: HashSet<&str> = body["ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    let items = collect(&state).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let shown: Vec<String> = items.iter().filter(|i| ids.contains(i.id.as_str())).map(|i| i.id.clone()).collect();
    state.db.mark_needs_you_seen(&shown)?;
    let v = look(&state, &items).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    emit_if_changed(&state, &v);
    Ok(Json(json!({ "ok": true, "new": v["new"], "new_urgent": v["new_urgent"] })))
}

/// The ids, and which are seen, last sent, so the watcher only emits on a change.
fn fingerprint(v: &Value) -> String {
    v["items"]
        .as_array()
        .map(|a| a.iter().filter_map(|i| Some(format!("{}{}", i["id"].as_str()?, if i["seen"] == true { "+" } else { "" }))).collect::<Vec<_>>().join("|"))
        .unwrap_or_default()
}

fn emit(state: &AppState, v: &Value) {
    let n = |k: &str| v[k].as_u64().unwrap_or(0);
    state.emit(Event::NeedsYou {
        count: n("count"),
        urgent: n("urgent"),
        new: n("new"),
        new_urgent: n("new_urgent"),
        ids: v["items"].as_array().map(|a| a.iter().map(|i| i["id"].clone()).collect()).unwrap_or_default(),
    });
}

/// Emits this snapshot unless clients already have it: a seen mark or a Done emits at once, and the next tick doesn't
/// send it again.
fn emit_if_changed(state: &AppState, v: &Value) {
    let fp = fingerprint(v);
    let mut last = state.needs_you.0.lock().unwrap();
    if last.sent != fp {
        last.sent = fp;
        emit(state, v);
    }
}

/// One tick's push items and snapshot, from a single read of everything.
fn tick_lists(state: &AppState) -> Result<(Vec<PushItem>, Value), String> {
    let items = collect(state)?;
    Ok((items.iter().map(to_push).collect(), look(state, &items)?))
}

/// Tells clients when the list changes (`needs_you` event with the count and ids). Checks every few
/// seconds instead of on every event: messages stream fast, and the list rarely changes. The list is
/// built once per tick; push and the event both come from it.
pub async fn watcher(state: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    let mut pusher = crate::push::Pusher::default();
    loop {
        tick.tick().await;
        let Ok((push, v)) = tick_lists(&state) else { continue };
        pusher.tick(&state, &push, v["new"].as_u64().unwrap_or(0)).await;
        emit_if_changed(&state, &v);
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
    async fn a_kill_with_no_rule_or_a_change_to_a_checkout_offers_no_always_allow() {
        let st = state();
        let dev = agent(&st, "Dev");
        for (id, tool, rule) in [("k1", "Kill", "Kill(/d/*/App)"), ("k2", "Kill", ""), ("c1", "Checkout", ""), ("c2", "Checkout", "Checkout(/d/loft)")] {
            st.db.save_permission(&PermissionRequest {
                id: id.into(), agent_id: dev.id.clone(), source: "prompt".into(), tool_name: tool.into(), input: json!({ "command": "pkill sleep" }),
                reason: String::new(), rule: rule.into(), status: "pending".into(), created_at: now(),
            }).unwrap();
        }
        let items = collect(&st).unwrap();
        let kinds = |id: &str| items.iter().find(|i| i.id == format!("permission:{id}")).unwrap().actions.iter().map(|a| a["kind"].as_str().unwrap()).collect::<Vec<_>>();
        assert!(kinds("k1").contains(&"approve_always"));
        assert_eq!(kinds("k2"), vec!["approve", "deny", "open"]);
        assert_eq!(kinds("c1"), vec!["approve", "deny", "open"]);
        assert_eq!(kinds("c2"), vec!["approve", "deny", "open"]);
    }

    #[tokio::test]
    async fn the_counter_counts_only_what_the_user_has_not_seen_on_any_client() {
        let st = state();
        let qa = agent(&st, "QA");
        let mk = |status: &str| {
            let t: Task = serde_json::from_value(json!({ "id": uuid::Uuid::new_v4().to_string(), "title": status, "status": status, "created_by": "user",
                "agent_id": qa.id, "created_at": now(), "updated_at": now() })).unwrap();
            st.db.save_task(&t).unwrap();
            t
        };
        let blocked = mk("blocked"); // urgency 2
        let failed = mk("failed"); // urgency 1
        let v = snapshot(&st).unwrap();
        assert_eq!((v["count"].clone(), v["urgent"].clone(), v["new"].clone(), v["new_urgent"].clone()), (json!(2), json!(1), json!(2), json!(1)), "nothing is seen yet");
        assert!(v["items"].as_array().unwrap().iter().all(|i| i["seen"] == false));
        let (blocked_id, failed_id) = (format!("task:{}:blocked", blocked.id), format!("task:{}:failed", failed.id));
        assert_eq!(item(&v, "task_failed")["actions"].as_array().unwrap().last().unwrap()["label"], "Done", "Dismiss is the Done check");

        // Opening the list on one client marks what it showed; ids not in the list are ignored.
        let r = mark_seen(State(st.clone()), Json(json!({ "ids": [blocked_id, failed_id, "question:gone"] }))).await.unwrap().0;
        assert_eq!((r["new"].clone(), r["new_urgent"].clone()), (json!(0), json!(0)));
        assert!(!st.db.seen_needs_you().unwrap().contains(&"question:gone".to_string()));
        // Every client reads the same marks: seen items stay in the list, the counter is empty.
        let v = snapshot(&st).unwrap();
        assert_eq!((v["count"].clone(), v["new"].clone(), v["new_urgent"].clone()), (json!(2), json!(0), json!(0)));
        assert!(v["items"].as_array().unwrap().iter().all(|i| i["seen"] == true));

        // A new item counts again; it is urgent only if it is.
        mk("failed");
        let v = snapshot(&st).unwrap();
        assert_eq!((v["count"].clone(), v["new"].clone(), v["new_urgent"].clone()), (json!(3), json!(1), json!(0)));
        // A changed item has a new id, so it counts again: an issue failed, then blocked.
        let mut failed2 = failed.clone();
        failed2.status = "blocked".into();
        st.db.save_task(&failed2).unwrap();
        let v = snapshot(&st).unwrap();
        assert_eq!((v["new"].clone(), v["new_urgent"].clone()), (json!(2), json!(1)), "the changed one is new and urgent");
        assert_eq!(v["items"].as_array().unwrap().iter().find(|i| i["id"] == format!("task:{}:blocked", failed.id)).unwrap()["seen"], false);

        // Done (dismiss) still takes an item out of the list, and out of the counts.
        assert!(dismiss_item(State(st.clone()), Path(blocked_id.clone())).await.is_ok());
        let v = snapshot(&st).unwrap();
        assert!(!ids(&v).contains(&blocked_id));
        assert_eq!(v["count"], json!(2));
    }

    #[tokio::test]
    async fn an_item_that_leaves_and_comes_back_with_the_same_id_is_new_again() {
        let st = state();
        let qa = agent(&st, "QA");
        let mut t: Task = serde_json::from_value(json!({ "id": uuid::Uuid::new_v4().to_string(), "title": "Pick a name", "status": "blocked",
            "created_by": "user", "agent_id": qa.id, "error": "Which name?", "created_at": now(), "updated_at": now() })).unwrap();
        st.db.save_task(&t).unwrap();
        let id = format!("task:{}:blocked", t.id);
        snapshot(&st).unwrap();
        let _ = mark_seen(State(st.clone()), Json(json!({ "ids": [id] }))).await.unwrap();
        assert_eq!(snapshot(&st).unwrap()["new"], json!(0));

        // Answered: it runs again and leaves the list (the watcher's next look)...
        t.status = "in_progress".into();
        st.db.save_task(&t).unwrap();
        tick_lists(&st).unwrap();
        // ...then blocks again with a new question, under the same id: it counts again, everywhere.
        t.status = "blocked".into();
        t.error = "And the icon?".into();
        st.db.save_task(&t).unwrap();
        let v = snapshot(&st).unwrap();
        assert_eq!((v["new"].clone(), v["new_urgent"].clone()), (json!(1), json!(1)), "blocked again: new and urgent");
        assert_eq!(item(&v, "task_blocked")["seen"], false);

        // Lists filled after startup keep their marks: a mark is only forgotten for an item seen leaving.
        let fresh = state();
        fresh.db.mark_needs_you_seen(&["review:o/r#7".to_string()]).unwrap();
        snapshot(&fresh).unwrap();
        assert!(fresh.db.seen_needs_you().unwrap().contains(&"review:o/r#7".to_string()), "nothing left at the first look");
    }

    #[tokio::test]
    async fn a_seen_mark_is_emitted_once() {
        let st = state();
        let qa = agent(&st, "QA");
        let t: Task = serde_json::from_value(json!({ "id": uuid::Uuid::new_v4().to_string(), "title": "x", "status": "failed",
            "created_by": "user", "agent_id": qa.id, "created_at": now(), "updated_at": now() })).unwrap();
        st.db.save_task(&t).unwrap();
        let mut rx = st.events.subscribe();
        let (_, v) = tick_lists(&st).unwrap();
        emit_if_changed(&st, &v);
        let mut events = || std::iter::from_fn(|| rx.try_recv().ok()).filter(|e| matches!(e, Event::NeedsYou { .. })).count();
        assert_eq!(events(), 1, "the watcher tells clients about the new item");

        let _ = mark_seen(State(st.clone()), Json(json!({ "ids": [format!("task:{}:failed", t.id)] }))).await.unwrap();
        let (_, v) = tick_lists(&st).unwrap();
        emit_if_changed(&st, &v);
        assert_eq!(events(), 1, "the mark emits at once, and the next tick doesn't send it again");
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

    #[tokio::test]
    async fn a_finished_issues_result_leaves_when_the_issue_is_opened_or_after_24_hours() {
        let st = state();
        let done = |title: &str, hours_ago: i64| {
            let (mut t, _) = crate::tasks::create(&st, &json!({ "title": title }), "user").unwrap();
            t.status = "done".into();
            t.completed_at = Some((Utc::now() - chrono::Duration::hours(hours_ago)).to_rfc3339());
            st.db.save_task(&t).unwrap();
            t
        };
        let results = |st: &AppState| collect(st).unwrap().into_iter().filter(|i| i.kind == "task_done").map(|i| i.title).collect::<Vec<_>>();
        let (fresh, day_old, other) = (done("Fresh", 1), done("Day old", 25), done("Other", 2));
        assert_eq!(results(&st), ["Other", "Fresh"], "a day later it's gone");

        let Json(r) = issue_opened(State(st.clone()), Path(fresh.id.clone())).await.unwrap();
        assert_eq!(r["cleared"], json!(1));
        assert_eq!(results(&st), ["Other"], "opening one leaves the other");
        // Opening it again, or opening an issue with no result to look at, changes nothing.
        assert_eq!(issue_opened(State(st.clone()), Path(fresh.id.clone())).await.unwrap().0["cleared"], json!(0));
        assert!(issue_opened(State(st.clone()), Path("nope".into())).await.is_err());
        assert_eq!(other.status, "done");
        // Nor does opening an issue with no item: not done, an agent's, or done over 24 hours ago.
        let (open, _) = crate::tasks::create(&st, &json!({ "title": "Open" }), "user").unwrap();
        let (mut agents, _) = crate::tasks::create(&st, &json!({ "title": "An agent's" }), "agent").unwrap();
        agents.status = "done".into();
        st.db.save_task(&agents).unwrap();
        for t in [&open.id, &agents.id, &day_old.id] {
            assert_eq!(issue_opened(State(st.clone()), Path(t.clone())).await.unwrap().0["cleared"], json!(0));
        }
        assert_eq!(results(&st), ["Other"]);
    }

    #[test]
    fn only_a_run_you_started_asks_you_something_and_your_message_or_an_agents_needs_action_answers_it() {
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

        // An agent's FYI after the question doesn't answer it; an agent's message that needs action does (the Lead
        // decided), and so does the user's.
        crate::agents::fyi_from_agent(&st, &qa, &dev, "Status update: CI is green.");
        assert_eq!(questions(&st), ["Dev"]);
        crate::agents::add_msg(&st, &dev.id, None, "user", "text", "Use S3.", from_qa);
        assert!(questions(&st).is_empty());
        reply(&st, &dev.id, true, "And the bucket name?");
        assert_eq!(questions(&st), ["Dev"]);
        crate::agents::add_msg(&st, &dev.id, None, "user", "text", "loft-backups", json!({}));
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
