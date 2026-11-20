@testable import LoftKit
import XCTest

/// The Mac chat header's actions against the fake API: each sends what the web's does.
final class AgentHeaderActionsTests: XCTestCase {
    @MainActor
    func testTheHeadersActionsCallTheAPI() async throws {
        let api = FixtureAPI()
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("ah-\(UUID()).json")))
        store.activate()
        defer { store.deactivate() }
        for _ in 0..<100 where store.connection != .connected || store.agents.isEmpty { try await Task.sleep(for: .milliseconds(20)) }

        // Running: Stop, and Restart, which keeps the queue.
        try await store.stopRun("be")
        try await store.restartRun("be")
        // Idle: Wake, then the context size's Compact and Clear.
        try await store.runNow("qa")
        try await store.compact("qa")
        try await store.resetSession("qa")
        XCTAssertEqual(api.agentActionsSent, ["stop:be", "restart:be", "wake:qa", "compact:qa", "clear:qa"])

        // Clear leaves no Session: the header says so, and Resume brings one back.
        XCTAssertEqual(store.agent("qa").map(AgentHeader.context), "no session")
        try await store.resumeSession("qa", sessionId: "s-qa-old")
        XCTAssertEqual(store.agent("qa").map(AgentHeader.context), "context 48k")
    }
}
