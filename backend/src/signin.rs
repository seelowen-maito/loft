//! Signing in to the push relay from Settings (ADR 0003): the relay gives a one-time code, the user allows it on the
//! push portal, and this backend, polling the relay, receives a relay key once and stores it like a pasted one.
//! The key never passes through the browser; only the portal link does.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::api::{ApiError, bad};
use crate::apns;
use crate::state::AppState;

/// The code being waited on, if any. A newer Sign in replaces it, and the older wait stops.
static WAITING: Mutex<Option<String>> = Mutex::new(None);

pub fn waiting() -> bool {
    WAITING.lock().unwrap().is_some()
}

/// `POST /api/push/signin`: asks the relay for a code and returns the portal link that allows it. The key is
/// collected in the background, so approving from a phone works too.
pub async fn start(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let (Some(relay), Some(portal)) = (apns::relay_url(&state), apns::portal_url(&state)) else {
        return Err(bad("Set the relay and portal addresses first (or paste a relay key)."));
    };
    let res = state.http.post(format!("{relay}/api/pair")).timeout(Duration::from_secs(15)).send().await.map_err(|_| bad("The push relay is unreachable."))?;
    if !res.status().is_success() {
        return Err(bad(format!("The push relay didn't give a code ({}).", res.status().as_u16())));
    }
    let v = res.json::<Value>().await.unwrap_or(Value::Null);
    let (Some(code), Some(secret)) = (v["code"].as_str().map(str::to_string), v["secret"].as_str().map(str::to_string)) else {
        return Err(bad("The push relay's answer had no code."));
    };
    // The code goes into a link: only the shape the relay makes is accepted.
    if code.len() != 22 || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(bad("The push relay's code looked wrong."));
    }
    let seconds = v["expires_in"].as_u64().unwrap_or(600).clamp(60, 900);
    *WAITING.lock().unwrap() = Some(code.clone());
    let url = format!("{portal}/connect?code={code}");
    tokio::spawn(async move {
        if let Err(e) = wait_for_key(&state, &relay, &code, &secret, Duration::from_secs(2), Duration::from_secs(seconds)).await {
            tracing::warn!("push sign-in: {e:#}");
        }
        let mut w = WAITING.lock().unwrap();
        if w.as_deref() == Some(code.as_str()) {
            *w = None;
        }
    });
    Ok(Json(json!({ "url": url })))
}

/// Polls the relay for `code` (proving it with `secret`, which never goes into the link) every `every` until it gives the key (stored), forgets the code, or `limit` passes.
async fn wait_for_key(state: &AppState, relay: &str, code: &str, secret: &str, every: Duration, limit: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(every).await;
        if WAITING.lock().unwrap().as_deref() != Some(code) {
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
        let relay = apns::tests::fake_apns_relay();
        let st = crate::tasks::tests::state();
        let mut s = st.settings.get();
        s.push.portal_url = "https://portal.example.com".into();
        st.settings.save(s).unwrap();
        let Json(r) = start(State(st.clone())).await.unwrap();
        let url = r["url"].as_str().unwrap();
        let code = url.strip_prefix("https://portal.example.com/connect?code=").expect(url).to_string();
        let secret = format!("s-{code}");
        assert!(!url.contains(&secret), "the secret stays out of the link");
        assert_eq!(r.as_object().unwrap().len(), 1, "only the link comes back, never a key");
        assert!(waiting());
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
    async fn without_addresses_or_a_reachable_relay_signing_in_says_why() {
        apns::tests::fake_apns();
        let st = crate::tasks::tests::state();
        let mut s = st.settings.get();
        s.push.relay_url = "http://127.0.0.1:9".into();
        s.push.portal_url = String::new();
        st.settings.save(s.clone()).unwrap();
        assert!(start(State(st.clone())).await.unwrap_err().1.contains("addresses"));
        s.push.portal_url = "https://portal.example.com".into();
        st.settings.save(s).unwrap();
        assert!(start(State(st.clone())).await.unwrap_err().1.contains("unreachable"));
    }
}
