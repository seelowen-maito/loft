import Foundation

/// The chat header's lines and the agents list's context size, as the web tells them (frontend lib/format.ts,
/// lib/context.ts, pages/agents/Chat.tsx).
public enum AgentHeader {
    /// "every 30m · custom · next in 12m": the interval (custom when quiet hours shape it), then the next wake-up,
    /// "next: any moment" when it's under 45 s away or overdue (the web's `nextRunText`). "manual" with no interval.
    public static func schedule(_ a: Agent, now: Date = Date()) -> String {
        let minutes = a.intervalMinutes ?? 0
        var s = interval(minutes)
        if minutes > 0, a.quietHours?.isEmpty == false { s += " · custom" }
        if minutes > 0, let next = a.nextRunAt {
            s += next.timeIntervalSince(now) < 45 ? " · next: any moment" : " · next \(timeAgo(next, now: now))"
        }
        return s
    }

    /// "Paused Mon–Fri 22:00–07:00": when its quiet hours keep it asleep; nil without any.
    public static func quietHours(_ a: Agent) -> String? {
        guard (a.intervalMinutes ?? 0) > 0, let q = a.quietHours, !q.isEmpty else { return nil }
        return "Paused " + q.map { "\(days($0.days)) \($0.start == $0.end ? "all day" : "\($0.start)–\($0.end)")" }.joined(separator: "; ")
    }

    /// Whether it has a Session: one to compact or clear. Older servers don't send the id; a known size still means one.
    public static func hasSession(_ a: Agent) -> Bool { a.sessionId != nil || a.contextTokens != nil }

    /// "context 84k", "context —" before the Session's first reply, or "no session".
    public static func context(_ a: Agent) -> String {
        guard hasSession(a) else { return "no session" }
        return "context \(a.contextTokens.map(tokens) ?? "—")"
    }

    /// The size a row of the agents list shows: only with a Session that has replied (web `listedContext`).
    public static func listedContext(_ a: Agent) -> Int? { a.sessionId != nil ? a.contextTokens : nil }

    /// The web's `fmtNum`: "950", "4.2k", "84k".
    public static func tokens(_ n: Int) -> String {
        if n >= 10_000 { return "\(Int((Double(n) / 1000).rounded()))k" }
        if n >= 1000 { return String(format: "%.1fk", Double(n) / 1000) }
        return "\(n)"
    }

    /// The web's `fmtInterval`: "manual", "every 30m", "every 2h", "every 1d".
    static func interval(_ m: Int) -> String {
        if m <= 0 { return "manual" }
        if m % 1440 == 0 { return "every \(m / 1440)d" }
        if m % 60 == 0 { return "every \(m / 60)h" }
        return "every \(m)m"
    }

    /// The web's `timeAgo`: "in 12m", "3h ago", "just now".
    static func timeAgo(_ date: Date, now: Date) -> String {
        let s = Int(now.timeIntervalSince(date).rounded()), a = abs(s)
        let unit = a < 45 ? nil
            : a < 3600 ? "\(Int((Double(a) / 60).rounded()))m"
            : a < 86_400 ? "\(Int((Double(a) / 3600).rounded()))h"
            : a < 86_400 * 30 ? "\(Int((Double(a) / 86_400).rounded()))d"
            : a < 86_400 * 365 ? "\(Int((Double(a) / (86_400 * 30)).rounded()))mo"
            : "\(Int((Double(a) / (86_400 * 365)).rounded()))y"
        guard let unit else { return "just now" }
        return s < 0 ? "in \(unit)" : "\(unit) ago"
    }

    private static let dayNames = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]

    /// The web's `fmtDays`: "Mon–Fri", "Sat, Sun", "Mon, Wed–Fri".
    static func days(_ days: [Int]) -> String {
        var parts: [String] = []
        var i = 0
        while i < days.count {
            var j = i
            while j + 1 < days.count, days[j + 1] == days[j] + 1 { j += 1 }
            let name = { (d: Int) in dayNames.indices.contains(d) ? dayNames[d] : "?" }
            parts.append(j == i ? name(days[i]) : j == i + 1 ? "\(name(days[i])), \(name(days[j]))" : "\(name(days[i]))–\(name(days[j]))")
            i = j + 1
        }
        return parts.joined(separator: ", ")
    }
}
