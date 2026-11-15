import XCTest
@testable import Loft
@testable import LoftKit

final class NeedsYouTests: XCTestCase {
    private func fixture() throws -> NeedsYouList {
        try JSONCoding.decoder.decode(NeedsYouList.self, from: Data(FixtureNeedsYou.json.utf8))
    }

    func testAKillOffersAlwaysAllowOnlyWithARule() {
        func request(_ tool: String, _ rule: String?) -> PermissionRequest {
            PermissionRequest(id: "p", agentId: "a", source: "prompt", toolName: tool, input: nil, reason: nil, rule: rule, status: "pending", createdAt: Date())
        }
        XCTAssertTrue(request("Kill", "Kill(/d/*/App)").canAlwaysAllow)
        for rule in [nil, ""] { XCTAssertFalse(request("Kill", rule).canAlwaysAllow, rule ?? "nil") }
        XCTAssertTrue(request("Bash", "Bash(git *)").canAlwaysAllow)
        XCTAssertTrue(request("Bash", nil).canAlwaysAllow)
        for rule in [nil, "", "Checkout(/d/loft)"] { XCTAssertFalse(request("Checkout", rule).canAlwaysAllow, rule ?? "nil") }
    }

    func testTheBackendsFixtureDecodes() throws {
        let l = try fixture()
        XCTAssertEqual(l.version, 1)
        XCTAssertEqual(l.items.first?.type, "permission", "the server's order is kept")
        XCTAssertEqual(Set(l.items.map(\.type)), ["permission", "task_blocked", "question", "grill_round", "stuck", "task_failed", "pr_ready", "closed_on_github", "comment_approval", "task_done"])
        let open = try XCTUnwrap(l.items.first { $0.type == "question" }?.openAction)
        XCTAssertEqual(open.target?.screen, "chat")
        XCTAssertFalse(l.items.first { $0.type == "permission" }!.canDismiss, "permissions can't be dismissed")
    }

    func testAnAnswerCarriesTheWordsAndAClientId() throws {
        let q = try XCTUnwrap(try fixture().items.first { $0.type == "question" }?.answerAction)
        guard case .object(let o)? = q.body(text: "Use B2", clientId: "c1") else { return XCTFail() }
        XCTAssertEqual(o["text"], .string("Use B2"))
        XCTAssertEqual(o["client_id"], .string("c1"))
        // Other actions are sent exactly as given.
        let approve = try XCTUnwrap(try fixture().items.first { $0.type == "permission" }?.actions.first { $0.kind == "approve" })
        XCTAssertEqual(approve.body(text: nil, clientId: "c2"), approve.body)
    }

    /// An issue reply names the picked agent when nobody is on the issue (#145); other answers never carry one.
    func testAnIssueReplyCarriesThePickedAgent() throws {
        let l = try fixture()
        let failed = try XCTUnwrap(l.items.first { $0.type == "task_failed" })
        XCTAssertNil(failed.agent)
        XCTAssertFalse(failed.actions.contains { $0.kind == "retry" }, "replying runs a failed issue again")
        let reply = try XCTUnwrap(failed.answerAction)
        guard case .object(let o)? = reply.body(text: "Go", clientId: "c1", agentId: "qa") else { return XCTFail() }
        XCTAssertEqual(o["agent_id"], .string("qa"))
        let q = try XCTUnwrap(l.items.first { $0.type == "question" }?.answerAction)
        guard case .object(let m)? = q.body(text: "Go", clientId: "c1", agentId: "qa") else { return XCTFail() }
        XCTAssertNil(m["agent_id"])
    }

    /// Closed on GitHub while an agent works on it (#177): Cancel or Keep working, which reopens the GitHub issue.
    func testClosedOnGitHubAsksCancelOrKeepWorking() throws {
        let item = try XCTUnwrap(try fixture().items.first { $0.type == "closed_on_github" })
        XCTAssertEqual(item.actions.filter(\.isRequest).map(\.kind), ["deny", "approve"])
        XCTAssertEqual(item.actions.first { $0.kind == "approve" }?.path, "/api/issues/t2/keep-working")
        XCTAssertEqual(item.openAction?.target?.taskId, "t2")
    }

    /// A comment waiting for GitHub (spec #65): Approve, "Edit and approve" starting from the comment's text, Keep in Loft.
    func testACommentApprovalEditsTheCommentItself() throws {
        let item = try XCTUnwrap(try fixture().items.first { $0.type == "comment_approval" })
        XCTAssertEqual(item.actions.map(\.label), ["Approve", "Edit and approve", "Keep in Loft", "Open issue"])
        let edit = try XCTUnwrap(item.answerAction)
        XCTAssertEqual(edit.draft, item.text, "editing starts from the comment, not an empty box")
        guard case .object(let o)? = edit.body(text: "Edited", clientId: "c1") else { return XCTFail() }
        XCTAssertEqual(o["text"], .string("Edited"))
        XCTAssertEqual(edit.path, "/api/issues/t1/comments/77/approve")
        // An ordinary answer still starts empty.
        XCTAssertEqual(try fixture().items.first { $0.type == "question" }?.answerAction?.draft, "")
    }

    @MainActor
    func testAnActionRefreshesTheList() async throws {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        // Connected, so the store accepts writes (actions are writes).
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.needsYou == nil { try await Task.sleep(for: .milliseconds(50)) }
        defer { store.deactivate() }
        let before = try XCTUnwrap(store.needsYou?.items.count)
        let restart = try XCTUnwrap(store.needsYou?.items.first { $0.type == "stuck" }?.actions.first { $0.kind == "restart" })
        let done = try await store.perform(restart)
        XCTAssertTrue(done)
        XCTAssertEqual(store.needsYou?.items.count, before - 1)
        XCTAssertNil(store.needsYou?.items.first { $0.type == "stuck" })
    }

    func testRepliesOnlyOfferWhatTheAgentSaid() throws {
        guard #available(iOS 26.0, *) else { throw XCTSkip("iOS 26") }
        let text = "Backblaze B2 costs about $1 a month; S3 is about $3. Should I use B2 or S3?"
        XCTAssertEqual(OnDeviceJobs.usableReplies(["Use B2", "Use S3", "Use Google Drive", "Later"], for: text), ["Use B2", "Use S3", "Later"])
        XCTAssertEqual(OnDeviceJobs.usableReplies(["Yes", "yes.", "This reply is far too long to be a chip", "No"], for: text), ["Yes", "No"])
        XCTAssertNotNil(OnDeviceJobs.problem(withSummary: "The weather is nice in Lisbon today", for: text), "a summary has to come from the item")
        XCTAssertNil(OnDeviceJobs.problem(withSummary: "Should I use Backblaze B2 or S3?", for: text))
    }

    private func item(_ id: String, _ type: String, urgency: Int, project: String? = nil, repo: String? = nil) -> NeedsYouItem {
        let isPR = type.hasPrefix("pr_") && (project != nil || repo != nil)
        return NeedsYouItem(id: id, type: type, urgency: urgency, agent: nil, taskId: nil, title: id, reason: "", text: "", createdAt: nil,
                            ageSeconds: nil, links: nil, actions: [], project: isPR ? nil : project,
                            pr: isPR ? .init(repo: repo, project: project, number: nil, url: nil) : nil)
    }

    func testTheBackendsPRItemDecodes() throws {
        let json = #"""
        {"id":"pr:1","type":"pr_review","urgency":1,"title":"shop#12","reason":"Review requested","text":"","actions":[],
         "project":"shop","pr":{"repo":"acme/shop","project":"shop","number":12,"title":"Fix","url":"https://github.com/x","author":"a",
         "draft":false,"checks":"passing","checks_green":true,"review_requested":true}}
        """#
        let i = try JSONCoding.decoder.decode(NeedsYouItem.self, from: Data(json.utf8))
        XCTAssertEqual(i.pr?.repo, "acme/shop")
        XCTAssertEqual(NeedsYouSection.build([i]).map(\.id), ["project:shop"])
    }

    func testANonPRItemsProjectDoesntGroupIt() {
        XCTAssertEqual(NeedsYouSection.build([item("t", "task_failed", urgency: 1, project: "loft")]).map(\.id), ["t"])
    }

    func testPullRequestsAreGroupedByProject() {
        let s = NeedsYouSection.build([
            item("perm", "permission", urgency: 3),
            item("a1", "pr_ready", urgency: 1, project: "loft"),
            item("q", "question", urgency: 2),
            item("b1", "pr_checks_failed", urgency: 2, repo: "acme/api"),
            item("a2", "pr_ready", urgency: 1, project: "loft"),
            item("t", "task_done", urgency: 0),
        ])
        XCTAssertEqual(s.map(\.id), ["perm", "q", "project:api", "project:loft", "t"],
                       "by urgency; a project goes after the items of its urgency; the repo's name when there's no project")
        guard case .project(_, let loft) = s[3] else { return XCTFail() }
        XCTAssertEqual(loft.map(\.id), ["a1", "a2"])
    }

    func testWithoutProjectFieldsTheListStaysFlat() {
        let items = [item("a", "pr_ready", urgency: 1), item("b", "permission", urgency: 3), item("c", "pr_ready", urgency: 1)]
        XCTAssertEqual(NeedsYouSection.build(items).map(\.id), ["b", "a", "c"])
    }

    func testBiggerProjectsFirstWithinAnUrgency() {
        let s = NeedsYouSection.build([item("x", "pr_ready", urgency: 1, project: "site"), item("y1", "pr_ready", urgency: 1, project: "app"),
                                       item("y2", "pr_ready", urgency: 1, project: "app")])
        XCTAssertEqual(s.map(\.id), ["project:app", "project:site"])
    }

    /// The seen mark (#395), shared with the web: the badge counts what no client has seen; a changed item (new id) is
    /// unseen again; while the list is open, what was new when it appeared stays new.
    func testTheSeenMarkDecodesAndOnlyUnseenItemsAreMarkedAndShownNew() throws {
        var l = try fixture()
        XCTAssertNil(l.new, "an older server sends no seen counts")
        XCTAssertEqual(l.unseen, l.count, "so the badge is the whole list")
        let json = #"{"version":1,"count":3,"urgent":1,"new":1,"new_urgent":0,"items":[]}"#
        let decoded = try JSONCoding.decoder.decode(NeedsYouList.self, from: Data(json.utf8))
        XCTAssertEqual(decoded.unseen, 1)
        XCTAssertEqual(decoded.newUrgent, 0)
        XCTAssertEqual(decoded.headline, "Needs you · 1 new of 3")

        l.items = Array(l.items.prefix(3))
        l.items[0].seen = true
        l.items[1].seen = false
        XCTAssertEqual(NeedsYouSeen.unseen(l.items), [l.items[1].id, l.items[2].id])
        var fresh = NeedsYouSeen.fresh(after: [], items: l.items)
        XCTAssertEqual(fresh, [l.items[1].id, l.items[2].id])
        // After marking: still new while the list is open; one that left the list drops out.
        var after = Array(l.items.prefix(2))
        after[1].seen = true
        fresh = NeedsYouSeen.fresh(after: fresh, items: after)
        XCTAssertEqual(fresh, [l.items[1].id])
        XCTAssertEqual(NeedsYouSeen.body(["a"]), .object(["ids": .array([.string("a")])]))
    }

    /// Each item that can go has a Done check; a permission (it needs an answer) doesn't.
    func testDoneIsTheCheckThatTakesAnItemOff() throws {
        let l = try fixture()
        XCTAssertNil(l.items.first { $0.type == "permission" }?.doneAction)
        XCTAssertEqual(l.items.first { $0.type == "task_failed" }?.doneAction?.path?.hasSuffix("/dismiss"), true)
    }

    /// The fixture server keeps the seen mark like the real one: marking what's shown empties the badge.
    func testMarkingSeenEmptiesTheBadge() async throws {
        let api = FixtureAPI()
        let before = try await api.needsYou()
        XCTAssertEqual(before.new, before.count)
        try await api.needsYouRequest(method: "POST", path: "/api/needs-you/seen", body: NeedsYouSeen.body(before.items.map(\.id)))
        let after = try await api.needsYou()
        XCTAssertEqual(after.new, 0)
        XCTAssertEqual(after.count, before.count, "seen items stay in the list")
        XCTAssertTrue(after.items.allSatisfy { $0.seen == true })
    }
}
