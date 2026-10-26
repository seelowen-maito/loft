import Foundation

/// "While you were away": per agent, what happened since you last looked, from the activity feed and the PR list.
/// Built in code from the facts only. (Rewording by the on-device model was measured and dropped: on 20 cases it
/// was faithful half the time and invented failures and approvals.)
struct Digest: Equatable {
    struct Line: Equatable, Identifiable {
        var agentId: String
        var agentName: String
        /// Plain facts, most pressing first ("Failed: “Fix the build”", "Opened a PR: …").
        var facts: [String]
        /// Something went wrong or waits for you: shown first, in the attention colour.
        var needsYou: Bool
        /// The most pressing fact is a failure: a red dot instead of the attention colour.
        var failed: Bool = false
        var id: String { agentId }

        /// The code's own line: the first two facts.
        var text: String { facts.prefix(2).joined(separator: " · ") + (facts.count > 2 ? " · +\(facts.count - 2) more" : "") }
    }

    var since: Date
    var lines: [Line]

    /// Away at least this long before the card shows.
    static let awayFor: TimeInterval = 30 * 60

    static func build(since: Date, activity: [ActivityItem], pullRequests: [PullRequest], agents: [Agent]) -> Digest {
        let names = Dictionary(agents.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        var facts: [String: [(rank: Int, text: String)]] = [:]
        func add(_ agent: String?, _ rank: Int, _ text: String) {
            guard let agent, names[agent] != nil else { return }
            if facts[agent]?.contains(where: { $0.text == text }) == true { return }
            facts[agent, default: []].append((rank, text))
        }
        for a in activity where a.at > since {
            // The summary is "<Agent> <verb> …"; keep what follows the agent's name.
            let detail = names[a.agentId ?? ""].map { n in a.summary.hasPrefix(n) ? String(a.summary.dropFirst(n.count)).trimmingCharacters(in: .whitespaces) : a.summary } ?? a.summary
            switch a.type {
            case "agent.failed": add(a.agentId, 0, "Run failed\(detail.isEmpty ? "" : ": \(Self.afterColon(detail))")")
            case "approval.required": add(a.agentId, 1, "Waiting for your approval")
            case "task.failed": add(a.agentId, 0, "Task failed: \(Self.quoted(detail))")
            case "task.blocked": add(a.agentId, 1, "Task blocked: \(Self.quoted(detail))")
            case "task.completed": add(a.agentId, 3, "Finished \(Self.quoted(detail))")
            default: break // started, finished, messages, created: noise for a digest
            }
        }
        for p in pullRequests where (p.createdAt ?? .distantPast) > since {
            add(p.agentId, p.state == "merged" ? 2 : 2, p.state == "merged" ? "PR merged: \(p.title)" : "Opened a PR: \(p.title)")
        }
        let lines = facts.map { id, f in
            let sorted = f.sorted { $0.rank < $1.rank }
            return Line(agentId: id, agentName: names[id] ?? id, facts: sorted.map(\.text), needsYou: sorted.first.map { $0.rank <= 1 } ?? false,
                        failed: sorted.first?.rank == 0)
        }
        .sorted { ($0.needsYou ? 0 : 1, $0.agentName) < ($1.needsYou ? 0 : 1, $1.agentName) }
        return Digest(since: since, lines: lines)
    }

    /// “Title” from an activity summary like "finished task “Title”".
    private static func quoted(_ s: String) -> String {
        if let open = s.firstIndex(where: { $0 == "“" || $0 == "\"" }) {
            let rest = s[s.index(after: open)...]
            if let close = rest.firstIndex(where: { $0 == "”" || $0 == "\"" }) { return "“\(rest[..<close])”" }
        }
        return s
    }

    private static func afterColon(_ s: String) -> String {
        s.split(separator: ":", maxSplits: 1).last.map { $0.trimmingCharacters(in: .whitespaces) } ?? s
    }
}
