import XCTest
@testable import LoftKit

/// An agent's settings as the Mac edits them (spec #382): only what changed is sent, and the list has it at once.
@MainActor
final class AgentDraftTests: XCTestCase {
    private let agent = Agent(id: "a", name: "Dev", project: "loft", model: "opus", effort: nil,
                              permissionMode: "auto", intervalMinutes: 60, status: "idle", prompt: "Build tickets.",
                              freshContext: "above_80k", group: "Loft")

    func testNothingChangedSendsNothing() {
        XCTAssertEqual(AgentDraft(agent).changes(from: AgentDraft(agent)), [:])
    }

    func testOnlyChangedFieldsAreSentByTheirAPINames() {
        let original = AgentDraft(agent)
        var d = original
        d.name = " Web Dev "
        d.permissionMode = "plan"
        d.intervalMinutes = 0
        d.group = ""
        d.role = "qa"
        XCTAssertEqual(d.changes(from: original), [
            "name": .string("Web Dev"), "permission_mode": .string("plan"), "interval_minutes": .number(0), "group": .string(""), "role": .string("qa"),
        ])
    }

    func testSavingUpdatesTheAgentInTheList() async throws {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("ad-\(UUID()).json")))
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.agents.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        let be = try XCTUnwrap(store.agent("be"))
        var d = AgentDraft(be)
        d.name = "API Agent"
        d.model = "sonnet"
        try await store.updateAgent("be", d.changes(from: AgentDraft(be)))
        XCTAssertEqual(store.agent("be")?.name, "API Agent")
        XCTAssertEqual(store.agent("be")?.model, "sonnet")
        store.deactivate()
    }
}
