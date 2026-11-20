import XCTest
@testable import LoftKit

/// The Mac's quick search: what a query matches and in which order.
final class QuickSearchTests: XCTestCase {
    private let agents = [Agent(id: "dev", name: "Loft mobile dev", status: "idle"), Agent(id: "qa", name: "Loft Mobile QA", status: "idle"),
                          Agent(id: "orch", name: "Orchestrator", status: "idle")]

    private func task(_ id: String, _ title: String, number: Int? = nil, labels: [String] = [], parent: String? = nil,
                      status: String = "ready", updated: Date = Date()) -> HBTask {
        var t = HBTask(id: id, title: title, description: "", kind: "work", status: status, priority: "normal", project: "loft", agentId: nil,
                       parentId: parent, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "", createdAt: updated,
                       updatedAt: updated)
        t.labels = labels
        t.github = number.map { .init(repo: "o/loft", number: $0, url: "", state: "open") }
        return t
    }

    private func search(_ q: String, tasks: [HBTask] = []) -> [QuickSearch.Result] {
        QuickSearch.results(q, agents: agents, tasks: tasks, deliveries: [])
    }

    func testAnEmptyQueryFindsNothing() {
        XCTAssertEqual(search("  "), [])
    }

    func testKindsAreTold() {
        let tasks = [task("s", "Mac quick search", number: 456, labels: ["spec"]), task("t", "Quick search panel", number: 459, parent: "s"),
                     task("i", "Quick fix for search")]
        let r = search("quick", tasks: tasks)
        XCTAssertEqual(Dictionary(uniqueKeysWithValues: r.map { ($0.target, $0.kind) }), ["s": .spec, "t": .ticket, "i": .issue])
        XCTAssertEqual(r.first { $0.target == "t" }?.number, "loft#459")
        XCTAssertNil(r.first { $0.target == "i" }?.number)
    }

    func testRankingWholeTitleThenStartThenWordThenAnywhereThenInitials() {
        XCTAssertEqual(QuickSearch.score("orchestrator", title: "Orchestrator"), 1)
        XCTAssertEqual(QuickSearch.score("orch", title: "Orchestrator"), 2)
        XCTAssertEqual(QuickSearch.score("qa", title: "Loft Mobile QA"), 3)
        XCTAssertEqual(QuickSearch.score("bile", title: "Loft Mobile QA"), 4)
        XCTAssertEqual(QuickSearch.score("lmq", title: "Loft Mobile QA"), 5)
        XCTAssertNil(QuickSearch.score("zz", title: "Loft Mobile QA"))
        XCTAssertEqual(search("lo").map(\.target), ["dev", "qa"], "Both start with it, by name")
        XCTAssertEqual(search("qa").first?.target, "qa")
    }

    func testLettersInOrderOnlyOnWordInitialsFromThreeLetters() {
        XCTAssertNil(QuickSearch.score("needs", title: "Implement backend endpoints"), "Not letters spread inside words")
        XCTAssertNil(QuickSearch.score("lm", title: "Loft Mobile QA"), "Too short")
        XCTAssertEqual(QuickSearch.score("lmq", title: "Loft Mobile QA"), 5)
        XCTAssertEqual(QuickSearch.score("l mq", title: "Loft Mobile QA"), 5)
    }

    func testANumberFindsItsIssueFirst() {
        let tasks = [task("a", "Ship 459 things"), task("t", "Quick search panel", number: 459)]
        for q in ["459", "#459", "loft#459", "Loft#459"] { XCTAssertEqual(search(q, tasks: tasks).first?.target, "t", q) }
        // Another repo's #459 isn't this one.
        for q in ["other#459", "xyz#459"] { XCTAssertNotEqual(search(q, tasks: tasks).first?.target, "t", q) }
        XCTAssertNil(QuickSearch.score("other#459", title: "Quick search panel", number: "loft#459"))
    }

    func testEqualMatchesPutOpenFirstThenAgentsSpecsTicketsIssuesThenTheLatest() {
        let old = Date(timeIntervalSince1970: 0)
        let tasks = [task("closed", "Search", status: "done"), task("issue-old", "Search", updated: old), task("issue-new", "Search"),
                     task("spec", "Search", labels: ["spec"], updated: old)]
        XCTAssertEqual(search("search", tasks: tasks).map(\.target), ["spec", "issue-new", "issue-old", "closed"])
    }

    func testPagesAreFoundAndThoseTheMacLacksAreWebOnly() {
        let r = search("simu")
        XCTAssertEqual(r.first?.kind, .page)
        XCTAssertEqual(r.first?.webPath, "/simulator")
        XCTAssertEqual(r.first?.webOnly, true)
        XCTAssertEqual(search("clean").first?.webOnly, true)
        XCTAssertEqual(search("needs").first { $0.kind == .page }?.webOnly, false)
    }

    func testWebPaths() {
        let r = search("search", tasks: [task("t1", "Search")])
        XCTAssertEqual(r.first?.webPath, "/issues/t1")
        XCTAssertEqual(search("orch").first?.webPath, "/agents/orch")
    }

    func testRowsReadLikeTheWebsSearch() {
        var dev = Agent(id: "dev", name: "Loft mobile dev", status: "idle")
        dev.project = "loft"
        dev.role = "lead"
        XCTAssertEqual(QuickSearch.results("mobile dev", agents: [dev], tasks: [], deliveries: []).first?.detail, "loft · Lead")
        XCTAssertEqual(search("orch").first?.detail, "Custom", "No project: the role alone")
        var done = task("d", "Search done", number: 480, status: "done")
        done.github?.state = "closed"
        let r = search("search", tasks: [task("o", "Search open", number: 12), done, task("l", "Search local")])
        XCTAssertEqual(Dictionary(uniqueKeysWithValues: r.map { ($0.target, $0.detail) }),
                       ["o": "#12 · open · loft", "l": "open · loft", "d": "#480 · closed · loft"])
    }

    func testPullRequestsAreFoundOnceAndOpenOnGitHub() {
        func pr(_ n: Int, _ title: String, state: String = "open", draft: Bool? = nil, agent: String = "dev") -> PullRequest {
            PullRequest(id: "\(agent)\(n)", agentId: agent, slug: "o/loft", number: n, url: "https://github.com/o/loft/pull/\(n)",
                        title: title, state: state, draft: draft)
        }
        let prs = [pr(7, "Search merged", state: "merged"), pr(9, "Search rows", draft: true), pr(9, "Search rows", draft: true, agent: "qa")]
        let r = QuickSearch.results("search", agents: [], tasks: [], deliveries: [], pullRequests: prs)
        XCTAssertEqual(r.map(\.target), ["https://github.com/o/loft/pull/9", "https://github.com/o/loft/pull/7"])
        XCTAssertEqual(r.map(\.detail), ["#9 · draft · loft", "#7 · merged · loft"])
        XCTAssertEqual(r.first?.kind, .pullRequest)
        XCTAssertEqual(r.first?.webOnly, true)
        XCTAssertEqual(QuickSearch.results("#9", agents: [], tasks: [], deliveries: [], pullRequests: prs).count, 1)
        XCTAssertEqual(QuickSearch.results("loft#9", agents: [], tasks: [], deliveries: [], pullRequests: prs).count, 1)
        XCTAssertEqual(QuickSearch.results("api#9", agents: [], tasks: [], deliveries: [], pullRequests: prs).count, 0)
    }

    func testAPullRequestOpensOnlyATrustedLink() {
        func row(_ url: String, kind: QuickSearch.Kind = .pullRequest) -> QuickSearch.Result {
            QuickSearch.Result(kind: kind, target: url, title: "PR", webPath: "")
        }
        XCTAssertEqual(row("https://github.com/o/loft/pull/9").link(loftHost: nil), URL(string: "https://github.com/o/loft/pull/9"))
        XCTAssertEqual(row("https://mac.ts.net/x").link(loftHost: "mac.ts.net"), URL(string: "https://mac.ts.net/x"))
        XCTAssertNil(row("file:///System/Applications/Calculator.app").link(loftHost: nil))
        XCTAssertNil(row("smb://host/share").link(loftHost: "host"))
        XCTAssertNil(row("http://github.com/o/loft/pull/9").link(loftHost: nil))
        XCTAssertNil(row("https://evil.example/pull/9").link(loftHost: "mac.ts.net"))
        XCTAssertNil(row("https://github.com/o/loft/pull/9", kind: .agent).link(loftHost: nil))
    }

    func testAgentStatusAsTheWebTellsIt() {
        func agent(_ status: String, running: Bool? = nil, waitingOn: [String]? = nil, every: Int? = nil) -> QuickSearch.AgentStatus {
            var a = Agent(id: "a", name: "A", status: status)
            a.running = running
            a.waitingOn = waitingOn
            a.intervalMinutes = every
            return QuickSearch.AgentStatus(a)
        }
        XCTAssertEqual(agent("waiting", running: true), .needsYou)
        XCTAssertEqual(agent("idle", running: true, waitingOn: ["480"]), .waiting)
        XCTAssertEqual(agent("idle", running: true), .working)
        XCTAssertEqual(agent("running"), .idle, "a saved running without a live run is stale")
        XCTAssertEqual(agent("running", running: true), .working)
        XCTAssertEqual(agent("error"), .error)
        XCTAssertEqual(agent("idle", every: 30), .sleeping)
        XCTAssertEqual(agent("idle"), .idle)
        XCTAssertEqual(QuickSearch.AgentStatus.sleeping.label, "Sleeping")
    }
}
