//! The mirror, GitHub → Loft: in a project with a GitHub repo, every open issue of the repo is a Loft
//! issue too, and so is every closed one that already has a Loft issue. Shared fields (title,
//! description, labels, owner) follow GitHub. Writing Loft's edits back is a later step.
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
/// Every this many polls, a full read instead of only the changes: 15 × 2 minutes.
const FULL_EVERY: u64 = 15;
/// Pages of 100 open issues read per repo at most.
const MAX_PAGES: usize = 10;
/// `created_by` of a Loft issue the mirror made.
pub const FROM_GITHUB: &str = "github";

/// What the mirror remembers per repo between polls, and a lock so two syncs of one repo never race.
#[derive(Default)]
pub struct Mirror {
    repos: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<Cursor>>>>,
}

/// Where the change poll of one repo stands.
#[derive(Default)]
struct Cursor {
    /// `since` of the change poll: the newest `updated_at` seen. None until a full read.
    since: Option<String>,
    /// ETag of the last change poll's answer, sent back so an unchanged repo answers 304.
    etag: Option<String>,
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
}

/// Polls every project's repo; the first round and every `FULL_EVERY`th are full reads.
pub async fn poller(state: AppState) {
    let mut round: u64 = 0;
    loop {
        let full = round.is_multiple_of(FULL_EVERY);
        for p in crate::issues::projects(&state).unwrap_or_default() {
            let _ = sync_project(&state, &p, full).await;
        }
        round += 1;
        tokio::time::sleep(POLL_EVERY).await;
    }
}

/// A full read of one project, in the background: after Loft publishes there, for example.
pub fn spawn_sync(state: &AppState, project: &str) {
    let (state, project) = (state.clone(), project.trim().to_string());
    tokio::spawn(async move {
        let _ = sync_project(&state, &project, true).await;
    });
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SyncBody {
    project: Option<String>,
}

/// `POST /api/issues/sync`: a full read now, of one project or all; what the list does on open and refresh.
pub async fn sync_now(State(state): State<AppState>, body: Option<Json<SyncBody>>) -> Result<Json<Value>, ApiError> {
    let wanted = body.and_then(|b| b.0.project).map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let projects = crate::issues::projects(&state)?;
    if let Some(p) = &wanted
        && !projects.contains(p)
    {
        return Err(crate::api::not_found());
    }
    let mut out = vec![];
    for p in projects.iter().filter(|p| wanted.as_ref().is_none_or(|w| w == *p)) {
        if let Some(r) = sync_project(&state, p, true).await {
            out.push(r);
        }
    }
    Ok(Json(json!({ "connected": crate::github::token(&state).is_some(), "projects": out })))
}

/// Syncs one project. None when there's nothing to mirror: no token, or no GitHub repo.
pub async fn sync_project(state: &AppState, project: &str, full: bool) -> Option<Report> {
    crate::github::token(state)?;
    let repo = crate::repos::github_repo(state, project).await.ok()?;
    let mut report = Report { project: project.into(), repo: repo.clone(), ..Default::default() };
    let cursor = state.mirror.cursor(&repo);
    let mut cursor = cursor.lock().await;
    let result = if full || cursor.since.is_none() { full_read(state, project, &repo, &mut cursor, &mut report).await } else { changes(state, project, &repo, &mut cursor, &mut report).await };
    if let Err(e) = result {
        tracing::warn!("mirror {repo}: {e}");
        report.error = Some(e.to_string());
    }
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
    for i in open.iter().filter(|i| i.get("pull_request").is_none()) {
        seen.insert(i["number"].as_i64().unwrap_or(0));
        newest = newest.max(i["updated_at"].as_str().unwrap_or("").to_string());
        apply(state, project, repo, i, report);
    }
    // Linked issues GitHub still had open last time but didn't list now: ask about each one.
    let gone: Vec<Task> = linked(state, repo).into_iter().filter(|t| t.github.as_ref().is_some_and(|g| g.state == "open" && !seen.contains(&g.number))).collect();
    for t in gone {
        let n = t.github.as_ref().unwrap().number;
        match crate::github::get(state, &format!("/repos/{repo}/issues/{n}")).await {
            Ok(i) if same_repo(&i, repo) => {
                newest = newest.max(i["updated_at"].as_str().unwrap_or("").to_string());
                apply(state, project, repo, &i, report);
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

/// What changed since the last read, open or closed. An unchanged answer comes back as 304 with the same ETag.
async fn changes(state: &AppState, project: &str, repo: &str, cursor: &mut Cursor, report: &mut Report) -> anyhow::Result<()> {
    let since = cursor.since.clone().unwrap_or_default();
    let path = format!("/repos/{repo}/issues?state=all&sort=updated&direction=asc&per_page=100&since={}", url_encode(&since));
    let Some((v, etag)) = crate::github::get_if_changed(state, &path, cursor.etag.as_deref()).await? else {
        return Ok(());
    };
    let mut newest = since.clone();
    for i in v.as_array().into_iter().flatten().filter(|i| i.get("pull_request").is_none()) {
        newest = newest.max(i["updated_at"].as_str().unwrap_or("").to_string());
        apply(state, project, repo, i, report);
    }
    // `since` is inclusive, so the newest issue comes back each time: the answer only changes, and the
    // ETag with it, when something new does. Keep the URL (and ETag) until then.
    if newest != since {
        cursor.since = Some(newest);
        cursor.etag = None;
    } else {
        cursor.etag = etag;
    }
    Ok(())
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
        url: i["html_url"].as_str().map(String::from).unwrap_or_else(|| format!("https://github.com/{repo}/issues/{number}")),
        state: i["state"].as_str().unwrap_or("open").to_string(),
        updated_at: i["updated_at"].as_str().unwrap_or("").to_string(),
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
            t.title = if title.is_empty() { t.title } else { title };
            t.description = description;
            t.labels = labels;
            t.owner = owner;
            t.github = Some(l);
            t.updated_at = now();
            if state.db.save_task(&t).is_ok() {
                crate::tasks::emit(state, &t);
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

/// Percent-encodes a timestamp for a query string (`+` and `:` in RFC 3339).
fn url_encode(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-._~".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::tests::own_fake;
    use crate::issues::tests::{project, state};

    fn gh(repo: &str, n: i64, title: &str, state: &str, updated: &str) -> Value {
        json!({
            "number": n, "id": n * 100, "title": title, "state": state, "body": format!("Body of {title}"),
            "html_url": format!("https://github.com/{repo}/issues/{n}"), "updated_at": updated,
            "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [{ "login": "octo-dev" }, { "login": "someone" }],
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
        assert_eq!(t["github"], json!({ "repo": "octo/mir-open", "number": 1, "url": "https://github.com/octo/mir-open/issues/1", "state": "open", "updated_at": "2026-10-02T00:00:00Z" }));
        let thread = st.db.list_task_comments(t["id"].as_str().unwrap(), 10, None).unwrap();
        assert_eq!(thread[0].text, "Mirrored from GitHub #1");

        // Reading the same again changes nothing and never duplicates.
        let r = sync_project(&st, "mir-open", true).await.unwrap();
        assert_eq!((r.created, r.updated), (0, 0));
        assert_eq!(mirrored(&st, "octo/mir-open").len(), 1);
        assert!(fake.seen("/repos/octo/mir-open/").iter().all(|s| s.method == "GET"), "the mirror only reads");
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
        assert_eq!(t4.status, "inbox", "closing on GitHub doesn't change the Loft status yet (write-through is #69)");
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
        assert_eq!(v, json!({ "connected": true, "projects": [] }));
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
}
