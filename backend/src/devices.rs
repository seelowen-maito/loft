//! `/api/devices` (phones registered for push) and `/api/push` (the APNs key and its status).
//! Device tokens and the key are private: reads refuse other websites, the key is never returned.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{ApiError, bad, not_found};
use crate::apns::{self, DEFAULT_KINDS, Device, KINDS, Push, QuietHours};
use crate::db::now;
use crate::guard::same_site_read;
use crate::state::AppState;

type ApiResult<T> = Result<Json<T>, ApiError>;

/// The kinds under their current names, or an error naming the first unknown one.
fn check_kinds(kinds: Vec<String>) -> Result<Vec<String>, ApiError> {
    match kinds.iter().find(|k| !KINDS.contains(&apns::kind(k))) {
        Some(k) => Err(bad(format!("unknown kind {k}; kinds are {}", KINDS.join(", ")))),
        None => Ok(kinds.iter().map(|k| apns::kind(k).to_string()).collect()),
    }
}

fn check_quiet(q: &Option<QuietHours>) -> Result<(), ApiError> {
    let hm = |s: &str| s.split_once(':').is_some_and(|(h, m)| h.parse::<u8>().is_ok_and(|h| h < 24) && m.parse::<u8>().is_ok_and(|m| m < 60) && m.len() == 2);
    match q {
        Some(q) if !(hm(&q.start) && hm(&q.end)) => Err(bad("quiet_hours start and end are HH:MM")),
        Some(q) if q.utc_offset_minutes.abs() > 14 * 60 => Err(bad("utc_offset_minutes is out of range")),
        _ => Ok(()),
    }
}

#[derive(Deserialize)]
pub struct Register {
    token: String,
    #[serde(default = "ios")]
    platform: String,
    environment: String,
    #[serde(default)]
    bundle_id: String,
    #[serde(default)]
    name: String,
    kinds: Option<Vec<String>>,
    quiet_hours: Option<QuietHours>,
}

fn ios() -> String {
    "ios".into()
}

/// `POST /api/devices`: register a phone, or refresh it (an upsert by token; call it on every launch).
pub async fn register(State(state): State<AppState>, Json(b): Json<Register>) -> ApiResult<Value> {
    let token = b.token.trim().to_lowercase();
    if !apns::valid_token(&token) {
        return Err(bad("token must be the APNs device token in hex"));
    }
    if !["development", "production"].contains(&b.environment.as_str()) {
        return Err(bad("environment must be development (Xcode builds) or production (TestFlight, App Store)"));
    }
    if b.platform != "ios" {
        return Err(bad("platform must be ios"));
    }
    let kinds = b.kinds.map(check_kinds).transpose()?;
    check_quiet(&b.quiet_hours)?;
    let existing = state.db.get_device(&token)?;
    let d = Device {
        token: token.clone(),
        platform: b.platform,
        environment: b.environment,
        bundle_id: b.bundle_id.trim().to_string(),
        name: b.name.trim().chars().take(80).collect(),
        // A refresh without kinds or quiet hours keeps what the device chose before.
        kinds: kinds.or_else(|| existing.as_ref().map(|d| d.kinds.clone())).unwrap_or_else(|| DEFAULT_KINDS.iter().map(|k| k.to_string()).collect()),
        quiet_hours: b.quiet_hours.or_else(|| existing.as_ref().and_then(|d| d.quiet_hours.clone())),
        created_at: existing.as_ref().map(|d| d.created_at.clone()).unwrap_or_else(now),
        last_seen_at: now(),
    };
    state.db.save_device(&d)?;
    Ok(Json(json!(d)))
}

/// `GET /api/devices`
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Value> {
    same_site_read(&headers)?;
    Ok(Json(json!(state.db.list_devices()?)))
}

#[derive(Deserialize)]
pub struct Update {
    kinds: Option<Vec<String>>,
    /// `null` clears quiet hours; leaving the field out keeps them.
    #[serde(default, deserialize_with = "some")]
    quiet_hours: Option<Option<QuietHours>>,
    name: Option<String>,
}

fn some<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<QuietHours>>, D::Error> {
    Option::<QuietHours>::deserialize(d).map(Some)
}

/// `PATCH /api/devices/{token}`: which kinds to push, quiet hours, name.
pub async fn update(State(state): State<AppState>, Path(token): Path<String>, Json(b): Json<Update>) -> ApiResult<Value> {
    let mut d = state.db.get_device(&token.to_lowercase())?.ok_or_else(not_found)?;
    if let Some(k) = b.kinds {
        d.kinds = check_kinds(k)?;
    }
    if let Some(q) = b.quiet_hours {
        check_quiet(&q)?;
        d.quiet_hours = q;
    }
    if let Some(n) = b.name {
        d.name = n.trim().chars().take(80).collect();
    }
    state.db.save_device(&d)?;
    Ok(Json(json!(d)))
}

/// `DELETE /api/devices/{token}`: the app signed out, or notifications were turned off.
pub async fn remove(State(state): State<AppState>, Path(token): Path<String>) -> ApiResult<Value> {
    if !state.db.delete_device(&token.to_lowercase())? {
        return Err(not_found());
    }
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/devices/{token}/test`: a test push, to check the whole chain.
pub async fn test(State(state): State<AppState>, Path(token): Path<String>) -> ApiResult<Value> {
    let d = state.db.get_device(&token.to_lowercase())?.ok_or_else(not_found)?;
    let push = Push {
        title: "Loft".into(),
        body: "Test notification: push works.".into(),
        badge: None,
        thread_id: Some("loft-test".into()),
        time_sensitive: false,
        collapse_id: Some("loft-test".into()),
        custom: json!({ "type": "test" }),
    };
    match apns::send(&state, &d, &push).await {
        Ok(apns::Outcome::Sent) => Ok(Json(json!({ "ok": true }))),
        Ok(apns::Outcome::Unregistered) => Err(ApiError(StatusCode::GONE, "Apple says this phone is no longer registered; it was removed. Open the app to register again.".into())),
        Err(e) => Err(ApiError(StatusCode::BAD_GATEWAY, e.to_string())),
    }
}

/// `GET /api/push`: which path pushes take (`own_key`, straight to Apple; `relay`, the Loft push relay;
/// `none`, no push) and the key's ids. Never the key or the relay key, only whether each is there.
pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Value> {
    same_site_read(&headers)?;
    let s = state.settings.get().push;
    let key = apns::key_present();
    let route = apns::route(&state);
    Ok(Json(json!({
        "path": route.name(),
        "relay_key": apns::relay_key_present(),
        // Sign in was pressed and the code isn't allowed yet.
        "signing_in": crate::signin::waiting(),
        // The addresses in use (saved, else the environment), and the saved ones the Settings fields show.
        "relay_url": apns::relay_url(&state),
        "portal_url": apns::portal_url(&state),
        "relay_address": s.relay_url,
        "portal_address": s.portal_url,
        "configured": route == apns::Route::OwnKey,
        "key_present": key,
        "key_id": s.key_id,
        "team_id": s.team_id,
        "bundle_id": s.bundle_id,
        "devices": state.db.list_devices()?.len(),
    })))
}

#[derive(Deserialize)]
pub struct Configure {
    key_id: Option<String>,
    team_id: Option<String>,
    bundle_id: Option<String>,
    /// The `.p8` file's text. Stored owner-only outside the repo; never echoed back.
    key_p8: Option<String>,
    /// Removes the stored key.
    #[serde(default)]
    remove_key: bool,
    /// The relay key from the push portal. Stored owner-only like the `.p8`; never echoed back.
    relay_key: Option<String>,
    /// Removes the stored relay key.
    #[serde(default)]
    remove_relay_key: bool,
    /// The push relay's and portal's addresses: https only, "" clears.
    relay_url: Option<String>,
    portal_url: Option<String>,
}

/// `PUT /api/push`: key id, team id, bundle id, and optionally the key itself or the relay key.
pub async fn configure(State(state): State<AppState>, headers: HeaderMap, Json(b): Json<Configure>) -> ApiResult<Value> {
    let ids = |s: &str| s.len() == 10 && s.bytes().all(|c| c.is_ascii_alphanumeric());
    let mut s = state.settings.get();
    if let Some(k) = b.key_id {
        let k = k.trim().to_uppercase();
        if !k.is_empty() && !ids(&k) {
            return Err(bad("The key id is the 10 letters and digits shown next to the key in the Apple Developer portal."));
        }
        s.push.key_id = k;
    }
    if let Some(t) = b.team_id {
        let t = t.trim().to_uppercase();
        if !t.is_empty() && !ids(&t) {
            return Err(bad("The team id is 10 letters and digits (Apple Developer portal → Membership details)."));
        }
        s.push.team_id = t;
    }
    if let Some(u) = b.relay_url {
        s.push.relay_url = apns::check_address(&u).map_err(|e| bad(format!("Relay address: {e}")))?;
    }
    if let Some(u) = b.portal_url {
        s.push.portal_url = apns::check_address(&u).map_err(|e| bad(format!("Portal address: {e}")))?;
    }
    if let Some(bid) = b.bundle_id {
        s.push.bundle_id = bid.trim().to_string();
    }
    if let Some(pem) = b.key_p8.filter(|p| !p.trim().is_empty()) {
        apns::store_key(&pem).map_err(|e| bad(e.to_string()))?;
    }
    if b.remove_key {
        apns::remove_key().map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if let Some(k) = b.relay_key.filter(|k| !k.trim().is_empty()) {
        apns::store_relay_key(&k).map_err(|e| bad(e.to_string()))?;
    }
    if b.remove_relay_key {
        apns::remove_relay_key().map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    state.settings.save(s).map_err(|e| bad(e.to_string()))?;
    status(State(state), headers).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::pkcs8::DecodePrivateKey;

    fn reg(token: &str) -> Json<Register> {
        Json(serde_json::from_value(json!({ "token": token, "environment": "production", "bundle_id": "com.example.loft", "name": "Sam's iPhone" })).unwrap())
    }

    #[tokio::test]
    async fn the_status_says_which_path_pushes_take_and_never_the_secrets() {
        crate::apns::tests::fake_apns();
        let st = crate::tasks::tests::state();
        let get = || async { status(State(st.clone()), HeaderMap::new()).await.unwrap().0 };
        let put = |p: Value| configure(State(st.clone()), HeaderMap::new(), Json(serde_json::from_value::<Configure>(p).unwrap()));

        // The test install holds a relay key and no key ids: the Loft relay.
        let s = get().await;
        assert_eq!((s["path"].clone(), s["relay_key"].clone(), s["configured"].clone()), (json!("relay"), json!(true), json!(false)));
        // Its own key and both ids win over the relay key.
        let Json(s) = put(json!({ "key_id": "ABC123DEFG", "team_id": "TEAM123456", "key_p8": crate::apns::tests::test_key_pem() })).await.unwrap();
        assert_eq!((s["path"].clone(), s["configured"].clone()), (json!("own_key"), json!(true)));
        let Json(s) = put(json!({ "key_id": "", "team_id": "" })).await.unwrap();
        assert_eq!(s["path"], json!("relay"));

        // Pasting a relay key: one that isn't shaped like one is refused, a good one is kept and never shown.
        let err = put(json!({ "relay_key": "not-a-key" })).await.unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        let Json(s) = put(json!({ "relay_key": crate::apns::tests::RELAY_KEY })).await.unwrap();
        assert_eq!(s["relay_key"], json!(true));
        let text = s.to_string();
        assert!(!text.contains("lrk_") && !text.contains("PRIVATE KEY"), "{text}");
        // No portal yet: no link to show.
        assert_eq!(s["portal_url"], Value::Null);
    }

    #[test]
    fn the_portal_link_comes_from_the_environment_when_nothing_is_saved() {
        let st = crate::tasks::tests::state();
        // Set and cleared in one test, so no other test sees it.
        unsafe { std::env::set_var("LOFT_PORTAL_URL", " https://loft-push-relay.vercel.app/account ") };
        assert_eq!(crate::apns::portal_url(&st).as_deref(), Some("https://loft-push-relay.vercel.app/account"));
        let mut s = st.settings.get();
        s.push.portal_url = "https://portal.example.com".into();
        st.settings.save(s).unwrap();
        assert_eq!(crate::apns::portal_url(&st).as_deref(), Some("https://portal.example.com"), "the saved address wins");
        let mut s = st.settings.get();
        s.push.portal_url = String::new();
        st.settings.save(s).unwrap();
        unsafe { std::env::set_var("LOFT_PORTAL_URL", "javascript:alert(1)") };
        assert_eq!(crate::apns::portal_url(&st), None);
        unsafe { std::env::remove_var("LOFT_PORTAL_URL") };
        assert_eq!(crate::apns::portal_url(&st), None, "empty by default");
    }

    #[tokio::test]
    async fn relay_and_portal_addresses_are_saved_checked_and_used_without_a_restart() {
        crate::apns::tests::fake_apns();
        let st = crate::tasks::tests::state();
        let put = |p: Value| configure(State(st.clone()), HeaderMap::new(), Json(serde_json::from_value::<Configure>(p).unwrap()));

        // Not https, or with a user name or password: refused, and nothing is saved.
        for (bad, why) in [
            ("http://relay.example.com", "must start with https://"),
            ("ftp://relay.example.com", "must start with https://"),
            ("https://me:secret@relay.example.com", "user name or password"),
            ("https://token@relay.example.com", "user name or password"),
            ("relay.example.com", "isn't a web address"),
        ] {
            let err = put(json!({ "relay_url": bad })).await.unwrap_err();
            assert_eq!(err.0, StatusCode::BAD_REQUEST, "{bad}");
            assert!(err.1.starts_with("Relay address: ") && err.1.contains(why), "{bad}: {}", err.1);
        }
        assert!(put(json!({ "portal_url": "http://portal.example.com" })).await.unwrap_err().1.starts_with("Portal address: "));
        assert_eq!((st.settings.get().push.relay_url, st.settings.get().push.portal_url), (String::new(), String::new()));

        // Saved and reported (no key); a trailing slash goes.
        let Json(s) = put(json!({ "relay_url": " https://relay.example.com/ ", "portal_url": "https://relay.example.com/account" })).await.unwrap();
        assert_eq!((&s["relay_address"], &s["relay_url"], &s["portal_address"], &s["portal_url"]), (&json!("https://relay.example.com"), &json!("https://relay.example.com"), &json!("https://relay.example.com/account"), &json!("https://relay.example.com/account")));
        assert!(!s.to_string().contains("lrk_"));

        // The saved address wins over LOFT_RELAY_URL (here the fake relay) at once: an address with nothing behind it
        // makes the test push fail; clearing it goes back to the environment's relay, with no restart.
        let phone = format!("{}0c", "ef".repeat(31));
        let _ = register(State(st.clone()), reg(&phone)).await.unwrap();
        let _ = put(json!({ "relay_url": "http://127.0.0.1:9" })).await.unwrap();
        let err = test(State(st.clone()), Path(phone.clone())).await.unwrap_err();
        assert!(err.1.contains("The push relay is unreachable"), "{}", err.1);
        let Json(s) = put(json!({ "relay_url": "" })).await.unwrap();
        assert_eq!(s["relay_address"], json!(""));
        assert!(test(State(st.clone()), Path(phone)).await.is_ok(), "the environment's relay again");

        // The general settings can't change them, even when a client sends the push section.
        let _ = put(json!({ "portal_url": "https://portal.example.com" })).await.unwrap();
        let mut general = serde_json::to_value(st.settings.get()).unwrap();
        general["push"]["relay_url"] = json!("https://evil.example.com");
        general["push"]["portal_url"] = json!("https://evil.example.com");
        let _ = crate::api::put_settings(State(st.clone()), Json(general)).await.ok().unwrap();
        assert_eq!((st.settings.get().push.relay_url, st.settings.get().push.portal_url), (String::new(), "https://portal.example.com".to_string()));

        // No agent tool can set them.
        let tools = crate::mcp::tools(&st).to_string();
        assert!(!tools.contains("relay_url") && !tools.contains("portal_url") && !tools.contains("relay_key"));
    }

    #[tokio::test]
    async fn register_configure_and_push_through_a_fake_apns() {
        let st = crate::tasks::tests::state();
        let seen = crate::apns::tests::fake_apns();
        let dir = crate::apns::key_dir();
        let mut s = st.settings.get();
        s.projects_dir = st.cfg.storage_dir.parent().unwrap().to_path_buf();
        st.settings.save(s).unwrap();
        let good = "ab".repeat(32);
        let gone = format!("dead{}", "cd".repeat(30));

        // Registering: validated, defaults, and an upsert that keeps choices.
        assert!(register(State(st.clone()), reg("xyz")).await.is_err());
        let Json(d) = register(State(st.clone()), reg(&good)).await.unwrap();
        assert_eq!(d["kinds"], json!(DEFAULT_KINDS));
        let _ = update(State(st.clone()), Path(good.clone()), Json(serde_json::from_value(json!({ "kinds": ["permission"], "quiet_hours": { "start": "22:00", "end": "07:00", "tz": "Europe/Lisbon", "utc_offset_minutes": 60 } })).unwrap())).await.unwrap();
        let Json(d) = register(State(st.clone()), reg(&good)).await.unwrap();
        assert_eq!((d["kinds"].clone(), d["quiet_hours"]["start"].clone()), (json!(["permission"]), json!("22:00")));
        assert!(update(State(st.clone()), Path(good.clone()), Json(serde_json::from_value(json!({ "kinds": ["everything"] })).unwrap())).await.is_err());
        // App builds from before rounds still ask for grill_question; it's stored under the new name.
        let Json(d) = update(State(st.clone()), Path(good.clone()), Json(serde_json::from_value(json!({ "kinds": ["permission", "grill_question"] })).unwrap())).await.unwrap();
        assert_eq!(d["kinds"], json!(["permission", "grill_round"]));
        let _ = update(State(st.clone()), Path(good.clone()), Json(serde_json::from_value(json!({ "kinds": ["permission"] })).unwrap())).await.unwrap();
        let _ = register(State(st.clone()), reg(&gone)).await.unwrap();

        // No key yet: the test push goes through the relay, not to Apple.
        assert!(test(State(st.clone()), Path(good.clone())).await.is_ok());
        let relayed = crate::apns::tests::relayed();
        assert!(relayed.lock().unwrap().pushes.iter().any(|(_, b)| b["device_token"] == good && b["payload"]["type"] == "test"));
        assert!(!seen.lock().unwrap().iter().any(|(t, ..)| *t == good));

        // Configure: a bad key is refused, a real P-256 .p8 is stored owner-only and never echoed.
        let cfg = |key: Option<String>| Json(serde_json::from_value::<Configure>(json!({ "key_id": "abc123defg", "team_id": "TEAM123456", "key_p8": key })).unwrap());
        assert!(configure(State(st.clone()), HeaderMap::new(), cfg(Some("-----BEGIN PRIVATE KEY-----\nnope\n-----END PRIVATE KEY-----".into()))).await.is_err());
        let pem = crate::apns::tests::test_key_pem();
        let Json(status) = configure(State(st.clone()), HeaderMap::new(), cfg(Some(pem.clone()))).await.unwrap();
        assert_eq!((status["configured"].clone(), status["key_id"].clone(), status["devices"].clone()), (json!(true), json!("ABC123DEFG"), json!(2)));
        assert!(!status.to_string().contains("PRIVATE KEY"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join("AuthKey.p8")).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);

        // Saving the general settings (which don't send `push`) keeps the push setup.
        let mut general = serde_json::to_value(st.settings.get()).unwrap();
        general.as_object_mut().unwrap().remove("push");
        let _ = crate::api::put_settings(State(st.clone()), Json(general)).await.ok().unwrap();
        assert_eq!(st.settings.get().push.key_id, "ABC123DEFG");

        // A test push reaches the fake APNs with the right headers and a JWT that verifies.
        let Json(r) = test(State(st.clone()), Path(good.clone())).await.unwrap();
        assert_eq!(r, json!({ "ok": true }));
        let (token, headers, body) = seen.lock().unwrap().iter().rev().find(|(t, ..)| *t == good).cloned().unwrap();
        assert_eq!(token, good);
        assert_eq!((headers["apns-topic"].clone(), headers["apns-push-type"].clone(), headers["apns-priority"].clone()), (json!("com.example.loft"), json!("alert"), json!("5")));
        assert_eq!((body["aps"]["alert"]["title"].clone(), body["type"].clone()), (json!("Loft"), json!("test")));
        let jwt = headers["authorization"].as_str().unwrap().strip_prefix("bearer ").or_else(|| headers["authorization"].as_str().unwrap().strip_prefix("Bearer ")).unwrap().to_string();
        let parts: Vec<&str> = jwt.split('.').collect();
        let key = p256::ecdsa::SigningKey::from_pkcs8_pem(&pem).unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[2]).unwrap()).unwrap();
        p256::ecdsa::VerifyingKey::from(&key).verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).unwrap();

        // Apple says 410: the device is removed.
        let err = test(State(st.clone()), Path(gone.clone())).await.unwrap_err();
        assert_eq!(err.0, StatusCode::GONE);
        assert!(st.db.get_device(&gone).unwrap().is_none());

        // Other websites can't list devices; removing works once.
        let mut cross = HeaderMap::new();
        cross.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(list(State(st.clone()), cross).await.is_err());
        assert!(remove(State(st.clone()), Path(good.clone())).await.is_ok());
        assert!(remove(State(st.clone()), Path(good)).await.is_err());
    }
}
