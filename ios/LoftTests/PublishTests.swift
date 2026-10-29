import XCTest
@testable import Loft

/// Publish to GitHub on a Loft-only issue (spec #65): it comes back mirrored, and the list has the link.
@MainActor
final class PublishTests: XCTestCase {
    private func store() async throws -> LoftStore {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("pub-\(UUID()).json")))
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.tasks.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        return store
    }

    func testPublishingGivesTheIssueItsGitHubLink() async throws {
        let store = try await store()
        // t3 is Loft-only, in the loft project.
        XCTAssertNil(store.task("t3")?.github)
        let t = try await store.publishTask("t3")
        XCTAssertEqual(t.github?.shortName, "loft#50")
        XCTAssertEqual(store.task("t3")?.github?.repo, "octo/loft", "the list has it at once")
        store.deactivate()
    }

    func testAnIssueWithoutAProjectSaysWhy() async throws {
        let store = try await store()
        let personal = try await store.createTask(["title": "Buy milk", "kind": "todo"])
        do {
            try await store.publishTask(personal.id)
            XCTFail("no project, no repo")
        } catch {
            guard case .server(_, let message) = APIError.from(error) else { return XCTFail("\(error)") }
            XCTAssertTrue(message.contains("no project"))
        }
        store.deactivate()
    }
}
