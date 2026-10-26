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

/// The REST/GraphQL API root; tests point it at a local fake with LOFT_GITHUB_API_URL.
fn api_base() -> String {
    std::env::var("LOFT_GITHUB_API_URL").unwrap_or_else(|_| "https://api.github.com".into())
}

pub async fn request(state: &AppState, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
    let mut req = state
        .http
        .request(method, format!("{}{path}", api_base()))
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
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// A request the fake GitHub got: method, path with query, the headers we check, JSON body.
    #[derive(Clone, Debug)]
    pub(crate) struct Seen {
        pub method: String,
        pub path: String,
        pub headers: Value,
        pub body: Value,
    }

    #[derive(Default)]
    pub(crate) struct Fake {
        routes: Mutex<HashMap<(String, String), (u16, Value)>>,
        seen: Mutex<Vec<Seen>>,
    }

    impl Fake {
        /// Answer `method path` (path without query) with this status and body; unrouted requests get a 404.
        pub(crate) fn route(&self, method: &str, path: &str, status: u16, body: Value) {
            self.routes.lock().unwrap().insert((method.into(), path.into()), (status, body));
        }

        /// Requests received whose path starts with `prefix`, oldest first.
        pub(crate) fn seen(&self, prefix: &str) -> Vec<Seen> {
            self.seen.lock().unwrap().iter().filter(|s| s.path.starts_with(prefix)).cloned().collect()
        }
    }

    /// One fake GitHub for the whole test process, on its own thread so it outlives each test's
    /// runtime; LOFT_GITHUB_API_URL points at it. Tests keep apart by using their own repo slugs.
    pub(crate) fn fake_github() -> Arc<Fake> {
        use axum::http::{HeaderMap, Method, StatusCode, Uri};
        static FAKE: std::sync::OnceLock<Arc<Fake>> = std::sync::OnceLock::new();
        FAKE.get_or_init(|| {
            let fake: Arc<Fake> = Default::default();
            let f2 = fake.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    let app = axum::Router::new().fallback(move |method: Method, uri: Uri, headers: HeaderMap, body: axum::body::Bytes| {
                        let fake = f2.clone();
                        async move {
                            let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                            fake.seen.lock().unwrap().push(Seen {
                                method: method.to_string(),
                                path: uri.path_and_query().map(|p| p.to_string()).unwrap_or_default(),
                                headers: json!({
                                    "authorization": h("authorization"), "accept": h("accept"),
                                    "user-agent": h("user-agent"), "x-github-api-version": h("x-github-api-version"),
                                }),
                                body: serde_json::from_slice(&body).unwrap_or(Value::Null),
                            });
                            let hit = fake.routes.lock().unwrap().get(&(method.to_string(), uri.path().to_string())).cloned();
                            let (status, body) = hit.unwrap_or((404, json!({ "message": "Not Found" })));
                            (StatusCode::from_u16(status).unwrap(), axum::Json(body))
                        }
                    });
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    tx.send(format!("http://{}", listener.local_addr().unwrap())).unwrap();
                    axum::serve(listener, app).await.unwrap();
                });
            });
            let url = rx.recv().unwrap();
            // Set once, before any test reads it.
            unsafe { std::env::set_var("LOFT_GITHUB_API_URL", &url) };
            fake
        })
        .clone()
    }

    /// A test state with a GitHub token in settings, so `gh auth token` is never consulted.
    pub(crate) fn state_with_token() -> AppState {
        let st = crate::tasks::tests::state();
        let mut s = st.settings.get();
        s.projects_dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        s.github_token = "test-token".into();
        st.settings.save(s).unwrap();
        st
    }

    #[tokio::test]
    async fn lists_issues_through_the_fake() {
        let fake = fake_github();
        let st = state_with_token();
        fake.route("GET", "/repos/octo/issues-demo/issues", 200, json!([
            { "number": 2, "title": "Second", "state": "open" },
            { "number": 1, "title": "First", "state": "closed" },
        ]));
        let issues = get(&st, "/repos/octo/issues-demo/issues?state=all").await.unwrap();
        let titles: Vec<&str> = issues.as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Second", "First"]);

        let seen = fake.seen("/repos/octo/issues-demo/");
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("GET", "/repos/octo/issues-demo/issues?state=all"));
        assert_eq!(seen[0].headers, json!({
            "authorization": "Bearer test-token", "accept": "application/vnd.github+json",
            "user-agent": "loft", "x-github-api-version": "2022-11-28",
        }));

        let err = get(&st, "/repos/octo/issues-demo/nope").await.unwrap_err();
        assert_eq!(err.to_string(), "GitHub 404: Not Found");
    }

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
