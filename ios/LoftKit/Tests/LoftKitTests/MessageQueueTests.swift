import XCTest
@testable import LoftKit

/// A typed message's queue mark (#668): queued while the agent holds it, cancelled once the user took it back.
final class MessageQueueTests: XCTestCase {
    private func msg(_ id: Int64 = 7, content: String = "Add a dark mode toggle", meta: JSONValue? = nil) -> ChatMessage {
        ChatMessage(id: id, agentId: "dev", runId: nil, role: "user", kind: "text", content: content, meta: meta, createdAt: .now)
    }

    private func agent(queued: [Int64]?) throws -> Agent {
        let q = queued.map { ",\"queued_messages\":[\($0.map(String.init).joined(separator: ","))]" } ?? ""
        let json = #"{"id":"dev","name":"Dev","status":"running""# + q + "}"
        return try JSONCoding.decoder.decode(Agent.self, from: Data(json.utf8))
    }

    func testDecodesTheQueueFromTheServer() throws {
        XCTAssertEqual(try agent(queued: [7, 9]).queuedMessages, [7, 9])
        XCTAssertNil(try agent(queued: nil).queuedMessages, "an older server")
    }

    func testAQueuedMessageIsMarkedQueuedUntilTheRunTakesIt() throws {
        XCTAssertEqual(QueueMark(msg(), in: try agent(queued: [7])), .queued)
        XCTAssertNil(QueueMark(msg(), in: try agent(queued: [])), "the run took it")
        XCTAssertNil(QueueMark(msg(), in: try agent(queued: nil)), "an older server")
    }

    func testACancelledMessageFromTheWebIsMarkedCancelled() throws {
        let m = try JSONCoding.decoder.decode(ChatMessage.self, from: Data(#"{"id":7,"agent_id":"dev","run_id":null,"role":"user","kind":"text","content":"Skip the iPad layout","meta":{"client_id":"c","cancelled":true},"created_at":"2026-11-24T12:00:00+00:00"}"#.utf8))
        XCTAssertTrue(m.isCancelled)
        XCTAssertEqual(QueueMark(m, in: try agent(queued: [])), .cancelled)
        XCTAssertFalse(msg().isCancelled)
    }

    func testCopyToBoxPutsTheTextAfterAnyDraft() {
        XCTAssertEqual(QueueMark.copiedToBox("", msg()), "Add a dark mode toggle")
        XCTAssertEqual(QueueMark.copiedToBox("  \n", msg()), "Add a dark mode toggle")
        XCTAssertEqual(QueueMark.copiedToBox("Also:", msg()), "Also:\n\nAdd a dark mode toggle")
    }
}
