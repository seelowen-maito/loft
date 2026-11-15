import XCTest
@testable import Loft
@testable import LoftKit

final class IssuePickerOrderTests: XCTestCase {
    private func agent(_ name: String, role: String?, projects: [String]) -> Agent {
        var a = Agent(id: name, name: name, status: "idle")
        a.role = role
        a.project = projects.first
        a.projects = projects
        return a
    }

    private func names(_ agents: [Agent], _ project: String) -> [String] {
        Agent.issuePickerOrder(agents, project: project).map(\.name)
    }

    /// On a loft issue the Dev is the default, not the Code reviewer that sorts before it by name.
    func testLoftDevBeforeLoftCodeReview() {
        let agents = [agent("Loft Code Review", role: "code_reviewer", projects: ["loft"]), agent("Loft Dev", role: "dev", projects: ["loft"])]
        XCTAssertEqual(names(agents, "loft"), ["Loft Dev", "Loft Code Review"])
    }

    /// The project's agents first, Devs first in each part, then by name.
    func testProjectThenDevThenName() {
        let agents = [agent("Zed QA", role: "qa", projects: ["loft"]), agent("Api Dev", role: "dev", projects: ["api"]),
                      agent("Alpha Lead", role: "lead", projects: ["api"]), agent("Loft Dev", role: "dev", projects: ["loft"]),
                      agent("Bee Dev", role: "dev", projects: ["api", "loft"]), agent("Loose", role: nil, projects: [])]
        XCTAssertEqual(names(agents, "loft"), ["Bee Dev", "Loft Dev", "Zed QA", "Api Dev", "Alpha Lead", "Loose"])
    }

    /// An issue with no project still lists Devs first.
    func testNoProjectListsDevsFirst() {
        let agents = [agent("Loft Code Review", role: "code_reviewer", projects: ["loft"]), agent("Api Dev", role: "dev", projects: ["api"]),
                      agent("Loft Dev", role: "dev", projects: ["loft"])]
        XCTAssertEqual(names(agents, ""), ["Api Dev", "Loft Dev", "Loft Code Review"])
    }
}
