//! Pull requests on GitHub that wait on the user, beyond the ones agents opened here: the user's own
//! (agents push with the user's token, so theirs count too) and ones where the user's review is
//! requested. One GraphQL search every few minutes, kept only for repos Loft knows: the projects
//! folder, the agents' workspaces, and repos agents opened PRs in. Only open, non-draft PRs whose checks
//! are green (or that have none) are kept; the "needs you" list reads the result.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::state::AppState;

const POLL_EVERY: Duration = Duration::from_secs(5 * 60);

/// A pull request found on GitHub.
#[derive(Debug, Clone, PartialEq)]
pub struct GhPr {
    /// owner/name
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub head_sha: String,
    pub created_at: String,
    /// SUCCESS, or "" when the PR has no checks.
    pub checks: String,
    /// The user's review is requested (someone else's PR).
    pub review_requested: bool,
}

const QUERY: &str = r#"query($mine: String!, $review: String!) {
  mine: search(query: $mine, type: ISSUE, first: 50) { nodes { ...P } }
  review: search(query: $review, type: ISSUE, first: 50) { nodes { ...P } }
}
fragment P on PullRequest {
  number title url isDraft createdAt mergeable headRefOid
  author { login }
  repository { nameWithOwner isArchived }
  commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
}"#;

/// Ready PRs from a GraphQL response, for the repos in `allowed` (lowercase owner/name).
pub fn parse(resp: &Value, allowed: &HashSet<String>) -> Vec<GhPr> {
    let mut out: Vec<GhPr> = vec![];
    for (key, review) in [("mine", false), ("review", true)] {
        for n in resp["data"][key]["nodes"].as_array().into_iter().flatten() {
            let repo = n["repository"]["nameWithOwner"].as_str().unwrap_or("").to_string();
            let checks = n["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["state"].as_str().unwrap_or("").to_string();
            let ready = !n["isDraft"].as_bool().unwrap_or(true)
                && !n["repository"]["isArchived"].as_bool().unwrap_or(false)
                && (checks.is_empty() || checks == "SUCCESS")
                // Your own PRs must be mergeable to be worth a look; a review can wait on a conflict.
                && (review || n["mergeable"].as_str() == Some("MERGEABLE"));
            if !ready || !allowed.contains(&repo.to_lowercase()) {
                continue;
            }
            let number = n["number"].as_i64().unwrap_or(0);
            if out.iter().any(|p| p.repo == repo && p.number == number) {
                continue;
            }
            out.push(GhPr {
                repo,
                number,
                title: n["title"].as_str().unwrap_or("").into(),
                url: n["url"].as_str().unwrap_or("").into(),
                author: n["author"]["login"].as_str().unwrap_or("").into(),
                head_sha: n["headRefOid"].as_str().unwrap_or("").into(),
                created_at: n["createdAt"].as_str().unwrap_or("").into(),
                checks,
                review_requested: review,
            });
        }
    }
    out
}

/// GitHub repos of git checkouts under `root` (a few levels deep), from their `origin` remote.
fn workspace_repos(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for p in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
            let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if hidden || !p.is_dir() || p.file_name().is_some_and(|n| n == "node_modules" || n == "target") {
                continue;
            }
            if let Ok(cfg) = std::fs::read_to_string(p.join(".git").join("config")) {
                let origin = cfg.split("[remote \"origin\"]").nth(1).unwrap_or("");
                if let Some(url) = origin.lines().find_map(|l| l.trim().strip_prefix("url = ")) {
                    out.extend(crate::repos::github_slug(url.trim()));
                }
            } else if depth < 3 {
                walk(&p, depth + 1, out);
            }
        }
    }
    let mut out = vec![];
    walk(root, 1, &mut out);
    out
}

/// Repos whose PRs count: the projects folder, the agents' workspaces, and repos agents opened PRs in.
async fn allowed(state: &AppState) -> HashSet<String> {
    let mut s: HashSet<String> = crate::repos::list(state, false).await.into_iter().filter_map(|r| r.github).collect();
    s.extend(workspace_repos(&state.cfg.storage_dir));
    s.extend(state.db.list_pull_requests(None).unwrap_or_default().into_iter().map(|p| p.slug));
    s.into_iter().map(|r| r.to_lowercase()).collect()
}

async fn refresh(state: &AppState) -> anyhow::Result<()> {
    let allowed = allowed(state).await;
    if allowed.is_empty() || crate::github::token(state).is_none() {
        state.rt.lock().unwrap().github_inbox.clear();
        return Ok(());
    }
    let vars = json!({
        "mine": "is:pr is:open draft:false archived:false author:@me",
        "review": "is:pr is:open draft:false archived:false review-requested:@me",
    });
    let resp = crate::github::request(state, reqwest::Method::POST, "/graphql", Some(json!({ "query": QUERY, "variables": vars }))).await?;
    if let Some(e) = resp["errors"][0]["message"].as_str() {
        anyhow::bail!("GitHub search: {e}");
    }
    state.rt.lock().unwrap().github_inbox = parse(&resp, &allowed);
    Ok(())
}

/// Refreshes the list every few minutes. One request per poll, so it stays far from rate limits.
pub async fn poller(state: AppState) {
    let mut tick = tokio::time::interval(POLL_EVERY);
    loop {
        tick.tick().await;
        if let Err(e) = refresh(&state).await {
            tracing::warn!("github inbox: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(repo: &str, n: i64, draft: bool, checks: Option<&str>, mergeable: &str) -> Value {
        json!({
            "number": n, "title": format!("PR {n}"), "url": format!("https://github.com/{repo}/pull/{n}"), "isDraft": draft,
            "createdAt": "2026-10-24T09:00:00Z", "mergeable": mergeable, "headRefOid": format!("sha{n}"),
            "author": { "login": "someone" }, "repository": { "nameWithOwner": repo, "isArchived": false },
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": checks.map(|c| json!({ "state": c })) } }] },
        })
    }

    #[test]
    fn keeps_only_ready_prs_in_known_repos() {
        let resp = json!({ "data": {
            "mine": { "nodes": [
                node("acme/shop", 1, false, Some("SUCCESS"), "MERGEABLE"),
                node("acme/shop", 2, true, Some("SUCCESS"), "MERGEABLE"),
                node("acme/shop", 3, false, Some("FAILURE"), "MERGEABLE"),
                node("acme/shop", 4, false, Some("PENDING"), "MERGEABLE"),
                node("acme/shop", 5, false, None, "MERGEABLE"),
                node("acme/shop", 6, false, Some("SUCCESS"), "CONFLICTING"),
                node("someone/elsewhere", 7, false, Some("SUCCESS"), "MERGEABLE"),
            ]},
            "review": { "nodes": [
                node("Octo/loft", 8, false, Some("SUCCESS"), "CONFLICTING"),
                node("acme/shop", 1, false, Some("SUCCESS"), "MERGEABLE"),
            ]},
        }});
        let allowed: HashSet<String> = ["acme/shop", "octo/loft"].iter().map(|s| s.to_string()).collect();
        let got = parse(&resp, &allowed);
        let ids: Vec<(String, i64, bool)> = got.iter().map(|p| (p.repo.clone(), p.number, p.review_requested)).collect();
        assert_eq!(ids, [("acme/shop".to_string(), 1, false), ("acme/shop".to_string(), 5, false), ("Octo/loft".to_string(), 8, true)]);
        assert_eq!((got[0].checks.as_str(), got[1].checks.as_str()), ("SUCCESS", ""));
    }

    #[test]
    fn finds_workspace_checkouts() {
        let root = std::env::temp_dir().join(format!("loft-ws-{}", uuid::Uuid::new_v4()));
        for (dir, url) in [("a/shop", "git@github.com:acme/shop.git"), ("b/deep/api", "https://github.com/acme/api"), ("c/node_modules/x", "https://github.com/x/y")] {
            std::fs::create_dir_all(root.join(dir).join(".git")).unwrap();
            std::fs::write(root.join(dir).join(".git/config"), format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {url}\n")).unwrap();
        }
        let mut got = workspace_repos(&root);
        got.sort();
        assert_eq!(got, ["acme/api", "acme/shop"]);
        std::fs::remove_dir_all(&root).ok();
    }
}
