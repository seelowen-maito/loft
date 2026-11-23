import XCTest
@testable import Loft
@testable import LoftKit

/// Rules the UI tests used to walk through on screen, checked on the store and the domain instead (#554).
@MainActor
final class FixtureFlowTests: XCTestCase {
    private func connected(_ api: LoftAPI) async throws -> LoftStore {
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("flow-\(UUID()).json")))
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.agents.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertEqual(store.connection, .connected)
        return store
    }

    /// The Issues list's Failed and Inbox filters (was testOverviewTasksAndAgents).
    func testFailedAndInboxFiltersNarrowTheList() {
        XCTAssertTrue(TaskFilter.failed.matches(.failed))
        XCTAssertFalse(TaskFilter.failed.matches(.inbox) || TaskFilter.failed.matches(.blocked) || TaskFilter.failed.matches(.done))
        XCTAssertTrue(TaskFilter.inbox.matches(.inbox))
        XCTAssertFalse(TaskFilter.inbox.matches(.failed) || TaskFilter.inbox.matches(.ready) || TaskFilter.inbox.matches(.inProgress))
    }

    /// A new issue is in the list at once, in the inbox (was testOverviewTasksAndAgents).
    func testANewIssueIsInTheListInTheInbox() async throws {
        let store = try await connected(FixtureAPI())
        let t = try await store.createTask(["title": .string("Write release notes")])
        let listed = try XCTUnwrap(store.tasks.first { $0.id == t.id })
        XCTAssertEqual(listed.title, "Write release notes")
        XCTAssertTrue(TaskFilter.inbox.matches(TaskState(raw: listed.status)))
        store.deactivate()
    }

    /// Allowing or denying a permission takes it off what waits for you (was testOverviewTasksAndAgents).
    func testAnsweringAPermissionTakesItOffTheList() async throws {
        let store = try await connected(FixtureAPI())
        let p = try XCTUnwrap(store.pendingApprovals().first { $0.id == "p1" })
        try await store.decide(p, decision: "allow_once")
        XCTAssertFalse(store.pendingApprovals().contains { $0.id == "p1" })
        XCTAssertEqual(store.permissions.first { $0.id == "p1" }?.status, "allowed_once")
        store.deactivate()
    }

    /// Resume after Pause opens the interview again (was testGrillRoundCardsAnswersAndSend).
    func testAPausedInterviewResumes() async throws {
        let api = FixtureAPI(grill: true)
        let store = try await connected(api)
        for _ in 0..<100 where store.grillSessions.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        try await store.sendGrillCommand(.pause, agentId: "orch")
        XCTAssertEqual(store.openGrill(for: "orch")?.state, .paused)
        try await store.sendGrillCommand(.resume, agentId: "orch")
        XCTAssertEqual(store.openGrill(for: "orch")?.state, .active)
        XCTAssertEqual(api.grillCommandsSent, [.pause, .resume])
        store.deactivate()
    }

    /// Restart on the run-watch bar: the store fetches the agent again, so the bar goes without a live event
    /// (was testRunWatchRestart).
    func testRestartingAWaitingRunClearsTheBar() async throws {
        let api = MockAPI()
        api.agentList[0].running = true
        api.agentList[0].waitingOn = ["Run the QA suite"]
        api.agentList[0].waitingSince = Date().addingTimeInterval(-14 * 60)
        let store = try await connected(api)
        let id = api.agentList[0].id
        XCTAssertNotNil(RunWatch.of(try XCTUnwrap(store.agent(id)), newestMessage: nil, now: Date()), "the bar shows")
        try await store.restartRun(id)
        XCTAssertNil(RunWatch.of(try XCTUnwrap(store.agent(id)), newestMessage: nil, now: Date()), "and goes")
        store.deactivate()
    }

    /// The demo starts fresh each time: nothing sent before Exit is there when it opens again
    /// (was testARelaunchResetsTheDemoAndExitWorksFromFullScreen).
    func testTheDemoStartsFreshEachTime() async throws {
        let store = LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("demo-\(UUID()).json")))
        store.enterDemo()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        let lead = try XCTUnwrap(store.orchestrator)
        try await store.loadConversation(lead.id)
        await store.send("Kept only until relaunch", to: lead.id)
        XCTAssertTrue(store.conversation(lead.id).contains { $0.content == "Kept only until relaunch" })
        store.exitDemo()
        store.enterDemo()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        try await store.loadConversation(lead.id)
        XCTAssertFalse(store.conversation(lead.id).contains { $0.content == "Kept only until relaunch" })
        store.deactivate()
    }

    /// Specs' group filter: a spec is in a group when one of its agents works on the spec's project or has it
    /// (was testSpecsListSectionsAndFilters).
    func testASpecIsInAGroupThroughItsAgents() {
        var dev = Agent(id: "d", name: "Dev", status: "idle")
        dev.group = "Loft"
        dev.project = "loft"
        func spec(_ project: String, agent: String? = nil) -> HBTask {
            HBTask(id: "s", title: "S", description: "", kind: "work", status: "inbox", priority: "normal", project: project, agentId: agent,
                   parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "",
                   createdAt: .now, updatedAt: .now, startedAt: nil, completedAt: nil)
        }
        XCTAssertTrue(SpecViews.inGroup(spec("loft"), "Loft", agents: [dev]))
        XCTAssertFalse(SpecViews.inGroup(spec("garden"), "Loft", agents: [dev]), "no agent of the group works in garden")
        XCTAssertTrue(SpecViews.inGroup(spec("garden", agent: "d"), "Loft", agents: [dev]), "unless one has it")
        XCTAssertFalse(SpecViews.inGroup(spec("loft"), "Web", agents: [dev]))
    }

    /// A risky request's warnings read as on the web (was testAPermissionShowsItsRisksAndExplainsOnRequest).
    func testRiskLabels() {
        XCTAssertEqual(PermissionRisk.pushesToMain.label, "Pushes to main")
        XCTAssertEqual(PermissionRisk.network.label, "Uses the network")
    }

    /// Overview's commit counts: the last 30 days, and the 30 before (was testOverviewActivityAndRepoDetail).
    func testRepoCommitCounts() {
        let r = RepoSummary(name: "loft", path: "", branch: "main", ahead: 0, behind: 0, staged: 0, modified: 0, untracked: 0, totalCommits: 0,
                            daily: Array(repeating: 1, count: 100) + Array(repeating: 2, count: 30))
        XCTAssertEqual(r.commits30, 60)
        XCTAssertEqual(r.commitsPrior30, 30)
    }
}
