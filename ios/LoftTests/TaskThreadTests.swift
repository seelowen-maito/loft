import XCTest
@testable import Loft

/// A Loft issue's thread (spec #65): where each comment stands with GitHub, and Approve / Keep in Loft on it.
@MainActor
final class TaskThreadTests: XCTestCase {
    func testTheBackendsCommentDecodes() throws {
        let json = #"{"id":77,"task_id":"t1","author":"be","kind":"comment","text":"Done","meta":{"author_name":"Backend Agent","github":{"state":"pending"}},"created_at":"2026-10-29T10:00:00Z"}"#
        let c = try JSONCoding.decoder.decode(TaskComment.self, from: Data(json.utf8))
        XCTAssertEqual(c.githubState, "pending")
        XCTAssertEqual(c.authorName, "Backend Agent")
        let plain = try JSONCoding.decoder.decode(TaskComment.self, from: Data(#"{"id":1,"task_id":"t","author":"user","kind":"note","text":"x","meta":{},"created_at":"2026-10-29T10:00:00Z"}"#.utf8))
        XCTAssertNil(plain.githubState, "not on GitHub: no mark")
    }

    private func store() async throws -> LoftStore {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("th-\(UUID()).json")))
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.tasks.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        return store
    }

    func testApproveAndKeepInLoft() async throws {
        let store = try await store()
        var thread = try await store.taskComments("t1")
        let pending = try XCTUnwrap(thread.first { $0.githubState == "pending" })
        try await store.approveComment(pending, text: "Edited")
        thread = try await store.taskComments("t1")
        XCTAssertEqual(thread.first { $0.id == pending.id }?.githubState, "posted")
        XCTAssertEqual(thread.first { $0.id == pending.id }?.text, "Edited")

        try await store.addTaskComment("t1", text: "Internal only")
        let afterAdd = try await store.taskComments("t1")
        let mine = try XCTUnwrap(afterAdd.last)
        XCTAssertEqual(mine.githubState, "pending", "on a mirrored issue a comment waits for approval")
        try await store.keepComment(mine)
        let afterKeep = try await store.taskComments("t1")
        XCTAssertEqual(afterKeep.last?.githubState, "kept")
        store.deactivate()
    }
}
