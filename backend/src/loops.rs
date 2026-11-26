//! `/loop <interval> <prompt>`: Loft sends the prompt to the agent every interval until it's stopped.
//! The loop lives on the agent, so it survives a restart. Quiet hours delay it, and while the agent is still busy
//! with the last run nothing is queued.
use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agents::{self, emit_agent, is_running};
use crate::api::{bad, not_found, ApiResult};
use crate::db::Agent;
use crate::schedule;
use crate::state::AppState;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Loop {
    pub prompt: String,
    pub interval_minutes: i64,
    /// When the prompt is next sent (RFC 3339).
    pub next_at: String,
}

/// "10m", "2h" or "1d" as minutes: at least a minute, at most 30 days.
pub fn parse_interval(s: &str) -> Option<i64> {
    let s = s.trim();
    let unit = s.chars().last()?;
    let n: i64 = s[..s.len() - unit.len_utf8()].parse().ok()?;
    let minutes = match unit {
        'm' => n,
        'h' => n.checked_mul(60)?,
        'd' => n.checked_mul(24 * 60)?,
        _ => return None,
    };
    (1..=30 * 24 * 60).contains(&minutes).then_some(minutes)
}

/// An interval from `now`, pushed past quiet hours.
fn next_at(a: &Agent, minutes: i64, now: DateTime<Utc>) -> String {
    let t = (now + chrono::Duration::minutes(minutes)).with_timezone(&Local);
    schedule::next_wake(&a.quiet_hours, t).with_timezone(&Utc).to_rfc3339()
}

fn set_next(state: &AppState, id: &str, next: String) {
    let r = state.db.update_agent(id, |a| {
        if let Some(l) = a.repeat.as_mut() {
            l.next_at = next;
        }
    });
    if let Ok(Some(a)) = r {
        emit_agent(state, &a);
    }
}

/// Sends the prompt of every loop due at `now`. Returns how many it sent.
pub fn tick(state: &AppState, now: DateTime<Utc>) -> usize {
    let Ok(list) = state.db.list_agents() else { return 0 };
    let mut sent = 0;
    for a in list {
        let Some(l) = &a.repeat else { continue };
        let due = DateTime::parse_from_rfc3339(&l.next_at).map(|t| t.with_timezone(&Utc));
        if !due.is_ok_and(|t| t <= now) {
            continue;
        }
        // Came due inside quiet hours (they changed, or the Mac slept): wait them out.
        if let Some(until) = schedule::quiet_until(&a.quiet_hours, now.with_timezone(&Local)) {
            set_next(state, &a.id, until.with_timezone(&Utc).to_rfc3339());
            continue;
        }
        // Still busy with the last run: it goes out once the agent is free.
        if is_running(state, &a.id) {
            continue;
        }
        set_next(state, &a.id, next_at(&a, l.interval_minutes, now));
        agents::enqueue_with(state, &a.id, Some(l.prompt.clone()), json!({ "loop": true }), "loop");
        sent += 1;
    }
    sent
}

/// One timer for both `/loop` and `/schedule`.
pub async fn scheduler(state: AppState) {
    let mut tick_every = tokio::time::interval(std::time::Duration::from_secs(15));
    loop {
        tick_every.tick().await;
        // Like the wake-ups, held while an update waits for idle.
        if !crate::updater::pending(&state) {
            let now = Utc::now();
            tick(&state, now);
            crate::schedules::tick(&state, now);
        }
    }
}

#[derive(Deserialize)]
pub struct Start {
    interval: String,
    prompt: String,
}

/// Starts (or replaces) the agent's loop and sends the prompt now.
pub async fn start(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<Start>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let minutes = parse_interval(&b.interval).ok_or_else(|| bad("The interval is a number and m, h or d (10m, 2h, 1d), at least 1m."))?;
    let prompt = b.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(bad("Add the prompt to repeat: /loop 10m check the deploy"));
    }
    let a = state
        .db
        .update_agent(&id, |a| {
            let next_at = next_at(a, minutes, Utc::now());
            a.repeat = Some(Loop { prompt: prompt.clone(), interval_minutes: minutes, next_at });
        })?
        .ok_or_else(not_found)?;
    emit_agent(&state, &a);
    agents::enqueue_with(&state, &id, Some(prompt), json!({ "loop": true }), "loop");
    Ok(Json(json!({ "ok": true })))
}

pub async fn stop(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let a = state.db.update_agent(&id, |a| a.repeat = None)?.ok_or_else(not_found)?;
    emit_agent(&state, &a);
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::QuietWindow;
    use std::sync::Arc;
    use tokio::sync::Notify;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        // 2026-10-12 is a Monday.
        chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 10, 12, h, m, 0).unwrap().with_timezone(&Utc)
    }

    fn user_texts(st: &AppState, id: &str) -> usize {
        st.db.list_messages(id, 400, None).unwrap().iter().filter(|m| m.role == "user" && m.content == "check the deploy").count()
    }

    fn loop_of(a: &Agent, minutes: i64, next: DateTime<Utc>) -> Option<Loop> {
        let _ = a;
        Some(Loop { prompt: "check the deploy".into(), interval_minutes: minutes, next_at: next.to_rfc3339() })
    }

    #[test]
    fn intervals() {
        assert_eq!([parse_interval("10m"), parse_interval("2h"), parse_interval("1d")], [Some(10), Some(120), Some(1440)]);
        for bad in ["", "m", "0m", "10", "10s", "-5m", "1.5h", "99999d", "10 m"] {
            assert_eq!(parse_interval(bad), None, "{bad}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_loop_sends_when_due_then_again_every_interval_and_stops() {
        let (st, a) = agents::tests::fake_claude_state();
        let t0 = at(10, 0);
        st.db.update_agent(&a.id, |x| x.repeat = loop_of(x, 10, t0 + chrono::Duration::minutes(10))).unwrap();

        assert_eq!(tick(&st, t0 + chrono::Duration::minutes(9)), 0, "not yet");
        assert_eq!(tick(&st, t0 + chrono::Duration::minutes(10)), 1);
        assert_eq!(user_texts(&st, &a.id), 1);
        let next = st.db.get_agent(&a.id).unwrap().unwrap().repeat.unwrap().next_at;
        assert_eq!(next, (t0 + chrono::Duration::minutes(20)).to_rfc3339());

        // Wait for the run to end, so the next one isn't held behind it.
        agents::tests::until("the run to end", 60, || !is_running(&st, &a.id)).await;
        assert_eq!(tick(&st, t0 + chrono::Duration::minutes(20)), 1);
        assert_eq!(user_texts(&st, &a.id), 2);

        st.db.update_agent(&a.id, |x| x.repeat = None).unwrap();
        agents::tests::until("the run to end", 60, || !is_running(&st, &a.id)).await;
        assert_eq!(tick(&st, t0 + chrono::Duration::minutes(40)), 0, "stopped");
    }

    #[test]
    fn a_busy_agent_gets_nothing_queued() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let t0 = at(10, 0);
        st.db.update_agent(&a.id, |x| x.repeat = loop_of(x, 10, t0)).unwrap();
        st.rt.lock().unwrap().running.insert(a.id.clone(), Arc::new(Notify::new()));
        assert_eq!(tick(&st, t0 + chrono::Duration::minutes(5)), 0);
        assert_eq!(user_texts(&st, &a.id), 0);
        assert!(st.rt.lock().unwrap().queued.get(&a.id).is_none_or(|q| q.is_empty()));
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().repeat.unwrap().next_at, t0.to_rfc3339(), "still due for when it's free");
    }

    #[test]
    fn quiet_hours_delay_it() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let quiet = vec![QuietWindow { days: vec![0], start: "09:00".into(), end: "17:00".into() }];
        let t0 = at(10, 0);
        st.db.update_agent(&a.id, |x| {
            x.quiet_hours = quiet;
            x.repeat = loop_of(x, 10, t0);
        })
        .unwrap();
        assert_eq!(tick(&st, t0), 0);
        assert_eq!(user_texts(&st, &a.id), 0);
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().repeat.unwrap().next_at, at(17, 0).to_rfc3339());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn starting_a_loop_replaces_the_one_running() {
        let (st, a) = agents::tests::fake_claude_state();
        let go = |interval: &str, prompt: &str| start(State(st.clone()), Path(a.id.clone()), Json(Start { interval: interval.into(), prompt: prompt.into() }));
        let _ = go("10m", "first").await.ok().unwrap();
        let _ = go("2h", "second").await.ok().unwrap();
        let l = st.db.get_agent(&a.id).unwrap().unwrap().repeat.unwrap();
        assert_eq!((l.prompt.as_str(), l.interval_minutes), ("second", 120));
        assert!(go("5", "x").await.is_err() && go("5m", "  ").await.is_err());
    }

    #[test]
    fn a_new_loop_is_pushed_past_quiet_hours() {
        let st = crate::tasks::tests::state();
        let mut a = crate::tasks::tests::agent(&st, "Worker");
        a.quiet_hours = vec![QuietWindow { days: vec![0], start: "09:00".into(), end: "17:00".into() }];
        assert_eq!(next_at(&a, 30, at(8, 45)), at(17, 0).to_rfc3339());
        assert_eq!(next_at(&a, 30, at(18, 0)), at(18, 30).to_rfc3339());
    }
}
