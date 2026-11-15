import XCTest
@testable import Loft
@testable import LoftKit

/// The Talk recipients both apps use (`TalkRecipients`, shared with the Mac since spec #382).
final class TalkAgentPickerTests: XCTestCase {
    private func agent(_ name: String, group: String? = nil, role: String? = nil, project: String? = nil) -> Agent {
        var a = Agent(id: name, name: name, status: "idle")
        a.group = group
        a.role = role
        a.project = project
        return a
    }

    private func ran(_ name: String, minutesAgo: Double?, lead: Bool = false, id: String? = nil, group: String? = nil) -> Agent {
        var a = Agent(id: id ?? name, name: name, status: "idle")
        a.lastRunAt = minutesAgo.map { Date().addingTimeInterval(-$0 * 60) }
        a.role = lead ? "lead" : "dev"
        a.group = group
        return a
    }

    /// The To row: the current agent, then every Lead by recency, then recent others up to five chips, no repeats.
    func testToRowShowsCurrentThenLeadsThenRecent() {
        let agents = [ran("Dev A", minutesAgo: 1), ran("Dev B", minutesAgo: 2), ran("Dev C", minutesAgo: 3), ran("Dev D", minutesAgo: 4),
                      ran("Old Lead", minutesAgo: 600, lead: true), ran("New Lead", minutesAgo: 30, lead: true), ran("Idle Lead", minutesAgo: nil, lead: true)]
        XCTAssertEqual(TalkRecipients.chips(agents, current: "Dev C").map(\.name), ["Dev C", "New Lead", "Old Lead", "Idle Lead", "Dev A"])
        XCTAssertEqual(TalkRecipients.chips(agents, current: "Old Lead").map(\.name), ["Old Lead", "New Lead", "Idle Lead", "Dev A", "Dev B"],
                       "the current Lead isn't shown twice")
        let manyLeads = (1...6).map { ran("Lead \($0)", minutesAgo: Double($0), lead: true) } + [ran("Dev A", minutesAgo: 0)]
        XCTAssertEqual(TalkRecipients.chips(manyLeads, current: nil).count, 6, "every Lead, even past five")
    }

    /// Two Leads with the same name show their group; a unique name shows alone.
    func testSameNamedLeadsShowTheirGroup() {
        let shown = [ran("Lead", minutesAgo: 1, lead: true, id: "l1", group: "Dineout"), ran("Lead", minutesAgo: 2, lead: true, id: "l2", group: "Loft"),
                     ran("Loft Dev", minutesAgo: 3, group: "Loft")]
        XCTAssertEqual(shown.map { TalkRecipients.label($0, among: shown) }, ["Lead · Dineout", "Lead · Loft", "Loft Dev"])
    }

    /// Talk's To row only offered five agents; More lists every one of them, by group, no group last.
    func testEveryAgentIsListedByGroup() {
        let agents = (1...8).map { agent("Agent \($0)", group: $0.isMultiple(of: 2) ? "Loft" : nil) }
            + [agent("Menu Dev", group: "Dineout")]
        let sections = TalkRecipients.sections(agents, query: "")
        XCTAssertEqual(sections.map(\.title), ["Dineout", "Loft", "No group"])
        XCTAssertEqual(sections.flatMap(\.agents).count, 9)
        XCTAssertEqual(sections[1].agents.map(\.name), ["Agent 2", "Agent 4", "Agent 6", "Agent 8"])
    }

    func testSearchMatchesNameRoleGroupOrProject() {
        let agents = [agent("Menu Dev", group: "Dineout", role: "dev", project: "menu"), agent("Checker", role: "code_reviewer"),
                      agent("Docs Agent", role: "custom", project: "website")]
        func names(_ q: String) -> [String] { TalkRecipients.sections(agents, query: q).flatMap(\.agents).map(\.name) }
        XCTAssertEqual(names("menu"), ["Menu Dev"])
        XCTAssertEqual(names("DINEOUT"), ["Menu Dev"])
        XCTAssertEqual(names("code rev"), ["Checker"], "the Role label, not its id")
        XCTAssertEqual(names("CUSTOM"), ["Docs Agent"])
        XCTAssertEqual(names("websi"), ["Docs Agent"])
        XCTAssertEqual(names("nobody"), [])
        XCTAssertEqual(names("code_reviewer"), [])
    }

    /// The role label table follows the backend's roles; no role (an older server) reads as Custom, an unknown one as it is.
    func testRoleLabels() {
        XCTAssertEqual(Agent.roles.map(\.id), ["lead", "dev", "code_reviewer", "qa", "security", "custom"])
        XCTAssertEqual(Agent.roles.map(\.label), ["Lead", "Dev", "Code reviewer", "QA", "Security", "Custom"])
        XCTAssertEqual(agent("A", role: "code_reviewer").roleLabel, "Code reviewer")
        XCTAssertEqual(agent("A", role: "custom").roleLabel, "Custom")
        XCTAssertEqual(agent("A").roleLabel, "Custom")
        XCTAssertEqual(agent("A", role: "").roleLabel, "Custom")
        XCTAssertEqual(agent("A", role: "designer").roleLabel, "designer")
    }
}
