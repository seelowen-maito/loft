import Foundation

/// `GET /api/deliveries/summary`: a spec's Delivery, with the spec it belongs to (backend `delivery::summary_route`).
struct DeliverySummary: Codable, Hashable {
    var state: String
    var agentName: String?
    var repo: String
    var spec: Int
    var specId: String?
    var local: Bool
    /// The spec PR's number, once there is one.
    var pr: Int? = nil
    var current: Current?
    var tickets: [Ticket]
    /// Ready tickets nobody has yet, while the Lead hands them out.
    var waitingForLead: [Int]? = nil
    var waitingOnYou: [Int]? = nil
    /// What its tickets wait on outside it.
    var waitsOn: [WaitsOn]? = nil
    /// Its state line as the server words it (the same as the issue page's); nil from an older backend.
    var stateLine: IssueDelivery.StateLine? = nil
    /// The spec PR's Checks tab on GitHub; nil in a plan kept in Loft, before the PR opens or from an older backend.
    var checksUrl: String? = nil
    /// When its earliest ticket waiting for CI started waiting (RFC 3339); nil when none is, or from an older backend.
    var ciSince: String? = nil

    struct Current: Codable, Hashable { var number: Int; var title: String }
    struct Ticket: Codable, Hashable {
        var number: Int
        var title: String
        var state: String
        /// The ticket's own Loft issue, for a plan kept in Loft.
        var id: String? = nil
        /// The agent on it while it's started, in review, closing or needs you; nil otherwise.
        var agentName: String? = nil
        /// Review role (qa, security…) to `pass` or still waiting.
        var verdicts: [String: String]? = nil
        /// Open tickets in another spec or repo it waits on (`owner/repo#n`).
        var waitingOn: [String]? = nil
        /// Its reviewers all passed, so only its CI is left and no agent is on it, since `ciSince` (RFC 3339).
        var waitingForCi: Bool? = nil
        var ciSince: String? = nil
    }
}

/// Where a spec stands; the same rules as the web's `specViews.ts`.
enum SpecStage: String, CaseIterable, Identifiable {
    case planned, delivery, ready, shipped, dropped

    var id: String { rawValue }
    var label: String {
        switch self {
        case .planned: "Planned"
        case .delivery: "In Delivery"
        case .ready: "Ready to merge"
        case .shipped: "Shipped"
        case .dropped: "Dropped"
        }
    }
}

enum SpecViews {
    static func isSpec(_ t: HBTask) -> Bool { (t.labels ?? []).contains { $0.lowercased() == "spec" } }

    /// Deliveries by the spec they belong to: a plan kept in Loft by its issue, a GitHub spec by repo and number.
    static func index(_ all: [DeliverySummary]) -> [String: DeliverySummary] {
        Dictionary(all.compactMap { d in (d.local ? d.specId.map { "id:\($0)" } : "\(d.repo)#\(d.spec)").map { ($0, d) } },
                   uniquingKeysWith: { first, _ in first })
    }

    /// The Delivery of a spec's issue, from `index(_:)`.
    static func delivery(of spec: HBTask, in index: [String: DeliverySummary]) -> DeliverySummary? {
        index["id:\(spec.id)"] ?? spec.github.flatMap { index["\($0.repo)#\($0.number)"] }
    }

    /// A Delivery's state in a few words, the same in a spec's row and its detail.
    static func deliveryLabel(_ state: String) -> String {
        switch state {
        case "planned": "Not started"
        case "running": "Running"
        case "needs_human": "Needs you"
        case "ready", "settled": "Ready to merge"
        case "merged": "Merged"
        default: state.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    /// A spec row's badge (the web's `badgeText`): "Waiting for CI" when no agent is at work and only CI is left (the
    /// server's `ci` tone), else the Delivery's state.
    static func badge(_ d: DeliverySummary) -> String {
        d.stateLine?.tone == "ci" ? "Waiting for CI" : deliveryLabel(d.state)
    }

    /// How long something has waited since `since` (RFC 3339), like the web's `fmtWait`: "<1 min", "6 min", "1 h",
    /// "2 h 5 min". Nil when `since` doesn't parse; a time ahead of `now` (clock skew) reads "<1 min".
    static func waited(since: String?, now: Date = Date()) -> String? {
        guard let since, let at = JSONCoding.parseDate(since) else { return nil }
        let m = Int(max(0, now.timeIntervalSince(at)) / 60)
        if m < 1 { return "<1 min" }
        if m < 60 { return "\(m) min" }
        return m % 60 == 0 ? "\(m / 60) h" : "\(m / 60) h \(m % 60) min"
    }

    /// Who is on which ticket, one line per running ticket: "Backend Agent on #43 Backend endpoints".
    static func onIt(_ d: DeliverySummary) -> [String] {
        d.tickets.compactMap { t in t.agentName.map { "\($0) on #\(t.number) \(t.title)" } }
    }

    /// "Waiting for the Lead to assign #44, #47", or nil when no ready ticket waits.
    static func waitingLine(_ numbers: [Int]?) -> String? {
        guard let numbers, !numbers.isEmpty else { return nil }
        return "Waiting for the Lead to assign " + numbers.map { "#\($0)" }.joined(separator: ", ")
    }

    /// "Waiting on you: #44", or nil when no ticket waits for the user.
    static func waitingOnYouLine(_ numbers: [Int]?) -> String? {
        guard let numbers, !numbers.isEmpty else { return nil }
        return "Waiting on you: " + numbers.map { "#\($0)" }.joined(separator: ", ")
    }

    /// Merged, or closed as completed, is Shipped; closed as not planned is Dropped; no Delivery or not started is Planned;
    /// ready or settled is Ready to merge; the rest is In Delivery.
    static func stage(of spec: HBTask, _ d: DeliverySummary?) -> SpecStage {
        if d?.state == "merged" { return .shipped }
        if spec.state.isClosed { return spec.state == .cancelled ? .dropped : .shipped }
        guard let d, d.state != "planned" else { return .planned }
        return d.state == "ready" || d.state == "settled" ? .ready : .delivery
    }

    /// `owner/repo#12`, or Loft and the project of a plan kept in Loft.
    static func ref(_ spec: HBTask) -> String {
        if let g = spec.github { return "\(g.repo)#\(g.number)" }
        return "Loft" + (spec.project.isEmpty ? "" : " · \(spec.project)")
    }

    /// The project a spec row shows next to its ref; nil when the ref already names it (a plan kept in Loft) or there is none.
    static func project(_ spec: HBTask) -> String? {
        spec.github == nil || spec.project.isEmpty ? nil : spec.project
    }

    /// Tickets done of all: the Delivery's, else GitHub's sub-issue count, else the spec's sub-issues in Loft; nil when none.
    static func progress(of spec: HBTask, _ d: DeliverySummary?, tasks: [HBTask]) -> (done: Int, total: Int)? {
        if let d, !d.tickets.isEmpty { return (d.tickets.filter { $0.state == "closed" }.count, d.tickets.count) }
        if let g = spec.github, let total = g.subTotal, total > 0 { return (g.subDone ?? 0, total) }
        let kids = tasks.filter { $0.parentId == spec.id && $0.state != .cancelled }
        return kids.isEmpty ? nil : (kids.filter { $0.state == .done }.count, kids.count)
    }

    /// A ticket as the spec's detail lists it, with its own issue (to open it) when Loft has one.
    struct TicketLine: Identifiable, Hashable {
        let id: String
        var number: Int?
        var title: String
        var state: String
        var taskId: String?
        var verdicts: [String: String]
        var current: Bool
        var waitingOn: [String] = []
        /// Only its CI is left (no agent on it), since `ciSince`.
        var waitingForCi = false
        var ciSince: String? = nil

        /// The review chips: a ticket waiting for CI says so in its label, so it has no "ci waiting" chip.
        var shownVerdicts: [(role: String, verdict: String)] {
            verdicts.filter { !(waitingForCi && $0.key == "ci") }.sorted { $0.key < $1.key }.map { ($0.key, $0.value) }
        }
    }

    /// A spec's tickets: its Delivery's when it has them, else its sub-issues in Loft (the web's `ticketsOf`).
    static func tickets(of spec: HBTask, _ d: DeliverySummary?, tasks: [HBTask]) -> [TicketLine] {
        if let d, !d.tickets.isEmpty {
            return d.tickets.map { t in
                let own = t.id.flatMap { id in tasks.first { $0.id == id } }
                    ?? tasks.first { x in spec.github.map { x.github?.repo == $0.repo && x.github?.number == t.number } ?? false }
                return TicketLine(id: "\(t.number)", number: own?.github?.number ?? t.number, title: t.title, state: t.state,
                                  taskId: own?.id, verdicts: t.verdicts ?? [:], current: t.agentName != nil, waitingOn: t.waitingOn ?? [],
                                  waitingForCi: t.waitingForCi == true, ciSince: t.ciSince)
            }
        }
        return tasks.filter { $0.parentId == spec.id && $0.state != .cancelled }.map { x in
            let state = x.state == .done ? "closed" : x.state == .inProgress ? "started" : [.blocked, .failed, .waitingApproval].contains(x.state) ? "needs_human" : "open"
            return TicketLine(id: x.id, number: x.github?.number, title: x.title, state: state, taskId: x.id, verdicts: [:], current: false)
        }
    }

    /// A ticket's state in a few words (the web's `ticketLabel`): an open one waiting on tickets in another spec
    /// says which; one in review whose reviewers all passed is waiting for CI, for how long since `ciSince`.
    static func ticketLabel(_ state: String, waitingOn: [String] = [], waitingForCi: Bool = false, ciSince: String? = nil, now: Date = Date()) -> String {
        if state == "open" && !waitingOn.isEmpty { return "waiting on " + waitingOn.joined(separator: ", ") }
        if state == "review" && waitingForCi { return "waiting for CI" + (waited(since: ciSince, now: now).map { " · \($0)" } ?? "") }
        return switch state {
        case "closed": "done"
        case "started": "building"
        case "review": "in review"
        case "closing": "closing"
        case "needs_human": "Waiting on you"
        case "elsewhere": "elsewhere"
        default: "open"
        }
    }

    /// Merge: a plan kept in Loft whose reviews passed; a spec on GitHub merges there.
    static func canMerge(_ spec: HBTask, _ d: DeliverySummary?) -> Bool {
        d?.local == true && d?.state == "ready" && !spec.state.isClosed
    }

    /// Whether an issue is in a group: the group's agents' projects, and the issues assigned to one of them.
    static func inGroup(_ t: HBTask, _ group: String, agents: [Agent]) -> Bool {
        agents.contains { $0.team == group && ((!t.project.isEmpty && $0.allProjects.contains(t.project)) || $0.id == t.agentId) }
    }
}
