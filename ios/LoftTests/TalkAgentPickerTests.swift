import XCTest
@testable import Loft

final class TalkAgentPickerTests: XCTestCase {
    private func agent(_ name: String, group: String? = nil, title: String? = nil, project: String? = nil) -> Agent {
        var a = Agent(id: name, name: name, status: "idle")
        a.group = group
        a.title = title
        a.project = project
        return a
    }

    /// Talk's To row only offered five agents; More lists every one of them, by group, no group last.
    func testEveryAgentIsListedByGroup() {
        let agents = (1...8).map { agent("Agent \($0)", group: $0.isMultiple(of: 2) ? "Loft" : nil) }
            + [agent("Menu Dev", group: "Dineout")]
        let sections = TalkAgentPicker.sections(agents, query: "")
        XCTAssertEqual(sections.map(\.title), ["Dineout", "Loft", "No group"])
        XCTAssertEqual(sections.flatMap(\.agents).count, 9)
        XCTAssertEqual(sections[1].agents.map(\.name), ["Agent 2", "Agent 4", "Agent 6", "Agent 8"])
    }

    func testSearchMatchesNameTitleGroupOrProject() {
        let agents = [agent("Menu Dev", group: "Dineout", project: "menu"), agent("QA Agent", title: "Runs the iOS suite"),
                      agent("Docs Agent", project: "website")]
        func names(_ q: String) -> [String] { TalkAgentPicker.sections(agents, query: q).flatMap(\.agents).map(\.name) }
        XCTAssertEqual(names("menu"), ["Menu Dev"])
        XCTAssertEqual(names("DINEOUT"), ["Menu Dev"])
        XCTAssertEqual(names("ios suite"), ["QA Agent"])
        XCTAssertEqual(names("websi"), ["Docs Agent"])
        XCTAssertEqual(names("nobody"), [])
    }
}
