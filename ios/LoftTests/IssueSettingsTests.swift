import XCTest
@testable import Loft

/// Settings › GitHub issues (#177): two global keys in `/api/settings`, saved without touching the rest.
@MainActor
final class IssueSettingsTests: XCTestCase {
    func testReadsTheGlobalKeysAndAsksForAnythingUnknown() {
        XCTAssertEqual(IssueSettings(["comment_approval_default": "own", "auto_publish_all": true]), IssueSettings(commentApproval: "own", autoPublish: true))
        XCTAssertEqual(IssueSettings(["comment_approval_default": "weird"])?.commentApproval, "ask")
        XCTAssertEqual(IssueSettings(["comment_approval_default": "ask", "auto_publish_all": nil])?.autoPublish, false, "not migrated yet: off")
        XCTAssertNil(IssueSettings(["auto_update": true]), "an older server has none")
    }

    func testSavingKeepsTheOtherSettingsAndTheGitHubToken() async throws {
        let api = FixtureAPI()
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("is-\(UUID()).json")))
        store.activate()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        try await store.saveIssueSettings(IssueSettings(commentApproval: "all", autoPublish: true))
        let sent = try XCTUnwrap(api.settingsPut.last)
        XCTAssertNil(sent["github_token"], "the empty token the server sends would clear the saved one")
        XCTAssertEqual(sent["auto_update"], true, "PUT replaces everything: the rest goes back as it came")
        let now = try await store.issueSettings()
        XCTAssertEqual(now, IssueSettings(commentApproval: "all", autoPublish: true))
        store.deactivate()
    }

    func testACommentCanStayOffGitHub() async throws {
        let api = FixtureAPI()
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("is-\(UUID()).json")))
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.tasks.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        try await store.addTaskComment("t1", text: "Just for us", github: false)
        let thread = try await store.taskComments("t1")
        XCTAssertNil(thread.last?.githubState, "Post on GitHub off: it stays in Loft")
        store.deactivate()
    }
}
