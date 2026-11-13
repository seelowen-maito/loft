import XCTest
@testable import Loft

/// The issue page's Delivery card (#340): what it says in each state, which buttons it shows, and that it stays live.
final class DeliveryCardTests: XCTestCase {
    private func delivery(_ json: String) throws -> IssueDelivery {
        try JSONCoding.decoder.decode(IssueDelivery.self, from: Data(json.utf8))
    }

    private static let spec = #"""
    {"state":"stalled","stall_reason":"restart","can_pause":false,"can_resume":true,"needs_human":false,"started":true,
     "agent_id":"dev","agent_name":"Dev","pr":334,"spec":true,"current":{"number":340,"title":"iOS Delivery card"},
     "tickets":[{"number":335,"title":"Stall","state":"closed","skipped":""},{"number":340,"title":"iOS Delivery card","state":"started","skipped":""},
                {"number":341,"title":"Docs","state":"elsewhere","skipped":"has its own open PR #9"}]}
    """#

    func testASpecCardShowsProgressStateAndSkippedTickets() throws {
        let d = try delivery(Self.spec)
        XCTAssertEqual(d.progressLine(issue: 333), "#340 iOS Delivery card is with Dev on spec PR #334. 1 of 3 tickets done.")
        XCTAssertEqual(d.line?.tone, .stalled)
        XCTAssertEqual(d.line?.text, "Stalled: its run was lost when Loft restarted.")
        XCTAssertEqual(d.skippedTickets.map(\.number), [341])
        XCTAssertTrue(!d.showsPause && d.showsResume)
    }

    /// #181: two tickets running at once, each with its Dev, and a ready ticket nobody has yet.
    func testASpecCardListsEveryRunningTicketAndWhatWaitsForTheLead() throws {
        let d = try delivery(#"""
        {"state":"running","started":true,"agent_id":null,"agent_name":null,"pr":334,"spec":true,"current":null,"waiting_for_lead":[343],
         "tickets":[{"number":340,"title":"Card","state":"started","skipped":"","agent_id":"a","agent_name":"Dev 1"},
                    {"number":342,"title":"Specs","state":"review","skipped":"","agent_id":"b","agent_name":"Dev 2"},
                    {"number":343,"title":"Docs","state":"open","skipped":"","agent_id":null,"agent_name":null}]}
        """#)
        XCTAssertEqual(d.runningLines, ["#340 Card is with Dev 1", "#342 Specs is with Dev 2"])
        XCTAssertEqual(d.progressLine(issue: 333), "On spec PR #334. 0 of 3 tickets done.")
        XCTAssertEqual(SpecViews.waitingLine(d.waitingForLead), "Waiting for the Lead to assign #343")
    }

    /// #434: a spec whose tickets wait on tickets in another spec: what they wait on, and which spec to start first.
    func testDecodesWhatItsTicketsWaitOnAndWhichSpecToStartFirst() throws {
        let d = try delivery(#"""
        {"state":"planned","started":false,"agent_id":null,"agent_name":null,"pr":null,"spec":true,"current":null,
         "tickets":[{"number":5,"title":"Screen","state":"open","skipped":"","waiting_on":["octo/api#12","octo/api#13"]},
                    {"number":6,"title":"Docs","state":"open","skipped":"","waiting_on":[]}],
         "waits_on":[{"blocker":"octo/api#12","delivery":{"repo":"octo/api","spec":10,"started":false,"issue_id":"api10"}},
                     {"blocker":"octo/api#13","delivery":{"repo":"octo/api","spec":10,"started":false,"issue_id":"api10"}},
                     {"blocker":"octo/web#3","delivery":{"repo":"octo/web","spec":1,"started":true,"issue_id":"web1"}},
                     {"blocker":"octo/x#4","delivery":{"repo":"octo/x","spec":2,"started":false,"issue_id":null}},
                     {"blocker":"octo/y#5","delivery":null}]}
        """#)
        XCTAssertEqual(d.tickets?.map(\.waitingOn), [["octo/api#12", "octo/api#13"], []])
        XCTAssertEqual(d.waitsOn?.map(\.blocker), ["octo/api#12", "octo/api#13", "octo/web#3", "octo/x#4", "octo/y#5"])
        XCTAssertEqual(d.waitsOn?.first?.delivery?.issueId, "api10")
        XCTAssertNil(d.waitsOn?.last?.delivery)
        // Once each, only specs not started yet, and only those Loft can start (with a Loft issue).
        XCTAssertEqual(WaitsOn.specsToStartFirst(d.waitsOn), [.init(repo: "octo/api", spec: 10, issueId: "api10")])
        XCTAssertEqual(WaitsOn.specsToStartFirst(nil), [])
    }

    func testTheStartWarningReadsLikeTheWebs() throws {
        let one = [WaitsOn(blocker: "octo/api#12", delivery: nil)]
        XCTAssertEqual(StartWarning.text(one, canStartFirst: true),
                       "Tickets of this spec wait on octo/api#12, which isn't done yet. Start the spec that builds it first, or start this one and let those tickets wait.")
        XCTAssertEqual(StartWarning.text(one + [WaitsOn(blocker: "octo/web#3", delivery: nil)], canStartFirst: false),
                       "Tickets of this spec wait on octo/api#12, octo/web#3, which aren't done yet. Start anyway and those tickets wait until it closes.")
    }

    func testEachStateReadsLikeTheWebCard() throws {
        func line(_ state: String, _ extra: String = "") throws -> String? {
            try delivery(#"{"state":"\#(state)","started":\#(state != "planned"),"agent_id":null,"agent_name":null,"pr":null\#(extra)}"#).line?.text
        }
        XCTAssertNil(try line("planned"), "nothing before it started")
        XCTAssertEqual(try line("running"), "Running.")
        XCTAssertEqual(try line("running", #","needs_human":true"#), "Needs you: a ticket had too many rounds of changes.")
        XCTAssertEqual(try line("stalled", #","stall_reason":"run_failed""#), "Stalled: the run Loft woke for its step failed.")
        XCTAssertEqual(try line("stalled", #","stall_reason":"no_call""#), "Stalled: the run Loft woke for its step ended without its call.")
        XCTAssertEqual(try line("paused"), "Paused: Loft sends no wakes until you resume it.")
        XCTAssertEqual(try line("needs_human"), "Needs you: too many rounds of changes in the final review.")
        XCTAssertEqual(try line("final_review"), "Final review: the Code reviewer is reviewing the whole PR.")
        XCTAssertEqual(try line("final_fixes"), "Final review: the Dev is fixing what the Code reviewer found.")
        for done in ["ready", "settled", "merged"] { XCTAssertEqual(try line(done), "Done: every review passed.") }
    }

    func testPauseAndResumeShowOnlyWhenTheServerSaysTheyApply() throws {
        func buttons(_ started: Bool, pause: Bool, resume: Bool) throws -> (Bool, Bool) {
            let d = try delivery(#"{"state":"running","started":\#(started),"can_pause":\#(pause),"can_resume":\#(resume),"agent_id":null,"agent_name":null,"pr":null}"#)
            return (d.showsPause, d.showsResume)
        }
        XCTAssertTrue(try buttons(true, pause: true, resume: false) == (true, false), "running: Pause")
        XCTAssertTrue(try buttons(true, pause: false, resume: true) == (false, true), "stalled, paused or needs you: Resume")
        XCTAssertTrue(try buttons(false, pause: true, resume: true) == (false, false), "not started: neither, Start delivery instead")
        let old = try delivery(#"{"state":"running","started":true,"agent_id":null,"agent_name":null,"pr":null}"#)
        XCTAssertTrue((old.showsPause, old.showsResume) == (false, false), "an older server says neither applies")
    }

    func testReopenShowsOnlyOnADoneDeliveryTheServerSaysHasNewTickets() throws {
        let reopen = try delivery(#"{"state":"ready","started":true,"can_pause":false,"can_resume":false,"can_reopen":true,"state_line":{"tone":"done","text":"Done, but tickets published since are open: Reopen delivery builds them."},"agent_id":null,"agent_name":null,"pr":500}"#)
        XCTAssertTrue(reopen.showsReopen && !reopen.showsResume && !reopen.showsPause)
        XCTAssertEqual(reopen.line?.text, "Done, but tickets published since are open: Reopen delivery builds them.")
        let done = try delivery(#"{"state":"ready","started":true,"can_reopen":false,"agent_id":null,"agent_name":null,"pr":500}"#)
        XCTAssertFalse(done.showsReopen)
        let old = try delivery(#"{"state":"ready","started":true,"agent_id":null,"agent_name":null,"pr":500}"#)
        XCTAssertFalse(old.showsReopen, "an older server says nothing")
        XCTAssertEqual(IssueDeliveryAction.reopen.rawValue, "reopen", "POST /api/issues/{id}/reopen-delivery")
    }

    func testAStandaloneTicketSaysWhoHasItOrThatTheyAreGone() throws {
        let with = try delivery(#"{"state":"running","started":true,"agent_id":"dev","agent_name":"Dev","pr":12}"#)
        XCTAssertEqual(with.progressLine(issue: 7), "With Dev on PR #12. Its reviewers look at it next; #7 closes when the PR merges.")
        let gone = try delivery(#"{"state":"stalled","started":true,"agent_id":null,"agent_name":null,"pr":12}"#)
        XCTAssertEqual(gone.progressLine(issue: 7), "Started on PR #12; the agent building it is gone. #7 closes when the PR merges.")
    }

    /// The server words the line: the card shows it as is, and only maps its tone to a colour.
    func testTheServersStateLineShowsAsIs() throws {
        let d = try delivery(#"{"state":"stalled","stall_reason":"no_call","state_line":{"tone":"stalled","text":"Stalled: reworded on the server."},"started":true,"agent_id":null,"agent_name":null,"pr":null}"#)
        XCTAssertEqual(d.line?.tone, .stalled)
        XCTAssertEqual(d.line?.text, "Stalled: reworded on the server.")
        let waits = try delivery(#"{"state":"running","state_line":{"tone":"needs_you","text":"Needs you."},"started":true,"agent_id":null,"agent_name":null,"pr":null}"#)
        XCTAssertEqual(waits.line?.tone, .needsYou)
        let new = try delivery(#"{"state":"blocked","state_line":{"tone":"blocked","text":"Blocked: a new state."},"started":true,"agent_id":null,"agent_name":null,"pr":null}"#)
        XCTAssertEqual(new.line?.tone, .running, "a tone this build doesn't know takes running's colour")
        XCTAssertEqual(new.line?.text, "Blocked: a new state.")
        let planned = try delivery(#"{"state":"planned","state_line":null,"started":false,"agent_id":null,"agent_name":null,"pr":null}"#)
        XCTAssertNil(planned.line)
        // An older backend sends no state_line: today's wording (testEachStateReadsLikeTheWebCard has each state).
        XCTAssertEqual(try delivery(Self.spec).line?.text, "Stalled: its run was lost when Loft restarted.")
    }

    /// #424: waiting for CI, the line says how long since the Delivery's `ci_since` (its earliest ticket waiting, a
    /// standalone ticket's too) and opens the Checks tab; no Dev is named on it.
    func testWaitingForCIShowsHowLongAndOpensTheChecks() throws {
        let d = try delivery(#"""
        {"state":"running","state_line":{"tone":"ci","text":"Waiting for CI on #41, #42"},"started":true,"agent_id":null,"agent_name":null,
         "pr":500,"checks_url":"https://github.com/o/r/pull/500/checks","ci_since":"2026-11-13T12:04:00.5+00:00","spec":true,"current":null,
         "tickets":[{"number":40,"title":"A","state":"closed","skipped":"","waiting_for_ci":false,"ci_since":null},
                    {"number":41,"title":"B","state":"review","skipped":"","agent_name":null,"waiting_for_ci":true,"ci_since":"2026-11-13T12:10:00+00:00"},
                    {"number":42,"title":"C","state":"review","skipped":"","agent_name":null,"waiting_for_ci":true,"ci_since":"2026-11-13T12:04:00.5+00:00"}]}
        """#)
        let now = JSONCoding.parseDate("2026-11-13T12:10:30Z")!
        XCTAssertEqual(d.line(now: now)?.tone, .ci)
        XCTAssertEqual(d.line(now: now)?.text, "Waiting for CI on #41, #42 · 6 min", "from the Delivery's ci_since")
        XCTAssertEqual(d.lineLink?.absoluteString, "https://github.com/o/r/pull/500/checks")
        XCTAssertEqual(d.runningLines, [])
        XCTAssertEqual(d.progressLine(issue: 420), "On spec PR #500. 1 of 3 tickets done.")

        // A standalone ticket sends no tickets: its card counts from ci_since all the same.
        let one = try delivery(#"{"state":"running","state_line":{"tone":"ci","text":"Waiting for CI on #7"},"started":true,"agent_id":null,"agent_name":null,"pr":12,"checks_url":"https://github.com/o/r/pull/12/checks","ci_since":"2026-11-13T12:08:00+00:00"}"#)
        XCTAssertEqual(one.line(now: now)?.text, "Waiting for CI on #7 · 2 min")
        XCTAssertEqual(one.lineLink?.absoluteString, "https://github.com/o/r/pull/12/checks")
        XCTAssertEqual(one.progressLine(issue: 7), "Its reviewers passed on PR #12; Loft waits for its CI. #7 closes when the PR merges.")
        let noTime = try delivery(#"{"state":"running","state_line":{"tone":"ci","text":"Waiting for CI on #7"},"started":true,"agent_id":null,"agent_name":null,"pr":12,"ci_since":null}"#)
        XCTAssertEqual(noTime.line(now: now)?.text, "Waiting for CI on #7", "no start time, no elapsed")
        let running = try delivery(#"{"state":"running","state_line":{"tone":"running","text":"Running."},"started":true,"agent_id":"dev","agent_name":"Dev","pr":12,"checks_url":"https://github.com/o/r/pull/12/checks"}"#)
        XCTAssertNil(running.lineLink, "only waiting for CI links to the checks")
    }

    func testAButtonsAnswerCarriesTheDeliveryAndWhatItDid() throws {
        let r = try JSONCoding.decoder.decode(IssueDeliveryReply.self, from: Data(#"{"ok":true,"message":"Paused spec #333.","state":"paused","started":true,"can_resume":true,"agent_id":"dev","agent_name":"Dev","pr":334}"#.utf8))
        XCTAssertEqual(r.message, "Paused spec #333.")
        XCTAssertEqual(r.delivery.state, "paused")
        XCTAssertTrue(r.delivery.showsResume)
    }

    /// Start delivery on a standalone ticket, as the backend answers it (`start_from_issue`): the Delivery plus
    /// `ok`, `duplicate` and `ticket`, with no `message`. The first start and a duplicate alike.
    func testAStandaloneStartReplyDecodes() throws {
        for duplicate in [false, true] {
            let json = #"{"ok":true,"duplicate":\#(duplicate),"ticket":50,"state":"running","stall_reason":null,"can_pause":true,"can_resume":false,"needs_human":false,"started":true,"agent_id":"dev","agent_name":"Dev","pr":51}"#
            let r = try JSONCoding.decoder.decode(IssueDeliveryReply.self, from: Data(json.utf8))
            XCTAssertNil(r.message)
            XCTAssertEqual(r.delivery.line?.text, "Running.")
            XCTAssertEqual(r.delivery.progressLine(issue: 50), "With Dev on PR #51. Its reviewers look at it next; #50 closes when the PR merges.")
            XCTAssertTrue(r.delivery.showsPause && !r.delivery.showsResume)
        }
    }

    func testTheDeliveryEventDecodes() throws {
        guard case .delivery(let repo, let spec) = try ServerEvent.decode(Data(#"{"type":"delivery","repo":"Octo/Site","spec":40}"#.utf8)) else { return XCTFail() }
        XCTAssertEqual(repo, "Octo/Site")
        XCTAssertEqual(spec, 40)
    }

    func testOnlySpecsAndStandaloneTicketsGetTheCard() {
        func t(_ labels: [String], parent: String? = nil, description: String = "", gh: String? = "open", status: String = "inbox") -> HBTask {
            HBTask(id: "t", title: "t", description: description, kind: "work", status: status, priority: "normal", project: "", agentId: nil,
                   parentId: parent, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "",
                   createdAt: .now, updatedAt: .now, startedAt: nil, completedAt: nil, labels: labels,
                   github: gh.map { .init(repo: "o/r", number: 1, url: "https://github.com/o/r/issues/1", state: $0) })
        }
        XCTAssertTrue(t(["spec"]).isDeliverableSpec)
        XCTAssertTrue(t(["Spec"]).hasDeliveryCard)
        XCTAssertTrue(t(["ready-for-agent"]).isStandaloneTicket)
        XCTAssertFalse(t(["ready-for-agent"], parent: "s").hasDeliveryCard, "a spec's ticket goes through the spec's card")
        XCTAssertFalse(t(["ready-for-agent"], description: "## Parent\n\n#333\n").hasDeliveryCard, "so does one naming its spec")
        XCTAssertFalse(t(["spec"], gh: "closed").hasDeliveryCard)
        XCTAssertFalse(t(["spec"], status: "done").hasDeliveryCard)
        XCTAssertFalse(t(["spec"], gh: nil).hasDeliveryCard, "a Loft-only issue")
        XCTAssertFalse(t(["bug"]).hasDeliveryCard)
    }

    func testDeliveryPushesOpenTheIssue() {
        for kind in ["delivery_stalled", "delivery_done"] {
            let info: [AnyHashable: Any] = ["type": kind, "needs_you_id": "x", "target": ["screen": "task", "task_id": "spec-333"]]
            XCTAssertEqual(PushDestination.from(info), .task(taskId: "spec-333"), kind)
        }
    }

    /// The card stays live: a `delivery` event about its spec reloads it, one about another spec doesn't.
    @MainActor
    func testADeliveryEventReloadsTheOpenCard() async throws {
        let api = MockAPI()
        api.taskList = [HBTask(id: "s", title: "Spec", description: "", kind: "work", status: "inbox", priority: "normal", project: "", agentId: nil,
                               parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "",
                               createdAt: .now, updatedAt: .now, startedAt: nil, completedAt: nil, labels: ["spec"],
                               github: .init(repo: "octo/site", number: 40, url: "https://github.com/octo/site/issues/40", state: "open"))]
        api.deliveryOf["s"] = try delivery(#"{"state":"running","started":true,"can_pause":true,"agent_id":"dev","agent_name":"Dev","pr":7}"#)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("snap-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: url) }
        let store = LoftStore(api: api, cache: SnapshotStore(url: url))
        store.activate()
        defer { store.deactivate() }
        for _ in 0..<100 where store.connection != .connected || store.task("s") == nil { try await Task.sleep(for: .milliseconds(20)) }
        store.showDeliveryCard("s")
        try await store.loadDelivery("s")
        XCTAssertEqual(store.deliveries["s"]?.state, "running")

        api.deliveryOf["s"] = try delivery(#"{"state":"stalled","stall_reason":"run_failed","started":true,"can_resume":true,"agent_id":"dev","agent_name":"Dev","pr":7}"#)
        let loads = api.deliveryLoads
        store.apply(.delivery(repo: "octo/other", spec: 40))
        store.apply(.delivery(repo: "octo/site", spec: 41))
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(api.deliveryLoads, loads, "events about other Deliveries leave it alone")
        store.apply(.delivery(repo: "Octo/Site", spec: 40))
        for _ in 0..<100 where store.deliveries["s"]?.state != "stalled" { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertEqual(store.deliveries["s"]?.line?.text, "Stalled: the run Loft woke for its step failed.")
    }

    /// A card that left the screen is forgotten: reconnects and `delivery` events stop reloading it, until it's back.
    @MainActor
    func testAForgottenCardIsNotReloaded() async throws {
        let api = MockAPI()
        api.taskList = [HBTask(id: "s", title: "Spec", description: "", kind: "work", status: "inbox", priority: "normal", project: "", agentId: nil,
                               parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "",
                               createdAt: .now, updatedAt: .now, startedAt: nil, completedAt: nil, labels: ["spec"],
                               github: .init(repo: "octo/site", number: 40, url: "https://github.com/octo/site/issues/40", state: "open"))]
        api.deliveryOf["s"] = try delivery(#"{"state":"running","started":true,"can_pause":true,"agent_id":"dev","agent_name":"Dev","pr":7}"#)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("snap-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: url) }
        let store = LoftStore(api: api, cache: SnapshotStore(url: url))
        try await store.refresh()

        store.showDeliveryCard("s")
        try await store.loadDelivery("s")
        var loads = api.deliveryLoads
        try await store.refresh()
        XCTAssertEqual(api.deliveryLoads, loads + 1, "a reconnect reloads the card on screen")

        store.hideDeliveryCard("s")
        loads = api.deliveryLoads
        try await store.refresh()
        store.apply(.delivery(repo: "octo/site", spec: 40))
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(api.deliveryLoads, loads, "neither a reconnect nor an event reloads a card off screen")

        // Two copies on screen: hiding one keeps it live.
        store.showDeliveryCard("s")
        store.showDeliveryCard("s")
        store.hideDeliveryCard("s")
        loads = api.deliveryLoads
        try await store.refresh()
        XCTAssertEqual(api.deliveryLoads, loads + 1, "the other copy still reloads")
    }

    /// A connected store with issue "s" loaded, whose Delivery has stalled. Deactivate it when done.
    @MainActor
    private func stalledStore(_ api: MockAPI) async throws -> LoftStore {
        api.taskList = [HBTask(id: "s", title: "Spec", description: "", kind: "work", status: "inbox", priority: "normal", project: "", agentId: nil,
                               parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "",
                               createdAt: .now, updatedAt: .now, startedAt: nil, completedAt: nil, labels: ["spec"],
                               github: .init(repo: "octo/site", number: 40, url: "https://github.com/octo/site/issues/40", state: "open"))]
        api.deliveryOf["s"] = try delivery(#"{"state":"stalled","stall_reason":"run_failed","started":true,"can_resume":true,"agent_id":"dev","agent_name":"Dev","pr":7}"#)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("snap-\(UUID().uuidString).json")
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        let store = LoftStore(api: api, cache: SnapshotStore(url: url))
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.task("s") == nil { try await Task.sleep(for: .milliseconds(20)) }
        return store
    }

    /// Loads that answer out of order: the older one, landing last, doesn't put the card back to Stalled.
    @MainActor
    func testAnOlderLoadLandingLateKeepsTheNewerDelivery() async throws {
        let api = MockAPI()
        let store = try await stalledStore(api)
        defer { store.deactivate() }
        api.deliveryDelays = [.milliseconds(300), .zero]
        let older = Task { try await store.loadDelivery("s") }
        for _ in 0..<100 where api.deliveryLoads == 0 { try await Task.sleep(for: .milliseconds(10)) }
        api.deliveryOf["s"] = try delivery(#"{"state":"running","started":true,"can_pause":true,"agent_id":"dev","agent_name":"Dev","pr":7}"#)
        try await store.loadDelivery("s")
        XCTAssertEqual(store.deliveries["s"]?.state, "running")
        try await older.value
        XCTAssertEqual(store.deliveries["s"]?.state, "running", "the older answer is dropped")
    }

    /// A load started before Resume that answers after it doesn't overwrite Resume's reply.
    @MainActor
    func testALoadStartedBeforeAnActionDoesntOverwriteItsReply() async throws {
        let api = MockAPI()
        let store = try await stalledStore(api)
        defer { store.deactivate() }
        api.deliveryDelays = [.milliseconds(300)]
        let older = Task { try await store.loadDelivery("s") }
        for _ in 0..<100 where api.deliveryLoads == 0 { try await Task.sleep(for: .milliseconds(10)) }
        api.deliveryReply = try JSONCoding.decoder.decode(IssueDeliveryReply.self, from: Data(#"{"ok":true,"message":"Resumed spec #40.","state":"running","started":true,"can_pause":true,"agent_id":"dev","agent_name":"Dev","pr":7}"#.utf8))
        let message = try await store.deliveryAction("s", .resume)
        XCTAssertEqual(message, "Resumed spec #40.")
        XCTAssertEqual(store.deliveries["s"]?.state, "running")
        try await older.value
        XCTAssertEqual(store.deliveries["s"]?.state, "running", "the load from before Resume is dropped")
    }
}
