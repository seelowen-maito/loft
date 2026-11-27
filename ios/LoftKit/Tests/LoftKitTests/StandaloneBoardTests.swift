import XCTest
@testable import LoftKit

/// A standalone ticket on the iPhone's and the Mac's board: once its Delivery starts it leaves the Issues column and moves
/// through the spec stages, like the web's (`specViews.test.ts`).
final class StandaloneBoardTests: XCTestCase {
    private func issue(_ id: String, number: Int?, labels: [String] = [], parent: String? = nil, status: String = "ready") -> HBTask {
        var t = HBTask(id: id, title: id, description: "", kind: "work", status: status, priority: "normal", project: "loft", agentId: nil,
                       parentId: parent, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "", createdAt: Date(),
                       updatedAt: Date())
        t.labels = labels
        t.github = number.map { .init(repo: "o/r", number: $0, url: "", state: "open") }
        return t
    }

    private func delivery(_ state: String, number: Int, standalone: Bool? = true, local: Bool = false, specId: String? = nil,
                          tickets: [DeliverySummary.Ticket] = []) -> DeliverySummary {
        var d = DeliverySummary(state: state, agentName: "Dev", repo: local ? "loft:loft" : "o/r", spec: number, specId: specId, local: local,
                                current: nil, tickets: tickets)
        d.standalone = standalone
        return d
    }

    private func board(_ tasks: [HBTask], _ deliveries: [DeliverySummary]) -> (issues: [String], stages: [SpecStage: [String]]) {
        let index = SpecViews.index(deliveries)
        let issues = SpecViews.boardIssues(tasks, deliveries, showClosed: true).map(\.id)
        var stages: [SpecStage: [String]] = [:]
        for s in SpecStage.allCases { stages[s] = SpecViews.specs(tasks, in: s, index).map(\.id) }
        return (issues, stages)
    }

    func testDecodesStandaloneAndNeedsHumanAndAnOlderServerSendsNeither() throws {
        let json = ###"[{"state":"running","agent_name":"Dev","spec":20,"repo":"o/r","spec_id":"","local":false,"standalone":true,"needs_human":true,"current":{"number":20,"title":"T"},"tickets":[]},{"state":"running","spec":9,"repo":"o/r","spec_id":"","local":false,"current":null,"tickets":[]}]"###
        let d = try JSONCoding.decoder.decode([DeliverySummary].self, from: Data(json.utf8))
        XCTAssertTrue(d[0].isStandalone)
        XCTAssertEqual(d[0].needsHuman, true)
        XCTAssertFalse(d[1].isStandalone)
    }

    func testAStartedStandaloneTicketLeavesIssuesForTheStagesInTheSameStep() {
        let t = issue("t", number: 20)
        for (state, stage) in [("running", SpecStage.delivery), ("needs_human", .delivery), ("final_review", .delivery), ("ready", .ready), ("merged", .shipped)] {
            let b = board([t], [delivery(state, number: 20)])
            XCTAssertEqual(b.issues, [], state)
            XCTAssertEqual(b.stages[stage], ["t"], state)
            XCTAssertEqual(SpecStage.allCases.flatMap { b.stages[$0] ?? [] }, ["t"], "\(state): in one stage only")
        }
    }

    func testBeforeItsDeliveryStartsItStaysInIssues() {
        let t = issue("t", number: 20)
        for ds in [[delivery("planned", number: 20)], []] {
            let b = board([t], ds)
            XCTAssertEqual(b.issues, ["t"])
            XCTAssertEqual(SpecStage.allCases.flatMap { b.stages[$0] ?? [] }, [])
        }
    }

    func testAStandaloneTicketKeptInLoftIsFoundByItsIssue() {
        let t = issue("loft-1", number: nil)
        let b = board([t], [delivery("running", number: 1, local: true, specId: "loft-1")])
        XCTAssertEqual(b.issues, [])
        XCTAssertEqual(b.stages[.delivery], ["loft-1"])
    }

    func testItsCardIsATicketsNoProgressItsDevAndNeedsYou() {
        let t = issue("t", number: 20)
        var d = delivery("running", number: 20)
        XCTAssertNil(SpecViews.progress(of: t, d, tasks: [issue("kid", number: 21, parent: "t")]), "no progress bar")
        XCTAssertEqual(SpecViews.onIt(d), ["Dev is on it"])
        XCTAssertEqual(SpecViews.badge(d), "Running")
        d.needsHuman = true
        XCTAssertEqual(SpecViews.onIt(d), [], "nobody is on it while it waits for the user")
        XCTAssertEqual(SpecViews.badge(d), "Needs you")
        XCTAssertTrue(SpecViews.waitsForYou(d))
        d.needsHuman = nil
        d.state = "ready"
        XCTAssertEqual(SpecViews.onIt(d), [])
    }

    func testSpecsAreAsBefore() {
        let spec = issue("s", number: 9, labels: ["spec"])
        let ticket = issue("k", number: 10, parent: "s")
        let plain = issue("p", number: 30)
        var d = delivery("running", number: 9, standalone: nil, tickets: [.init(number: 10, title: "k", state: "started", agentName: "Dev")])
        let b = board([spec, ticket, plain], [d])
        XCTAssertEqual(b.issues, ["p"])
        XCTAssertEqual(b.stages[.delivery], ["s"])
        XCTAssertEqual(SpecViews.onIt(d), ["Dev on #10 k"])
        XCTAssertEqual(SpecViews.progress(of: spec, d, tasks: [])?.total, 1)
        d.state = "planned"
        XCTAssertEqual(board([spec], [d]).stages[.planned], ["s"], "a spec not started is Planned, not in Issues")
        XCTAssertEqual(board([spec], [d]).issues, [])
    }
}
