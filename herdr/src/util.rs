//! Times as Loft sends them (RFC 3339), without a date library.

/// Seconds since the epoch for `2026-11-10T12:34:56.789Z` or `…+02:00`; None when unreadable.
pub fn epoch(ts: &str) -> Option<i64> {
    let ts = ts.trim();
    let (date, rest) = ts.split_once(['T', ' '])?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let time_end = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (time, zone) = rest.split_at(time_end);
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next()?.parse().ok()?;
    let s: f64 = t.next().unwrap_or("0").parse().ok()?;
    let offset = match zone.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let z = &zone[1..];
            let (zh, zm) = z.split_once(':').unwrap_or((&z[..z.len().min(2)], if z.len() > 2 { &z[2..] } else { "0" }));
            let o = zh.parse::<i64>().ok()? * 3600 + zm.parse::<i64>().unwrap_or(0) * 60;
            if sign == '+' { o } else { -o }
        }
        _ => 0,
    };
    Some(days_from_civil(y, m, day) * 86400 + h * 3600 + mi * 60 + s as i64 - offset)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "45s", "3m 05s", "2h 10m".
pub fn span(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// "just now", "5 min ago", "3 h ago", "2 d ago".
pub fn ago(ts: &str) -> String {
    let Some(t) = epoch(ts) else { return String::new() };
    let d = now() - t;
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{} min ago", d / 60)
    } else if d < 86400 {
        format!("{} h ago", d / 3600)
    } else {
        format!("{} d ago", d / 86400)
    }
}

/// "12.3k" for token counts.
pub fn tokens(n: f64) -> String {
    if n >= 1000.0 { format!("{:.1}k", n / 1000.0) } else { format!("{n:.0}") }
}

pub fn bytes(n: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{v:.0} B") } else { format!("{v:.1} {}", units[i]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rfc3339() {
        assert_eq!(epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch("2026-11-10T12:00:00.250Z"), Some(1_794_312_000));
        assert_eq!(epoch("2026-11-10T14:00:00+02:00"), Some(1_794_312_000));
        assert_eq!(epoch("2026-11-10 12:00:00"), Some(1_794_312_000));
        assert_eq!(epoch("garbage"), None);
    }

    #[test]
    fn formats_spans() {
        assert_eq!(span(42), "42s");
        assert_eq!(span(185), "3m 05s");
        assert_eq!(span(7800), "2h 10m");
        assert_eq!(tokens(45210.0), "45.2k");
        assert_eq!(bytes(1536.0), "1.5 KB");
    }
}
