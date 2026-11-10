//! Paired clients (ADR 0004): only a browser or phone the user paired with Loft may change things through the API.
//!
//! A new client calls `POST /api/pair` and gets a short display code plus a secret. It polls with the secret
//! (never the code) and receives its token once, after a paired client, or at the Mac the very first one, allows
//! that code. Pending pairings live in memory only. A token is stored as a SHA-256 hash, with a name and when
//! it was last used. Nothing here trusts a request for coming from the Mac itself: Tailscale makes every request
//! look local.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::api::{ApiError, bad, not_found};
use crate::state::AppState;

/// The web's token cookie.
pub const COOKIE: &str = "__Host-loft";
/// A pairing nobody allows within this long is dropped.
const EXPIRES: Duration = Duration::from_secs(120);
const MAX_PENDING: usize = 10;
/// `POST /api/pair` is open to anyone, so it takes only this many requests a minute.
const MAX_PER_MINUTE: usize = 5;

enum Answer {
    Waiting,
    Allowed(String),
    Denied,
}

struct Pending {
    code: String,
    name: String,
    at: Instant,
    /// When it asked, as Needs you shows it.
    since: String,
    answer: Answer,
}

#[derive(Default)]
struct Inner {
    /// secret -> the pairing waiting on it
    pending: HashMap<String, Pending>,
    /// When each recent `POST /api/pair` came, for the rate limit.
    recent: Vec<Instant>,
}

#[derive(Default)]
pub struct Pairing {
    /// Whether the very first client is offered a Mac dialog and a terminal code (off in tests).
    prompts: bool,
    inner: Mutex<Inner>,
    /// One Mac dialog at a time, so a flood of pairings can't bury the user's own.
    dialog: tokio::sync::Mutex<()>,
}

enum Polled {
    Waiting,
    Allowed(String, String),
    Denied,
    Gone,
}

impl Pairing {
    pub fn with_prompts() -> Self {
        Self { prompts: true, ..Default::default() }
    }

    fn start(&self, name: String) -> Result<(String, String), ApiError> {
        let mut i = self.inner.lock().unwrap();
        i.pending.retain(|_, p| p.at.elapsed() < EXPIRES);
        i.recent.retain(|t| t.elapsed() < Duration::from_secs(60));
        if i.recent.len() >= MAX_PER_MINUTE || i.pending.len() >= MAX_PENDING {
            return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "too many pairing requests: try again in a minute".into()));
        }
        i.recent.push(Instant::now());
        let code = loop {
            let c = format!("{:04}", u32::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..4].try_into().unwrap()) % 10_000);
            if !i.pending.values().any(|p| p.code == c) {
                break c;
            }
        };
        let secret = random_hex();
        i.pending.insert(secret.clone(), Pending { code: code.clone(), name, at: Instant::now(), since: crate::db::now(), answer: Answer::Waiting });
        Ok((secret, code))
    }

    /// What the client polling with this secret gets. A denial is dropped once told. An allowed pairing stays
    /// until `collected`, so a failed insert of its token doesn't lose it.
    fn poll(&self, secret: &str) -> Polled {
        let mut i = self.inner.lock().unwrap();
        i.pending.retain(|_, p| p.at.elapsed() < EXPIRES);
        match i.pending.get(secret) {
            None => Polled::Gone,
            Some(Pending { answer: Answer::Waiting, .. }) => Polled::Waiting,
            Some(Pending { answer: Answer::Allowed(token), name, .. }) => Polled::Allowed(token.clone(), name.clone()),
            Some(_) => {
                i.pending.remove(secret);
                Polled::Denied
            }
        }
    }

    /// Answers the waiting pairing that shows this code. False when none does.
    fn decide(&self, code: &str, allow: bool) -> bool {
        let mut i = self.inner.lock().unwrap();
        i.pending.retain(|_, p| p.at.elapsed() < EXPIRES);
        match i.pending.values_mut().find(|p| p.code == code && matches!(p.answer, Answer::Waiting)) {
            Some(p) => {
                p.answer = if allow { Answer::Allowed(random_hex()) } else { Answer::Denied };
                true
            }
            None => false,
        }
    }

    /// Pairings waiting for an answer: (code, name, when it asked). Needs you lists these for paired devices.
    pub fn waiting_list(&self) -> Vec<(String, String, String)> {
        let i = self.inner.lock().unwrap();
        let mut v: Vec<_> = i.pending.values().filter(|p| matches!(p.answer, Answer::Waiting) && p.at.elapsed() < EXPIRES).map(|p| (p.code.clone(), p.name.clone(), p.since.clone())).collect();
        v.sort_by(|a, b| a.2.cmp(&b.2));
        v
    }

    fn waiting(&self, code: &str) -> bool {
        let i = self.inner.lock().unwrap();
        i.pending.values().any(|p| p.code == code && matches!(p.answer, Answer::Waiting) && p.at.elapsed() < EXPIRES)
    }
}

/// 32 random bytes as hex (two v4 UUIDs, which come from the OS's random source).
fn random_hex() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn count(state: &AppState) -> i64 {
    state.db.paired_client_count()
}

fn token_of(headers: &HeaderMap) -> Option<String> {
    if let Some(t) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) {
        return Some(t.trim().to_string());
    }
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('=')).map(str::to_string)
}

/// Whether the request carries a paired client's token. Counts as that client's use.
pub fn paired(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = token_of(headers) else { return false };
    state.db.use_paired_client(&hash(&token))
}

/// What needs a paired client: every write under the API, plus reading devices, push, settings and Clean's listings and log. Only the
/// calls a client with no token must make to get one stay open.
fn needs_token(method: &Method, path: &str) -> bool {
    let path = path.trim_end_matches('/');
    if *method == Method::POST && matches!(path, "/pair" | "/pair/poll" | "/pair/cancel") {
        return false;
    }
    match *method {
        Method::GET | Method::HEAD | Method::OPTIONS => {
            matches!(path, "/devices" | "/push" | "/settings" | "/paired-clients" | "/workspaces/clean/preview" | "/workspaces/clean/folders" | "/workspaces/log")
        }
        _ => true,
    }
}

pub async fn require_paired(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if !needs_token(req.method(), req.uri().path()) || paired(&state, req.headers()) {
        next.run(req).await
    } else {
        ApiError(StatusCode::UNAUTHORIZED, "pair this client with Loft first".into()).into_response()
    }
}

#[derive(Deserialize, Default)]
pub struct StartBody {
    name: Option<String>,
}

/// `POST /api/pair`: a new client asks to be paired. It shows the code and polls with the secret.
pub async fn start(State(state): State<AppState>, body: Option<Json<StartBody>>) -> Result<Json<Value>, ApiError> {
    let name: String = body.and_then(|b| b.0.name).unwrap_or_default().chars().filter(|c| !c.is_control()).take(60).collect();
    let name = if name.trim().is_empty() { "Unnamed client".to_string() } else { name.trim().to_string() };
    let (secret, code) = state.pairing.start(name.clone())?;
    // The very first client has no paired device to approve it: the Mac asks instead.
    if state.pairing.prompts && count(&state) == 0 {
        ask_at_the_mac(state.clone(), code.clone(), name);
    }
    Ok(Json(json!({ "secret": secret, "code": code, "expires_in": EXPIRES.as_secs() })))
}

#[derive(Deserialize)]
pub struct SecretBody {
    secret: String,
    /// The web asks for its token as a cookie, so page scripts never see it.
    #[serde(default)]
    cookie: bool,
}

/// `POST /api/pair/poll`: `pending`, `denied`, `gone` (expired, cancelled or already collected), or `allowed`
/// with the token, once.
pub async fn poll(State(state): State<AppState>, Json(b): Json<SecretBody>) -> Result<Response, ApiError> {
    Ok(match state.pairing.poll(&b.secret) {
        Polled::Waiting => Json(json!({ "status": "pending" })).into_response(),
        Polled::Denied => Json(json!({ "status": "denied" })).into_response(),
        Polled::Gone => Json(json!({ "status": "gone" })).into_response(),
        Polled::Allowed(token, name) => {
            // Store first, then drop the pairing: if the insert fails the client can poll again.
            state.db.add_paired_client(&hash(&token), &name)?;
            state.pairing.inner.lock().unwrap().pending.remove(&b.secret);
            if b.cookie {
                // Secure: Safari drops it on plain http://localhost, so web pairing there needs https (ADR 0004).
                let cookie = format!("{COOKIE}={token}; Secure; HttpOnly; Path=/; SameSite=Strict; Max-Age=34560000");
                ([(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap())], Json(json!({ "status": "allowed" }))).into_response()
            } else {
                Json(json!({ "status": "allowed", "token": token })).into_response()
            }
        }
    })
}

/// `POST /api/pair/cancel`: the client gives up its own pending pairing.
pub async fn cancel(State(state): State<AppState>, Json(b): Json<SecretBody>) -> Json<Value> {
    state.pairing.inner.lock().unwrap().pending.remove(&b.secret);
    Json(json!({ "ok": true }))
}

#[derive(Deserialize)]
pub struct CodeBody {
    code: String,
}

/// `POST /api/pair/approve`: a paired client allows the pairing that shows this code.
pub async fn approve(State(state): State<AppState>, Json(b): Json<CodeBody>) -> Result<Json<Value>, ApiError> {
    answer(&state, &b.code, true)
}

/// `POST /api/pair/deny`
pub async fn deny(State(state): State<AppState>, Json(b): Json<CodeBody>) -> Result<Json<Value>, ApiError> {
    answer(&state, &b.code, false)
}

fn answer(state: &AppState, code: &str, allow: bool) -> Result<Json<Value>, ApiError> {
    if code.len() != 4 {
        return Err(bad("the code has 4 digits"));
    }
    if state.pairing.decide(code, allow) { Ok(Json(json!({ "ok": true }))) } else { Err(not_found()) }
}

/// A paired client's id in the list: the start of its token's hash, so the hash itself never leaves.
fn client_id(hash: &str) -> &str {
    &hash[..16]
}

/// `GET /api/paired-clients`: every paired client, with `this` on the one asking.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let me = token_of(&headers).map(|t| hash(&t)).unwrap_or_default();
    let clients: Vec<Value> = state
        .db
        .paired_clients()?
        .into_iter()
        .map(|(h, name, paired_at, last_used_at)| json!({ "id": client_id(&h), "name": name, "paired_at": paired_at, "last_used_at": last_used_at, "this": h == me }))
        .collect();
    Ok(Json(json!(clients)))
}

/// `DELETE /api/paired-clients/{id}`: Forget this device. That client is refused from its next request on.
pub async fn forget(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    if state.db.forget_paired_client(&id)? { Ok(Json(json!({ "ok": true }))) } else { Err(not_found()) }
}

/// `POST /api/paired-clients/unpair-all`: unpairs every client but the one asking.
pub async fn unpair_all(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    // The middleware let this through, so the request carries a paired token.
    let me = token_of(&headers).map(|t| hash(&t)).ok_or_else(not_found)?;
    Ok(Json(json!({ "unpaired": state.db.unpair_all_but(&me)? })))
}

/// The first client is never trusted automatically: a native dialog at the Mac, or the code typed into the
/// terminal Loft runs in, allows it. Printed with `println!`, so the code is never in a log.
fn ask_at_the_mac(state: AppState, code: String, name: String) {
    println!("Pairing request from \"{name}\", code {code}. Allow it in the dialog at the Mac, or type {code} here and press Enter.");
    tokio::spawn(async move {
        let _one_at_a_time = state.pairing.dialog.lock().await;
        if !state.pairing.waiting(&code) {
            return;
        }
        // The text goes in as an argument, not into the script: a client's name can't change the dialog's buttons.
        let text = format!("Allow \"{name}\" to control Loft?\n\nCode {code}");
        let out = tokio::process::Command::new("osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "display dialog (item 1 of argv) with title \"Loft\" buttons {\"Deny\", \"Allow\"} default button \"Deny\" cancel button \"Deny\" giving up after 120",
                "-e",
                "end run",
                "--",
                &text,
            ])
            .output()
            .await;
        match out {
            Ok(o) if String::from_utf8_lossy(&o.stdout).contains("button returned:Allow") => {
                state.pairing.decide(&code, true);
            }
            // Deny, Escape (osascript fails with "User canceled"): refused. No answer at all (no osascript): the terminal stays.
            Ok(o) if !o.status.success() => {
                state.pairing.decide(&code, false);
            }
            _ => {}
        }
    });
}

/// Reads the terminal Loft runs in: a code typed there allows that pairing, but only while no client is paired.
pub async fn read_terminal(state: AppState) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if count(&state) == 0 {
            state.pairing.decide(line.trim(), true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::routing::{get, post};

    /// The real API behind a real socket: a request from 127.0.0.1 like an agent's `curl`.
    async fn serve() -> (String, AppState) {
        let st = crate::tasks::tests::state();
        let app = crate::app(&st, &std::env::temp_dir().join("no-dist"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, st)
    }

    async fn req(m: &str, url: &str, token: Option<&str>, body: Value) -> (u16, Value) {
        let mut r = reqwest::Client::new().request(m.parse().unwrap(), url).json(&body);
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        let res = r.send().await.unwrap();
        (res.status().as_u16(), res.json().await.unwrap_or(Value::Null))
    }

    /// Pairs a client the way the first one is: allowed by code, then collected with the secret.
    async fn pair(base: &str, st: &AppState, name: &str) -> String {
        let (_, v) = req("POST", &format!("{base}/api/pair"), None, json!({ "name": name })).await;
        assert!(st.pairing.decide(v["code"].as_str().unwrap(), true));
        let (_, v) = req("POST", &format!("{base}/api/pair/poll"), None, json!({ "secret": v["secret"] })).await;
        v["token"].as_str().unwrap().to_string()
    }

    /// Every endpoint ADR 0004 lists, plus a few of the writes it leaves to the default.
    const GUARDED: &[(&str, &str)] = &[
        ("PUT", "/push"),
        ("POST", "/push/signin"),
        ("POST", "/devices"),
        ("PATCH", "/devices/abc"),
        ("DELETE", "/devices/abc"),
        ("POST", "/devices/abc/test"),
        ("GET", "/push"),
        ("GET", "/devices"),
        ("POST", "/permissions/abc"),
        ("POST", "/issues/abc/comments/1/approve"),
        ("POST", "/issues/abc/comments/1/keep"),
        ("POST", "/tasks/abc/comments/1/approve"),
        ("POST", "/tasks/abc/comments/1/keep"),
        ("POST", "/pull-requests/abc/merge"),
        ("POST", "/deliveries/merge"),
        ("PUT", "/settings"),
        ("GET", "/settings"),
        ("POST", "/agents"),
        ("PATCH", "/agents/abc"),
        ("DELETE", "/agents/abc"),
        ("POST", "/restart"),
        ("POST", "/system/mac-access"),
        ("POST", "/simulator/input"),
        ("POST", "/agents/abc/open-editor"),
        ("POST", "/agents/abc/messages"),
        ("POST", "/agents/abc/uploads"),
        ("POST", "/workspaces/clean"),
        ("GET", "/workspaces/clean/preview"),
        ("GET", "/workspaces/clean/folders"),
        ("GET", "/workspaces/log"),
        ("POST", "/issues"),
        ("POST", "/agents/abc/run"),
        ("POST", "/agents/abc/memories"),
        ("POST", "/pair/approve"),
        ("POST", "/pair/deny"),
        ("GET", "/paired-clients"),
        ("DELETE", "/paired-clients/abc"),
        ("POST", "/paired-clients/unpair-all"),
    ];

    #[tokio::test]
    async fn every_listed_endpoint_refuses_a_request_with_no_token_even_from_127_0_0_1() {
        let (base, st) = serve().await;
        for (m, p) in GUARDED {
            let (code, _) = req(m, &format!("{base}/api{p}"), None, json!({})).await;
            assert_eq!(code, 401, "{m} {p}");
        }
        let (code, _) = req("GET", &format!("{base}/api/devices"), Some("not-a-token"), Value::Null).await;
        assert_eq!(code, 401, "a token nobody was given");
        assert_eq!(count(&st), 0);
    }

    #[tokio::test]
    async fn a_paired_client_gets_through_with_a_bearer_header_or_the_cookie() {
        let (base, st) = serve().await;
        let token = pair(&base, &st, "Test phone").await;
        // Real handlers that change nothing here (the rest of the list is the middleware's, which looks at the method and path only).
        for (m, p) in [("GET", "/devices"), ("GET", "/push"), ("GET", "/settings"), ("POST", "/permissions/abc"), ("PATCH", "/devices/abc"), ("POST", "/agents/abc/read")] {
            let (code, _) = req(m, &format!("{base}/api{p}"), Some(&token), json!({})).await;
            assert_ne!(code, 401, "{m} {p}");
        }
        let res = reqwest::Client::new().get(format!("{base}/api/devices")).header("cookie", format!("other=1; {COOKIE}={token}")).send().await.unwrap();
        assert_eq!(res.status(), 200);
    }

    #[tokio::test]
    async fn the_middleware_guards_any_write_and_the_named_reads_and_leaves_pairing_open() {
        let st = crate::tasks::tests::state();
        let app: Router = Router::new()
            .route("/anything", post(|| async { "ok" }).delete(|| async { "ok" }).get(|| async { "ok" }))
            .route("/settings", get(|| async { "ok" }))
            .route("/pair", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(st.clone(), require_paired));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let none = |m: &'static str, p: &'static str| {
            let url = format!("{base}{p}");
            async move { req(m, &url, None, json!({})).await }
        };
        assert_eq!(none("POST", "/anything").await.0, 401);
        assert_eq!(none("DELETE", "/anything").await.0, 401);
        assert_eq!(none("GET", "/settings").await.0, 401);
        assert_eq!(none("GET", "/anything").await.0, 200, "other reads stay as they were");
        assert_eq!(none("POST", "/pair").await.0, 200);
    }

    #[tokio::test]
    async fn the_secret_gets_the_token_once_and_the_code_gets_nothing() {
        let (base, st) = serve().await;
        let (code, v) = req("POST", &format!("{base}/api/pair"), None, json!({ "name": "Safari" })).await;
        assert_eq!(code, 200);
        let (secret, shown) = (v["secret"].as_str().unwrap().to_string(), v["code"].as_str().unwrap().to_string());
        assert_eq!(secret.len(), 64, "32 bytes as hex");
        let poll = |s: String| {
            let url = format!("{base}/api/pair/poll");
            async move { req("POST", &url, None, json!({ "secret": s })).await.1 }
        };
        assert_eq!(poll(secret.clone()).await["status"], "pending");
        assert_eq!(poll(shown.clone()).await["status"], "gone", "the code is not a secret");
        assert!(st.pairing.decide(&shown, true));
        assert_eq!(poll(shown.clone()).await["status"], "gone", "still nothing for the code once it is allowed");
        let v = poll(secret.clone()).await;
        assert_eq!(v["status"], "allowed");
        let token = v["token"].as_str().unwrap();
        assert_eq!(poll(secret).await["status"], "gone", "the token is handed out once");
        assert_eq!(req("GET", &format!("{base}/api/devices"), Some(token), Value::Null).await.0, 200);
    }

    #[tokio::test]
    async fn the_web_gets_its_token_as_a_host_cookie_and_not_in_the_body() {
        let (base, st) = serve().await;
        let (_, v) = req("POST", &format!("{base}/api/pair"), None, json!({})).await;
        st.pairing.decide(v["code"].as_str().unwrap(), true);
        let res = reqwest::Client::new().post(format!("{base}/api/pair/poll")).json(&json!({ "secret": v["secret"], "cookie": true })).send().await.unwrap();
        let cookie = res.headers()[header::SET_COOKIE].to_str().unwrap().to_string();
        assert!(cookie.starts_with("__Host-loft=") && cookie.contains("; Secure") && cookie.contains("; HttpOnly") && cookie.contains("; Path=/"), "{cookie}");
        assert!(res.json::<Value>().await.unwrap().get("token").is_none());
        let token = cookie.split(';').next().unwrap().trim_start_matches("__Host-loft=").to_string();
        assert_eq!(req("GET", &format!("{base}/api/settings"), Some(&token), Value::Null).await.0, 200);
    }

    #[tokio::test]
    async fn several_pairings_wait_at_once_and_deny_and_cancel_end_one() {
        let (base, st) = serve().await;
        let mut pending = vec![];
        for i in 0..3 {
            let (_, v) = req("POST", &format!("{base}/api/pair"), None, json!({ "name": format!("c{i}") })).await;
            pending.push((v["secret"].as_str().unwrap().to_string(), v["code"].as_str().unwrap().to_string()));
        }
        let status = |s: String| {
            let url = format!("{base}/api/pair/poll");
            async move { req("POST", &url, None, json!({ "secret": s })).await.1["status"].as_str().unwrap().to_string() }
        };
        for (s, _) in &pending {
            assert_eq!(status(s.clone()).await, "pending");
        }
        assert!(st.pairing.decide(&pending[0].1, false));
        assert_eq!(status(pending[0].0.clone()).await, "denied");
        assert_eq!(status(pending[0].0.clone()).await, "gone");
        req("POST", &format!("{base}/api/pair/cancel"), None, json!({ "secret": pending[1].0 })).await;
        assert_eq!(status(pending[1].0.clone()).await, "gone");
        assert_eq!(status(pending[2].0.clone()).await, "pending", "the others keep waiting");
        assert!(!st.pairing.decide(&pending[1].1, true), "nothing left to allow for a cancelled one");
    }

    #[tokio::test]
    async fn a_paired_client_allows_a_new_one_through_the_api() {
        let (base, st) = serve().await;
        let first = pair(&base, &st, "Mac browser").await;
        let (_, v) = req("POST", &format!("{base}/api/pair"), None, json!({ "name": "iPhone" })).await;
        let code = v["code"].as_str().unwrap();
        assert_eq!(req("POST", &format!("{base}/api/pair/approve"), None, json!({ "code": code })).await.0, 401);
        assert_eq!(req("POST", &format!("{base}/api/pair/approve"), Some(&first), json!({ "code": code })).await.0, 200);
        let (_, v) = req("POST", &format!("{base}/api/pair/poll"), None, json!({ "secret": v["secret"] })).await;
        assert_eq!(v["status"], "allowed");
    }

    #[tokio::test]
    async fn a_pending_pairing_is_a_needs_you_item_for_paired_devices_until_it_is_answered() {
        let (base, st) = serve().await;
        let items = || crate::needs_you::snapshot(&st).unwrap()["items"].as_array().unwrap().iter().filter(|i| i["type"] == "pairing").cloned().collect::<Vec<_>>();
        // With nobody paired, the Mac answers: no item.
        req("POST", &format!("{base}/api/pair"), None, json!({ "name": "first" })).await;
        assert!(items().is_empty());
        let first = pair(&base, &st, "Mac browser").await;
        let (_, a) = req("POST", &format!("{base}/api/pair"), None, json!({ "name": "iPhone" })).await;
        let (_, b) = req("POST", &format!("{base}/api/pair"), None, json!({ "name": "iPad" })).await;
        let list = items();
        assert_eq!(list.len(), 3, "the unanswered first request shows now that a client is paired");
        let it = list.iter().find(|i| i["id"] == format!("pairing:{}", a["code"].as_str().unwrap())).unwrap();
        assert_eq!(it["title"], "A new device wants to pair");
        assert!(it["text"].as_str().unwrap().contains(a["code"].as_str().unwrap()));
        assert_eq!((it["actions"][0]["path"].clone(), it["actions"][1]["path"].clone()), (json!("/api/pair/approve"), json!("/api/pair/deny")));
        // The push says the code and goes out as a permission.
        let push = crate::needs_you::collect(&st).unwrap().iter().map(crate::needs_you::to_push).find(|p| p.id.starts_with("pairing:")).unwrap();
        assert_eq!((push.kind, push.title.as_str()), ("permission", "A new device wants to pair"));
        assert!(push.body.contains("Code "));
        // Allow ends the item and hands out the token; deny ends it and the device is told.
        assert_eq!(req("POST", &format!("{base}/api/pair/approve"), Some(&first), json!({ "code": a["code"] })).await.0, 200);
        assert_eq!(req("POST", &format!("{base}/api/pair/deny"), Some(&first), json!({ "code": b["code"] })).await.0, 200);
        assert_eq!(items().len(), 1);
        assert_eq!(req("POST", &format!("{base}/api/pair/poll"), None, json!({ "secret": a["secret"] })).await.1["status"], "allowed");
        assert_eq!(req("POST", &format!("{base}/api/pair/poll"), None, json!({ "secret": b["secret"] })).await.1["status"], "denied");
        // A cancelled pairing, and an expired one, disappear too.
        let (_, c) = req("POST", &format!("{base}/api/pair"), None, json!({})).await;
        assert_eq!(items().len(), 2);
        req("POST", &format!("{base}/api/pair/cancel"), None, json!({ "secret": c["secret"] })).await;
        assert_eq!(items().len(), 1);
        req("POST", &format!("{base}/api/pair"), None, json!({})).await;
        for p in st.pairing.inner.lock().unwrap().pending.values_mut() {
            p.at = Instant::now() - EXPIRES - Duration::from_secs(1);
        }
        assert!(items().is_empty());
    }

    #[tokio::test]
    async fn the_pair_endpoint_is_rate_limited() {
        let (base, _st) = serve().await;
        for _ in 0..MAX_PER_MINUTE {
            assert_eq!(req("POST", &format!("{base}/api/pair"), None, json!({})).await.0, 200);
        }
        assert_eq!(req("POST", &format!("{base}/api/pair"), None, json!({})).await.0, 429);
    }

    #[tokio::test]
    async fn a_pairing_nobody_allows_expires_and_nothing_pending_is_stored() {
        let (base, st) = serve().await;
        let (_, v) = req("POST", &format!("{base}/api/pair"), None, json!({})).await;
        for p in st.pairing.inner.lock().unwrap().pending.values_mut() {
            p.at = Instant::now() - EXPIRES - Duration::from_secs(1);
        }
        assert!(!st.pairing.decide(v["code"].as_str().unwrap(), true));
        assert_eq!(req("POST", &format!("{base}/api/pair/poll"), None, json!({ "secret": v["secret"] })).await.1["status"], "gone");
        // Pending pairings are only in memory: the database holds paired clients and nothing else of this.
        assert_eq!(count(&st), 0);
    }

    #[tokio::test]
    async fn only_the_hash_of_a_token_is_stored_with_a_name_and_last_used() {
        let (base, st) = serve().await;
        let token = pair(&base, &st, "Gustavo's iPhone").await;
        let (h, name, last): (String, String, Option<String>) =
            st.db.conn().query_row("SELECT hash, name, last_used_at FROM paired_clients", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(h, hash(&token));
        assert_ne!(h, token);
        assert_eq!(name, "Gustavo's iPhone");
        let before = last.unwrap();
        st.db.conn().execute("UPDATE paired_clients SET last_used_at = '2000-01-01T00:00:00Z'", []).unwrap();
        assert_eq!(req("GET", &format!("{base}/api/devices"), Some(&token), Value::Null).await.0, 200);
        let after: String = st.db.conn().query_row("SELECT last_used_at FROM paired_clients", [], |r| r.get(0)).unwrap();
        assert!(after.as_str() > "2000-01-01T00:00:00Z" && !before.is_empty());
    }

    #[tokio::test]
    async fn last_used_is_written_at_most_once_a_minute() {
        let (base, st) = serve().await;
        let token = pair(&base, &st, "iPhone").await;
        let recent = (chrono::Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
        st.db.conn().execute("UPDATE paired_clients SET last_used_at = ?1", [&recent]).unwrap();
        assert_eq!(req("GET", &format!("{base}/api/devices"), Some(&token), Value::Null).await.0, 200);
        let after: String = st.db.conn().query_row("SELECT last_used_at FROM paired_clients", [], |r| r.get(0)).unwrap();
        assert_eq!(after, recent, "used 10 seconds ago: not written again");
    }

    #[tokio::test]
    async fn a_failed_insert_keeps_the_pairing_so_the_client_can_poll_again() {
        let (base, st) = serve().await;
        let (_, v) = req("POST", &format!("{base}/api/pair"), None, json!({ "name": "iPhone" })).await;
        assert!(st.pairing.decide(v["code"].as_str().unwrap(), true));
        st.db.conn().execute("ALTER TABLE paired_clients RENAME TO paired_away", []).unwrap();
        let poll = json!({ "secret": v["secret"] });
        assert_eq!(req("POST", &format!("{base}/api/pair/poll"), None, poll.clone()).await.0, 500);
        st.db.conn().execute("ALTER TABLE paired_away RENAME TO paired_clients", []).unwrap();
        let (code, v) = req("POST", &format!("{base}/api/pair/poll"), None, poll).await;
        assert_eq!((code, &v["status"]), (200, &json!("allowed")));
        assert!(v["token"].is_string());
    }

    #[tokio::test]
    async fn the_list_shows_each_client_and_marks_the_one_asking_without_its_hash() {
        let (base, st) = serve().await;
        let phone = pair(&base, &st, "iPhone").await;
        pair(&base, &st, "Safari on Mac").await;
        let (code, v) = req("GET", &format!("{base}/api/paired-clients"), Some(&phone), Value::Null).await;
        assert_eq!(code, 200);
        let list = v.as_array().unwrap();
        assert_eq!(list.len(), 2);
        let me = list.iter().find(|c| c["this"] == true).unwrap();
        assert_eq!(me["name"], "iPhone");
        assert!(me["paired_at"].is_string() && me["last_used_at"].is_string());
        assert_eq!(list.iter().filter(|c| c["this"] == true).count(), 1);
        assert_eq!(me["id"].as_str().unwrap().len(), 16, "the start of the hash, not all of it");
        assert!(!v.to_string().contains(&hash(&phone)));
    }

    #[tokio::test]
    async fn forget_this_device_ends_that_clients_access_on_its_next_request() {
        let (base, st) = serve().await;
        let phone = pair(&base, &st, "iPhone").await;
        let mac = pair(&base, &st, "Safari on Mac").await;
        let (_, v) = req("GET", &format!("{base}/api/paired-clients"), Some(&mac), Value::Null).await;
        let id = v.as_array().unwrap().iter().find(|c| c["name"] == "iPhone").unwrap()["id"].as_str().unwrap().to_string();
        assert_eq!(req("DELETE", &format!("{base}/api/paired-clients/{id}"), None, Value::Null).await.0, 401, "only a paired client unpairs");
        assert_eq!(req("DELETE", &format!("{base}/api/paired-clients/{id}"), Some(&mac), Value::Null).await.0, 200);
        assert_eq!(req("GET", &format!("{base}/api/devices"), Some(&phone), Value::Null).await.0, 401);
        assert_eq!(req("GET", &format!("{base}/api/devices"), Some(&mac), Value::Null).await.0, 200);
        assert_eq!(req("DELETE", &format!("{base}/api/paired-clients/{id}"), Some(&mac), Value::Null).await.0, 404);
        // A client can forget itself: its very next request is refused.
        let (_, v) = req("GET", &format!("{base}/api/paired-clients"), Some(&mac), Value::Null).await;
        let own = v[0]["id"].as_str().unwrap().to_string();
        assert_eq!(req("DELETE", &format!("{base}/api/paired-clients/{own}"), Some(&mac), Value::Null).await.0, 200);
        assert_eq!(req("GET", &format!("{base}/api/paired-clients"), Some(&mac), Value::Null).await.0, 401);
    }

    #[tokio::test]
    async fn unpair_all_ends_every_other_clients_access_and_keeps_this_one() {
        let (base, st) = serve().await;
        let a = pair(&base, &st, "a").await;
        let b = pair(&base, &st, "b").await;
        let me = pair(&base, &st, "me").await;
        let (code, v) = req("POST", &format!("{base}/api/paired-clients/unpair-all"), Some(&me), json!({})).await;
        assert_eq!((code, v["unpaired"].as_i64()), (200, Some(2)));
        for t in [&a, &b] {
            assert_eq!(req("GET", &format!("{base}/api/settings"), Some(t), Value::Null).await.0, 401);
        }
        let (code, v) = req("GET", &format!("{base}/api/paired-clients"), Some(&me), Value::Null).await;
        assert_eq!(code, 200);
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["this"], true);
    }

    #[tokio::test]
    async fn the_first_client_is_not_paired_automatically_and_agents_mcp_route_needs_no_token() {
        // The first client is only ever allowed by `decide`, which the Mac dialog and the terminal call; with prompts
        // off (tests) nobody calls it, so the pairing stays pending.
        let (base, st) = serve().await;
        let (_, v) = req("POST", &format!("{base}/api/pair"), None, json!({})).await;
        assert_eq!(req("POST", &format!("{base}/api/pair/poll"), None, json!({ "secret": v["secret"] })).await.1["status"], "pending");
        assert_eq!(count(&st), 0);
        // The agents' MCP route is not under `/api`, so it never asks for a token.
        let res = reqwest::Client::new().post(format!("{base}/mcp/nobody")).json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })).send().await.unwrap();
        assert_ne!(res.status(), 401);
    }

    #[tokio::test]
    async fn the_herdr_client_is_built_only_for_a_paired_client() {
        // The test state has no herdr folder, so nothing is built yet.
        let (base, st) = serve().await;
        let url = format!("{base}/api/herdr/bin/{}", crate::herdr::platform());
        let (code, _) = req("GET", &url, None, Value::Null).await;
        assert_eq!(code, 401, "an unpaired client doesn't start a build");
        let token = pair(&base, &st, "loft-herdr").await;
        let (code, _) = req("GET", &url, Some(&token), Value::Null).await;
        assert_ne!(code, 401, "a paired client gets as far as the build");
    }
}
