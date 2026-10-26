//! Thin GitHub REST client shared by the repo dashboard and agent PR cards.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde_json::Value;

use crate::state::AppState;

/// Token from settings, else `gh auth token`, re-checked periodically so
/// logging in to `gh` after startup still works.
#[derive(Default)]
pub struct TokenCache(Mutex<Option<(Instant, Option<String>)>>);

const TOKEN_TTL: Duration = Duration::from_secs(300);

fn gh_auth_token() -> Option<String> {
    let out = std::process::Command::new("gh").args(["auth", "token"]).output().ok()?;
    let t = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !t.is_empty()).then_some(t)
}

pub fn token(state: &AppState) -> Option<String> {
    let t = state.settings.get().github_token;
    if !t.trim().is_empty() {
        return Some(t.trim().to_string());
    }
    let mut cache = state.gh_token.0.lock().unwrap();
    if let Some((at, t)) = &*cache {
        if at.elapsed() < TOKEN_TTL {
            return t.clone();
        }
    }
    let t = gh_auth_token();
    *cache = Some((Instant::now(), t.clone()));
    t
}

pub async fn request(state: &AppState, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
    let mut req = state
        .http
        .request(method, format!("https://api.github.com{path}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "loft")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(t) = token(state) {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body["message"].as_str().unwrap_or("request failed");
        bail!("GitHub {}: {msg}", status.as_u16());
    }
    Ok(body)
}

pub async fn get(state: &AppState, path: &str) -> Result<Value> {
    request(state, reqwest::Method::GET, path, None).await
}

/// Every `owner/repo#number` PR link in a blob of text, in order, deduplicated.
pub fn extract_pr_links(text: &str) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = vec![];
    let mut rest = text;
    while let Some(i) = rest.find("github.com/") {
        rest = &rest[i + "github.com/".len()..];
        let seg: Vec<&str> = rest
            .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
            .next()
            .unwrap_or("")
            .split('/')
            .collect();
        if seg.len() >= 4 && seg[2] == "pull" && !seg[0].is_empty() && !seg[1].is_empty() {
            if let Ok(n) = seg[3].parse::<i64>() {
                let item = (format!("{}/{}", seg[0], seg[1].trim_end_matches(".git")), n);
                if !out.contains(&item) {
                    out.push(item);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::extract_pr_links;

    #[test]
    fn finds_pr_links() {
        let t = "remote: \nhttps://github.com/octo/demo/pull/5\nsee also https://github.com/a/b.c/pull/12/files and github.com/x/y/issues/3";
        assert_eq!(
            extract_pr_links(t),
            vec![("octo/demo".to_string(), 5), ("a/b.c".to_string(), 12)]
        );
        assert!(extract_pr_links(r#"{"html_url":"https://github.com/o/r/pull/7"}"#) == vec![("o/r".to_string(), 7)]);
    }
}
