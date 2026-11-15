import XCTest
@testable import Loft
@testable import LoftKit

/// The Overview tab's badge (#543) counts what's new in Needs you, like the app icon.
final class OverviewBadgeTests: XCTestCase {
    private func cache() -> SnapshotStore {
        SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("badge-\(UUID()).json"))
    }

    @MainActor
    func testTheBadgeIsWhatsNewInNeedsYouAndSeeingClearsIt() async throws {
        let store = LoftStore(api: FixtureAPI(), cache: cache())
        store.activate()
        defer { store.deactivate() }
        for _ in 0..<100 where store.connection != .connected || store.needsYou == nil { try await Task.sleep(for: .milliseconds(50)) }
        let list = try XCTUnwrap(store.needsYou)
        XCTAssertGreaterThan(list.unseen, 0, "the fixture has new items")
        XCTAssertEqual(store.attentionBadge, list.unseen)
        await store.markNeedsYouSeen(NeedsYouSeen.unseen(list.items))
        XCTAssertEqual(store.needsYou?.unseen, 0)
        XCTAssertEqual(store.attentionBadge, 0, "nothing new, no badge")
    }

    @MainActor
    func testWithoutNeedsYouItCountsFailedAndBlockedIssues() async throws {
        let api = MockAPI()
        let now = Date()
        func t(_ id: String, _ status: String) -> HBTask {
            HBTask(id: id, title: id, description: "", kind: "work", status: status, priority: "normal", project: "p", agentId: nil,
                   parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "", createdAt: now,
                   updatedAt: now, startedAt: nil, completedAt: nil)
        }
        api.taskList = [t("f", "failed"), t("b", "blocked"), t("d", "done"), t("i", "in_progress")]
        let store = LoftStore(api: api, cache: cache())
        store.activate()
        defer { store.deactivate() }
        for _ in 0..<100 where store.connection != .connected || store.tasks.count < 4 { try await Task.sleep(for: .milliseconds(50)) }
        XCTAssertFalse(store.supportsNeedsYou)
        XCTAssertEqual(store.attentionBadge, 2)
    }
}
