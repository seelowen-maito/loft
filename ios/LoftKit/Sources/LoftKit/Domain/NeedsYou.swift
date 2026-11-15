import Foundation

// "Needs you" (GET /api/needs-you, contract docs/needs-you-api.md): everything waiting on you, from all agents,
// already ordered by the server (urgency, then oldest). All text is plain; credentials are redacted server-side.

public struct NeedsYouList: Codable, Hashable {
    public var version: Int
    /// The whole list, and how many of it are urgent.
    public var count: Int
    public var urgent: Int
    /// What the user hasn't seen on any client (the badge), and how many of those are urgent. Older servers send none.
    public var new: Int?
    public var newUrgent: Int?
    public var generatedAt: Date?
    public var items: [NeedsYouItem]

    /// The badge: what hasn't been seen, or everything from an older server.
    public var unseen: Int { new ?? count }

    /// The card's header: how many are new, and how many in all.
    public var headline: String {
        let n = new ?? 0
        return n > 0 ? "Needs you · \(n) new of \(count)" : "Needs you · \(count)"
    }
}

public struct NeedsYouItem: Codable, Hashable, Identifiable {
    public struct AgentRef: Codable, Hashable { public var id: String; public var name: String }
    public struct Link: Codable, Hashable { public var label: String; public var url: String?; public var path: String? }

    public let id: String
    public var type: String
    /// 3 blocks an agent now · 2 waiting for your answer · 1 worth a look · 0 to review
    public var urgency: Int
    public var agent: AgentRef?
    public var taskId: String?
    public var title: String
    public var reason: String
    public var text: String
    public var createdAt: Date?
    public var ageSeconds: Int?
    public var links: [Link]?
    public var actions: [NeedsYouAction]
    /// The item's project (for a PR, its GitHub repo's name). Older servers send none.
    public var project: String?
    /// The pull request, on PR items (`pr_ready`, `pr_review`, …).
    public var pr: PullRequestRef?
    /// Seen on some client (the list was opened there, web or iPhone). Seen items stay listed, less prominent.
    public var seen: Bool?

    public struct PullRequestRef: Codable, Hashable {
        /// "owner/name"
        public var repo: String?
        public var project: String?
        public var number: Int?
        public var url: String?
    }

    public var isPullRequest: Bool { pr != nil || type.hasPrefix("pr_") }

    /// Answerable by typing (a reply, an interview answer, a task comment).
    public var answerAction: NeedsYouAction? { actions.first { $0.kind == "answer" } }
    public var openAction: NeedsYouAction? { actions.first { $0.kind == "open" } }
    public var canDismiss: Bool { actions.contains { $0.kind == "dismiss" } }
    /// The Done check: takes the item off the list (the `dismiss` action).
    public var doneAction: NeedsYouAction? { actions.first { $0.kind == "dismiss" && $0.isRequest } }
}

/// The seen mark (POST /api/needs-you/seen), shared with the web: opening the list marks what it shows.
public enum NeedsYouSeen {
    /// The items shown that no client has seen yet: the ones to mark.
    public static func unseen(_ items: [NeedsYouItem]) -> [String] { items.filter { $0.seen != true }.map(\.id) }

    /// The items shown as new while the list is open: those unseen when they appeared stay new until it closes, even
    /// once marked; ids that left the list drop out.
    public static func fresh(after fresh: Set<String>, items: [NeedsYouItem]) -> Set<String> {
        let ids = Set(items.map(\.id))
        return fresh.intersection(ids).union(unseen(items))
    }

    /// The request body that marks `ids` seen.
    public static func body(_ ids: [String]) -> JSONValue { .object(["ids": .array(ids.map(JSONValue.string))]) }
}

public struct NeedsYouAction: Codable, Hashable {
    public struct Target: Codable, Hashable { public var screen: String?; public var agentId: String?; public var taskId: String?; public var url: String? }

    public var kind: String
    public var label: String
    public var method: String?
    public var path: String?
    public var body: JSONValue?
    public var target: Target?

    /// The text an answer starts from: what the server put in its body ("Edit and approve" on a comment), or nothing.
    public var draft: String { body?["text"]?.stringValue ?? "" }

    /// An HTTP action (anything but `open`).
    public var isRequest: Bool { method != nil && path != nil }

    /// Answers an issue: the server delivers it to the issue's agent, or to `agentId` when nobody is on it.
    public var isIssueReply: Bool { kind == "answer" && path?.hasSuffix("/comments") == true }

    /// The body to send: `text` filled in for answers and retries, plus a client id for messages and comments, and on
    /// an issue with nobody on it, the agent picked to take it.
    public func body(text: String?, clientId: String, agentId: String? = nil) -> JSONValue? {
        guard case .object(var o)? = body else { return body }
        if let text, o["text"] != nil { o["text"] = .string(text) }
        if let p = path, p.contains("/messages") || p.contains("/comments") { o["client_id"] = .string(clientId) }
        if let agentId, isIssueReply { o["agent_id"] = .string(agentId) }
        return .object(o)
    }
}

/// The Needs you list as shown: PR items under a header per project, everything else as it comes. Without project
/// fields (an older server) it's the flat list.
public enum NeedsYouSection: Identifiable, Hashable {
    case item(NeedsYouItem)
    case project(name: String, items: [NeedsYouItem])

    public var id: String {
        switch self {
        case .item(let i): i.id
        case .project(let name, _): "project:\(name)"
        }
    }

    public var urgency: Int {
        switch self {
        case .item(let i): i.urgency
        case .project(_, let items): items.map(\.urgency).max() ?? 0
        }
    }

    public static func build(_ items: [NeedsYouItem]) -> [NeedsYouSection] {
        func project(_ i: NeedsYouItem) -> String? {
            guard i.isPullRequest else { return nil }
            let p = (i.pr?.project ?? i.pr?.repo.map { String($0.split(separator: "/").last ?? "") } ?? i.project)?.trimmingCharacters(in: .whitespaces)
            return p?.isEmpty == false ? p : nil
        }
        var groups: [String: [NeedsYouItem]] = [:]
        var order: [(section: NeedsYouSection, index: Int)] = []
        for (n, i) in items.enumerated() {
            if let p = project(i) {
                if groups[p] == nil { order.append((.project(name: p, items: []), n)) }
                groups[p, default: []].append(i)
            } else {
                order.append((.item(i), n))
            }
        }
        let sections = order.map { e -> (section: NeedsYouSection, index: Int) in
            if case .project(let name, _) = e.section { return (.project(name: name, items: groups[name] ?? []), e.index) }
            return e
        }
        // The server's order (urgency, then oldest) for items; within an urgency, projects after the items, the
        // bigger first.
        return sections.sorted { a, b in
            if a.section.urgency != b.section.urgency { return a.section.urgency > b.section.urgency }
            switch (a.section, b.section) {
            case (.item, .item): return a.index < b.index
            case (.item, .project): return true
            case (.project, .item): return false
            case (.project(let an, let ai), .project(let bn, let bi)): return ai.count != bi.count ? ai.count > bi.count : an < bn
            }
        }.map(\.section)
    }
}
