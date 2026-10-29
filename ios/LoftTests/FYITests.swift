import XCTest
@testable import Loft

final class FYITests: XCTestCase {
    private func msg(_ role: String, _ meta: JSONValue?) -> ChatMessage {
        ChatMessage(id: 1, agentId: "dev", runId: nil, role: role, kind: "text", content: "Merged #12", meta: meta, createdAt: .now)
    }

    func testAnFYIFromAnotherAgentGetsTheMark() {
        XCTAssertTrue(msg("user", ["from_agent": ["id": "l", "name": "Lead"], "fyi": true]).isFYI)
    }

    func testNeedsActionAndUserMessagesLookAsBefore() {
        XCTAssertFalse(msg("user", ["from_agent": ["id": "l", "name": "Lead"]]).isFYI, "needs action")
        XCTAssertFalse(msg("user", ["fyi": true]).isFYI, "not from an agent")
        XCTAssertFalse(msg("user", nil).isFYI, "the user's own message")
        XCTAssertFalse(msg("assistant", ["from_agent": ["id": "l", "name": "Lead"], "fyi": true]).isFYI)
    }

    func testDecodesFromTheServer() throws {
        let json = #"{"id":7,"agent_id":"dev","run_id":null,"role":"user","kind":"text","content":"Merged #12","meta":{"from_agent":{"id":"l","name":"Lead"},"fyi":true},"created_at":"2026-10-28T13:00:00.5+00:00"}"#
        let m = try JSONCoding.decoder.decode(ChatMessage.self, from: Data(json.utf8))
        XCTAssertTrue(m.isFYI)
    }
}
