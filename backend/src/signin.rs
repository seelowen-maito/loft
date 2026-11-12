//! Signing in to the push relay from Settings (ADR 0003): the relay gives a one-time code, the user allows it on the
//! push portal (at the relay's own address), and this backend, polling the relay, receives a relay key once and stores it like a pasted one.
//! The key never passes through the browser; only the portal link does. The portal's Allow page asks for a short
//! confirmation code that only this Loft shows, so a portal link sent to someone else gets the sender nothing.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::api::{ApiError, bad};
use crate::apns;
use crate::state::AppState;

/// A sign-in code being waited on, with its confirmation code (none from a relay that doesn't give one).
struct Waiting {
    code: String,
    confirm: Option<String>,
}

/// The code being waited on, if any. A newer Sign in replaces it, and the older wait stops.
static WAITING: Mutex<Option<Waiting>> = Mutex::new(None);

pub fn waiting() -> bool {
    WAITING.lock().unwrap().is_some()
}

/// The confirmation code of the sign-in being waited on, for Settings to show.
pub fn confirmation() -> Option<String> {
    WAITING.lock().unwrap().as_ref().and_then(|w| w.confirm.clone())
}

fn waiting_on(code: &str) -> bool {
    WAITING.lock().unwrap().as_ref().is_some_and(|w| w.code == code)
}

/// The relay address that last answered that its portal isn't configured: no one can allow a code there.
static NO_PORTAL: Mutex<Option<String>> = Mutex::new(None);

/// Whether Settings offers Sign in: there is a relay, and it hasn't said its portal isn't configured.
pub fn offered(state: &AppState) -> bool {
    apns::relay_url(state).is_some_and(|r| NO_PORTAL.lock().unwrap().as_deref() != Some(r.as_str()))
}

/// `POST /api/push/signin`: asks the relay for a code and returns the portal link that allows it, with the
/// confirmation code the user types there. The key is collected in the background, so approving from a phone works too.
pub async fn start(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let Some(relay) = apns::relay_url(&state) else {
        return Err(bad("Set the relay address first (or paste a relay key)."));
    };
    let res = state.http.post(format!("{relay}/api/pair")).timeout(Duration::from_secs(15)).send().await.map_err(|_| bad("The push relay is unreachable."))?;
    let ok = res.status().is_success();
    let status = res.status().as_u16();
    let v = res.json::<Value>().await.unwrap_or(Value::Null);
    if v["portal"] == json!(false) {
        *NO_PORTAL.lock().unwrap() = Some(relay);
        return Err(bad("This relay has no sign-in portal: paste a relay key from its operator."));
    }
    if !ok {
        return Err(bad(format!("The push relay didn't give a code ({status}).")));
    }
    let (Some(code), Some(secret)) = (v["code"].as_str().map(str::to_string), v["secret"].as_str().map(str::to_string)) else {
        return Err(bad("The push relay's answer had no code."));
    };
    // The code goes into a link: only the shape the relay makes is accepted.
    if code.len() != 22 || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(bad("The push relay's code looked wrong."));
    }
    // Shown on the page: only a short code of letters and digits.
    let confirm = v["confirm"].as_str().filter(|c| (4..=12).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())).map(str::to_string);
    let seconds = v["expires_in"].as_u64().unwrap_or(600).clamp(60, 900);
    *WAITING.lock().unwrap() = Some(Waiting { code: code.clone(), confirm: confirm.clone() });
    let url = format!("{relay}/connect?code={code}");
    tokio::spawn(async move {
        if let Err(e) = wait_for_key(&state, &relay, &code, &secret, Duration::from_secs(2), Duration::from_secs(seconds)).await {
            tracing::warn!("push sign-in: {e:#}");
        }
        let mut w = WAITING.lock().unwrap();
        if w.as_ref().is_some_and(|w| w.code == code) {
            *w = None;
        }
    });
    Ok(Json(json!({ "url": url, "confirm": confirm })))
}

/// Polls the relay for `code` (proving it with `secret`, which never goes into the link) every `every` until it gives the key (stored), forgets the code, or `limit` passes.
async fn wait_for_key(state: &AppState, relay: &str, code: &str, secret: &str, every: Duration, limit: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(every).await;
        if !waiting_on(code) {
            return Ok(());
        }
        // A failed poll is tried again until the code runs out.
        let Ok(res) = state.http.post(format!("{relay}/api/pair-poll")).timeout(Duration::from_secs(15)).json(&json!({ "code": code, "secret": secret })).send().await else { continue };
        match res.status().as_u16() {
            200 => {
                let v = res.json::<Value>().await.context("the relay's answer wasn't JSON")?;
                apns::store_relay_key(v["key"].as_str().unwrap_or("")).context("the relay's key wasn't usable")?;
                return Ok(());
            }
            202 => {}
            s => bail!("the relay no longer knows this code ({s})"),
        }
    }
    bail!("the code expired before it was allowed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[tokio::test]
    async fn the_code_flow_ends_with_the_relays_key_stored_once_and_never_returned() {
        // Only the relay address is set (LOFT_RELAY_URL): the portal's page is at that same address.
        let relay = apns::tests::fake_apns_relay();
        let st = crate::tasks::tests::state();
        let Json(r) = start(State(st.clone())).await.unwrap();
        let url = r["url"].as_str().unwrap();
        let code = url.strip_prefix(&format!("{relay}/connect?code=")).expect(url).to_string();
        let secret = format!("s-{code}");
        assert!(!url.contains(&secret), "the secret stays out of the link");
        assert_eq!(r.as_object().unwrap().len(), 2, "only the link and the confirmation code come back, never a key");
        // The confirmation code the portal asks for is shown in Settings while the sign-in waits.
        assert_eq!(r["confirm"], json!(apns::tests::confirm_of(&code)));
        assert!(waiting());
        let Json(p) = crate::devices::status(State(st.clone()), HeaderMap::new()).await.unwrap();
        assert_eq!((p["signing_in"].clone(), p["signin_code"].clone()), (json!(true), r["confirm"].clone()));
        // Waiting: nothing yet, and the wait goes on. Allowed: the key is stored and the wait ends.
        let fast = Duration::from_millis(20);
        assert!(wait_for_key(&st, &relay, &code, &secret, fast, Duration::from_millis(100)).await.is_err());
        apns::tests::approve_code(&code);
        wait_for_key(&st, &relay, &code, &secret, fast, Duration::from_secs(2)).await.unwrap();
        let key_file = apns::key_dir().join("relay-key");
        assert_eq!(std::fs::read_to_string(&key_file).unwrap(), apns::tests::RELAY_KEY);
        std::fs::remove_file(&key_file).unwrap();
        // The code is dead: a second wait is refused instead of getting a key again.
        assert!(format!("{:#}", wait_for_key(&st, &relay, &code, &secret, fast, Duration::from_secs(2)).await.unwrap_err()).contains("no longer knows"));
        assert!(!format!("{:?}", crate::devices::status(State(st.clone()), HeaderMap::new()).await.unwrap().0).contains("lrk_"));
    }

    #[tokio::test]
    async fn without_a_reachable_relay_signing_in_says_why() {
        apns::tests::fake_apns();
        let st = crate::tasks::tests::state();
        let mut s = st.settings.get();
        s.push.relay_url = "http://127.0.0.1:9".into();
        st.settings.save(s).unwrap();
        assert!(start(State(st.clone())).await.unwrap_err().1.contains("unreachable"));
    }

    #[tokio::test]
    async fn a_relay_without_a_portal_is_not_offered_sign_in_again() {
        let relay = apns::tests::fake_apns_relay();
        let st = crate::tasks::tests::state();
        let mut s = st.settings.get();
        s.push.relay_url = format!("{relay}/no-portal");
        st.settings.save(s).unwrap();
        assert!(offered(&st), "offered until the relay says otherwise");
        let err = start(State(st.clone())).await.unwrap_err();
        assert!(err.1.contains("paste a relay key"), "{}", err.1);
        assert!(!offered(&st));
        let Json(p) = crate::devices::status(State(st.clone()), HeaderMap::new()).await.unwrap();
        assert_eq!(p["signin"], json!(false));
        // Another address is offered again.
        let mut s = st.settings.get();
        s.push.relay_url = String::new();
        st.settings.save(s).unwrap();
        assert!(offered(&st));
    }
}
