import Foundation
import os

/// The Loft REST API (backend/src/main.rs). A protocol so tests and UI fixtures can stand in for the server.
protocol LoftAPI: Sendable {
    var baseURL: URL { get }
    func info() async throws -> ServerInfo
    func agents() async throws -> [Agent]
    func messages(agentId: String, limit: Int, before: Int64?) async throws -> [ChatMessage]
    func sendMessage(agentId: String, text: String, clientId: String) async throws -> SendMessageResponse
    /// `meta` carries only what the server accepts from clients (today `grill`).
    func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?) async throws -> SendMessageResponse
    /// A message with uploads (`images`: names from `upload`).
    func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?, images: [String]) async throws -> SendMessageResponse
    /// `POST /agents/{id}/uploads` with the file's bytes; returns the server's name for it. `progress` gets 0...1 as it goes up.
    func upload(agentId: String, file: URL, progress: @escaping @Sendable (Double) -> Void) async throws -> String
    /// Where an upload is served (it answers Range requests, so a video plays from it).
    func uploadURL(agentId: String, file: String) -> URL
    /// The agent's open interview, or nil when there is none (404).
    func grillSession(agentId: String) async throws -> GrillSession?
    func grillCommand(sessionId: String, command: GrillCommand, questionId: String?, clientId: String) async throws -> GrillSession?
    func grillPhrases() async throws -> GrillPhrases
    /// "Set up the team with loft" on the end-of-planning card `messageId`: the server sends its setup message
    /// once and returns the card with `sent_at` (`duplicate` when it was already sent).
    func sendTeamSetup(agentId: String, messageId: Int64) async throws -> SendMessageResponse
    /// "Start delivery" on the end-of-planning card `messageId`: the server starts the spec's first ready ticket
    /// on the team's Dev once and returns the card with `delivery.started` (`duplicate` when it had started).
    /// `initGit`: the user said yes to setting up git in a project folder that has none (`needsGit` came back).
    func startDelivery(agentId: String, messageId: Int64, initGit: Bool) async throws -> SendMessageResponse
    /// A standalone ticket's or a spec's Delivery, for its issue page; nil when it has none.
    func issueDelivery(taskId: String) async throws -> IssueDelivery?
    /// The issue page's Delivery card buttons: Start delivery (once), Pause on a running one, Resume on a stalled
    /// or paused one or one waiting for the user. Each answers with the Delivery as the card shows it now.
    func issueDeliveryAction(taskId: String, _ action: IssueDeliveryAction) async throws -> IssueDeliveryReply
    func taskPlan(id: String) async throws -> TaskPlan
    func needsYou() async throws -> NeedsYouList
    func needsYouRequest(method: String, path: String, body: JSONValue?) async throws
    func registerDevice(_ d: DeviceRegistration) async throws
    func unregisterDevice(token: String) async throws
    func testPush(token: String) async throws
    /// `PATCH /api/devices/{token}` with some of `kinds`, `quiet_hours` (null clears them), `name`.
    func updateDevice(token: String, _ fields: [String: JSONValue]) async throws
    func runNow(agentId: String) async throws
    /// `PATCH /api/agents/{id}` with some of its editable fields (today `fresh_context`).
    func updateAgent(id: String, _ fields: [String: JSONValue]) async throws -> Agent
    func stopRun(agentId: String) async throws
    /// Bytes in the agent's workspace: what deleting it with the agent frees.
    func workspaceSize(agentId: String) async throws -> Int64
    /// Deletes the agent; with `purgeFiles` its workspace folder too.
    func deleteAgent(id: String, purgeFiles: Bool) async throws
    /// Stops the run and keeps the queue; queued messages run next (`run_watchdog`).
    func restartRun(agentId: String) async throws
    /// Clear context: a fresh Claude session, memory kept.
    func resetSession(agentId: String) async throws
    /// Compact: Claude summarizes the Session so later runs carry less. Fails while the agent works.
    func compact(agentId: String) async throws
    /// The agent's past Sessions, newest first.
    func sessions(agentId: String) async throws -> PastSessions
    /// Makes a past Session the agent's own: its next run resumes it. Fails while the agent works.
    func resumeSession(agentId: String, sessionId: String) async throws -> Agent
    func permissions() async throws -> [PermissionRequest]
    func decidePermission(id: String, decision: String) async throws
    func pullRequests() async throws -> [PullRequest]
    /// Every spec's Delivery in one read (the web board's source).
    func deliverySummaries() async throws -> [DeliverySummary]
    /// Merge for a plan kept in Loft (`POST /deliveries/merge`); the server's message says what merged.
    func mergeDelivery(repo: String, spec: Int) async throws -> String
    func tasks() async throws -> [HBTask]
    func createTask(_ fields: [String: JSONValue]) async throws -> HBTask
    func updateTask(id: String, _ fields: [String: JSONValue]) async throws -> HBTask
    func deleteTask(id: String) async throws
    func executeTask(id: String, agentId: String?, note: String) async throws -> ExecuteResponse
    func cancelTask(id: String, stopRun: Bool) async throws -> CancelResponse
    /// Publish a Loft-only issue to its project's GitHub repo (spec #65); it comes back mirrored.
    func publishTask(id: String) async throws -> HBTask
    /// A Loft issue's thread, oldest first.
    func taskComments(id: String) async throws -> [TaskComment]
    /// Comment on a Loft issue; it goes to the issue's agent. `github`: on a mirrored issue, whether it may go to
    /// GitHub (after approval, as the setting says); nil leaves it to the server.
    func addTaskComment(id: String, text: String, clientId: String, github: Bool?) async throws
    /// The server's settings object (`GET /api/settings`'s `settings`), and replacing it.
    func serverSettings() async throws -> JSONValue
    func putServerSettings(_ settings: JSONValue) async throws -> JSONValue
    /// The Mac's backups (`backups`), and Back up now, which answers once it's done or fails with the error.
    func backups() async throws -> BackupStatus
    func backUpNow() async throws -> BackupStatus
    /// Settings › Disk: every workspace's size and the free space, and Clean: removes the build output of one
    /// agent (`agentId`) or of every idle agent (nil), whatever its age.
    func workspaceSizes() async throws -> WorkspaceSizes
    /// What Clean (one agent) or Clean all with nil lists; `clean` removes the paths ticked.
    func cleanPreview(agentId: String?) async throws -> [CleanItem]
    /// The folders in `rel` inside one of the agent's main clones.
    func cleanFolders(agentId: String, rel: String) async throws -> [CleanItem]
    func clean(agentId: String?, paths: [String]) async throws -> CleanLog
    /// The running or last clean's log, the daily sweep's included.
    func cleanLog() async throws -> CleanLog
    /// A comment waiting for GitHub: post it (with `text` when edited), or keep it in Loft.
    func approveComment(taskId: String, commentId: Int64, text: String?) async throws
    func keepComment(taskId: String, commentId: Int64) async throws
    func activity(limit: Int, before: Date?) async throws -> [ActivityItem]
    func usage() async throws -> PlanUsage
    func system() async throws -> SystemReading
    func agentSummaries() async throws -> [AgentSummary]
    /// Tidy a long dictation on the Mac. Returns the original text with `cleaned: false` when that fails.
    func cleanVoice(text: String) async throws -> VoiceCleanResponse
    /// A long dictation as a brief, by Claude on the Mac (`voice_brief`). The original comes back when it can't.
    func briefVoice(text: String) async throws -> VoiceBriefResponse
    /// Server-sent events; the caller handles reconnecting.
    func eventLines() async throws -> AsyncThrowingStream<String, Error>
    /// Pairing (spec #164): ask to be paired, poll with the secret, or give the pending pairing up.
    func startPairing(name: String) async throws -> PairingStart
    func pollPairing(secret: String) async throws -> PairingPoll
    func cancelPairing(secret: String) async throws
    /// Paired devices (#170): the list, Forget this device, and Unpair all (every client but this one).
    func pairedClients() async throws -> [PairedClient]
    func forgetClient(id: String) async throws
    func unpairAll() async throws
}

struct LiveLoftAPI: LoftAPI {
    let baseURL: URL
    /// This app's pairing token for `baseURL`, from the Keychain; nil until paired (and always in the Simulator).
    private let token: String?
    var isPaired: Bool { token != nil }
    private let session: URLSession
    private let streamSession: URLSession

    /// How many were made: every request the app sends goes through one, so a test can prove the demo made none.
    nonisolated(unsafe) private static var made = 0
    private static let madeLock = NSLock()
    static var instancesMade: Int { madeLock.withLock { made } }

    init(baseURL: URL) {
        Self.madeLock.withLock { Self.made += 1 }
        self.baseURL = baseURL
        token = ClientToken.load(for: baseURL)
        let cfg = URLSessionConfiguration.default
        cfg.timeoutIntervalForRequest = 15
        cfg.waitsForConnectivity = false
        cfg.requestCachePolicy = .reloadIgnoringLocalCacheData
        session = URLSession(configuration: cfg)
        let sse = URLSessionConfiguration.default
        // Keep-alives arrive every ~15s; a minute of silence means the stream is dead.
        sse.timeoutIntervalForRequest = 60
        sse.timeoutIntervalForResource = .infinity
        streamSession = URLSession(configuration: sse)
    }

    /// Normalizes what the user typed: adds https://, drops a trailing slash or `/api`.
    static func normalize(_ raw: String) -> URL? {
        var s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !s.isEmpty else { return nil }
        if !s.contains("://") { s = "https://" + s }
        while s.hasSuffix("/") { s.removeLast() }
        if s.hasSuffix("/api") { s.removeLast(4) }
        guard let url = URL(string: s), let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme),
              let host = url.host, !host.isEmpty else { return nil }
        return url
    }

    private func url(_ path: String, query: [URLQueryItem] = []) -> URL {
        var c = URLComponents(url: baseURL.appendingPathComponent("api" + path), resolvingAgainstBaseURL: false)!
        if !query.isEmpty { c.queryItems = query }
        return c.url!
    }

    private func send(_ method: String, _ path: String, query: [URLQueryItem] = [], body: JSONValue? = nil, timeout: TimeInterval? = nil) async throws -> Data {
        var req = URLRequest(url: url(path, query: query))
        if let timeout { req.timeoutInterval = timeout }
        req.httpMethod = method
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        if let token { req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        if let body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = try JSONEncoder().encode(body)
        }
        let started = Date()
        let (data, resp): (Data, URLResponse)
        do {
            (data, resp) = try await session.data(for: req)
        } catch {
            Log.net.error("\(method, privacy: .public) \(path, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
            throw APIError.from(error)
        }
        return try checked(data, resp, method, path, started)
    }

    private func checked(_ data: Data, _ resp: URLResponse, _ method: String, _ path: String, _ started: Date) throws -> Data {
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        Log.net.debug("\(method, privacy: .public) \(path, privacy: .public) → \(status) in \(Int(Date().timeIntervalSince(started) * 1000))ms")
        guard (200..<300).contains(status) else {
            if status == 401 { Pairing.reportUnpaired() }
            let message = (try? JSONDecoder().decode([String: String].self, from: data))?["error"]
                ?? HTTPURLResponse.localizedString(forStatusCode: status)
            throw APIError.server(status: status, message: message)
        }
        return data
    }

    private func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
        do {
            return try JSONCoding.decoder.decode(type, from: data)
        } catch {
            Log.net.error("decoding \(String(describing: T.self), privacy: .public): \(String(describing: error), privacy: .public)")
            throw APIError.from(error)
        }
    }

    private func get<T: Decodable>(_ path: String, query: [URLQueryItem] = []) async throws -> T {
        try decode(T.self, try await send("GET", path, query: query))
    }

    func info() async throws -> ServerInfo { try await get("/info") }
    func taskPlan(id: String) async throws -> TaskPlan { try await get("/tasks/\(id)/plan") }
    func needsYou() async throws -> NeedsYouList { try await get("/needs-you") }
    func needsYouRequest(method: String, path: String, body: JSONValue?) async throws {
        // Actions name the full path ("/api/permissions/p1"); `send` adds the /api prefix itself.
        let p = path.hasPrefix("/api/") ? String(path.dropFirst(4)) : path
        _ = try await send(method, p, body: body)
    }
    func registerDevice(_ d: DeviceRegistration) async throws { _ = try await send("POST", "/devices", body: d.json) }
    func unregisterDevice(token: String) async throws { _ = try await send("DELETE", "/devices/\(token)") }
    func testPush(token: String) async throws { _ = try await send("POST", "/devices/\(token)/test") }
    func updateDevice(token: String, _ fields: [String: JSONValue]) async throws { _ = try await send("PATCH", "/devices/\(token)", body: .object(fields)) }
    func agents() async throws -> [Agent] { try await get("/agents") }

    func messages(agentId: String, limit: Int, before: Int64?) async throws -> [ChatMessage] {
        var q = [URLQueryItem(name: "limit", value: String(limit))]
        if let before { q.append(URLQueryItem(name: "before", value: String(before))) }
        return try await get("/agents/\(agentId)/messages", query: q)
    }

    func sendMessage(agentId: String, text: String, clientId: String) async throws -> SendMessageResponse {
        try await sendMessage(agentId: agentId, text: text, clientId: clientId, meta: nil)
    }

    func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?) async throws -> SendMessageResponse {
        try await sendMessage(agentId: agentId, text: text, clientId: clientId, meta: meta, images: [])
    }

    func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?, images: [String]) async throws -> SendMessageResponse {
        var body: [String: JSONValue] = ["text": .string(text), "client_id": .string(clientId)]
        if let meta { body["meta"] = meta }
        if !images.isEmpty { body["images"] = .array(images.map(JSONValue.string)) }
        return try decode(SendMessageResponse.self, try await send("POST", "/agents/\(agentId)/messages", body: .object(body)))
    }

    func upload(agentId: String, file: URL, progress: @escaping @Sendable (Double) -> Void) async throws -> String {
        let path = "/agents/\(agentId)/uploads"
        var req = URLRequest(url: url(path))
        req.httpMethod = "POST"
        req.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        // Up to 200 MB over the tailnet: give the server time to answer once it has it all.
        req.timeoutInterval = 120
        let started = Date()
        let (data, resp): (Data, URLResponse)
        do {
            (data, resp) = try await session.upload(for: req, fromFile: file, delegate: UploadProgress(progress))
        } catch {
            Log.net.error("POST \(path, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
            throw APIError.from(error)
        }
        let r = try decode([String: String].self, try checked(data, resp, "POST", path, started))
        guard let name = r["file"] else { throw APIError.server(status: 200, message: "The upload has no name") }
        return name
    }

    func uploadURL(agentId: String, file: String) -> URL { url("/agents/\(agentId)/uploads/\(file)") }

    func grillSession(agentId: String) async throws -> GrillSession? {
        do {
            return try Self.decodeGrillSession(try await send("GET", "/agents/\(agentId)/grill"))
        } catch APIError.server(let status, _) where status == 404 {
            return nil
        }
    }

    func grillCommand(sessionId: String, command: GrillCommand, questionId: String?, clientId: String) async throws -> GrillSession? {
        var body: [String: JSONValue] = ["command": .string(command.rawValue), "client_id": .string(clientId)]
        if let questionId { body["question_id"] = .string(questionId) }
        let data = try await send("POST", "/grill/\(sessionId)/command", body: .object(body))
        return try? Self.decodeGrillSession(data)
    }

    func grillPhrases() async throws -> GrillPhrases { try await get("/grill/commands") }

    func sendTeamSetup(agentId: String, messageId: Int64) async throws -> SendMessageResponse {
        try decode(SendMessageResponse.self, try await send("POST", "/agents/\(agentId)/messages/\(messageId)/team-setup"))
    }

    func startDelivery(agentId: String, messageId: Int64, initGit: Bool) async throws -> SendMessageResponse {
        try decode(SendMessageResponse.self, try await send("POST", "/agents/\(agentId)/messages/\(messageId)/start-delivery\(initGit ? "?init_git=true" : "")"))
    }

    func issueDelivery(taskId: String) async throws -> IssueDelivery? { try await get("/issues/\(taskId)/delivery") }

    func issueDeliveryAction(taskId: String, _ action: IssueDeliveryAction) async throws -> IssueDeliveryReply {
        try decode(IssueDeliveryReply.self, try await send("POST", "/issues/\(taskId)/\(action.rawValue)-delivery"))
    }

    /// The grill endpoints answer with the session either bare or wrapped as `{ "session": … }`
    /// (`{ "session": null }` when there is none).
    static func decodeGrillSession(_ data: Data) throws -> GrillSession? {
        // Only a body with a "session" key is the wrapped form; otherwise the envelope's optional field
        // would swallow a bare session as nil.
        if case .object(let o)? = try? JSONDecoder().decode(JSONValue.self, from: data), o["session"] != nil {
            return try JSONCoding.decoder.decode(GrillSessionEnvelope.self, from: data).session
        }
        return try JSONCoding.decoder.decode(GrillSession.self, from: data)
    }

    func runNow(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/run") }
    func updateAgent(id: String, _ fields: [String: JSONValue]) async throws -> Agent {
        try decode(Agent.self, try await send("PATCH", "/agents/\(id)", body: .object(fields)))
    }
    func stopRun(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/stop") }
    func workspaceSize(agentId: String) async throws -> Int64 {
        struct Size: Decodable { let bytes: Int64 }
        return try await (get("/agents/\(agentId)/workspace-size") as Size).bytes
    }
    func deleteAgent(id: String, purgeFiles: Bool) async throws {
        _ = try await send("DELETE", "/agents/\(id)", query: [URLQueryItem(name: "purge_files", value: String(purgeFiles))])
    }
    func restartRun(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/restart") }
    func resetSession(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/reset-session") }
    func compact(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/compact", body: .object(["instructions": .string("")])) }
    func sessions(agentId: String) async throws -> PastSessions { try await get("/agents/\(agentId)/sessions") }
    func resumeSession(agentId: String, sessionId: String) async throws -> Agent {
        try decode(Agent.self, try await send("POST", "/agents/\(agentId)/resume-session", body: ["session_id": .string(sessionId)]))
    }
    func permissions() async throws -> [PermissionRequest] { try await get("/permissions") }

    func decidePermission(id: String, decision: String) async throws {
        _ = try await send("POST", "/permissions/\(id)", body: ["decision": .string(decision)])
    }

    func pullRequests() async throws -> [PullRequest] { try await get("/pull-requests") }
    func deliverySummaries() async throws -> [DeliverySummary] { try await get("/deliveries/summary") }
    func mergeDelivery(repo: String, spec: Int) async throws -> String {
        struct Merged: Decodable { var message: String }
        return try decode(Merged.self, try await send("POST", "/deliveries/merge", body: ["repo": .string(repo), "spec": .number(Double(spec))], timeout: 120)).message
    }
    func tasks() async throws -> [HBTask] { try await get("/tasks") }

    func createTask(_ fields: [String: JSONValue]) async throws -> HBTask {
        try decode(HBTask.self, try await send("POST", "/tasks", body: .object(fields)))
    }

    func updateTask(id: String, _ fields: [String: JSONValue]) async throws -> HBTask {
        try decode(HBTask.self, try await send("PATCH", "/tasks/\(id)", body: .object(fields)))
    }

    func deleteTask(id: String) async throws { _ = try await send("DELETE", "/tasks/\(id)") }

    func executeTask(id: String, agentId: String?, note: String) async throws -> ExecuteResponse {
        let body: JSONValue = ["agent_id": .optional(agentId), "note": .string(note)]
        return try decode(ExecuteResponse.self, try await send("POST", "/tasks/\(id)/execute", body: body))
    }

    func cancelTask(id: String, stopRun: Bool) async throws -> CancelResponse {
        try decode(CancelResponse.self, try await send("POST", "/tasks/\(id)/cancel", body: ["stop_run": .bool(stopRun)]))
    }

    func publishTask(id: String) async throws -> HBTask {
        try decode(HBTask.self, try await send("POST", "/tasks/\(id)/publish", body: [:]))
    }

    func taskComments(id: String) async throws -> [TaskComment] { try await get("/tasks/\(id)/comments") }

    func addTaskComment(id: String, text: String, clientId: String, github: Bool?) async throws {
        var body: [String: JSONValue] = ["text": .string(text), "client_id": .string(clientId)]
        if let github { body["github"] = .bool(github) }
        _ = try await send("POST", "/tasks/\(id)/comments", body: .object(body))
    }

    func serverSettings() async throws -> JSONValue {
        let v: JSONValue = try await get("/settings")
        return v["settings"] ?? .null
    }

    func putServerSettings(_ settings: JSONValue) async throws -> JSONValue {
        let v = try decode(JSONValue.self, try await send("PUT", "/settings", body: settings))
        return v["settings"] ?? .null
    }

    func backups() async throws -> BackupStatus { try await get("/backups") }
    func workspaceSizes() async throws -> WorkspaceSizes { try await get("/workspaces") }
    func cleanPreview(agentId: String?) async throws -> [CleanItem] {
        let l: CleanListing = try await get("/workspaces/clean/preview", query: agentId.map { [URLQueryItem(name: "agent", value: $0)] } ?? [])
        return l.items
    }
    func cleanFolders(agentId: String, rel: String) async throws -> [CleanItem] {
        let l: CleanListing = try await get("/workspaces/clean/folders", query: [URLQueryItem(name: "agent", value: agentId), URLQueryItem(name: "rel", value: rel)])
        return l.items
    }
    func clean(agentId: String?, paths: [String]) async throws -> CleanLog {
        let body: JSONValue = .object(["agent": agentId.map { .string($0) } ?? .null, "paths": .array(paths.map { .string($0) })])
        return try decode(CleanLog.self, try await send("POST", "/workspaces/clean", body: body, timeout: 600))
    }
    func cleanLog() async throws -> CleanLog { try await get("/workspaces/log") }

    func backUpNow() async throws -> BackupStatus {
        // A backup of a big database takes a while; the server answers once it's done.
        try decode(BackupStatus.self, try await send("POST", "/backups", body: [:], timeout: 120))
    }

    func approveComment(taskId: String, commentId: Int64, text: String?) async throws {
        _ = try await send("POST", "/tasks/\(taskId)/comments/\(commentId)/approve", body: text.map { ["text": .string($0)] } ?? [:])
    }

    func keepComment(taskId: String, commentId: Int64) async throws {
        _ = try await send("POST", "/tasks/\(taskId)/comments/\(commentId)/keep", body: [:])
    }

    func activity(limit: Int, before: Date?) async throws -> [ActivityItem] {
        var q = [URLQueryItem(name: "limit", value: String(limit))]
        if let before { q.append(URLQueryItem(name: "before", value: JSONCoding.formatDate(before))) }
        return try await get("/activity", query: q)
    }

    func usage() async throws -> PlanUsage { try await get("/usage") }
    func system() async throws -> SystemReading { try await get("/system") }
    func agentSummaries() async throws -> [AgentSummary] { try await get("/agents/summary") }

    func cleanVoice(text: String) async throws -> VoiceCleanResponse {
        try decode(VoiceCleanResponse.self, try await send("POST", "/voice/clean", body: ["text": .string(text)], timeout: 60))
    }

    func briefVoice(text: String) async throws -> VoiceBriefResponse {
        try decode(VoiceBriefResponse.self, try await send("POST", "/voice/brief", body: ["text": .string(text)], timeout: LoftStore.briefTimeout))
    }

    func startPairing(name: String) async throws -> PairingStart {
        try decode(PairingStart.self, try await send("POST", "/pair", body: ["name": .string(name)]))
    }

    func pollPairing(secret: String) async throws -> PairingPoll {
        try decode(PairingPoll.self, try await send("POST", "/pair/poll", body: ["secret": .string(secret)]))
    }

    func cancelPairing(secret: String) async throws { _ = try await send("POST", "/pair/cancel", body: ["secret": .string(secret)]) }

    func pairedClients() async throws -> [PairedClient] { try decode([PairedClient].self, try await send("GET", "/paired-clients")) }

    func forgetClient(id: String) async throws { _ = try await send("DELETE", "/paired-clients/\(id)") }

    func unpairAll() async throws { _ = try await send("POST", "/paired-clients/unpair-all", body: [:]) }

    func eventLines() async throws -> AsyncThrowingStream<String, Error> {
        var req = URLRequest(url: url("/events"))
        req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        if let token { req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        let bytes: URLSession.AsyncBytes, resp: URLResponse
        do {
            (bytes, resp) = try await streamSession.bytes(for: req)
        } catch {
            throw APIError.from(error)
        }
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else { throw APIError.server(status: status, message: "event stream unavailable") }
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    // `lines` skips empty lines, which SSE uses as event separators, so split by hand.
                    var buf = [UInt8]()
                    for try await byte in bytes {
                        if byte == UInt8(ascii: "\n") {
                            var line = String(decoding: buf, as: UTF8.self)
                            if line.hasSuffix("\r") { line.removeLast() }
                            continuation.yield(line)
                            buf.removeAll(keepingCapacity: true)
                        } else {
                            buf.append(byte)
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: APIError.from(error))
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

/// Reports how much of an upload's body has gone out.
private final class UploadProgress: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    let report: @Sendable (Double) -> Void
    init(_ report: @escaping @Sendable (Double) -> Void) { self.report = report }

    func urlSession(_ session: URLSession, task: URLSessionTask, didSendBodyData bytesSent: Int64, totalBytesSent: Int64,
                    totalBytesExpectedToSend: Int64) {
        guard totalBytesExpectedToSend > 0 else { return }
        report(Double(totalBytesSent) / Double(totalBytesExpectedToSend))
    }
}

/// Defaults for stand-ins written before grill mode (test mocks): no meta, no sessions.
extension LoftAPI {
    func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?, images: [String]) async throws -> SendMessageResponse {
        guard images.isEmpty else { throw APIError.unsupported("Videos") }
        return try await sendMessage(agentId: agentId, text: text, clientId: clientId, meta: meta)
    }
    func upload(agentId: String, file: URL, progress: @escaping @Sendable (Double) -> Void) async throws -> String { throw APIError.unsupported("Videos") }
    func uploadURL(agentId: String, file: String) -> URL {
        baseURL.appendingPathComponent("api/agents/\(agentId)/uploads/\(file)")
    }
    func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?) async throws -> SendMessageResponse {
        try await sendMessage(agentId: agentId, text: text, clientId: clientId)
    }
    func grillSession(agentId: String) async throws -> GrillSession? { nil }
    func grillCommand(sessionId: String, command: GrillCommand, questionId: String?, clientId: String) async throws -> GrillSession? {
        throw APIError.unsupported("Grill mode")
    }
    func grillPhrases() async throws -> GrillPhrases { throw APIError.unsupported("Grill mode") }
    func sendTeamSetup(agentId: String, messageId: Int64) async throws -> SendMessageResponse { throw APIError.unsupported("Team setup") }
    func startDelivery(agentId: String, messageId: Int64, initGit: Bool) async throws -> SendMessageResponse { throw APIError.unsupported("Delivery") }
    func issueDelivery(taskId: String) async throws -> IssueDelivery? { nil }
    func issueDeliveryAction(taskId: String, _ action: IssueDeliveryAction) async throws -> IssueDeliveryReply { throw APIError.unsupported("Delivery") }
    func publishTask(id: String) async throws -> HBTask { throw APIError.unsupported("Publishing to GitHub") }
    func mergeDelivery(repo: String, spec: Int) async throws -> String { throw APIError.unsupported("Delivery") }
    func updateAgent(id: String, _ fields: [String: JSONValue]) async throws -> Agent { throw APIError.unsupported("Agent settings") }
    func workspaceSize(agentId: String) async throws -> Int64 { throw APIError.unsupported("Workspace size") }
    func deleteAgent(id: String, purgeFiles: Bool) async throws { throw APIError.unsupported("Deleting agents") }
    func taskComments(id: String) async throws -> [TaskComment] { [] }
    func addTaskComment(id: String, text: String, clientId: String, github: Bool?) async throws { throw APIError.unsupported("Comments") }
    func serverSettings() async throws -> JSONValue { throw APIError.unsupported("Server settings") }
    func putServerSettings(_ settings: JSONValue) async throws -> JSONValue { throw APIError.unsupported("Server settings") }
    func backups() async throws -> BackupStatus { throw APIError.unsupported("Backups") }
    func backUpNow() async throws -> BackupStatus { throw APIError.unsupported("Backups") }
    func workspaceSizes() async throws -> WorkspaceSizes { throw APIError.unsupported("Disk usage") }
    func cleanPreview(agentId: String?) async throws -> [CleanItem] { throw APIError.unsupported("Disk usage") }
    func cleanFolders(agentId: String, rel: String) async throws -> [CleanItem] { throw APIError.unsupported("Disk usage") }
    func clean(agentId: String?, paths: [String]) async throws -> CleanLog { throw APIError.unsupported("Disk usage") }
    func cleanLog() async throws -> CleanLog { throw APIError.unsupported("Disk usage") }
    func approveComment(taskId: String, commentId: Int64, text: String?) async throws { throw APIError.unsupported("Comments") }
    func keepComment(taskId: String, commentId: Int64) async throws { throw APIError.unsupported("Comments") }
    func briefVoice(text: String) async throws -> VoiceBriefResponse { throw APIError.unsupported("Briefs") }
    func taskPlan(id: String) async throws -> TaskPlan { throw APIError.unsupported("Plans") }
    func needsYou() async throws -> NeedsYouList { throw APIError.unsupported("Needs you") }
    /// A "Needs you" action as the server describes it: `method` + `path` (with the /api prefix) + `body`.
    func needsYouRequest(method: String, path: String, body: JSONValue?) async throws { throw APIError.unsupported("Needs you") }
    func registerDevice(_ d: DeviceRegistration) async throws { throw APIError.unsupported("Push") }
    func unregisterDevice(token: String) async throws { throw APIError.unsupported("Push") }
    func testPush(token: String) async throws { throw APIError.unsupported("Push") }
    func updateDevice(token: String, _ fields: [String: JSONValue]) async throws { throw APIError.unsupported("Push") }
    func startPairing(name: String) async throws -> PairingStart { throw APIError.unsupported("Pairing") }
    func pollPairing(secret: String) async throws -> PairingPoll { throw APIError.unsupported("Pairing") }
    func cancelPairing(secret: String) async throws {}
    func pairedClients() async throws -> [PairedClient] { throw APIError.unsupported("Pairing") }
    func forgetClient(id: String) async throws { throw APIError.unsupported("Pairing") }
    func unpairAll() async throws { throw APIError.unsupported("Pairing") }
}
