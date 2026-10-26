//! Quiet hours: weekly windows when an agent's autonomous wake-ups are held, so
//! background runs stay out of the user's working hours. Times are this Mac's local time.

use chrono::{DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveTime, TimeZone};

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
            return Err(format!("quiet_hours: day {d} is out of range (0 = Monday … 6 = Sunday)"));
        }
        for t in [&w.start, &w.end] {
            if parse(t).is_none() {
                return Err(format!("quiet_hours: \"{t}\" isn't a time (use HH:MM)"));
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

/// The latest end among the windows covering `t`.
fn covering_end(windows: &[QuietWindow], t: DateTime<Local>) -> Option<DateTime<Local>> {
    let mut end = None;
    for w in windows {
        let (Some(s), Some(e)) = (parse(&w.start), parse(&w.end)) else { continue };
        // A window that runs past midnight started the day before.
        for back in 0..2 {
            let d = t.date_naive() - Duration::days(back);
            if !w.days.contains(&(d.weekday().num_days_from_monday() as u8)) {
                continue;
            }
            let start = at(d, s);
            let stop = if e > s { at(d, e) } else { at(d + Duration::days(1), e) };
            if start <= t && t < stop {
                end = end.max(Some(stop));
            }
        }
    }
    end
}

/// When the quiet hours covering `t` end, or None if `t` is outside them.
pub fn quiet_until(windows: &[QuietWindow], t: DateTime<Local>) -> Option<DateTime<Local>> {
    let mut until = covering_end(windows, t)?;
    // Windows can touch (Mon 09:00–17:00, then 17:00–23:00); follow them to the first free moment.
    for _ in 0..16 {
        match covering_end(windows, until) {
            Some(e) => until = e,
            None => break,
        }
    }
    Some(until)
}

/// `t`, or the end of the quiet hours it falls in.
pub fn next_wake(windows: &[QuietWindow], t: DateTime<Local>) -> DateTime<Local> {
    quiet_until(windows, t).unwrap_or(t)
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
        assert_eq!(quiet_until(&q, t(12, 8, 59)), None);
        assert_eq!(quiet_until(&q, t(12, 9, 0)), Some(t(12, 17, 0)));
        assert_eq!(quiet_until(&q, t(16, 16, 30)), Some(t(16, 17, 0)));
        assert_eq!(quiet_until(&q, t(12, 17, 0)), None);
        assert_eq!(quiet_until(&q, t(17, 12, 0)), None); // Saturday
    }

    #[test]
    fn overnight_and_whole_days() {
        let q = [w(&[4], "22:00", "06:00"), w(&[5, 6], "00:00", "00:00")];
        // Friday night runs into the weekend, which runs into Monday morning.
        assert_eq!(quiet_until(&q, t(16, 23, 0)), Some(t(19, 0, 0)));
        assert_eq!(quiet_until(&q, t(18, 12, 0)), Some(t(19, 0, 0)));
        assert_eq!(quiet_until(&q, t(16, 21, 0)), None);
    }

    #[test]
    fn touching_windows() {
        let q = [w(&[0], "09:00", "17:00"), w(&[0], "17:00", "23:00")];
        assert_eq!(quiet_until(&q, t(12, 10, 0)), Some(t(12, 23, 0)));
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
