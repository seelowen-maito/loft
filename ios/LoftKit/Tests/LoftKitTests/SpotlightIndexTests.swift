import CoreSpotlight
import XCTest
@testable import LoftKit

/// Loft in macOS Spotlight: the items built from the store, how a chosen one is read back, and what a change re-indexes.
final class SpotlightIndexTests: XCTestCase {
    private let agents = [Agent(id: "dev", name: "Loft mobile dev", status: "idle")]

    private func task(_ id: String, _ title: String, number: Int? = nil, labels: [String] = [], parent: String? = nil,
                      status: String = "ready") -> HBTask {
        var t = HBTask(id: id, title: title, description: "", kind: "work", status: status, priority: "normal", project: "loft", agentId: nil,
                       parentId: parent, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "", createdAt: Date(),
                       updatedAt: Date())
        t.labels = labels
        t.github = number.map { .init(repo: "o/loft", number: $0, url: "", state: "open") }
        return t
    }

    func testAgentsAndOpenIssuesTicketsAndSpecsAreIndexedNotClosedOnes() {
        let tasks = [task("s", "Mac app web look", number: 456, labels: ["spec"]), task("t", "Loft in macOS Spotlight", number: 460, parent: "s"),
                     task("i", "Fix the badge"), task("c", "Old work", status: "done")]
        let r = SpotlightIndex.results(agents: agents, tasks: tasks, deliveries: [])
        XCTAssertEqual(r.map(\.id), ["agent:dev", "spec:s", "ticket:t", "issue:i"])
    }

    func testAnItemCarriesTitleKindNumberAndItsIdentifier() {
        let r = SpotlightIndex.results(agents: [], tasks: [task("t", "Loft in macOS Spotlight", number: 460, parent: "s"),
                                                           task("s", "Spec", labels: ["spec"])], deliveries: [])[0]
        let item = SpotlightIndex.item(r)
        XCTAssertEqual(item.uniqueIdentifier, "ticket:t")
        XCTAssertEqual(item.domainIdentifier, "loft")
        XCTAssertEqual(item.attributeSet.title, "Loft in macOS Spotlight")
        XCTAssertEqual(item.attributeSet.contentDescription, "Ticket · loft#460")
        XCTAssertEqual(item.attributeSet.keywords, ["Ticket", "460", "#460"])

        let agent = SpotlightIndex.item(SpotlightIndex.results(agents: agents, tasks: [], deliveries: [])[0])
        XCTAssertEqual(agent.attributeSet.title, "Loft mobile dev")
        XCTAssertEqual(agent.attributeSet.contentDescription, "Agent")
        XCTAssertEqual(agent.attributeSet.keywords, ["Agent"])
    }

    func testAChosenItemNamesWhatToOpen() {
        XCTAssertEqual(SpotlightIndex.target(of: "agent:dev")?.kind, .agent)
        XCTAssertEqual(SpotlightIndex.target(of: "agent:dev")?.id, "dev")
        XCTAssertEqual(SpotlightIndex.target(of: "issue:a:b")?.id, "a:b")
        XCTAssertNil(SpotlightIndex.target(of: "page:/repos"))
        XCTAssertNil(SpotlightIndex.target(of: "agent:"))
        XCTAssertNil(SpotlightIndex.target(of: "nonsense"))
    }

    func testNewRenamedAndClosedItemsChangeTheIndex() {
        let before = SpotlightIndex.results(agents: agents, tasks: [task("a", "One"), task("b", "Two")], deliveries: [])
        let after = SpotlightIndex.results(agents: agents, tasks: [task("a", "One, renamed"), task("b", "Two", status: "done"), task("c", "Three")],
                                           deliveries: [])
        let change = SpotlightIndex.changes(from: before, to: after)
        XCTAssertEqual(change.index.map(\.id), ["issue:a", "issue:c"])
        XCTAssertEqual(change.remove, ["issue:b"])
        XCTAssertTrue(SpotlightIndex.changes(from: after, to: after).index.isEmpty)
    }
}
