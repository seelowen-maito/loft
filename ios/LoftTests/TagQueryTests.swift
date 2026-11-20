import XCTest
@testable import Loft
@testable import LoftKit

final class TagQueryTests: XCTestCase {
    func testOnlyAHashTokenAtTheEndOpensThePicker() {
        XCTAssertEqual(TagQuery.query(in: "see #"), "")
        XCTAssertEqual(TagQuery.query(in: "#18"), "18")
        XCTAssertEqual(TagQuery.query(in: "fix (#18"), "18")
        XCTAssertNil(TagQuery.query(in: "see #18 and"), "a finished tag")
        XCTAssertNil(TagQuery.query(in: "color#fff"), "a # inside a word")
        XCTAssertNil(TagQuery.query(in: "hello"))
    }

    func testAPickReplacesTheTokenAndEndsItWithASpace() {
        XCTAssertEqual(TagQuery.insert("#199", into: "look at #sp"), "look at #199 ")
        XCTAssertEqual(TagQuery.insert("o/api#5", into: "#"), "o/api#5 ")
    }

    private func task(_ id: String, _ title: String, project: String = "loft", labels: [String] = [], github: HBTask.GitHubLink? = nil) -> HBTask {
        var t = HBTask(id: id, title: title, description: "", kind: "work", status: "inbox", priority: "normal", project: project, agentId: nil,
                       parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "",
                       createdAt: .now, updatedAt: .now, startedAt: nil, completedAt: nil)
        t.labels = labels; t.github = github
        return t
    }
    private func link(_ repo: String, _ n: Int) -> HBTask.GitHubLink { .init(repo: repo, number: n, url: "", state: "open") }
    private func pr(_ title: String, _ slug: String, _ n: Int) -> PullRequest {
        PullRequest(id: "p\(n)\(title)", agentId: "a", slug: slug, number: n, url: "https://x/\(n)", title: title, state: "open")
    }

    func testTagsAreHashNInTheAgentsProjectOwnerRepoElsewhereAndTheIdForALoftOnlyIssue() {
        let rows = TagQuery.rows(
            tasks: [task("1", "Tag", labels: ["spec"], github: link("o/loft", 199)),
                    task("2", "Elsewhere", project: "api", github: link("o/api", 5)),
                    task("3", "Local")],
            prs: [pr("Spec PR", "o/loft", 200), pr("Spec PR", "o/loft", 200)], project: "loft")
        XCTAssertEqual(rows.map { "\($0.kind.rawValue) \($0.tag)" }, ["spec #199", "issue o/api#5", "issue #3", "pr #200"], "one PR, however many agents have it")
        XCTAssertEqual(TagQuery.rows(tasks: [task("1", "T", github: link("o/loft", 1))], prs: [], project: "")[0].tag, "o/loft#1", "no project: the full form")
    }

    func testMatchesListSpecsIssuesThenPRsAndANumberFindsThatItem() {
        let rows = TagQuery.rows(
            tasks: [task("1", "Tag things", labels: ["spec"], github: link("o/loft", 199)), task("2", "Tag bug", github: link("o/loft", 7))],
            prs: [pr("Tag PR", "o/loft", 200)], project: "loft")
        XCTAssertEqual(TagQuery.matches(rows, "tag").map(\.kind), [.spec, .issue, .pr])
        XCTAssertEqual(TagQuery.matches(rows, "7").map(\.tag), ["#7"])
        XCTAssertEqual(TagQuery.matches(rows, "").count, 3, "nothing typed: the open ones")
    }

    func testMatchingIsTheWebSearch() {
        let rows = TagQuery.rows(
            tasks: [task("1", "Old tag work", github: link("o/loft", 199)),
                    task("2", "Fix tags", github: link("o/loft", 1)),
                    task("3", "Billing", project: "api", github: link("o/api", 12)),
                    task("4", "Retag the board", github: link("o/loft", 3))],
            prs: [], project: "loft")
        XCTAssertEqual(TagQuery.matches(rows, "1").map(\.tag), ["#1"], "a number finds that number only, not #199 or #12")
        XCTAssertEqual(TagQuery.matches(rows, "api").map(\.title), ["Billing"], "the project counts too")
        XCTAssertEqual(TagQuery.matches(rows, "tag").map(\.title), ["Old tag work", "Fix tags", "Retag the board"], "titles containing it, in order")
        XCTAssertEqual(TagQuery.matches(rows, "TAG").map(\.title), ["Old tag work", "Fix tags", "Retag the board"], "case doesn't matter")
        XCTAssertEqual(TagQuery.matches(rows, "fix").map(\.title), ["Fix tags"])

        var closed = task("5", "Tag closed", github: link("o/loft", 5)); closed.status = "done"
        let ranked = TagQuery.rows(tasks: [closed, task("6", "A tag", github: link("o/loft", 6)), task("7", "Tag open", github: link("o/loft", 7))], prs: [], project: "loft")
        XCTAssertEqual(TagQuery.matches(ranked, "tag").map(\.title), ["Tag open", "A tag", "Tag closed"], "open first, then titles starting with it")
    }
}
