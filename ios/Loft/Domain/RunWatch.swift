import Foundation

/// Why a running agent looks stuck, for the bar above the composer: it replied and is waiting on
/// background work, or nothing has come out of it for a while. Mirrors the web's rule.
enum RunWatch: Equatable {
    case waiting(first: String, more: Int, minutes: Int)
    case silent(minutes: Int)

    /// Silence this long counts as stuck.
    static let silentAfter: TimeInterval = 10 * 60

    /// `newestMessage`: when the agent's newest chat message was written. Agent updates aren't sent on
    /// every event, so the last activity is the later of that and `lastOutputAt`.
    static func of(_ agent: Agent, newestMessage: Date?, now: Date = Date()) -> RunWatch? {
        let running = agent.running == true || agent.status == "running"
        guard running else { return nil }
        if let waits = agent.waitingOn, let first = waits.first {
            let since = agent.waitingSince ?? agent.lastOutputAt ?? now
            return .waiting(first: first, more: waits.count - 1, minutes: minutes(since, now))
        }
        guard let last = [agent.lastOutputAt, newestMessage].compactMap({ $0 }).max() else { return nil }
        let quiet = now.timeIntervalSince(last)
        return quiet >= silentAfter ? .silent(minutes: Int(quiet / 60)) : nil
    }

    private static func minutes(_ since: Date, _ now: Date) -> Int { max(0, Int(now.timeIntervalSince(since) / 60)) }

    /// For tight spaces (accessibility sizes): "Waiting · 14 min" / "Quiet · 23 min". Non-breaking spaces keep
    /// "·" with the word before it and "14 min" together when it wraps.
    var shortText: String {
        switch self {
        case .waiting(_, _, let minutes): "Waiting\u{00A0}· \(minutes)\u{00A0}min"
        case .silent(let minutes): "Quiet\u{00A0}· \(minutes)\u{00A0}min"
        }
    }

    /// "Waiting on a background task: Run the QA suite (+1 more) · 14 min" / "No activity for 23 min".
    var text: String {
        switch self {
        case .waiting(let first, let more, let minutes):
            "Waiting on a background task: \(first)\(more > 0 ? " (+\(more) more)" : "") · \(minutes) min"
        case .silent(let minutes):
            "No activity for \(minutes) min"
        }
    }
}
