import Foundation

/// What the Delivery progress widget shows (spec #462): the picked spec's title, its tickets closed out of the total,
/// the ticket being built and its Dev, its reviews' verdicts, and whether it runs, stalled or finished.
public struct DeliveryWidgetSnapshot: Equatable {
    public enum Phase: Equatable {
        /// The app hasn't connected to a Loft yet.
        case notConnected
        /// Connected, but Loft didn't answer.
        case unreachable
        /// Nobody picked a Delivery yet.
        case notChosen
        /// The picked spec's Delivery is gone.
        case gone
        case ready
    }

    /// Where the Delivery stands, worded for the widget.
    public enum Status: Equatable {
        case running, finalReview, needsYou, stalled, paused, done, merged

        /// Its Delivery `state` (and `waiting_on_you`), the board's rules: the user waited on wins over running.
        init(_ d: DeliverySummary) {
            switch d.state {
            case "stalled": self = .stalled
            case "paused": self = .paused
            case "needs_human": self = .needsYou
            case "final_review", "final_fixes": self = .finalReview
            case "ready", "settled": self = .done
            case "merged": self = .merged
            default: self = (d.waitingOnYou ?? []).isEmpty ? .running : .needsYou
            }
        }

        public var label: String {
            switch self {
            case .running: "Running"
            case .finalReview: "Final review"
            case .needsYou: "Needs you"
            case .stalled: "Stalled"
            case .paused: "Paused"
            case .done: "Done"
            case .merged: "Merged"
            }
        }

        public var finished: Bool { self == .done || self == .merged }
    }

    /// One of the reviews' verdicts.
    public enum Verdict: Equatable { case pending, passed, failed }

    public var phase: Phase
    /// The spec's key, as its widget configuration holds it: `owner/repo#42`, or `id:<issue>` for a plan kept in Loft.
    public var key = ""
    /// "#462 Widgets: Needs you with actions": the spec, as the picker names it.
    public var title = ""
    /// The spec's Loft issue, which a tap opens; nil when the app doesn't have it.
    public var specTaskId: String?
    public var status = Status.running
    public var done = 0
    public var total = 0
    /// "#465 One-agent widget" and the Dev building it; nil when no ticket is being built.
    public var ticket: String?
    public var dev: String?
    /// "#464 Settings screen": the ticket waiting for the user, when one is.
    public var waitingOnYou: String?

    /// "2 of 4 tickets closed", "0 of 1 ticket closed".
    public var progress: String { "\(done) of \(total) ticket\(total == 1 ? "" : "s") closed" }
    /// The final Code Review; QA and Security are the ticket in review's (every ticket's once it's done).
    public var review = Verdict.pending
    public var qa = Verdict.pending
    public var security = Verdict.pending

    public init(phase: Phase, key: String = "", title: String = "") {
        self.phase = phase
        self.key = key
        self.title = title
    }

    /// `title` is what the widget's configuration remembers, shown when the Delivery is gone.
    public init(key: String, title: String, deliveries: [DeliverySummary], tasks: [HBTask]) {
        guard let d = deliveries.first(where: { Self.key($0) == key }) else {
            self.init(phase: .gone, key: key, title: title)
            return
        }
        let spec = Self.specTask(d, in: tasks)
        self.init(phase: .ready, key: key, title: spec.map { Self.title(d, $0) } ?? (title.isEmpty ? "#\(d.spec)" : title))
        specTaskId = spec?.id
        status = Status(d)
        // Tickets built outside the spec don't count.
        let counted = d.tickets.filter { $0.state != "elsewhere" }
        total = counted.count
        done = counted.filter { $0.state == "closed" }.count
        if !status.finished {
            let building = counted.first { $0.agentName != nil && ["started", "review", "changes", "closing"].contains($0.state) }
            if let t = building {
                ticket = "#\(t.number) \(t.title)"
                dev = t.agentName
            } else if let c = d.current {
                ticket = "#\(c.number) \(c.title)"
                dev = d.agentName
            }
            waitingOnYou = (d.waitingOnYou ?? []).first.map { n in "#\(n)\(counted.first { $0.number == n }.map { " \($0.title)" } ?? "")" }
        }
        review = switch d.state {
        case "ready", "settled", "merged": .passed
        case "final_fixes": .failed
        default: .pending
        }
        // Not the spec PR's labels: a ticket's pass labels stay on it for the tickets after. A ticket sent back has
        // its verdicts cleared and reads as changes (which reviewer asked isn't kept).
        if status.finished {
            (qa, security) = (.passed, .passed)
        } else if let t = counted.first(where: { ["review", "changes", "closing"].contains($0.state) }) {
            func verdict(_ role: String) -> Verdict { t.state == "changes" ? .failed : t.verdicts?[role] == "pass" ? .passed : .pending }
            (qa, security) = (verdict("qa"), verdict("security"))
        }
    }

    /// The configuration's key for a Delivery (`SpecViews.index`'s).
    public static func key(_ d: DeliverySummary) -> String { d.local ? "id:\(d.specId ?? "")" : "\(d.repo)#\(d.spec)" }

    static func specTask(_ d: DeliverySummary, in tasks: [HBTask]) -> HBTask? {
        tasks.first { d.local ? $0.id == d.specId : $0.github?.repo == d.repo && $0.github?.number == d.spec }
    }

    static func title(_ d: DeliverySummary, _ spec: HBTask) -> String { d.local ? spec.title : "#\(d.spec) \(spec.title)" }

    /// The Deliveries the picker offers, with their titles: the started ones, those still going first, then the most
    /// recently updated spec first.
    public static func choices(_ deliveries: [DeliverySummary], tasks: [HBTask]) -> [(key: String, title: String, repo: String)] {
        deliveries.filter { $0.state != "planned" }
            .map { d in (d, specTask(d, in: tasks)) }
            .sorted { a, b in
                let (fa, fb) = (Status(a.0).finished, Status(b.0).finished)
                if fa != fb { return !fa }
                return (a.1?.updatedAt ?? .distantPast) > (b.1?.updatedAt ?? .distantPast)
            }
            .map { d, spec in (key(d), spec.map { title(d, $0) } ?? "#\(d.spec)", d.repo) }
    }

    /// Reads the Delivery from Loft at the address the app connected to.
    public static func load(key: String?, title: String) async -> DeliveryWidgetSnapshot {
        guard let server = SharedServer.address else { return DeliveryWidgetSnapshot(phase: .notConnected) }
        guard let key else { return DeliveryWidgetSnapshot(phase: .notChosen) }
        let api = LiveLoftAPI(baseURL: server)
        return await load(key: key, title: title, deliveries: api.deliverySummaries, tasks: api.tasks, titles: .standard)
    }

    /// The Deliveries are required; the issues are an extra, so their failing leaves the title short instead of
    /// making Loft look unreachable. `titles` remembers each spec's title it showed, since the
    /// configuration loses it once the Delivery is gone.
    static func load(key: String, title: String, deliveries: () async throws -> [DeliverySummary], tasks: () async throws -> [HBTask],
                     titles: UserDefaults? = nil) async -> DeliveryWidgetSnapshot {
        let remembered = "deliveryWidget.title.\(key)"
        let title = title.isEmpty ? titles?.string(forKey: remembered) ?? "" : title
        do {
            async let list = deliveries()
            async let issues = try? tasks()
            let s = DeliveryWidgetSnapshot(key: key, title: title, deliveries: try await list, tasks: await issues ?? [])
            if s.phase == .ready { titles?.set(s.title, forKey: remembered) }
            return s
        } catch {
            return DeliveryWidgetSnapshot(phase: .unreachable, key: key, title: title)
        }
    }
}
