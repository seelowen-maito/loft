//! Thin GitHub REST client shared by the repo dashboard and agent PR cards.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::state::AppState;

tokio::task_local! {
    /// What a request is for: `prs`, `mirror`, `repos`, `inbox`, `user` or `agent`, for the per-source counts.
    static SOURCE: &'static str;
}

/// Runs `fut` with its GitHub requests counted as `source`.
pub async fn as_source<F: std::future::Future>(source: &'static str, fut: F) -> F::Output {
    SOURCE.scope(source, fut).await
}

fn source() -> &'static str {
    SOURCE.try_with(|s| *s).unwrap_or("other")
}

/// What GitHub's rate limit headers last said, per resource (`core`, `graphql`), and the requests Loft
/// made by source in the current core window. One token is shared with the agents' `gh`, so `used` can
/// be well above Loft's own count.
#[derive(Default)]
pub struct Budget(Mutex<BudgetState>);

#[derive(Default)]
struct BudgetState {
    resources: std::collections::BTreeMap<String, Value>,
    /// The core window the counts belong to: its reset time.
    window: i64,
    /// source -> (requests that counted, 304s that didn't)
    by_source: std::collections::BTreeMap<&'static str, (u64, u64)>,
}

impl Budget {
    fn record(&self, headers: &reqwest::header::HeaderMap, not_modified: bool) {
        let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<i64>().ok());
        let mut b = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let resource = headers.get("x-ratelimit-resource").and_then(|v| v.to_str().ok()).unwrap_or("core").to_string();
        if let (Some(limit), Some(remaining), Some(reset)) = (h("x-ratelimit-limit"), h("x-ratelimit-remaining"), h("x-ratelimit-reset")) {
            if resource == "core" && reset > b.window {
                // A new hour: the counts start again with it.
                b.window = reset;
                b.by_source.clear();
            }
            let used = h("x-ratelimit-used").unwrap_or(limit - remaining);
            b.resources.insert(resource, json!({ "limit": limit, "remaining": remaining, "used": used, "reset": reset }));
        }
        let c = b.by_source.entry(source()).or_default();
        if not_modified { c.1 += 1 } else { c.0 += 1 }
    }

    /// Whether a request for `resource` from `source` may go out. At 0 left nothing goes until the reset;
    /// below a tenth only what a person (or an agent's tool) asked for. What's known after the reset
    /// time is stale, so then everything goes again.
    fn allow(&self, resource: &str, source: &str) -> Result<(), String> {
        let b = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(r) = b.resources.get(resource) else { return Ok(()) };
        let (limit, remaining, reset) = (r["limit"].as_i64().unwrap_or(0), r["remaining"].as_i64().unwrap_or(0), r["reset"].as_i64().unwrap_or(0));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        if reset <= now || limit <= 0 {
            return Ok(());
        }
        let wait = reset - now;
        if remaining <= 0 {
            return Err(format!("{PAUSED}: none left, back in {wait} s"));
        }
        if remaining * 10 < limit && !matches!(source, "user" | "agent" | "repos") {
            return Err(format!("{PAUSED}: {remaining} of {limit} left, kept for your own clicks until the reset in {wait} s"));
        }
        Ok(())
    }

    /// For the API: the resources and the counts by source.
    pub fn snapshot(&self) -> Value {
        let b = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let by_source: serde_json::Map<String, Value> = b.by_source.iter().map(|(s, (n, free))| (s.to_string(), json!({ "requests": n, "not_modified": free }))).collect();
        json!({ "resources": b.resources, "by_source": by_source, "window_reset": (b.window > 0).then_some(b.window) })
    }
}

/// What a request turned away by the budget says, so callers can tell it from a GitHub error.
const PAUSED: &str = "GitHub paused";

/// The request didn't go out: the budget is (nearly) used up. Pollers skip their tick on it.
pub fn is_paused(e: &anyhow::Error) -> bool {
    e.to_string().starts_with(PAUSED)
}

/// Background work: it waits for the next tick instead of retrying when GitHub is busy.
fn in_background() -> bool {
    !matches!(source(), "user" | "agent" | "repos")
}

/// `GET /api/github/budget`: how much of GitHub's rate limit is left and where Loft's requests went.
pub async fn budget(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<Value> {
    axum::Json(state.gh_budget.snapshot())
}

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
/// them, like publishing 50 tickets, otherwise trips its secondary rate limit. GraphQL queries are
/// POSTs too but only read, so they aren't paced.
pub struct Pace {
    last: tokio::sync::Mutex<Option<Instant>>,
    gap: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self::with_gap(if cfg!(test) { Duration::ZERO } else { Duration::from_secs(1) })
    }
}

impl Pace {
    pub(crate) fn with_gap(gap: Duration) -> Self {
        Pace { last: Default::default(), gap }
    }

    async fn wait(&self) {
        let mut last = self.last.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep_until((at + self.gap).into()).await;
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
    Ok(exchange(state, method, path, body, None).await?.map_or(Value::Null, |(v, _)| v))
}

/// A GET with `If-None-Match`: Ok(None) when GitHub answers 304 Not Modified, which doesn't count
/// against the rate limit; otherwise the body and the response's ETag, for the next poll.
pub async fn get_if_changed(state: &AppState, path: &str, etag: Option<&str>) -> Result<Option<(Value, Option<String>)>> {
    exchange(state, reqwest::Method::GET, path, None, etag).await
}

/// GitHub said the thing isn't there (404) or is gone for good (410).
pub fn is_gone(e: &anyhow::Error) -> bool {
    let m = e.to_string();
    m.starts_with("GitHub 404") || m.starts_with("GitHub 410")
}

async fn exchange(state: &AppState, method: reqwest::Method, path: &str, body: Option<Value>, etag: Option<&str>) -> Result<Option<(Value, Option<String>)>> {
    let resource = if path == "/graphql" { "graphql" } else { "core" };
    if let Err(why) = state.gh_budget.allow(resource, source()) {
        bail!(why);
    }
    // A poller doesn't sleep on a rate limit: it tries again on its next tick.
    let mut retried = in_background();
    loop {
        if method != reqwest::Method::GET && path != "/graphql" {
            state.gh_pace.wait().await;
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
        if let Some(e) = etag {
            req = req.header("If-None-Match", e);
        }
        let resp = req.send().await?;
        let status = resp.status();
        state.gh_budget.record(resp.headers(), status == reqwest::StatusCode::NOT_MODIFIED);
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        let headers = resp.headers().clone();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            let tag = headers.get("etag").and_then(|v| v.to_str().ok()).map(String::from);
            return Ok(Some((body, tag)));
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
                                    "if-none-match": h("if-none-match"),
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
            "user-agent": "loft", "x-github-api-version": "2022-11-28", "if-none-match": "",
        }));
        assert_eq!(seen[0].body, Value::Null, "a GET sends no body");

        let err = get(&st, "/repos/octo/issues-demo/nope").await.unwrap_err();
        assert_eq!(err.to_string(), "GitHub 404: Not Found");
    }

    #[tokio::test]
    async fn waits_out_a_rate_limit_once_then_gives_up() {
        // Something a person asked for: it waits and retries once (background work doesn't, see below).
        as_source("user", waits_out_a_rate_limit_once()).await;
    }

    async fn waits_out_a_rate_limit_once() {
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
    async fn paces_writes_but_not_reads() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let gap = Duration::from_millis(300);
        let st = AppState { gh_pace: Arc::new(Pace::with_gap(gap)), ..st };
        fake.route("POST", "/repos/octo/paced/issues", 201, json!({ "number": 1 }));
        fake.route("GET", "/repos/octo/paced/issues", 200, json!([]));
        fake.route("POST", "/graphql", 200, json!({ "data": {} }));

        let start = Instant::now();
        for _ in 0..3 {
            request(&st, reqwest::Method::POST, "/repos/octo/paced/issues", Some(json!({ "title": "A" }))).await.unwrap();
        }
        assert!(start.elapsed() >= 2 * gap, "three writes take two gaps: {:?}", start.elapsed());

        // Right after a write, reads and GraphQL queries go straight through.
        let start = Instant::now();
        for _ in 0..3 {
            get(&st, "/repos/octo/paced/issues").await.unwrap();
            request(&st, reqwest::Method::POST, "/graphql", Some(json!({ "query": "{}" }))).await.unwrap();
        }
        assert!(start.elapsed() < gap, "reads aren't paced: {:?}", start.elapsed());
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

    #[tokio::test]
    async fn the_budget_shows_what_github_says_is_left_and_who_used_it() {
        let st = state_with_token();
        let fake = own_fake(&st);
        let core = |remaining: &'static str, reset: &'static str| vec![("x-ratelimit-limit", "5000"), ("x-ratelimit-remaining", remaining), ("x-ratelimit-used", "800"), ("x-ratelimit-reset", reset), ("x-ratelimit-resource", "core")];
        fake.route("GET", "/repos/octo/budget/pulls/1", 200, json!({ "number": 1 }));
        fake.with_headers("GET", "/repos/octo/budget/pulls/1", &core("4200", "1900000000"));
        fake.route("GET", "/repos/octo/budget/issues", 304, Value::Null);
        fake.with_headers("GET", "/repos/octo/budget/issues", &core("4200", "1900000000"));
        fake.route("POST", "/graphql", 200, json!({ "data": {} }));
        fake.with_headers("POST", "/graphql", &[("x-ratelimit-limit", "5000"), ("x-ratelimit-remaining", "4990"), ("x-ratelimit-reset", "1900000100"), ("x-ratelimit-resource", "graphql")]);

        as_source("prs", async { get(&st, "/repos/octo/budget/pulls/1").await.unwrap() }).await;
        as_source("prs", async { get(&st, "/repos/octo/budget/pulls/1").await.unwrap() }).await;
        as_source("mirror", async { assert!(get_if_changed(&st, "/repos/octo/budget/issues", Some("\"e\"")).await.unwrap().is_none()) }).await;
        as_source("inbox", async { request(&st, reqwest::Method::POST, "/graphql", Some(json!({}))).await.unwrap() }).await;
        get(&st, "/repos/octo/budget/pulls/1").await.unwrap();

        let axum::Json(v) = budget(axum::extract::State(st.clone())).await;
        assert_eq!(v["resources"]["core"], json!({ "limit": 5000, "remaining": 4200, "used": 800, "reset": 1900000000i64 }));
        assert_eq!(v["resources"]["graphql"], json!({ "limit": 5000, "remaining": 4990, "used": 10, "reset": 1900000100i64 }));
        assert_eq!(v["by_source"], json!({
            "prs": { "requests": 2, "not_modified": 0 },
            "mirror": { "requests": 0, "not_modified": 1 },
            "inbox": { "requests": 1, "not_modified": 0 },
            "other": { "requests": 1, "not_modified": 0 },
        }));

        // The next hour starts the counts again.
        fake.with_headers("GET", "/repos/octo/budget/pulls/1", &core("4999", "1900003600"));
        as_source("user", async { get(&st, "/repos/octo/budget/pulls/1").await.unwrap() }).await;
        let axum::Json(v) = budget(axum::extract::State(st)).await;
        assert_eq!(v["by_source"], json!({ "user": { "requests": 1, "not_modified": 0 } }));
        assert_eq!((&v["resources"]["core"]["remaining"], &v["window_reset"]), (&json!(4999), &json!(1900003600i64)));
    }


    /// The budget as if GitHub's last answer said: this many of 5000 left, resetting in `secs`.
    fn left(st: &AppState, remaining: i64, secs: i64) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let mut h = reqwest::header::HeaderMap::new();
        for (k, v) in [("x-ratelimit-limit", 5000), ("x-ratelimit-remaining", remaining), ("x-ratelimit-reset", now + secs)] {
            h.insert(k, v.to_string().parse().unwrap());
        }
        st.gh_budget.record(&h, false);
    }

    #[tokio::test]
    async fn below_a_tenth_only_people_reach_github_and_at_zero_nobody_until_the_reset() {
        let st = state_with_token();
        let fake = own_fake(&st);
        fake.route("GET", "/repos/octo/pause/pulls/1", 200, json!({ "number": 1 }));
        let calls = || fake.seen("/repos/octo/pause/pulls/1").len();

        // 400 of 5000 left: the pollers wait, a person's click and an agent's tool still go out.
        left(&st, 400, 1800);
        for poller in ["prs", "mirror", "inbox", "other"] {
            let e = as_source(poller, async { get(&st, "/repos/octo/pause/pulls/1").await.unwrap_err() }).await;
            assert!(is_paused(&e) && e.to_string().contains("400 of 5000 left"), "{poller}: {e}");
        }
        assert_eq!(calls(), 0, "nothing went out");
        for person in ["user", "agent", "repos"] {
            as_source(person, async { get(&st, "/repos/octo/pause/pulls/1").await.unwrap() }).await;
        }
        assert_eq!(calls(), 3);

        // None left: nobody, until the reset.
        left(&st, 0, 1800);
        let e = as_source("user", async { get(&st, "/repos/octo/pause/pulls/1").await.unwrap_err() }).await;
        assert!(is_paused(&e) && e.to_string().contains("none left"), "{e}");
        assert_eq!(calls(), 3);

        // Once the reset time has passed, what Loft knew is stale and everything goes again.
        left(&st, 0, -1);
        as_source("prs", async { get(&st, "/repos/octo/pause/pulls/1").await.unwrap() }).await;
        assert_eq!(calls(), 4);
    }

    #[tokio::test]
    async fn a_poller_skips_its_tick_on_a_rate_limit_instead_of_waiting() {
        let st = state_with_token();
        let fake = own_fake(&st);
        fake.in_turn("GET", "/repos/octo/skip/pulls/1", vec![(429, json!({ "message": "Too Many Requests" })), (200, json!({ "number": 1 }))]);
        fake.with_headers("GET", "/repos/octo/skip/pulls/1", &[("retry-after", "30")]);
        let started = std::time::Instant::now();
        let e = as_source("prs", async { get(&st, "/repos/octo/skip/pulls/1").await.unwrap_err() }).await;
        assert!(e.to_string().contains("rate limited"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(5), "no 30 s sleep");
        assert_eq!(fake.seen("/repos/octo/skip/pulls/1").len(), 1, "no retry: the next tick tries again");
    }

}
