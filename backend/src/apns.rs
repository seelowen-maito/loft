//! Push notifications through Apple's APNs, so alerts reach the iPhone while the app is closed.
//!
//! Token-based auth: an APNs auth key (`.p8`, P-256) signs a short-lived JWT; key id, team id and the
//! app's bundle id (the topic) are settings. The key itself is a secret: it lives in
//! `~/Library/Application Support/Loft/apns/` (owner-only), outside the repo and Loft's data
//! folder, and is never logged, returned by the API or shown to agents.
//!
//! Each device says which APNs environment its build uses: Xcode/debug builds `development`
//! (api.sandbox.push.apple.com), TestFlight and App Store builds `production` (api.push.apple.com).
//! The same key works for both.
//!
//! An install without a key of its own can send through the hosted push relay instead (ADR 0002,
//! `relay/`), with a relay key the user pastes in Settings (kept next to where the `.p8` would be, as
//! secret as it). Each push carries the phone's token and environment; the relay passes Apple's answer
//! back. With neither, nothing is pushed.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::pkcs8::DecodePrivateKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::AppState;

/// A registered phone (`/api/devices`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Device {
    /// APNs device token, lowercase hex.
    pub token: String,
    pub platform: String,
    /// development (Xcode builds) | production (TestFlight, App Store)
    pub environment: String,
    pub bundle_id: String,
    pub name: String,
    /// "Needs you" item types this device wants pushed.
    pub kinds: Vec<String>,
    /// No pushes in this window, in the device's time zone. None = always.
    pub quiet_hours: Option<QuietHours>,
    pub created_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuietHours {
    /// "22:00"
    pub start: String,
    /// "07:00"
    pub end: String,
    /// IANA name, e.g. "Europe/Lisbon"; informational, the offset is what's used.
    #[serde(default)]
    pub tz: String,
    /// Minutes east of UTC at registration, so the server needs no time zone database.
    #[serde(default)]
    pub utc_offset_minutes: i32,
}

/// Every type a device can ask for, and the ones on by default.
pub const KINDS: &[&str] = &["permission", "task_approval", "task_blocked", "question", "grill_round", "stuck", "task_failed", "pr_ready", "pr_review"];
pub const DEFAULT_KINDS: &[&str] = &["permission", "task_approval", "task_blocked", "question", "grill_round", "stuck"];

/// A kind under its current name: app builds from before rounds ask for `grill_question`.
pub fn kind(k: &str) -> &str {
    if k == "grill_question" { "grill_round" } else { k }
}

/// Where the key lives; tests point it elsewhere with LOFT_APNS_DIR.
pub fn key_dir() -> PathBuf {
    std::env::var_os("LOFT_APNS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("Loft").join("apns"))
}

fn key_path() -> PathBuf {
    key_dir().join("AuthKey.p8")
}

pub fn key_present() -> bool {
    key_path().is_file()
}

/// Whether this install has its own key (the `.p8`, key id and team id), to send straight to Apple.
pub fn own_key(state: &AppState) -> bool {
    let s = state.settings.get().push;
    key_present() && !s.key_id.trim().is_empty() && !s.team_id.trim().is_empty()
}

/// Which way this install's pushes go.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Route {
    /// Straight to Apple with its own key.
    OwnKey,
    /// Through the push relay with its relay key.
    Relay,
    /// Neither: nothing is pushed.
    Off,
}

impl Route {
    /// As `GET /api/push` names it.
    pub fn name(self) -> &'static str {
        match self {
            Route::OwnKey => "own_key",
            Route::Relay => "relay",
            Route::Off => "none",
        }
    }
}

/// Its own key wins over a relay key; with neither, push is off.
fn route_for(own_key: bool, relay_key: bool) -> Route {
    match (own_key, relay_key) {
        (true, _) => Route::OwnKey,
        (false, true) => Route::Relay,
        (false, false) => Route::Off,
    }
}

/// A relay key only routes through the relay when this build knows where the relay is.
pub fn route(state: &AppState) -> Route {
    route_for(own_key(state), relay_key_present() && relay_url(state).is_some())
}

/// The hosted push relay. Empty until the operator's Vercel project exists (#259), so no one else's
/// project can receive relay keys and push text under its name; LOFT_RELAY_URL sets it meanwhile.
const RELAY_URL: &str = "";

/// The push portal, where a user signs in and gets a relay key (spec #265). Empty until it's live, so
/// Settings shows no dead link; LOFT_PORTAL_URL sets it meanwhile.
const PORTAL_URL: &str = "";

/// A relay or portal address as typed in Settings: https, with a host and no user name or password; ""
/// clears it. Tests may also use plain http on this Mac, for their fake relay.
pub fn check_address(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    let url = reqwest::Url::parse(raw).map_err(|_| format!("{raw} isn't a web address: it starts with https://"))?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if !(url.scheme() == "https" || (cfg!(test) && url.scheme() == "http" && local)) {
        return Err("The address must start with https://".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The address can't hold a user name or password".into());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("The address needs a host, like https://loft-push-relay.vercel.app".into());
    }
    Ok(raw.trim_end_matches('/').to_string())
}

/// The address saved in Settings, else the environment variable `var`, else `fallback`; checked like a typed one.
fn address(saved: &str, var: &str, fallback: &str) -> Option<String> {
    let raw = if saved.trim().is_empty() { std::env::var(var).unwrap_or_else(|_| fallback.into()) } else { saved.to_string() };
    check_address(&raw).ok().filter(|u| !u.is_empty())
}

pub fn portal_url(state: &AppState) -> Option<String> {
    address(&state.settings.get().push.portal_url, "LOFT_PORTAL_URL", PORTAL_URL)
}

pub fn relay_url(state: &AppState) -> Option<String> {
    address(&state.settings.get().push.relay_url, "LOFT_RELAY_URL", if cfg!(test) { "http://127.0.0.1:9" } else { RELAY_URL })
}

fn relay_key_path() -> PathBuf {
    key_dir().join("relay-key")
}

pub fn relay_key_present() -> bool {
    relay_key_path().is_file()
}

/// Checks a relay key's shape (`lrk_` and 43 base64url characters) and stores it owner-only. It never
/// reaches a log, the API or an agent.
pub fn store_relay_key(key: &str) -> Result<()> {
    let key = key.trim();
    let ok = key.len() == 47 && key.starts_with("lrk_") && key[4..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !ok {
        bail!("That isn't a relay key: it starts with lrk_ and is 47 characters long.");
    }
    write_secret(&relay_key_path(), key)
}

pub fn remove_relay_key() -> Result<()> {
    match std::fs::remove_file(relay_key_path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Writes `text` to `path` in the key folder, owner-only, through a temporary `<name>.tmp` beside it.
fn write_secret(path: &std::path::Path, text: &str) -> Result<()> {
    let dir = key_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    let tmp = path.with_file_name(format!("{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("secret")));
    std::fs::write(&tmp, text)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Checks a `.p8` is a P-256 private key and stores it owner-only. The text never reaches a log.
pub fn store_key(pem: &str) -> Result<()> {
    let pem = pem.trim();
    SigningKey::from_pkcs8_pem(pem).map_err(|_| anyhow!("That isn't an APNs auth key: expected the .p8 file Apple gives you (a P-256 private key)."))?;
    write_secret(&key_path(), &format!("{pem}\n"))?;
    // A new key invalidates any cached token.
    *JWT.lock().unwrap() = None;
    Ok(())
}

pub fn remove_key() -> Result<()> {
    match std::fs::remove_file(key_path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    *JWT.lock().unwrap() = None;
    Ok(())
}

/// Cached provider token: Apple wants it reused for up to an hour and refreshed at least every hour.
static JWT: Mutex<Option<(String, String, Instant)>> = Mutex::new(None);
const JWT_TTL: Duration = Duration::from_secs(45 * 60);

fn provider_token(key_id: &str, team_id: &str) -> Result<String> {
    let cache_key = format!("{key_id}/{team_id}");
    if let Some((k, t, at)) = JWT.lock().unwrap().as_ref()
        && *k == cache_key
        && at.elapsed() < JWT_TTL
    {
        return Ok(t.clone());
    }
    let pem = std::fs::read_to_string(key_path()).context("No APNs key: add the .p8 file in Settings → Push notifications")?;
    let key = SigningKey::from_pkcs8_pem(pem.trim()).map_err(|_| anyhow!("The stored APNs key can't be read; add it again in Settings"))?;
    let token = sign_jwt(&key, key_id, team_id, chrono::Utc::now().timestamp());
    *JWT.lock().unwrap() = Some((cache_key, token.clone(), Instant::now()));
    Ok(token)
}

/// ES256 JWT: `{"alg":"ES256","kid":…}.{"iss":team,"iat":…}`, signature as raw r‖s.
pub fn sign_jwt(key: &SigningKey, key_id: &str, team_id: &str, iat: i64) -> String {
    let header = URL_SAFE_NO_PAD.encode(json!({ "alg": "ES256", "kid": key_id }).to_string());
    let claims = URL_SAFE_NO_PAD.encode(json!({ "iss": team_id, "iat": iat }).to_string());
    let input = format!("{header}.{claims}");
    let sig: Signature = key.sign(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig.to_bytes()))
}

/// One notification to send.
pub struct Push {
    pub title: String,
    pub body: String,
    pub badge: Option<u64>,
    /// Groups notifications on the phone (agent id).
    pub thread_id: Option<String>,
    /// Urgency 3 breaks through Focus as Time Sensitive.
    pub time_sensitive: bool,
    /// Replaces an earlier notification with the same id (≤ 64 bytes; longer ids are hashed).
    pub collapse_id: Option<String>,
    /// App-defined keys next to `aps`.
    pub custom: Value,
}

impl Push {
    pub fn payload(&self) -> Value {
        let mut aps = json!({
            "alert": { "title": self.title, "body": self.body },
            "sound": "default",
            "interruption-level": if self.time_sensitive { "time-sensitive" } else { "active" },
        });
        if let Some(b) = self.badge {
            aps["badge"] = json!(b);
        }
        if let Some(t) = &self.thread_id {
            aps["thread-id"] = json!(t);
        }
        let mut p = json!({ "aps": aps });
        if let Value::Object(extra) = &self.custom {
            for (k, v) in extra {
                p[k] = v.clone();
            }
        }
        p
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Sent,
    /// The token is no longer valid for this app: the device was removed.
    Unregistered,
}

fn host(environment: &str) -> String {
    if let Ok(url) = std::env::var("LOFT_APNS_URL") {
        return url;
    }
    if environment == "development" { "https://api.sandbox.push.apple.com".into() } else { "https://api.push.apple.com".into() }
}

fn collapse(id: &str) -> String {
    if id.len() <= 64 {
        return id.to_string();
    }
    // Stable and short: FNV-1a, hex.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in id.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Sends to one device, with the install's own key or through the push relay. A 410, or a 400 saying
/// the token is bad or for another app, removes it.
pub async fn send(state: &AppState, device: &Device, push: &Push) -> Result<Outcome> {
    let (status, reason) = match route(state) {
        Route::OwnKey => direct(state, device, push).await?,
        Route::Relay => via_relay(state, &relay_url(state).unwrap_or_default(), device, push).await?,
        Route::Off => bail!("Push is off: sign in to get push (paste your relay key), or add your own APNs key in Settings"),
    };
    if status == 200 {
        return Ok(Outcome::Sent);
    }
    if status == 410 || (status == 400 && ["BadDeviceToken", "DeviceTokenNotForTopic"].contains(&reason.as_str())) {
        state.db.delete_device(&device.token)?;
        tracing::info!("apns: removed a device that is no longer registered ({reason})");
        return Ok(Outcome::Unregistered);
    }
    bail!("APNs refused the push: {status} {reason}")
}

/// The app's bundle id for a device: the one it registered with, else the setting.
fn topic(state: &AppState, device: &Device) -> String {
    if device.bundle_id.is_empty() { state.settings.get().push.bundle_id } else { device.bundle_id.clone() }
}

/// Straight to Apple with the install's key: Apple's status and reason.
async fn direct(state: &AppState, device: &Device, push: &Push) -> Result<(u16, String)> {
    let s = state.settings.get().push;
    let jwt = provider_token(s.key_id.trim(), s.team_id.trim())?;
    let topic = topic(state, device);
    if topic.trim().is_empty() {
        bail!("Push isn't set up: add the app's bundle id in Settings");
    }
    let base = host(&device.environment);
    // Apple only speaks HTTP/2; over TLS reqwest negotiates it, a plain test server needs prior knowledge.
    let client = if base.starts_with("http://") {
        reqwest::Client::builder().http2_prior_knowledge().build()?
    } else {
        state.http.clone()
    };
    let mut req = client
        .post(format!("{base}/3/device/{}", device.token))
        .bearer_auth(jwt)
        .header("apns-topic", topic)
        .header("apns-push-type", "alert")
        .header("apns-priority", if push.time_sensitive { "10" } else { "5" })
        .timeout(Duration::from_secs(15))
        .json(&push.payload());
    if let Some(c) = &push.collapse_id {
        req = req.header("apns-collapse-id", collapse(c));
    }
    let res = req.send().await.context("APNs unreachable")?;
    let status = res.status().as_u16();
    let reason = res.json::<Value>().await.ok().and_then(|v| v["reason"].as_str().map(String::from)).unwrap_or_default();
    Ok((status, reason))
}

/// Through the push relay at `base`: Apple's status and reason, as the relay passes them back.
async fn via_relay(state: &AppState, base: &str, device: &Device, push: &Push) -> Result<(u16, String)> {
    let key = std::fs::read_to_string(relay_key_path()).context("No relay key: paste it in Settings → Push notifications")?;
    let mut body = json!({
        "device_token": device.token,
        "environment": device.environment,
        "bundle_id": topic(state, device),
        "payload": push.payload(),
        "priority": if push.time_sensitive { 10 } else { 5 },
    });
    if let Some(c) = &push.collapse_id {
        body["collapse_id"] = json!(collapse(c));
    }
    let res = state
        .http
        .post(format!("{base}/api/push"))
        .bearer_auth(key.trim())
        .timeout(Duration::from_secs(15))
        .json(&body)
        .send()
        .await
        .context("The push relay is unreachable")?;
    let status = res.status().as_u16();
    let v = res.json::<Value>().await.unwrap_or(Value::Null);
    match status {
        200 => match v["apple_status"].as_u64() {
            Some(s) => Ok((s as u16, v["reason"].as_str().unwrap_or("").to_string())),
            None => bail!("The push relay's answer had no status from Apple"),
        },
        401 => bail!("The push relay doesn't accept this relay key"),
        429 => bail!("The push relay's limit for this install is reached; later pushes go through"),
        s => bail!("The push relay refused the push: {s} {}", v["error"].as_str().unwrap_or("")),
    }
}

/// Valid lowercase hex device token (APNs tokens are 32 bytes today; allow longer for the future).
pub fn valid_token(t: &str) -> bool {
    (64..=200).contains(&t.len()) && t.len().is_multiple_of(2) && t.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use p256::ecdsa::{VerifyingKey, signature::Verifier};

    /// Requests the fake APNs server got: (device token, headers we check, body).
    pub(crate) type Seen = std::sync::Arc<Mutex<Vec<(String, Value, Value)>>>;

    /// The relay key every test install holds (stored with the test `.p8` when the fakes start).
    pub(crate) const RELAY_KEY: &str = "lrk_test-relay-key-aaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// What the fake push relay got: each push as (authorization, body).
    #[derive(Default)]
    pub(crate) struct Relayed {
        pub pushes: Vec<(String, Value)>,
        /// Sign-in codes handed out, and whether the user allowed them yet.
        pub codes: std::collections::HashMap<String, bool>,
    }
    static RELAYED: std::sync::OnceLock<std::sync::Arc<Mutex<Relayed>>> = std::sync::OnceLock::new();

    /// The fake push relay's log, shared by every test (started with the fake APNs).
    pub(crate) fn relayed() -> std::sync::Arc<Mutex<Relayed>> {
        fake_apns();
        RELAYED.get().unwrap().clone()
    }

    /// A fake relay with the real one's contract: 401 for any key but RELAY_KEY, and for a push Apple's
    /// answer as the fake APNs would give it (410 for device tokens starting "dead"), or 503 when the
    /// alert's title starts with "Refused".
    fn relay_routes(log: std::sync::Arc<Mutex<Relayed>>) -> axum::Router {
        use axum::http::{HeaderMap, StatusCode};
        let (pairs, polls) = (log.clone(), log.clone());
        axum::Router::new()
            .route("/api/pair", axum::routing::post(move || {
                let log = pairs.clone();
                async move {
                    let code = uuid::Uuid::new_v4().simple().to_string()[..22].to_string();
                    log.lock().unwrap().codes.insert(code.clone(), false);
                    axum::Json(json!({ "code": code, "secret": format!("s-{code}"), "expires_in": 600 }))
                }
            }))
            .route("/api/pair-poll", axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let log = polls.clone();
                async move {
                    let code = body["code"].as_str().unwrap_or("").to_string();
                    // The fake's secret is "s-<code>": the code alone gets nothing.
                    let code = if body["secret"].as_str() == Some(&format!("s-{code}")) { code } else { String::new() };
                    let mut l = log.lock().unwrap();
                    match l.codes.get(&code).copied() {
                        Some(true) => { l.codes.remove(&code); (StatusCode::OK, axum::Json(json!({ "key": RELAY_KEY }))) }
                        Some(false) => (StatusCode::ACCEPTED, axum::Json(json!({ "status": "waiting" }))),
                        None => (StatusCode::NOT_FOUND, axum::Json(json!({ "error": "unknown or expired code" }))),
                    }
                }
            }))
            .route("/api/push", axum::routing::post(move |headers: HeaderMap, axum::Json(body): axum::Json<Value>| {
                let log = log.clone();
                async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                    log.lock().unwrap().pushes.push((auth.clone(), body.clone()));
                    if auth != format!("Bearer {RELAY_KEY}") {
                        return (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": "a valid relay key is required" })));
                    }
                    if body["payload"]["aps"]["alert"]["title"].as_str().is_some_and(|t| t.starts_with("Refused")) {
                        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({ "error": "busy" })));
                    }
                    let gone = body["device_token"].as_str().is_some_and(|t| t.starts_with("dead"));
                    (StatusCode::OK, axum::Json(if gone { json!({ "apple_status": 410, "reason": "Unregistered" }) } else { json!({ "apple_status": 200, "reason": null }) }))
                }
            }))
    }

    /// The fake relay's address.
    pub(crate) fn fake_apns_relay() -> String {
        fake_apns();
        std::env::var("LOFT_RELAY_URL").unwrap()
    }

    /// The user allows `code` on the (fake) portal.
    pub(crate) fn approve_code(code: &str) {
        relayed().lock().unwrap().codes.insert(code.to_string(), true);
    }

    /// One fake APNs (HTTP/2, 200, or 410 for tokens starting "dead") for the whole test process,
    /// on its own thread so it outlives each test's runtime; LOFT_APNS_URL/DIR point at it and a
    /// shared key folder. Tests tell their requests apart by device token.
    pub(crate) fn fake_apns() -> Seen {
        use axum::extract::Path;
        use axum::http::{HeaderMap, StatusCode};
        static SEEN: std::sync::OnceLock<Seen> = std::sync::OnceLock::new();
        SEEN.get_or_init(|| {
            let seen: Seen = Default::default();
            let s2 = seen.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    let app = axum::Router::new().route(
                        "/3/device/{token}",
                        axum::routing::post(move |Path(token): Path<String>, headers: HeaderMap, axum::Json(body): axum::Json<Value>| {
                            let seen = s2.clone();
                            async move {
                                let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                                seen.lock().unwrap().push((token.clone(), json!({
                                    "authorization": h("authorization"), "apns-topic": h("apns-topic"), "apns-push-type": h("apns-push-type"),
                                    "apns-priority": h("apns-priority"), "apns-collapse-id": h("apns-collapse-id"),
                                }), body));
                                if token.starts_with("dead") {
                                    (StatusCode::GONE, axum::Json(json!({ "reason": "Unregistered" })))
                                } else {
                                    (StatusCode::OK, axum::Json(json!({})))
                                }
                            }
                        }),
                    );
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let log = RELAYED.get_or_init(Default::default).clone();
                    tx.send((format!("http://{}", listener.local_addr().unwrap()), format!("http://{}", relay.local_addr().unwrap()))).unwrap();
                    tokio::spawn(async move { axum::serve(relay, relay_routes(log)).await.unwrap() });
                    axum::serve(listener, app).await.unwrap();
                });
            });
            let (url, relay) = rx.recv().unwrap();
            let dir = std::env::temp_dir().join(format!("loft-apns-test-{}", uuid::Uuid::new_v4()));
            // Set once, before any test reads them.
            unsafe {
                std::env::set_var("LOFT_APNS_URL", &url);
                std::env::set_var("LOFT_RELAY_URL", &relay);
                std::env::set_var("LOFT_APNS_DIR", &dir);
            }
            store_key(&test_key_pem()).unwrap();
            store_relay_key(RELAY_KEY).unwrap();
            seen
        })
        .clone()
    }

    /// Settings with push configured, for a test state.
    pub(crate) fn configure(st: &AppState) {
        let mut s = st.settings.get();
        s.projects_dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        s.push.key_id = "ABC123DEFG".into();
        s.push.team_id = "TEAM123456".into();
        st.settings.save(s).unwrap();
    }

    /// A throwaway P-256 key from fixed bytes, as a `.p8` (PKCS#8 PEM) like Apple's.
    pub(crate) fn test_key_pem() -> String {
        use p256::pkcs8::{EncodePrivateKey, LineEnding};
        SigningKey::from_bytes(&[7u8; 32].into()).unwrap().to_pkcs8_pem(LineEnding::LF).unwrap().to_string()
    }

    #[test]
    fn its_own_key_wins_and_with_neither_key_push_is_off() {
        assert_eq!(route_for(true, true), Route::OwnKey);
        assert_eq!(route_for(true, false), Route::OwnKey);
        assert_eq!(route_for(false, true), Route::Relay);
        assert_eq!(route_for(false, false), Route::Off);
        assert_eq!([Route::OwnKey, Route::Relay, Route::Off].map(Route::name), ["own_key", "relay", "none"]);
    }

    #[test]
    fn only_a_relay_key_shaped_like_one_is_stored() {
        fake_apns();
        for bad in ["", "lrk_short", "lrt_test-relay-key-aaaaaaaaaaaaaaaaaaaaaaaaaaaa", "lrk_test-relay-key-aaaaaaaaaaaaaaaaaaaaaaaaaa/a", &format!("{RELAY_KEY}x")] {
            assert!(store_relay_key(bad).is_err(), "{bad:?}");
        }
        // The same key again (the fakes stored it): kept owner-only, as the .p8.
        store_relay_key(&format!(" {RELAY_KEY}\n")).unwrap();
        let file = key_dir().join("relay-key");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), RELAY_KEY);
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn an_unreachable_relay_is_an_error_not_a_crash() {
        fake_apns();
        let st = crate::tasks::tests::state();
        let d = Device { token: "ab".repeat(32), environment: "production".into(), bundle_id: "com.example.loft".into(), ..Default::default() };
        let p = Push { title: "t".into(), body: "b".into(), badge: None, thread_id: None, time_sensitive: false, collapse_id: None, custom: json!({}) };
        let err = via_relay(&st, "http://127.0.0.1:9", &d, &p).await.unwrap_err();
        assert!(format!("{err:#}").starts_with("The push relay is unreachable"), "{err:#}");
    }

    #[test]
    fn jwt_is_es256_and_verifies() {
        let key = SigningKey::from_pkcs8_pem(&test_key_pem()).unwrap();
        let jwt = sign_jwt(&key, "ABC123DEFG", "TEAM123456", 1_700_000_000);
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(header, json!({ "alg": "ES256", "kid": "ABC123DEFG" }));
        assert_eq!(claims, json!({ "iss": "TEAM123456", "iat": 1_700_000_000 }));
        let sig = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
        VerifyingKey::from(&key).verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).unwrap();
    }

    #[test]
    fn payload_and_helpers() {
        let p = Push {
            title: "Dev needs your approval".into(),
            body: "Bash: Push the fix".into(),
            badge: Some(3),
            thread_id: Some("agent-1".into()),
            time_sensitive: true,
            collapse_id: None,
            custom: json!({ "needs_you_id": "permission:p1", "type": "permission" }),
        };
        assert_eq!(p.payload(), json!({
            "aps": { "alert": { "title": "Dev needs your approval", "body": "Bash: Push the fix" }, "sound": "default",
                     "interruption-level": "time-sensitive", "badge": 3, "thread-id": "agent-1" },
            "needs_you_id": "permission:p1", "type": "permission",
        }));
        assert_eq!(collapse("short"), "short");
        assert_eq!(collapse(&"x".repeat(100)).len(), 16);
        assert!(valid_token(&"ab".repeat(32)) && !valid_token("xyz") && !valid_token(&"zz".repeat(32)));
    }
}
