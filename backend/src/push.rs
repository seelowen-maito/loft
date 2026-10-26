//! Which "needs you" items become push notifications, and when (the sending is in apns.rs).
//!
//! - One push per item id, ever: a changed item has a new id; a dismissed or resolved one is no longer
//!   in the list, so it's never pushed again. Items already waiting when Loft starts (or while push
//!   isn't set up) are marked as seen, not pushed.
//! - Per device: only the kinds it chose, never inside its quiet hours.
//! - At most `LIMIT` pushes per device per `WINDOW`; the rest become one "N more things need you".

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use chrono::{Timelike, Utc};
use serde_json::json;

use crate::apns::{self, Device, Push, QuietHours};
use crate::needs_you::PushItem;
use crate::state::AppState;

const LIMIT: usize = 5;
const WINDOW: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
pub struct Pusher {
    seeded: bool,
    /// device token -> when its recent pushes went out
    sent_at: HashMap<String, VecDeque<Instant>>,
    /// device token -> when it last got a "N more" summary
    summary_at: HashMap<String, Instant>,
}

fn minutes(hm: &str) -> Option<u32> {
    let (h, m) = hm.split_once(':')?;
    Some(h.parse::<u32>().ok()? * 60 + m.parse::<u32>().ok()?)
}

/// Is `utc_minute` (minutes since midnight UTC) inside the device's quiet hours? Handles windows
/// across midnight (22:00–07:00).
pub fn in_quiet(q: &QuietHours, utc_minute: u32) -> bool {
    let (Some(start), Some(end)) = (minutes(&q.start), minutes(&q.end)) else { return false };
    let local = (utc_minute as i64 + q.utc_offset_minutes as i64).rem_euclid(24 * 60) as u32;
    match start.cmp(&end) {
        std::cmp::Ordering::Less => local >= start && local < end,
        std::cmp::Ordering::Greater => local >= start || local < end,
        std::cmp::Ordering::Equal => false,
    }
}

fn configured(state: &AppState) -> bool {
    let s = state.settings.get().push;
    apns::key_present() && !s.key_id.trim().is_empty() && !s.team_id.trim().is_empty()
}

impl Pusher {
    /// Called with the current list every few seconds by the "needs you" watcher.
    pub async fn tick(&mut self, state: &AppState, items: &[PushItem]) {
        let Ok(seen) = state.db.push_seen_ids() else { return };
        let fresh: Vec<&PushItem> = items.iter().filter(|i| !seen.contains(&i.id)).collect();
        if fresh.is_empty() {
            self.seeded = true;
            return;
        }
        // Mark them seen first: whatever happens below, an item is pushed at most once.
        for i in &fresh {
            let _ = state.db.mark_push_seen(&i.id);
        }
        let devices = state.db.list_devices().unwrap_or_default();
        if !self.seeded || devices.is_empty() || !configured(state) {
            // What was already waiting at startup, or before push was set up, isn't news.
            self.seeded = true;
            return;
        }
        let now_min = { let n = Utc::now(); n.hour() * 60 + n.minute() };
        for d in &devices {
            let wanted: Vec<&&PushItem> = fresh
                .iter()
                .filter(|i| d.kinds.iter().any(|k| k == i.kind))
                .filter(|_| d.quiet_hours.as_ref().is_none_or(|q| !in_quiet(q, now_min)))
                .collect();
            let mut skipped = 0;
            for i in wanted {
                if !self.allow(&d.token) {
                    skipped += 1;
                    continue;
                }
                self.send(state, d, item_push(i, items.len())).await;
            }
            if skipped > 0 && self.summary_at.get(&d.token).is_none_or(|t| t.elapsed() > WINDOW) {
                self.summary_at.insert(d.token.clone(), Instant::now());
                let p = Push {
                    title: "Loft".into(),
                    body: format!("{skipped} more {} need you", if skipped == 1 { "thing" } else { "things" }),
                    badge: Some(items.len() as u64),
                    thread_id: Some("loft-summary".into()),
                    time_sensitive: false,
                    collapse_id: Some("loft-summary".into()),
                    custom: json!({ "type": "summary", "target": { "screen": "needs_you" } }),
                };
                self.send(state, d, p).await;
            }
        }
    }

    /// Room for one more push to this device in the current window?
    fn allow(&mut self, token: &str) -> bool {
        let q = self.sent_at.entry(token.to_string()).or_default();
        while q.front().is_some_and(|t| t.elapsed() > WINDOW) {
            q.pop_front();
        }
        if q.len() >= LIMIT {
            return false;
        }
        q.push_back(Instant::now());
        true
    }

    async fn send(&self, state: &AppState, d: &Device, p: Push) {
        // Errors name the device and status only: never the payload.
        if let Err(e) = apns::send(state, d, &p).await {
            tracing::warn!("push to {}: {e}", if d.name.is_empty() { "a device" } else { d.name.as_str() });
        }
    }
}

/// The notification for one item: its push title and body (names and titles only), the count as badge,
/// a thread per agent, Time Sensitive for urgency 3, and where tapping it goes.
pub fn item_push(i: &PushItem, count: usize) -> Push {
    Push {
        title: i.title.clone(),
        body: i.body.clone(),
        badge: Some(count as u64),
        thread_id: Some(i.agent_id.clone().unwrap_or_else(|| "loft".into())),
        time_sensitive: i.urgency >= 3,
        collapse_id: Some(i.id.clone()),
        custom: json!({
            "needs_you_id": i.id, "type": i.kind, "urgency": i.urgency,
            "agent_id": i.agent_id, "task_id": i.task_id, "target": i.target,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{PermissionRequest, now};
    use crate::needs_you::{collect, to_push};
    use crate::tasks::tests::{agent, state};
    use serde_json::Value;

    fn q(start: &str, end: &str, off: i32) -> QuietHours {
        QuietHours { start: start.into(), end: end.into(), tz: String::new(), utc_offset_minutes: off }
    }

    #[test]
    fn quiet_hours_cross_midnight_and_use_the_offset() {
        let night = q("22:00", "07:00", 0);
        assert!(in_quiet(&night, 23 * 60) && in_quiet(&night, 3 * 60) && !in_quiet(&night, 12 * 60) && !in_quiet(&night, 7 * 60));
        let lunch = q("12:00", "13:00", 60);
        assert!(in_quiet(&lunch, 11 * 60 + 30), "11:30 UTC is 12:30 at UTC+1");
        assert!(!in_quiet(&lunch, 12 * 60 + 30));
        assert!(!in_quiet(&q("09:00", "09:00", 0), 9 * 60));
    }

    fn device(st: &AppState, token: &str, kinds: &[&str], quiet: Option<QuietHours>) {
        st.db.save_device(&Device {
            token: token.into(), platform: "ios".into(), environment: "production".into(), bundle_id: "com.example.loft".into(),
            name: "Phone".into(), kinds: kinds.iter().map(|k| k.to_string()).collect(), quiet_hours: quiet, created_at: now(), last_seen_at: now(),
        }).unwrap();
    }

    fn got(seen: &crate::apns::tests::Seen, token: &str) -> Vec<(Value, Value)> {
        seen.lock().unwrap().iter().filter(|(t, ..)| t == token).map(|(_, h, b)| (h.clone(), b.clone())).collect()
    }

    fn permission(st: &AppState, id: &str, agent_id: &str) {
        st.db.save_permission(&PermissionRequest {
            id: id.into(), agent_id: agent_id.into(), source: "prompt".into(), tool_name: "Bash".into(),
            input: serde_json::json!({ "command": "git push https://x:ghp_abcdefghijklmnopqrstuvwxyz123456@github.com/o/r" }),
            reason: String::new(), rule: String::new(), status: "pending".into(), created_at: now(),
        }).unwrap();
    }

    async fn tick(st: &AppState, p: &mut Pusher) {
        let items: Vec<_> = collect(st).unwrap().iter().map(to_push).collect();
        p.tick(st, &items).await;
    }

    #[tokio::test]
    async fn new_items_are_pushed_once_safely_to_the_devices_that_want_them() {
        let seen = crate::apns::tests::fake_apns();
        let st = state();
        crate::apns::tests::configure(&st);
        let dev = agent(&st, "Web Dev");
        let all = format!("{}01", "aa".repeat(31));
        let only_questions = format!("{}02", "aa".repeat(31));
        let sleeping = format!("{}03", "aa".repeat(31));
        device(&st, &all, crate::apns::DEFAULT_KINDS, None);
        device(&st, &only_questions, &["question"], None);
        // Quiet from an hour ago to an hour from now.
        let n = Utc::now();
        let hm = |m: i64| { let m = m.rem_euclid(24 * 60); format!("{:02}:{:02}", m / 60, m % 60) };
        let now_m = (n.hour() * 60 + n.minute()) as i64;
        device(&st, &sleeping, crate::apns::DEFAULT_KINDS, Some(q(&hm(now_m - 60), &hm(now_m + 60), 0)));

        // Already waiting at startup: not news.
        permission(&st, "old", &dev.id);
        let mut p = Pusher::default();
        tick(&st, &mut p).await;
        assert!(got(&seen, &all).is_empty());

        // A new permission prompt: pushed once, to the device that wants permissions.
        permission(&st, "new", &dev.id);
        tick(&st, &mut p).await;
        tick(&st, &mut p).await;
        let pushes = got(&seen, &all);
        assert_eq!(pushes.len(), 1, "one push per item, however many ticks");
        let (h, b) = &pushes[0];
        assert_eq!((b["aps"]["alert"]["title"].clone(), b["aps"]["alert"]["body"].clone()), (serde_json::json!("Web Dev needs your approval"), serde_json::json!("Wants to use Bash")));
        assert!(!b.to_string().contains("git push") && !b.to_string().contains("ghp_"), "no tool input in a push: {b}");
        assert_eq!((b["aps"]["interruption-level"].clone(), b["aps"]["badge"].clone(), b["aps"]["thread-id"].clone()), (serde_json::json!("time-sensitive"), serde_json::json!(2), serde_json::json!(dev.id)));
        assert_eq!((b["needs_you_id"].clone(), b["type"].clone(), b["target"]["screen"].clone()), (serde_json::json!("permission:new"), serde_json::json!("permission"), serde_json::json!("chat")));
        assert_eq!((h["apns-priority"].clone(), h["apns-collapse-id"].clone()), (serde_json::json!("10"), serde_json::json!("permission:new")));
        assert!(got(&seen, &only_questions).is_empty(), "it only wants questions");
        assert!(got(&seen, &sleeping).is_empty(), "quiet hours");

        // Answered (no longer in the list) and then nothing new: nothing more is sent.
        st.db.save_permission(&PermissionRequest { status: "allowed_once".into(), ..st.db.list_permissions(None).unwrap().into_iter().find(|p| p.id == "new").unwrap() }).unwrap();
        tick(&st, &mut p).await;
        assert_eq!(got(&seen, &all).len(), 1);
    }

    #[tokio::test]
    async fn a_burst_becomes_five_pushes_and_one_summary() {
        let seen = crate::apns::tests::fake_apns();
        let st = state();
        crate::apns::tests::configure(&st);
        let dev = agent(&st, "Dev");
        let phone = format!("{}04", "bb".repeat(31));
        device(&st, &phone, crate::apns::DEFAULT_KINDS, None);
        let mut p = Pusher::default();
        tick(&st, &mut p).await;
        for i in 0..8 {
            permission(&st, &format!("burst{i}"), &dev.id);
        }
        tick(&st, &mut p).await;
        let pushes = got(&seen, &phone);
        assert_eq!(pushes.len(), 6, "5 items + 1 summary");
        let summary = &pushes[5].1;
        assert_eq!((summary["type"].clone(), summary["aps"]["alert"]["body"].clone()), (serde_json::json!("summary"), serde_json::json!("3 more things need you")));
        // More in the same window: no second summary.
        permission(&st, "burst9", &dev.id);
        tick(&st, &mut p).await;
        assert_eq!(got(&seen, &phone).len(), 6);
    }

    #[tokio::test]
    async fn nothing_is_sent_or_saved_up_while_push_is_not_set_up() {
        let seen = crate::apns::tests::fake_apns();
        let st = state();
        let dev = agent(&st, "Dev");
        let phone = format!("{}05", "cc".repeat(31));
        device(&st, &phone, crate::apns::DEFAULT_KINDS, None);
        let mut p = Pusher::default();
        tick(&st, &mut p).await;
        permission(&st, "while-off", &dev.id);
        tick(&st, &mut p).await;
        // Set up afterwards: the earlier item isn't pushed late.
        crate::apns::tests::configure(&st);
        tick(&st, &mut p).await;
        assert!(got(&seen, &phone).is_empty());
    }
}
