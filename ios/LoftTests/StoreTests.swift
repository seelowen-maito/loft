import XCTest
@testable import Loft

/// Scriptable server for store tests.
final class MockAPI: LoftAPI, @unchecked Sendable {
    let baseURL = URL(string: "https://mock.invalid")!
    var agentList: [Agent] = [Agent(id: "a1", name: "Orchestrator Bot", title: nil, project: "p", model: nil, effort: nil, permissionMode: nil,
                                    intervalMinutes: 0, status: "idle", statusText: nil, running: false, lastRunAt: nil, nextRunAt: nil,
                                    createdAt: nil, totalCostUsd: nil, contextTokens: nil, prompt: nil)]
    var features = ["tasks", "activity", "idempotent_messages"]
    var failSends = false
    var sentClientIds: [String] = []
    var reachable = true
    private var nextId: Int64 = 100

    private func check() throws { if !reachable { throw APIError.unreachable("down") } }
    func info() async throws -> ServerInfo { try check(); return ServerInfo(version: "t", host: "mock", features: features) }
    func agents() async throws -> [Agent] { try check(); return agentList }
    func messages(agentId: String, limit: Int, before: Int64?) async throws -> [ChatMessage] {
        try check()
        return [ChatMessage(id: 1, agentId: agentId, runId: nil, role: "assistant", kind: "text", content: "hi", meta: nil, createdAt: .now)]
    }
    func sendMessage(agentId: String, text: String, clientId: String) async throws -> SendMessageResponse {
        try check()
        // Like URLSession: a request made from a cancelled task fails as cancelled.
        try Task.checkCancellation()
        sentClientIds.append(clientId)
        if failSends { throw APIError.timeout }
        nextId += 1
        let m = ChatMessage(id: nextId, agentId: agentId, runId: nil, role: "user", kind: "text", content: text,
                            meta: ["client_id": .string(clientId)], createdAt: .now)
        return SendMessageResponse(ok: true, queued: false, duplicate: false, message: m)
    }
    func runNow(agentId: String) async throws {}
    func stopRun(agentId: String) async throws {}
    func restartRun(agentId: String) async throws {}
    func resetSession(agentId: String) async throws {}
    func compact(agentId: String) async throws {}
    func permissions() async throws -> [PermissionRequest] { try check(); return [] }
    func decidePermission(id: String, decision: String) async throws {}
    func pullRequests() async throws -> [PullRequest] { [] }
    func tasks() async throws -> [HBTask] { try check(); return [] }
    func createTask(_ fields: [String: JSONValue]) async throws -> HBTask {
        HBTask(id: fields["id"]?.stringValue ?? "x", title: fields["title"]?.stringValue ?? "", description: "", kind: "todo", status: "inbox",
               priority: "normal", project: "", agentId: nil, parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "",
               summary: "", error: "", createdAt: .now, updatedAt: .now, startedAt: nil, completedAt: nil)
    }
    func updateTask(id: String, _ fields: [String: JSONValue]) async throws -> HBTask { throw APIError.server(status: 404, message: "not found") }
    func deleteTask(id: String) async throws {}
    func executeTask(id: String, agentId: String?, note: String) async throws -> ExecuteResponse { throw APIError.server(status: 404, message: "not found") }
    func cancelTask(id: String, stopRun: Bool) async throws -> CancelResponse { throw APIError.server(status: 404, message: "not found") }
    func activity(limit: Int, before: Date?) async throws -> [ActivityItem] { try check(); return [] }
    var planUsage = PlanUsage(error: nil, windows: [.init(key: "five_hour", label: "session", percent: 30, resetsAt: nil)])
    func usage() async throws -> PlanUsage { try check(); return planUsage }
    func system() async throws -> SystemReading { try check(); return SystemReading(cpuTempC: 61, level: "nominal", sampledAt: .now) }
    func agentSummaries() async throws -> [AgentSummary] { try check(); return [] }
    func cleanVoice(text: String) async throws -> VoiceCleanResponse { VoiceCleanResponse(text: text, cleaned: false, reason: nil) }
    func eventLines() async throws -> AsyncThrowingStream<String, Error> {
        try check()
        return AsyncThrowingStream { c in
            let t = Task { while !Task.isCancelled { try? await Task.sleep(for: .seconds(60)) }; c.finish() }
            c.onTermination = { _ in t.cancel() }
        }
    }
}

@MainActor
final class StoreTests: XCTestCase {
    var cacheURL: URL!

    override func setUp() {
        cacheURL = FileManager.default.temporaryDirectory.appendingPathComponent("snap-\(UUID().uuidString).json")
        UserDefaults.standard.removeObject(forKey: Prefs.orchestratorId)
    }

    override func tearDown() { try? FileManager.default.removeItem(at: cacheURL) }

    private func connectedStore(_ api: MockAPI) async throws -> LoftStore {
        let store = LoftStore(api: api, cache: SnapshotStore(url: cacheURL))
        store.activate()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertEqual(store.connection, .connected)
        return store
    }

    /// A reply that arrives without a live event (missed in a reconnect) is still picked up and read: the wait
    /// refetches history and counts the reply once nothing new has come for 2 s. (Was a 40 s UI test.)
    func testAReplyWithoutAnEventIsStillPickedUp() async throws {
        let store = LoftStore(api: FixtureAPI(replies: true), cache: SnapshotStore(url: cacheURL))
        store.activate()
        defer { store.deactivate() }
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        try await store.loadConversation("qa")
        let before = store.conversation("qa").last?.id ?? 0
        await store.send("Run the smoke tests", to: "qa")
        let started = Date()
        let reply = await store.waitForReply(agentId: "qa", after: before) { Date().timeIntervalSince(started) < 20 }
        XCTAssertEqual(reply, "Done. I ran it and everything passed.")
        XCTAssertLessThan(Date().timeIntervalSince(started), 5, "picked up by the early refetch, not after a long fixed wait")
    }

    func testConnectsAndSuggestsOrchestrator() async throws {
        let store = try await connectedStore(MockAPI())
        XCTAssertEqual(store.agents.count, 1)
        XCTAssertEqual(store.orchestrator?.id, "a1")
        XCTAssertTrue(store.supportsTasks)
        XCTAssertTrue(store.isLive)
        store.deactivate()
    }

    func testSendConfirmsOnceEvenWhenTheEventAlsoArrives() async throws {
        let api = MockAPI()
        let store = try await connectedStore(api)
        try await store.loadConversation("a1")
        await store.send("  hello  ", to: "a1")
        XCTAssertTrue(store.pending.isEmpty)
        let sent = try XCTUnwrap(store.conversation("a1").last)
        XCTAssertEqual(sent.content, "hello")
        // The same message echoed over SSE isn't added twice.
        store.apply(.message(sent))
        XCTAssertEqual(store.conversation("a1").filter { $0.id == sent.id }.count, 1)
        store.deactivate()
    }

    func testFailedSendIsKeptAndRetriedWithTheSameClientId() async throws {
        let api = MockAPI()
        api.failSends = true
        let store = try await connectedStore(api)
        await store.send("deploy", to: "a1")
        let p = try XCTUnwrap(store.pending.first)
        XCTAssertTrue(p.isFailed)
        api.failSends = false
        await store.retry(p)
        XCTAssertTrue(store.pending.isEmpty)
        XCTAssertEqual(api.sentClientIds, [p.id, p.id])
        store.deactivate()
    }

    func testASendIsNotCancelledByTheTaskThatStartedIt() async throws {
        // The Talk screen's countdown cancelled itself and then sent from inside: "Not sent: Cancelled".
        let api = MockAPI()
        let store = try await connectedStore(api)
        let caller = Task { @MainActor in
            withUnsafeCurrentTask { $0?.cancel() }
            await store.send("deploy the docs", to: "a1")
        }
        await caller.value
        XCTAssertTrue(store.pending.isEmpty, "sent, not kept as failed: \(store.pending.map { $0.failure ?? "" })")
        XCTAssertEqual(api.sentClientIds.count, 1)
        store.deactivate()
    }

    func testPendingIsClearedByItsEcho() async throws {
        let api = MockAPI()
        api.failSends = true
        let store = try await connectedStore(api)
        try await store.loadConversation("a1")
        await store.send("x", to: "a1")
        let p = try XCTUnwrap(store.pending.first)
        store.apply(.message(ChatMessage(id: 500, agentId: "a1", runId: nil, role: "user", kind: "text", content: "x",
                                         meta: ["client_id": .string(p.id)], createdAt: .now)))
        XCTAssertTrue(store.pending.isEmpty)
        store.deactivate()
    }

    func testWritesAreRefusedWhileDisconnected() async throws {
        let api = MockAPI()
        api.reachable = false
        let store = LoftStore(api: api, cache: SnapshotStore(url: cacheURL))
        XCTAssertFalse(store.canWrite)
        do {
            try await store.createTask(["title": "x"])
            XCTFail("should refuse")
        } catch let e as APIError {
            XCTAssertEqual(e, .offline)
        }
        await store.send("later", to: "a1")
        XCTAssertEqual(store.pending.first?.failure, APIError.offline.title)
        XCTAssertTrue(api.sentClientIds.isEmpty)
    }

    func testOlderServerHidesTasks() async throws {
        let api = MockAPI()
        api.features = []
        let store = try await connectedStore(api)
        XCTAssertFalse(store.supportsTasks)
        do { try await store.createTask(["title": "x"]); XCTFail() } catch let e as APIError { XCTAssertEqual(e, .unsupported("Tasks")) }
        store.deactivate()
    }

    func testSnapshotRestoresAsStaleData() async throws {
        let api = MockAPI()
        let store = try await connectedStore(api)
        store.deactivate() // saves
        api.reachable = false
        let offline = LoftStore(api: api, cache: SnapshotStore(url: cacheURL))
        XCTAssertEqual(offline.agents.map(\.id), ["a1"])
        XCTAssertNotNil(offline.lastSync)
        XCTAssertFalse(offline.isLive)
        // A snapshot from another server is never shown.
        XCTAssertNil(SnapshotStore(url: cacheURL).load(for: "https://other.invalid"))
    }

    func testAgentEventsUpsertAndDelete() async throws {
        let store = try await connectedStore(MockAPI())
        var a = store.agents[0]
        a.status = "running"
        store.apply(.agentUpdated(a))
        XCTAssertEqual(store.state(of: store.agents[0]), .working)
        store.apply(.agentDeleted("a1"))
        XCTAssertTrue(store.agents.isEmpty)
        store.deactivate()
    }
}
