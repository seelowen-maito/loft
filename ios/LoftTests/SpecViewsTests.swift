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
        let all = [delivery("running", spec: 8), delivery("ready", spec: 7), delivery("running", spec: 1, local: true, specId: "loft-1")]
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

    func testDecodesTheSummaryEndpoint() throws {
        let json = ###"[{"state":"running","started":true,"agent_id":"a","agent_name":"Dev","pr":null,"spec":146,"current":{"number":148,"title":"T"},"repo":"o/r","spec_id":"x","local":false,"tickets":[{"number":148,"id":null,"title":"T","state":"started","skipped":"","verdicts":{"qa":"pass"}}]}]"###
        let d = try JSONCoding.decoder.decode([DeliverySummary].self, from: Data(json.utf8))
        XCTAssertEqual(d[0].current?.number, 148)
        XCTAssertEqual(d[0].tickets[0].state, "started")
    }
}
