import XCTest
@testable import Loft

/// The stage rules mirror the web's `specViews.ts` tests.
final class SpecViewsTests: XCTestCase {
    private func spec(_ status: String = "ready", number: Int = 7, id: String = "s") -> HBTask {
        var t = HBTask(id: id, title: "Spec", description: "", kind: "work", status: status, priority: "normal", project: "loft", agentId: nil, parentId: nil,
                       dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "", createdAt: Date(), updatedAt: Date())
        t.github = .init(repo: "o/r", number: number, url: "", state: "open")
        return t
    }

    private func delivery(_ state: String, spec: Int = 7, local: Bool = false, specId: String? = nil) -> DeliverySummary {
        DeliverySummary(state: state, agentName: nil, repo: "o/r", spec: spec, specId: specId, local: local, current: nil, tickets: [])
    }

    func testNoDeliveryOrNotStartedIsPlanned() {
        XCTAssertEqual(SpecViews.stage(of: spec(), nil), .planned)
        XCTAssertEqual(SpecViews.stage(of: spec(), delivery("planned")), .planned)
    }

    func testRunningIsInDeliveryAndReadyOrSettledIsReadyToMerge() {
        XCTAssertEqual(SpecViews.stage(of: spec(), delivery("running")), .delivery)
        XCTAssertEqual(SpecViews.stage(of: spec(), delivery("needs_human")), .delivery)
        XCTAssertEqual(SpecViews.stage(of: spec(), delivery("ready")), .ready)
        XCTAssertEqual(SpecViews.stage(of: spec(), delivery("settled")), .ready)
    }

    func testMergedOrClosedAsCompletedIsShippedAndNotPlannedIsDropped() {
        XCTAssertEqual(SpecViews.stage(of: spec(), delivery("merged")), .shipped)
        XCTAssertEqual(SpecViews.stage(of: spec("done"), nil), .shipped)
        XCTAssertEqual(SpecViews.stage(of: spec("done"), delivery("running")), .shipped)
        XCTAssertEqual(SpecViews.stage(of: spec("cancelled"), nil), .dropped)
        XCTAssertEqual(SpecViews.stage(of: spec("cancelled"), delivery("ready")), .dropped)
    }

    func testADeliveryBelongsToItsSpecByRepoAndNumberOrByIssueWhenKeptInLoft() {
        let all = SpecViews.index([delivery("running", spec: 8), delivery("ready", spec: 7), delivery("running", spec: 1, local: true, specId: "loft-1")])
        XCTAssertEqual(SpecViews.delivery(of: spec(), in: all)?.state, "ready")
        var local = spec(id: "loft-1"); local.github = nil
        XCTAssertEqual(SpecViews.delivery(of: local, in: all)?.state, "running")
        XCTAssertNil(SpecViews.delivery(of: spec(number: 99), in: all))
    }

    func testProgressAndRef() {
        var d = delivery("running")
        d.tickets = [.init(number: 1, title: "a", state: "closed"), .init(number: 2, title: "b", state: "open")]
        XCTAssertEqual(SpecViews.progress(of: spec(), d, tasks: [])?.done, 1)
        XCTAssertEqual(SpecViews.progress(of: spec(), d, tasks: [])?.total, 2)
        XCTAssertNil(SpecViews.progress(of: spec(), nil, tasks: []))
        XCTAssertEqual(SpecViews.ref(spec()), "o/r#7")
        var local = spec(); local.github = nil
        XCTAssertEqual(SpecViews.ref(local), "Loft · loft")
    }

    func testProjectShowsNextToAGitHubRefButNotTwiceForALoftSpec() {
        XCTAssertEqual(SpecViews.project(spec()), "loft")
        var local = spec(); local.github = nil
        XCTAssertNil(SpecViews.project(local), "the ref already reads Loft · loft")
        var none = spec(); none.project = ""
        XCTAssertNil(SpecViews.project(none))
    }

    func testDecodesTheSummaryEndpoint() throws {
        let json = ###"[{"state":"running","started":true,"agent_id":"a","agent_name":"Dev","pr":null,"spec":146,"current":{"number":148,"title":"T"},"repo":"o/r","spec_id":"x","local":false,"tickets":[{"number":148,"id":null,"title":"T","state":"started","skipped":"","verdicts":{"qa":"pass"}}]}]"###
        let d = try JSONCoding.decoder.decode([DeliverySummary].self, from: Data(json.utf8))
        XCTAssertEqual(d[0].current?.number, 148)
        XCTAssertEqual(d[0].tickets[0].state, "started")
    }

    func testTicketsComeFromTheDeliveryWithTheirOwnIssueAndVerdicts() {
        var d = delivery("running")
        d.tickets = [.init(number: 1, title: "a", state: "closed", verdicts: ["qa": "pass"]), .init(number: 2, title: "b", state: "started", agentName: "Dev")]
        var own = spec(id: "own"); own.github = .init(repo: "o/r", number: 2, url: "", state: "open")
        let lines = SpecViews.tickets(of: spec(), d, tasks: [own])
        XCTAssertEqual(lines.map(\.number), [1, 2])
        XCTAssertEqual(lines[0].verdicts, ["qa": "pass"])
        XCTAssertNil(lines[0].taskId)
        XCTAssertEqual(lines[1].taskId, "own")
        XCTAssertTrue(lines[1].current)
    }

    func testWithoutADeliveryTicketsAreTheSubIssuesInLoft() {
        var kid = spec("done", id: "k"); kid.parentId = "s"; kid.github = nil
        var dropped = spec("cancelled", id: "x"); dropped.parentId = "s"
        let lines = SpecViews.tickets(of: spec(), nil, tasks: [kid, dropped])
        XCTAssertEqual(lines.map(\.taskId), ["k"])
        XCTAssertEqual(lines[0].state, "closed")
    }

    func testMergeOnlyForAReadyPlanKeptInLoft() {
        XCTAssertTrue(SpecViews.canMerge(spec(), delivery("ready", local: true)))
        XCTAssertFalse(SpecViews.canMerge(spec(), delivery("running", local: true)))
        XCTAssertFalse(SpecViews.canMerge(spec(), delivery("ready")), "a spec on GitHub merges there")
        XCTAssertFalse(SpecViews.canMerge(spec("done"), delivery("ready", local: true)))
    }

    /// #171: a spec's row and its detail say the same thing about its Delivery.
    func testDeliveryLabelAndWhoIsOnIt() {
        XCTAssertEqual(SpecViews.deliveryLabel("ready"), "Ready to merge")
        XCTAssertEqual(SpecViews.deliveryLabel("settled"), "Ready to merge")
        XCTAssertEqual(SpecViews.deliveryLabel("needs_human"), "Needs you")
        XCTAssertEqual(SpecViews.deliveryLabel("some_new_state"), "Some New State")
        var d = delivery("running")
        XCTAssertEqual(SpecViews.onIt(d), [])
        d.tickets = [.init(number: 43, title: "Backend endpoints", state: "started", agentName: "Backend Agent"),
                     .init(number: 44, title: "iOS screen", state: "open"),
                     .init(number: 45, title: "Docs", state: "review", agentName: "Docs Agent")]
        XCTAssertEqual(SpecViews.onIt(d), ["Backend Agent on #43 Backend endpoints", "Docs Agent on #45 Docs"], "every running ticket")
        XCTAssertNil(SpecViews.waitingLine(nil))
        XCTAssertNil(SpecViews.waitingLine([]))
        XCTAssertEqual(SpecViews.waitingLine([44, 47]), "Waiting for the Lead to assign #44, #47")
        XCTAssertNil(SpecViews.waitingOnYouLine([]))
        XCTAssertEqual(SpecViews.waitingOnYouLine([47]), "Waiting on you: #47")
        XCTAssertEqual(SpecViews.ticketLabel("needs_human"), "Waiting on you")
    }

    /// #181: the summary names every started ticket's agent and the ready tickets waiting for the Lead.
    func testDecodesEachTicketsAgentAndTheTicketsWaitingForTheLead() throws {
        let json = ###"[{"state":"running","agent_id":null,"agent_name":null,"current":null,"repo":"o/r","spec":9,"spec_id":"x","local":false,"waiting_for_lead":[3],"tickets":[{"number":1,"id":null,"title":"A","state":"started","skipped":"","agent_id":"a","agent_name":"Dev 1","verdicts":{}},{"number":2,"id":null,"title":"B","state":"review","skipped":"","agent_id":"b","agent_name":"Dev 2","verdicts":{}},{"number":3,"id":null,"title":"C","state":"open","skipped":"","agent_id":null,"agent_name":null,"verdicts":{}}]}]"###
        let d = try JSONCoding.decoder.decode([DeliverySummary].self, from: Data(json.utf8))[0]
        XCTAssertEqual(d.tickets.map(\.agentName), ["Dev 1", "Dev 2", nil])
        XCTAssertEqual(d.waitingForLead, [3])
        XCTAssertEqual(SpecViews.tickets(of: spec(), d, tasks: []).map(\.current), [true, true, false], "every running ticket is current")
    }
}
