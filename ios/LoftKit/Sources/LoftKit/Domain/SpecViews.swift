import Foundation

/// `GET /api/deliveries/summary`: a spec's Delivery, with the spec it belongs to, or a standalone ticket's (`spec` is then
/// the ticket's number, `specId` its Loft issue, and it has no `tickets`) (backend `delivery::summary_route`).
public struct DeliverySummary: Codable, Hashable {
    public var state: String
    public var agentName: String?
    public var repo: String
    public var spec: Int
    public var specId: String?
    public var local: Bool
    /// A standalone ticket's Delivery; nil from an older backend, which sends none of those.
    public var standalone: Bool? = nil
    /// A standalone ticket waits for the user after too many rounds of changes (it has no tickets to say so).
    public var needsHuman: Bool? = nil
    /// The spec PR's number, once there is one.
    public var pr: Int? = nil
    public var current: Current?
    public var tickets: [Ticket]
    /// Ready tickets nobody has yet, while the Lead hands them out.
    public var waitingForLead: [Int]? = nil
    public var waitingOnYou: [Int]? = nil
    /// What its tickets wait on outside it.
    public var waitsOn: [WaitsOn]? = nil
    /// Its state line as the server words it (the same as the issue page's); nil from an older backend.
    public var stateLine: IssueDelivery.StateLine? = nil
    /// The spec PR's Checks tab on GitHub; nil in a plan kept in Loft, before the PR opens or from an older backend.
    public var checksUrl: String? = nil
    /// When its earliest ticket waiting for CI started waiting (RFC 3339); nil when none is, or from an older backend.
    public var ciSince: String? = nil

    public var isStandalone: Bool { standalone == true }

    public struct Current: Codable, Hashable { public var number: Int; public var title: String }
    public struct Ticket: Codable, Hashable {
        public var number: Int
        public var title: String
        public var state: String
        /// The ticket's own Loft issue, for a plan kept in Loft.
        public var id: String? = nil
        /// The agent on it while it's started, in review, closing or needs you; nil otherwise.
        public var agentName: String? = nil
        /// Review role (qa, security…) to `pass` or still waiting.
        public var verdicts: [String: String]? = nil
        /// Open tickets in another spec or repo it waits on (`owner/repo#n`).
        public var waitingOn: [String]? = nil
        /// Its reviewers all passed, so only its CI is left and no agent is on it, since `ciSince` (RFC 3339).
        public var waitingForCi: Bool? = nil
        public var ciSince: String? = nil
    }
}

/// Where a spec stands; the same rules as the web's `specViews.ts`.
public enum SpecStage: String, CaseIterable, Identifiable {
    case planned, delivery, ready, shipped, dropped

    public var id: String { rawValue }
    public var label: String {
        switch self {
        case .planned: "Planned"
        case .delivery: "In Delivery"
        case .ready: "Ready to merge"
        case .shipped: "Shipped"
        case .dropped: "Dropped"
        }
    }
}

public enum SpecViews {
    public static func isSpec(_ t: HBTask) -> Bool { (t.labels ?? []).contains { $0.lowercased() == "spec" } }

    /// Deliveries by the spec they belong to: a plan kept in Loft by its issue, a GitHub spec by repo and number.
    public static func index(_ all: [DeliverySummary]) -> [String: DeliverySummary] {
        Dictionary(all.compactMap { d in (d.local ? d.specId.map { "id:\($0)" } : "\(d.repo)#\(d.spec)").map { ($0, d) } },
                   uniquingKeysWith: { first, _ in first })
    }

    /// The Delivery of a spec's issue, from `index(_:)`.
    public static func delivery(of spec: HBTask, in index: [String: DeliverySummary]) -> DeliverySummary? {
        index["id:\(spec.id)"] ?? spec.github.flatMap { index["\($0.repo)#\($0.number)"] }
    }

    /// A Delivery's state in a few words, the same in a spec's row and its detail.
    public static func deliveryLabel(_ state: String) -> String {
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
    /// server's `ci` tone), "Needs you" when it waits for the user, else the Delivery's state.
    public static func badge(_ d: DeliverySummary) -> String {
        d.stateLine?.tone == "ci" ? "Waiting for CI" : waitsForYou(d) ? deliveryLabel("needs_human") : deliveryLabel(d.state)
    }

    /// The Delivery waits for the user: its final review, or a standalone ticket (which shows no tickets) after too many
    /// rounds of changes (the web's `waitsForYou`).
    public static func waitsForYou(_ d: DeliverySummary) -> Bool {
        d.state == "needs_human" || (d.isStandalone && d.needsHuman == true)
    }

    /// How long something has waited since `since` (RFC 3339), like the web's `fmtWait`: "<1 min", "6 min", "1 h",
    /// "2 h 5 min". Nil when `since` doesn't parse; a time ahead of `now` (clock skew) reads "<1 min".
    /// Non-breaking spaces, so a long title never wraps it mid-time ("1 h" / "9 min").
    static func waited(since: String?, now: Date = Date()) -> String? {
        guard let since, let at = JSONCoding.parseDate(since) else { return nil }
        let m = Int(max(0, now.timeIntervalSince(at)) / 60)
        if m < 1 { return "<1\u{a0}min" }
        if m < 60 { return "\(m)\u{a0}min" }
        return m % 60 == 0 ? "\(m / 60)\u{a0}h" : "\(m / 60)\u{a0}h\u{a0}\(m % 60)\u{a0}min"
    }

    /// Who is on which ticket, one line per running ticket: "Backend Agent on #43 Backend endpoints". A standalone
    /// ticket has no tickets: its Dev, "Dev is on it", while its Delivery runs and doesn't wait for the user.
    public static func onIt(_ d: DeliverySummary) -> [String] {
        if d.isStandalone {
            guard let name = d.agentName, d.needsHuman != true, ["running", "final_review", "final_fixes"].contains(d.state) else { return [] }
            return ["\(name) is on it"]
        }
        return d.tickets.compactMap { t in t.agentName.map { "\($0) on #\(t.number) \(t.title)" } }
    }

    /// "Waiting for the Lead to assign #44, #47", or nil when no ready ticket waits.
    public static func waitingLine(_ numbers: [Int]?) -> String? {
        guard let numbers, !numbers.isEmpty else { return nil }
        return "Waiting for the Lead to assign " + numbers.map { "#\($0)" }.joined(separator: ", ")
    }

    /// "Waiting on you: #44", or nil when no ticket waits for the user.
    public static func waitingOnYouLine(_ numbers: [Int]?) -> String? {
        guard let numbers, !numbers.isEmpty else { return nil }
        return "Waiting on you: " + numbers.map { "#\($0)" }.joined(separator: ", ")
    }

    /// Merged, or closed as completed, is Shipped; closed as not planned is Dropped; no Delivery or not started is Planned;
    /// ready or settled is Ready to merge; the rest is In Delivery.
    public static func stage(of spec: HBTask, _ d: DeliverySummary?) -> SpecStage {
        if d?.state == "merged" { return .shipped }
        if spec.state.isClosed { return spec.state == .cancelled ? .dropped : .shipped }
        guard let d, d.state != "planned" else { return .planned }
        return d.state == "ready" || d.state == "settled" ? .ready : .delivery
    }

    /// `owner/repo#12`, or Loft and the project of a plan kept in Loft.
    public static func ref(_ spec: HBTask) -> String {
        if let g = spec.github { return "\(g.repo)#\(g.number)" }
        return "Loft" + (spec.project.isEmpty ? "" : " · \(spec.project)")
    }

    /// The project a spec row shows next to its ref, by its last path part (`pos`, not `dineout/dineout/pos`, which repeats
    /// the repo); nil when the ref already names it (a plan kept in Loft) or there is none.
    public static func project(_ spec: HBTask) -> String? {
        guard spec.github != nil, let last = spec.project.split(separator: "/").last else { return nil }
        return String(last)
    }

    /// Tickets done of all: the Delivery's, else GitHub's sub-issue count, else the spec's sub-issues in Loft; nil when none,
    /// and for a standalone ticket, whose card is a ticket's.
    public static func progress(of spec: HBTask, _ d: DeliverySummary?, tasks: [HBTask]) -> (done: Int, total: Int)? {
        if d?.isStandalone == true { return nil }
        if let d, !d.tickets.isEmpty { return (d.tickets.filter { $0.state == "closed" }.count, d.tickets.count) }
        if let g = spec.github, let total = g.subTotal, total > 0 { return (g.subDone ?? 0, total) }
        let kids = tasks.filter { $0.parentId == spec.id && $0.state != .cancelled }
        return kids.isEmpty ? nil : (kids.filter { $0.state == .done }.count, kids.count)
    }

    /// A ticket as the spec's detail lists it, with its own issue (to open it) when Loft has one.
    public struct TicketLine: Identifiable, Hashable {
        public let id: String
        public var number: Int?
        public var title: String
        public var state: String
        public var taskId: String?
        public var verdicts: [String: String]
        public var current: Bool
        public var waitingOn: [String] = []
        /// Only its CI is left (no agent on it), since `ciSince`.
        public var waitingForCi = false
        public var ciSince: String? = nil

        /// The review chips: a ticket waiting for CI says so in its label, so it has no "ci waiting" chip.
        public var shownVerdicts: [(role: String, verdict: String)] {
            verdicts.filter { !(waitingForCi && $0.key == "ci") }.sorted { $0.key < $1.key }.map { ($0.key, $0.value) }
        }
    }

    /// A spec's tickets: its Delivery's when it has them, else its sub-issues in Loft (the web's `ticketsOf`).
    public static func tickets(of spec: HBTask, _ d: DeliverySummary?, tasks: [HBTask]) -> [TicketLine] {
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
    public static func ticketLabel(_ state: String, waitingOn: [String] = [], waitingForCi: Bool = false, ciSince: String? = nil, now: Date = Date()) -> String {
        if state == "open" && !waitingOn.isEmpty { return "waiting on " + waitingOn.joined(separator: ", ") }
        if state == "review" && waitingForCi { return "waiting for CI" + (waited(since: ciSince, now: now).map { " ·\u{a0}\($0)" } ?? "") }
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
    public static func canMerge(_ spec: HBTask, _ d: DeliverySummary?) -> Bool {
        d?.local == true && d?.state == "ready" && !spec.state.isClosed
    }

    /// Whether an issue is in a group: the group's agents' projects, and the issues assigned to one of them.
    public static func inGroup(_ t: HBTask, _ group: String, agents: [Agent]) -> Bool {
        agents.contains { $0.team == group && ((!t.project.isEmpty && $0.allProjects.contains(t.project)) || $0.id == t.agentId) }
    }
}

/// The Issues board as the web shows it (`SpecBoard.tsx`): the Issues column, the spec stage columns, or both.
public enum BoardType: String, CaseIterable, Identifiable {
    case all, issues, specs

    public var id: String { rawValue }
    public var label: String {
        switch self {
        case .all: "All"
        case .issues: "Issues"
        case .specs: "Specs"
        }
    }
}

extension SpecViews {
    /// A spec's ticket: it has a parent, names its spec in the template's `## Parent` section, or is in a spec's Delivery's
    /// tickets (a standalone ticket's has none) (the web's `isSpecTicket`).
    public static func isSpecTicket(_ t: HBTask, _ deliveries: [DeliverySummary]) -> Bool {
        t.parentId != nil
            || t.description.range(of: #"(?m)^## Parent\s+#\d+"#, options: .regularExpression) != nil
            || deliveries.contains { d in
                d.tickets.contains { x in x.id.map { $0 == t.id } ?? (t.github.map { d.repo == $0.repo && x.number == $0.number } ?? false) }
            }
    }

    /// Whether issue `t` is a standalone ticket whose Delivery started: it leaves the Issues column for the stage columns,
    /// like a spec (the web's `standaloneOnStages`). Before its Delivery starts it stays in Issues.
    public static func standaloneOnStages(_ t: HBTask, _ index: [String: DeliverySummary]) -> Bool {
        guard let d = delivery(of: t, in: index) else { return false }
        return d.isStandalone && d.state != "planned"
    }

    /// Whether issue `t` goes in the stage columns: a spec, or a standalone ticket whose Delivery started.
    public static func onStages(_ t: HBTask, _ index: [String: DeliverySummary]) -> Bool {
        isSpec(t) || standaloneOnStages(t, index)
    }

    /// The board's Issues column: the issues that are no spec, no spec's ticket and no standalone ticket in Delivery (that
    /// one is in the stage columns), closed ones only when asked for, open ones first, then the most recently updated
    /// (the web's `boardIssues`).
    public static func boardIssues(_ tasks: [HBTask], _ deliveries: [DeliverySummary], showClosed: Bool) -> [HBTask] {
        let byIssue = index(deliveries)
        return tasks.filter { !isSpec($0) && !isSpecTicket($0, deliveries) && !standaloneOnStages($0, byIssue) && (showClosed || !$0.state.isClosed) }
            .sorted { a, b in a.state.isClosed != b.state.isClosed ? !a.state.isClosed : a.updatedAt > b.updatedAt }
    }

    /// The specs, and the standalone tickets whose Delivery started, in one stage column, the most recently updated first.
    public static func specs(_ tasks: [HBTask], in stage: SpecStage, _ index: [String: DeliverySummary]) -> [HBTask] {
        tasks.filter { onStages($0, index) && self.stage(of: $0, delivery(of: $0, in: index)) == stage }
            .sorted { $0.updatedAt > $1.updatedAt }
    }
}
