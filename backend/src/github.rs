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
    // Tests never pick up the machine's own `gh` login.
    if cfg!(test) {
        return None;
    }
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

/// Keeps write requests (POST, PATCH, PUT, DELETE) at least a second apart, as GitHub asks: a burst of
/// them, like publishing 50 tickets, otherwise trips its secondary rate limit.
#[derive(Default)]
pub struct Pace(tokio::sync::Mutex<Option<Instant>>);

const WRITE_GAP: Duration = if cfg!(test) { Duration::ZERO } else { Duration::from_secs(1) };

impl Pace {
    async fn wait(&self, gap: Duration) {
        let mut last = self.0.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep_until((at + gap).into()).await;
        }
        *last = Some(Instant::now());
    }
}

/// The longest a rate-limited request waits for its one retry; a later reset fails the request instead.
const MAX_WAIT: Duration = Duration::from_secs(120);

/// How long GitHub asks us to wait when it turned a request down for its rate limits, if it did:
/// `Retry-After`, else until `x-ratelimit-reset` when nothing is left, else a minute for a secondary limit.
fn rate_limit_wait(status: reqwest::StatusCode, headers: &reqwest::header::HeaderMap, msg: &str) -> Option<Duration> {
    if !matches!(status.as_u16(), 403 | 429) {
        return None;
    }
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(s) = h("retry-after") {
        return Some(Duration::from_secs(s));
    }
    if h("x-ratelimit-remaining") == Some(0) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        return Some(Duration::from_secs(h("x-ratelimit-reset").unwrap_or(now + 60).saturating_sub(now) + 1));
    }
    (status.as_u16() == 429 || msg.to_lowercase().contains("rate limit")).then_some(Duration::from_secs(60))
}

pub async fn request(state: &AppState, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
    let mut retried = false;
    loop {
        if method != reqwest::Method::GET {
            state.gh_pace.wait(WRITE_GAP).await;
        }
        let mut req = state
            .http
            .request(method.clone(), format!("{}{path}", api_base()))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "loft")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = token(state) {
            req = req.bearer_auth(t);
        }
        if let Some(b) = &body {
            req = req.json(b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        let msg = body["message"].as_str().unwrap_or("request failed");
        // Rate limited: wait as long as GitHub asks and try once more.
        if let Some(wait) = rate_limit_wait(status, &headers, msg) {
            if !retried && wait <= MAX_WAIT {
                retried = true;
                tokio::time::sleep(wait).await;
                continue;
            }
            bail!("GitHub {}: {msg} (rate limited, try again in {} s)", status.as_u16(), wait.as_secs().max(1));
        }
        // 422s say what was wrong in `errors`, e.g. an assignee that can't be assigned.
        let detail: Vec<String> = body["errors"]
            .as_array()
            .map(|es| es.iter().filter_map(|e| e["message"].as_str().map(String::from).or_else(|| e["field"].as_str().map(|f| format!("{f} {}", e["code"].as_str().unwrap_or("invalid"))))).collect())
            .unwrap_or_default();
        if detail.is_empty() {
            bail!("GitHub {}: {msg}", status.as_u16());
        }
        bail!("GitHub {}: {msg} ({})", status.as_u16(), detail.join("; "));
    }
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
        routes: Mutex<HashMap<(String, String), std::collections::VecDeque<(u16, Value)>>>,
        headers: Mutex<HashMap<(String, String), Vec<(String, String)>>>,
        seen: Mutex<Vec<Seen>>,
    }

    impl Fake {
        /// Answer `method path` (path without query) with this status and body; unrouted requests get a 404.
        pub(crate) fn route(&self, method: &str, path: &str, status: u16, body: Value) {
            self.in_turn(method, path, vec![(status, body)]);
        }

        /// Answer `method path` with these replies one request at a time; the last one then repeats.
        pub(crate) fn in_turn(&self, method: &str, path: &str, replies: Vec<(u16, Value)>) {
            self.routes.lock().unwrap().insert((method.into(), path.into()), replies.into());
        }

        /// Send these response headers with every reply to `method path`.
        pub(crate) fn with_headers(&self, method: &str, path: &str, headers: &[(&str, &str)]) {
            let hs = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            self.headers.lock().unwrap().insert((method.into(), path.into()), hs);
        }

        /// Requests received whose path starts with `prefix`, oldest first.
        pub(crate) fn seen(&self, prefix: &str) -> Vec<Seen> {
            self.seen.lock().unwrap().iter().filter(|s| s.path.starts_with(prefix)).cloned().collect()
        }
    }

    /// Fakes of their own, by the token whose requests they answer.
    static OWN: Mutex<Option<HashMap<String, Arc<Fake>>>> = Mutex::new(None);

    /// One fake GitHub server for the whole test process, on its own thread so it outlives each test's
    /// runtime; LOFT_GITHUB_API_URL points at it. A request carrying a token from `own_fake` goes to that
    /// test's fake; the rest share this one and keep apart by using their own repo slugs.
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
                        let token = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("").to_string();
                        let own = OWN.lock().unwrap().as_ref().and_then(|m| m.get(&token).cloned());
                        let fake = own.unwrap_or_else(|| f2.clone());
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
                            let route = (method.to_string(), uri.path().to_string());
                            let hit = fake.routes.lock().unwrap().get_mut(&route).and_then(|q| if q.len() > 1 { q.pop_front() } else { q.front().cloned() });
                            let (status, body) = hit.unwrap_or((404, json!({ "message": "Not Found" })));
                            let mut hs = HeaderMap::new();
                            for (k, v) in fake.headers.lock().unwrap().get(&route).into_iter().flatten() {
                                hs.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
                            }
                            (StatusCode::from_u16(status).unwrap(), hs, axum::Json(body))
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

    /// A fake GitHub only this state talks to: its own replies and its own recorded requests. The state's
    /// token becomes one no other test uses, and the server routes by it.
    pub(crate) fn own_fake(st: &AppState) -> Arc<Fake> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        fake_github();
        let token = format!("own-token-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let mut s = st.settings.get();
        s.github_token = token.clone();
        st.settings.save(s).unwrap();
        let fake: Arc<Fake> = Default::default();
        OWN.lock().unwrap().get_or_insert_with(HashMap::new).insert(token, fake.clone());
        fake
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
        assert_eq!(seen[0].body, Value::Null, "a GET sends no body");

        let err = get(&st, "/repos/octo/issues-demo/nope").await.unwrap_err();
        assert_eq!(err.to_string(), "GitHub 404: Not Found");
    }

    #[tokio::test]
    async fn waits_out_a_rate_limit_once_then_gives_up() {
        let st = state_with_token();
        let fake = own_fake(&st);
        // Secondary limit: GitHub says when to come back, and the retry goes through.
        fake.in_turn("POST", "/repos/octo/limited/issues", vec![(403, json!({ "message": "You have exceeded a secondary rate limit." })), (201, json!({ "number": 1 }))]);
        fake.with_headers("POST", "/repos/octo/limited/issues", &[("retry-after", "0")]);
        let made = request(&st, reqwest::Method::POST, "/repos/octo/limited/issues", Some(json!({ "title": "A" }))).await.unwrap();
        assert_eq!(made["number"], 1);
        let seen = fake.seen("/repos/octo/limited/issues");
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].body, json!({ "title": "A" }), "the retry sends the same body");

        // Still limited after the one retry: the error says so.
        fake.route("GET", "/repos/octo/limited/labels", 429, json!({ "message": "Too Many Requests" }));
        fake.with_headers("GET", "/repos/octo/limited/labels", &[("retry-after", "0")]);
        let err = get(&st, "/repos/octo/limited/labels").await.unwrap_err();
        assert_eq!(err.to_string(), "GitHub 429: Too Many Requests (rate limited, try again in 1 s)");
        assert_eq!(fake.seen("/repos/octo/limited/labels").len(), 2);

        // The primary limit resets too far off to wait for: no retry.
        let reset = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 1800;
        fake.route("GET", "/repos/octo/limited/pulls", 403, json!({ "message": "API rate limit exceeded" }));
        fake.with_headers("GET", "/repos/octo/limited/pulls", &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", &reset.to_string())]);
        let err = get(&st, "/repos/octo/limited/pulls").await.unwrap_err().to_string();
        assert!(err.starts_with("GitHub 403: API rate limit exceeded (rate limited, try again in 18"), "{err}");
        assert_eq!(fake.seen("/repos/octo/limited/pulls").len(), 1);

        // A 403 that isn't a rate limit is not retried.
        fake.route("GET", "/repos/octo/limited/hooks", 403, json!({ "message": "Must have admin rights to Repository." }));
        assert_eq!(get(&st, "/repos/octo/limited/hooks").await.unwrap_err().to_string(), "GitHub 403: Must have admin rights to Repository.");
        assert_eq!(fake.seen("/repos/octo/limited/hooks").len(), 1);
    }

    #[tokio::test]
    async fn paces_writes_apart() {
        let pace = Pace::default();
        let start = Instant::now();
        for _ in 0..3 {
            pace.wait(Duration::from_millis(50)).await;
        }
        assert!(start.elapsed() >= Duration::from_millis(100), "{:?}", start.elapsed());
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
