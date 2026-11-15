import XCTest
@testable import LoftKit

/// The Mac's demo (#484), on the store it shares with the iPhone: the address form opens it with no request, quick
/// search finds the sample agents and issues, and Exit goes back to the address screen with nothing kept. (The store
/// needs an app host for notifications, so this runs with the iPhone's tests; LoftMacUITests drives the Mac's screens.)
@MainActor
final class MacDemoTests: XCTestCase {
    func testTheDemoOpensAndLeavesWithNoRequest() async throws {
        let had = UserDefaults.standard.string(forKey: Prefs.serverURL)
        UserDefaults.standard.removeObject(forKey: Prefs.serverURL)
        defer { if let had { UserDefaults.standard.set(had, forKey: Prefs.serverURL) } }
        let cacheURL = FileManager.default.temporaryDirectory.appendingPathComponent("mac-demo-\(UUID()).json")
        let store = LoftStore(api: nil, cache: SnapshotStore(url: cacheURL))
        XCTAssertEqual(store.connection, .notConfigured, "the address screen")
        let before = LiveLoftAPI.instancesMade

        XCTAssertTrue(LoftStore.isDemoAddress("demo-loft-dev.get-loft.com"))
        XCTAssertTrue(store.connect(address: "demo-loft-dev.get-loft.com"))
        for _ in 0..<100 where store.connection != .connected || store.tasks.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertTrue(store.isDemo)
        let found = QuickSearch.results("a", agents: store.agents, tasks: store.tasks, deliveries: Array(store.deliverySummaries.values),
                                        pullRequests: store.pullRequests)
        XCTAssertTrue(found.contains { $0.kind == .agent }, "quick search finds the sample agents")
        XCTAssertTrue(found.contains { $0.kind != .agent && $0.kind != .page }, "and the sample issues")

        store.exitDemo()
        XCTAssertFalse(store.isDemo)
        XCTAssertEqual(store.connection, .notConfigured, "back on the address screen")
        XCTAssertTrue(store.agents.isEmpty && store.tasks.isEmpty, "nothing from the demo is kept")
        XCTAssertNil(UserDefaults.standard.string(forKey: Prefs.serverURL), "the demo address isn't saved")
        XCTAssertFalse(FileManager.default.fileExists(atPath: cacheURL.path), "nor cached")
        XCTAssertEqual(LiveLoftAPI.instancesMade, before, "no network client, so no request")
    }

    /// A real address typed in Settings during the demo leaves it.
    func testARealAddressLeavesTheDemo() async throws {
        let had = UserDefaults.standard.string(forKey: Prefs.serverURL)
        defer { if let had { UserDefaults.standard.set(had, forKey: Prefs.serverURL) } else { UserDefaults.standard.removeObject(forKey: Prefs.serverURL) } }
        let store = LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("mac-demo-\(UUID()).json")))
        store.enterDemo()
        XCTAssertTrue(store.connect(address: "my-mac.tailnet.ts.net"))
        XCTAssertFalse(store.isDemo)
        XCTAssertTrue(store.api is LiveLoftAPI)
        store.deactivate()
    }
}
