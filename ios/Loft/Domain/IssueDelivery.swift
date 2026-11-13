import Foundation

/// A Delivery as an issue page shows it (`GET /api/issues/{id}/delivery`, backend delivery.rs `delivery_json`).
/// A spec's also lists its tickets: "elsewhere" ones are being built outside it (`skipped` says why).
struct IssueDelivery: Codable, Hashable {
    struct Ticket: Codable, Hashable {
        var number: Int
        var title: String
        /// open | started | review | needs_human | closed | elsewhere
        var state: String
        var skipped: String
        /// The agent on it while it's started, in review, closing or needs you; nil otherwise.
        var agentName: String? = nil
        /// Open tickets in another spec or repo it waits on (`owner/repo#n`).
        var waitingOn: [String]? = nil
        /// Its reviewers all passed, so only its CI is left and no agent is on it, since `ciSince` (RFC 3339).
        var waitingForCi: Bool? = nil
        var ciSince: String? = nil
    }
    struct Current: Codable, Hashable {
        var number: Int
        var title: String
    }
    /// The card's state line as the server words it: `tone` is running, ci, stalled, paused, needs_you, final or done.
    struct StateLine: Codable, Hashable {
        var tone: String
        var text: String
    }

    /// planned | running | final_review | final_fixes | needs_human | stalled | paused | ready | settled | merged
    var state: String
    /// Why it stalled: run_failed, no_call, restart or ci_missing; nil unless stalled.
    var stallReason: String? = nil
    /// The server's state line; nil before it started, and from a backend older than `state_line`.
    var stateLine: StateLine? = nil
    /// Pause applies: it's running (its final review and fixes included), and no ticket waits for the user.
    var canPause: Bool? = nil
    /// Resume applies: it's stalled or paused, or a ticket or its final review waits for the user.
    var canResume: Bool? = nil
    /// Reopen delivery applies: it's done, and tickets published into the spec since are open.
    var canReopen: Bool? = nil
    /// A ticket, or the final review, waits for the user after too many change rounds.
    var needsHuman: Bool? = nil
    var started: Bool
    var agentId: String?
    var agentName: String?
    var pr: Int?
    /// Its PR's Checks tab on GitHub (the spec PR, or a standalone ticket's PR); nil in a plan kept in Loft, before
    /// the PR opens or from an older backend.
    var checksUrl: String? = nil
    var spec: Bool? = nil
    var current: Current? = nil
    var tickets: [Ticket]? = nil
    /// Ready tickets nobody has yet, while the Lead hands them out.
    var waitingForLead: [Int]? = nil
    var waitingOnYou: [Int]? = nil
    /// What the spec's tickets wait on outside it: starting it before those close leaves them waiting.
    var waitsOn: [WaitsOn]? = nil

    /// How a card's state line reads: the colour it takes.
    enum Tone: String { case running, ci, stalled, paused, needsYou = "needs_you", final, done }

    /// The line the card shows, or nil before it started: the server's `state_line` as is (a tone this build
    /// doesn't know takes running's colour). A backend older than it sends none, so it's worded here like the web
    /// card's fallback (deliveryState.ts): waiting for the user (a ticket after too many change rounds, or the
    /// final review) wins over running.
    var line: (tone: Tone, text: String)? { line() }

    /// The card's line at `now`: waiting for CI adds how long its earliest ticket has waited ("· 6 min"), like the
    /// web's `deliveryStateLine`.
    func line(now: Date = Date()) -> (tone: Tone, text: String)? {
        guard started else { return nil }
        if let stateLine {
            let tone = Tone(rawValue: stateLine.tone) ?? .running
            guard tone == .ci, let since = ciSince, let waited = SpecViews.waited(since: since, now: now) else { return (tone, stateLine.text) }
            return (tone, "\(stateLine.text) · \(waited)")
        }
        switch state {
        case "stalled": return (.stalled, "Stalled: \(Self.stallWhy(stallReason)).")
        case "paused": return (.paused, "Paused: Loft sends no wakes until you resume it.")
        case "needs_human": return (.needsYou, "Needs you: too many rounds of changes in the final review.")
        case "final_review": return (.final, "Final review: the Code reviewer is reviewing the whole PR.")
        case "final_fixes": return (.final, "Final review: the Dev is fixing what the Code reviewer found.")
        case "ready", "settled", "merged": return (.done, "Done: every review passed.")
        default: break
        }
        if needsHuman == true { return (.needsYou, "Needs you: a ticket had too many rounds of changes.") }
        return (.running, "Running.")
    }

    /// When its earliest ticket waiting for CI started waiting (RFC 3339), or nil when none is.
    var ciSince: String? {
        (tickets ?? []).filter { $0.waitingForCi == true }.compactMap { t in t.ciSince.flatMap { s in JSONCoding.parseDate(s).map { (s, $0) } } }
            .min { $0.1 < $1.1 }?.0
    }

    /// Where the line takes the user: the Checks tab on GitHub while it waits for CI, else nowhere.
    var lineLink: URL? {
        guard line?.tone == .ci else { return nil }
        return checksUrl.flatMap(URL.init(string:))
    }

    private static func stallWhy(_ reason: String?) -> String {
        switch reason {
        case "run_failed": "the run Loft woke for its step failed"
        case "restart": "its run was lost when Loft restarted"
        default: "the run Loft woke for its step ended without its call"
        }
    }

    /// Pause shows while it runs (final review included), Resume when it's stalled or paused or a ticket or the
    /// final review waits for the user: each only when the server says it applies, and only once it started.
    var showsPause: Bool { started && canPause == true }
    var showsResume: Bool { started && canResume == true }
    var showsReopen: Bool { started && canReopen == true }

    var ticketsDone: Int { (tickets ?? []).filter { $0.state == "closed" }.count }
    var skippedTickets: [Ticket] { (tickets ?? []).filter { $0.state == "elsewhere" } }
    /// Every ticket with an agent on it: "#43 Backend endpoints is with Backend Agent".
    var runningLines: [String] { (tickets ?? []).compactMap { t in t.agentName.map { "#\(t.number) \(t.title) is with \($0)" } } }

    /// The progress line once it started: who has which ticket, on which PR and, for a spec, how many are done. While
    /// it only waits for CI no agent is on it, so it names none.
    func progressLine(issue number: Int) -> String {
        let who = agentName ?? "the team"
        let onCI = line?.tone == .ci
        if spec == true {
            // Several tickets can run at once: the card lists each with its agent under this line.
            if !runningLines.isEmpty || onCI {
                return "\(pr.map { "On spec PR #\($0). " } ?? "")\(ticketsDone) of \((tickets ?? []).count) tickets done."
            }
            let with = current.map { "#\($0.number) \($0.title) is with \(who)" } ?? "With \(who)"
            return "\(with)\(pr.map { " on spec PR #\($0)" } ?? ""). \(ticketsDone) of \((tickets ?? []).count) tickets done."
        }
        if onCI {
            return "Its reviewers passed\(pr.map { " on PR #\($0)" } ?? ""); Loft waits for its CI. #\(number) closes when the PR merges."
        }
        if agentName != nil {
            return "With \(who)\(pr.map { " on PR #\($0)" } ?? ""). Its reviewers look at it next; #\(number) closes when the PR merges."
        }
        return "Started\(pr.map { " on PR #\($0)" } ?? ""); the agent building it is gone. #\(number) closes when the PR merges."
    }
}

/// An open ticket outside a spec that one of its tickets waits on (`owner/repo#n`), and the Delivery that owns it
/// (nil when it has none). Backend delivery.rs `waits_on`.
struct WaitsOn: Codable, Hashable {
    struct Owner: Codable, Hashable {
        var repo: String
        var spec: Int
        var started: Bool
        /// The spec's Loft issue, to start its Delivery from here.
        var issueId: String?
    }
    var blocker: String
    var delivery: Owner?

    /// A spec to start first: its repo, number and Loft issue.
    struct First: Hashable { var repo: String; var spec: Int; var issueId: String }

    /// The specs to start first: each other spec that owns an open blocker and hasn't started, once (the web's
    /// `specsToStartFirst`).
    static func specsToStartFirst(_ waits: [WaitsOn]?) -> [First] {
        var out: [First] = []
        for case let d? in (waits ?? []).map(\.delivery) where !d.started {
            if let id = d.issueId, !out.contains(where: { $0.repo == d.repo && $0.spec == d.spec }) {
                out.append(First(repo: d.repo, spec: d.spec, issueId: id))
            }
        }
        return out
    }
}

/// A Delivery card button, as its endpoint names it (`POST /api/issues/{id}/<action>-delivery`).
enum IssueDeliveryAction: String { case start, pause, resume, reopen }

/// A Delivery card button's answer: the Delivery as the card shows it now, and what the button did.
struct IssueDeliveryReply: Decodable {
    var delivery: IssueDelivery
    var message: String?

    private enum Keys: String, CodingKey { case message }
    init(from decoder: Decoder) throws {
        delivery = try IssueDelivery(from: decoder)
        message = try decoder.container(keyedBy: Keys.self).decodeIfPresent(String.self, forKey: .message)
    }
}

extension HBTask {
    private var openOnGitHub: Bool { !state.isClosed && github?.state == "open" }

    /// A standalone ticket on GitHub (open, `ready-for-agent`, under no spec): Loft runs it as a Delivery of its
    /// own. A spec's ticket has a parent, or names its spec in the ticket template's `## Parent` section.
    var isStandaloneTicket: Bool {
        openOnGitHub && (labels ?? []).contains("ready-for-agent") && parentId == nil
            && description.range(of: #"(?m)^## Parent\s+#\d+"#, options: .regularExpression) == nil
    }

    /// An open spec on GitHub (labelled `spec`): Loft delivers its open sub-issues from its page.
    var isDeliverableSpec: Bool { openOnGitHub && (labels ?? []).contains { $0.lowercased() == "spec" } }

    /// The issue page shows a Delivery card: on a standalone ticket or a spec.
    var hasDeliveryCard: Bool { isStandaloneTicket || isDeliverableSpec }
}
