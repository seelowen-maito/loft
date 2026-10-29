import XCTest
@testable import Loft

/// Demo mode (spec #144) runs on bundled data and leaves the real connection and its cache alone.
@MainActor
final class DemoTests: XCTestCase {
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
