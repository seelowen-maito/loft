import XCTest
@testable import LoftKit

/// Which Needs you items notify on the Mac (spec #382).
final class NeedsYouAlertsTests: XCTestCase {
    private func item(_ id: String, _ type: String, agent: String? = "a1", open: NeedsYouAction.Target? = nil) -> NeedsYouItem {
        NeedsYouItem(id: id, type: type, urgency: 2, agent: agent.map { .init(id: $0, name: "Agent") }, taskId: nil, title: "t", reason: "", text: "",
                     createdAt: nil, ageSeconds: nil, links: nil,
                     actions: open.map { [NeedsYouAction(kind: "open", label: "Open", target: $0)] } ?? [], project: nil, pr: nil)
    }
    private func list(_ items: NeedsYouItem...) -> NeedsYouList {
        NeedsYouList(version: 1, count: items.count, urgent: 0, generatedAt: nil, items: items)
    }

    func testTheFirstListOnlySetsTheBaseline() {
        var a = NeedsYouAlerts()
        XCTAssertTrue(a.fresh(in: list(item("q1", "question"), item("p1", "permission"))).isEmpty)
    }

    func testQuestionsPermissionsAndReadyPRsNotifyOnce() {
        var a = NeedsYouAlerts()
        _ = a.fresh(in: list())
        let l = list(item("q1", "question"), item("p1", "permission"), item("r1", "pr_ready"))
        XCTAssertEqual(a.fresh(in: l).map(\.id), ["q1", "p1", "r1"])
        XCTAssertTrue(a.fresh(in: l).isEmpty, "the same items don't notify again")
    }

    func testOtherKindsStayQuiet() {
        var a = NeedsYouAlerts()
        _ = a.fresh(in: list())
        XCTAssertTrue(a.fresh(in: list(item("s1", "stuck"), item("t1", "task_approval"), item("v1", "pr_review"))).isEmpty)
    }

    func testAnItemThatLeavesAndComesBackNotifiesAgain() {
        var a = NeedsYouAlerts()
        _ = a.fresh(in: list(item("q1", "question")))
        XCTAssertTrue(a.fresh(in: list()).isEmpty, "it left the list")
        XCTAssertEqual(a.fresh(in: list(item("q1", "question"))).map(\.id), ["q1"], "back on the list: a new notification")
    }

    func testTheBadgeIsTheCountAndNoneWhenEmptyOrNotLoaded() {
        XCTAssertEqual(NeedsYouAlerts.badge(for: list(item("a", "question"), item("b", "stuck"))), "2")
        XCTAssertNil(NeedsYouAlerts.badge(for: list()))
        XCTAssertNil(NeedsYouAlerts.badge(for: nil))
    }

    func testTheTargetIsTheAgentsChatOrTheLink() {
        XCTAssertEqual(NeedsYouAlerts.target(of: item("q", "question", agent: "a9")), ["screen": "chat", "agent_id": "a9"])
        XCTAssertEqual(NeedsYouAlerts.target(of: item("r", "pr_ready", agent: nil, open: .init(url: "https://github.com/o/r/pull/1"))), ["url": "https://github.com/o/r/pull/1"])
        XCTAssertEqual(NeedsYouAlerts.target(of: item("x", "pr_ready", agent: nil)), ["screen": "needs_you"])
    }
}
