import XCTest
@testable import Loft
@testable import LoftKit

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

    /// #148: a pull request card with its status and checks, and agents both busy and idle.
    func testTheDemoShowsAPullRequestAndBusyAndIdleAgents() async throws {
        let store = LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("demo-\(UUID()).json")))
        store.enterDemo()
        for _ in 0..<100 where store.connection != .connected || store.agents.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        let items = try await store.api!.needsYou().items
        let pr = try XCTUnwrap(items.first { $0.type == "pr_ready" })
        XCTAssertEqual(pr.title, "loft#214: Settings endpoints")
        XCTAssertTrue(pr.reason.contains("checks passed"), "its checks")
        XCTAssertEqual(pr.actions.first?.kind, "merge")
        XCTAssertTrue(store.agents.contains { $0.running == true }, "some agents at work")
        XCTAssertTrue(store.agents.contains { $0.status == "idle" }, "and some idle")
        XCTAssertFalse(store.task("t1")?.description.isEmpty ?? true, "issues open with a description")
        store.deactivate()
    }

    /// App Review types the demo address: it matches on the text alone, whatever its case, spaces, scheme or slash.
    func testTheDemoAddress() {
        for a in ["https://demo-loft-dev.get-loft.com", "demo-loft-dev.get-loft.com", "  HTTPS://Demo-Loft-Dev.Get-Loft.com/ ",
                  "http://demo-loft-dev.get-loft.com//", "DEMO-LOFT-DEV.GET-LOFT.COM"] {
            XCTAssertTrue(LoftStore.isDemoAddress(a), a)
        }
        for a in ["https://demo-loft-dev.get-loft.com/api/x", "demo.get-loft.com", "https://my-mac.tailnet.ts.net", "", "demo-loft-dev.get-loft.com.evil.com"] {
            XCTAssertFalse(LoftStore.isDemoAddress(a), a)
        }
    }

    /// The demo makes no network request: every request goes through a LiveLoftAPI, and typing the demo address,
    /// then using the demo (chat, Needs you, issues), makes none.
    func testTheDemoAddressOpensTheDemoWithNoRequest() async throws {
        let store = LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("demo-\(UUID()).json")))
        let saved = UserDefaults.standard.string(forKey: Prefs.serverURL)
        let before = LiveLoftAPI.instancesMade
        XCTAssertTrue(store.connect(address: " HTTPS://demo-loft-dev.get-loft.com/ "))
        XCTAssertTrue(store.isDemo)
        XCTAssertTrue(store.api is FixtureAPI)
        for _ in 0..<100 where store.connection != .connected || store.tasks.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        let lead = try XCTUnwrap(store.orchestrator)
        await store.send("What's the status?", to: lead.id)
        let items = try await store.api!.needsYou().items
        let item = try XCTUnwrap(items.first { $0.type == "comment_approval" })
        _ = try await store.perform(try XCTUnwrap(item.actions.first { $0.kind == "approve" }))
        _ = try await store.taskComments("t1")
        try await Task.sleep(for: .milliseconds(800))
        XCTAssertEqual(LiveLoftAPI.instancesMade, before, "no network client, so no request")
        XCTAssertEqual(UserDefaults.standard.string(forKey: Prefs.serverURL), saved, "the demo address isn't saved")
        store.deactivate()
        // Any other address is a server, as before.
        let other = LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("o-\(UUID()).json")))
        let had = UserDefaults.standard.string(forKey: Prefs.serverURL)
        XCTAssertTrue(other.connect(address: "my-mac.tailnet.ts.net"))
        XCTAssertFalse(other.isDemo)
        XCTAssertEqual(LiveLoftAPI.instancesMade, before + 1)
        other.deactivate()
        if let had { UserDefaults.standard.set(had, forKey: Prefs.serverURL) } else { UserDefaults.standard.removeObject(forKey: Prefs.serverURL) }
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
