import XCTest
@testable import Loft

/// Grill mode in rounds: `meta.grill` on messages, the session state with its answers, the server event and commands.
final class GrillDecodingTests: XCTestCase {
    private func message(_ json: String) throws -> ChatMessage {
        try JSONCoding.decoder.decode(ChatMessage.self, from: Data(json.utf8))
    }

    func testRoundMessage() throws {
        let m = try message(#"""
        {"id": 7, "agent_id": "lead", "role": "assistant", "kind": "text", "created_at": "2026-10-22T10:00:00Z",
         "content": "**Round 2**\n\n1. Which billing periods?\n   _Recommendation:_ Both",
         "meta": {"grill": {"session_id": "g1", "role": "round", "round": 2, "topic": "pricing", "questions": [
            {"id": "q3", "text": "Which billing periods?", "speak": "Which billing periods?", "kind": "single_choice",
             "options": ["Monthly", "Yearly", "Both"], "recommendation": "Both", "from_round": 1},
            {"id": "q4", "text": "Priority support? See [PR #31](https://github.com/x/y/pull/31).", "speak": "Priority support?",
             "kind": "free_text", "options": [], "recommendation": "Only on yearly", "from_round": null}]}}}
        """#)
        let g = try XCTUnwrap(m.grill)
        XCTAssertEqual(g.role, .round)
        XCTAssertEqual(g.round, 2)
        XCTAssertEqual(g.topic, "pricing")
        let qs = try XCTUnwrap(g.questions)
        XCTAssertEqual(qs.map(\.id), ["q3", "q4"])
        XCTAssertEqual(qs[0].kind, .singleChoice)
        XCTAssertEqual(qs[0].options, ["Monthly", "Yearly", "Both"])
        XCTAssertEqual(qs[0].recommendation, "Both")
        XCTAssertEqual(qs[0].fromRound, 1)
        XCTAssertNil(qs[1].fromRound)
        XCTAssertEqual(qs[0].state, .open, "a posted round has no answers yet")
    }

    func testUnknownValuesFromANewerServer() throws {
        // An unknown kind or state is dropped, not fatal; an unknown role means "not a grill message".
        let r = try message(#"{"id": 1, "agent_id": "a", "role": "assistant", "kind": "text", "content": "x", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"role": "round", "questions": [{"id": "q", "text": "?", "kind": "slider", "state": "pondering", "recommendation": "No"}]}}}"#)
        XCTAssertEqual(r.grill?.role, .round)
        XCTAssertNil(r.grill?.questions?.first?.kind)
        XCTAssertEqual(r.grill?.questions?.first?.state, .open)
        let other = try message(#"{"id": 2, "agent_id": "a", "role": "assistant", "kind": "text", "content": "x", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"role": "hologram"}}}"#)
        XCTAssertNil(other.grill)
        // A question message from before rounds is plain chat now.
        let old = try message(#"{"id": 3, "agent_id": "a", "role": "assistant", "kind": "text", "content": "x", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"role": "question", "question_id": "q1"}}}"#)
        XCTAssertNil(old.grill)
        let plain = try message(#"{"id": 4, "agent_id": "a", "role": "assistant", "kind": "text", "content": "x", "created_at": "2026-10-22T10:00:00Z"}"#)
        XCTAssertNil(plain.grill)
    }

    func testStatusAnswersAndSummary() throws {
        let s = try message(#"{"id": 9, "agent_id": "a", "role": "system", "kind": "notice", "content": "Question 2 left to the agent: No", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"session_id": "g1", "role": "status", "state": "decided", "command": "decide", "round": 1, "question_id": "q2"}}}"#)
        XCTAssertEqual(s.grill?.role, .status)
        XCTAssertEqual(s.grill?.command, .decide)
        XCTAssertEqual(s.grill?.questionId, "q2")
        let sent = try message(#"{"id": 10, "agent_id": "a", "role": "user", "kind": "text", "content": "Answers to round 1:\n\n1. Yes (No)", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"session_id": "g1", "role": "answers", "round": 1}}}"#)
        XCTAssertEqual(sent.grill?.role, .answers)
        XCTAssertEqual(sent.grill?.round, 1)
        let sum = try message(#"{"id": 11, "agent_id": "a", "role": "assistant", "kind": "text", "content": "Decided…", "created_at": "2026-10-22T10:00:00Z", "meta": {"grill": {"session_id": "g1", "role": "summary", "final": true, "notes_path": "notes/grill-pricing.md"}}}"#)
        XCTAssertEqual(sum.grill?.role, .summary)
        XCTAssertEqual(sum.grill?.final, true)
        XCTAssertEqual(sum.grill?.notesPath, "notes/grill-pricing.md")
    }

    func testSessionWithAnswerStates() throws {
        let json = #"""
        {"session": {"session_id": "g1", "agent_id": "lead", "state": "active", "end_reason": null, "topic": "pricing",
         "round": 2, "total": 5, "answered": 4, "waiting": false, "started_at": "2026-10-22T10:00:00Z", "notes_path": null,
         "questions": [
           {"id": "a", "text": "A?", "speak": "A?", "kind": "free_text", "options": [], "recommendation": "Ra", "state": "answered", "answer": "Ra", "accepted": true, "answered_at": "2026-10-22T10:01:00Z", "from_round": null},
           {"id": "b", "text": "B?", "speak": "B?", "kind": "free_text", "options": [], "recommendation": "Rb", "state": "answered", "answer": "My own", "accepted": false, "from_round": null},
           {"id": "c", "text": "C?", "speak": "C?", "kind": "free_text", "options": [], "recommendation": "Rc", "state": "discuss", "answer": "Why?", "accepted": false, "from_round": 1},
           {"id": "d", "text": "D?", "speak": "D?", "kind": "free_text", "options": [], "recommendation": "Rd", "state": "decided", "answer": "Rd", "accepted": false, "from_round": null},
           {"id": "e", "text": "E?", "speak": "E?", "kind": "free_text", "options": [], "recommendation": "Re", "state": "open", "answer": null, "accepted": false, "from_round": null}]}}
        """#
        let s = try XCTUnwrap(LiveLoftAPI.decodeGrillSession(Data(json.utf8)))
        XCTAssertEqual(s.round, 2)
        XCTAssertEqual(s.answered, 4)
        XCTAssertEqual(s.questions?.map(\.state), [.answered, .answered, .discuss, .decided, .open])
        XCTAssertEqual(s.questions?.map(\.accepted), [true, false, false, false, false])
        XCTAssertEqual(s.questions?[2].answer, "Why?")
        XCTAssertTrue(s.hasOpenRound)
        XCTAssertEqual(s.number(of: "c"), 3)

        // Sent and waiting for the next round, preparing, or ended: no round to answer.
        let waiting = #"{"session_id": "g1", "state": "active", "round": 2, "waiting": true, "questions": [{"id": "a", "text": "A?", "recommendation": "R", "state": "carried"}]}"#
        let w = try XCTUnwrap(LiveLoftAPI.decodeGrillSession(Data(waiting.utf8)))
        XCTAssertEqual(w.questions?.first?.state, .carried)
        XCTAssertFalse(w.hasOpenRound)
        let preparing = try XCTUnwrap(LiveLoftAPI.decodeGrillSession(Data(#"{"session_id": "g1", "state": "preparing", "round": null, "questions": null, "waiting": false}"#.utf8)))
        XCTAssertTrue(preparing.isOpen)
        XCTAssertFalse(preparing.hasOpenRound)
        XCTAssertNil(try LiveLoftAPI.decodeGrillSession(Data(#"{"session": null}"#.utf8)))
    }

    func testServerEvent() throws {
        let e = try ServerEvent.decode(Data(#"{"type": "grill", "session": {"session_id": "g1", "agent_id": "lead", "state": "paused", "round": 1, "waiting": false}}"#.utf8))
        guard case .grill(let s) = e else { return XCTFail("\(e)") }
        XCTAssertEqual(s.state, .paused)
        XCTAssertEqual(s.round, 1)
    }

    func testPhrases() throws {
        let p = try JSONCoding.decoder.decode(GrillPhrases.self, from: Data(#"{"version": "3", "max_words": 6, "commands": {"skip": {"en": ["skip", "next"], "pt": ["pula"]}, "send": {"en": ["send"], "pt": ["envia"]}}}"#.utf8))
        XCTAssertEqual(p.maxWords, 6)
        XCTAssertEqual(p.phrases(for: .skip), ["skip", "next", "pula"])
        XCTAssertEqual(p.phrases(for: .send), ["send", "envia"])
        XCTAssertEqual(p.phrases(for: .end), [])
        let full = try JSONCoding.decoder.decode(GrillPhrases.self, from: Data(#"{"version": "3", "max_words": 6, "fillers": ["um", "please"], "client_only": ["skip", "back", "repeat"], "commands": {"repeat": {"en": ["again"], "pt": ["de novo"]}}}"#.utf8))
        XCTAssertEqual(full.fillers, ["um", "please"])
        XCTAssertEqual(full.clientOnly, ["skip", "back", "repeat"])
        XCTAssertEqual(full.phrases(forKey: "repeat"), ["again", "de novo"])
    }

    func testAnswerTagCarriesTheQuestion() {
        let tag = GrillMeta.answer(sessionId: "g1", questionId: "q3")
        XCTAssertEqual(tag["grill"]?["role"]?.stringValue, "answer")
        XCTAssertEqual(tag["grill"]?["question_id"]?.stringValue, "q3")
        XCTAssertEqual(tag["grill"]?["session_id"]?.stringValue, "g1")
        XCTAssertNil(tag["grill"]?["discuss"])
        XCTAssertEqual(GrillMeta.answer(sessionId: "g1", questionId: "q3", discuss: true)["grill"]?["discuss"], .bool(true))
        XCTAssertEqual(GrillMeta.startRequest["grill"]?["role"]?.stringValue, "start_request")
    }

    func testAQuestionIsReadWithItsRecommendation() {
        let q = GrillQuestion(id: "q", text: "Should Pro include priority support? See [PR #31](https://x/y).",
                              speak: "Should Pro include priority support?", recommendation: "Only on yearly")
        XCTAssertEqual(q.spoken(number: 2, of: 3), "Question 2 of 3. Should Pro include priority support? Recommended: Only on yearly.")
        XCTAssertEqual(q.spoken(number: 1, of: 1), "Should Pro include priority support? Recommended: Only on yearly.", "no count for a one-question round")
        let bare = GrillQuestion(id: "q", text: "Who pays?", recommendation: "")
        XCTAssertEqual(bare.spoken(number: 1, of: 2), "Question 1 of 2. Who pays?")
    }
}

/// The voice walks a round one question at a time and sends it after the last.
final class GrillWalkTests: XCTestCase {
    private func qs(_ states: [GrillQuestion.State]) -> [GrillQuestion] {
        states.enumerated().map { GrillQuestion(id: "q\($0.offset + 1)", text: "?", recommendation: "R", state: $0.element) }
    }

    func testStartsAtTheFirstOpenQuestion() {
        XCTAssertEqual(GrillWalk.start(qs([.open, .open])), "q1")
        XCTAssertEqual(GrillWalk.start(qs([.answered, .decided, .open])), "q3", "answers given on the cards are passed over")
        XCTAssertNil(GrillWalk.start(qs([.answered, .discuss])))
    }

    func testMovesOnAndSendsAfterTheLast() {
        let round = qs([.open, .answered, .open, .open])
        XCTAssertEqual(GrillWalk.next(after: "q1", in: round), .ask("q3"), "an answered question is passed over")
        XCTAssertEqual(GrillWalk.next(after: "q3", in: round), .ask("q4"))
        XCTAssertEqual(GrillWalk.next(after: "q4", in: round), .send)
        // A question skipped earlier doesn't hold the round: it's carried into the next one.
        XCTAssertEqual(GrillWalk.next(after: "q3", in: qs([.open, .answered, .open])), .send)
        // The round changed under the voice: start again from its first open question.
        XCTAssertEqual(GrillWalk.next(after: "gone", in: round), .ask("q1"))
    }

    func testBackGoesToThePreviousQuestionAnsweredOrNot() {
        let round = qs([.answered, .open])
        XCTAssertEqual(GrillWalk.back(from: "q2", in: round), "q1")
        XCTAssertNil(GrillWalk.back(from: "q1", in: round))
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

    func testRestoresTheOpenRoundAndStartsAtItsFirstQuestion() async throws {
        let store = try await store(FixtureAPI(grill: true))
        XCTAssertTrue(store.supportsGrill)
        let s = try XCTUnwrap(store.openGrill(for: "orch"))
        XCTAssertEqual(s.round, 2)
        XCTAssertEqual(s.questions?.map(\.id), ["q3", "q4", "q5"])
        XCTAssertEqual(s.questions?.first?.fromRound, 1, "carried over from round 1")
        XCTAssertEqual(store.currentGrillQuestion(for: "orch")?.id, "q3")
        try await store.loadConversation("orch")
        XCTAssertEqual(store.grillRoundMessage(for: "orch")?.grill?.round, 2)
        XCTAssertNil(store.openGrill(for: "be"))
        store.deactivate()
    }

    func testCardAnswersAreHeldWithoutMovingTheVoice() async throws {
        let api = FixtureAPI(grill: true)
        let store = try await store(api)
        try await store.loadConversation("orch")
        let before = store.conversation("orch").count
        try await store.answerGrill("Yes", questionId: "q4", agentId: "orch")
        try await store.answerGrill("Why not a trial?", questionId: "q5", agentId: "orch", discuss: true)
        try await store.sendGrillCommand(.decide, agentId: "orch", questionId: "q3")
        let qs = try XCTUnwrap(store.openGrill(for: "orch")?.questions)
        XCTAssertEqual(qs.map(\.state), [.decided, .answered, .discuss])
        XCTAssertTrue(qs[1].accepted, "yes accepts the recommendation")
        XCTAssertEqual(qs[1].answer, "Only on yearly")
        XCTAssertEqual(store.conversation("orch").count, before + 1, "answers aren't chat messages; deciding adds its status row")
        XCTAssertTrue(store.pending.isEmpty)
        XCTAssertEqual(api.grillCommandsSent, [.decide], "nothing sent the round")
        store.deactivate()
    }

    func testVoiceStepsThroughTheRoundAndSendsItAfterTheLastAnswer() async throws {
        let api = FixtureAPI(grill: true)
        let store = try await store(api)
        try await store.loadConversation("orch")
        try await store.answerCurrentGrill("Monthly only", agentId: "orch")
        XCTAssertEqual(store.openGrill(for: "orch")?.questions?.first?.answer, "Monthly only")
        XCTAssertEqual(store.currentGrillQuestion(for: "orch")?.id, "q4")
        try await store.sendGrillCommand(.skip, agentId: "orch")
        XCTAssertEqual(store.currentGrillQuestion(for: "orch")?.id, "q5")
        try await store.sendGrillCommand(.back, agentId: "orch")
        XCTAssertEqual(store.currentGrillQuestion(for: "orch")?.id, "q4")
        // "I don't know" by voice: left to the agent, and on to the next question.
        try await store.sendGrillCommand(.decide, agentId: "orch")
        XCTAssertEqual(store.openGrill(for: "orch")?.questions?[1].state, .decided)
        XCTAssertEqual(store.currentGrillQuestion(for: "orch")?.id, "q5")
        XCTAssertEqual(api.grillCommandsSent, [.decide], "skip and back stay on the phone")

        // "Yes" to the last question accepts it and sends the round.
        try await store.answerCurrentGrill("yes", agentId: "orch")
        XCTAssertEqual(api.grillCommandsSent, [.decide, .send])
        let sent = try XCTUnwrap(store.conversation("orch").last { $0.grill?.role == .answers })
        XCTAssertEqual(sent.grill?.round, 2)
        XCTAssertTrue(sent.content.contains("Yes (No: a 14-day trial instead)"), sent.content)
        let s = try XCTUnwrap(store.openGrill(for: "orch"))
        XCTAssertEqual(s.round, 3)
        XCTAssertEqual(store.currentGrillQuestion(for: "orch")?.id, s.questions?.first?.id, "the next round starts at its first question")
        XCTAssertEqual(store.grillRoundMessage(for: "orch")?.grill?.round, 3)
        store.deactivate()
    }

    func testPauseAndEnd() async throws {
        let api = FixtureAPI(grill: true)
        let store = try await store(api)
        try await store.sendGrillCommand(.pause, agentId: "orch")
        XCTAssertEqual(store.openGrill(for: "orch")?.state, .paused)
        XCTAssertNotNil(store.currentGrillQuestion(for: "orch"), "a paused round can still be answered")
        try await store.sendGrillCommand(.end, agentId: "orch")
        XCTAssertNil(store.openGrill(for: "orch"), "an ended interview isn't open")
        XCTAssertNil(store.currentGrillQuestion(for: "orch"))
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
        do { try await store.sendGrillCommand(.send, agentId: "orch"); XCTFail("no session") } catch {}
        store.deactivate()
    }
}

final class GrillChatItemTests: XCTestCase {
    func testRoundsAndSentAnswersAreChatItems() {
        func m(_ id: Int64, _ role: String, _ grill: [String: JSONValue]) -> ChatMessage {
            ChatMessage(id: id, agentId: "orch", runId: nil, role: role, kind: "text", content: "x", meta: ["grill": .object(grill)], createdAt: Date())
        }
        let items = ChatItem.build([m(1, "assistant", ["role": "round", "round": .number(1)]), m(2, "user", ["role": "answers", "round": .number(1)]),
                                    m(3, "assistant", ["role": "round", "round": .number(2)])])
        XCTAssertEqual(items.map(\.id), ["m1", "m2", "m3"])
    }
}
