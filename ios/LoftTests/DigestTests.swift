import XCTest
@testable import Loft

final class DigestTests: XCTestCase {
    private let now = Date()
    private let agents = [Agent(id: "be", name: "Backend Agent", status: "idle"), Agent(id: "qa", name: "QA Agent", status: "idle"),
                          Agent(id: "docs", name: "Docs Agent", status: "error")]
    private func a(_ type: String, _ agent: String, _ summary: String, minutesAgo: Double) -> ActivityItem {
        ActivityItem(id: UUID().uuidString, type: type, at: now.addingTimeInterval(-minutesAgo * 60), agentId: agent, taskId: nil,
                     messageId: nil, summary: summary, severity: "info")
    }

    func testOnlyWhatMattersSinceYouLooked() {
        let d = Digest.build(since: now.addingTimeInterval(-3600), activity: [
            a("task.completed", "be", "Backend Agent finished task “Inspect existing architecture”", minutesAgo: 30),
            a("agent.started", "be", "Backend Agent started working", minutesAgo: 31),
            a("agent.message", "qa", "QA Agent messaged Backend Agent: hi", minutesAgo: 20),
            a("task.blocked", "qa", "QA Agent is blocked on task “Test the integrated flow”", minutesAgo: 10),
            a("agent.failed", "docs", "Docs Agent failed: link checker crashed", minutesAgo: 90), // before you left
        ], pullRequests: [], agents: agents)
        XCTAssertEqual(d.lines.map(\.agentName), ["QA Agent", "Backend Agent"], "problems first; started/messaged and old items left out")
        XCTAssertEqual(d.lines.first?.text, "Task blocked: “Test the integrated flow”")
        XCTAssertTrue(d.lines.first!.needsYou)
        XCTAssertFalse(d.lines.first!.failed, "blocked is amber, not red")
        XCTAssertEqual(d.lines.last?.text, "Finished “Inspect existing architecture”")
        XCTAssertFalse(d.lines.last!.needsYou)
    }

    func testAFailureIsMarkedFailed() {
        let d = Digest.build(since: now.addingTimeInterval(-3600), activity: [
            a("agent.failed", "docs", "Docs Agent failed: link checker crashed", minutesAgo: 10),
            a("approval.required", "qa", "QA Agent needs approval", minutesAgo: 5),
        ], pullRequests: [], agents: agents)
        XCTAssertEqual(d.lines.first { $0.agentId == "docs" }?.failed, true)
        XCTAssertEqual(d.lines.first { $0.agentId == "qa" }?.failed, false)
    }

    func testPRsAndSeveralFactsPerAgent() {
        let d = Digest.build(since: now.addingTimeInterval(-3600), activity: [
            a("task.completed", "be", "Backend Agent finished task “Endpoints”", minutesAgo: 30),
            a("approval.required", "be", "Backend Agent asked for approval: Bash", minutesAgo: 5),
        ], pullRequests: [PullRequest(id: "p", agentId: "be", slug: "x/y", number: 7, url: "u", title: "Settings endpoints", state: "open",
                                      draft: false, createdAt: now.addingTimeInterval(-600))], agents: agents)
        XCTAssertEqual(d.lines.count, 1)
        XCTAssertEqual(d.lines[0].facts, ["Waiting for your approval", "Opened a PR: Settings endpoints", "Finished “Endpoints”"])
        XCTAssertEqual(d.lines[0].text, "Waiting for your approval · Opened a PR: Settings endpoints · +1 more")
    }

    func testNothingNewMeansNoCard() {
        XCTAssertTrue(Digest.build(since: now, activity: [a("task.completed", "be", "x", minutesAgo: 5)], pullRequests: [], agents: agents).lines.isEmpty)
    }
}
