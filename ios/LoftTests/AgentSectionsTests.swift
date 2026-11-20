import XCTest
@testable import Loft
@testable import LoftKit

final class AgentSectionsTests: XCTestCase {
    private func agent(_ name: String, group: String? = nil, lead: Bool = false) -> Agent {
        Agent(id: name, name: name, status: "idle", role: lead ? "lead" : "dev", group: group)
    }

    func testLeadsFirstThenGroupsThenNoGroup() {
        let agents = [
            agent("Zed", group: "passora"), agent("Solo"), agent("Lead B", group: "Dineout", lead: true),
            agent("Amy", group: "Loft"), agent("Lead A", group: "Loft", lead: true), agent("Blank", group: "  "),
            agent("Bob", group: "Loft"),
        ]
        let sections = AgentsListView.sections(agents)
        // Groups sort ignoring case; Dineout has only its Lead, so no Dineout section.
        XCTAssertEqual(sections.map(\.title), ["Leads", "Loft", "passora", "No group"])
        XCTAssertEqual(sections.first?.agents.map(\.name), ["Lead B", "Lead A"])
        XCTAssertEqual(sections.first { $0.title == "Loft" }?.agents.map(\.name), ["Amy", "Bob"])
        XCTAssertEqual(sections.last?.title, "No group")
        XCTAssertEqual(sections.last?.agents.map(\.name), ["Solo", "Blank"])
        // A Lead is only in Leads.
        XCTAssertEqual(sections.flatMap(\.agents).count, agents.count)
    }

    func testEmptySectionsAreLeftOut() {
        XCTAssertEqual(AgentsListView.sections([agent("Amy", group: "Loft")]).map(\.title), ["Loft"])
        XCTAssertTrue(AgentsListView.sections([]).isEmpty)
    }

    func testChatTabGroupsLikeAgentsTabWithoutTheOrchestrator() {
        let agents = [agent("Solo"), agent("Amy", group: "Loft"), agent("Lead A", group: "Loft", lead: true),
                      agent("Orch", group: "Loft")]
        let sections = ConversationsView.sections(agents, orchestratorId: "Orch")
        XCTAssertEqual(sections.map(\.title), ["Leads", "Loft", "No group"])
        XCTAssertEqual(sections.map(\.title), AgentsListView.sections(agents).map(\.title))
        XCTAssertFalse(sections.flatMap(\.agents).contains { $0.id == "Orch" })
        XCTAssertEqual(ConversationsView.sections(agents, orchestratorId: nil).flatMap(\.agents).count, agents.count)
    }
}
