import XCTest
@testable import LoftKit

/// Replying to a message (spec #572): the quote a reply carries, the composer's quote and what goes up with the send.
final class MessageReplyTests: XCTestCase {
    private func msg(_ role: String, _ kind: String = "text", content: String = "Which port?", meta: JSONValue? = nil) -> ChatMessage {
        ChatMessage(id: 7, agentId: "dev", runId: nil, role: role, kind: kind, content: content, meta: meta, createdAt: .now)
    }

    func testDecodesTheQuoteFromTheServer() throws {
        let json = #"{"id":9,"agent_id":"dev","run_id":null,"role":"user","kind":"text","content":"7420, please","meta":{"client_id":"c","reply_to":{"id":7,"role":"assistant","author":"Dev","created_at":"2026-11-16T12:00:00+00:00","excerpt":"Which port?"}},"created_at":"2026-11-16T12:01:00+00:00"}"#
        let m = try JSONCoding.decoder.decode(ChatMessage.self, from: Data(json.utf8))
        XCTAssertEqual(m.replyQuote, MessageQuote(id: 7, author: "Dev", excerpt: "Which port?"))
        XCTAssertEqual(m.replyQuote?.authorLabel, "Dev")
    }

    func testAQuoteOfTheUsersOwnMessageIsYou() {
        let q = msg("user", meta: ["reply_to": ["id": .number(3), "author": nil, "excerpt": "Ship it"]]).replyQuote
        XCTAssertEqual(q, MessageQuote(id: 3, author: nil, excerpt: "Ship it"))
        XCTAssertEqual(q?.authorLabel, "You")
        XCTAssertNil(msg("user").replyQuote, "not a reply")
    }

    func testOnlyChatMessagesCanBeQuoted() {
        XCTAssertTrue(msg("user").isQuotable)
        XCTAssertTrue(msg("assistant").isQuotable)
        for (role, kind) in [("system", "run_start"), ("assistant", "thinking"), ("assistant", "tool_use"), ("tool", "tool_result"), ("system", "notice")] {
            XCTAssertFalse(msg(role, kind).isQuotable, kind)
        }
    }

    func testTheComposersQuoteNamesTheAuthorAndCutsTheText() {
        XCTAssertEqual(MessageQuote(of: msg("assistant", content: "Which port\n  should it use?"), agentName: "Dev"),
                       MessageQuote(id: 7, author: "Dev", excerpt: "Which port should it use?"))
        XCTAssertNil(MessageQuote(of: msg("user"), agentName: "Dev").author, "the user's own")
        XCTAssertEqual(MessageQuote(of: msg("user", meta: ["from_agent": ["id": "q", "name": "QA"]]), agentName: "Dev").author, "QA")
        let long = MessageQuote(of: msg("assistant", content: String(repeating: "é", count: 300)), agentName: "Dev").excerpt
        XCTAssertEqual(long, String(repeating: "é", count: 200) + "…", "200 and an ellipsis, as the server")
    }

    func testTheSendCarriesTheQuoteNextToAnAttachedIssue() {
        let q = MessageQuote(id: 7, author: "Dev", excerpt: "Which port?")
        XCTAssertEqual(MessageQuote.meta(nil, replyingTo: q), ["reply_to": .number(7)])
        XCTAssertEqual(MessageQuote.meta(["task_id": "t1"], replyingTo: q), ["task_id": "t1", "reply_to": .number(7)])
        XCTAssertEqual(MessageQuote.meta(["task_id": "t1"], replyingTo: nil), ["task_id": "t1"], "the quote was removed")
        XCTAssertNil(MessageQuote.meta(nil, replyingTo: nil))
    }

    func testTheFixtureStoresAReplyAndRefusesAMissingOne() async throws {
        let api = FixtureAPI()
        let r = try await api.sendMessage(agentId: "orch", text: "Wait for QA", clientId: "c1", meta: ["reply_to": .number(2)])
        XCTAssertEqual(r.message?.replyQuote, MessageQuote(id: 2, author: "Orchestrator", excerpt: "On it. I'll split this into backend, iOS and QA sub-issues."))
        do {
            _ = try await api.sendMessage(agentId: "orch", text: "x", clientId: "c2", meta: ["reply_to": .number(999_999)])
            XCTFail("a missing message is a 400")
        } catch {}
    }
}
