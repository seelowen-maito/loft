#if DEBUG
import Foundation

/// Deterministic in-memory server for UI tests (`-ui-testing`). Debug builds only;
/// the shipping app always talks to a real Loft.
final class FixtureAPI: LoftAPI, @unchecked Sendable {
    enum Mode { case normal, empty, offline, failSends }

    let baseURL = URL(string: "https://fixture.invalid")!
    private let mode: Mode
    private let lock = NSLock()
    private var agentList: [Agent]
    private var taskList: [HBTask]
    private var msgs: [String: [ChatMessage]]
    private var perms: [PermissionRequest]
    private var nextId: Int64 = 1000
    /// `-fixture-grill`: the server supports grill mode and the Orchestrator is mid-interview (`-fixture-grill-ended`: just done).
    private let grill: Bool
    private var grillState: GrillSession?
    /// Commands the app sent, for UI tests to check.
    private(set) var grillCommandsSent: [GrillCommand] = []
    /// The next grill command fails with this, as when the network drops (tests).
    var failNextGrillCommand: APIError?
    /// Team setup messages the server sent to `loft`, for tests to check it happens once.
    private(set) var teamSetupsSent = 0

    /// The text after `-<name>` on the command line. A value starting with `base64:` is decoded first: free text
    /// with an apostrophe reached the app mangled ("isn't hidden" as "isnt\ hidden") on its way through
    /// xcodebuild and the Simulator, so UI tests send such text encoded.
    static func launchText(_ name: String) -> String? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-" + name), i + 1 < args.count else { return nil }
        let v = args[i + 1]
        guard v.hasPrefix("base64:") else { return v }
        return Data(base64Encoded: String(v.dropFirst(7))).flatMap { String(data: $0, encoding: .utf8) }
    }

    static func fromLaunchArguments() -> FixtureAPI {
        let args = ProcessInfo.processInfo.arguments
        let mode: Mode = args.contains("-fixture-offline") ? .offline
            : args.contains("-fixture-empty") ? .empty
            : args.contains("-fixture-fail-sends") ? .failSends : .normal
        return FixtureAPI(mode: mode, grill: args.contains("-fixture-grill"), replies: args.contains("-fixture-replies"))
    }

    /// The agent answers each message two seconds later, with no live event (like one missed in a reconnect).
    private let replies: Bool

    /// `grillEnded` and `setupSent` are `-fixture-grill-ended` and `-fixture-setup-sent` for unit tests.
    init(mode: Mode = .normal, grill: Bool = false, replies: Bool = false, grillEnded: Bool = false, setupSent: Bool = false) {
        self.mode = mode
        self.replies = replies
        self.grill = grill && mode != .empty
        let now = Date()
        func ago(_ m: Double) -> Date { now.addingTimeInterval(-m * 60) }
        func agent(_ id: String, _ name: String, _ project: String, _ status: String, _ text: String, running: Bool, last: Double, ctx: Int? = nil) -> Agent {
            Agent(id: id, name: name, title: nil, project: project, model: "opus", effort: nil, permissionMode: "auto",
                  intervalMinutes: 60, status: status, statusText: text, running: running, lastRunAt: ago(last),
                  nextRunAt: nil, createdAt: ago(10000), totalCostUsd: 3.2, contextTokens: ctx, prompt: nil,
                  sessionId: ctx == nil ? nil : "s-\(id)")
        }
        func task(_ id: String, _ title: String, _ status: String, agent: String?, parent: String? = nil, by: String = "user",
                  note: String = "", error: String = "", priority: String = "normal", minutes: Double) -> HBTask {
            HBTask(id: id, title: title, description: "", kind: agent == nil ? "todo" : "work", status: status, priority: priority,
                   project: "loft", agentId: agent, parentId: parent, dependsOn: [], dueAt: nil, createdBy: by, note: note,
                   summary: status == "done" ? "Finished and pushed." : "", error: error, createdAt: ago(minutes), updatedAt: ago(minutes / 2),
                   startedAt: status == "inbox" ? nil : ago(minutes), completedAt: status == "done" ? ago(1) : nil)
        }
        if mode == .empty {
            agentList = []; taskList = []; msgs = [:]; perms = []
            return
        }
        agentList = [
            agent("orch", "Orchestrator", "loft", "running", "Planning the settings work", running: true, last: 12, ctx: 182_000),
            agent("be", "Backend Agent", "loft", "running", "Running cargo test", running: true, last: 8, ctx: 64_000),
            agent("ios", "iOS Agent", "loft", "waiting", "Waiting for permission: Bash", running: true, last: 5, ctx: 214_000),
            agent("qa", "QA Agent", "loft", "idle", "", running: false, last: 90, ctx: 12_000),
            agent("docs", "Docs Agent", "website", "error", "Last run failed", running: false, last: 30),
        ]
        // `-fixture-run-waiting`: the Orchestrator replied and waits on a background task; `-fixture-run-silent`: no output for a while.
        let args = ProcessInfo.processInfo.arguments
        // `-fixture-lead`: QA leads loft (initials, no sticker) and the Docs Agent leads website (with its sticker).
        if args.contains("-fixture-lead") {
            agentList[3].lead = true
            agentList[4].lead = true
        }
        // `-fixture-many-agents`: six more idle agents, more than fit in the @ picker without scrolling.
        if args.contains("-fixture-many-agents") {
            for name in ["Android Agent", "Data Agent", "Design Agent", "Infra Agent", "Security Agent", "Web Agent"] {
                agentList.append(agent(name.lowercased().replacingOccurrences(of: " ", with: "-"), name, "loft", "idle", "",
                                       running: false, last: 120))
            }
        }
        if args.contains("-fixture-run-waiting") {
            agentList[0].waitingOn = ["Run the QA suite", "Build the docs site"]
            agentList[0].waitingSince = ago(14)
            agentList[0].lastOutputAt = ago(14)
        } else if args.contains("-fixture-run-silent") {
            agentList[0].lastOutputAt = ago(23)
        } else {
            agentList[0].lastOutputAt = ago(1)
        }
        // QA Agent has no sticker, so the initials fallback stays covered.
        for (id, svg) in FixtureStickers.byAgent {
            if let i = agentList.firstIndex(where: { $0.id == id }) { agentList[i].avatar = Avatar(svg: svg) }
        }
        taskList = [
            task("t1", "Implement settings functionality", "in_progress", agent: "orch", priority: "high", minutes: 40),
            task("t2", "Implement backend endpoints", "in_progress", agent: "be", parent: "t1", by: "orch", note: "3 of 4 endpoints done", minutes: 30),
            task("t3", "Build the iOS settings screen", "waiting_approval", agent: "ios", parent: "t1", by: "orch", minutes: 30),
            task("t4", "Test the integrated flow", "blocked", agent: "qa", parent: "t1", by: "orch", error: "Waiting for the backend and iOS work", minutes: 30),
            task("t5", "Inspect existing architecture", "done", agent: "orch", parent: "t1", by: "orch", minutes: 39),
            task("t6", "Renew the domain", "inbox", agent: nil, minutes: 300),
            task("t7", "Fix broken docs links", "failed", agent: "docs", error: "Link checker crashed", minutes: 60),
        ]
        func m(_ id: Int64, _ agent: String, _ role: String, _ kind: String, _ content: String, _ minutes: Double, meta: JSONValue? = nil) -> ChatMessage {
            ChatMessage(id: id, agentId: agent, runId: nil, role: role, kind: kind, content: content, meta: meta, createdAt: ago(minutes))
        }
        msgs = [
            "orch": [
                m(1, "orch", "user", "text", "Implement a new settings screen, add the backend endpoints, and test the whole flow.", 41),
                m(2, "orch", "assistant", "text", "On it. I'll split this into backend, iOS and QA subtasks.", 40),
                m(3, "orch", "assistant", "tool_use", "Creating task: Implement backend endpoints", 39, meta: ["name": "mcp__loft__create_task"]),
                m(4, "orch", "assistant", "tool_use", "Creating task: Build the iOS settings screen", 39, meta: ["name": "mcp__loft__create_task"]),
                m(5, "orch", "assistant", "text", "Delegated: **Backend Agent** has the endpoints, **iOS Agent** the screen, and **QA Agent** tests once both land.", 38),
                m(9, "orch", "system", "notice", "Plan: Implement settings functionality", 38, meta: ["plan_task_id": "t1"]),
                // An @-mention and the mentioned agent's reply, copied into this chat (#92).
                m(10, "orch", "user", "text", "@QA Agent which simulator do you use for the suite?", 37,
                  meta: ["mentions": .array([["id": "qa", "name": "QA Agent"]]), "mentions_only": true]),
                m(11, "orch", "assistant", "text", "The iPhone 17 Pro on iOS 26.5. I can switch if you need another one.", 36,
                  meta: ["from_agent": ["id": "qa", "name": "QA Agent"], "mention_reply": true]),
                // A task handed to the orchestrator, and reports from the agents it delegated to (`meta.task_role`).
                m(6, "orch", "user", "text", "[Task t6] Renew the domain\n\nThe loft.dev domain expires on Nov 3. Renew it for two years and turn on auto-renew.\n\n[Report progress on this task with update_task.]", 20,
                  meta: ["task_id": "t6", "task_role": "brief"]),
                m(7, "orch", "user", "text", "[Task t2] “Implement backend endpoints”, which you delegated to me, is now done: All 4 endpoints are in, with tests.", 12,
                  meta: ["task_id": "t2", "task_role": "report", "task_status": "done", "from_agent": ["id": "be", "name": "Backend Agent"]]),
                // Agent messages: needs action, and an FYI read on the next run (#112).
                m(12, "orch", "user", "text", "Please review PR #41 before I merge it.", 9, meta: ["from_agent": ["id": "be", "name": "Backend Agent"]]),
                m(13, "orch", "user", "text", "Merged #40 on main; the endpoints are live.", 8, meta: ["from_agent": ["id": "be", "name": "Backend Agent"], "fyi": true]),
                m(8, "orch", "user", "text", "[Task t4] “Test the integrated flow”, which you delegated to me, is now blocked: Waiting for the backend and iOS work", 6,
                  meta: ["task_id": "t4", "task_role": "report", "task_status": "blocked", "from_agent": ["id": "qa", "name": "QA Agent"]]),
            ],
            // A finished run, for the chat's "Done in" line.
            "qa": [
                m(30, "qa", "user", "text", "Run the smoke tests on the staging build.", 93),
                ChatMessage(id: 31, agentId: "qa", runId: "qa-r1", role: "system", kind: "run_start", content: "", meta: nil, createdAt: ago(93)),
                ChatMessage(id: 32, agentId: "qa", runId: "qa-r1", role: "assistant", kind: "text", content: "All 14 smoke tests passed on staging.",
                            meta: nil, createdAt: ago(90.7)),
                ChatMessage(id: 33, agentId: "qa", runId: "qa-r1", role: "system", kind: "run_end", content: "",
                            meta: ["duration_ms": .number(143_000)], createdAt: ago(93).addingTimeInterval(143)),
            ],
        ]
        perms = [
            PermissionRequest(id: "p1", agentId: "ios", source: "prompt", toolName: "Bash", input: ["command": "xcodebuild test -scheme Loft"],
                              reason: nil, rule: "Bash(xcodebuild *)", status: "pending", createdAt: ago(4)),
        ]
        if self.grill { addGrill(now: now) }
        // `-fixture-grill-ended`: the interview is over and its end-of-planning card is in the chat
        // (`-fixture-setup-sent`: the web already sent its team setup message).
        if self.grill, grillEnded || args.contains("-fixture-grill-ended"), var s = grillState {
            endGrill(&s, setupSent: setupSent || args.contains("-fixture-setup-sent"))
            grillState = s
        }
        if args.contains("-fixture-risky-permission") {
            // A request the explainer has to flag: it pushes to main over the network.
            perms.append(PermissionRequest(id: "p2", agentId: "be", source: "prompt", toolName: "Bash", input: ["command": "git push origin main"],
                                           reason: "Ship the settings endpoints", rule: "Bash(git push *)", status: "pending", createdAt: ago(2)))
        }
        if args.contains("-fixture-agent-asks") {
            // The Orchestrator's last word is a question: suggested replies show under it.
            msgs["orch"]?.append(m(20, "orch", "assistant", "text", "Tests are green on the settings PR. Should I merge it now or wait for QA?", 1))
        }
        if args.contains("-fixture-run-silent") {
            // Nothing new in the chat either: the last message is older than the silence.
            msgs["orch"] = msgs["orch"]?.map { var m = $0; m.createdAt = m.createdAt.addingTimeInterval(-25 * 60); return m }
        }
    }

    /// The Orchestrator mid-interview: round 1 sent with one answer and one question skipped, round 2 waiting
    /// with that question carried over, one with options and a link, and one more.
    private func addGrill(now: Date) {
        func ago(_ m: Double) -> Date { now.addingTimeInterval(-m * 60) }
        let sid = "g1", topic = "the pricing page"
        let q1 = GrillQuestion(id: "q1", text: "Who is the pricing page for: people seeing Loft for the first time, or customers comparing plans?",
                               speak: "Who is the pricing page for: first-time visitors, or customers comparing plans?",
                               recommendation: "First-time visitors")
        let q2 = GrillQuestion(id: "q2", text: "Which billing periods should we show?", kind: .singleChoice, options: ["Monthly", "Yearly", "Both"],
                               recommendation: "Both, with yearly highlighted")
        let round2 = [
            GrillQuestion(id: "q3", text: q2.text, kind: .singleChoice, options: q2.options, recommendation: q2.recommendation, fromRound: 1),
            GrillQuestion(id: "q4", text: "Should the Pro plan include priority support? The current tiers are in [PR #31](https://github.com/Octo/loft/pull/31).",
                          speak: "Should the Pro plan include priority support?", kind: .singleChoice, options: ["Yes", "No", "Only on yearly"],
                          recommendation: "Only on yearly"),
            GrillQuestion(id: "q5", text: "Should there be a free tier?", recommendation: "No: a 14-day trial instead"),
        ]
        grillState = GrillSession(sessionId: sid, agentId: "orch", state: .active, topic: topic, startedAt: ago(5),
                                  round: 2, total: round2.count, answered: 0, waiting: false, questions: round2, language: "en")
        let script: [(String, String, GrillMeta, Double)] = [
            ("user", "Grill me on the pricing page", GrillMeta(sessionId: nil, role: .startRequest), 5),
            ("assistant", "Grill started: the pricing page", GrillMeta(sessionId: sid, role: .status, state: "started"), 4.9),
            ("assistant", Self.roundText(1, [q1, q2]), GrillMeta(sessionId: sid, role: .round, round: 1, topic: topic, questions: [q1, q2]), 4.8),
            ("user", "Answers to round 1:\n\n1. Yes (First-time visitors)\n2. (no answer)", GrillMeta(sessionId: sid, role: .answers, round: 1), 4),
            ("assistant", Self.roundText(2, round2), GrillMeta(sessionId: sid, role: .round, round: 2, topic: topic, questions: round2), 2.9),
        ]
        for (i, (role, text, g, minutes)) in script.enumerated() {
            appendGrill(id: 100 + Int64(i), role: role, text: text, g, at: ago(minutes))
        }
    }

    /// A round as the backend writes it, for clients without cards.
    private static func roundText(_ n: Int, _ qs: [GrillQuestion]) -> String {
        "**Round \(n)**\n" + qs.enumerated().map { "\n\($0.offset + 1). \($0.element.text)\n   _Recommendation:_ \($0.element.recommendation)" }.joined(separator: "\n")
    }

    /// Holds an answer in the open round, as the backend does: "yes" accepts the recommendation (lock held).
    private func recordGrillAnswer(_ text: String, questionId: String, discuss: Bool) {
        guard var s = grillState, var qs = s.questions, let i = qs.firstIndex(where: { $0.id == questionId }) else { return }
        let yes = ["yes", "yep", "sure", "ok", "sim", "isso"].contains(VoiceText.normalize(text))
        qs[i].accepted = !discuss && yes
        qs[i].answer = qs[i].accepted ? qs[i].recommendation : text
        qs[i].state = discuss ? .discuss : .answered
        s.questions = qs
        s.answered = qs.filter { $0.state != .open }.count
        grillState = s
    }

    /// The open round sent from another client (the web) while the phone was on it: the agent is now writing
    /// the next round (tests).
    func sendGrillRoundElsewhere() {
        locked { sendGrillRound(next: false) }
    }

    /// The round was sent: the answers go in the chat and the next round follows at once, with what was
    /// left open carried first (lock held). Without `next`, the agent is still working on it.
    private func sendGrillRound(next postNext: Bool = true) {
        guard var s = grillState, let qs = s.questions, let n = s.round else { return }
        let lines = qs.enumerated().map { i, q in
            "\(i + 1). " + (q.accepted ? "Yes (\(q.recommendation))" : q.state == .open ? "(no answer)" : (q.answer ?? ""))
        }
        grillMessage("Answers to round \(n):\n\n" + lines.joined(separator: "\n"), GrillMeta(sessionId: s.sessionId, role: .answers, round: n), role: "user")
        guard postNext else {
            s.questions = qs.map { var q = $0; if q.state == .open { q.state = .carried }; return q }
            s.waiting = true
            grillState = s
            return
        }
        let carried = qs.filter { $0.state == .open }.enumerated().map { i, q in
            GrillQuestion(id: "r\(n + 1)c\(i)", text: q.text, speak: q.speak, kind: q.kind, options: q.options,
                          recommendation: q.recommendation, fromRound: q.fromRound ?? n)
        }
        let next = carried + [GrillQuestion(id: "r\(n + 1)q", text: "What's the one thing a visitor must understand within five seconds?",
                                           recommendation: "That Loft runs your agents on your own Mac")]
        s.round = n + 1
        s.questions = next
        s.total = next.count
        s.answered = 0
        s.waiting = false
        grillState = s
        grillMessage(Self.roundText(n + 1, next), GrillMeta(sessionId: s.sessionId, role: .round, round: n + 1, topic: s.topic, questions: next))
    }

    /// Ends the interview with a summary and the end-of-planning card: a spec, two tickets, no reviewer on the team.
    /// `setupSent`: the web already sent the team setup message.
    private func endGrill(_ s: inout GrillSession, setupSent: Bool = false) {
        s.state = .ended
        grillMessage("Ended", GrillMeta(sessionId: s.sessionId, role: .status, state: "ended", endReason: "stopped"))
        // `-fixture-plan-in-loft`: a plan kept in Loft, its issues linked as `/tasks/<id>` (the fixture's t1, t2, t3).
        let inLoft = ProcessInfo.processInfo.arguments.contains("-fixture-plan-in-loft")
        let issue = { (n: Int, t: String) in
            EndCard.Issue(number: n, title: t, url: URL(string: inLoft ? "/tasks/t\(n - 39)" : "https://github.com/octo/site/issues/\(n)")!)
        }
        let card = EndCard(repo: inLoft ? "" : "octo/site", spec: issue(40, "Pricing page"), tickets: [issue(41, "Plans table"), issue(42, "Yearly toggle")],
                           nextMessage: "Implement spec #40: hand the ready tickets to the team", missingRoles: ["reviewer"],
                           setup: .init(agentId: "loft", message: "Set up the team for the site project: it has no reviewer yet.",
                                        sentAt: setupSent ? JSONCoding.formatDate(Date()) : nil))
        grillMessage("**Summary:** new visitors are the audience; show monthly and yearly, yearly highlighted.",
                     GrillMeta(sessionId: s.sessionId, role: .summary, final: true, endCard: card), role: "assistant")
    }

    private func grillMessage(_ text: String, _ g: GrillMeta, role: String = "assistant") {
        nextId += 1
        appendGrill(id: nextId, role: role, text: text, g, at: Date())
    }

    private func appendGrill(id: Int64, role: String, text: String, _ g: GrillMeta, at date: Date) {
        msgs["orch", default: []].append(ChatMessage(id: id, agentId: "orch", runId: nil, role: role, kind: "text", content: text,
                                                    meta: ["grill": Self.wire(g)], createdAt: date))
    }

    private static func wire(_ g: GrillMeta) -> JSONValue {
        (try? JSONEncoder().encode(GrillMetaWire(g))).flatMap { try? JSONDecoder().decode(JSONValue.self, from: $0) } ?? .null
    }

    /// Like the backend: marks the card's setup sent once and only then sends it to `loft`; a repeat is a duplicate.
    func sendTeamSetup(agentId: String, messageId: Int64) async throws -> SendMessageResponse {
        try check()
        return try locked {
            guard let i = msgs[agentId]?.firstIndex(where: { $0.id == messageId }), var m = msgs[agentId]?[i] else {
                throw APIError.server(status: 404, message: "not found")
            }
            guard var g = m.grill, var setup = g.endCard?.setup else { throw APIError.server(status: 400, message: "This card has no team setup.") }
            if setup.sentAt != nil { return SendMessageResponse(ok: true, queued: nil, duplicate: true, message: m) }
            setup.sentAt = JSONCoding.formatDate(Date())
            g.endCard?.setup = setup
            m.meta = ["grill": Self.wire(g)]
            msgs[agentId]?[i] = m
            nextId += 1
            msgs[setup.agentId, default: []].append(ChatMessage(id: nextId, agentId: setup.agentId, runId: nil, role: "user", kind: "text",
                                                                content: setup.message, meta: nil, createdAt: Date()))
            teamSetupsSent += 1
            return SendMessageResponse(ok: true, queued: nil, duplicate: false, message: m)
        }
    }

    private func locked<T>(_ f: () throws -> T) rethrows -> T {
        lock.lock(); defer { lock.unlock() }
        return try f()
    }

    private func check() throws {
        if mode == .offline { throw APIError.unreachable("fixture offline") }
    }

    func info() async throws -> ServerInfo {
        try check()
        return ServerInfo(version: "0.1.0", host: "fixture-mac.tailnet.ts.net",
                          features: ["tasks", "activity", "idempotent_messages", "voice_clean", "system", "run_watchdog", "needs_you", "push", "voice_brief", "plans"] + (grill ? ["grill"] : []))
    }

    func agents() async throws -> [Agent] { try check(); return locked { agentList } }

    func messages(agentId: String, limit: Int, before: Int64?) async throws -> [ChatMessage] {
        try check()
        return locked { (msgs[agentId] ?? []).filter { before == nil || $0.id < before! }.suffix(limit) }
    }

    /// `-fixture-long-reply`: a reply long enough to be read short.
    static let longReply = """
        The QA suite **failed** on two checks:

        1. `LoginTests.testExpiredToken`: the token refresh returns 500
        2. `CheckoutTests.testCoupon`: timeout after 30 s

        Everything else (212 tests) passed. I opened [#88](https://example.com/88) for the token bug and assigned it to \
        Backend Agent, who is already looking at the refresh endpoint. The coupon timeout looks like the staging database \
        being slow again; I asked QA Agent to rerun it in an hour before anyone digs into it.
        """

    func sendMessage(agentId: String, text: String, clientId: String) async throws -> SendMessageResponse {
        try await sendMessage(agentId: agentId, text: text, clientId: clientId, meta: nil)
    }

    func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?) async throws -> SendMessageResponse {
        try check()
        if mode == .failSends { throw APIError.timeout }
        return try locked {
            nextId += 1
            var tags: [String: JSONValue] = ["client_id": .string(clientId)]
            if let g = meta?["grill"] { tags["grill"] = g }
            // An attached task, as the backend stores it (#116): `task_id`, and first in `task_refs`.
            if let t = meta?["task_id"]?.stringValue {
                guard taskList.contains(where: { $0.id == t }) else { throw APIError.server(status: 400, message: "The attached task doesn't exist.") }
                tags["task_id"] = .string(t)
                tags["task_refs"] = .array([.string(t)])
            }
            let m = ChatMessage(id: nextId, agentId: agentId, runId: nil, role: "user", kind: "text", content: text,
                                meta: .object(tags), createdAt: Date())
            msgs[agentId, default: []].append(m)
            // An answer to a question of the open round is held, not shown in the chat, as the backend does.
            if grill, m.grill?.role == .answer, let qid = m.grill?.questionId {
                msgs[agentId]?.removeLast()
                // Like the backend: only a question of the round still waiting for answers takes one.
                guard let s = grillState, s.state != .ended else { throw APIError.server(status: 409, message: "grill_closed") }
                guard s.waiting != true, s.questions?.contains(where: { $0.id == qid }) == true else {
                    throw APIError.server(status: 409, message: "question_closed")
                }
                recordGrillAnswer(text, questionId: qid, discuss: meta?["grill"]?["discuss"] == .bool(true))
                return SendMessageResponse(ok: true, queued: false, duplicate: false, message: nil)
            }
            // `-fixture-replies`: the agent answers half a second later, with no live event (like one missed in a reconnect).
            if m.grill == nil, replies {
                Task { [weak self] in
                    try? await Task.sleep(for: .milliseconds(500))
                    self?.locked {
                        guard let self else { return }
                        self.nextId += 1
                        self.msgs[agentId, default: []].append(ChatMessage(
                            id: self.nextId, agentId: agentId, runId: nil, role: "assistant", kind: "text",
                            content: ProcessInfo.processInfo.arguments.contains("-fixture-long-reply") ? Self.longReply
                                : "Done. I ran it and everything passed.", meta: nil, createdAt: Date()))
                    }
                }
            }
            return SendMessageResponse(ok: true, queued: false, duplicate: false, message: m)
        }
    }

    func grillSession(agentId: String) async throws -> GrillSession? {
        try check()
        guard grill else { throw APIError.server(status: 404, message: "Not Found") }
        return locked { agentId == "orch" ? grillState : nil }
    }

    func grillCommand(sessionId: String, command: GrillCommand, questionId: String?, clientId: String) async throws -> GrillSession? {
        try check()
        guard grill else { throw APIError.server(status: 404, message: "Not Found") }
        return try locked { () -> GrillSession? in
            grillCommandsSent.append(command)
            if let e = failNextGrillCommand {
                failNextGrillCommand = nil
                throw e
            }
            guard var s = grillState, s.sessionId == sessionId else { return nil }
            if s.state == .ended { throw APIError.server(status: 409, message: "grill_closed") }
            if [.send, .decide].contains(command), s.waiting == true { throw APIError.server(status: 409, message: "round_closed") }
            switch command {
            case .send:
                sendGrillRound(); s = grillState!
            case .decide:
                guard var qs = s.questions, let i = qs.firstIndex(where: { $0.id == questionId }) else { break }
                qs[i].state = .decided
                qs[i].answer = qs[i].recommendation
                s.questions = qs
                s.answered = qs.filter { $0.state != .open }.count
                grillMessage("Question \(i + 1) left to the agent: \(qs[i].recommendation)",
                             GrillMeta(sessionId: s.sessionId, role: .status, questionId: qs[i].id, state: "decided", command: .decide))
            case .skip, .back:
                // Stepping through a round is the phone's job; the backend ignores these.
                break
            case .pause:
                s.state = .paused
                grillMessage("Paused", GrillMeta(sessionId: s.sessionId, role: .status, state: "paused"))
            case .resume:
                s.state = .active
                grillMessage("Resumed", GrillMeta(sessionId: s.sessionId, role: .status, state: "resumed"))
            case .summarize:
                grillMessage("**So far:** the page is for new visitors, and pricing stays monthly + yearly.",
                             GrillMeta(sessionId: s.sessionId, role: .summary, final: false), role: "assistant")
            case .end:
                endGrill(&s)
            }
            grillState = s
            return s
        }
    }

    func grillPhrases() async throws -> GrillPhrases {
        try check()
        guard grill else { throw APIError.server(status: 404, message: "Not Found") }
        return GrillPhrases(version: "fixture-1", maxWords: 4, commands: [
            "skip": .init(en: ["skip", "next", "pass"], pt: ["pula", "próxima"]),
            "decide": .init(en: ["i don't know", "you decide"], pt: ["não sei", "decide você"]),
        ])
    }

    func runNow(agentId: String) async throws { try check() }
    func stopRun(agentId: String) async throws { try check() }
    /// The plan view the backend builds, from the fixture's tasks.
    func taskPlan(id: String) async throws -> TaskPlan {
        try check()
        return try locked {
            guard let parent = taskList.first(where: { $0.id == id }) else { throw APIError.server(status: 404, message: "not found") }
            let names = Dictionary(agentList.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
            let ref = { (id: String?) in id.flatMap { i in names[i].map { TaskPlan.AgentRef(id: i, name: $0) } } }
            let subs = taskList.filter { $0.parentId == id }.sorted { $0.createdAt < $1.createdAt }
            let live = subs.filter { $0.status != "cancelled" }
            let done = live.filter { $0.status == "done" }.count
            func count(_ st: [String]) -> Int { subs.filter { st.contains($0.status) }.count }
            return TaskPlan(
                task: .init(id: parent.id, title: parent.title, status: parent.status, agent: ref(parent.agentId), summary: parent.summary),
                subtasks: subs.map { t in
                    .init(id: t.id, title: t.title, status: t.status, agent: ref(t.agentId), note: t.note, summary: t.summary, error: t.error,
                          updatedAt: t.updatedAt, prs: t.id == "t2" ? [.init(id: "p1", number: 214, title: "Settings endpoints", url: "https://github.com/x/y/pull/214", state: "open")] : [])
                },
                progress: .init(total: live.count, done: done, inProgress: count(["in_progress"]), waiting: count(["waiting_approval", "blocked"]),
                                failed: count(["failed"]), todo: count(["inbox", "ready"]), cancelled: count(["cancelled"]),
                                percent: live.isEmpty ? 0 : done * 100 / live.count))
        }
    }

    private var needsYouDone: Set<String> = []

    func needsYou() async throws -> NeedsYouList {
        try check()
        var list = try JSONCoding.decoder.decode(NeedsYouList.self, from: Data(FixtureNeedsYou.json.utf8))
        let done = locked { needsYouDone }
        // `-fixture-needs-you-projects`: PRs from two GitHub projects, with the repo fields (grouped by project).
        if ProcessInfo.processInfo.arguments.contains("-fixture-needs-you-projects"), let pr = list.items.first(where: \.isPullRequest) {
            func make(_ id: String, _ repo: String, _ title: String, urgency: Int) -> NeedsYouItem {
                NeedsYouItem(id: id, type: pr.type, urgency: urgency, agent: pr.agent, taskId: nil, title: title, reason: pr.reason, text: pr.text,
                             createdAt: pr.createdAt, ageSeconds: pr.ageSeconds, links: pr.links, actions: pr.actions,
                             project: String(repo.split(separator: "/").last!),
                             pr: .init(repo: repo, project: String(repo.split(separator: "/").last!), number: nil, url: nil))
            }
            list.items.removeAll { $0.isPullRequest }
            list.items += [make("pr:a", "Octo/loft", "loft#112: Approval cards: risk labels", urgency: 1),
                           make("pr:b", "Octo/loft", "loft#113: Composer: @ picker", urgency: 1),
                           make("pr:c", "acme/api", "api#1765: Scope menu discounts to the restaurant", urgency: 2),
                           make("pr:d", "Octo/loft", "loft#111: Make it a brief", urgency: 0)]
        }
        list.items.removeAll { done.contains($0.id) }
        list.count = list.items.count
        list.urgent = list.items.filter { $0.urgency >= 2 }.count
        return list
    }

    /// Any action resolves its item, as the real server's next list would show.
    func needsYouRequest(method: String, path: String, body: JSONValue?) async throws {
        try check()
        let list = try JSONCoding.decoder.decode(NeedsYouList.self, from: Data(FixtureNeedsYou.json.utf8))
        guard let item = list.items.first(where: { $0.actions.contains { $0.path == path } }) else { return }
        _ = locked { needsYouDone.insert(item.id) }
    }

    func registerDevice(_ d: DeviceRegistration) async throws { try check() }
    func unregisterDevice(token: String) async throws { try check() }
    func testPush(token: String) async throws { try check() }
    func updateDevice(token: String, _ fields: [String: JSONValue]) async throws { try check() }

    func resetSession(agentId: String) async throws {
        try check()
        locked {
            guard let i = agentList.firstIndex(where: { $0.id == agentId }) else { return }
            agentList[i].sessionId = nil
            agentList[i].contextTokens = nil
            nextId += 1
            msgs[agentId, default: []].append(ChatMessage(id: nextId, agentId: agentId, runId: nil, role: "system", kind: "clear",
                                                          content: "Context cleared.", meta: nil, createdAt: Date()))
        }
    }
    func compact(agentId: String) async throws { try check() }

    /// Every agent's Sessions: its first one, then two older ones, the oldest started by the Orchestrator.
    func sessions(agentId: String) async throws -> PastSessions {
        try check()
        return locked { PastSessions(sessions: Self.pastSessions(agentId), current: agentList.first { $0.id == agentId }?.sessionId) }
    }

    private static func pastSessions(_ agentId: String) -> [PastSession] {
        let now = Date()
        return [
            PastSession(id: "s-\(agentId)", startedAt: now.addingTimeInterval(-3600), firstMessage: "Fix the login bug",
                        fromAgent: nil, contextTokens: 12_000),
            PastSession(id: "s-\(agentId)-old", startedAt: now.addingTimeInterval(-3 * 86_400), firstMessage: "Run the smoke tests on the settings page",
                        fromAgent: nil, contextTokens: 48_000),
            PastSession(id: "s-\(agentId)-older", startedAt: now.addingTimeInterval(-9 * 86_400), firstMessage: "Check the login flow before the release",
                        fromAgent: "Orchestrator", contextTokens: 96_000),
        ]
    }

    func resumeSession(agentId: String, sessionId: String) async throws -> Agent {
        try check()
        return try locked {
            guard let i = agentList.firstIndex(where: { $0.id == agentId }) else { throw APIError.server(status: 404, message: "Not Found") }
            if agentList[i].running == true {
                throw APIError.server(status: 409, message: "Agent is working. Stop it before resuming a Session.")
            }
            if agentList[i].sessionId == sessionId { throw APIError.server(status: 400, message: "That's already the agent's Session.") }
            guard let s = Self.pastSessions(agentId).first(where: { $0.id == sessionId }) else {
                throw APIError.server(status: 404, message: "No such Session in this agent's transcripts.")
            }
            agentList[i].sessionId = s.id
            agentList[i].contextTokens = s.contextTokens
            nextId += 1
            let started = s.startedAt.ISO8601Format()
            msgs[agentId, default: []].append(ChatMessage(id: nextId, agentId: agentId, runId: nil, role: "system", kind: "resume",
                                                          content: "Resumed the Session from \(started.prefix(10)).",
                                                          meta: .object(["session_id": .string(s.id), "started_at": .string(started)]), createdAt: Date()))
            return agentList[i]
        }
    }

    func restartRun(agentId: String) async throws {
        try check()
        guard let i = agentList.firstIndex(where: { $0.id == agentId }) else { return }
        agentList[i].waitingOn = []
        agentList[i].waitingSince = nil
        agentList[i].lastOutputAt = Date()
    }
    func permissions() async throws -> [PermissionRequest] { try check(); return locked { perms } }

    func decidePermission(id: String, decision: String) async throws {
        try check()
        locked { if let i = perms.firstIndex(where: { $0.id == id }) { perms[i].status = decision == "deny" ? "denied" : "allowed_once" } }
    }

    func pullRequests() async throws -> [PullRequest] {
        try check()
        return [PullRequest(id: "pr1", agentId: "be", slug: "x/loft", number: 214, url: "https://github.com/x/loft/pull/214",
                            title: "Settings endpoints", state: "open", draft: false, createdAt: Date().addingTimeInterval(-1500))]
    }
    func tasks() async throws -> [HBTask] { try check(); return locked { taskList } }

    func createTask(_ f: [String: JSONValue]) async throws -> HBTask {
        try check()
        return locked {
            let agent = f["agent_id"]?.stringValue
            let t = HBTask(id: f["id"]?.stringValue ?? UUID().uuidString, title: f["title"]?.stringValue ?? "", description: f["description"]?.stringValue ?? "",
                           kind: agent == nil ? "todo" : "work", status: agent == nil ? "inbox" : "ready", priority: f["priority"]?.stringValue ?? "normal",
                           project: f["project"]?.stringValue ?? "", agentId: agent, parentId: f["parent_id"]?.stringValue, dependsOn: [], dueAt: nil,
                           createdBy: "user", note: "", summary: "", error: "", createdAt: Date(), updatedAt: Date(), startedAt: nil, completedAt: nil)
            taskList.insert(t, at: 0)
            return t
        }
    }

    func updateTask(id: String, _ f: [String: JSONValue]) async throws -> HBTask {
        try check()
        return try locked {
            guard let i = taskList.firstIndex(where: { $0.id == id }) else { throw APIError.server(status: 404, message: "not found") }
            if let s = f["status"]?.stringValue { taskList[i].status = s }
            if let t = f["title"]?.stringValue { taskList[i].title = t }
            taskList[i].updatedAt = Date()
            return taskList[i]
        }
    }

    func deleteTask(id: String) async throws { try check(); locked { taskList.removeAll { $0.id == id } } }

    func executeTask(id: String, agentId: String?, note: String) async throws -> ExecuteResponse {
        try check()
        return try locked {
            guard let i = taskList.firstIndex(where: { $0.id == id }) else { throw APIError.server(status: 404, message: "not found") }
            taskList[i].status = "in_progress"
            taskList[i].agentId = agentId ?? taskList[i].agentId
            return ExecuteResponse(task: taskList[i], queued: false)
        }
    }

    func cancelTask(id: String, stopRun: Bool) async throws -> CancelResponse {
        try check()
        return try locked {
            guard let i = taskList.firstIndex(where: { $0.id == id }) else { throw APIError.server(status: 404, message: "not found") }
            taskList[i].status = "cancelled"
            return CancelResponse(task: taskList[i], stopped: stopRun)
        }
    }

    func activity(limit: Int, before: Date?) async throws -> [ActivityItem] {
        try check()
        let now = Date()
        return [
            ActivityItem(id: "a1", type: "approval.required", at: now.addingTimeInterval(-240), agentId: "ios", taskId: nil, messageId: 9,
                         summary: "iOS Agent asked for approval: Bash", severity: "attention"),
            ActivityItem(id: "a2", type: "task.created", at: now.addingTimeInterval(-1800), agentId: "be", taskId: "t2", messageId: nil,
                         summary: "Orchestrator created subtask “Implement backend endpoints”", severity: "info"),
            ActivityItem(id: "a3", type: "agent.failed", at: now.addingTimeInterval(-1900), agentId: "docs", taskId: nil, messageId: 8,
                         summary: "Docs Agent failed: link checker crashed", severity: "error"),
            ActivityItem(id: "a4", type: "task.completed", at: now.addingTimeInterval(-2000), agentId: "be", taskId: "t5", messageId: nil,
                         summary: "Backend Agent finished task “Inspect existing architecture”", severity: "success"),
            ActivityItem(id: "a5", type: "task.blocked", at: now.addingTimeInterval(-2100), agentId: "qa", taskId: "t4", messageId: nil,
                         summary: "QA Agent is blocked on task “Test the integrated flow”", severity: "attention"),
            ActivityItem(id: "a6", type: "agent.completed", at: now.addingTimeInterval(-2200), agentId: "orch", taskId: nil, messageId: nil,
                         summary: "Orchestrator finished", severity: "info"),
        ].filter { before == nil || $0.at < before! }
    }

    func usage() async throws -> PlanUsage {
        try check()
        return PlanUsage(error: nil, windows: [
            .init(key: "five_hour", label: "session", percent: 42, resetsAt: Date().addingTimeInterval(2 * 3600 + 900)),
            .init(key: "seven_day", label: "week", percent: 18, resetsAt: Date().addingTimeInterval(4 * 86400)),
        ])
    }

    func system() async throws -> SystemReading {
        try check()
        return SystemReading(cpuTempC: 64.3, level: "nominal", sampledAt: Date())
    }

    func agentSummaries() async throws -> [AgentSummary] {
        try check()
        let agents = locked { agentList }
        var out: [AgentSummary] = []
        for (i, a) in agents.enumerated() {
            let auto: [Int] = (0..<14).map { ($0 * 7 + i * 3) % 9 }
            let user: [Int] = (0..<14).map { ($0 * 5 + i) % 6 }
            let tools: [JSONValue] = [.array([.string("Bash"), .number(Double(80 - i * 10))]), .array([.string("Read"), .number(Double(40 - i * 5))])]
            out.append(AgentSummary(agent: a, days: 14, dailyAuto: auto, dailyUser: user, toolCalls: 120 * (i + 1),
                                    userMessages: 9 + i, memories: 3 + i, pendingPermissions: a.status == "waiting" ? 1 : 0,
                                    lastError: a.status == "error" ? "Link checker crashed" : nil, lastReply: nil,
                                    topTools: tools, prs: .init(total: 6 - i, open: i % 2, merged: 5 - i)))
        }
        return out
    }

    /// `-fixture-brief "<text>"`: that brief (`\n` for new lines); otherwise the server's "already short" or a
    /// made-up one-line goal.
    func briefVoice(text: String) async throws -> VoiceBriefResponse {
        try check()
        if let b = FixtureAPI.launchText("fixture-brief") {
            return VoiceBriefResponse(text: b.replacingOccurrences(of: "\\n", with: "\n"), briefed: true)
        }
        if text.split(separator: " ").count < LoftStore.briefMinWords {
            return VoiceBriefResponse(text: text, briefed: false, reason: "already short")
        }
        return VoiceBriefResponse(text: "Goal: " + text.split(separator: " ").prefix(12).joined(separator: " "), briefed: true)
    }

    func cleanVoice(text: String) async throws -> VoiceCleanResponse {
        try check()
        return VoiceCleanResponse(text: text.replacingOccurrences(of: " uh", with: ""), cleaned: true, reason: nil)
    }

    func eventLines() async throws -> AsyncThrowingStream<String, Error> {
        try check()
        // A stream that stays open, like a quiet server.
        return AsyncThrowingStream { c in
            let t = Task { while !Task.isCancelled { try? await Task.sleep(for: .seconds(3600)) }; c.finish() }
            c.onTermination = { _ in t.cancel() }
        }
    }
}
/// GrillMeta in the server's snake_case, for fixture messages.
private struct GrillMetaWire: Encodable {
    let g: GrillMeta
    init(_ g: GrillMeta) { self.g = g }
    func encode(to encoder: Encoder) throws {
        let e = JSONEncoder()
        e.keyEncodingStrategy = .convertToSnakeCase
        var c = encoder.singleValueContainer()
        try c.encode(try JSONDecoder().decode(JSONValue.self, from: try e.encode(g)))
    }
}
#endif

/// Hand-drawn stickers in the backend's avatar format, exercising paths (curves, arcs), gradients,
/// clip paths, transforms and strokes.
enum FixtureStickers {
    private static func eyes(_ lx: Int, _ rx: Int, _ y: Int, look: String = "-2 -3") -> String {
        """
        <ellipse cx="\(lx)" cy="\(y)" rx="11" ry="12.5" fill="#fff"/><ellipse cx="\(rx)" cy="\(y)" rx="11" ry="12.5" fill="#fff"/>\
        <g transform="translate(\(look))"><ellipse cx="\(lx)" cy="\(y)" rx="4.5" ry="6.5" transform="rotate(-10 \(lx) \(y))" fill="#17171c"/>\
        <ellipse cx="\(rx)" cy="\(y)" rx="4.5" ry="6.5" transform="rotate(-10 \(rx) \(y))" fill="#17171c"/></g>
        """
    }

    static let byAgent: [String: String] = [
        // Purple blob with a darker lower edge (clip path) and a party hat.
        "orch": """
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><clipPath id="b">\
        <path d="M50 14C74 14 90 30 90 54S74 92 50 92 10 78 10 54 26 14 50 14Z"/></clipPath></defs>\
        <path d="M50 14C74 14 90 30 90 54S74 92 50 92 10 78 10 54 26 14 50 14Z" fill="#5b4fd6"/>\
        <g clip-path="url(#b)"><path d="M50 14C74 14 90 30 90 54S74 92 50 92 10 78 10 54 26 14 50 14Z" fill="#7c6cf5" transform="translate(-1 -5)"/></g>\
        <polygon points="50,2 62,22 38,22" fill="#ffc94d"/><circle cx="50" cy="3" r="4" fill="#ff7eb6"/>\
        \(eyes(38, 62, 46))<path d="M40 68 Q50 77 60 68" fill="none" stroke="#17171c" stroke-width="4" stroke-linecap="round"/></svg>
        """,
        // Orange server box with a linear gradient and a smirk.
        "be": """
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">\
        <stop offset="0.7" stop-color="#ff9a3d"/><stop offset="1" stop-color="#e5762a"/></linearGradient></defs>\
        <rect x="12" y="16" width="76" height="72" rx="16" fill="url(#g)"/>\
        <rect x="24" y="76" width="52" height="5" rx="2.5" fill="#c4611d" opacity=".6"/>\
        \(eyes(38, 62, 44, look: "2 -3"))<path d="M38 64 Q52 70 62 60" fill="none" stroke="#17171c" stroke-width="4" stroke-linecap="round"/></svg>
        """,
        // Blue drop with a wink, drawn with an arc.
        "ios": """
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">\
        <path d="M50 8C62 28 84 44 84 62A34 30 0 0 1 16 62C16 44 38 28 50 8Z" fill="#2f8cff"/>\
        <path d="M20 70A34 30 0 0 0 80 70A34 26 0 0 1 20 70Z" fill="#1f6fd6"/>\
        <ellipse cx="38" cy="56" rx="11" ry="12.5" fill="#fff"/><ellipse cx="40" cy="53" rx="4.5" ry="6.5" fill="#17171c"/>\
        <path d="M56 50 L66 56 L56 62" fill="none" stroke="#17171c" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/>\
        <ellipse cx="30" cy="72" rx="5" ry="3" fill="#ff8fb1" opacity=".7"/><ellipse cx="70" cy="72" rx="5" ry="3" fill="#ff8fb1" opacity=".7"/>\
        <path d="M42 74 Q50 82 58 74" fill="none" stroke="#17171c" stroke-width="4" stroke-linecap="round"/></svg>
        """,
        // Green note with a folded corner, looking worried (its last run failed).
        "docs": """
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">\
        <polygon points="16,10 68,10 86,28 86,90 16,90" fill="#2fbf71"/><polygon points="68,10 86,28 68,28" fill="#1f9a59"/>\
        <rect x="16" y="84" width="70" height="6" fill="#1f9a59"/>\
        \(eyes(38, 62, 46, look: "0 -1"))<path d="M41 72 Q50 64 59 72" fill="none" stroke="#17171c" stroke-width="4" stroke-linecap="round"/></svg>
        """,
    ]
}
