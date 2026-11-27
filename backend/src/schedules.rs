//! `/schedule <when> <prompt>`: Loft sends the prompt to the agent once, or at each due time of a repeat.
//! Schedules live on the agent, so they survive a restart; one that came due while Loft was down is sent once on start.
//! Times are the Mac's local time. Quiet hours don't apply: the user named the time, unlike a `/loop`, which Loft paces.
use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agents::{self, emit_agent};
use crate::api::{bad, not_found, ApiResult};
use crate::state::AppState;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Schedule {
    pub id: String,
    pub prompt: String,
    /// "day" or "weekday" at `time` for a repeat; None for a one-off.
    #[serde(default)]
    pub every: Option<String>,
    /// "HH:MM" local, for a repeat.
    #[serde(default)]
    pub time: Option<String>,
    /// When the prompt is next sent (RFC 3339).
    pub next_at: String,
}

const EXAMPLES: &str = "Use /schedule <when> <prompt>, with when like: tomorrow 9:00, today 17:30, in 2h (m, h or d), every day 8:00 or every weekday 9:30.";

fn parse_time(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.split_once(':')?;
    if m.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// The first `time` on or after `from`'s day that is after `from` and, for "weekday", not a weekend.
fn next_occurrence(every: &str, time: NaiveTime, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let start = from.with_timezone(&Local).date_naive();
    (0..9).find_map(|d| {
        let day = start + Duration::days(d);
        if every == "weekday" && matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            return None;
        }
        let t = Local.from_local_datetime(&day.and_time(time)).earliest()?.with_timezone(&Utc);
        (t > from).then_some(t)
    })
}

/// "tomorrow 9:00 run the tests" as a schedule (without an id): the time words, then the prompt.
pub fn parse(text: &str, now: DateTime<Utc>) -> Result<Schedule, String> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    // The text after the first n words, as typed (the prompt may span lines).
    let rest = |n: usize| (0..n).fold(text.trim(), |t, _| t.split_once(char::is_whitespace).map_or("", |(_, r)| r.trim_start())).trim().to_string();
    let bad = || EXAMPLES.to_string();
    let (n, every, time, next_at) = match words.as_slice() {
        ["in", d, ..] => {
            let minutes = crate::loops::parse_interval(d).ok_or_else(bad)?;
            (2, None, None, now + Duration::minutes(minutes))
        }
        [day @ ("today" | "tomorrow"), t, ..] => {
            let time = parse_time(t).ok_or_else(bad)?;
            let date = now.with_timezone(&Local).date_naive();
            let date = if *day == "tomorrow" { date + Duration::days(1) } else { date };
            let at = Local.from_local_datetime(&date.and_time(time)).earliest().ok_or_else(bad)?.with_timezone(&Utc);
            if at <= now {
                return Err("That time has already passed today. Pick a later time, or use tomorrow.".into());
            }
            (2, None, None, at)
        }
        ["every", kind @ ("day" | "weekday"), t, ..] => {
            let time = parse_time(t).ok_or_else(bad)?;
            (3, Some(kind.to_string()), Some(t.to_string()), next_occurrence(kind, time, now).ok_or_else(bad)?)
        }
        _ => return Err(bad()),
    };
    let prompt = rest(n);
    if prompt.is_empty() {
        return Err(format!("Add the prompt to send. {EXAMPLES}"));
    }
    let time = time.map(|t| parse_time(&t).map(|t| t.format("%H:%M").to_string()).unwrap_or_default());
    Ok(Schedule { id: String::new(), prompt, every, time, next_at: next_at.to_rfc3339() })
}

/// Sends the prompt of every schedule due at `now`: a one-off is removed, a repeat moves to its next time.
/// Returns how many it sent.
pub fn tick(state: &AppState, now: DateTime<Utc>) -> usize {
    let Ok(list) = state.db.list_agents() else { return 0 };
    let mut sent = 0;
    for a in list {
        let due: Vec<Schedule> = a
            .schedules
            .iter()
            .filter(|s| DateTime::parse_from_rfc3339(&s.next_at).is_ok_and(|t| t.with_timezone(&Utc) <= now))
            .cloned()
            .collect();
        if due.is_empty() {
            continue;
        }
        let r = state.db.update_agent(&a.id, |a| {
            a.schedules.retain_mut(|s| {
                if !due.iter().any(|d| d.id == s.id) {
                    return true;
                }
                let next = s.time.as_deref().and_then(parse_time).zip(s.every.as_deref()).and_then(|(t, e)| next_occurrence(e, t, now));
                match next {
                    Some(n) => {
                        s.next_at = n.to_rfc3339();
                        true
                    }
                    None => false,
                }
            });
        });
        // Only a schedule that was updated is sent, or a failed write would send it again on every tick.
        let Ok(Some(updated)) = r else { continue };
        emit_agent(state, &updated);
        for s in due {
            agents::enqueue_with(state, &a.id, Some(s.prompt), json!({ "scheduled": true }), "scheduled");
            sent += 1;
        }
    }
    sent
}

#[derive(Deserialize)]
pub struct Add {
    text: String,
}

pub async fn add(State(state): State<AppState>, Path(id): Path<String>, Json(b): Json<Add>) -> ApiResult<Value> {
    state.db.get_agent(&id)?.ok_or_else(not_found)?;
    let mut s = parse(&b.text, Utc::now()).map_err(|e| bad(&e))?;
    s.id = uuid::Uuid::new_v4().to_string();
    let a = state.db.update_agent(&id, |a| a.schedules.push(s.clone()))?.ok_or_else(not_found)?;
    emit_agent(&state, &a);
    Ok(Json(json!({ "ok": true })))
}

pub async fn cancel(State(state): State<AppState>, Path((id, sid)): Path<(String, String)>) -> ApiResult<Value> {
    let a = state.db.update_agent(&id, |a| a.schedules.retain(|s| s.id != sid))?.ok_or_else(not_found)?;
    emit_agent(&state, &a);
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
// Most of these fake commands with shell scripts, which run on Unix only.
#[cfg_attr(not(unix), allow(dead_code, unused_imports))]
mod tests {
    use super::*;

    fn at(day: u32, h: u32, m: u32) -> DateTime<Utc> {
        // 2026-10-12 is a Monday.
        Local.with_ymd_and_hms(2026, 10, day, h, m, 0).unwrap().with_timezone(&Utc)
    }

    fn user_texts(st: &AppState, id: &str) -> usize {
        st.db.list_messages(id, 400, None).unwrap().iter().filter(|m| m.role == "user" && m.content == "run the tests").count()
    }

    #[test]
    fn parses_one_offs_and_repeats() {
        let now = at(12, 10, 0);
        let s = parse("tomorrow 9:00 run the tests", now).unwrap();
        assert_eq!((s.prompt.as_str(), s.every.clone(), s.next_at), ("run the tests", None, at(13, 9, 0).to_rfc3339()));
        assert_eq!(parse("Tomorrow 9:00 Run The Tests", now).unwrap().prompt, "Run The Tests", "the words are any case, the prompt is kept as typed");
        assert_eq!(parse("today 17:30 go", now).unwrap().next_at, at(12, 17, 30).to_rfc3339());
        assert_eq!(parse("in 2h go on", now).unwrap().next_at, at(12, 12, 0).to_rfc3339());
        let d = parse("every day 8:00 daily report", now).unwrap();
        assert_eq!((d.every.as_deref(), d.time.as_deref(), d.next_at), (Some("day"), Some("08:00"), at(13, 8, 0).to_rfc3339()));
        assert_eq!(parse("every weekday 9:30 standup", at(16, 10, 0)).unwrap().next_at, at(19, 9, 30).to_rfc3339(), "Friday 10:00 → Monday");
        assert_eq!(parse("every day 11:00 x", now).unwrap().next_at, at(12, 11, 0).to_rfc3339(), "later today");
    }

    #[test]
    fn unclear_times_are_refused_with_an_example() {
        let now = at(12, 10, 0);
        for bad in ["", "tomorrow", "soon do it", "tomorrow 9 go", "tomorrow 25:00 go", "in 5 go", "in 0m go", "every fortnight 9:00 go", "every day go", "tomorrow 9:00", "every day 8:00"] {
            let e = parse(bad, now).unwrap_err();
            assert!(e.contains("tomorrow 9:00"), "{bad}: {e}");
        }
        assert!(parse("today 9:00 go", now).unwrap_err().contains("passed"));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_one_off_goes_out_once_and_is_removed() {
        let (st, a) = agents::tests::fake_claude_state();
        let now = at(12, 10, 0);
        let mut s = parse("tomorrow 9:00 run the tests", now).unwrap();
        s.id = "s1".into();
        st.db.update_agent(&a.id, |x| x.schedules.push(s)).unwrap();
        assert_eq!(tick(&st, at(13, 8, 59)), 0, "not yet");
        assert_eq!(tick(&st, at(13, 9, 0)), 1);
        assert_eq!(user_texts(&st, &a.id), 1);
        assert!(st.db.get_agent(&a.id).unwrap().unwrap().schedules.is_empty());
        assert_eq!(tick(&st, at(13, 9, 1)), 0, "once");
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_repeat_moves_to_its_next_time_and_a_missed_one_runs_once() {
        let (st, a) = agents::tests::fake_claude_state();
        let mut s = parse("every weekday 9:30 run the tests", at(12, 10, 0)).unwrap();
        s.id = "s1".into();
        assert_eq!(s.next_at, at(13, 9, 30).to_rfc3339());
        st.db.update_agent(&a.id, |x| x.schedules.push(s)).unwrap();
        // Loft was down for three days: one send, then the next time after now.
        assert_eq!(tick(&st, at(15, 12, 0)), 1);
        assert_eq!(user_texts(&st, &a.id), 1);
        let next = st.db.get_agent(&a.id).unwrap().unwrap().schedules[0].next_at.clone();
        assert_eq!(next, at(16, 9, 30).to_rfc3339());
        assert_eq!(tick(&st, at(15, 12, 1)), 0);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn quiet_hours_do_not_hold_a_schedule() {
        let (st, a) = agents::tests::fake_claude_state();
        let mut s = parse("tomorrow 9:30 run the tests", at(12, 10, 0)).unwrap();
        s.id = "s1".into();
        st.db.update_agent(&a.id, |x| {
            x.quiet_hours = (0..7).map(|d| crate::db::QuietWindow { days: vec![d], start: "00:00".into(), end: "23:59".into() }).collect();
            x.schedules.push(s);
        })
        .unwrap();
        assert_eq!(tick(&st, at(13, 9, 30)), 1);
        assert_eq!(user_texts(&st, &a.id), 1);
    }

    #[test]
    fn a_repeat_always_finds_a_time_after_now() {
        // A local time the clock jumps over (a DST gap) has no instant: that day is skipped, the next day is used.
        let t = NaiveTime::from_hms_opt(2, 30, 0).unwrap();
        let from = at(12, 10, 0);
        let n = next_occurrence("day", t, from).unwrap();
        assert!(n > from && n <= from + Duration::days(2));
    }

    #[test]
    fn an_unknown_agent_or_schedule_is_not_found_and_changes_nothing() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let e = rt.block_on(add(State(st.clone()), Path("nope".into()), Json(Add { text: "in 2h go".into() }))).err().unwrap();
        assert_eq!(e.0, axum::http::StatusCode::NOT_FOUND);
        let mut s = parse("in 2h go", at(12, 10, 0)).unwrap();
        s.id = "s1".into();
        st.db.update_agent(&a.id, |x| x.schedules.push(s)).unwrap();
        let _ = rt.block_on(cancel(State(st.clone()), Path((a.id.clone(), "other".into())))).ok().unwrap();
        assert_eq!(st.db.get_agent(&a.id).unwrap().unwrap().schedules.len(), 1, "an unknown schedule id removes nothing");
        assert!(rt.block_on(cancel(State(st.clone()), Path(("nope".into(), "s1".into())))).is_err());
    }

    #[test]
    fn cancelling_removes_it() {
        let st = crate::tasks::tests::state();
        let a = crate::tasks::tests::agent(&st, "Worker");
        let mut s = parse("in 2h go", at(12, 10, 0)).unwrap();
        s.id = "s1".into();
        st.db.update_agent(&a.id, |x| x.schedules.push(s)).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _ = rt.block_on(cancel(State(st.clone()), Path((a.id.clone(), "s1".into())))).ok().unwrap();
        assert!(st.db.get_agent(&a.id).unwrap().unwrap().schedules.is_empty());
    }
}
