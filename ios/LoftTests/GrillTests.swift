import XCTest
@testable import Loft

/// Grill mode, contract v2: `meta.grill` on messages, the session state, the server event and commands.
final class GrillDecodingTests: XCTestCase {
    private func message(_ json: String) throws -> ChatMessage {
        try JSONCoding.decoder.decode(ChatMessage.self, from: Data(json.utf8))
    }

    func testQuestionMessage() throws {
        let m = try message(#"""
        {"id": 7, "agent_id": "orch", "role": "assistant", "kind": "text", "created_at": "2026-10-22T10:00:00Z",
         "content": "Should Pro include priority support? See [PR #31](https://github.com/x/y/pull/31).",
         "meta": {"grill": {"session_id": "g1", "role": "question", "question_id": "q3", "index": 3, "total": 8,
                  "topic": "pricing", "kind": "single_choice", "options": ["Yes", "No"], "allow_free_text": true,
                  "speak": "Should Pro include priority support? Yes or no?"}}}
        """#)
        let g = try XCTUnwrap(m.grill)
        XCTAssertEqual(g.role, .question)
        XCTAssertEqual(g.sessionId, "g1")
        XCTAssertEqual(g.questionId, "q3")
        XCTAssertEqual(g.index, 3)
        XCTAssertEqual(g.total, 8)
        XCTAssertEqual(g.kind, .singleChoice)
        XCTAssertEqual(g.options, ["Yes", "No"])
        XCTAssertEqual(g.allowFreeText, true)
        XCTAssertFalse(g.speak!.contains("PR"), "the spoken version leaves PR numbers and links out")
        XCTAssertNil(g.repeat)
    }

    func testUnknownValuesFromANewerServer() throws {
        // An unknown kind or command is dropped, not fatal; an unknown role means "not a grill message".
        let q = try message(#"{"id": 1, "agent_id": "a", "role": "assistant", "kind": "text", "content": "x", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"role": "question", "kind": "slider", "total": null}}}"#)
        XCTAssertEqual(q.grill?.role, .question)
        XCTAssertNil(q.grill?.kind)
        XCTAssertNil(q.grill?.total)
        let other = try message(#"{"id": 2, "agent_id": "a", "role": "assistant", "kind": "text", "content": "x", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"role": "hologram"}}}"#)
        XCTAssertNil(other.grill)
        let plain = try message(#"{"id": 3, "agent_id": "a", "role": "assistant", "kind": "text", "content": "x", "created_at": "2026-10-22T10:00:00Z"}"#)
        XCTAssertNil(plain.grill)
    }

    func testStatusAndSummary() throws {
        let s = try message(#"{"id": 9, "agent_id": "a", "role": "assistant", "kind": "text", "content": "Skipped question 3", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"session_id": "g1", "role": "status", "state": "command", "command": "skip", "tagged_message_id": 8}}}"#)
        XCTAssertEqual(s.grill?.role, .status)
        XCTAssertEqual(s.grill?.command, .skip)
        XCTAssertEqual(s.grill?.taggedMessageId, 8)
        let sum = try message(#"{"id": 10, "agent_id": "a", "role": "assistant", "kind": "text", "content": "So far…", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"session_id": "g1", "role": "summary", "final": false}}}"#)
        XCTAssertEqual(sum.grill?.role, .summary)
        XCTAssertEqual(sum.grill?.final, false)
    }

    func testSessionBareWrappedAndNull() throws {
        let bare = #"{"session_id": "g1", "state": "preparing", "index": 0, "total": null, "topic": "pricing", "started_at": "2026-10-22T10:00:00Z", "current": null}"#
        let s = try XCTUnwrap(LiveLoftAPI.decodeGrillSession(Data(bare.utf8)))
        XCTAssertEqual(s.state, .preparing)
        XCTAssertTrue(s.isOpen)
        XCTAssertNil(s.current)
        let wrapped = #"{"session": {"session_id": "g1", "agent_id": "orch", "state": "active", "index": 3, "total": 8, "current": {"question_id": "q3", "index": 3}, "answered": 2, "end_reason": null}}"#
        let w = try XCTUnwrap(LiveLoftAPI.decodeGrillSession(Data(wrapped.utf8)))
        XCTAssertEqual(w.current, GrillSession.Current(questionId: "q3", index: 3))
        XCTAssertEqual(w.agentId, "orch")
        XCTAssertEqual(w.answered, 2)
        XCTAssertNil(w.endReason)
        XCTAssertNil(try LiveLoftAPI.decodeGrillSession(Data(#"{"session": null}"#.utf8)))
    }

    func testServerEvent() throws {
        let e = try ServerEvent.decode(Data(#"{"type": "grill", "session": {"session_id": "g1", "agent_id": "orch", "state": "paused"}}"#.utf8))
        guard case .grill(let s) = e else { return XCTFail("\(e)") }
        XCTAssertEqual(s.state, .paused)
        XCTAssertFalse(s.state == .ended)
    }

    func testPhrases() throws {
        let p = try JSONCoding.decoder.decode(GrillPhrases.self, from: Data(#"{"version": "3", "max_words": 4, "commands": {"skip": {"en": ["skip", "next"], "pt": ["pula"]}}}"#.utf8))
        XCTAssertEqual(p.maxWords, 4)
        XCTAssertEqual(p.phrases(for: .skip), ["skip", "next", "pula"])
        XCTAssertEqual(p.phrases(for: .end), [])
        let full = try JSONCoding.decoder.decode(GrillPhrases.self, from: Data(#"{"version": "grill-commands-1", "max_words": 4, "fillers": ["um", "please"], "client_only": ["repeat"], "commands": {"repeat": {"en": ["again"], "pt": ["de novo"]}}}"#.utf8))
        XCTAssertEqual(full.fillers, ["um", "please"])
        XCTAssertEqual(full.clientOnly, ["repeat"])
        XCTAssertEqual(full.phrases(forKey: "repeat"), ["again", "de novo"])
    }

    func testAnswerTagCarriesTheQuestion() {
        let q = GrillMeta(sessionId: "g1", role: .question, questionId: "q3", index: 3)
        let tag = GrillMeta.answer(to: q)
        XCTAssertEqual(tag["grill"]?["role"]?.stringValue, "answer")
        XCTAssertEqual(tag["grill"]?["question_id"]?.stringValue, "q3")
        XCTAssertEqual(tag["grill"]?["session_id"]?.stringValue, "g1")
        XCTAssertEqual(GrillMeta.startRequest["grill"]?["role"]?.stringValue, "start_request")
    }
}

@MainActor
final class GrillStoreTests: XCTestCase {
    var cacheURL: URL!

    override func setUp() {
        cacheURL = FileManager.default.temporaryDirectory.appendingPathComponent("grill-\(UUID().uuidString).json")
        UserDefaults.standard.removeObject(forKey: Prefs.orchestratorId)
    }

    override func tearDown() { try? FileManager.default.removeItem(at: cacheURL) }

    private func store(_ api: FixtureAPI) async throws -> LoftStore {
        let store = LoftStore(api: api, cache: SnapshotStore(url: cacheURL))
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.grillSessions.isEmpty && store.supportsGrill {
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTAssertEqual(store.connection, .connected)
        return store
    }

    func testRestoresTheSessionAndFindsTheCurrentQuestion() async throws {
        let store = try await store(FixtureAPI(grill: true))
        XCTAssertTrue(store.supportsGrill)
        XCTAssertEqual(store.openGrill(for: "orch")?.current?.questionId, "q3")
        try await store.loadConversation("orch")
        XCTAssertEqual(store.currentGrillQuestion(for: "orch")?.grill?.questionId, "q3")
        XCTAssertNil(store.openGrill(for: "be"))
        store.deactivate()
    }

    func testAnswerIsTaggedAndCommandsUpdateTheSession() async throws {
        let api = FixtureAPI(grill: true)
        let store = try await store(api)
        try await store.loadConversation("orch")
        let q3 = try XCTUnwrap(store.currentGrillQuestion(for: "orch"))
        await store.answerGrill("Only on yearly", to: q3)
        XCTAssertTrue(store.pending.isEmpty)
        let sent = try XCTUnwrap(store.conversation("orch").last { $0.role == "user" })
        XCTAssertEqual(sent.grill?.role, .answer)
        XCTAssertEqual(sent.grill?.questionId, "q3")
        // After an answer the app catches up with the server, so the next question is already waiting.
        XCTAssertEqual(store.currentGrillQuestion(for: "orch")?.grill?.questionId, "q4")

        try await store.sendGrillCommand(.pause, agentId: "orch")
        XCTAssertEqual(store.openGrill(for: "orch")?.state, .paused)
        try await store.sendGrillCommand(.end, agentId: "orch")
        XCTAssertNil(store.openGrill(for: "orch"), "an ended interview isn't open")
        XCTAssertEqual(api.grillCommandsSent, [.pause, .end])
        store.deactivate()
    }

    func testEventForAKnownSessionUpdatesIt() async throws {
        let store = try await store(FixtureAPI(grill: true))
        store.apply(.grill(GrillSession(sessionId: "g1", agentId: nil, state: .paused)))
        XCTAssertEqual(store.openGrill(for: "orch")?.state, .paused, "matched to the agent by session id")
        store.deactivate()
    }

    func testServerWithoutGrill() async throws {
        let store = try await store(FixtureAPI())
        XCTAssertFalse(store.supportsGrill)
        XCTAssertTrue(store.grillSessions.isEmpty)
        do { try await store.sendGrillCommand(.skip, agentId: "orch"); XCTFail("no session") } catch {}
        store.deactivate()
    }
}

final class GrillChatItemTests: XCTestCase {
    private func q(_ id: Int64, _ qid: String, repeat again: Bool = false) -> ChatMessage {
        var g: [String: JSONValue] = ["role": "question", "session_id": "g1", "question_id": .string(qid)]
        if again { g["repeat"] = true }
        return ChatMessage(id: id, agentId: "orch", runId: nil, role: "assistant", kind: "text", content: "Q \(qid)",
                           meta: ["grill": .object(g)], createdAt: Date())
    }

    func testARepostedQuestionShowsOnlyItsNewestCard() {
        let items = ChatItem.build([q(1, "q2"), q(2, "q3"), q(3, "q2", repeat: true)])
        XCTAssertEqual(items.map(\.id), ["m2", "m3"], "the first q2 card is replaced by the re-posted one")
    }
}
