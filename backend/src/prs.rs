//! Pull requests opened by agents: detection, live status polling, and merging.

use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::db::{PullRequest, now};
use crate::github;
use crate::state::{AppState, Event};

const POLL_EVERY: Duration = Duration::from_secs(30);
/// Drafts, and every PR while no app is open, are polled this often instead.
const SLOW_EVERY: Duration = Duration::from_secs(5 * 60);

/// PRs with a mergeability retry already waiting, so refreshes don't stack more.
static RETRYING: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);

/// Does this tool call open a pull request?
pub fn creates_pr(tool: &str, input: &Value) -> bool {
    if tool.contains("create_pull_request") || tool == "mcp__loft__share_pull_request" {
        return true;
    }
    if tool != "Bash" {
        return false;
    }
    let cmd = input["command"].as_str().unwrap_or("");
    let api_post = cmd.contains("gh api")
        && cmd.contains("/pulls")
        && (cmd.contains("POST") || cmd.contains(" -f ") || cmd.contains(" -F ") || cmd.contains("--field") || cmd.contains("--raw-field"));
    cmd.contains("gh pr create") || api_post
}

fn emit(state: &AppState, pr: &PullRequest) {
    state.emit(Event::PullRequest { pr: serde_json::to_value(pr).unwrap() });
}

/// Record a PR for an agent (idempotent) and fetch its status.
pub async fn register(state: &AppState, agent_id: &str, slug: &str, number: i64, tool_use_id: Option<String>) -> Result<PullRequest> {
    if let Some(existing) = state.db.find_pull_request(agent_id, slug, number)? {
        return Ok(existing);
    }
    let pr = PullRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent_id: agent_id.into(),
        slug: slug.into(),
        number,
        url: format!("https://github.com/{slug}/pull/{number}"),
        tool_use_id,
        created_at: now(),
        ..Default::default()
    };
    state.db.save_pull_request(&pr)?;
    Ok(refresh(state, &pr.id).await.unwrap_or(pr))
}

/// Pull fresh state, mergeability and checks from GitHub.
pub async fn refresh(state: &AppState, id: &str) -> Result<PullRequest> {
    let pr = refresh_once(state, id).await?;
    // GitHub computes mergeability lazily; ask again shortly when it's unknown, with one retry
    // waiting per PR at most.
    let first = RETRYING.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(pr.id.clone());
    if !first {
        return Ok(pr);
    }
    if pr.state != "open" || pr.mergeable.is_some() || pr.error.is_some() {
        RETRYING.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|s| s.remove(&pr.id));
    } else {
        let (state, id) = (state.clone(), pr.id.clone());
        tokio::spawn(github::as_source("prs", async move {
            for delay in [3, 8, 20] {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                match refresh_once(&state, &id).await {
                    Ok(p) if p.mergeable.is_none() && p.state == "open" => continue,
                    _ => break,
                }
            }
            RETRYING.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|s| s.remove(&id));
        }));
    }
    Ok(pr)
}

async fn refresh_once(state: &AppState, id: &str) -> Result<PullRequest> {
    let Some(mut pr) = state.db.get_pull_request(id)? else { bail!("no such pull request") };
    // With the ETag, an unchanged PR answers 304, which doesn't count against the rate limit.
    match github::get_if_changed(state, &format!("/repos/{}/pulls/{}", pr.slug, pr.number), pr.etag.as_deref()).await {
        Ok(changed) => {
            if let Some((v, etag)) = changed {
                apply(&mut pr, &v);
                pr.etag = etag;
            }
            pr.error = None;
            pr.gone = false;
            // Checks change without the PR changing, so they're read until they settle on its head.
            let settled = pr.checks_sha == pr.head_sha && pr.checks["pending"].as_i64() == Some(0);
            if pr.state == "open"
                && !pr.head_sha.is_empty()
                && !settled
                && let Ok(c) = checks(state, &pr.slug, &pr.head_sha).await
            {
                pr.checks = c;
                pr.checks_sha = pr.head_sha.clone();
            }
        }
        // Paused for the rate limit: nothing went out, so the card keeps what it had.
        Err(e) if github::is_paused(&e) => return Ok(pr),
        Err(e) if github::is_gone(&e) => {
            pr.gone = true;
            pr.error = Some("This pull request is gone from GitHub (deleted, or its repo is), so Loft stopped checking it.".into());
        }
        Err(e) => pr.error = Some(e.to_string()),
    }
    pr.checked_at = Some(now());
    state.db.save_pull_request(&pr)?;
    emit(state, &pr);
    Ok(pr)
}

fn apply(pr: &mut PullRequest, v: &Value) {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    pr.title = s("title");
    pr.url = v["html_url"].as_str().map(Into::into).unwrap_or(pr.url.clone());
    pr.draft = v["draft"].as_bool().unwrap_or(false);
    pr.state = if v["merged"].as_bool().unwrap_or(false) || !v["merged_at"].is_null() {
        "merged".into()
    } else {
        s("state")
    };
    pr.mergeable = v["mergeable"].as_bool();
    pr.mergeable_state = s("mergeable_state");
    pr.head_ref = v["head"]["ref"].as_str().unwrap_or("").into();
    pr.head_sha = v["head"]["sha"].as_str().unwrap_or("").into();
    pr.base_ref = v["base"]["ref"].as_str().unwrap_or("").into();
    pr.author = v["user"]["login"].as_str().unwrap_or("").into();
    pr.additions = v["additions"].as_i64().unwrap_or(0);
    pr.deletions = v["deletions"].as_i64().unwrap_or(0);
    pr.changed_files = v["changed_files"].as_i64().unwrap_or(0);
    pr.commits = v["commits"].as_i64().unwrap_or(0);
    pr.comments = v["comments"].as_i64().unwrap_or(0) + v["review_comments"].as_i64().unwrap_or(0);
    pr.merged_by = v["merged_by"]["login"].as_str().map(Into::into);
    pr.merged_at = v["merged_at"].as_str().map(Into::into);
    pr.closed_at = v["closed_at"].as_str().map(Into::into);
    let repo = &v["base"]["repo"];
    let allowed = |k: &str| repo[k].as_bool().unwrap_or(true);
    pr.merge_methods = [("squash", "allow_squash_merge"), ("merge", "allow_merge_commit"), ("rebase", "allow_rebase_merge")]
        .iter()
        .filter(|(_, k)| allowed(k))
        .map(|(m, _)| m.to_string())
        .collect();
}

/// Summarise check runs + legacy commit statuses for a commit.
async fn checks(state: &AppState, slug: &str, sha: &str) -> Result<Value> {
    let runs_path = format!("/repos/{slug}/commits/{sha}/check-runs?per_page=100");
    let status_path = format!("/repos/{slug}/commits/{sha}/status");
    let (runs, statuses) = tokio::join!(github::get(state, &runs_path), github::get(state, &status_path));
    let (mut passed, mut failed, mut pending) = (0, 0, 0);
    if let Ok(r) = runs {
        for c in r["check_runs"].as_array().into_iter().flatten() {
            match (c["status"].as_str(), c["conclusion"].as_str()) {
                (Some("completed"), Some("success" | "neutral" | "skipped")) => passed += 1,
                (Some("completed"), _) => failed += 1,
                _ => pending += 1,
            }
        }
    }
    if let Ok(s) = statuses {
        for c in s["statuses"].as_array().into_iter().flatten() {
            match c["state"].as_str() {
                Some("success") => passed += 1,
                Some("pending") => pending += 1,
                _ => failed += 1,
            }
        }
    }
    Ok(json!({ "total": passed + failed + pending, "passed": passed, "failed": failed, "pending": pending }))
}

pub async fn merge(state: &AppState, id: &str, method: &str) -> Result<PullRequest> {
    let pr = refresh(state, id).await?;
    if pr.state != "open" {
        bail!("pull request is already {}", pr.state);
    }
    if pr.draft {
        bail!("draft pull requests can't be merged");
    }
    let body = json!({ "merge_method": method, "sha": pr.head_sha });
    github::request(
        state,
        reqwest::Method::PUT,
        &format!("/repos/{}/pulls/{}/merge", pr.slug, pr.number),
        Some(body),
    )
    .await?;
    refresh(state, id).await
}

/// Keep open PRs current so merges made elsewhere show up.
pub async fn poller(state: AppState) {
    let mut tick = tokio::time::interval(POLL_EVERY);
    github::as_source("prs", async {
        loop {
            tick.tick().await;
            let Ok(prs) = state.db.list_pull_requests(None) else { continue };
            // An open app (the web or a phone) is someone looking.
            let watched = state.events.receiver_count() > 0;
            let due: Vec<PullRequest> = prs.into_iter().filter(|p| due(p, watched)).collect();
            for chunk in due.chunks(BATCH) {
                if let Err(e) = refresh_batch(&state, chunk).await
                    && !github::is_paused(&e)
                {
                    tracing::warn!("pull requests: {e}");
                }
            }
        }
    })
    .await
}

/// PRs read in one GraphQL query; the query costs about one point however many it holds.
const BATCH: usize = 20;

/// What the poller reads of each PR, the same fields `apply` and `checks` give the card.
const PR_FIELDS: &str = "number title url state isDraft mergeable mergeStateStatus headRefName headRefOid baseRefName \
    author { login } additions deletions changedFiles commits { totalCount } comments { totalCount } reviewThreads { totalCount } \
    mergedBy { login } mergedAt closedAt \
    baseRepository { squashMergeAllowed mergeCommitAllowed rebaseMergeAllowed } \
    last: commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes { \
    __typename ... on CheckRun { status conclusion } ... on StatusContext { state } } } } } } }";

/// One GraphQL query for all these PRs, instead of a pull, a check-runs and a status request each.
/// A PR GitHub no longer has is marked gone. The REST `refresh` stays for clicks and merges.
async fn refresh_batch(state: &AppState, prs: &[PullRequest]) -> Result<()> {
    let parts: Vec<String> = prs
        .iter()
        .enumerate()
        .filter_map(|(i, pr)| {
            let (owner, name) = pr.slug.split_once('/')?;
            Some(format!("p{i}: repository(owner: {}, name: {}) {{ pullRequest(number: {}) {{ {PR_FIELDS} }} }}", json!(owner), json!(name), pr.number))
        })
        .collect();
    if parts.is_empty() {
        return Ok(());
    }
    let v = github::request(state, reqwest::Method::POST, "/graphql", Some(json!({ "query": format!("query {{ {} }}", parts.join(" ")) }))).await?;
    for (i, pr) in prs.iter().enumerate() {
        let Some(mut pr) = state.db.get_pull_request(&pr.id)? else { continue };
        let key = format!("p{i}");
        let node = &v["data"][&key]["pullRequest"];
        if node.is_null() {
            // Not found (or no access), as GitHub reports in `errors` next to the data; anything else waits a tick.
            let missing = v["errors"].as_array().is_some_and(|es| es.iter().any(|e| e["path"][0] == key.as_str() && e["type"] == "NOT_FOUND"));
            if !missing {
                continue;
            }
            pr.gone = true;
            pr.error = Some("This pull request is gone from GitHub (deleted, or its repo is), so Loft stopped checking it.".into());
        } else {
            apply_graphql(&mut pr, node);
            pr.error = None;
        }
        pr.checked_at = Some(now());
        state.db.save_pull_request(&pr)?;
        emit(state, &pr);
    }
    Ok(())
}

/// The GraphQL PR onto the card, as `apply` and `checks` do with the REST answers.
fn apply_graphql(pr: &mut PullRequest, n: &Value) {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    pr.title = s(&n["title"]);
    if let Some(u) = n["url"].as_str() {
        pr.url = u.into();
    }
    pr.draft = n["isDraft"].as_bool().unwrap_or(false);
    pr.state = s(&n["state"]).to_lowercase();
    pr.mergeable = match n["mergeable"].as_str() {
        Some("MERGEABLE") => Some(true),
        Some("CONFLICTING") => Some(false),
        _ => None,
    };
    pr.mergeable_state = s(&n["mergeStateStatus"]).to_lowercase();
    pr.head_ref = s(&n["headRefName"]);
    pr.head_sha = s(&n["headRefOid"]);
    pr.base_ref = s(&n["baseRefName"]);
    pr.author = s(&n["author"]["login"]);
    pr.additions = n["additions"].as_i64().unwrap_or(0);
    pr.deletions = n["deletions"].as_i64().unwrap_or(0);
    pr.changed_files = n["changedFiles"].as_i64().unwrap_or(0);
    pr.commits = n["commits"]["totalCount"].as_i64().unwrap_or(0);
    pr.comments = n["comments"]["totalCount"].as_i64().unwrap_or(0) + n["reviewThreads"]["totalCount"].as_i64().unwrap_or(0);
    pr.merged_by = n["mergedBy"]["login"].as_str().map(Into::into);
    pr.merged_at = n["mergedAt"].as_str().map(Into::into);
    pr.closed_at = n["closedAt"].as_str().map(Into::into);
    let repo = &n["baseRepository"];
    let allowed = |k: &str| repo[k].as_bool().unwrap_or(true);
    pr.merge_methods = [("squash", "squashMergeAllowed"), ("merge", "mergeCommitAllowed"), ("rebase", "rebaseMergeAllowed")]
        .iter()
        .filter(|(_, k)| allowed(k))
        .map(|(m, _)| m.to_string())
        .collect();
    let (mut passed, mut failed, mut pending) = (0, 0, 0);
    let contexts = &n["last"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"];
    for c in contexts.as_array().into_iter().flatten() {
        match (c["__typename"].as_str(), c["status"].as_str(), c["conclusion"].as_str(), c["state"].as_str()) {
            (Some("CheckRun"), Some("COMPLETED"), Some("SUCCESS" | "NEUTRAL" | "SKIPPED"), _) => passed += 1,
            (Some("CheckRun"), Some("COMPLETED"), _, _) => failed += 1,
            (Some("CheckRun"), _, _, _) => pending += 1,
            (_, _, _, Some("SUCCESS")) => passed += 1,
            (_, _, _, Some("PENDING" | "EXPECTED")) => pending += 1,
            _ => failed += 1,
        }
    }
    pr.checks = json!({ "total": passed + failed + pending, "passed": passed, "failed": failed, "pending": pending });
    pr.checks_sha = pr.head_sha.clone();
}

/// Does `pr` get polled this tick: open and not gone, every tick while someone looks at a non-draft,
/// else every `SLOW_EVERY`.
fn due(pr: &PullRequest, watched: bool) -> bool {
    if pr.gone || !(pr.state == "open" || pr.state.is_empty()) {
        return false;
    }
    if watched && !pr.draft {
        return true;
    }
    let since = pr.checked_at.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok());
    since.is_none_or(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).to_std().unwrap_or_default() >= SLOW_EVERY)
}

/// Find PRs agents opened before this feature existed, anchored to their tool calls.
pub async fn backfill(state: AppState) {
    let Ok(uses) = state.db.pr_tool_uses() else { return };
    for (use_msg, result) in uses {
        let name = use_msg.meta["name"].as_str().unwrap_or("");
        if !creates_pr(name, &use_msg.meta["input"]) {
            continue;
        }
        let tool_use_id = use_msg.meta["id"].as_str().map(String::from);
        for (slug, n) in github::extract_pr_links(&result.content) {
            let _ = register(&state, &use_msg.agent_id, &slug, n, tool_use_id.clone()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::tests::{own_fake, state_with_token};

    fn pull(slug: &str, n: i64, sha: &str) -> Value {
        json!({ "number": n, "title": "Retry cap", "state": "open", "draft": false, "mergeable": true, "mergeable_state": "clean",
                "head": { "ref": "cap", "sha": sha }, "base": { "ref": "main", "repo": {} }, "user": { "login": "octo" },
                "html_url": format!("https://github.com/{slug}/pull/{n}") })
    }

    fn settled_checks(fake: &crate::github::tests::Fake, slug: &str, sha: &str) {
        fake.route("GET", &format!("/repos/{slug}/commits/{sha}/check-runs"), 200, json!({ "check_runs": [{ "status": "completed", "conclusion": "success" }] }));
        fake.route("GET", &format!("/repos/{slug}/commits/{sha}/status"), 200, json!({ "statuses": [] }));
    }

    #[tokio::test]
    async fn an_unchanged_pr_costs_one_304_and_no_checks_call() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let slug = "octo/pr-etag";
        let a = crate::tasks::tests::agent(&st, "Dev");
        fake.in_turn("GET", "/repos/octo/pr-etag/pulls/7", vec![(200, pull(slug, 7, "abc")), (304, Value::Null)]);
        fake.with_headers("GET", "/repos/octo/pr-etag/pulls/7", &[("etag", "\"v1\"")]);
        settled_checks(&fake, slug, "abc");

        let pr = register(&st, &a.id, slug, 7, None).await.unwrap();
        assert_eq!((pr.checks["passed"].as_i64(), pr.etag.as_deref(), pr.checks_sha.as_str()), (Some(1), Some("\"v1\""), "abc"));
        let before_checks = fake.seen("/repos/octo/pr-etag/commits/").len();
        let pr = refresh(&st, &pr.id).await.unwrap();
        assert_eq!((pr.title.as_str(), pr.checks["passed"].as_i64(), pr.error.as_deref()), ("Retry cap", Some(1), None), "the card keeps what it had");
        let pulls = fake.seen("/repos/octo/pr-etag/pulls/7");
        assert_eq!(pulls.len(), 2);
        assert_eq!(pulls[1].headers["if-none-match"], "\"v1\"");
        assert_eq!(fake.seen("/repos/octo/pr-etag/commits/").len(), before_checks, "settled checks on the same head aren't read again");
    }

    #[tokio::test]
    async fn pending_checks_and_a_new_head_are_read_again() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let slug = "octo/pr-pending";
        let a = crate::tasks::tests::agent(&st, "Dev");
        fake.in_turn("GET", "/repos/octo/pr-pending/pulls/8", vec![(200, pull(slug, 8, "abc")), (304, Value::Null), (200, pull(slug, 8, "def"))]);
        fake.in_turn("GET", "/repos/octo/pr-pending/commits/abc/check-runs", vec![(200, json!({ "check_runs": [{ "status": "in_progress" }] })), (200, json!({ "check_runs": [{ "status": "completed", "conclusion": "success" }] }))]);
        fake.route("GET", "/repos/octo/pr-pending/commits/abc/status", 200, json!({ "statuses": [] }));
        settled_checks(&fake, slug, "def");
        let pr = register(&st, &a.id, slug, 8, None).await.unwrap();
        assert_eq!(pr.checks["pending"], 1);
        let pr = refresh(&st, &pr.id).await.unwrap();
        assert_eq!((pr.checks["pending"].as_i64(), pr.checks["passed"].as_i64()), (Some(0), Some(1)), "pending checks are read on a 304 too");
        let pr = refresh(&st, &pr.id).await.unwrap();
        assert_eq!((pr.head_sha.as_str(), pr.checks_sha.as_str()), ("def", "def"));
        assert_eq!(fake.seen("/repos/octo/pr-pending/commits/def/check-runs").len(), 1);
    }

    #[tokio::test]
    async fn a_pr_github_no_longer_has_stops_being_polled() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let a = crate::tasks::tests::agent(&st, "Dev");
        fake.in_turn("GET", "/repos/octo/pr-gone/pulls/9", vec![(200, pull("octo/pr-gone", 9, "abc")), (404, json!({ "message": "Not Found" }))]);
        settled_checks(&fake, "octo/pr-gone", "abc");
        let pr = register(&st, &a.id, "octo/pr-gone", 9, None).await.unwrap();
        assert!(due(&pr, true));
        let pr = refresh(&st, &pr.id).await.unwrap();
        assert!(pr.gone && pr.error.as_deref().is_some_and(|e| e.contains("gone from GitHub")), "{:?}", pr.error);
        assert!(!due(&pr, true) && !due(&pr, false), "never polled again");
    }

    #[test]
    fn drafts_and_unwatched_prs_poll_slowly() {
        let recent = PullRequest { state: "open".into(), checked_at: Some(now()), ..Default::default() };
        assert!(due(&recent, true), "watched: every tick");
        assert!(!due(&recent, false), "nobody looking: every 5 min");
        assert!(!due(&PullRequest { draft: true, ..recent.clone() }, true), "a draft: every 5 min");
        let old = (chrono::Utc::now() - chrono::Duration::minutes(6)).to_rfc3339();
        assert!(due(&PullRequest { draft: true, checked_at: Some(old.clone()), ..recent.clone() }, false));
        assert!(!due(&PullRequest { state: "merged".into(), checked_at: Some(old), ..recent }, true));
    }

    #[tokio::test]
    async fn only_one_mergeability_retry_waits_per_pr() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let a = crate::tasks::tests::agent(&st, "Dev");
        let mut unknown = pull("octo/pr-retry", 3, "abc");
        unknown["mergeable"] = Value::Null;
        fake.route("GET", "/repos/octo/pr-retry/pulls/3", 200, unknown);
        settled_checks(&fake, "octo/pr-retry", "abc");
        let pr = register(&st, &a.id, "octo/pr-retry", 3, None).await.unwrap();
        for _ in 0..4 {
            refresh(&st, &pr.id).await.unwrap();
        }
        let waiting = RETRYING.lock().unwrap().as_ref().map_or(0, |s| s.iter().filter(|i| **i == pr.id).count());
        assert_eq!(waiting, 1);
    }

    fn gql_pr(n: i64, state: &str, sha: &str, checks: Value) -> Value {
        json!({ "pullRequest": {
            "number": n, "title": format!("PR {n}"), "url": format!("https://github.com/octo/pr-batch/pull/{n}"), "state": state,
            "isDraft": false, "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN", "headRefName": "b", "headRefOid": sha,
            "baseRefName": "main", "author": { "login": "octo" }, "additions": 3, "deletions": 1, "changedFiles": 2,
            "commits": { "totalCount": 1 }, "comments": { "totalCount": 2 }, "reviewThreads": { "totalCount": 1 },
            "mergedBy": null, "mergedAt": null, "closedAt": null,
            "baseRepository": { "squashMergeAllowed": true, "mergeCommitAllowed": false, "rebaseMergeAllowed": true },
            "last": { "nodes": [{ "commit": { "statusCheckRollup": { "contexts": { "nodes": checks } } } }] },
        } })
    }

    #[tokio::test]
    async fn the_poller_reads_every_open_pr_in_one_graphql_query() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let a = crate::tasks::tests::agent(&st, "Dev");
        let mut prs = vec![];
        for n in [1, 2, 3] {
            let pr = PullRequest { id: format!("pr-batch-{n}"), agent_id: a.id.clone(), slug: "octo/pr-batch".into(), number: n, state: "open".into(), created_at: now(), ..Default::default() };
            st.db.save_pull_request(&pr).unwrap();
            prs.push(pr);
        }
        let checks = json!([
            { "__typename": "CheckRun", "status": "COMPLETED", "conclusion": "SUCCESS" },
            { "__typename": "CheckRun", "status": "IN_PROGRESS", "conclusion": null },
            { "__typename": "StatusContext", "state": "FAILURE" },
        ]);
        fake.route("POST", "/graphql", 200, json!({
            "data": { "p0": gql_pr(1, "OPEN", "aaa", checks), "p1": gql_pr(2, "MERGED", "bbb", json!([])), "p2": { "pullRequest": null } },
            "errors": [{ "type": "NOT_FOUND", "path": ["p2", "pullRequest"], "message": "Could not resolve to a PullRequest with the number of 3." }],
        }));

        refresh_batch(&st, &prs).await.unwrap();
        assert_eq!(fake.seen("/").len(), 1, "one request for three PRs, no REST calls");
        let q = fake.seen("/graphql")[0].body["query"].as_str().unwrap().to_string();
        assert!(q.contains(r#"p0: repository(owner: "octo", name: "pr-batch") { pullRequest(number: 1)"#) && q.contains("p2:"), "{q}");

        let get = |n: i64| st.db.get_pull_request(&format!("pr-batch-{n}")).unwrap().unwrap();
        let one = get(1);
        assert_eq!((one.title.as_str(), one.state.as_str(), one.mergeable, one.mergeable_state.as_str(), one.head_sha.as_str()), ("PR 1", "open", Some(true), "clean", "aaa"));
        assert_eq!(one.checks, json!({ "total": 3, "passed": 1, "failed": 1, "pending": 1 }));
        assert_eq!((one.comments, one.merge_methods.clone(), one.checks_sha.as_str()), (3, vec!["squash".to_string(), "rebase".to_string()], "aaa"));
        assert_eq!(get(2).state, "merged");
        let gone = get(3);
        assert!(gone.gone && !due(&gone, true), "not found: gone, and not polled again");
    }

}
