//! Pull requests opened by agents: detection, live status polling, and merging.

use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::db::{PullRequest, now};
use crate::github;
use crate::state::{AppState, Event};

const POLL_EVERY: Duration = Duration::from_secs(30);

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
    // GitHub computes mergeability lazily; ask again shortly when it's unknown.
    if pr.state == "open" && pr.mergeable.is_none() && pr.error.is_none() {
        let (state, id) = (state.clone(), pr.id.clone());
        tokio::spawn(github::as_source("prs", async move {
            for delay in [3, 8, 20] {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                match refresh_once(&state, &id).await {
                    Ok(p) if p.mergeable.is_none() && p.state == "open" => continue,
                    _ => break,
                }
            }
        }));
    }
    Ok(pr)
}

async fn refresh_once(state: &AppState, id: &str) -> Result<PullRequest> {
    let Some(mut pr) = state.db.get_pull_request(id)? else { bail!("no such pull request") };
    match github::get(state, &format!("/repos/{}/pulls/{}", pr.slug, pr.number)).await {
        Ok(v) => {
            apply(&mut pr, &v);
            pr.error = None;
            if pr.state == "open" && !pr.head_sha.is_empty() {
                pr.checks = checks(state, &pr.slug, &pr.head_sha).await.unwrap_or(pr.checks);
            }
        }
        // Paused for the rate limit: nothing went out, so the card keeps what it had.
        Err(e) if github::is_paused(&e) => return Ok(pr),
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
            for pr in prs.into_iter().filter(|p| p.state == "open" || p.state.is_empty()) {
                let _ = refresh(&state, &pr.id).await;
            }
        }
    })
    .await
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
