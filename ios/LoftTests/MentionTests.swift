import XCTest
@testable import Loft
@testable import LoftKit

final class MentionTests: XCTestCase {
    private func msg(_ role: String, _ meta: JSONValue) -> ChatMessage {
        ChatMessage(id: 1, agentId: "orch", runId: nil, role: role, kind: "text", content: "x", meta: meta, createdAt: .now)
    }

    func testMentionsOnYourMessage() {
        let m = msg("user", ["mentions": .array([["id": "qa", "name": "QA Agent"], ["id": "be", "name": "Backend Agent"]]), "mentions_only": false])
        XCTAssertEqual(m.mentionedNames, ["QA Agent", "Backend Agent"])
        XCTAssertFalse(m.mentionsOnly, "the chat's agent got it too: Also sent to")
        XCTAssertTrue(msg("user", ["mentions": .array([["id": "qa", "name": "QA Agent"]]), "mentions_only": true]).mentionsOnly)
        XCTAssertEqual(msg("user", ["client_id": "c"]).mentionedNames, [])
    }

    func testAMentionReplyShowsTheOtherAgent() {
        let r = msg("assistant", ["from_agent": ["id": "qa", "name": "QA Agent"], "mention_reply": true])
        XCTAssertTrue(r.isMentionReply)
        XCTAssertEqual(r.fromAgentName, "QA Agent")
        XCTAssertFalse(msg("assistant", [:]).isMentionReply)
    }

    func testTagsOnYourMessage() {
        let m = msg("user", ["tags": .array([
            ["tag": "#42", "kind": "spec", "title": "Settings", "state": "open", "url": "https://github.com/o/loft/issues/42", "task_id": "t1"],
            ["tag": "o/loft#7", "kind": "pr", "title": "Fix", "state": "open", "url": "https://github.com/o/loft/pull/7"],
        ])])
        XCTAssertEqual(m.tags.map(\.tag), ["#42", "o/loft#7"])
        XCTAssertEqual(m.tags[0].taskId, "t1", "a Loft issue opens in the app")
        XCTAssertNil(m.tags[1].taskId, "a pull request opens on GitHub")
        XCTAssertEqual(msg("user", [:]).tags, [])
    }
}
