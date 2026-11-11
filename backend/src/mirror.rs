//! The mirror: in a project with a GitHub repo, every open issue of the repo is a Loft issue too, and
//! so is every closed one that already has a Loft issue. The shared fields (title, description,
//! labels, owner, open or closed) are kept in step both ways: GitHub's changes arrive on the poll, and
//! Loft's edits are written to GitHub right away. When both sides changed, GitHub wins. Done closes
//! the GitHub issue as completed, cancelled as not planned, and the reverse.
//!
//! Each repo is polled every 2 minutes for what changed since the last read, with `If-None-Match`, so
//! a quiet repo costs a 304 that doesn't count against the rate limit. A full read of the open issues
//! (on start, on open and refresh, after Loft publishes, and every half hour) also finds the issues
//! that left: closed, deleted, or transferred to another repo. Projects without a repo, and Loft
//! without a GitHub token, are left alone.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::ApiError;
use crate::db::{GithubLink, Task, now};
use crate::state::AppState;

const POLL_EVERY: Duration = Duration::from_secs(120);
/// Every this many polls, a full read instead of only the changes: 30 × 2 minutes, hourly.
const FULL_EVERY: u64 = 30;
/// Pages of 100 open issues read per repo at most.
const MAX_PAGES: usize = 10;
/// Closed links missing their author few enough to read one by one rather than page by page.
const FEW_AUTHORS: usize = 10;
/// Closed links read one by one for their authors per run at most; the rest wait for the next run.
const MAX_AUTHOR_READS: usize = 100;
/// `created_by` of a Loft issue the mirror made.
pub const FROM_GITHUB: &str = "github";

/// What the mirror remembers per repo between polls, and a lock so two syncs of one repo never race.
#[derive(Default)]
pub struct Mirror {
    repos: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<Cursor>>>>,
    /// Why each skipped project was last skipped, so the log says it once, not every poll.
    skipped: std::sync::Mutex<HashMap<String, String>>,
    /// ETag of each issue of another repo last read by number (`repo#n`), so an unchanged one answers 304.
    etags: std::sync::Mutex<HashMap<String, String>>,
    /// Other repos already logged as unreadable this run, so the log says it once.
    unreadable: std::sync::Mutex<HashSet<String>>,
}

/// Where the change poll of one repo stands.
#[derive(Default)]
struct Cursor {
    /// `since` of the change poll: the newest `updated_at` seen. None until a full read.
    since: Option<String>,
    /// ETag of the last change poll's answer, sent back so an unchanged repo answers 304.
    etag: Option<String>,
    /// The closed issues were read for the authors their links lack (see `closed_authors`), this run.
    authors_read: bool,
}

impl Mirror {
    fn cursor(&self, repo: &str) -> Arc<tokio::sync::Mutex<Cursor>> {
        self.repos.lock().unwrap().entry(repo.to_lowercase()).or_default().clone()
    }
}

/// What one sync did on one project.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize)]
pub struct Report {
    pub project: String,
    pub repo: String,
    pub created: usize,
    pub updated: usize,
    /// Loft issues whose GitHub issue was deleted or transferred: unlinked, or removed when nobody had touched them.
    pub reconciled: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Numbers and bodies of the tickets this sync saw closed on GitHub: each may have been its spec's last.
    #[serde(skip)]
    pub closed_tickets: Vec<(i64, String)>,
}

/// Every project of every agent, A–Z (case-insensitive), each once.
pub(crate) fn projects(state: &AppState) -> anyhow::Result<Vec<String>> {
    let mut ps: Vec<String> = state.db.list_agents()?.iter().flat_map(|a| a.projects.iter().chain([&a.project])).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    ps.sort();
    ps.dedup();
    ps.sort_by_key(|p| p.to_lowercase());
    Ok(ps)
}

/// Why `project` isn't mirrored, if it isn't: no GitHub token, or no GitHub repo. Logged the first
/// time (and whenever the reason changes), so a project that silently stays out can be explained.
pub async fn skip_reason(state: &AppState, project: &str) -> Option<String> {
    let why = if crate::github::token(state).is_none() {
        Some("GitHub isn't connected (no token in Settings, no `gh auth login`)".to_string())
    } else {
        crate::repos::github_repo(state, project).await.err()
    };
    let mut said = state.mirror.skipped.lock().unwrap();
    match &why {
        Some(w) if said.get(project) != Some(w) => {
            tracing::info!("mirror: not mirroring {project}: {w}");
            said.insert(project.to_string(), w.clone());
        }
        None => {
            said.remove(project);
        }
        _ => {}
    }
    why
}

/// Polls every project's repo; the first round and every `FULL_EVERY`th are full reads.
pub async fn poller(state: AppState) {
    crate::github::as_source("mirror", poll(state)).await
}

async fn poll(state: AppState) {
    let mut round: u64 = 0;
    loop {
        let full = round.is_multiple_of(FULL_EVERY);
        // Projects that share a repo read it once.
        let mut read = HashSet::new();
        for p in projects(&state).unwrap_or_default() {
            if let Ok(repo) = crate::repos::github_repo(&state, &p).await
                && !read.insert(repo.to_lowercase())
            {
                continue;
            }
            let _ = sync_project(&state, &p, full).await;
        }
        round += 1;
        tokio::time::sleep(POLL_EVERY).await;
    }
}

/// A full read of one project, in the background: after Loft publishes there, for example.
pub fn spawn_sync(state: &AppState, project: &str) {
    let (state, project) = (state.clone(), project.trim().to_string());
    tokio::spawn(crate::github::as_source("mirror", async move {
        let _ = sync_project(&state, &project, true).await;
    }));
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SyncBody {
    project: Option<String>,
}

/// `POST /api/issues/sync`: what changed on GitHub, now, for one project or all; what the list does on
/// open and refresh. A change poll, so a quiet repo costs a free 304; a repo never read yet gets a full read.
/// `login` is the GitHub user of Loft's token (null when nothing is mirrored or it can't be read).
pub async fn sync_now(State(state): State<AppState>, body: Option<Json<SyncBody>>) -> Result<Json<Value>, ApiError> {
    let wanted = body.and_then(|b| b.0.project).map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let projects = projects(&state)?;
    if let Some(p) = &wanted
        && !projects.contains(p)
    {
        return Err(crate::api::not_found());
    }
    let (mut out, mut skipped) = (vec![], vec![]);
    let mut read: HashMap<String, Report> = HashMap::new();
    for p in projects.iter().filter(|p| wanted.as_ref().is_none_or(|w| w == *p)) {
        // A project on a repo already read this time reports that read instead of reading it again.
        if let Ok(repo) = crate::repos::github_repo(&state, p).await
            && let Some(r) = read.get(&repo.to_lowercase())
        {
            out.push(Report { project: p.clone(), ..r.clone() });
            continue;
        }
        match sync_project(&state, p, false).await {
            Some(r) => {
                read.insert(r.repo.to_lowercase(), r.clone());
                out.push(r)
            }
            None => skipped.push(json!({ "project": p, "reason": skip_reason(&state, p).await })),
        }
    }
    // Who Loft's token belongs to, so "Created by me" can keep the GitHub issues they opened; read
    // (once per token) only when a project is mirrored.
    let connected = crate::github::token(&state).is_some();
    let login = if connected && !out.is_empty() { crate::planning::login(&state).await.ok() } else { None };
    Ok(Json(json!({ "connected": connected, "login": login, "projects": out, "skipped": skipped })))
}

/// Syncs one project. None when there's nothing to mirror: no token, or no GitHub repo.
pub async fn sync_project(state: &AppState, project: &str, full: bool) -> Option<Report> {
    if skip_reason(state, project).await.is_some() {
        return None;
    }
    let repo = crate::repos::github_repo(state, project).await.ok()?;
    let mut report = Report { project: project.into(), repo: repo.clone(), ..Default::default() };
    let cursor = state.mirror.cursor(&repo);
    let mut cursor = cursor.lock().await;
    let full = full || cursor.since.is_none();
    let result = if full { full_read(state, project, &repo, &mut cursor, &mut report).await } else { changes(state, project, &repo, &mut cursor, &mut report).await };
    if let Err(e) = result {
        // Paused for the rate limit is expected and says so in the list; it isn't worth a warning each tick.
        if !crate::github::is_paused(&e) {
            tracing::warn!("mirror {repo}: {e}");
        }
        report.error = Some(e.to_string());
    }
    drop(cursor);
    if full {
        other_repos(state, project, &repo, &mut report).await;
    }
    // A ticket closed on GitHub by hand: when it was its spec's last, the spec PR is ready for review.
    // Delivery records change under the repo's publish lock, like every other Delivery step, so a poll
    // can't write back a record a submit or review changed meanwhile.
    {
        let _held = state.publish_locks.of(&repo).lock_owned().await;
        for (n, body) in std::mem::take(&mut report.closed_tickets) {
            if let Err(e) = crate::planning::ready_when_last(state, &repo, n, &body).await {
                tracing::warn!("mirror {repo}: {e}");
            }
        }
        crate::planning::retry_ready(state, &repo, full).await;
    }
    // Takes the lock itself.
    crate::delivery::retry(state, &repo).await;
    Some(report)
}

/// Every open issue, then the linked ones that weren't among them: closed, deleted or transferred.
async fn full_read(state: &AppState, project: &str, repo: &str, cursor: &mut Cursor, report: &mut Report) -> anyhow::Result<()> {
    let mut open = vec![];
    for page in 1..=MAX_PAGES {
        let v = crate::github::get(state, &format!("/repos/{repo}/issues?state=open&per_page=100&page={page}")).await?;
        let items = v.as_array().cloned().unwrap_or_default();
        let n = items.len();
        open.extend(items);
        if n < 100 {
            break;
        }
    }
    let mut newest = cursor.since.clone().unwrap_or_default();
    let mut seen = HashSet::new();
    let mut touched = vec![];
    for i in open.iter().filter(|i| i.get("pull_request").is_none()) {
        seen.insert(i["number"].as_i64().unwrap_or(0));
        newest = newest.max(i["updated_at"].as_str().unwrap_or("").to_string());
        let before = report.created + report.updated;
        apply(state, project, repo, i, report);
        if report.created + report.updated > before {
            touched.push(i);
        }
    }
    // The parent is in each issue's own JSON: every listed issue gets it, changed or not, so a ticket
    // mirrored before its parent was set (or before Loft read it) catches up without a request.
    parents_from_github(state, repo, open.iter().filter(|i| i.get("pull_request").is_none()));
    // Links and comments only for issues that are new or changed since the last read: an unchanged
    // issue costs no more requests, which keeps a full read (on every list open) cheap. After every
    // issue is in, so a link to one listed later still resolves.
    for i in touched {
        links_from_github(state, repo, i).await;
        comments_from_github(state, repo, i).await;
    }
    // Only a backfill: when it fails it's tried again on the next full read, and the rest of this one goes on.
    if !cursor.authors_read {
        match closed_authors(state, repo, report).await {
            Ok(()) => cursor.authors_read = true,
            Err(e) => tracing::warn!("mirror {repo}: authors of closed issues: {e}"),
        }
    }
    // Linked issues GitHub still had open last time but didn't list now: ask about each one.
    let gone: Vec<Task> = linked(state, repo).into_iter().filter(|t| t.github.as_ref().is_some_and(|g| g.state == "open" && !seen.contains(&g.number))).collect();
    for t in gone {
        let n = t.github.as_ref().unwrap().number;
        match crate::github::get(state, &format!("/repos/{repo}/issues/{n}")).await {
            Ok(i) if same_repo(&i, repo) => {
                newest = newest.max(i["updated_at"].as_str().unwrap_or("").to_string());
                apply(state, project, repo, &i, report);
                parents_from_github(state, repo, [&i]);
            }
            Ok(i) => reconcile(state, t, &format!("moved to {}", i["html_url"].as_str().unwrap_or("another repo")), report).await,
            Err(e) if crate::github::is_gone(&e) => reconcile(state, t, "was deleted", report).await,
            Err(e) => tracing::warn!("mirror {repo}#{n}: {e}"),
        }
    }
    if !newest.is_empty() {
        cursor.since = Some(newest);
    } else if cursor.since.is_none() {
        cursor.since = Some(now());
    }
    cursor.etag = None;
    Ok(())
}

/// Loft issues of `project` linked to a repo other than its own (its origin moved, say) aren't in its
/// repo's reads: a full read reads each one still open by number and brings GitHub's changes in. Only
/// those: nothing else of that repo is imported. A repo that can't be read is left as it is until the
/// next full read, and logged once per run.
async fn other_repos(state: &AppState, project: &str, repo: &str, report: &mut Report) {
    let mut by_repo: HashMap<String, Vec<i64>> = HashMap::new();
    for t in state.db.list_tasks().unwrap_or_default().into_iter().filter(|t| t.project == project) {
        if let Some(g) = t.github.filter(|g| g.state == "open" && !g.repo.eq_ignore_ascii_case(repo)) {
            by_repo.entry(g.repo).or_default().push(g.number);
        }
    }
    for (other, numbers) in by_repo {
        let cursor = state.mirror.cursor(&other);
        let _held = cursor.lock().await;
        for n in numbers {
            let key = format!("{}#{n}", other.to_lowercase());
            let etag = state.mirror.etags.lock().unwrap().get(&key).cloned();
            let e = match crate::github::get_if_changed(state, &format!("/repos/{other}/issues/{n}"), etag.as_deref()).await {
                Ok(None) => continue,
                Ok(Some((i, etag))) => {
                    if let Some(etag) = etag {
                        state.mirror.etags.lock().unwrap().insert(key, etag);
                    }
                    // Transferred away: left as it is, like a link whose repo is gone. Unlinked or deleted
                    // in Loft while it was read: dropped, so nothing of that repo is ever imported.
                    let still_linked = i["number"].as_i64().is_some_and(|n| linked(state, &other).iter().any(|t| t.github.as_ref().is_some_and(|g| g.number == n)));
                    if same_repo(&i, &other) && still_linked {
                        let mut r = Report::default();
                        apply(state, project, &other, &i, &mut r);
                        report.updated += r.updated;
                    }
                    continue;
                }
                Err(e) => e,
            };
            if crate::github::is_paused(&e) {
                break;
            }
            if state.mirror.unreadable.lock().unwrap().insert(other.to_lowercase()) {
                tracing::warn!("mirror: can't read {other}, its linked issues stay as they are: {e}");
            }
            // One issue deleted: the others are still read. Anything else: the repo waits for the next full read.
            if !crate::github::is_gone(&e) {
                break;
            }
        }
    }
}

/// What changed since the last read, open or closed. An unchanged answer comes back as 304 with the same ETag.
/// More than a page changed: the next page starts from the newest `updated_at` read so far, so an issue
/// edited while paging moves to the end instead of pushing an unread one onto a page already read.
async fn changes(state: &AppState, project: &str, repo: &str, cursor: &mut Cursor, report: &mut Report) -> anyhow::Result<()> {
    let since = cursor.since.clone().unwrap_or_default();
    let mut from = since.clone();
    // Only when a full page shares one `updated_at` does the next page need GitHub's own page number.
    let mut page = 1;
    let mut changed: Vec<Value> = vec![];
    let mut first_etag = None;
    for read in 0..MAX_PAGES {
        let path = format!("/repos/{repo}/issues?state=all&sort=updated&direction=asc&per_page=100&since={}{}", url_encode(&from), if page > 1 { format!("&page={page}") } else { String::new() });
        let v = if read == 0 {
            let Some((v, etag)) = crate::github::get_if_changed(state, &path, cursor.etag.as_deref()).await? else {
                return Ok(());
            };
            first_etag = etag;
            v
        } else {
            crate::github::get(state, &path).await?
        };
        let items = v.as_array().cloned().unwrap_or_default();
        let full = items.len() == 100;
        let newest = items.iter().filter_map(|i| i["updated_at"].as_str()).max().unwrap_or("").to_string();
        for i in items {
            let n = i["number"].as_i64();
            changed.retain(|c| c["number"].as_i64() != n);
            changed.push(i);
        }
        if !full {
            break;
        }
        if newest > from {
            from = newest;
            page = 1;
        } else {
            page += 1;
        }
    }
    let changed: Vec<&Value> = changed.iter().filter(|i| i.get("pull_request").is_none()).collect();
    let mut newest = since.clone();
    for i in &changed {
        newest = newest.max(i["updated_at"].as_str().unwrap_or("").to_string());
        apply(state, project, repo, i, report);
    }
    parents_from_github(state, repo, changed.iter().copied());
    for i in &changed {
        links_from_github(state, repo, i).await;
        comments_from_github(state, repo, i).await;
    }
    // A ticket closing or reopening changes its spec's sub-issue count, but not the spec's `updated_at`,
    // so the spec isn't among the changes: read each such parent again for its count.
    let numbers: HashSet<i64> = changed.iter().filter_map(|i| i["number"].as_i64()).collect();
    let mut parents: Vec<i64> = changed.iter().filter_map(|i| parent_number(repo, i)).filter(|p| !numbers.contains(p)).collect();
    parents.sort();
    parents.dedup();
    let mirrored = linked(state, repo);
    for p in parents.into_iter().filter(|p| mirrored.iter().any(|t| t.github.as_ref().is_some_and(|g| g.number == *p))) {
        refresh_link(state, repo, p).await;
    }
    // `since` is inclusive, so the newest issue comes back each time: the answer only changes, and the
    // ETag with it, when something new does. Keep the URL (and ETag) until then.
    if newest != since {
        cursor.since = Some(newest);
        cursor.etag = None;
    } else {
        cursor.etag = first_etag;
    }
    Ok(())
}

/// Links saved before authors were kept: the open ones get theirs from the full read's listing, but a
/// closed issue is only read again when it changes. So, once per run, read the closed issues page by
/// page while many closed links lack their author, then the few left (or those older than the pages
/// read) one by one, and keep just the author. One that left GitHub is reconciled like an open one.
async fn closed_authors(state: &AppState, repo: &str, report: &mut Report) -> anyhow::Result<()> {
    let mut missing: HashMap<i64, Task> = linked(state, repo)
        .into_iter()
        .filter(|t| t.github.as_ref().is_some_and(|g| g.state == "closed" && g.author.is_empty()))
        .map(|t| (t.github.as_ref().unwrap().number, t))
        .collect();
    for page in 1..=MAX_PAGES {
        if missing.len() <= FEW_AUTHORS {
            break;
        }
        let v = crate::github::get(state, &format!("/repos/{repo}/issues?state=closed&per_page=100&page={page}")).await?;
        let items = v.as_array().cloned().unwrap_or_default();
        for i in items.iter().filter(|i| i.get("pull_request").is_none()) {
            if let Some(t) = i["number"].as_i64().and_then(|n| missing.remove(&n)) {
                keep_author(state, t, i);
            }
        }
        if items.len() < 100 {
            break;
        }
    }
    let mut left: Vec<(i64, Task)> = missing.into_iter().collect();
    left.sort_by_key(|(n, _)| -n);
    for (n, t) in left.into_iter().take(MAX_AUTHOR_READS) {
        match crate::github::get(state, &format!("/repos/{repo}/issues/{n}")).await {
            Ok(i) if same_repo(&i, repo) => keep_author(state, t, &i),
            Ok(i) => reconcile(state, t, &format!("moved to {}", i["html_url"].as_str().unwrap_or("another repo")), report).await,
            Err(e) if crate::github::is_gone(&e) => reconcile(state, t, "was deleted", report).await,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The author of GitHub issue `i` onto its link `t`.
fn keep_author(state: &AppState, mut t: Task, i: &Value) {
    let author = i["user"]["login"].as_str().unwrap_or("");
    if author.is_empty() {
        return;
    }
    t.github.as_mut().unwrap().author = author.into();
    if state.db.save_task(&t).is_ok() {
        crate::tasks::emit(state, &t);
    }
}

/// The Loft issues linked to issues of `repo`.
fn linked(state: &AppState, repo: &str) -> Vec<Task> {
    state.db.list_tasks().unwrap_or_default().into_iter().filter(|t| t.github.as_ref().is_some_and(|g| g.repo.eq_ignore_ascii_case(repo))).collect()
}

fn same_repo(i: &Value, repo: &str) -> bool {
    i["html_url"].as_str().is_some_and(|u| u.to_lowercase().starts_with(&format!("https://github.com/{}/issues/", repo.to_lowercase())))
}

/// The shared fields of a GitHub issue, as a Loft issue holds them.
fn fields(i: &Value) -> (String, String, Vec<String>, String) {
    let title: String = i["title"].as_str().unwrap_or("").trim().chars().take(200).collect();
    let body: String = i["body"].as_str().unwrap_or("").chars().take(20_000).collect();
    let labels = i["labels"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str().or(l.as_str()).map(String::from)).collect();
    // GitHub allows several assignees; a Loft issue has one owner, the first.
    let owner = i["assignees"].as_array().and_then(|a| a.first()).and_then(|a| a["login"].as_str()).unwrap_or("").to_string();
    (title, body, labels, owner)
}

fn link(repo: &str, i: &Value) -> GithubLink {
    let number = i["number"].as_i64().unwrap_or(0);
    GithubLink {
        repo: repo.into(),
        number,
        id: i["id"].as_i64().unwrap_or(0),
        url: i["html_url"].as_str().map(String::from).unwrap_or_else(|| format!("https://github.com/{repo}/issues/{number}")),
        state: i["state"].as_str().unwrap_or("open").to_string(),
        updated_at: i["updated_at"].as_str().unwrap_or("").to_string(),
        sub_total: i["sub_issues_summary"]["total"].as_i64().unwrap_or(0),
        sub_done: i["sub_issues_summary"]["completed"].as_i64().unwrap_or(0),
        author: i["user"]["login"].as_str().unwrap_or("").to_string(),
    }
}

/// Brings one GitHub issue in: a new Loft issue for an open one nobody has yet, or GitHub's changes
/// onto the one that mirrors it. A closed issue without a Loft issue stays out.
fn apply(state: &AppState, project: &str, repo: &str, i: &Value, report: &mut Report) {
    let Some(number) = i["number"].as_i64() else { return };
    let existing = linked(state, repo).into_iter().find(|t| t.github.as_ref().is_some_and(|g| g.number == number));
    let (title, description, labels, owner) = fields(i);
    match existing {
        Some(mut t) => {
            let l = link(repo, i);
            if t.github.as_ref() == Some(&l) {
                return;
            }
            // A link saved before authors were kept, and only the author is new: nothing changed on
            // GitHub, so it's no update (and costs no reads of its links and comments).
            if t.github.as_ref().is_some_and(|g| g.author.is_empty() && GithubLink { author: l.author.clone(), ..g.clone() } == l) {
                t.github = Some(l);
                if state.db.save_task(&t).is_ok() {
                    crate::tasks::emit(state, &t);
                }
                return;
            }
            // Open/closed follows GitHub only when GitHub moved: a Loft close still being written isn't undone.
            let moved = t.github.as_ref().is_some_and(|g| g.state != l.state) && t.state() != l.state;
            // Closed on GitHub while an agent works on it: the work isn't stopped behind the user's back.
            // The issue stays open, the mismatch is shown (`github.state` closed, issue open) and asked about.
            let working = crate::tasks::is_active(&t);
            let status = match (moved, l.state.as_str()) {
                (true, "closed") if working => None,
                (true, "closed") => Some(if i["state_reason"] == "not_planned" { "cancelled" } else { "done" }),
                (true, _) => Some("inbox"),
                _ => None,
            };
            let closed_under_work = moved && working && l.state == "closed";
            if t.github.as_ref().is_some_and(|g| g.state == "open") && l.state == "closed" && labels.iter().any(|l| l == "ready-for-agent") {
                report.closed_tickets.push((i["number"].as_i64().unwrap_or_default(), description.clone()));
            }
            t.title = if title.is_empty() { t.title } else { title };
            t.description = description;
            t.labels = labels;
            t.owner = owner;
            t.github = Some(l);
            if let Some(s) = status {
                t.status = s.into();
                t.completed_at = (s != "inbox").then(now);
            }
            t.updated_at = now();
            if state.db.save_task(&t).is_ok() {
                crate::tasks::emit(state, &t);
                if let Some(s) = status {
                    let text = match s { "done" => "Closed on GitHub", "cancelled" => "Closed on GitHub as not planned", _ => "Reopened on GitHub" };
                    crate::tasks::log_status(state, &t, FROM_GITHUB, text);
                }
                if closed_under_work {
                    let n = t.github.as_ref().map_or(0, |g| g.number);
                    crate::tasks::log_status(state, &t, FROM_GITHUB, &format!("#{n} was closed on GitHub while it's being worked on: cancel it, or keep working (reopens #{n})"));
                }
                report.updated += 1;
            }
        }
        None if i["state"] == "open" => {
            let ts = now();
            let t = Task {
                id: uuid::Uuid::new_v4().to_string(),
                title: if title.is_empty() { format!("#{number}") } else { title },
                description,
                labels,
                owner,
                github: Some(link(repo, i)),
                kind: "todo".into(),
                status: "inbox".into(),
                priority: "normal".into(),
                project: project.into(),
                created_by: FROM_GITHUB.into(),
                created_at: ts.clone(),
                updated_at: ts,
                ..Default::default()
            };
            if state.db.save_task(&t).is_ok() {
                crate::tasks::emit(state, &t);
                crate::tasks::log_status(state, &t, FROM_GITHUB, &format!("Mirrored from GitHub #{number}"));
                report.created += 1;
            }
        }
        None => {}
    }
}

/// Brings GitHub issue `i` in now (Planning just published it): the Loft issue that mirrors it, and
/// whether it was made just now rather than already there.
pub(crate) fn bring_in(state: &AppState, project: &str, repo: &str, i: &Value) -> Option<(Task, bool)> {
    let mut report = Report::default();
    apply(state, project, repo, i, &mut report);
    parents_from_github(state, repo, [i]);
    let n = i["number"].as_i64()?;
    let t = linked(state, repo).into_iter().find(|t| t.github.as_ref().is_some_and(|g| g.number == n))?;
    Some((t, report.created > 0))
}

/// Publish to GitHub: creates the GitHub issue for a Loft-only issue in a project with a repo, and
/// links them. A retry (or a second click) finds the one already published, through the published-issue
/// memory, and creates no other. `by` is "user" or the agent's id, for the thread.
pub async fn publish(state: &AppState, id: &str, by: &str) -> Result<Task, String> {
    let t = state.db.get_task(id).map_err(|e| e.to_string())?.ok_or("No Loft issue with that id.")?;
    if t.github.is_some() {
        return Ok(t);
    }
    if t.project.trim().is_empty() {
        return Err("Pick a project first: this Loft issue has no project, so there's no GitHub repo to publish it to.".into());
    }
    let repo = crate::repos::github_repo(state, &t.project).await.map_err(|e| format!("{e}, so there's no GitHub repo to publish it to."))?;
    if crate::github::token(state).is_none() {
        return Err("GitHub isn't connected: add a GitHub token in Settings or run `gh auth login`.".into());
    }
    if t.state() == "closed" {
        return Err("It's closed: reopen it before publishing it.".into());
    }
    let cursor = state.mirror.cursor(&repo);
    let _held = cursor.lock().await;
    let mut t = state.db.get_task(id).map_err(|e| e.to_string())?.ok_or("No Loft issue with that id.")?;
    if t.github.is_some() {
        return Ok(t);
    }
    let issue = match state.db.published_issue(&repo, "loft_issue", &t.id).ok().flatten() {
        Some(n) => crate::github::get(state, &format!("/repos/{repo}/issues/{n}")).await.map_err(|e| format!("Couldn't read #{n} on {repo}: {e}"))?,
        None => {
            let mut body = json!({ "title": t.title, "body": t.description, "labels": t.labels });
            if !t.owner.is_empty() {
                body["assignees"] = json!([t.owner]);
            }
            let v = crate::github::request(state, reqwest::Method::POST, &format!("/repos/{repo}/issues"), Some(body)).await.map_err(|e| format!("GitHub rejected it on {repo}: {e}"))?;
            let n = v["number"].as_i64().ok_or("GitHub created the issue but didn't return its number.")?;
            state.db.record_published_issue(&repo, "loft_issue", &t.id, n).map_err(|e| e.to_string())?;
            v
        }
    };
    let l = link(&repo, &issue);
    let n = l.number;
    t.github = Some(l);
    t.updated_at = now();
    state.db.save_task(&t).map_err(|e| e.to_string())?;
    crate::tasks::emit(state, &t);
    crate::tasks::log_status(state, &t, by, &format!("Published to GitHub as #{n}"));
    state.emit(crate::state::Event::Issues { project: t.project.clone() });
    Ok(t)
}

/// `POST /api/issues/{id}/keep-working`: the answer to "closed on GitHub while being worked on": the
/// work goes on and the GitHub issue is reopened, so both sides agree again.
pub async fn keep_working(State(state): State<AppState>, axum::extract::Path(id): axum::extract::Path<String>) -> Result<Json<Value>, ApiError> {
    let t = state.db.get_task(&id)?.ok_or_else(crate::api::not_found)?;
    let Some(repo) = t.github.as_ref().map(|g| g.repo.clone()) else {
        return Err(crate::api::bad("It isn't on GitHub, so there's nothing to reopen."));
    };
    // Held like a sync or an edit, so a poll can't apply GitHub's close in between; then checked again.
    let cursor = state.mirror.cursor(&repo);
    let _held = cursor.lock().await;
    let t = state.db.get_task(&id)?.ok_or_else(crate::api::not_found)?;
    if !crate::tasks::is_active(&t) {
        return Err(crate::api::bad("Nobody is working on it now, so there's nothing to keep working on."));
    }
    let Some(g) = t.github.clone().filter(|g| g.state == "closed" && t.state() == "open") else {
        return Err(crate::api::bad("It isn't closed on GitHub, so there's nothing to reopen."));
    };
    let path = format!("/repos/{}/issues/{}", g.repo, g.number);
    let v = crate::github::request(&state, reqwest::Method::PATCH, &path, Some(json!({ "state": "open" }))).await.map_err(|e| crate::api::bad(format!("Couldn't reopen #{} on GitHub: {e}", g.number)))?;
    keep_link(&state, &g.repo, &v, true);
    let t = state.db.get_task(&id)?.ok_or_else(crate::api::not_found)?;
    crate::tasks::log_status(&state, &t, "user", &format!("Kept working; #{} reopened on GitHub", g.number));
    Ok(Json(json!(t)))
}

/// `POST /api/issues/{id}/publish`
pub async fn publish_route(State(state): State<AppState>, axum::extract::Path(id): axum::extract::Path<String>) -> Result<Json<Value>, ApiError> {
    publish(&state, &id, "user").await.map(|t| Json(json!(t))).map_err(crate::api::bad)
}

/// Publishes a new Loft issue in the background when auto-publish is on.
pub fn auto_publish(state: &AppState, t: &Task) {
    if t.github.is_some() || state.settings.get().auto_publish_all != Some(true) {
        return;
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    let (state, id, by) = (state.clone(), t.id.clone(), t.created_by.clone());
    rt.spawn(async move {
        if let Err(e) = publish(&state, &id, &by).await {
            tracing::warn!("auto-publish {id}: {e}");
        }
    });
}

/// GitHub's comments on issue `i` into the Loft thread, each once (by its GitHub comment id), when
/// GitHub counts more than the thread has. Loft's own approved comments carry their id, so they aren't
/// brought back as new ones.
async fn comments_from_github(state: &AppState, repo: &str, i: &Value) {
    let Some(n) = i["number"].as_i64() else { return };
    let Some(t) = linked(state, repo).into_iter().find(|t| t.github.as_ref().is_some_and(|g| g.number == n)) else { return };
    let known = state.db.github_comment_ids(&t.id).unwrap_or_default();
    if i["comments"].as_i64().unwrap_or(0) <= known.len() as i64 {
        return;
    }
    let cs = match crate::github::get(state, &format!("/repos/{repo}/issues/{n}/comments?per_page=100")).await {
        Ok(v) => v,
        Err(e) => return tracing::warn!("mirror {repo}#{n} comments: {e}"),
    };
    for c in cs.as_array().into_iter().flatten() {
        let Some(id) = c["id"].as_i64() else { continue };
        if known.contains(&id) {
            continue;
        }
        let login = c["user"]["login"].as_str().unwrap_or("someone");
        let body: String = c["body"].as_str().unwrap_or("").chars().take(4000).collect();
        let meta = json!({ "author_name": login, "github": { "state": "from_github", "comment_id": id } });
        if let Ok(c) = state.db.add_task_comment(&t.id, FROM_GITHUB, "comment", &body, meta) {
            state.emit(crate::state::Event::TaskComment { comment: json!(c) });
        }
    }
}

/// The comment `cid` on Loft issue `id`, and the issue, when the issue is on GitHub.
fn comment_on(state: &AppState, id: &str, cid: i64) -> Result<(Task, crate::db::TaskComment, GithubLink), String> {
    let c = state.db.get_task_comment(cid).map_err(|e| e.to_string())?.filter(|c| c.task_id == id).ok_or("No such comment on this Loft issue.")?;
    let t = state.db.get_task(id).map_err(|e| e.to_string())?.ok_or("No Loft issue with that id.")?;
    let g = t.github.clone().ok_or("This Loft issue isn't on GitHub, so its comments stay in Loft.")?;
    if c.meta["github"]["state"].is_null() || c.meta["github"]["state"] == "from_github" {
        return Err("That entry isn't a Loft comment waiting for GitHub.".into());
    }
    Ok((t, c, g))
}

fn save_comment(state: &AppState, c: &mut crate::db::TaskComment, text: &str, github: Value) -> Result<(), String> {
    c.text = text.to_string();
    c.meta["github"] = github;
    state.db.update_task_comment(c.id, &c.text, &c.meta).map_err(|e| e.to_string())?;
    state.emit(crate::state::Event::TaskComment { comment: json!(c) });
    Ok(())
}

/// Approve: posts the comment (with `text` instead, when the user edited it) on the GitHub issue from
/// the user's account, an agent's with its name in bold. Approving one already posted posts nothing.
pub async fn approve(state: &AppState, id: &str, cid: i64, text: Option<&str>) -> Result<crate::db::TaskComment, String> {
    let (_, _, g) = comment_on(state, id, cid)?;
    let cursor = state.mirror.cursor(&g.repo);
    let _held = cursor.lock().await;
    let (_, mut c, _) = comment_on(state, id, cid)?;
    if c.meta["github"]["state"] == "posted" {
        return Ok(c);
    }
    let text = text.map(str::trim).filter(|t| !t.is_empty()).map_or_else(|| c.text.clone(), str::to_string);
    let body = if c.author == "user" { text.clone() } else { format!("**{}**\n\n{text}", c.meta["author_name"].as_str().unwrap_or("An agent")) };
    let v = crate::github::request(state, reqwest::Method::POST, &format!("/repos/{}/issues/{}/comments", g.repo, g.number), Some(json!({ "body": body })))
        .await
        .map_err(|e| format!("GitHub turned the comment down: {e}"))?;
    let posted = if c.meta["github"]["auto"] == true { json!({ "state": "posted", "comment_id": v["id"], "auto": true }) } else { json!({ "state": "posted", "comment_id": v["id"] }) };
    save_comment(state, &mut c, &text, posted)?;
    Ok(c)
}

/// Posts an auto-approved comment in the background (the project's approval mode let it through).
pub fn spawn_approve(state: &AppState, id: &str, cid: i64) {
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    let (state, id) = (state.clone(), id.to_string());
    rt.spawn(async move {
        if let Err(e) = approve(&state, &id, cid, None).await {
            tracing::warn!("auto-approve comment {cid}: {e}");
        }
    });
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ApproveBody {
    text: Option<String>,
}

/// `POST /api/issues/{id}/comments/{cid}/approve`, optionally `{text}`: the edited text to post.
pub async fn approve_route(State(state): State<AppState>, axum::extract::Path((id, cid)): axum::extract::Path<(String, i64)>, body: Option<Json<ApproveBody>>) -> Result<Json<Value>, ApiError> {
    let text = body.and_then(|b| b.0.text);
    approve(&state, &id, cid, text.as_deref()).await.map(|c| Json(json!(c))).map_err(crate::api::bad)
}

/// `POST /api/issues/{id}/comments/{cid}/keep`: Keep in Loft. The comment stays in the thread and is never posted.
pub async fn keep_route(State(state): State<AppState>, axum::extract::Path((id, cid)): axum::extract::Path<(String, i64)>) -> Result<Json<Value>, ApiError> {
    let (_, mut c, _) = comment_on(&state, &id, cid).map_err(crate::api::bad)?;
    if c.meta["github"]["state"] == "posted" {
        return Err(crate::api::bad("It's already on GitHub."));
    }
    let text = c.text.clone();
    save_comment(&state, &mut c, &text, json!({ "state": "kept" })).map_err(crate::api::bad)?;
    Ok(Json(json!(c)))
}

/// A linked issue that left GitHub (deleted, or moved to another repo). A Loft issue the mirror made
/// and nobody touched goes with it; any other stays, unlinked, as a Loft-only issue with a note why.
async fn reconcile(state: &AppState, mut t: Task, what: &str, report: &mut Report) {
    let n = t.github.as_ref().map_or(0, |g| g.number);
    let thread = state.db.list_task_comments(&t.id, 100, None).unwrap_or_default();
    let untouched = t.created_by == FROM_GITHUB && t.status == "inbox" && t.agent_id.is_none() && thread.iter().all(|c| c.kind == "status");
    if untouched {
        if crate::tasks::delete(State(state.clone()), axum::extract::Path(t.id.clone())).await.is_ok() {
            report.reconciled += 1;
        }
        return;
    }
    t.github = None;
    t.updated_at = now();
    if state.db.save_task(&t).is_ok() {
        crate::tasks::emit(state, &t);
        crate::tasks::log_status(state, &t, FROM_GITHUB, &format!("GitHub #{n} {what}: this stays in Loft only"));
        report.reconciled += 1;
    }
}

/// What changed in the shared fields between `before` and `after`, as a GitHub issue PATCH body.
/// Loft-only fields (agent, status beyond open/closed, priority, due date, notes) are never in it.
fn patch(before: &Task, after: &Task) -> Value {
    let mut p = json!({});
    if before.title != after.title {
        p["title"] = json!(after.title);
    }
    if before.description != after.description {
        p["body"] = json!(after.description);
    }
    if before.labels != after.labels {
        p["labels"] = json!(after.labels);
    }
    if before.owner != after.owner {
        p["assignees"] = if after.owner.is_empty() { json!([]) } else { json!([after.owner]) };
    }
    if before.state() != after.state() {
        p["state"] = json!(after.state());
        if after.state() == "closed" {
            p["state_reason"] = json!(if after.status == "cancelled" { "not_planned" } else { "completed" });
        }
    }
    p
}

/// Writes a Loft edit of a mirrored issue to GitHub. If GitHub changed the issue since Loft last read
/// it, GitHub wins: its version is brought in instead. A failure is noted in the thread, never fatal.
pub async fn write_through(state: &AppState, before: &Task, after: &Task) {
    let Some(g) = after.github.clone() else { return };
    let mut body = patch(before, after);
    // Already closed on GitHub (cancelling after "closed while being worked on"): leave GitHub's reason as it is.
    if g.state == "closed" && body["state"] == "closed" {
        let o = body.as_object_mut().unwrap();
        o.remove("state");
        o.remove("state_reason");
    }
    let fields = body.as_object().is_some_and(|o| !o.is_empty());
    if !fields && !links_changed(before, after) {
        return;
    }
    let cursor = state.mirror.cursor(&g.repo);
    let _held = cursor.lock().await;
    // What Loft knows of GitHub now, under the lock: an earlier write (or a refresh) may have moved it on
    // since this edit was made.
    let known = state.db.get_task(&after.id).ok().flatten().and_then(|t| t.github).filter(|k| k.repo == g.repo && k.number == g.number).unwrap_or_else(|| g.clone());
    let path = format!("/repos/{}/issues/{}", g.repo, g.number);
    let result = async {
        let now_there = crate::github::get(state, &path).await?;
        // A close the poll hasn't applied yet (its link still says open) is a change too: GitHub's reason isn't written over.
        if now_there["updated_at"].as_str().is_some_and(|u| u != known.updated_at) || now_there["state"].as_str().is_some_and(|s| s != known.state) {
            apply(state, &after.project, &g.repo, &now_there, &mut Report::default());
            links_from_github(state, &g.repo, &now_there).await;
            if let Ok(Some(t)) = state.db.get_task(&after.id) {
                crate::tasks::log_status(state, &t, FROM_GITHUB, &format!("GitHub #{} changed meanwhile: its version wins", g.number));
            }
            return Ok(());
        }
        let patched = if fields { Some(crate::github::request(state, reqwest::Method::PATCH, &path, Some(body)).await?) } else { None };
        let parents = links_to_github(state, &g, &now_there, before, after).await?;
        // Remember what GitHub says now, so the next poll (or edit) doesn't take Loft's own write for a
        // change: the PATCH answer has it, unless a link written after it moved the issue on again.
        match patched.filter(|_| !links_changed(before, after)) {
            Some(v) => keep_link(state, &g.repo, &v, true),
            None => refresh_link(state, &g.repo, g.number).await,
        }
        for n in parents {
            refresh_link(state, &g.repo, n).await;
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!("mirror {path}: {e}");
        crate::tasks::log_status(state, after, FROM_GITHUB, &format!("Couldn't update GitHub #{}: {e}", g.number));
    }
}

/// `write_through` without waiting, for callers that can't await. Nothing to write, or no runtime: nothing spawned.
pub fn spawn_write_through(state: &AppState, before: Task, after: Task) {
    if after.github.is_none() || (patch(&before, &after).as_object().is_none_or(|o| o.is_empty()) && !links_changed(&before, &after)) {
        return;
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    let state = state.clone();
    rt.spawn(crate::github::as_source("mirror", async move { write_through(&state, &before, &after).await }));
}

fn links_changed(before: &Task, after: &Task) -> bool {
    before.depends_on != after.depends_on || before.parent_id != after.parent_id
}

/// The GitHub link of Loft issue `id`, when it mirrors an issue of `repo`.
fn link_of(state: &AppState, id: &str, repo: &str) -> Option<GithubLink> {
    state.db.get_task(id).ok().flatten()?.github.filter(|g| g.repo.eq_ignore_ascii_case(repo))
}

/// Loft's dependency and parent changes as GitHub blocking links and sub-issues. Links to Loft-only
/// issues stay in Loft. The issue numbers whose links changed, besides this one.
async fn links_to_github(state: &AppState, g: &GithubLink, now_there: &Value, before: &Task, after: &Task) -> anyhow::Result<Vec<i64>> {
    use reqwest::Method;
    let (repo, n) = (&g.repo, g.number);
    let me = now_there["id"].as_i64().unwrap_or(g.id);
    for d in after.depends_on.iter().filter(|d| !before.depends_on.contains(d)) {
        if let Some(b) = link_of(state, d, repo) {
            crate::github::request(state, Method::POST, &format!("/repos/{repo}/issues/{n}/dependencies/blocked_by"), Some(json!({ "issue_id": b.id }))).await?;
        }
    }
    for d in before.depends_on.iter().filter(|d| !after.depends_on.contains(d)) {
        if let Some(b) = link_of(state, d, repo) {
            crate::github::request(state, Method::DELETE, &format!("/repos/{repo}/issues/{n}/dependencies/blocked_by/{}", b.id), None).await?;
        }
    }
    let mut parents = vec![];
    if before.parent_id != after.parent_id {
        if let Some(p) = before.parent_id.as_deref().and_then(|p| link_of(state, p, repo)) {
            crate::github::request(state, Method::DELETE, &format!("/repos/{repo}/issues/{}/sub_issue", p.number), Some(json!({ "sub_issue_id": me }))).await?;
            parents.push(p.number);
        }
        if let Some(p) = after.parent_id.as_deref().and_then(|p| link_of(state, p, repo)) {
            crate::github::request(state, Method::POST, &format!("/repos/{repo}/issues/{}/sub_issues", p.number), Some(json!({ "sub_issue_id": me, "replace_parent": true }))).await?;
            parents.push(p.number);
        }
    }
    Ok(parents)
}

/// Reads issue #n again and keeps GitHub's `updated_at` (and state) on the Loft issue that mirrors it.
async fn refresh_link(state: &AppState, repo: &str, n: i64) {
    if let Ok(v) = crate::github::get(state, &format!("/repos/{repo}/issues/{n}")).await {
        keep_link(state, repo, &v, false);
    }
}

/// Keeps what GitHub says of issue `v` (`updated_at`, state) on the Loft issue that mirrors it. `v` is
/// GitHub's answer to Loft's own write (`own`), or a read. A state read that the Loft issue doesn't have
/// yet (a spec read closed here before the poll got to it) stays as it was, so the poll's `apply` still
/// sees GitHub move and moves the Loft issue with it. A state Loft wrote is kept as is, even when the
/// Loft issue moved on again meanwhile: its own write is no GitHub change.
fn keep_link(state: &AppState, repo: &str, v: &Value, own: bool) {
    let Some(n) = v["number"].as_i64() else { return };
    if let Some(mut t) = linked(state, repo).into_iter().find(|t| t.github.as_ref().is_some_and(|g| g.number == n)) {
        let mut l = link(repo, v);
        if !own && l.state != t.state() {
            l.state = t.github.as_ref().unwrap().state.clone();
        }
        t.github = Some(l);
        if state.db.save_task(&t).is_ok() {
            crate::tasks::emit(state, &t);
        }
    }
}

/// The parent GitHub gives each issue (`parent_issue_url`) onto the Loft issue that mirrors it, so a
/// spec's ticket knows its spec without waiting for the spec to be read again. GitHub leaves the field
/// out of an issue with no parent (it never sends null), so this only sets a parent: one taken off its
/// spec is dropped when the spec's sub-issues are read again.
fn parents_from_github<'a>(state: &AppState, repo: &str, issues: impl IntoIterator<Item = &'a Value>) {
    let mut all = linked(state, repo);
    for i in issues {
        let Some(n) = i["number"].as_i64() else { continue };
        let Some(p) = parent_number(repo, i) else { continue };
        let Some(k) = all.iter().position(|t| t.github.as_ref().is_some_and(|g| g.number == n)) else { continue };
        let Some(parent) = all.iter().find(|x| x.github.as_ref().is_some_and(|g| g.number == p)).map(|x| x.id.clone()) else { continue };
        if parent == all[k].id || all[k].parent_id.as_deref() == Some(parent.as_str()) {
            continue;
        }
        let t = &mut all[k];
        t.parent_id = Some(parent);
        t.updated_at = now();
        if state.db.save_task(t).is_ok() {
            crate::tasks::emit(state, t);
            crate::tasks::log_status(state, t, FROM_GITHUB, &format!("Made a sub-issue of #{p} on GitHub"));
        }
    }
}

/// The number of issue `i`'s parent in the same repo, from `parent_issue_url`.
fn parent_number(repo: &str, i: &Value) -> Option<i64> {
    let prefix = format!("https://api.github.com/repos/{}/issues/", repo.to_lowercase());
    i["parent_issue_url"].as_str().and_then(|u| u.to_lowercase().strip_prefix(&prefix).and_then(|x| x.parse::<i64>().ok()))
}

/// GitHub's blocking links and sub-issues of issue `i` onto the Loft issues that mirror them. Only
/// read when GitHub says the issue has some, or Loft has some to drop. Links to issues Loft doesn't
/// mirror, and Loft's links to Loft-only issues, are left alone.
async fn links_from_github(state: &AppState, repo: &str, i: &Value) {
    let Some(n) = i["number"].as_i64() else { return };
    let all = linked(state, repo);
    let Some(mut t) = all.iter().find(|t| t.github.as_ref().is_some_and(|g| g.number == n)).cloned() else { return };
    let number_of = |id: &str| all.iter().find(|x| x.id == id).and_then(|x| x.github.as_ref().map(|g| g.number));
    let by_number = |n: i64| all.iter().find(|x| x.github.as_ref().is_some_and(|g| g.number == n)).map(|x| x.id.clone());
    let numbers = |v: Value| v.as_array().into_iter().flatten().filter_map(|x| x["number"].as_i64()).collect::<Vec<_>>();

    let blocked = i["issue_dependencies_summary"]["total_blocked_by"].as_i64().or(i["issue_dependencies_summary"]["blocked_by"].as_i64()).unwrap_or(0);
    if blocked > 0 || t.depends_on.iter().any(|d| number_of(d).is_some()) {
        match crate::github::get(state, &format!("/repos/{repo}/issues/{n}/dependencies/blocked_by?per_page=100")).await {
            Ok(v) => {
                let mut deps: Vec<String> = t.depends_on.iter().filter(|d| number_of(d).is_none()).cloned().collect();
                deps.extend(numbers(v).into_iter().filter_map(by_number).filter(|d| *d != t.id));
                if deps != t.depends_on {
                    t.depends_on = deps;
                    t.updated_at = now();
                    if state.db.save_task(&t).is_ok() {
                        crate::tasks::emit(state, &t);
                        crate::tasks::log_status(state, &t, FROM_GITHUB, "Dependencies changed on GitHub");
                    }
                }
            }
            Err(e) => tracing::warn!("mirror {repo}#{n} blocked_by: {e}"),
        }
    }
    let subs = i["sub_issues_summary"]["total"].as_i64().unwrap_or(0);
    let children: Vec<&Task> = all.iter().filter(|x| x.parent_id.as_deref() == Some(&t.id)).collect();
    if subs > 0 || !children.is_empty() {
        match crate::github::get(state, &format!("/repos/{repo}/issues/{n}/sub_issues?per_page=100")).await {
            Ok(v) => {
                let now_children: Vec<String> = numbers(v).into_iter().filter_map(by_number).filter(|c| *c != t.id).collect();
                for x in all.iter().filter(|x| now_children.contains(&x.id) != (x.parent_id.as_deref() == Some(&t.id))) {
                    let mut x = x.clone();
                    let joining = now_children.contains(&x.id);
                    x.parent_id = joining.then(|| t.id.clone());
                    x.updated_at = now();
                    if state.db.save_task(&x).is_ok() {
                        crate::tasks::emit(state, &x);
                        let what = if joining { format!("Made a sub-issue of #{n} on GitHub") } else { format!("No longer a sub-issue of #{n} on GitHub") };
                        crate::tasks::log_status(state, &x, FROM_GITHUB, &what);
                    }
                }
            }
            Err(e) => tracing::warn!("mirror {repo}#{n} sub_issues: {e}"),
        }
    }
}

/// Percent-encodes a timestamp for a query string (`+` and `:` in RFC 3339).
fn url_encode(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-._~".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::github::tests::own_fake;
    use crate::github::tests::state_with_token;

    /// A test state with a GitHub token whose projects folder is its own temp dir, read at boot like Loft does.
    pub(crate) fn state() -> AppState {
        let st = state_with_token();
        let dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        let projects = dir.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(dir.join("settings.json"), json!({ "projects_dir": projects, "github_token": "test-token" }).to_string()).unwrap();
        AppState { settings: std::sync::Arc::new(crate::settings::Store::load(&dir).unwrap()), ..st }
    }

    /// An agent on `project`, whose folder is a git repo with this `origin` (none: not a repo at all).
    pub(crate) fn project(st: &AppState, project: &str, origin: Option<&str>) {
        let mut a = crate::tasks::tests::agent(st, &format!("{project}-dev"));
        a.project = project.into();
        st.db.save_agent(&a).unwrap();
        let Some(o) = origin else { return };
        let dir = st.settings.boot().projects_dir.join(project);
        assert!(dir.starts_with(std::env::temp_dir()), "tests never write into the real projects folder");
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap().status.success());
        git(&["init", "-q"]);
        if !o.is_empty() {
            git(&["remote", "add", "origin", o]);
        }
    }

    fn gh(repo: &str, n: i64, title: &str, state: &str, updated: &str) -> Value {
        json!({
            "number": n, "id": n * 100, "title": title, "state": state, "body": format!("Body of {title}"),
            "html_url": format!("https://github.com/{repo}/issues/{n}"), "updated_at": updated,
            "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [{ "login": "octo-dev" }, { "login": "someone" }],
            "user": { "login": "octo-author" },
        })
    }

    fn mirrored(st: &AppState, repo: &str) -> Vec<Value> {
        let mut v: Vec<Value> = linked(st, repo).iter().map(|t| json!(t)).collect();
        v.sort_by_key(|t| t["github"]["number"].as_i64());
        v
    }

    #[tokio::test]
    async fn an_open_github_issue_becomes_a_loft_issue_after_a_poll() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "mir-open", Some("git@github.com:octo/mir-open.git"));
        let pr = json!({ "number": 9, "title": "A PR", "state": "open", "pull_request": {}, "updated_at": "2026-10-02T00:00:00Z" });
        fake.route("GET", "/repos/octo/mir-open/issues", 200, json!([gh("octo/mir-open", 1, "Retry cap", "open", "2026-10-02T00:00:00Z"), pr]));

        let r = sync_project(&st, "mir-open", false).await.unwrap();
        assert_eq!((r.created, r.updated, r.error), (1, 0, None), "the first poll is a full read; the PR isn't an issue");
        let Json(list) = crate::tasks::list(State(st.clone()), axum::extract::Query(serde_json::from_value(json!({})).unwrap())).await.unwrap();
        let t = &list[0];
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!((&t["title"], &t["description"], &t["labels"], &t["owner"]), (&json!("Retry cap"), &json!("Body of Retry cap"), &json!(["bug"]), &json!("octo-dev")));
        assert_eq!((&t["project"], &t["status"], &t["state"], &t["created_by"]), (&json!("mir-open"), &json!("inbox"), &json!("open"), &json!("github")));
        assert_eq!(t["github"], json!({ "repo": "octo/mir-open", "number": 1, "id": 100, "url": "https://github.com/octo/mir-open/issues/1", "state": "open", "updated_at": "2026-10-02T00:00:00Z", "sub_total": 0, "sub_done": 0, "author": "octo-author" }));
        let thread = st.db.list_task_comments(t["id"].as_str().unwrap(), 10, None).unwrap();
        assert_eq!(thread[0].text, "Mirrored from GitHub #1");

        // Reading the same again changes nothing and never duplicates.
        let r = sync_project(&st, "mir-open", true).await.unwrap();
        assert_eq!((r.created, r.updated), (0, 0));
        assert_eq!(mirrored(&st, "octo/mir-open").len(), 1);
        assert!(fake.seen("/repos/octo/mir-open/").iter().all(|s| s.method == "GET"), "the mirror only reads");
    }

    #[tokio::test]
    async fn issues_mirrored_before_authors_were_kept_get_theirs_on_the_next_read() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "mir-author", Some("https://github.com/octo/mir-author"));
        let path = "/repos/octo/mir-author/issues";
        let open = gh("octo/mir-author", 1, "Retry cap", "open", "2026-10-02T00:00:00Z");
        let mut closed = gh("octo/mir-author", 2, "Old work", "closed", "2026-10-01T00:00:00Z");
        closed["user"]["login"] = json!("octo-me");
        let was_open = { let mut c = closed.clone(); c["state"] = json!("open"); c };
        fake.in_turn("GET", path, vec![
            // Mirrored while both were open.
            (200, json!([open.clone(), was_open])),
            // #2 closed on GitHub: the change poll.
            (200, json!([closed.clone()])),
            // After the upgrade: the full read lists the open ones; the one closed link lacking its author is read on its own.
            (200, json!([open.clone()])),
        ]);
        fake.route("GET", &format!("{path}/2"), 200, closed.clone());
        fake.route("GET", "/user", 200, json!({ "login": "octo-me" }));
        sync_project(&st, "mir-author", true).await.unwrap();
        sync_project(&st, "mir-author", false).await.unwrap();
        let author = |n: i64| linked(&st, "octo/mir-author").into_iter().find_map(|t| t.github.filter(|g| g.number == n)).unwrap().author;
        assert_eq!((author(1), author(2)), ("octo-author".into(), "octo-me".into()));
        // As saved before authors were kept: links without one. Then Loft restarts, so its next read is a full one.
        for mut t in linked(&st, "octo/mir-author") {
            t.github.as_mut().unwrap().author = String::new();
            st.db.save_task(&t).unwrap();
        }
        assert_eq!(st.db.get_task(&linked(&st, "octo/mir-author")[0].id).unwrap().unwrap().github.unwrap().author, "");
        st.mirror.repos.lock().unwrap().clear();
        let before = fake.seen(path).len();

        let Json(v) = sync_now(State(st.clone()), None).await.unwrap();
        assert_eq!((author(1), author(2)), ("octo-author".into(), "octo-me".into()), "the open one from the listing, the closed one from the closed issues");
        assert_eq!((v["projects"][0]["created"].as_i64(), v["projects"][0]["updated"].as_i64()), (Some(0), Some(0)), "a new author alone isn't a change on GitHub");
        let reads: Vec<String> = fake.seen(path)[before..].iter().map(|s| s.path.clone()).collect();
        assert_eq!(reads.len(), 2, "{reads:?}");
        assert!(reads[1].ends_with("/issues/2"), "{reads:?}");
        assert_eq!(v["login"], "octo-me", "the list learns who Loft's token belongs to");

        // Once every link has its author, the closed issues aren't read again.
        st.mirror.repos.lock().unwrap().clear();
        sync_project(&st, "mir-author", true).await.unwrap();
        assert!(fake.seen(path)[before + 2..].iter().all(|s| !s.path.contains("state=closed") && !s.path.ends_with("/2")));
        let _ = sync_now(State(st.clone()), None).await.unwrap();
        assert_eq!(fake.seen("/user").len(), 1, "the login is read once");
    }

    #[tokio::test]
    async fn a_failed_author_backfill_doesnt_stop_the_full_read() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "mir-backfill", Some("https://github.com/octo/mir-backfill"));
        let path = "/repos/octo/mir-backfill/issues";
        let open = |n: i64, title: &str| gh("octo/mir-backfill", n, title, "open", "2026-10-02T00:00:00Z");
        let closed = |n: i64, title: &str| gh("octo/mir-backfill", n, title, "closed", "2026-10-03T00:00:00Z");
        fake.in_turn("GET", path, vec![
            (200, json!([open(1, "Retry cap"), open(2, "Old work"), open(3, "Quiet fix")])),
            // #2 closed: the change poll.
            (200, json!([closed(2, "Old work")])),
            // After the restart: #3 isn't listed open any more.
            (200, json!([open(1, "Retry cap")])),
        ]);
        fake.route("GET", &format!("{path}/3"), 200, closed(3, "Quiet fix"));
        // Reading #2 for its author fails, then works on the next full read.
        fake.in_turn("GET", &format!("{path}/2"), vec![(500, json!({ "message": "Server Error" })), (200, closed(2, "Old work"))]);
        sync_project(&st, "mir-backfill", false).await.unwrap();
        sync_project(&st, "mir-backfill", false).await.unwrap();
        for mut t in linked(&st, "octo/mir-backfill") {
            t.github.as_mut().unwrap().author = String::new();
            st.db.save_task(&t).unwrap();
        }
        st.mirror.repos.lock().unwrap().clear();
        let link = |n: i64| linked(&st, "octo/mir-backfill").into_iter().find_map(|t| t.github.filter(|g| g.number == n)).unwrap();

        let r = sync_project(&st, "mir-backfill", false).await.unwrap();
        assert_eq!(r.error, None, "a cosmetic backfill isn't the read's error");
        assert_eq!(link(3).state, "closed", "the open issue GitHub stopped listing was still asked about");
        assert_eq!(link(2).author, "");

        sync_project(&st, "mir-backfill", true).await.unwrap();
        assert_eq!(link(2).author, "octo-author", "the next full read tries again");
        assert_eq!(fake.seen(&format!("{path}/2")).len(), 2);
    }

    #[tokio::test]
    async fn closed_links_older_than_the_pages_read_get_their_authors_one_by_one_and_only_once() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "mir-old", Some("https://github.com/octo/mir-old"));
        let path = "/repos/octo/mir-old/issues";
        let issue = |n: i64, state: &str| gh("octo/mir-old", n, &format!("Work {n}"), state, if state == "open" { "2026-10-02T00:00:00Z" } else { "2026-10-03T00:00:00Z" });
        let links: Vec<i64> = (1..=12).collect();
        // Over 1000 newer closed issues, none of them linked: every page is full and has none of ours.
        let page = |p: i64| json!((0..100).map(|k| issue(5000 + p * 100 + k, "closed")).collect::<Vec<_>>());
        let mut replies = vec![
            (200, json!(links.iter().map(|&n| issue(n, "open")).collect::<Vec<_>>())),
            (200, json!(links.iter().map(|&n| issue(n, "closed")).collect::<Vec<_>>())),
            // After the restart.
            (200, json!([])),
        ];
        replies.extend((1..=MAX_PAGES as i64).map(|p| (200, page(p))));
        // Every read after that.
        replies.push((200, json!([])));
        fake.in_turn("GET", path, replies);
        for &n in &links[..11] {
            fake.route("GET", &format!("{path}/{n}"), 200, issue(n, "closed"));
        }
        // #12 was deleted on GitHub.
        sync_project(&st, "mir-old", false).await.unwrap();
        sync_project(&st, "mir-old", false).await.unwrap();
        for mut t in linked(&st, "octo/mir-old") {
            t.github.as_mut().unwrap().author = String::new();
            st.db.save_task(&t).unwrap();
        }
        st.mirror.repos.lock().unwrap().clear();
        let before = fake.seen(path).len();

        let r = sync_project(&st, "mir-old", false).await.unwrap();
        assert_eq!((r.error, r.reconciled), (None, 1));
        let left = linked(&st, "octo/mir-old");
        assert_eq!(left.len(), 11, "the deleted one went with its untouched Loft issue");
        assert!(left.iter().all(|t| t.github.as_ref().unwrap().author == "octo-author"));
        let reads: Vec<String> = fake.seen(path)[before..].iter().map(|s| s.path.clone()).collect();
        assert_eq!(reads.iter().filter(|p| p.contains("state=closed")).count(), MAX_PAGES, "{reads:?}");
        assert_eq!(reads.iter().filter(|p| !p.contains('?')).count(), 12, "then each one left on its own");

        // Nothing is left to look for after another restart.
        st.mirror.repos.lock().unwrap().clear();
        let before = fake.seen(path).len();
        sync_project(&st, "mir-old", false).await.unwrap();
        assert_eq!(fake.seen(path).len() - before, 1, "just the open issues");
    }

    #[tokio::test]
    async fn a_github_change_arrives_on_the_next_poll_and_quiet_polls_cost_a_304() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "mir-change", Some("https://github.com/octo/mir-change"));
        let path = "/repos/octo/mir-change/issues";
        let v1 = gh("octo/mir-change", 1, "Retry cap", "open", "2026-10-02T00:00:00Z");
        let v2 = json!({ "title": "Retry cap at 5", "labels": [{ "name": "bug" }, { "name": "sync" }], "assignees": [], "updated_at": "2026-10-03T00:00:00Z" });
        let mut v2full = v1.clone();
        for (k, v) in v2.as_object().unwrap() {
            v2full[k] = v.clone();
        }
        fake.in_turn("GET", path, vec![
            (200, json!([v1])),
            // The change poll: #1 edited, #2 closed and never seen (stays out).
            (200, json!([v2full.clone(), gh("octo/mir-change", 2, "Old", "closed", "2026-10-03T00:00:00Z")])),
            // Nothing new: the same answer, which GitHub tags with the same ETag.
            (200, json!([v2full.clone()])),
            (304, Value::Null),
        ]);
        fake.with_headers("GET", path, &[("etag", "\"e1\"")]);

        sync_project(&st, "mir-change", true).await.unwrap();
        let r = sync_project(&st, "mir-change", false).await.unwrap();
        assert_eq!((r.created, r.updated), (0, 1));
        let t = &mirrored(&st, "octo/mir-change")[0];
        assert_eq!((&t["title"], &t["labels"], &t["owner"], &t["github"]["updated_at"]), (&json!("Retry cap at 5"), &json!(["bug", "sync"]), &json!(""), &json!("2026-10-03T00:00:00Z")));
        assert_eq!(mirrored(&st, "octo/mir-change").len(), 1, "a closed issue without a Loft issue isn't imported");

        let r = sync_project(&st, "mir-change", false).await.unwrap();
        assert_eq!((r.created, r.updated), (0, 0));
        let r = sync_project(&st, "mir-change", false).await.unwrap();
        assert_eq!((r.created, r.updated, r.error), (0, 0, None), "a 304 is no change and no error");

        let seen = fake.seen(path);
        assert_eq!(seen.len(), 4);
        assert!(seen[0].path.contains("state=open"), "{}", seen[0].path);
        assert!(seen[1].path.contains("state=all") && seen[1].path.contains("since=2026-10-02T00%3A00%3A00Z"), "{}", seen[1].path);
        assert!(seen[2].path.contains("since=2026-10-03T00%3A00%3A00Z"), "{}", seen[2].path);
        assert_eq!((&seen[2].headers["if-none-match"], &seen[3].headers["if-none-match"]), (&json!(""), &json!("\"e1\"")), "the ETag goes back once the answer settles");
        assert_eq!(seen[3].path, seen[2].path);
    }

    #[tokio::test]
    async fn a_closed_linked_issue_follows_and_deleted_or_transferred_ones_are_reconciled() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "mir-gone", Some("https://github.com/octo/mir-gone"));
        let repo = "octo/mir-gone";
        let at = "2026-10-02T00:00:00Z";
        fake.in_turn("GET", "/repos/octo/mir-gone/issues", vec![
            (200, json!([gh(repo, 1, "Deleted, untouched", "open", at), gh(repo, 2, "Deleted, worked on", "open", at), gh(repo, 3, "Moved", "open", at), gh(repo, 4, "Closed", "open", at)])),
            (200, json!([])),
        ]);
        sync_project(&st, "mir-gone", true).await.unwrap();
        let ids: HashMap<i64, String> = mirrored(&st, repo).iter().map(|t| (t["github"]["number"].as_i64().unwrap(), t["id"].as_str().unwrap().to_string())).collect();
        // Someone worked on #2 and #3 in Loft.
        let a = crate::tasks::tests::agent(&st, "Builder");
        let _ = crate::tasks::update(State(st.clone()), axum::extract::Path(ids[&2].clone()), Json(json!({ "agent_id": a.id }))).await.unwrap();
        let _ = crate::tasks::update(State(st.clone()), axum::extract::Path(ids[&3].clone()), Json(json!({ "priority": "high", "status": "ready" }))).await.unwrap();

        fake.route("GET", "/repos/octo/mir-gone/issues/1", 404, json!({ "message": "Not Found" }));
        fake.route("GET", "/repos/octo/mir-gone/issues/2", 410, json!({ "message": "This issue was deleted" }));
        // A transferred issue answers from its new home.
        fake.route("GET", "/repos/octo/mir-gone/issues/3", 200, gh("octo/elsewhere", 7, "Moved", "open", "2026-10-03T00:00:00Z"));
        fake.route("GET", "/repos/octo/mir-gone/issues/4", 200, gh(repo, 4, "Closed", "closed", "2026-10-03T00:00:00Z"));
        let r = sync_project(&st, "mir-gone", true).await.unwrap();
        assert_eq!((r.reconciled, r.updated, r.error), (3, 1, None));

        assert!(st.db.get_task(&ids[&1]).unwrap().is_none(), "untouched and deleted on GitHub: gone");
        for (n, why) in [(2, "GitHub #2 was deleted: this stays in Loft only"), (3, "GitHub #3 moved to https://github.com/octo/elsewhere/issues/7: this stays in Loft only")] {
            let t = st.db.get_task(&ids[&n]).unwrap().unwrap();
            assert!(t.github.is_none(), "#{n} unlinked");
            assert!(st.db.list_task_comments(&t.id, 20, None).unwrap().iter().any(|c| c.text == why), "#{n}: {why}");
        }
        let t4 = st.db.get_task(&ids[&4]).unwrap().unwrap();
        assert_eq!(t4.github.as_ref().map(|g| g.state.as_str()), Some("closed"), "a closed issue that has a Loft issue stays mirrored");
        assert_eq!(t4.status, "done", "closed on GitHub as completed: done in Loft");
    }

    #[tokio::test]
    async fn issues_linked_to_another_repo_follow_it_on_a_full_read_and_nothing_else_comes_in() {
        let st = state();
        let fake = own_fake(&st);
        // The project's origin moved to octo/new; some Loft issues still link to octo/old, one to a repo that's gone.
        project(&st, "mir-moved", Some("https://github.com/octo/new"));
        fake.route("GET", "/repos/octo/new/issues", 200, json!([]));
        let at = "2026-10-02T00:00:00Z";
        let mut ids = HashMap::new();
        for (repo, n) in [("octo/gone", 5), ("octo/old", 1), ("octo/old", 2), ("octo/old", 3)] {
            let t = Task {
                id: format!("moved-{n}"),
                title: format!("#{n}"),
                kind: "todo".into(),
                status: "ready".into(),
                project: "mir-moved".into(),
                github: Some(link(repo, &gh(repo, n, &format!("#{n}"), "open", at))),
                ..Default::default()
            };
            st.db.save_task(&t).unwrap();
            ids.insert(n, t.id);
        }
        fake.route("GET", "/repos/octo/old/issues/1", 200, gh("octo/old", 1, "Shipped", "closed", "2026-10-03T00:00:00Z"));
        fake.in_turn("GET", "/repos/octo/old/issues/2", vec![(200, gh("octo/old", 2, "Renamed", "open", "2026-10-03T00:00:00Z")), (304, Value::Null)]);
        fake.with_headers("GET", "/repos/octo/old/issues/2", &[("etag", "\"e2\"")]);
        let mut dropped = gh("octo/old", 3, "Dropped", "closed", "2026-10-03T00:00:00Z");
        dropped["state_reason"] = json!("not_planned");
        fake.route("GET", "/repos/octo/old/issues/3", 200, dropped);
        // octo/gone answers 404 (unrouted): left as it is, and the rest is still read.

        let r = sync_project(&st, "mir-moved", true).await.unwrap();
        assert_eq!((r.created, r.updated, r.error), (0, 3, None));
        let task = |n: i64| st.db.get_task(&ids[&n]).unwrap().unwrap();
        assert_eq!((task(1).status.as_str(), task(1).github.unwrap().state.as_str()), ("done", "closed"), "closed as completed on its own repo: done");
        assert_eq!(task(3).status, "cancelled", "closed as not planned: cancelled");
        assert_eq!((task(2).title.as_str(), task(2).status.as_str()), ("Renamed", "ready"), "still open: its title follows");
        assert_eq!((task(5).status.as_str(), task(5).github.unwrap().state.as_str()), ("ready", "open"), "an unreadable repo's link stays as it is");
        assert!(fake.seen("/repos/octo/old/issues?").is_empty() && fake.seen("/repos/octo/old/issues").len() == 3, "only the linked issues, by number: nothing of octo/old is imported");

        // The next full read: only #2 is still open, and it answers 304 to its ETag. A change poll doesn't read other repos.
        sync_project(&st, "mir-moved", false).await.unwrap();
        let r = sync_project(&st, "mir-moved", true).await.unwrap();
        assert_eq!((r.updated, r.error), (0, None));
        let old = fake.seen("/repos/octo/old/");
        assert_eq!(old.len(), 4, "{old:?}");
        assert_eq!((old[3].path.as_str(), &old[3].headers["if-none-match"]), ("/repos/octo/old/issues/2", &json!("\"e2\"")));
        assert_eq!(fake.seen("/repos/octo/gone/").len(), 2, "tried again on each full read");
        assert_eq!(linked(&st, "octo/old").len(), 3);
    }

    #[tokio::test]
    async fn a_sync_reads_every_project_of_an_agent() {
        let st = state();
        let fake = own_fake(&st);
        for p in ["grp-api", "grp-b2b", "grp-b2c"] {
            project(&st, p, Some(&format!("https://github.com/octo/{p}")));
            fake.route("GET", &format!("/repos/octo/{p}/issues"), 200, json!([gh(&format!("octo/{p}"), 1, &format!("{p} issue"), "open", "2026-10-02T00:00:00Z")]));
        }
        // One agent in all three, instead of one agent each.
        for a in st.db.list_agents().unwrap() {
            st.db.delete_agent(&a.id).unwrap();
        }
        let mut web = crate::tasks::tests::agent(&st, "Web");
        (web.group, web.project, web.projects) = ("Grp".into(), "grp-b2c".into(), vec!["grp-b2c".into(), "grp-api".into(), "grp-b2b".into()]);
        st.db.save_agent(&web).unwrap();

        let Json(v) = sync_now(State(st.clone()), None).await.unwrap();
        let synced: Vec<&str> = v["projects"].as_array().unwrap().iter().map(|r| r["project"].as_str().unwrap()).collect();
        assert_eq!(synced, ["grp-api", "grp-b2b", "grp-b2c"]);
        for p in ["grp-api", "grp-b2b", "grp-b2c"] {
            assert_eq!(mirrored(&st, &format!("octo/{p}")).len(), 1, "{p}");
        }
    }

    #[tokio::test]
    async fn projects_without_a_repo_or_a_token_are_left_alone() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "mir-plain", None);
        project(&st, "mir-local", Some(""));
        let (t, _) = crate::tasks::create(&st, &json!({ "title": "Local work", "project": "mir-plain" }), "user").unwrap();
        let before = json!(t);

        let Json(v) = sync_now(State(st.clone()), None).await.unwrap();
        // Skipped, and each says why, so a project that stays out of the list can be explained.
        assert_eq!(v["projects"], json!([]));
        let why: Vec<(String, String)> = v["skipped"].as_array().unwrap().iter().map(|s| (s["project"].as_str().unwrap().into(), s["reason"].as_str().unwrap().into())).collect();
        assert_eq!(why.len(), 2);
        assert!(why.iter().any(|(p, r)| p == "mir-local" && r == "The project mir-local has no `origin` remote"), "{why:?}");
        assert!(why.iter().any(|(p, r)| p == "mir-plain" && r.ends_with("mir-plain isn't a git repo")), "{why:?}");
        assert!(fake.seen("/").is_empty(), "nothing asked of GitHub");
        assert_eq!(json!(st.db.get_task(&t.id).unwrap().unwrap()), before);
        assert_eq!(st.db.list_tasks().unwrap().len(), 1);
        assert!(sync_now(State(st.clone()), Some(Json(SyncBody { project: Some("../elsewhere".into()) }))).await.is_err());

        // A repo, but no token: nothing either.
        project(&st, "mir-tokenless", Some("https://github.com/octo/mir-tokenless"));
        let mut s = st.settings.get();
        s.github_token = String::new();
        st.settings.save(s).unwrap();
        assert!(sync_project(&st, "mir-tokenless", true).await.is_none());
        assert!(fake.seen("/").is_empty());
    }

    /// A project mirroring #1 of `repo`, last read at `at`; the Loft issue's id.
    async fn one_mirrored(st: &AppState, fake: &crate::github::tests::Fake, project_name: &str, repo: &str, at: &str) -> String {
        project(st, project_name, Some(&format!("https://github.com/{repo}")));
        fake.route("GET", &format!("/repos/{repo}/issues"), 200, json!([gh(repo, 1, "Retry cap", "open", at)]));
        sync_project(st, project_name, true).await.unwrap();
        mirrored(st, repo)[0]["id"].as_str().unwrap().to_string()
    }

    fn patches(fake: &crate::github::tests::Fake, repo: &str) -> Vec<Value> {
        let path = format!("/repos/{repo}/issues/1");
        fake.seen(&path).into_iter().filter(|r| r.method == "PATCH" && r.path == path).map(|r| r.body).collect()
    }

    #[tokio::test]
    async fn a_loft_edit_reaches_github_and_loft_only_fields_never_do() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/wt-edit";
        let id = one_mirrored(&st, &fake, "wt-edit", repo, "2026-10-02T00:00:00Z").await;
        // Read before writing (unchanged), then again after, as GitHub has it now.
        fake.in_turn("GET", "/repos/octo/wt-edit/issues/1", vec![
            (200, gh(repo, 1, "Retry cap", "open", "2026-10-02T00:00:00Z")),
            (200, gh(repo, 1, "Retry cap at 5", "open", "2026-10-04T00:00:00Z")),
        ]);
        fake.route("PATCH", "/repos/octo/wt-edit/issues/1", 200, gh(repo, 1, "Retry cap at 5", "open", "2026-10-04T00:00:00Z"));

        let edit = json!({ "title": "Retry cap at 5", "labels": ["bug", "sync"], "owner": "pedro", "priority": "high", "due_at": "2026-11-01T00:00:00Z", "description": "Body of Retry cap" });
        let Json(v) = crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(edit)).await.unwrap();
        assert_eq!(patches(&fake, repo), [json!({ "title": "Retry cap at 5", "labels": ["bug", "sync"], "assignees": ["pedro"] })], "only the shared fields that changed");
        assert_eq!(v["github"]["updated_at"], "2026-10-04T00:00:00Z", "Loft remembers GitHub's answer, so its own write isn't a change");

        // Loft-only fields alone send nothing.
        let a = crate::tasks::tests::agent(&st, "Builder");
        let _ = crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(json!({ "priority": "low", "agent_id": a.id, "due_at": null }))).await.unwrap();
        crate::tasks::mcp_call(&st, &a, "update_issue", &json!({ "id": id, "status": "in_progress", "note": "halfway" }), |k: &str| Err(k.to_string())).unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(patches(&fake, repo).len(), 1);
        // A Loft-only issue never talks to GitHub.
        let (own, _) = crate::tasks::create(&st, &json!({ "title": "Local", "project": "wt-edit" }), "user").unwrap();
        let _ = crate::tasks::update(State(st.clone()), axum::extract::Path(own.id), Json(json!({ "title": "Local 2", "status": "done" }))).await.unwrap();
        assert_eq!(fake.seen("/repos/").iter().filter(|r| r.method != "GET").count(), 1);
    }

    #[tokio::test]
    async fn done_and_cancelled_close_with_the_right_reason_and_reopening_reopens() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/wt-state";
        let at = "2026-10-02T00:00:00Z";
        let id = one_mirrored(&st, &fake, "wt-state", repo, at).await;
        fake.route("GET", "/repos/octo/wt-state/issues/1", 200, gh(repo, 1, "Retry cap", "open", at));
        fake.route("PATCH", "/repos/octo/wt-state/issues/1", 200, gh(repo, 1, "Retry cap", "open", at));
        let up = |p: Value| crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(p));

        let _ = up(json!({ "status": "done" })).await.unwrap();
        let _ = up(json!({ "status": "inbox" })).await.unwrap();
        let _ = crate::tasks::cancel(State(st.clone()), axum::extract::Path(id.clone()), None).await.unwrap();
        let _ = up(json!({ "status": "ready" })).await.unwrap();
        // An agent finishing it closes it too.
        let a = crate::tasks::tests::agent(&st, "Builder");
        let _ = up(json!({ "agent_id": a.id })).await.unwrap();
        crate::tasks::mcp_call(&st, &a, "update_issue", &json!({ "id": id, "status": "done", "summary": "Capped." }), |k: &str| Err(k.to_string())).unwrap().unwrap();
        for _ in 0..50 {
            if patches(&fake, repo).len() == 5 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(patches(&fake, repo), [
            json!({ "state": "closed", "state_reason": "completed" }),
            json!({ "state": "open" }),
            json!({ "state": "closed", "state_reason": "not_planned" }),
            json!({ "state": "open" }),
            json!({ "state": "closed", "state_reason": "completed" }),
        ]);
    }

    #[tokio::test]
    async fn a_github_close_or_reopen_updates_the_loft_issue() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/wt-close";
        project(&st, "wt-close", Some("https://github.com/octo/wt-close"));
        let mut not_planned = gh(repo, 2, "Two", "closed", "2026-10-03T00:00:00Z");
        not_planned["state_reason"] = json!("not_planned");
        fake.in_turn("GET", "/repos/octo/wt-close/issues", vec![
            (200, json!([gh(repo, 1, "One", "open", "2026-10-02T00:00:00Z"), gh(repo, 2, "Two", "open", "2026-10-02T00:00:00Z")])),
            (200, json!([gh(repo, 1, "One", "closed", "2026-10-03T00:00:00Z"), not_planned])),
            (200, json!([gh(repo, 1, "One", "open", "2026-10-04T00:00:00Z")])),
        ]);
        sync_project(&st, "wt-close", true).await.unwrap();
        sync_project(&st, "wt-close", false).await.unwrap();
        let by = |n: i64| linked(&st, repo).into_iter().find(|t| t.github.as_ref().unwrap().number == n).unwrap();
        assert_eq!((by(1).status.as_str(), by(2).status.as_str()), ("done", "cancelled"));
        assert!(by(1).completed_at.is_some());
        let said = |t: &Task| st.db.list_task_comments(&t.id, 20, None).unwrap().into_iter().map(|c| c.text).collect::<Vec<_>>();
        assert!(said(&by(1)).contains(&"Closed on GitHub".to_string()) && said(&by(2)).contains(&"Closed on GitHub as not planned".to_string()));

        sync_project(&st, "wt-close", false).await.unwrap();
        assert_eq!((by(1).status.as_str(), by(1).state()), ("inbox", "open"));
        assert!(said(&by(1)).contains(&"Reopened on GitHub".to_string()));
        assert!(fake.seen("/repos/").iter().all(|r| r.method == "GET"), "following GitHub writes nothing back");
    }

    #[tokio::test]
    async fn when_both_sides_changed_github_wins() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/wt-conflict";
        let id = one_mirrored(&st, &fake, "wt-conflict", repo, "2026-10-02T00:00:00Z").await;
        // Someone retitled it on GitHub after Loft's last read.
        fake.route("GET", "/repos/octo/wt-conflict/issues/1", 200, gh(repo, 1, "Their title", "open", "2026-10-05T00:00:00Z"));

        let Json(v) = crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(json!({ "title": "My title" }))).await.unwrap();
        assert!(patches(&fake, repo).is_empty(), "nothing written over GitHub's change");
        assert_eq!((&v["title"], &v["github"]["updated_at"]), (&json!("Their title"), &json!("2026-10-05T00:00:00Z")));
        let thread = st.db.list_task_comments(&id, 20, None).unwrap();
        assert!(thread.iter().any(|c| c.text == "GitHub #1 changed meanwhile: its version wins"));

        // A GitHub error is noted in the thread and never fails the edit.
        fake.route("GET", "/repos/octo/wt-conflict/issues/1", 200, gh(repo, 1, "Their title", "open", "2026-10-05T00:00:00Z"));
        fake.route("PATCH", "/repos/octo/wt-conflict/issues/1", 422, json!({ "message": "Validation Failed", "errors": [{ "message": "could not add assignee" }] }));
        let r = crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(json!({ "owner": "nobody-here" }))).await;
        assert!(r.is_ok());
        assert!(st.db.list_task_comments(&id, 20, None).unwrap().iter().any(|c| c.text.starts_with("Couldn't update GitHub #1: GitHub 422")));
    }


    /// Mirrors #1, #2 and #3 of `repo` (database ids 100, 200, 300); their Loft ids by number.
    async fn three_mirrored(st: &AppState, fake: &crate::github::tests::Fake, name: &str, repo: &str, first: Value) -> HashMap<i64, String> {
        project(st, name, Some(&format!("https://github.com/{repo}")));
        let at = "2026-10-02T00:00:00Z";
        fake.route("GET", &format!("/repos/{repo}/issues"), 200, json!([first, gh(repo, 2, "Two", "open", at), gh(repo, 3, "Three", "open", at)]));
        sync_project(st, name, true).await.unwrap();
        mirrored(st, repo).iter().map(|t| (t["github"]["number"].as_i64().unwrap(), t["id"].as_str().unwrap().to_string())).collect()
    }

    fn writes(fake: &crate::github::tests::Fake, repo: &str) -> Vec<(String, String, Value)> {
        fake.seen(&format!("/repos/{repo}/")).into_iter().filter(|r| r.method != "GET").map(|r| (r.method, r.path, r.body)).collect()
    }

    #[tokio::test]
    async fn a_dependency_or_subtask_added_in_loft_becomes_a_github_link() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/ln-out";
        let at = "2026-10-02T00:00:00Z";
        let ids = three_mirrored(&st, &fake, "ln-out", repo, gh(repo, 1, "One", "open", at)).await;
        for n in [1, 3] {
            fake.route("GET", &format!("/repos/{repo}/issues/{n}"), 200, gh(repo, n, ["", "One", "", "Three"][n as usize], "open", at));
        }
        for (m, path) in [("POST", "/repos/octo/ln-out/issues/1/dependencies/blocked_by"), ("DELETE", "/repos/octo/ln-out/issues/1/dependencies/blocked_by/200"), ("POST", "/repos/octo/ln-out/issues/1/sub_issues"), ("DELETE", "/repos/octo/ln-out/issues/1/sub_issue")] {
            fake.route(m, path, 200, json!({}));
        }
        let up = |id: &String, p: Value| crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(p));
        let (local, _) = crate::tasks::create(&st, &json!({ "title": "Loft only", "project": "ln-out" }), "user").unwrap();

        let _ = up(&ids[&1], json!({ "depends_on": [ids[&2], local.id] })).await.unwrap();
        let _ = up(&ids[&3], json!({ "parent_id": ids[&1] })).await.unwrap();
        let _ = up(&ids[&1], json!({ "depends_on": [local.id] })).await.unwrap();
        let _ = up(&ids[&3], json!({ "parent_id": null })).await.unwrap();
        assert_eq!(writes(&fake, repo), [
            ("POST".into(), "/repos/octo/ln-out/issues/1/dependencies/blocked_by".into(), json!({ "issue_id": 200 })),
            ("POST".into(), "/repos/octo/ln-out/issues/1/sub_issues".into(), json!({ "sub_issue_id": 300, "replace_parent": true })),
            ("DELETE".into(), "/repos/octo/ln-out/issues/1/dependencies/blocked_by/200".into(), Value::Null),
            ("DELETE".into(), "/repos/octo/ln-out/issues/1/sub_issue".into(), json!({ "sub_issue_id": 300 })),
        ], "the link to the Loft-only issue stays in Loft");
    }

    #[tokio::test]
    async fn a_link_made_on_github_appears_in_loft_after_a_poll() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/ln-in";
        let mut one = gh(repo, 1, "One", "open", "2026-10-02T00:00:00Z");
        one["issue_dependencies_summary"] = json!({ "blocked_by": 1, "total_blocked_by": 1 });
        one["sub_issues_summary"] = json!({ "total": 1, "completed": 0 });
        fake.in_turn("GET", "/repos/octo/ln-in/issues/1/dependencies/blocked_by", vec![(200, json!([{ "number": 2, "id": 200 }, { "number": 99, "id": 9900 }])), (200, json!([]))]);
        fake.in_turn("GET", "/repos/octo/ln-in/issues/1/sub_issues", vec![(200, json!([{ "number": 3, "id": 300 }])), (200, json!([]))]);
        let ids = three_mirrored(&st, &fake, "ln-in", repo, one).await;
        let get = |n: i64| st.db.get_task(&ids[&n]).unwrap().unwrap();
        assert_eq!(get(1).depends_on, [ids[&2].clone()], "#99 isn't mirrored, so it's left out");
        assert_eq!(get(3).parent_id.as_deref(), Some(ids[&1].as_str()));

        // Both links removed on GitHub: the next poll that sees #1 change drops them.
        fake.route("GET", "/repos/octo/ln-in/issues", 200, json!([gh(repo, 1, "One", "open", "2026-10-03T00:00:00Z")]));
        sync_project(&st, "ln-in", false).await.unwrap();
        assert!(get(1).depends_on.is_empty());
        assert_eq!(get(3).parent_id, None);
        assert!(writes(&fake, repo).is_empty(), "following GitHub writes nothing back");
    }

    #[tokio::test]
    async fn a_ticket_takes_its_parent_from_its_own_github_issue() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/ln-parent";
        let at = "2026-10-02T00:00:00Z";
        // Mirrored before it was made a sub-issue, and GitHub's spec (#1) has no sub-issue count to read.
        let ids = three_mirrored(&st, &fake, "ln-parent", repo, gh(repo, 1, "One", "open", at)).await;
        let get = |n: i64| st.db.get_task(&ids[&n]).unwrap().unwrap();
        assert_eq!(get(3).parent_id, None);

        // The next full read: #3 unchanged, but its own JSON names #1 as its parent. #2's parent isn't mirrored.
        let mut three = gh(repo, 3, "Three", "open", at);
        three["parent_issue_url"] = json!(format!("https://api.github.com/repos/{repo}/issues/1"));
        let mut two = gh(repo, 2, "Two", "open", at);
        two["parent_issue_url"] = json!(format!("https://api.github.com/repos/{repo}/issues/99"));
        fake.route("GET", &format!("/repos/{repo}/issues"), 200, json!([gh(repo, 1, "One", "open", at), two, three]));
        sync_project(&st, "ln-parent", true).await.unwrap();
        assert_eq!(get(3).parent_id.as_deref(), Some(ids[&1].as_str()));
        assert_eq!(get(2).parent_id, None);
        assert!(fake.seen(&format!("/repos/{repo}/issues/1/sub_issues")).is_empty(), "no extra request");

        // GitHub leaves the field out of an issue with no parent: a list without it keeps the parent Loft has.
        let mut kept = get(2);
        kept.parent_id = Some(ids[&1].clone());
        st.db.save_task(&kept).unwrap();
        fake.route("GET", &format!("/repos/{repo}/issues"), 200, json!([gh(repo, 2, "Two", "open", "2026-10-04T00:00:00Z")]));
        sync_project(&st, "ln-parent", false).await.unwrap();
        assert_eq!(get(2).parent_id.as_deref(), Some(ids[&1].as_str()));
        assert!(writes(&fake, repo).is_empty(), "following GitHub writes nothing back");
    }

    #[tokio::test]
    async fn on_a_link_conflict_github_wins() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/ln-conflict";
        let ids = three_mirrored(&st, &fake, "ln-conflict", repo, gh(repo, 1, "One", "open", "2026-10-02T00:00:00Z")).await;
        // Meanwhile on GitHub, #1 got #3 as its blocker.
        let mut now_there = gh(repo, 1, "One", "open", "2026-10-05T00:00:00Z");
        now_there["issue_dependencies_summary"] = json!({ "blocked_by": 1, "total_blocked_by": 1 });
        fake.route("GET", "/repos/octo/ln-conflict/issues/1", 200, now_there);
        fake.route("GET", "/repos/octo/ln-conflict/issues/1/dependencies/blocked_by", 200, json!([{ "number": 3, "id": 300 }]));

        let _ = crate::tasks::update(State(st.clone()), axum::extract::Path(ids[&1].clone()), Json(json!({ "depends_on": [ids[&2]] }))).await.unwrap();
        assert!(writes(&fake, repo).is_empty(), "Loft's link isn't written over GitHub's change");
        assert_eq!(st.db.get_task(&ids[&1]).unwrap().unwrap().depends_on, [ids[&3].clone()]);
    }


    #[tokio::test]
    async fn publishing_a_loft_issue_creates_one_github_issue_and_links_it() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/pub-one";
        project(&st, "pub-one", Some("https://github.com/octo/pub-one"));
        fake.route("POST", "/repos/octo/pub-one/issues", 201, gh(repo, 7, "Export notes", "open", "2026-10-28T10:00:00Z"));
        fake.route("GET", "/repos/octo/pub-one/issues/7", 200, gh(repo, 7, "Export notes", "open", "2026-10-28T10:00:00Z"));
        let (t, _) = crate::tasks::create(&st, &json!({ "title": "Export notes", "description": "As Markdown.", "labels": ["feature"], "owner": "octo-dev", "priority": "high", "project": "pub-one" }), "user").unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(fake.seen("/repos/").is_empty(), "a new Loft issue stays Loft-only");

        let Json(v) = publish_route(State(st.clone()), axum::extract::Path(t.id.clone())).await.unwrap();
        assert_eq!((&v["github"]["number"], &v["github"]["url"]), (&json!(7), &json!("https://github.com/octo/pub-one/issues/7")));
        let posted: Vec<Value> = fake.seen("/repos/octo/pub-one/issues").into_iter().filter(|r| r.method == "POST").map(|r| r.body).collect();
        assert_eq!(posted, [json!({ "title": "Export notes", "body": "As Markdown.", "labels": ["feature"], "assignees": ["octo-dev"] })], "no Loft-only fields");
        assert!(st.db.list_task_comments(&t.id, 10, None).unwrap().iter().any(|c| c.text == "Published to GitHub as #7"));

        // Again, and again after the link was lost: no second issue.
        let _ = publish_route(State(st.clone()), axum::extract::Path(t.id.clone())).await.unwrap();
        let mut lost = st.db.get_task(&t.id).unwrap().unwrap();
        lost.github = None;
        st.db.save_task(&lost).unwrap();
        let Json(v) = publish_route(State(st.clone()), axum::extract::Path(t.id.clone())).await.unwrap();
        assert_eq!(v["github"]["number"], json!(7));
        assert_eq!(fake.seen("/repos/octo/pub-one/issues").iter().filter(|r| r.method == "POST").count(), 1);

        // An agent publishes its own; not someone else's.
        let a = crate::tasks::tests::agent(&st, "Builder");
        let (mine, _) = crate::tasks::create(&st, &json!({ "title": "Mine", "project": "pub-one" }), &a.id).unwrap();
        let (theirs, _) = crate::tasks::create(&st, &json!({ "title": "Theirs", "project": "pub-one" }), "user").unwrap();
        fake.route("POST", "/repos/octo/pub-one/issues", 201, gh(repo, 8, "Mine", "open", "2026-10-28T10:00:00Z"));
        let out = crate::planning::mcp_call(&st, &a, "publish_issue", &json!({ "id": mine.id })).await.unwrap().unwrap();
        assert_eq!(out, "Published to GitHub as #8: https://github.com/octo/pub-one/issues/8");
        assert!(crate::planning::mcp_call(&st, &a, "publish_issue", &json!({ "id": theirs.id })).await.unwrap().is_err());
    }

    #[tokio::test]
    async fn auto_publish_publishes_new_loft_issues_and_projects_without_a_repo_cant_publish() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/pub-auto";
        project(&st, "pub-auto", Some("https://github.com/octo/pub-auto"));
        project(&st, "pub-plain", None);
        let mut s = st.settings.get();
        s.auto_publish_all = Some(true);
        st.settings.save(s).unwrap();
        fake.route("POST", "/repos/octo/pub-auto/issues", 201, gh(repo, 3, "Auto", "open", "2026-10-28T10:00:00Z"));

        let (t, _) = crate::tasks::create(&st, &json!({ "title": "Auto", "project": "pub-auto" }), "user").unwrap();
        for _ in 0..50 {
            if st.db.get_task(&t.id).unwrap().unwrap().github.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(st.db.get_task(&t.id).unwrap().unwrap().github.map(|g| g.number), Some(3));

        // No repo: nothing is published, by hand or automatically, and GitHub isn't asked.
        let (plain, _) = crate::tasks::create(&st, &json!({ "title": "Plain", "project": "pub-plain" }), "user").unwrap();
        let err = publish_route(State(st.clone()), axum::extract::Path(plain.id.clone())).await.unwrap_err();
        assert!(err.1.contains("isn't a git repo, so there's no GitHub repo to publish it to"), "{}", err.1);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(st.db.get_task(&plain.id).unwrap().unwrap().github.is_none());
        assert_eq!(fake.seen("/repos/").len(), 1, "only the one auto-published issue");
    }


    #[tokio::test]
    async fn comments_wait_for_approval_and_github_comments_arrive_once() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/cm-flow";
        let id = one_mirrored(&st, &fake, "cm-flow", repo, "2026-10-02T00:00:00Z").await;
        let a = crate::tasks::tests::agent(&st, "Builder");
        let _ = crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(json!({ "agent_id": a.id }))).await.unwrap();
        fake.in_turn("POST", "/repos/octo/cm-flow/issues/1/comments", vec![(201, json!({ "id": 901 })), (201, json!({ "id": 902 }))]);
        let comment_posts = || fake.seen("/repos/octo/cm-flow/issues/1/comments").into_iter().filter(|r| r.method == "POST").map(|r| r.body).collect::<Vec<_>>();

        // Yours and an agent's: in the thread at once, not on GitHub yet, both in Needs you. The agent isn't blocked.
        let body: crate::tasks::CommentBody = serde_json::from_value(json!({ "text": "Ship it Friday", "send": true })).unwrap();
        let _ = crate::tasks::add_comment(State(st.clone()), axum::extract::Path(id.clone()), Json(body)).await.unwrap();
        let out = crate::tasks::mcp_call(&st, &a, "comment_issue", &json!({ "id": id, "text": "Done with the cap" }), |k: &str| Err(k.to_string())).unwrap();
        assert!(out.is_ok(), "{out:?}");
        let thread = st.db.list_task_comments(&id, 50, None).unwrap();
        let mine = thread.iter().find(|c| c.text == "Ship it Friday").unwrap().clone();
        let theirs = thread.iter().find(|c| c.text == "Done with the cap").unwrap().clone();
        assert_eq!((&mine.meta["github"], &theirs.meta["github"]), (&json!({ "state": "pending" }), &json!({ "state": "pending" })));
        assert!(thread.iter().filter(|c| c.kind == "status").all(|c| c.meta["github"].is_null()), "status entries never wait for GitHub");
        let items = crate::needs_you::collect(&st).unwrap();
        let waiting: Vec<_> = items.iter().filter(|i| i.kind == "comment_approval").collect();
        assert_eq!(waiting.len(), 2);
        assert!(comment_posts().is_empty(), "nothing on GitHub until approved");

        // Approve the agent's with edited text: posted once, its name in bold.
        let c = approve(&st, &id, theirs.id, Some("Done with the retry cap")).await.unwrap();
        assert_eq!((c.text.as_str(), &c.meta["github"]), ("Done with the retry cap", &json!({ "state": "posted", "comment_id": 901 })));
        approve(&st, &id, theirs.id, None).await.unwrap();
        // Keep yours in Loft: never posted.
        let Json(kept) = keep_route(State(st.clone()), axum::extract::Path((id.clone(), mine.id))).await.unwrap();
        assert_eq!(kept["meta"]["github"]["state"], "kept");
        assert_eq!(comment_posts(), [json!({ "body": "**Builder**\n\nDone with the retry cap" })]);
        assert!(crate::needs_you::collect(&st).unwrap().iter().all(|i| i.kind != "comment_approval"));
        // A status entry can't be approved.
        let status = thread.iter().find(|c| c.kind == "status").unwrap();
        assert!(approve(&st, &id, status.id, None).await.is_err());

        // GitHub has Pedro's comment and the one Loft posted: only Pedro's is added, and only once.
        let mut changed = gh(repo, 1, "Retry cap", "open", "2026-10-06T00:00:00Z");
        changed["comments"] = json!(2);
        fake.route("GET", "/repos/octo/cm-flow/issues", 200, json!([changed]));
        fake.route("GET", "/repos/octo/cm-flow/issues/1/comments", 200, json!([
            { "id": 901, "user": { "login": "octo-me" }, "body": "**Builder**\n\nDone with the retry cap" },
            { "id": 500, "user": { "login": "pedro" }, "body": "Looks good" },
        ]));
        sync_project(&st, "cm-flow", true).await.unwrap();
        sync_project(&st, "cm-flow", true).await.unwrap();
        let from_gh: Vec<_> = st.db.list_task_comments(&id, 50, None).unwrap().into_iter().filter(|c| c.author == FROM_GITHUB && c.kind == "comment").collect();
        assert_eq!(from_gh.len(), 1);
        assert_eq!((from_gh[0].text.as_str(), &from_gh[0].meta["author_name"]), ("Looks good", &json!("pedro")));
        assert_eq!(comment_posts().len(), 1, "a GitHub comment never goes back to GitHub");
    }


    /// Posts a comment by `who` ("user" or an agent) on `id`; its thread entry once it's there.
    async fn comment_by(st: &AppState, id: &str, who: &crate::db::Agent, user: bool, text: &str) -> crate::db::TaskComment {
        if user {
            let body: crate::tasks::CommentBody = serde_json::from_value(json!({ "text": text, "send": true })).unwrap();
            let _ = crate::tasks::add_comment(State(st.clone()), axum::extract::Path(id.to_string()), Json(body)).await.unwrap();
        } else {
            crate::tasks::mcp_call(st, who, "comment_issue", &json!({ "id": id, "text": text }), |k: &str| Err(k.to_string())).unwrap().unwrap();
        }
        st.db.list_task_comments(id, 100, None).unwrap().into_iter().find(|c| c.text == text).unwrap()
    }

    async fn settled(st: &AppState, cid: i64) -> Value {
        for _ in 0..50 {
            let c = st.db.get_task_comment(cid).unwrap().unwrap();
            if c.meta["github"]["state"] != "pending" || c.meta["github"]["auto"] != true {
                return c.meta["github"].clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        st.db.get_task_comment(cid).unwrap().unwrap().meta["github"].clone()
    }

    #[tokio::test]
    async fn the_global_approval_mode_decides_which_comments_go_straight_to_github() {
        let st = state();
        let fake = own_fake(&st);
        let a = crate::tasks::tests::agent(&st, "Builder");
        let mut ids = HashMap::new();
        for (p, repo) in [("ap-one", "octo/ap-one"), ("ap-two", "octo/ap-two")] {
            let id = one_mirrored(&st, &fake, p, repo, "2026-10-02T00:00:00Z").await;
            let _ = crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(json!({ "agent_id": a.id }))).await.unwrap();
            fake.route("POST", &format!("/repos/{repo}/issues/1/comments"), 201, json!({ "id": 700 }));
            ids.insert(p, id);
        }
        let posts = |repo: &str| fake.seen(&format!("/repos/{repo}/issues/1/comments")).into_iter().filter(|r| r.method == "POST").map(|r| r.body).collect::<Vec<_>>();
        let set_mode = |m: &str| {
            let mut s = st.settings.get();
            s.comment_approval_default = m.into();
            // Old per-project choices are ignored.
            s.comment_approval = [("ap-one".to_string(), "all".to_string())].into();
            st.settings.save(s).unwrap();
        };

        // Ask (the default): every comment in every project waits.
        assert_eq!(st.settings.get().approval_mode(), "ask");
        for p in ["ap-one", "ap-two"] {
            let u = comment_by(&st, &ids[p], &a, true, "ask: mine").await;
            let g = comment_by(&st, &ids[p], &a, false, "ask: agent's").await;
            assert_eq!((settled(&st, u.id).await["state"].clone(), settled(&st, g.id).await["state"].clone()), (json!("pending"), json!("pending")));
        }
        assert!(posts("octo/ap-one").is_empty() && posts("octo/ap-two").is_empty());

        // Own: yours goes, the agent's waits, in both projects.
        set_mode("own");
        for p in ["ap-one", "ap-two"] {
            let u = comment_by(&st, &ids[p], &a, true, "own: mine").await;
            let g = comment_by(&st, &ids[p], &a, false, "own: agent's").await;
            assert_eq!(settled(&st, u.id).await, json!({ "state": "posted", "comment_id": 700, "auto": true }));
            assert_eq!(settled(&st, g.id).await, json!({ "state": "pending" }));
        }
        assert_eq!(posts("octo/ap-two"), [json!({ "body": "own: mine" })]);

        // All: both go, labelled auto-approved, in both projects.
        set_mode("all");
        for p in ["ap-one", "ap-two"] {
            let u = comment_by(&st, &ids[p], &a, true, "all: mine").await;
            let g = comment_by(&st, &ids[p], &a, false, "all: agent's").await;
            assert_eq!((settled(&st, u.id).await["auto"].clone(), settled(&st, g.id).await["state"].clone()), (json!(true), json!("posted")));
        }
        assert_eq!(posts("octo/ap-two"), [json!({ "body": "own: mine" }), json!({ "body": "all: mine" }), json!({ "body": "**Builder**\n\nall: agent's" })]);

        // Anything unknown asks.
        set_mode("sometimes");
        assert_eq!(st.settings.get().approval_mode(), "ask");
    }


    #[tokio::test]
    async fn an_issue_takes_its_agents_project_and_one_without_a_project_says_so() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/pub-proj";
        project(&st, "pub-proj", Some("https://github.com/octo/pub-proj"));
        let mut dev = crate::tasks::tests::agent(&st, "Dev");
        dev.project = "pub-proj".into();
        st.db.save_agent(&dev).unwrap();
        let lead = crate::tasks::tests::agent(&st, "Lead");

        // Through the API and the Lead's create_issue: no project given, the assignee's.
        let (code, Json(v)) = crate::tasks::create_task(State(st.clone()), Json(json!({ "title": "Via API", "agent_id": dev.id }))).await.unwrap();
        assert_eq!((code.as_u16(), &v["project"]), (201, &json!("pub-proj")));
        crate::tasks::mcp_call(&st, &lead, "create_issue", &json!({ "title": "Via Lead", "agent": "Dev" }), |k: &str| if k == "Dev" { Ok(dev.clone()) } else { Err(k.to_string()) }).unwrap().unwrap();
        let by_lead = st.db.list_tasks().unwrap().into_iter().find(|t| t.title == "Via Lead").unwrap();
        assert_eq!(by_lead.project, "pub-proj");
        // An explicit project wins.
        let (_, Json(v)) = crate::tasks::create_task(State(st.clone()), Json(json!({ "title": "Mine", "agent_id": dev.id, "project": "other" }))).await.unwrap();
        assert_eq!(v["project"], "other");

        // No project: Publish is refused with the reason, and GitHub isn't asked.
        let (bare, _) = crate::tasks::create(&st, &json!({ "title": "Bare" }), "user").unwrap();
        let err = publish_route(State(st.clone()), axum::extract::Path(bare.id.clone())).await.unwrap_err();
        assert!(err.1.starts_with("Pick a project first"), "{}", err.1);
        assert!(fake.seen("/").is_empty());

        // Set the project through PATCH (both names): then it publishes.
        let Json(v) = crate::tasks::update(State(st.clone()), axum::extract::Path(bare.id.clone()), Json(json!({ "project": "pub-proj" }))).await.unwrap();
        assert_eq!(v["project"], "pub-proj");
        fake.route("POST", "/repos/octo/pub-proj/issues", 201, gh(repo, 5, "Bare", "open", "2026-10-29T00:00:00Z"));
        let Json(v) = publish_route(State(st.clone()), axum::extract::Path(bare.id)).await.unwrap();
        assert_eq!(v["github"]["number"], 5);
    }

    #[tokio::test]
    async fn a_spec_read_closed_by_a_refresh_before_the_poll_still_closes_in_loft() {
        for (name, reason, worked, status) in [("kl-done", "completed", false, "done"), ("kl-dropped", "not_planned", false, "cancelled"), ("kl-worked", "completed", true, "in_progress")] {
            let st = state();
            let fake = own_fake(&st);
            let repo = format!("octo/{name}");
            let id = one_mirrored(&st, &fake, name, &repo, "2026-10-02T00:00:00Z").await;
            if worked {
                let a = crate::tasks::tests::agent(&st, "Builder");
                crate::tasks::tests::busy(&st, &a);
                crate::tasks::execute(&st, &id, Some(&a.id), "", None).unwrap();
            }
            // Closed on GitHub: a refresh (its ticket changed) reads it first, then the full read no longer lists it.
            let mut closed = gh(&repo, 1, "Retry cap", "closed", "2026-10-05T00:00:00Z");
            closed["state_reason"] = json!(reason);
            fake.route("GET", &format!("/repos/{repo}/issues/1"), 200, closed);
            refresh_link(&st, &repo, 1).await;
            fake.route("GET", &format!("/repos/{repo}/issues"), 200, json!([]));
            let r = sync_project(&st, name, true).await.unwrap();
            assert_eq!((r.updated, r.error), (1, None), "{name}");
            let t = st.db.get_task(&id).unwrap().unwrap();
            assert_eq!((t.status.as_str(), t.github.as_ref().unwrap().state.as_str()), (status, "closed"), "{name}");
            let asked = st.db.list_task_comments(&id, 20, None).unwrap().iter().any(|c| c.text.starts_with("#1 was closed on GitHub while it's being worked on"));
            assert_eq!(asked, worked, "{name}: only the worked-on one is asked about");
        }
    }

    #[tokio::test]
    async fn an_answer_from_another_repo_without_a_link_any_more_is_dropped() {
        let st = state();
        let fake = own_fake(&st);
        project(&st, "mir-drop", Some("https://github.com/octo/new2"));
        fake.route("GET", "/repos/octo/new2/issues", 200, json!([]));
        let t = Task { id: "drop-1".into(), title: "#1".into(), kind: "todo".into(), status: "ready".into(), project: "mir-drop".into(), github: Some(link("octo/old2", &gh("octo/old2", 1, "#1", "open", "2026-10-02T00:00:00Z"))), ..Default::default() };
        st.db.save_task(&t).unwrap();
        // The answer is for an issue no Loft issue links to any more (as when its link goes while the read is in flight).
        fake.route("GET", "/repos/octo/old2/issues/1", 200, gh("octo/old2", 9, "Someone else's", "open", "2026-10-03T00:00:00Z"));
        let r = sync_project(&st, "mir-drop", true).await.unwrap();
        assert_eq!((r.created, r.updated, r.error), (0, 0, None));
        assert_eq!(linked(&st, "octo/old2").len(), 1, "nothing of octo/old2 is imported");
        assert_eq!(st.db.get_task("drop-1").unwrap().unwrap().github.unwrap().updated_at, "2026-10-02T00:00:00Z");
    }

    #[tokio::test]
    async fn done_in_loft_doesnt_write_over_a_github_close_the_poll_hasnt_applied() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/wt-reason";
        let id = one_mirrored(&st, &fake, "wt-reason", repo, "2026-10-02T00:00:00Z").await;
        // Closed as not planned on GitHub; a refresh read it, so the link has its updated_at but still says open.
        let mut closed = gh(repo, 1, "Retry cap", "closed", "2026-10-05T00:00:00Z");
        closed["state_reason"] = json!("not_planned");
        fake.route("GET", "/repos/octo/wt-reason/issues/1", 200, closed);
        refresh_link(&st, repo, 1).await;
        let Json(v) = crate::tasks::update(State(st.clone()), axum::extract::Path(id.clone()), Json(json!({ "status": "done" }))).await.unwrap();
        assert!(patches(&fake, repo).is_empty(), "GitHub's not planned isn't written over with completed");
        assert_eq!((&v["status"], &v["github"]["state"]), (&json!("done"), &json!("closed")));
        assert!(st.db.list_task_comments(&id, 20, None).unwrap().iter().any(|c| c.text == "GitHub #1 changed meanwhile: its version wins"));
    }

    #[tokio::test]
    async fn a_fast_close_then_reopen_in_loft_reopens_on_github() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/wt-flip";
        let id = one_mirrored(&st, &fake, "wt-flip", repo, "2026-10-02T00:00:00Z").await;
        let open = st.db.get_task(&id).unwrap().unwrap();
        let done = Task { status: "done".into(), ..open.clone() };
        // Done, then inbox again at once: both writes carry the link as it was at the edits. The close
        // is written while the Loft issue is already open again.
        fake.in_turn("GET", "/repos/octo/wt-flip/issues/1", vec![(200, gh(repo, 1, "Retry cap", "open", "2026-10-02T00:00:00Z")), (200, gh(repo, 1, "Retry cap", "closed", "2026-10-03T00:00:00Z"))]);
        fake.in_turn("PATCH", "/repos/octo/wt-flip/issues/1", vec![(200, gh(repo, 1, "Retry cap", "closed", "2026-10-03T00:00:00Z")), (200, gh(repo, 1, "Retry cap", "open", "2026-10-04T00:00:00Z"))]);
        write_through(&st, &open, &done).await;
        write_through(&st, &done, &open).await;
        assert_eq!(patches(&fake, repo).len(), 2);
        assert_eq!(patches(&fake, repo)[1], json!({ "state": "open" }), "Loft's reopen reaches GitHub");
        let t = st.db.get_task(&id).unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.github.unwrap().state.as_str()), ("inbox", "open"), "not undone as closed on GitHub");
    }

    #[tokio::test]
    async fn closed_on_github_while_being_worked_on_asks_the_user_instead() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/cw";
        let id = one_mirrored(&st, &fake, "cw", repo, "2026-10-02T00:00:00Z").await;
        let a = crate::tasks::tests::agent(&st, "Builder");
        crate::tasks::tests::busy(&st, &a);
        crate::tasks::execute(&st, &id, Some(&a.id), "", None).unwrap();

        // Someone closes it on GitHub mid-work.
        fake.route("GET", "/repos/octo/cw/issues", 200, json!([gh(repo, 1, "Retry cap", "closed", "2026-10-05T00:00:00Z")]));
        fake.route("GET", "/repos/octo/cw/issues/1", 200, gh(repo, 1, "Retry cap", "closed", "2026-10-05T00:00:00Z"));
        sync_project(&st, "cw", true).await.unwrap();
        let t = st.db.get_task(&id).unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.state(), t.github.as_ref().unwrap().state.as_str()), ("in_progress", "open", "closed"), "the work isn't stopped");
        assert!(st.db.list_task_comments(&id, 20, None).unwrap().iter().any(|c| c.text.starts_with("#1 was closed on GitHub while it's being worked on")));
        let items = crate::needs_you::collect(&st).unwrap();
        let ask: Vec<_> = items.iter().filter(|i| i.kind == "closed_on_github").collect();
        assert_eq!(ask.len(), 1);

        // Keep working: GitHub reopens it, and the question goes away.
        fake.route("PATCH", "/repos/octo/cw/issues/1", 200, gh(repo, 1, "Retry cap", "open", "2026-10-06T00:00:00Z"));
        let Json(v) = keep_working(State(st.clone()), axum::extract::Path(id.clone())).await.unwrap();
        assert_eq!((&v["status"], &v["github"]["state"]), (&json!("in_progress"), &json!("open")));
        let patched: Vec<Value> = fake.seen("/repos/octo/cw/issues/1").into_iter().filter(|r| r.method == "PATCH").map(|r| r.body).collect();
        assert_eq!(patched, [json!({ "state": "open" })]);
        assert!(crate::needs_you::collect(&st).unwrap().iter().all(|i| i.kind != "closed_on_github"));
        assert!(keep_working(State(st.clone()), axum::extract::Path(id.clone())).await.is_err(), "nothing to reopen now");

        // Not being worked on (here: cancelled): Keep working is refused and GitHub isn't asked.
        let before = fake.seen("/repos/octo/cw/issues/1").len();
        let mut t = st.db.get_task(&id).unwrap().unwrap();
        t.status = "ready".into();
        t.github.as_mut().unwrap().state = "closed".into();
        st.db.save_task(&t).unwrap();
        let err = keep_working(State(st.clone()), axum::extract::Path(id.clone())).await.unwrap_err();
        assert!(err.1.starts_with("Nobody is working on it now"), "{}", err.1);
        assert_eq!(fake.seen("/repos/octo/cw/issues/1").len(), before);
    }

    #[tokio::test]
    async fn cancelling_after_a_github_close_leaves_githubs_reason_alone() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/cw2";
        let id = one_mirrored(&st, &fake, "cw2", repo, "2026-10-02T00:00:00Z").await;
        let a = crate::tasks::tests::agent(&st, "Builder");
        crate::tasks::tests::busy(&st, &a);
        crate::tasks::execute(&st, &id, Some(&a.id), "", None).unwrap();
        fake.route("GET", "/repos/octo/cw2/issues", 200, json!([]));
        fake.route("GET", "/repos/octo/cw2/issues/1", 200, gh(repo, 1, "Retry cap", "closed", "2026-10-05T00:00:00Z"));
        sync_project(&st, "cw2", true).await.unwrap();
        let _ = crate::tasks::cancel(State(st.clone()), axum::extract::Path(id.clone()), None).await.unwrap();
        assert_eq!(st.db.get_task(&id).unwrap().unwrap().status, "cancelled");
        assert!(fake.seen("/repos/octo/cw2/issues/1").iter().all(|r| r.method == "GET"), "no PATCH: it was already closed");
    }


    #[tokio::test]
    async fn a_rate_limited_first_read_is_reported_and_the_next_poll_mirrors() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/rl-first";
        project(&st, "rl-first", Some("https://github.com/octo/rl-first"));
        // GitHub's core limit is used up when Loft starts (by agents' `gh`, other tools on the token...).
        fake.in_turn("GET", "/repos/octo/rl-first/issues", vec![
            (403, json!({ "message": "API rate limit exceeded for user ID 1." })),
            (200, json!([gh(repo, 1, "Retry cap", "open", "2026-10-02T00:00:00Z")])),
        ]);
        fake.with_headers("GET", "/repos/octo/rl-first/issues", &[("retry-after", "900")]);
        let Json(v) = sync_now(State(st.clone()), None).await.unwrap();
        assert!(v["projects"][0]["error"].as_str().unwrap().contains("rate limit"), "the list can say why it's empty: {v}");
        assert!(mirrored(&st, repo).is_empty());
        // The next poll is a full read again (nothing was read yet), and it mirrors.
        let r = sync_project(&st, "rl-first", false).await.unwrap();
        assert_eq!((r.created, r.error), (1, None));
        let first = fake.seen("/repos/octo/rl-first/issues?");
        assert!(first.iter().all(|r| r.path.contains("state=open") && r.headers["if-none-match"] == ""), "no stale since or ETag before a first read");
    }

    #[tokio::test]
    async fn a_full_read_of_unchanged_issues_reads_no_links_or_comments() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/cheap";
        project(&st, "cheap", Some("https://github.com/octo/cheap"));
        let mut one = gh(repo, 1, "Spec", "open", "2026-10-02T00:00:00Z");
        one["comments"] = json!(3);
        one["sub_issues_summary"] = json!({ "total": 2, "completed": 0 });
        one["issue_dependencies_summary"] = json!({ "blocked_by": 1, "total_blocked_by": 1 });
        fake.route("GET", "/repos/octo/cheap/issues", 200, json!([one]));
        for what in ["comments", "sub_issues", "dependencies/blocked_by"] {
            fake.route("GET", &format!("/repos/octo/cheap/issues/1/{what}"), 200, json!([]));
        }
        sync_project(&st, "cheap", true).await.unwrap();
        let after_first = fake.seen("/repos/octo/cheap/issues/1/").len();
        assert_eq!(after_first, 3, "a new issue's links and comments are read once");
        // Opening the list again (a full read each time) with nothing changed on GitHub: one request each.
        for _ in 0..5 {
            sync_project(&st, "cheap", true).await.unwrap();
        }
        assert_eq!(fake.seen("/repos/octo/cheap/issues/1/").len(), after_first);
        assert_eq!(fake.seen("/repos/octo/cheap/issues?").len(), 6);
    }


    #[tokio::test]
    async fn opening_the_list_asks_only_what_changed_and_a_shared_repo_is_read_once() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/rl-less";
        project(&st, "less-web", Some("https://github.com/octo/rl-less"));
        project(&st, "less-ios", Some("git@github.com:octo/rl-less.git"));
        fake.route("GET", "/repos/octo/rl-less/issues", 200, json!([gh(repo, 1, "Retry cap", "open", "2026-10-02T00:00:00Z")]));

        // First open: the repo was never read, so a full read; both projects report it, one request.
        let Json(v) = sync_now(State(st.clone()), None).await.unwrap();
        assert_eq!(v["projects"].as_array().unwrap().len(), 2);
        assert_eq!(v["projects"][0]["created"].as_i64().unwrap() + v["projects"][1]["created"].as_i64().unwrap(), 2, "{v}");
        let lists = || fake.seen("/repos/octo/rl-less/issues?");
        assert_eq!(lists().len(), 1);
        assert!(lists()[0].path.contains("state=open"));

        // Opening it again: the change poll, not a full read.
        let _ = sync_now(State(st.clone()), None).await.unwrap();
        assert_eq!(lists().len(), 2);
        assert!(lists()[1].path.contains("state=all") && lists()[1].path.contains("since="), "{}", lists()[1].path);
        assert_eq!(mirrored(&st, repo).len(), 1, "no duplicate for the second project");
    }

    #[tokio::test]
    async fn an_edit_keeps_the_patch_answer_instead_of_reading_the_issue_again() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/wt-reuse";
        let id = one_mirrored(&st, &fake, "wt-reuse", repo, "2026-10-02T00:00:00Z").await;
        fake.route("GET", "/repos/octo/wt-reuse/issues/1", 200, gh(repo, 1, "Retry cap", "open", "2026-10-02T00:00:00Z"));
        fake.route("PATCH", "/repos/octo/wt-reuse/issues/1", 200, gh(repo, 1, "Capped", "open", "2026-10-07T00:00:00Z"));
        let Json(v) = crate::tasks::update(State(st.clone()), axum::extract::Path(id), Json(json!({ "title": "Capped" }))).await.unwrap();
        assert_eq!(v["github"]["updated_at"], "2026-10-07T00:00:00Z");
        let gets = fake.seen("/repos/octo/wt-reuse/issues/1").into_iter().filter(|r| r.method == "GET").count();
        assert_eq!(gets, 1, "only the conflict check before writing");
    }

    #[tokio::test]
    async fn private_notes_and_handoff_notes_never_go_to_github_whatever_the_mode() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/priv";
        let id = one_mirrored(&st, &fake, "priv", repo, "2026-10-02T00:00:00Z").await;
        fake.route("POST", "/repos/octo/priv/issues/1/comments", 201, json!({ "id": 800 }));
        let mut s = st.settings.get();
        s.comment_approval_default = "all".into();
        st.settings.save(s).unwrap();
        let a = crate::tasks::tests::agent(&st, "Builder");
        crate::tasks::tests::busy(&st, &a);

        // Handed over with a note, then a private note: in Loft, never queued for GitHub, never posted.
        crate::tasks::execute(&st, &id, Some(&a.id), "The staging password is in 1Password", None).unwrap();
        let body: crate::tasks::CommentBody = serde_json::from_value(json!({ "text": "Ask Tom about the deadline", "send": false })).unwrap();
        let _ = crate::tasks::add_comment(State(st.clone()), axum::extract::Path(id.clone()), Json(body)).await.unwrap();
        // A comment meant for the issue still goes (mode: all).
        let body: crate::tasks::CommentBody = serde_json::from_value(json!({ "text": "Shipping Friday", "send": true })).unwrap();
        let _ = crate::tasks::add_comment(State(st.clone()), axum::extract::Path(id.clone()), Json(body)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let thread = st.db.list_task_comments(&id, 50, None).unwrap();
        for text in ["The staging password is in 1Password", "Ask Tom about the deadline"] {
            let c = thread.iter().find(|c| c.text == text).unwrap();
            assert!(c.meta["github"].is_null(), "{text}: {:?}", c.meta);
        }
        let posted: Vec<Value> = fake.seen("/repos/octo/priv/issues/1/comments").into_iter().filter(|r| r.method == "POST").map(|r| r.body).collect();
        assert_eq!(posted, [json!({ "body": "Shipping Friday" })]);
        assert!(crate::needs_you::collect(&st).unwrap().iter().all(|i| i.kind != "comment_approval"));
    }

    #[tokio::test]
    async fn post_on_github_says_where_a_comment_goes_agent_or_not() {
        let st = state();
        let fake = own_fake(&st);
        let id = one_mirrored(&st, &fake, "gh-flag", "octo/gh-flag", "2026-10-02T00:00:00Z").await;
        fake.route("POST", "/repos/octo/gh-flag/issues/1/comments", 201, json!({ "id": 801 }));
        let mut s = st.settings.get();
        s.comment_approval_default = "all".into();
        st.settings.save(s).unwrap();
        assert!(st.db.get_task(&id).unwrap().unwrap().agent_id.is_none());
        let post = |b: Value| {
            let st = st.clone();
            let id = id.clone();
            async move {
                let body: crate::tasks::CommentBody = serde_json::from_value(b).unwrap();
                let _ = crate::tasks::add_comment(State(st), axum::extract::Path(id), Json(body)).await.unwrap();
            }
        };

        // Nobody works on it: a sent comment still goes to GitHub, unless it says not to.
        post(json!({ "text": "Shipping Friday", "send": true })).await;
        post(json!({ "text": "Only for Loft", "send": true, "github": false })).await;
        post(json!({ "text": "A private note", "send": false })).await;
        post(json!({ "text": "Tell GitHub anyway", "send": false, "github": true })).await;
        tokio::time::sleep(Duration::from_millis(300)).await;

        let posted: Vec<Value> = fake.seen("/repos/octo/gh-flag/issues/1/comments").into_iter().filter(|r| r.method == "POST").map(|r| r.body).collect();
        assert_eq!(posted, [json!({ "body": "Shipping Friday" }), json!({ "body": "Tell GitHub anyway" })]);
        let thread = st.db.list_task_comments(&id, 50, None).unwrap();
        let private = |text: &str| thread.iter().find(|c| c.text == text).unwrap().meta["private"].clone();
        assert_eq!(private("Shipping Friday"), json!(false));
        assert_eq!(private("A private note"), json!(true));
    }


    #[tokio::test]
    async fn a_spec_keeps_githubs_sub_issue_count_and_is_never_run_as_one_task() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/spec";
        project(&st, "spec", Some("https://github.com/octo/spec"));
        let mut one = gh(repo, 42, "Checkout spec", "open", "2026-10-02T00:00:00Z");
        one["labels"] = json!([{ "name": "spec" }]);
        // #43 is closed and not mirrored; #44 and #45 are open. GitHub counts all three.
        one["sub_issues_summary"] = json!({ "total": 3, "completed": 1, "percent_completed": 33 });
        fake.route("GET", "/repos/octo/spec/issues", 200, json!([one, gh(repo, 44, "Store", "open", "2026-10-02T00:00:00Z"), gh(repo, 45, "UI", "open", "2026-10-02T00:00:00Z")]));
        fake.route("GET", "/repos/octo/spec/issues/42/sub_issues", 200, json!([{ "number": 44, "id": 4400 }, { "number": 45, "id": 4500 }]));
        sync_project(&st, "spec", true).await.unwrap();

        let spec = linked(&st, repo).into_iter().find(|t| t.github.as_ref().is_some_and(|g| g.number == 42)).unwrap();
        let g = spec.github.as_ref().unwrap();
        assert_eq!((g.sub_total, g.sub_done), (3, 1), "GitHub's own numbers, not the two mirrored children");
        assert_eq!(mirrored(&st, repo)[0]["github"]["sub_total"], 3);

        // The spec is never handed to an agent as one task; nothing starts.
        let a = crate::tasks::tests::agent(&st, "Builder");
        let err = crate::tasks::execute(&st, &spec.id, Some(&a.id), "", None).unwrap_err();
        assert!(err.1.starts_with("A spec is split into tickets"), "{}", err.1);
        assert_eq!(st.db.get_task(&spec.id).unwrap().unwrap().status, spec.status);
        // GitHub labels are case-insensitive: "Spec" is a spec too.
        let mut other = st.db.get_task(&spec.id).unwrap().unwrap();
        other.labels = vec!["SPEC".into()];
        st.db.save_task(&other).unwrap();
        assert!(crate::tasks::execute(&st, &spec.id, Some(&a.id), "", None).is_err());
    }

    #[tokio::test]
    async fn the_change_poll_reads_every_page_when_more_than_a_hundred_changed() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/mir-pages";
        project(&st, "mir-pages", Some("https://github.com/octo/mir-pages"));
        let path = "/repos/octo/mir-pages/issues";
        let at = |n: i64| format!("2026-10-02T00:{:02}:{:02}Z", n / 60, n % 60);
        let page1: Vec<Value> = (1..=100).map(|n| gh(repo, n, &format!("Issue {n}"), "open", &at(n))).collect();
        fake.in_turn("GET", path, vec![
            (200, json!([page1[0].clone()])),
            // 101 issues changed: the first page is full, the next one starts from its newest.
            (200, json!(page1)),
            (200, json!([page1[99].clone(), gh(repo, 101, "Issue 101", "open", &at(101))])),
        ]);
        sync_project(&st, "mir-pages", true).await.unwrap();
        let r = sync_project(&st, "mir-pages", false).await.unwrap();
        assert_eq!((r.created, r.error), (100, None), "none is left on a page never read");
        assert_eq!(mirrored(&st, repo).len(), 101);
        let seen = fake.seen(path);
        assert_eq!(seen.len(), 3);
        assert!(seen[2].path.contains(&format!("since={}", url_encode(&at(100)))) && !seen[2].path.contains("&page="), "{}", seen[2].path);
        assert_eq!(state_cursor(&st, repo), Some(at(101)));
    }

    #[tokio::test]
    async fn a_full_page_changed_in_one_second_reads_the_next_page_by_number() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/mir-burst";
        project(&st, "mir-burst", Some("https://github.com/octo/mir-burst"));
        let path = "/repos/octo/mir-burst/issues";
        let t = "2026-10-02T00:00:00Z";
        let page1: Vec<Value> = (1..=100).map(|n| gh(repo, n, &format!("Issue {n}"), "open", t)).collect();
        fake.in_turn("GET", path, vec![(200, json!([page1[0].clone()])), (200, json!(page1)), (200, json!([gh(repo, 101, "Issue 101", "open", t)]))]);
        sync_project(&st, "mir-burst", true).await.unwrap();
        sync_project(&st, "mir-burst", false).await.unwrap();
        assert_eq!(mirrored(&st, repo).len(), 101);
        assert!(fake.seen(path)[2].path.ends_with("&page=2"), "{}", fake.seen(path)[2].path);
    }

    fn state_cursor(st: &AppState, repo: &str) -> Option<String> {
        let c = st.mirror.cursor(repo);
        c.try_lock().unwrap().since.clone()
    }

    #[tokio::test]
    async fn a_ticket_closing_updates_its_specs_progress_on_the_next_poll() {
        let st = state();
        let fake = own_fake(&st);
        let repo = "octo/mir-progress";
        project(&st, "mir-progress", Some("https://github.com/octo/mir-progress"));
        let mut spec = gh(repo, 1, "A spec", "open", "2026-10-02T00:00:00Z");
        spec["labels"] = json!([{ "name": "spec" }]);
        spec["sub_issues_summary"] = json!({ "total": 2, "completed": 0 });
        let mut ticket = gh(repo, 2, "A ticket", "open", "2026-10-02T00:00:00Z");
        ticket["parent_issue_url"] = json!(format!("https://api.github.com/repos/{repo}/issues/1"));
        let mut closed = ticket.clone();
        closed["state"] = json!("closed");
        closed["updated_at"] = json!("2026-10-03T00:00:00Z");
        // Closing a sub-issue leaves the spec's own `updated_at` alone: only the ticket is listed as changed.
        fake.in_turn("GET", "/repos/octo/mir-progress/issues", vec![(200, json!([spec.clone(), ticket])), (200, json!([closed]))]);
        let mut now = spec.clone();
        now["sub_issues_summary"] = json!({ "total": 2, "completed": 1 });
        fake.route("GET", "/repos/octo/mir-progress/issues/1", 200, now);
        sync_project(&st, "mir-progress", true).await.unwrap();
        sync_project(&st, "mir-progress", false).await.unwrap();

        let g = mirrored(&st, repo)[0]["github"].clone();
        assert_eq!((g["sub_done"].as_i64(), g["sub_total"].as_i64()), (Some(1), Some(2)), "the spec's progress follows its ticket");
    }
}
