import XCTest
@testable import LoftKit

/// The server drops events for a client that lagged and sends `resync` (spec #593): the app loads everything again.
final class ResyncEventTests: XCTestCase {
    func testDecodesResync() throws {
        guard case .resync = try ServerEvent.decode(Data(#"{"type":"resync"}"#.utf8)) else { return XCTFail("not .resync") }
    }

    @MainActor
    func testResyncReloadsTheAgents() async throws {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        XCTAssertTrue(store.agents.isEmpty)
        store.apply(.resync)
        for _ in 0..<50 where store.agents.isEmpty { try await Task.sleep(nanoseconds: 100_000_000) }
        XCTAssertFalse(store.agents.isEmpty)
    }
}
