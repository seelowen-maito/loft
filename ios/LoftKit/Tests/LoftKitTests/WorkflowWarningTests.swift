import XCTest
@testable import LoftKit

/// A PR that changed `.github/workflows/` or `.github/actions/` (#701): the `pr_workflows` item and the PR's `changes_workflows`.
final class WorkflowWarningTests: XCTestCase {
    private func item(_ json: String) throws -> NeedsYouItem {
        try JSONCoding.decoder.decode(NeedsYouItem.self, from: Data(json.utf8))
    }

    func testAPrWorkflowsItemWarnsAndOpensThePR() throws {
        let i = try item(FixtureNeedsYou.prWorkflows)
        XCTAssertTrue(i.changesWorkflows)
        XCTAssertEqual(i.openAction?.target?.url, "https://github.com/octo/loft/pull/214/files")
        XCTAssertFalse(try item(FixtureNeedsYou.backupFailed).changesWorkflows)
    }

    func testAPrWorkflowsItemIsNotFoldedIntoItsProject() throws {
        let warn = try item(FixtureNeedsYou.prWorkflows)
        var ready = warn
        ready.type = "pr_ready"
        XCTAssertEqual(NeedsYouSection.build([ready]).map(\.id), ["project:loft"], "a plain PR item still groups")
        XCTAssertEqual(NeedsYouSection.build([warn]).map(\.id), [warn.id], "the warning stays on its own")
    }

    func testThePRCardWarnsForAnOpenPRThatChangedWorkflows() throws {
        func pr(_ json: String) throws -> PullRequest {
            try JSONCoding.decoder.decode(PullRequest.self, from: Data("""
                {"id": "p", "agent_id": "a", "slug": "o/r", "number": 1, "url": "u", "title": "t", \(json)}
                """.utf8))
        }
        XCTAssertTrue(try pr(#""state": "open", "changes_workflows": true"#).showsWorkflowWarning)
        XCTAssertFalse(try pr(#""state": "merged", "changes_workflows": true"#).showsWorkflowWarning)
        XCTAssertFalse(try pr(#""state": "open", "changes_workflows": false"#).showsWorkflowWarning)
        XCTAssertFalse(try pr(#""state": "open", "changes_workflows": null"#).showsWorkflowWarning, "its files aren't read yet")
        XCTAssertFalse(try pr(#""state": "open""#).showsWorkflowWarning, "an older server sends none")
    }
}
