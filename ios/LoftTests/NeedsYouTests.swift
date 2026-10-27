import XCTest
@testable import Loft

final class NeedsYouTests: XCTestCase {
    private func fixture() throws -> NeedsYouList {
        try JSONCoding.decoder.decode(NeedsYouList.self, from: Data(FixtureNeedsYou.json.utf8))
    }

    func testTheBackendsFixtureDecodes() throws {
        let l = try fixture()
        XCTAssertEqual(l.version, 1)
        XCTAssertEqual(l.items.first?.type, "permission", "the server's order is kept")
        XCTAssertEqual(Set(l.items.map(\.type)), ["permission", "task_blocked", "question", "grill_round", "stuck", "task_failed", "pr_ready", "task_done"])
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
}
