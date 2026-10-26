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
pub const KINDS: &[&str] = &["permission", "task_approval", "task_blocked", "question", "grill_question", "stuck", "task_failed", "pr_ready", "pr_review"];
pub const DEFAULT_KINDS: &[&str] = &["permission", "task_approval", "task_blocked", "question", "grill_question", "stuck"];

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

/// Checks a `.p8` is a P-256 private key and stores it owner-only. The text never reaches a log.
pub fn store_key(pem: &str) -> Result<()> {
    let pem = pem.trim();
    SigningKey::from_pkcs8_pem(pem).map_err(|_| anyhow!("That isn't an APNs auth key: expected the .p8 file Apple gives you (a P-256 private key)."))?;
    let dir = key_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    let tmp = dir.join("AuthKey.p8.tmp");
    std::fs::write(&tmp, format!("{pem}\n"))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, key_path())?;
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

/// Sends to one device. A 410, or a 400 saying the token is bad or for another app, removes it.
pub async fn send(state: &AppState, device: &Device, push: &Push) -> Result<Outcome> {
    let s = state.settings.get().push;
    if s.key_id.trim().is_empty() || s.team_id.trim().is_empty() {
        bail!("Push isn't set up: add the APNs key, key id and team id in Settings");
    }
    let jwt = provider_token(s.key_id.trim(), s.team_id.trim())?;
    let topic = if device.bundle_id.is_empty() { s.bundle_id.clone() } else { device.bundle_id.clone() };
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
    if status == 200 {
        return Ok(Outcome::Sent);
    }
    let reason = res.json::<Value>().await.ok().and_then(|v| v["reason"].as_str().map(String::from)).unwrap_or_default();
    if status == 410 || (status == 400 && ["BadDeviceToken", "DeviceTokenNotForTopic"].contains(&reason.as_str())) {
        state.db.delete_device(&device.token)?;
        tracing::info!("apns: removed a device that is no longer registered ({reason})");
        return Ok(Outcome::Unregistered);
    }
    bail!("APNs refused the push: {status} {reason}")
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
                    tx.send(format!("http://{}", listener.local_addr().unwrap())).unwrap();
                    axum::serve(listener, app).await.unwrap();
                });
            });
            let url = rx.recv().unwrap();
            let dir = std::env::temp_dir().join(format!("loft-apns-test-{}", uuid::Uuid::new_v4()));
            // Set once, before any test reads them.
            unsafe {
                std::env::set_var("LOFT_APNS_URL", &url);
                std::env::set_var("LOFT_APNS_DIR", &dir);
            }
            store_key(&test_key_pem()).unwrap();
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
