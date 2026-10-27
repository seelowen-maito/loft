import Foundation

/// The chat's run line text: `2m 23s` while the agent works, `Done in 2m 23s · Thu 16:23` after. Mirrors the
/// web's lib/runTimer.ts.
enum RunTimer {
    /// "45s", "2m 23s", "1h 4m 9s". Negative spans (clock skew) read as 0s.
    static func elapsed(_ seconds: TimeInterval) -> String {
        let s = max(0, Int(seconds.rounded(.down)))
        let h = s / 3600, m = (s % 3600) / 60, sec = s % 60
        if h > 0 { return "\(h)h \(m)m \(sec)s" }
        if m > 0 { return "\(m)m \(sec)s" }
        return "\(sec)s"
    }

    /// Time since the run's start (the server's `last_run_at`), so a reopened chat shows the right count.
    static func elapsed(since start: Date?, now: Date) -> String {
        elapsed(start.map { now.timeIntervalSince($0) } ?? 0)
    }

    /// "Done in 2m 23s · Thu 16:23": the finished run's duration, then its end in the user's locale.
    static func done(_ duration: TimeInterval, endedAt: Date, locale: Locale = .current) -> String {
        let day = endedAt.formatted(.dateTime.weekday(.abbreviated).locale(locale))
        let time = endedAt.formatted(.dateTime.hour().minute().locale(locale))
        return "Done in \(elapsed(duration)) · \(day) \(time)"
    }

    /// The run that ended last, for the "Done in" line; nil after a /clear, or when the latest run ended without a
    /// run_end (claude exited without a result). Measured run_start → run_end, like the live timer.
    static func lastFinished(_ messages: [ChatMessage]) -> (duration: TimeInterval, endedAt: Date)? {
        for (i, m) in messages.enumerated().reversed() {
            if m.kind == "clear" || m.kind == "run_start" { return nil }
            guard m.kind == "run_end" else { continue }
            let start = messages[..<i].last { $0.kind == "run_start" && $0.runId == m.runId }
            let duration = start.map { m.createdAt.timeIntervalSince($0.createdAt) }
                ?? m.meta?["duration_ms"]?.numberValue.map { $0 / 1000 }
            return duration.map { ($0, m.createdAt) }
        }
        return nil
    }
}
