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

    struct Current: Codable, Hashable { var number: Int; var title: String }
    struct Ticket: Codable, Hashable {
        var number: Int
        var title: String
        var state: String
        /// The ticket's own Loft issue, for a plan kept in Loft.
        var id: String? = nil
        /// Review role (qa, security…) to `pass` or still waiting.
        var verdicts: [String: String]? = nil
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

    /// The Delivery of a spec's issue: a GitHub spec by repo and number, a plan kept in Loft by its issue.
    static func delivery(of spec: HBTask, in all: [DeliverySummary]) -> DeliverySummary? {
        all.first { d in
            d.local ? d.specId == spec.id : spec.github.map { d.repo == $0.repo && d.spec == $0.number } ?? false
        }
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
    }

    /// A spec's tickets: its Delivery's when it has them, else its sub-issues in Loft (the web's `ticketsOf`).
    static func tickets(of spec: HBTask, _ d: DeliverySummary?, tasks: [HBTask]) -> [TicketLine] {
        if let d, !d.tickets.isEmpty {
            return d.tickets.map { t in
                let own = t.id.flatMap { id in tasks.first { $0.id == id } }
                    ?? tasks.first { x in spec.github.map { x.github?.repo == $0.repo && x.github?.number == t.number } ?? false }
                return TicketLine(id: "\(t.number)", number: own?.github?.number ?? t.number, title: t.title, state: t.state,
                                  taskId: own?.id, verdicts: t.verdicts ?? [:], current: d.current?.number == t.number)
            }
        }
        return tasks.filter { $0.parentId == spec.id && $0.state != .cancelled }.map { x in
            let state = x.state == .done ? "closed" : x.state == .inProgress ? "started" : [.blocked, .failed, .waitingApproval].contains(x.state) ? "needs_human" : "open"
            return TicketLine(id: x.id, number: x.github?.number, title: x.title, state: state, taskId: x.id, verdicts: [:], current: false)
        }
    }

    /// A ticket's state in a few words (the web's `ticketLabel`).
    static func ticketLabel(_ state: String) -> String {
        switch state {
        case "closed": "done"
        case "started": "building"
        case "review": "in review"
        case "closing": "closing"
        case "needs_human": "needs you"
        case "elsewhere": "elsewhere"
        default: "open"
        }
    }

    /// Start delivery: an open spec on GitHub whose Delivery hasn't started (the web's `isDeliverableSpec`).
    static func canStartDelivery(_ spec: HBTask, _ d: DeliverySummary?) -> Bool {
        isSpec(spec) && spec.github?.state == "open" && (d == nil || d?.state == "planned")
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
