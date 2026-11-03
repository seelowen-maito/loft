import XCTest
@testable import Loft

/// Demo mode (spec #144) runs on bundled data and leaves the real connection and its cache alone.
@MainActor
final class DemoTests: XCTestCase {
    /// #146: the Lead answers from a script, picked by what the message is about; no AI call.
    func testTheDemoLeadAnswers() async throws {
        let store = LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("demo-\(UUID()).json")))
        store.enterDemo()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        let lead = try XCTUnwrap(store.orchestrator)
        try await store.loadConversation(lead.id)
        await store.send("What's the status?", to: lead.id)
        // The chat picks the answer up without a live event, as it does on a real server.
        var reply: String?
        for _ in 0..<50 where reply == nil {
            try await Task.sleep(for: .milliseconds(200))
            try await store.loadConversation(lead.id)
            reply = store.conversation(lead.id).first { $0.role == "assistant" && $0.content.contains("3 of 4 endpoints") }?.content
        }
        XCTAssertNotNil(reply, "the scripted status answer")
        XCTAssertTrue(FixtureAPI.demoReply(to: "Build a pricing page", from: "Lead").contains("demo"), "anything else gets the default")
        XCTAssertTrue(store.agents.count >= 3, "a few agents in the list")
        store.deactivate()
    }

    /// #147: the waiting comment's Approve, Edit and approve, and Keep in Loft each show in its issue's thread.
    func testNeedsYouApprovalsShowInTheThread() async throws {
        for (label, text, expected) in [("Approve", nil, "posted"), ("Edit and approve", "Edited in the demo", "posted"), ("Keep in Loft", nil, "kept")] {
            let store = LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("demo-\(UUID()).json")))
            store.enterDemo()
            for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
            let list = try await store.api!.needsYou()
            let item = try XCTUnwrap(list.items.first { $0.type == "comment_approval" })
            let action = try XCTUnwrap(item.actions.first { $0.label == label })
            _ = try await store.perform(action, text: text)
            let thread = try await store.taskComments(item.taskId!)
            let c = try XCTUnwrap(thread.first { $0.id == 77 })
            XCTAssertEqual(c.githubState, expected, label)
            if let text { XCTAssertEqual(c.text, text) }
            store.deactivate()
        }
    }

    func testTheDemoNeverTouchesTheRealCacheOrServer() async throws {
        let cacheURL = FileManager.default.temporaryDirectory.appendingPathComponent("demo-\(UUID()).json")
        let store = LoftStore(api: nil, cache: SnapshotStore(url: cacheURL))
        let saved = UserDefaults.standard.string(forKey: Prefs.serverURL)
        store.enterDemo()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertTrue(store.isDemo)
        XCTAssertFalse(store.tasks.isEmpty, "sample issues, with no server")
        store.saveNow()
        XCTAssertFalse(FileManager.default.fileExists(atPath: cacheURL.path), "the demo isn't cached over the real server's data")
        store.exitDemo()
        XCTAssertFalse(store.isDemo)
        XCTAssertEqual(UserDefaults.standard.string(forKey: Prefs.serverURL), saved, "the saved server is untouched")
        XCTAssertTrue(store.tasks.isEmpty, "nothing from the demo is left")
        store.deactivate()
    }
}
