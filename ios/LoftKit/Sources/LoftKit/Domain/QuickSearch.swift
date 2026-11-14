import Foundation

/// The Mac's quick search (spec #456): what a typed query finds among the agents, issues, tickets, specs and Loft's
/// pages, best match first.
public enum QuickSearch {
    public enum Kind: String, CaseIterable, Sendable {
        case agent, spec, ticket, issue, page

        public var label: String {
            switch self {
            case .agent: "Agent"
            case .spec: "Spec"
            case .ticket: "Ticket"
            case .issue: "Issue"
            case .page: "Page"
            }
        }
    }

    public struct Result: Identifiable, Hashable, Sendable {
        public var kind: Kind
        /// The agent's or issue's id; a page's web path.
        public var target: String
        public var title: String
        /// "loft#459" for an issue on GitHub; nil for agents, pages and Loft-only issues.
        public var number: String?
        /// The page on Loft's web, from its address: `/agents/<id>`, `/issues/<id>`, `/simulator`.
        public var webPath: String
        /// A page only Loft's web has: it always opens in the browser.
        public var webOnly = false
        public var id: String { "\(kind.rawValue):\(target)" }
    }

    /// Loft's web pages, with those the Mac app doesn't have marked web-only.
    public static let pages: [Result] = [
        page("Overview", "/repos"), page("Agents", "/agents"), page("Issues", "/issues"), page("Needs you", "/needs-you"),
        page("Settings", "/settings"), page("Clean", "/settings/clean", webOnly: true), page("Simulator", "/simulator", webOnly: true),
    ]

    private static func page(_ title: String, _ path: String, webOnly: Bool = false) -> Result {
        Result(kind: .page, target: path, title: title, webPath: path, webOnly: webOnly)
    }

    /// The results for `query`, best first, at most `limit`. An empty query finds nothing.
    public static func results(_ query: String, agents: [Agent], tasks: [HBTask], deliveries: [DeliverySummary],
                               limit: Int = 12) -> [Result] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !q.isEmpty else { return [] }
        var hits: [(score: Int, closed: Bool, updated: Date, result: Result)] = []
        for a in agents {
            if let s = score(q, title: a.name) {
                hits.append((s, false, .distantPast, Result(kind: .agent, target: a.id, title: a.name, webPath: "/agents/\(a.id)")))
            }
        }
        for t in tasks {
            let number = t.github.map { "#\($0.number)" }
            guard let s = score(q, title: t.title, number: number) else { continue }
            let kind: Kind = SpecViews.isSpec(t) ? .spec : SpecViews.isSpecTicket(t, deliveries) ? .ticket : .issue
            hits.append((s, t.state.isClosed, t.updatedAt,
                         Result(kind: kind, target: t.id, title: t.title, number: t.github?.shortName, webPath: "/issues/\(t.id)")))
        }
        for p in pages {
            if let s = score(q, title: p.title) { hits.append((s, false, .distantPast, p)) }
        }
        let order = Dictionary(uniqueKeysWithValues: Kind.allCases.enumerated().map { ($1, $0) })
        return hits.sorted { a, b in
            if a.score != b.score { return a.score < b.score }
            if a.closed != b.closed { return !a.closed }
            if a.result.kind != b.result.kind { return order[a.result.kind]! < order[b.result.kind]! }
            if a.updated != b.updated { return a.updated > b.updated }
            return a.result.title.localizedCaseInsensitiveCompare(b.result.title) == .orderedAscending
        }
        .prefix(limit).map(\.result)
    }

    /// How well a lowercased query matches, lower is better, nil for no match: the number exactly ("459", "#459" or
    /// "loft#459"), the whole title, the start of the title, the start of a word, anywhere in it, then the words' first
    /// letters in order.
    static func score(_ q: String, title: String, number: String? = nil) -> Int? {
        if let number, "#" + q == number || q.hasSuffix(number) { return 0 }
        let t = title.lowercased()
        if t == q { return 1 }
        if t.hasPrefix(q) { return 2 }
        let words = t.split(whereSeparator: { !$0.isLetter && !$0.isNumber })
        if words.contains(where: { $0.hasPrefix(q) }) { return 3 }
        if t.contains(q) { return 4 }
        // Letters in order only on the words' first letters ("lmq" for Loft Mobile QA), and from three letters: across
        // whole titles, short queries match nearly every issue.
        let letters = q.filter { !$0.isWhitespace }
        guard letters.count >= 3 else { return nil }
        var rest = Substring(letters)
        for w in words where w.first == rest.first { rest = rest.dropFirst() }
        return rest.isEmpty ? 5 : nil
    }
}
