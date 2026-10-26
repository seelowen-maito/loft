import XCTest
@testable import Loft

/// "Make it a brief" with Claude on the Mac (#118): the brief, or a note and the words kept.
final class MacBriefTests: XCTestCase {
    @MainActor
    private func connected(_ api: LoftAPI) async throws -> LoftStore {
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        store.activate()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(50)) }
        return store
    }

    @MainActor
    func testTheThreeOutcomes() async throws {
        let long = Array(repeating: "word", count: 90).joined(separator: " ")
        let ok = try await connected(FixtureAPI())
        defer { ok.deactivate() }
        XCTAssertTrue(ok.supportsBrief)
        let b = await ok.brief(long)
        XCTAssertNotNil(b.brief)

        let short = await ok.brief("fix the login screen please")
        XCTAssertNil(short.brief)
        XCTAssertEqual(short.note, "Already short")
    }

    @MainActor
    func testAServerWithoutBriefsOffersNothingAndKeepsTheWords() async {
        let store = LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        XCTAssertFalse(store.supportsBrief)
        let r = await store.brief("anything")
        XCTAssertNil(r.brief)
        XCTAssertEqual(r.note, "Couldn't make a brief, kept your words")
    }
}
