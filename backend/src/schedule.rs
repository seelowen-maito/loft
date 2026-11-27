//! Working hours: weekly windows when an agent may wake up on its own; outside them it waits.
//! No windows means any time. Times are this Mac's local time. (Stored as `quiet_hours`, which used to be
//! the windows when it paused.)

use chrono::{DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveTime, TimeZone, Timelike};

use crate::db::QuietWindow;

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn parse(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// Normalize windows from the API: sorted unique days, valid "HH:MM" times, no empty windows.
pub fn validate(windows: Vec<QuietWindow>) -> Result<Vec<QuietWindow>, String> {
    let mut out = Vec::new();
    for mut w in windows {
        if let Some(d) = w.days.iter().find(|d| **d > 6) {
            return Err(format!("working hours: day {d} is out of range (0 = Monday … 6 = Sunday)"));
        }
        for t in [&w.start, &w.end] {
            if parse(t).is_none() {
                return Err(format!("working hours: \"{t}\" isn't a time (use HH:MM)"));
            }
        }
        w.days.sort_unstable();
        w.days.dedup();
        if !w.days.is_empty() {
            out.push(w);
        }
    }
    Ok(out)
}

/// A local date and time as a real instant. Skipped DST hours move forward an hour.
fn at(d: NaiveDate, t: NaiveTime) -> DateTime<Local> {
    let n = d.and_time(t);
    match Local.from_local_datetime(&n) {
        LocalResult::Single(x) | LocalResult::Ambiguous(x, _) => x,
        LocalResult::None => Local
            .from_local_datetime(&(n + Duration::hours(1)))
            .earliest()
            .unwrap_or_else(|| Local.from_utc_datetime(&n)),
    }
}

/// Whether `t` falls in one of the windows.
fn covered(windows: &[QuietWindow], t: DateTime<Local>) -> bool {
    windows.iter().any(|w| {
        let (Some(s), Some(e)) = (parse(&w.start), parse(&w.end)) else { return false };
        // A window that runs past midnight started the day before.
        (0..2).any(|back| {
            let d = t.date_naive() - Duration::days(back);
            if !w.days.contains(&(d.weekday().num_days_from_monday() as u8)) {
                return false;
            }
            let stop = if e > s { at(d, e) } else { at(d + Duration::days(1), e) };
            at(d, s) <= t && t < stop
        })
    })
}

/// The first window start after `t`, within the next week.
fn next_start(windows: &[QuietWindow], t: DateTime<Local>) -> Option<DateTime<Local>> {
    let mut best: Option<DateTime<Local>> = None;
    for w in windows {
        let Some(s) = parse(&w.start) else { continue };
        for ahead in 0..=7 {
            let d = t.date_naive() + Duration::days(ahead);
            if !w.days.contains(&(d.weekday().num_days_from_monday() as u8)) {
                continue;
            }
            let start = at(d, s);
            if start > t {
                best = Some(best.map_or(start, |b| b.min(start)));
            }
        }
    }
    best
}

/// When the agent may next wake up if `t` is outside its working hours, or None if it may wake at `t`
/// (inside them, or it has none).
pub fn quiet_until(windows: &[QuietWindow], t: DateTime<Local>) -> Option<DateTime<Local>> {
    if windows.is_empty() || covered(windows, t) {
        return None;
    }
    // Windows with no days never start: look again in a week.
    Some(next_start(windows, t).unwrap_or(t + Duration::days(7)))
}

/// `t`, or the start of the next working hours if `t` is outside them.
pub fn next_wake(windows: &[QuietWindow], t: DateTime<Local>) -> DateTime<Local> {
    quiet_until(windows, t).unwrap_or(t)
}

/// One-off migration: windows that used to say when an agent is paused, as the windows when it works.
/// Each day's working minutes become windows, and days with the same hours share one.
pub fn pauses_to_working(paused: &[QuietWindow]) -> Vec<QuietWindow> {
    let mut quiet = [[false; 1440]; 7];
    for w in paused {
        let (Some(s), Some(e)) = (parse(&w.start), parse(&w.end)) else { continue };
        let (s, e) = (s.num_seconds_from_midnight() as usize / 60, e.num_seconds_from_midnight() as usize / 60);
        for &d in w.days.iter().filter(|d| **d < 7) {
            let (d, next) = (d as usize, (d as usize + 1) % 7);
            if e > s {
                quiet[d][s..e].iter_mut().for_each(|m| *m = true);
            } else {
                // Past midnight, or all day when equal.
                quiet[d][s..].iter_mut().for_each(|m| *m = true);
                quiet[next][..e].iter_mut().for_each(|m| *m = true);
            }
        }
    }
    let hhmm = |m: usize| format!("{:02}:{:02}", m / 60 % 24, m % 60);
    let mut runs: Vec<(String, String, Vec<u8>)> = Vec::new();
    for (d, day) in quiet.iter().enumerate() {
        let mut m = 0;
        while m < 1440 {
            if day[m] {
                m += 1;
                continue;
            }
            let start = m;
            while m < 1440 && !day[m] {
                m += 1;
            }
            // An end at midnight reads "00:00", which runs to the end of the day.
            let (s, e) = (hhmm(start), hhmm(m));
            match runs.iter_mut().find(|r| r.0 == s && r.1 == e) {
                Some(r) => r.2.push(d as u8),
                None => runs.push((s, e, vec![d as u8])),
            }
        }
    }
    if runs.is_empty() {
        // Paused all week: a window with no days keeps it from waking at any time.
        return vec![QuietWindow { days: vec![], start: "00:00".into(), end: "00:00".into() }];
    }
    runs.into_iter().map(|(start, end, days)| QuietWindow { days, start, end }).collect()
}

/// "Mon–Fri", "Sat, Sun", "Mon, Wed–Fri".
fn fmt_days(days: &[u8]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < days.len() {
        let mut j = i;
        while j + 1 < days.len() && days[j + 1] == days[j] + 1 {
            j += 1;
        }
        let (a, b) = (DAYS[days[i] as usize], DAYS[days[j] as usize]);
        match j - i {
            0 => parts.push(a.to_string()),
            1 => parts.push(format!("{a}, {b}")),
            _ => parts.push(format!("{a}–{b}")),
        }
        i = j + 1;
    }
    parts.join(", ")
}

/// "Mon–Fri 09:00–17:00; Sat, Sun all day", for the agent's system prompt.
pub fn describe(windows: &[QuietWindow]) -> String {
    windows
        .iter()
        .map(|w| {
            let hours = if w.start == w.end { "all day".to_string() } else { format!("{}–{}", w.start, w.end) };
            format!("{} {hours}", fmt_days(&w.days))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(days: &[u8], start: &str, end: &str) -> QuietWindow {
        QuietWindow { days: days.to_vec(), start: start.into(), end: end.into() }
    }

    // 2026-10-12 is a Monday.
    fn t(day: u32, h: u32, m: u32) -> DateTime<Local> {
        at(NaiveDate::from_ymd_opt(2026, 10, day).unwrap(), NaiveTime::from_hms_opt(h, m, 0).unwrap())
    }

    #[test]
    fn work_hours() {
        let q = [w(&[0, 1, 2, 3, 4], "09:00", "17:00")];
        assert_eq!(quiet_until(&q, t(12, 9, 0)), None);
        assert_eq!(quiet_until(&q, t(16, 16, 59)), None);
        // Outside the hours it waits for the next start.
        assert_eq!(quiet_until(&q, t(12, 8, 59)), Some(t(12, 9, 0)));
        assert_eq!(quiet_until(&q, t(12, 17, 0)), Some(t(13, 9, 0)));
        assert_eq!(quiet_until(&q, t(16, 18, 0)), Some(t(19, 9, 0))); // Friday evening: Monday
        assert_eq!(quiet_until(&q, t(17, 12, 0)), Some(t(19, 9, 0))); // Saturday
        assert_eq!(next_wake(&q, t(12, 8, 0)), t(12, 9, 0));
        assert_eq!(next_wake(&q, t(12, 10, 0)), t(12, 10, 0));
    }

    #[test]
    fn no_windows_means_any_time() {
        assert_eq!(quiet_until(&[], t(12, 3, 0)), None);
    }

    #[test]
    fn overnight_and_whole_days() {
        let q = [w(&[4], "22:00", "06:00"), w(&[5, 6], "00:00", "00:00")];
        // Friday night runs into Saturday, which is a whole day.
        assert_eq!(quiet_until(&q, t(16, 23, 0)), None);
        assert_eq!(quiet_until(&q, t(17, 5, 0)), None);
        assert_eq!(quiet_until(&q, t(18, 12, 0)), None);
        // Monday morning waits for Friday night.
        assert_eq!(quiet_until(&q, t(19, 0, 0)), Some(t(23, 22, 0)));
        assert_eq!(quiet_until(&q, t(16, 21, 0)), Some(t(16, 22, 0)));
    }

    #[test]
    fn pause_windows_become_the_same_working_hours() {
        let cases = [
            vec![w(&[0, 1, 2, 3, 4], "09:00", "17:00")],
            vec![w(&[4], "22:00", "06:00"), w(&[5, 6], "00:00", "00:00")],
            vec![w(&[0], "09:00", "17:00"), w(&[0], "17:00", "23:00")],
            vec![w(&[0, 1, 2, 3, 4, 5, 6], "00:00", "00:00")],
            vec![w(&[6], "20:00", "02:00")],
        ];
        for paused in cases {
            let working = pauses_to_working(&paused);
            // Every half hour of the week 2026-10-12 …: it wakes exactly when it used to.
            for day in 0..7 {
                for slot in 0..48 {
                    let at_t = t(12 + day, slot / 2, slot % 2 * 30) + Duration::minutes(7);
                    // The old rule: paused inside a window.
                    assert_eq!(quiet_until(&working, at_t).is_some(), covered(&paused, at_t), "{paused:?} at day {day} slot {slot}");
                }
            }
        }
    }

    #[test]
    fn validation_and_labels() {
        assert!(validate(vec![w(&[7], "09:00", "17:00")]).is_err());
        assert!(validate(vec![w(&[0], "9am", "17:00")]).is_err());
        let v = validate(vec![w(&[4, 0, 1, 1, 2, 3], "09:00", "17:00"), w(&[], "01:00", "02:00")]).unwrap();
        assert_eq!(v, vec![w(&[0, 1, 2, 3, 4], "09:00", "17:00")]);
        assert_eq!(describe(&[w(&[0, 1, 2, 3, 4], "09:00", "17:00"), w(&[5, 6], "00:00", "00:00")]), "Mon–Fri 09:00–17:00; Sat, Sun all day");
        assert_eq!(fmt_days(&[0, 2, 3, 4]), "Mon, Wed–Fri");
    }
}
