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

/// The verdicts a checker can set: (kind, label for a pass, label for a failure).
pub const VERDICTS: [(&str, &str, &str); 3] = [("tests", "tests:green", "tests:failing"), ("qa", "qa:passed", "qa:failed"), ("security", "security:passed", "security:findings")];

fn is_pass(label: &str) -> bool {
    VERDICTS.iter().any(|(_, pass, _)| *pass == label)
}

/// owner/repo + label pairs Loft has made sure exist, so a repo gets each created once.
static ENSURED: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);

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

pub(crate) fn emit(state: &AppState, pr: &PullRequest) {
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
                clear_stale_passes(state, &mut pr).await;
            }
            mark_ready_if_due(state, &mut pr).await;
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
    pr.node_id = s("node_id");
    pr.labels = v["labels"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str().map(Into::into)).collect();
    pr.merge_methods = [("squash", "allow_squash_merge"), ("merge", "allow_merge_commit"), ("rebase", "allow_rebase_merge")]
        .iter()
        .filter(|(_, k)| allowed(k))
        .map(|(m, _)| m.to_string())
        .collect();
}

/// What one check of a commit says: a check run, by its status and conclusion, or a legacy commit status, by its state.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Check {
    Passed,
    Failed,
    Pending,
}

impl Check {
    pub(crate) fn of_run(c: &Value) -> Self {
        match (c["status"].as_str(), c["conclusion"].as_str()) {
            (Some("completed"), Some("success" | "neutral" | "skipped")) => Check::Passed,
            (Some("completed"), _) => Check::Failed,
            _ => Check::Pending,
        }
    }

    pub(crate) fn of_status(s: &Value) -> Self {
        match s["state"].as_str() {
            Some("success") => Check::Passed,
            Some("pending") => Check::Pending,
            _ => Check::Failed,
        }
    }
}

/// A commit's check runs and its legacy commit statuses, as GitHub lists them, each read on its own.
pub(crate) async fn commit_checks(state: &AppState, slug: &str, sha: &str) -> (Result<Vec<Value>>, Result<Vec<Value>>) {
    let runs_path = format!("/repos/{slug}/commits/{sha}/check-runs?per_page=100");
    let status_path = format!("/repos/{slug}/commits/{sha}/status");
    let (runs, statuses) = tokio::join!(github::get(state, &runs_path), github::get(state, &status_path));
    let list = |v: Value, key: &str| v[key].as_array().cloned().unwrap_or_default();
    (runs.map(|v| list(v, "check_runs")), statuses.map(|v| list(v, "statuses")))
}

/// Summarise check runs + legacy commit statuses for a commit.
async fn checks(state: &AppState, slug: &str, sha: &str) -> Result<Value> {
    let (runs, statuses) = commit_checks(state, slug, sha).await;
    let (runs, statuses) = (runs.unwrap_or_default(), statuses.unwrap_or_default());
    let all: Vec<Check> = runs.iter().map(Check::of_run).chain(statuses.iter().map(Check::of_status)).collect();
    let count = |c: Check| all.iter().filter(|x| **x == c).count();
    let (passed, failed, pending) = (count(Check::Passed), count(Check::Failed), count(Check::Pending));
    Ok(json!({ "total": passed + failed + pending, "passed": passed, "failed": failed, "pending": pending }))
}

/// A checker's verdict on a PR: sets the label for `kind` (tests, qa or security) and removes its opposite in one
/// call, after creating the label if the repo doesn't have it. `sha` is the head commit the checker tested.
/// Only a QA, Security or Code reviewer agent in the team of the agent that opened the PR may.
pub async fn set_verdict(state: &AppState, me: &crate::db::Agent, url: &str, kind: &str, pass: bool, sha: &str) -> Result<String, String> {
    let (slug, number) = github::extract_pr_links(url).into_iter().next().ok_or("pr must look like https://github.com/<owner>/<repo>/pull/<number>")?;
    let (_, good, bad) = *VERDICTS.iter().find(|(k, _, _)| *k == kind).ok_or("kind must be tests, qa or security")?;
    // The first agent that tracked it is its opener: a later share by another agent doesn't change whose team may.
    let known = state.db.list_pull_requests(None).map_err(|e| e.to_string())?.into_iter().filter(|p| p.slug == slug && p.number == number).min_by(|a, b| a.created_at.cmp(&b.created_at));
    let pr = known.ok_or_else(|| format!("Loft doesn't track {slug}#{number}: its opener shares it with share_pull_request first."))?;
    let opener = state.db.get_agent(&pr.agent_id).ok().flatten();
    let checker = matches!(me.role.as_str(), "qa" | "security" | "code_reviewer");
    if !checker || opener.is_none_or(|o| me.team().is_empty() || o.team() != me.team()) {
        return Err("Only a QA, Security or Code reviewer agent of the team that opened this pull request can set its verdict.".into());
    }
    // Read it fresh (a 304 when unchanged): a verdict is about the head commit it was tested on.
    let pr = refresh(state, &pr.id).await.map_err(|e| e.to_string())?;
    if sha.len() < 7 || !pr.head_sha.starts_with(sha) {
        return Err(format!("Its head is now {}, not {sha}. Test the new head and set the verdict again.", pr.head_sha.chars().take(7).collect::<String>()));
    }
    let (want, opposite) = if pass { (good, bad) } else { (bad, good) };
    let ensure = format!("{slug}|{want}");
    if !ENSURED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).contains(&ensure) {
        let color = if want.ends_with("green") || want.ends_with("passed") { "2da44e" } else { "cf222e" };
        let made = github::request(state, reqwest::Method::POST, &format!("/repos/{slug}/labels"), Some(json!({ "name": want, "color": color, "description": "Set by Loft's checkers" }))).await;
        // 422 is "already exists", which is what we wanted.
        if let Err(e) = made
            && !e.to_string().starts_with("GitHub 422")
        {
            return Err(format!("Couldn't create the label {want}: {e}"));
        }
        ENSURED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(ensure);
    }
    // Add, then remove its opposite: never a full replace, so a label someone else added meanwhile survives.
    let mut pr = pr;
    add_label(state, &mut pr, want).await?;
    remove_label(state, &mut pr, opposite).await?;
    if pass {
        pr.passes_sha = pr.head_sha.clone();
    }
    if !pr.labels.iter().any(|l| is_pass(l)) {
        pr.passes_sha.clear();
    }
    // The last verdict a PR needs makes it ready now, not at the next poll.
    mark_ready_if_due(state, &mut pr).await;
    state.db.save_pull_request(&pr).map_err(|e| e.to_string())?;
    emit(state, &pr);
    Ok(format!("{want} is on {slug}#{number}."))
}

/// What a PR's changed files make it: `ios` (any file under ios/), else `ui` (anything under frontend/), else
/// `docs` (only documentation), else `backend`.
fn kind_of(files: &[String]) -> &'static str {
    let doc = |f: &str| f.starts_with("docs/") || f.starts_with("notes/") || f.to_lowercase().ends_with(".md") || f == "LICENSE";
    if files.iter().any(|f| f.starts_with("ios/")) {
        "ios"
    } else if files.iter().any(|f| f.starts_with("frontend/")) {
        "ui"
    } else if !files.is_empty() && files.iter().all(|f| doc(f)) {
        "docs"
    } else {
        "backend"
    }
}

/// The verdict kinds a PR of this kind needs passed: backend tests and security; UI and iOS add QA (and the iOS
/// tests are its `tests` verdict); docs only needs none.
fn required(kind: &str) -> &'static [&'static str] {
    match kind {
        "docs" => &[],
        "backend" => &["tests", "security"],
        _ => &["tests", "qa", "security"],
    }
}

/// Marks an open draft ready for review once the verdicts its kind of change needs are on its current head,
/// and none is a failure. One files read per draft head (cached on the PR), one GraphQL call when it flips; done
/// once per head. Loft never merges.
async fn mark_ready_if_due(state: &AppState, pr: &mut PullRequest) {
    if pr.state != "open" || !pr.draft || pr.gone || pr.node_id.is_empty() || pr.head_sha.is_empty() || pr.ready_sha == pr.head_sha {
        return;
    }
    if pr.labels.iter().any(|l| VERDICTS.iter().any(|(_, _, bad)| bad == l)) {
        return;
    }
    if pr.kind_sha != pr.head_sha {
        // More files than one page lists: take the strictest kind rather than guess.
        let kind = if pr.changed_files > 100 {
            Ok("ios")
        } else {
            match github::get(state, &format!("/repos/{}/pulls/{}/files?per_page=100", pr.slug, pr.number)).await {
                Ok(v) => Ok(kind_of(&v.as_array().into_iter().flatten().filter_map(|f| f["filename"].as_str().map(String::from)).collect::<Vec<_>>())),
                Err(e) => Err(e),
            }
        };
        match kind {
            Ok(k) => (pr.kind, pr.kind_sha) = (k.into(), pr.head_sha.clone()),
            Err(e) => return tracing::warn!("files of {}#{}: {e}", pr.slug, pr.number),
        }
    }
    let have = |kind: &str| VERDICTS.iter().any(|(k, good, _)| *k == kind && pr.labels.iter().any(|l| l == good));
    if !required(&pr.kind).iter().all(|k| have(k)) || (!required(&pr.kind).is_empty() && pr.passes_sha != pr.head_sha) {
        return;
    }
    let query = format!("mutation {{ markPullRequestReadyForReview(input: {{ pullRequestId: {} }}) {{ pullRequest {{ isDraft }} }} }}", json!(pr.node_id));
    match github::request(state, reqwest::Method::POST, "/graphql", Some(json!({ "query": query }))).await {
        Ok(v) if v["errors"].as_array().is_none_or(|e| e.is_empty()) => {
            pr.draft = false;
            pr.ready_sha = pr.head_sha.clone();
        }
        Ok(v) => tracing::warn!("marking {}#{} ready: {}", pr.slug, pr.number, v["errors"]),
        Err(e) => tracing::warn!("marking {}#{} ready: {e}", pr.slug, pr.number),
    }
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|l| l["name"].as_str().map(Into::into)).collect()
}

/// Adds one label; GitHub answers with all the PR's labels, which the PR keeps.
async fn add_label(state: &AppState, pr: &mut PullRequest, label: &str) -> Result<(), String> {
    let v = github::request(state, reqwest::Method::POST, &format!("/repos/{}/issues/{}/labels", pr.slug, pr.number), Some(json!({ "labels": [label] })))
        .await
        .map_err(|e| format!("Couldn't add {label}: {e}"))?;
    pr.labels = names(&v);
    Ok(())
}

/// Removes one label. A 404 means it wasn't on the PR, which is fine.
async fn remove_label(state: &AppState, pr: &mut PullRequest, label: &str) -> Result<(), String> {
    let path = format!("/repos/{}/issues/{}/labels/{}", pr.slug, pr.number, label.replace(':', "%3A"));
    match github::request(state, reqwest::Method::DELETE, &path, None).await {
        Ok(v) => pr.labels = names(&v),
        Err(e) if github::is_gone(&e) => pr.labels.retain(|l| l != label),
        Err(e) => return Err(format!("Couldn't remove {label}: {e}")),
    }
    Ok(())
}

/// When the head moved on from the commit the pass labels were set for, removes them (at most three, one call
/// each); failure labels stay until the next verdict. A pass first seen here (set by hand) is taken as for this head.
async fn clear_stale_passes(state: &AppState, pr: &mut PullRequest) {
    if !pr.labels.iter().any(|l| is_pass(l)) {
        pr.passes_sha.clear();
        return;
    }
    if pr.passes_sha.is_empty() || pr.head_sha.is_empty() {
        pr.passes_sha = pr.head_sha.clone();
        return;
    }
    if pr.passes_sha == pr.head_sha {
        return;
    }
    for label in pr.labels.clone().into_iter().filter(|l| is_pass(l)) {
        if let Err(e) = remove_label(state, pr, &label).await {
            return tracing::warn!("clearing the passed labels of {}#{}: {e}", pr.slug, pr.number);
        }
    }
    pr.passes_sha.clear();
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
const PR_FIELDS: &str = "id number title url state isDraft mergeable mergeStateStatus headRefName headRefOid baseRefName \
    author { login } additions deletions changedFiles commits { totalCount } comments { totalCount } reviewThreads { totalCount } \
    mergedBy { login } mergedAt closedAt labels(first: 30) { nodes { name } } \
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
            // How far its branch is behind the base, by the head branch the last read named.
            let behind = if pr.head_ref.is_empty() { String::new() } else { format!(" baseRef {{ compare(headRef: {}) {{ behindBy }} }}", json!(pr.head_ref)) };
            Some(format!("p{i}: repository(owner: {}, name: {}) {{ pullRequest(number: {}) {{ {PR_FIELDS}{behind} }} }}", json!(owner), json!(name), pr.number))
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
            clear_stale_passes(state, &mut pr).await;
            mark_ready_if_due(state, &mut pr).await;
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
    pr.node_id = s(&n["id"]);
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
    pr.behind_by = n["baseRef"]["compare"]["behindBy"].as_i64().unwrap_or(0);
    pr.author = s(&n["author"]["login"]);
    pr.additions = n["additions"].as_i64().unwrap_or(0);
    pr.deletions = n["deletions"].as_i64().unwrap_or(0);
    pr.changed_files = n["changedFiles"].as_i64().unwrap_or(0);
    pr.commits = n["commits"]["totalCount"].as_i64().unwrap_or(0);
    pr.comments = n["comments"]["totalCount"].as_i64().unwrap_or(0) + n["reviewThreads"]["totalCount"].as_i64().unwrap_or(0);
    pr.merged_by = n["mergedBy"]["login"].as_str().map(Into::into);
    pr.merged_at = n["mergedAt"].as_str().map(Into::into);
    pr.closed_at = n["closedAt"].as_str().map(Into::into);
    pr.labels = n["labels"]["nodes"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str().map(Into::into)).collect();
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

    #[tokio::test]
    async fn the_labels_of_a_pr_are_in_the_api_without_another_request() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let slug = "octo/pr-labels";
        let a = crate::tasks::tests::agent(&st, "Dev");
        let mut p = pull(slug, 5, "abc");
        p["labels"] = json!([{ "name": "tests:green" }, { "name": "qa:failed" }, { "name": "security:passed" }]);
        fake.route("GET", "/repos/octo/pr-labels/pulls/5", 200, p);
        settled_checks(&fake, slug, "abc");
        let pr = register(&st, &a.id, slug, 5, None).await.unwrap();
        let before = fake.seen("/").len();
        let axum::Json(list) = crate::api::list_pull_requests(axum::extract::State(st.clone()), axum::extract::Query(crate::api::PrQuery { agent_id: None })).await.unwrap();
        let ours = list.as_array().unwrap().iter().find(|v| v["id"] == pr.id).unwrap();
        assert_eq!(ours["labels"], json!(["tests:green", "qa:failed", "security:passed"]));
        assert_eq!(fake.seen("/").len(), before, "reading the PR list asks GitHub for nothing");
        assert_eq!(fake.seen("/repos/octo/pr-labels/pulls/5").len(), 1, "the labels came with the one PR request");
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
            "mergedBy": null, "mergedAt": null, "closedAt": null, "labels": { "nodes": [{ "name": "tests:green" }, { "name": "security:passed" }] },
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
        assert_eq!(one.labels, ["tests:green", "security:passed"], "the labels come in the same query: no extra request");
        assert_eq!(get(2).state, "merged");
        let gone = get(3);
        assert!(gone.gone && !due(&gone, true), "not found: gone, and not polled again");
    }

    #[tokio::test]
    async fn the_poller_reads_how_far_a_branch_is_behind_its_base() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let a = crate::tasks::tests::agent(&st, "Dev");
        let pr = PullRequest { id: "pr-behind".into(), agent_id: a.id.clone(), slug: "octo/pr-behind".into(), number: 1, state: "open".into(), head_ref: "b".into(), created_at: now(), ..Default::default() };
        st.db.save_pull_request(&pr).unwrap();
        let mut node = gql_pr(1, "OPEN", "aaa", json!([]));
        node["pullRequest"]["baseRef"] = json!({ "compare": { "behindBy": 4 } });
        fake.route("POST", "/graphql", 200, json!({ "data": { "p0": node } }));

        refresh_batch(&st, &[pr]).await.unwrap();
        let q = fake.seen("/graphql")[0].body["query"].as_str().unwrap().to_string();
        assert!(q.contains(r#"baseRef { compare(headRef: "b") { behindBy } }"#), "{q}");
        assert_eq!(st.db.get_pull_request("pr-behind").unwrap().unwrap().behind_by, 4);
    }


    /// A dev that opened the PR, and a reviewer of the same project.
    fn team(st: &AppState, project: &str) -> (crate::db::Agent, crate::db::Agent) {
        let mut dev = crate::tasks::tests::agent(st, &format!("Dev {project}"));
        (dev.project, dev.group) = (project.into(), project.into());
        st.db.save_agent(&dev).unwrap();
        let mut qa = crate::tasks::tests::agent(st, &format!("Tester {project}"));
        (qa.project, qa.group) = (project.into(), project.into());
        qa.role = "qa".into();
        st.db.save_agent(&qa).unwrap();
        (dev, qa)
    }

    fn label_objects(names: &[&str]) -> Value {
        json!(names.iter().map(|n| json!({ "name": n })).collect::<Vec<_>>())
    }

    #[tokio::test]
    async fn a_verdict_sets_one_label_and_removes_its_opposite_in_one_call_creating_the_label_first() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let slug = "octo/pr-verdict";
        let (dev, qa) = team(&st, "pr-verdict");
        let mut p = pull(slug, 5, "abc1234def");
        p["labels"] = label_objects(&["bug", "tests:failing"]);
        fake.route("GET", "/repos/octo/pr-verdict/pulls/5", 200, p);
        settled_checks(&fake, slug, "abc1234def");
        fake.route("POST", "/repos/octo/pr-verdict/labels", 201, json!({}));
        fake.route("POST", "/repos/octo/pr-verdict/issues/5/labels", 200, label_objects(&["bug", "tests:failing", "tests:green"]));
        fake.route("DELETE", "/repos/octo/pr-verdict/issues/5/labels/tests%3Afailing", 200, label_objects(&["bug", "tests:green"]));
        register(&st, &dev.id, slug, 5, None).await.unwrap();
        let url = "https://github.com/octo/pr-verdict/pull/5";

        let said = set_verdict(&st, &qa, url, "tests", true, "abc1234").await.unwrap();
        assert!(said.contains("tests:green"), "{said}");
        let writes: Vec<_> = fake.seen("/repos/octo/pr-verdict/").into_iter().filter(|r| r.method != "GET").collect();
        assert_eq!(writes.iter().map(|r| r.method.as_str()).collect::<Vec<_>>(), ["POST", "POST", "DELETE"], "the label is created, added, and its opposite removed");
        assert_eq!(writes[0].body["name"], "tests:green");
        assert_eq!(writes[1].body, json!({ "labels": ["tests:green"] }), "only the one label is added: no full replace");
        assert!(writes[2].path.ends_with("/labels/tests%3Afailing"), "{}", writes[2].path);
        let pr = st.db.find_pull_request(&dev.id, slug, 5).unwrap().unwrap();
        assert_eq!(pr.labels, ["bug", "tests:green"]);
        assert_eq!(pr.passes_sha, "abc1234def");

        // The label now exists: a second verdict of the same kind doesn't create it again.
        fake.route("POST", "/repos/octo/pr-verdict/issues/5/labels", 200, label_objects(&["bug", "tests:green", "qa:failed"]));
        fake.route("DELETE", "/repos/octo/pr-verdict/issues/5/labels/qa%3Apassed", 404, json!({ "message": "Label does not exist" }));
        fake.route("POST", "/repos/octo/pr-verdict/labels", 201, json!({}));
        set_verdict(&st, &qa, url, "qa", false, "abc1234").await.unwrap();
        let pr = st.db.find_pull_request(&dev.id, slug, 5).unwrap().unwrap();
        assert_eq!(pr.labels, ["bug", "tests:green", "qa:failed"], "a 404 on the opposite is fine: it wasn't there");
        let creates = fake.seen("/repos/octo/pr-verdict/labels").into_iter().filter(|r| r.method == "POST").count();
        assert_eq!(creates, 2, "one per new label, not per verdict");
        assert!(fake.seen("/repos/octo/pr-verdict/").iter().all(|r| r.method != "PUT"), "never a full replace");

        // Someone else labelled the PR after Loft last read it: their label survives, as GitHub's answer is what is kept.
        fake.route("POST", "/repos/octo/pr-verdict/issues/5/labels", 200, label_objects(&["bug", "needs-docs", "tests:green", "security:passed"]));
        fake.route("DELETE", "/repos/octo/pr-verdict/issues/5/labels/security%3Afindings", 404, json!({ "message": "Label does not exist" }));
        fake.route("POST", "/repos/octo/pr-verdict/labels", 201, json!({}));
        set_verdict(&st, &qa, url, "security", true, "abc1234").await.unwrap();
        let pr = st.db.find_pull_request(&dev.id, slug, 5).unwrap().unwrap();
        assert!(pr.labels.contains(&"needs-docs".to_string()), "{:?}", pr.labels);
    }

    #[tokio::test]
    async fn a_new_head_removes_the_passed_labels_and_keeps_the_failures() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let slug = "octo/pr-newhead";
        let (dev, _) = team(&st, "pr-newhead");
        let mut old = pull(slug, 6, "aaa1111");
        old["labels"] = label_objects(&["tests:green", "qa:failed", "security:passed"]);
        let mut new = pull(slug, 6, "bbb2222");
        new["labels"] = label_objects(&["tests:green", "qa:failed", "security:passed"]);
        fake.in_turn("GET", "/repos/octo/pr-newhead/pulls/6", vec![(200, old), (200, new)]);
        settled_checks(&fake, slug, "aaa1111");
        settled_checks(&fake, slug, "bbb2222");
        fake.route("DELETE", "/repos/octo/pr-newhead/issues/6/labels/tests%3Agreen", 200, label_objects(&["qa:failed", "security:passed"]));
        fake.route("DELETE", "/repos/octo/pr-newhead/issues/6/labels/security%3Apassed", 200, label_objects(&["qa:failed"]));
        let pr = register(&st, &dev.id, slug, 6, None).await.unwrap();
        assert_eq!(pr.passes_sha, "aaa1111", "passes first seen are taken as for that head");
        assert!(fake.seen("/repos/octo/pr-newhead/issues").is_empty(), "same head: nothing is removed");

        let pr = refresh(&st, &pr.id).await.unwrap();
        let calls: Vec<_> = fake.seen("/repos/octo/pr-newhead/issues/6/labels");
        assert_eq!(calls.iter().map(|r| r.method.as_str()).collect::<Vec<_>>(), ["DELETE", "DELETE"], "the two passes, one call each, no full replace");
        assert_eq!((pr.labels.clone(), pr.passes_sha.as_str()), (vec!["qa:failed".to_string()], ""));
    }

    #[tokio::test]
    async fn only_a_checker_of_the_prs_team_can_set_a_verdict_and_only_for_its_head() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let slug = "octo/pr-auth";
        let (dev, qa) = team(&st, "pr-auth");
        let (_, other_qa) = team(&st, "elsewhere");
        fake.route("GET", "/repos/octo/pr-auth/pulls/7", 200, pull(slug, 7, "ccc3333444"));
        settled_checks(&fake, slug, "ccc3333444");
        register(&st, &dev.id, slug, 7, None).await.unwrap();
        let url = "https://github.com/octo/pr-auth/pull/7";
        let before = fake.seen("/").len();

        for who in [&dev, &other_qa] {
            let err = set_verdict(&st, who, url, "tests", true, "ccc3333").await.unwrap_err();
            assert!(err.starts_with("Only a QA, Security or Code reviewer agent of the team"), "{err}");
        }
        // Every checker role of the team's group may, wherever its project is; the old "reviewer" role may not.
        let mut sec = qa.clone();
        (sec.role, sec.group, sec.project) = ("security".into(), "Grp".into(), "other-repo".into());
        let mut opener = dev.clone();
        (opener.group, opener.project) = ("Grp".into(), "pr-auth".into());
        st.db.save_agent(&opener).unwrap();
        let mut qa = qa;
        qa.group = "Grp".into();
        st.db.save_agent(&qa).unwrap();
        let mut old = sec.clone();
        old.role = "reviewer".into();
        assert!(set_verdict(&st, &old, url, "tests", true, "zzz9999").await.unwrap_err().starts_with("Only a QA"));
        let err = set_verdict(&st, &sec, url, "tests", true, "zzz9999").await.unwrap_err();
        assert!(err.contains("Its head is now"), "{err}");
        let err = set_verdict(&st, &qa, url, "tests", true, "zzz9999").await.unwrap_err();
        assert!(err.contains("Its head is now ccc3333"), "{err}");
        assert!(set_verdict(&st, &qa, url, "lint", true, "ccc3333").await.unwrap_err().contains("kind must be"));
        assert!(set_verdict(&st, &qa, "https://github.com/octo/pr-auth/pull/99", "tests", true, "ccc3333").await.unwrap_err().contains("doesn't track octo/pr-auth#99"));
        let writes = fake.seen("/").into_iter().skip(before).filter(|r| r.method != "GET").count();
        assert_eq!(writes, 0, "a refused verdict writes nothing to GitHub");
    }


    /// A draft PR with the given labels, and the files it changes, for the ready rule.
    async fn draft_pr(st: &AppState, fake: &crate::github::tests::Fake, project: &str, n: i64, labels: &[&str], files: &[&str]) -> PullRequest {
        let slug = format!("octo/{project}");
        let (dev, _) = team(st, project);
        let mut p = pull(&slug, n, "abcdef1234");
        p["draft"] = json!(true);
        p["node_id"] = json!(format!("PR_node_{project}"));
        p["labels"] = label_objects(labels);
        fake.route("GET", &format!("/repos/{slug}/pulls/{n}"), 200, p);
        settled_checks(fake, &slug, "abcdef1234");
        fake.route("GET", &format!("/repos/{slug}/pulls/{n}/files"), 200, json!(files.iter().map(|f| json!({ "filename": f })).collect::<Vec<_>>()));
        fake.route("POST", "/graphql", 200, json!({ "data": { "markPullRequestReadyForReview": { "pullRequest": { "isDraft": false } } } }));
        register(st, &dev.id, &slug, n, None).await.unwrap()
    }

    fn ready_calls(fake: &crate::github::tests::Fake) -> usize {
        fake.seen("/graphql").iter().filter(|r| r.body["query"].as_str().is_some_and(|q| q.contains("markPullRequestReadyForReview"))).count()
    }

    #[tokio::test]
    async fn a_later_share_by_another_agent_doesnt_change_who_may_set_verdicts() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let slug = "octo/pr-takeover";
        let (dev, qa) = team(&st, "pr-takeover");
        let (other_dev, other_qa) = team(&st, "pr-takeover-b");
        fake.route("GET", "/repos/octo/pr-takeover/pulls/7", 200, pull(slug, 7, "ccc3333444"));
        settled_checks(&fake, slug, "ccc3333444");
        fake.route("POST", "/repos/octo/pr-takeover/labels", 201, json!({}));
        fake.route("POST", "/repos/octo/pr-takeover/issues/7/labels", 200, label_objects(&["qa:passed"]));
        fake.route("DELETE", "/repos/octo/pr-takeover/issues/7/labels/qa%3Afailed", 404, json!({ "message": "Label does not exist" }));
        register(&st, &dev.id, slug, 7, None).await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        register(&st, &other_dev.id, slug, 7, None).await.unwrap();
        let url = "https://github.com/octo/pr-takeover/pull/7";

        assert!(set_verdict(&st, &other_qa, url, "qa", true, "ccc3333").await.unwrap_err().starts_with("Only a QA"));
        set_verdict(&st, &qa, url, "qa", true, "ccc3333").await.unwrap();
    }

    #[tokio::test]
    async fn a_backend_pr_with_tests_and_security_passed_becomes_ready_once() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let pr = draft_pr(&st, &fake, "rdy-backend", 1, &["tests:green", "security:passed"], &["backend/src/prs.rs", "docs/x.md"]).await;
        assert_eq!((pr.kind.as_str(), pr.draft, pr.ready_sha.as_str()), ("backend", false, "abcdef1234"));
        assert_eq!(ready_calls(&fake), 1);
        assert!(fake.seen("/graphql")[0].body["query"].as_str().unwrap().contains("PR_node_rdy-backend"));
        // GitHub still says draft on the next read (the user flipped it back): not flipped again for the same head.
        let pr = refresh(&st, &pr.id).await.unwrap();
        assert!(pr.draft && pr.ready_sha == "abcdef1234");
        assert_eq!(ready_calls(&fake), 1, "once per head");
        assert_eq!(fake.seen("/repos/octo/rdy-backend/pulls/1/files").len(), 1, "the files are read once per head");
    }

    #[tokio::test]
    async fn a_ui_or_ios_pr_needs_qa_too() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let ui = draft_pr(&st, &fake, "rdy-ui", 2, &["tests:green", "security:passed"], &["frontend/src/a.tsx", "backend/src/b.rs"]).await;
        assert_eq!((ui.kind.as_str(), ui.draft), ("ui", true), "no qa:passed: still a draft");
        assert_eq!(ready_calls(&fake), 0);
        let ios = draft_pr(&st, &fake, "rdy-ios", 3, &["tests:green", "security:passed"], &["ios/Loft/A.swift"]).await;
        assert_eq!((ios.kind.as_str(), ios.draft), ("ios", true));
        let ui_ok = draft_pr(&st, &fake, "rdy-ui2", 4, &["tests:green", "security:passed", "qa:passed"], &["frontend/src/a.tsx"]).await;
        assert!(!ui_ok.draft, "with qa:passed it is ready");
    }

    #[tokio::test]
    async fn a_failure_label_keeps_the_pr_a_draft_and_a_docs_pr_needs_no_verdict() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let failing = draft_pr(&st, &fake, "rdy-fail", 5, &["tests:green", "security:passed", "qa:failed"], &["backend/src/a.rs"]).await;
        assert!(failing.draft, "a failure keeps it a draft");
        let docs = draft_pr(&st, &fake, "rdy-docs", 6, &[], &["docs/plans/a.md", "README.md"]).await;
        assert_eq!((docs.kind.as_str(), docs.draft), ("docs", false), "docs only: ready without any verdict");
        let docs_failing = draft_pr(&st, &fake, "rdy-docs2", 7, &["tests:failing"], &["docs/a.md"]).await;
        assert!(docs_failing.draft);
        assert_eq!(ready_calls(&fake), 1);
    }

    #[test]
    fn the_kind_of_a_change_comes_from_its_files() {
        let k = |f: &[&str]| kind_of(&f.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(k(&["backend/src/a.rs"]), "backend");
        assert_eq!(k(&["backend/src/a.rs", "frontend/src/a.tsx"]), "ui");
        assert_eq!(k(&["frontend/src/a.tsx", "ios/Loft/A.swift"]), "ios");
        assert_eq!(k(&["docs/a.md", "notes/b.txt"]), "docs");
        assert_eq!(k(&["docs/a.md", "setup.sh"]), "backend");
        assert_eq!(k(&[]), "backend", "nothing known: not docs-only");
    }

    #[tokio::test]
    async fn the_last_verdict_marks_it_ready_and_needs_you_says_so() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let pr = draft_pr(&st, &fake, "rdy-last", 8, &["tests:green"], &["backend/src/a.rs"]).await;
        assert!(pr.draft);
        let qa = st.db.list_agents().unwrap().into_iter().find(|a| a.name == "Tester rdy-last").unwrap();
        fake.route("POST", "/repos/octo/rdy-last/labels", 201, json!({}));
        fake.route("POST", "/repos/octo/rdy-last/issues/8/labels", 200, label_objects(&["tests:green", "security:passed"]));
        fake.route("DELETE", "/repos/octo/rdy-last/issues/8/labels/security%3Afindings", 404, json!({ "message": "Label does not exist" }));
        set_verdict(&st, &qa, "https://github.com/octo/rdy-last/pull/8", "security", true, "abcdef1").await.unwrap();
        let pr = st.db.get_pull_request(&pr.id).unwrap().unwrap();
        assert!(!pr.draft && pr.ready_sha == "abcdef1234");
        assert_eq!(ready_calls(&fake), 1);
        let items = crate::needs_you::snapshot(&st).unwrap();
        let reasons: Vec<&str> = items["items"].as_array().unwrap().iter().filter(|i| i["type"] == "pr_ready").filter_map(|i| i["reason"].as_str()).collect();
        assert!(reasons.iter().any(|r| r.starts_with("Loft marked it ready")), "{reasons:?}");
    }

}
