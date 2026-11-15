import Foundation
import os

/// The Loft REST API (backend/src/main.rs). A protocol so tests and UI fixtures can stand in for the server.
public protocol LoftAPI: Sendable {
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
    func startDelivery(agentId: String, messageId: Int64, repo: String, spec: Int, initGit: Bool) async throws -> SendMessageResponse
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
    /// The user opened the issue: a finished one's result leaves Needs you on every client.
    func issueOpened(id: String) async throws
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
    /// Clean's read-only git check of one of the agent's clones or worktrees; only on the user's tap.
    func cleanCheck(agentId: String, rel: String) async throws -> GitCheck
    func clean(agentId: String?, paths: [String]) async throws -> CleanLog
    /// The running or last clean's log, the daily sweep's included.
    func cleanLog() async throws -> CleanLog
    /// What each idle agent left running, by agent id; Stop them stops one agent's.
    func leftRunning() async throws -> [String: [LeftProcess]]
    func stopLeftRunning(agentId: String) async throws
    /// Agents' Simulator devices, biggest first.
    func agentDevices() async throws -> [AgentDevice]
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
    /// The web's home summary (Loft for Mac's Overview): the projects folder's repos and their GitHub PRs and issues.
    func repos() async throws -> [RepoSummary]
    func repoGithub() async throws -> [String: RepoGithub]
    func agentsTotals() async throws -> AgentsTotals
    /// `GET /api/settings` whole: `settings`, `github_token_set`, `pending_restart`, `running_agents`.
    func settingsView() async throws -> JSONValue
    /// Restarts Loft to apply restart-only settings; refused (409) while agents run.
    func restart() async throws
    /// What's left of GitHub's hourly limit and where Loft's requests went (`GET /api/github/budget`).
    func githubBudget() async throws -> JSONValue
    /// Whether macOS lets the agents into the folders it guards (`POST /api/system/mac-access`).
    func macAccess() async throws -> JSONValue
    /// A repo's recent commits, newest first.
    func repoCommits(name: String) async throws -> [RepoCommit]
    /// Settings › Push notifications: `GET /api/push` (where pushes go), `PUT /api/push` with what changed, Sign in, and
    /// the phones registered (`GET /api/devices`).
    func pushStatus() async throws -> JSONValue
    func configurePush(_ changes: [String: JSONValue]) async throws -> JSONValue
    func pushSignIn() async throws -> JSONValue
    func pushDevices() async throws -> [JSONValue]
    /// Settings › Herdr UI: `GET /api/herdr`, and Set up on this machine (`POST /api/herdr/setup`).
    func herdrStatus() async throws -> JSONValue
    func herdrSetup() async throws -> JSONValue
    /// The agent's "/" menu: `GET /api/agents/{id}/commands`.
    func commands(agentId: String) async throws -> [AgentCommand]
}

public struct LiveLoftAPI: LoftAPI {
    public let baseURL: URL
    private let session: URLSession
    private let streamSession: URLSession

    /// How many were made: every request the app sends goes through one, so a test can prove the demo made none.
    nonisolated(unsafe) private static var made = 0
    private static let madeLock = NSLock()
    public static var instancesMade: Int { madeLock.withLock { made } }

    public init(baseURL: URL) {
        Self.madeLock.withLock { Self.made += 1 }
        self.baseURL = baseURL
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
    public static func normalize(_ raw: String) -> URL? {
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

    public func info() async throws -> ServerInfo { try await get("/info") }
    public func taskPlan(id: String) async throws -> TaskPlan { try await get("/tasks/\(id)/plan") }
    public func needsYou() async throws -> NeedsYouList { try await get("/needs-you") }
    public func needsYouRequest(method: String, path: String, body: JSONValue?) async throws {
        // Actions name the full path ("/api/permissions/p1"); `send` adds the /api prefix itself.
        let p = path.hasPrefix("/api/") ? String(path.dropFirst(4)) : path
        _ = try await send(method, p, body: body)
    }
    public func registerDevice(_ d: DeviceRegistration) async throws { _ = try await send("POST", "/devices", body: d.json) }
    public func unregisterDevice(token: String) async throws { _ = try await send("DELETE", "/devices/\(token)") }
    public func testPush(token: String) async throws { _ = try await send("POST", "/devices/\(token)/test") }
    public func updateDevice(token: String, _ fields: [String: JSONValue]) async throws { _ = try await send("PATCH", "/devices/\(token)", body: .object(fields)) }
    public func agents() async throws -> [Agent] { try await get("/agents") }

    public func messages(agentId: String, limit: Int, before: Int64?) async throws -> [ChatMessage] {
        var q = [URLQueryItem(name: "limit", value: String(limit))]
        if let before { q.append(URLQueryItem(name: "before", value: String(before))) }
        return try await get("/agents/\(agentId)/messages", query: q)
    }

    public func sendMessage(agentId: String, text: String, clientId: String) async throws -> SendMessageResponse {
        try await sendMessage(agentId: agentId, text: text, clientId: clientId, meta: nil)
    }

    public func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?) async throws -> SendMessageResponse {
        try await sendMessage(agentId: agentId, text: text, clientId: clientId, meta: meta, images: [])
    }

    public func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?, images: [String]) async throws -> SendMessageResponse {
        var body: [String: JSONValue] = ["text": .string(text), "client_id": .string(clientId)]
        if let meta { body["meta"] = meta }
        if !images.isEmpty { body["images"] = .array(images.map(JSONValue.string)) }
        return try decode(SendMessageResponse.self, try await send("POST", "/agents/\(agentId)/messages", body: .object(body)))
    }

    public func upload(agentId: String, file: URL, progress: @escaping @Sendable (Double) -> Void) async throws -> String {
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

    public func uploadURL(agentId: String, file: String) -> URL { url("/agents/\(agentId)/uploads/\(file)") }

    public func grillSession(agentId: String) async throws -> GrillSession? {
        do {
            return try Self.decodeGrillSession(try await send("GET", "/agents/\(agentId)/grill"))
        } catch APIError.server(let status, _) where status == 404 {
            return nil
        }
    }

    public func grillCommand(sessionId: String, command: GrillCommand, questionId: String?, clientId: String) async throws -> GrillSession? {
        var body: [String: JSONValue] = ["command": .string(command.rawValue), "client_id": .string(clientId)]
        if let questionId { body["question_id"] = .string(questionId) }
        let data = try await send("POST", "/grill/\(sessionId)/command", body: .object(body))
        return try? Self.decodeGrillSession(data)
    }

    public func grillPhrases() async throws -> GrillPhrases { try await get("/grill/commands") }

    public func sendTeamSetup(agentId: String, messageId: Int64) async throws -> SendMessageResponse {
        try decode(SendMessageResponse.self, try await send("POST", "/agents/\(agentId)/messages/\(messageId)/team-setup"))
    }

    public func startDelivery(agentId: String, messageId: Int64, repo: String, spec: Int, initGit: Bool) async throws -> SendMessageResponse {
        var query = [URLQueryItem(name: "repo", value: repo), URLQueryItem(name: "spec", value: String(spec))]
        if initGit { query.append(URLQueryItem(name: "init_git", value: "true")) }
        return try decode(SendMessageResponse.self, try await send("POST", "/agents/\(agentId)/messages/\(messageId)/start-delivery", query: query))
    }

    public func issueDelivery(taskId: String) async throws -> IssueDelivery? { try await get("/issues/\(taskId)/delivery") }

    public func issueDeliveryAction(taskId: String, _ action: IssueDeliveryAction) async throws -> IssueDeliveryReply {
        try decode(IssueDeliveryReply.self, try await send("POST", "/issues/\(taskId)/\(action.rawValue)-delivery"))
    }

    /// The grill endpoints answer with the session either bare or wrapped as `{ "session": … }`
    /// (`{ "session": null }` when there is none).
    public static func decodeGrillSession(_ data: Data) throws -> GrillSession? {
        // Only a body with a "session" key is the wrapped form; otherwise the envelope's optional field
        // would swallow a bare session as nil.
        if case .object(let o)? = try? JSONDecoder().decode(JSONValue.self, from: data), o["session"] != nil {
            return try JSONCoding.decoder.decode(GrillSessionEnvelope.self, from: data).session
        }
        return try JSONCoding.decoder.decode(GrillSession.self, from: data)
    }

    public func runNow(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/run") }
    public func updateAgent(id: String, _ fields: [String: JSONValue]) async throws -> Agent {
        try decode(Agent.self, try await send("PATCH", "/agents/\(id)", body: .object(fields)))
    }
    public func stopRun(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/stop") }
    public func workspaceSize(agentId: String) async throws -> Int64 {
        struct Size: Decodable { let bytes: Int64 }
        return try await (get("/agents/\(agentId)/workspace-size") as Size).bytes
    }
    public func deleteAgent(id: String, purgeFiles: Bool) async throws {
        _ = try await send("DELETE", "/agents/\(id)", query: [URLQueryItem(name: "purge_files", value: String(purgeFiles))])
    }
    public func restartRun(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/restart") }
    public func resetSession(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/reset-session") }
    public func compact(agentId: String) async throws { _ = try await send("POST", "/agents/\(agentId)/compact", body: .object(["instructions": .string("")])) }
    public func sessions(agentId: String) async throws -> PastSessions { try await get("/agents/\(agentId)/sessions") }
    public func resumeSession(agentId: String, sessionId: String) async throws -> Agent {
        try decode(Agent.self, try await send("POST", "/agents/\(agentId)/resume-session", body: ["session_id": .string(sessionId)]))
    }
    public func permissions() async throws -> [PermissionRequest] { try await get("/permissions") }

    public func decidePermission(id: String, decision: String) async throws {
        _ = try await send("POST", "/permissions/\(id)", body: ["decision": .string(decision)])
    }

    public func pullRequests() async throws -> [PullRequest] { try await get("/pull-requests") }
    public func deliverySummaries() async throws -> [DeliverySummary] { try await get("/deliveries/summary") }
    public func mergeDelivery(repo: String, spec: Int) async throws -> String {
        struct Merged: Decodable { var message: String }
        return try decode(Merged.self, try await send("POST", "/deliveries/merge", body: ["repo": .string(repo), "spec": .number(Double(spec))], timeout: 120)).message
    }
    public func tasks() async throws -> [HBTask] { try await get("/tasks") }

    public func createTask(_ fields: [String: JSONValue]) async throws -> HBTask {
        try decode(HBTask.self, try await send("POST", "/tasks", body: .object(fields)))
    }

    public func updateTask(id: String, _ fields: [String: JSONValue]) async throws -> HBTask {
        try decode(HBTask.self, try await send("PATCH", "/tasks/\(id)", body: .object(fields)))
    }

    public func deleteTask(id: String) async throws { _ = try await send("DELETE", "/tasks/\(id)") }

    public func executeTask(id: String, agentId: String?, note: String) async throws -> ExecuteResponse {
        let body: JSONValue = ["agent_id": .optional(agentId), "note": .string(note)]
        return try decode(ExecuteResponse.self, try await send("POST", "/tasks/\(id)/execute", body: body))
    }

    public func cancelTask(id: String, stopRun: Bool) async throws -> CancelResponse {
        try decode(CancelResponse.self, try await send("POST", "/tasks/\(id)/cancel", body: ["stop_run": .bool(stopRun)]))
    }

    public func publishTask(id: String) async throws -> HBTask {
        try decode(HBTask.self, try await send("POST", "/tasks/\(id)/publish", body: [:]))
    }

    public func issueOpened(id: String) async throws {
        _ = try await send("POST", "/issues/\(id)/opened", body: [:])
    }

    public func taskComments(id: String) async throws -> [TaskComment] { try await get("/tasks/\(id)/comments") }

    public func addTaskComment(id: String, text: String, clientId: String, github: Bool?) async throws {
        var body: [String: JSONValue] = ["text": .string(text), "client_id": .string(clientId)]
        if let github { body["github"] = .bool(github) }
        _ = try await send("POST", "/tasks/\(id)/comments", body: .object(body))
    }

    public func serverSettings() async throws -> JSONValue {
        let v: JSONValue = try await get("/settings")
        return v["settings"] ?? .null
    }

    public func putServerSettings(_ settings: JSONValue) async throws -> JSONValue {
        let v = try decode(JSONValue.self, try await send("PUT", "/settings", body: settings))
        return v["settings"] ?? .null
    }

    public func backups() async throws -> BackupStatus { try await get("/backups") }
    public func workspaceSizes() async throws -> WorkspaceSizes { try await get("/workspaces") }
    public func cleanPreview(agentId: String?) async throws -> [CleanItem] {
        let l: CleanListing = try await get("/workspaces/clean/preview", query: agentId.map { [URLQueryItem(name: "agent", value: $0)] } ?? [])
        return l.items
    }
    public func cleanFolders(agentId: String, rel: String) async throws -> [CleanItem] {
        let l: CleanListing = try await get("/workspaces/clean/folders", query: [URLQueryItem(name: "agent", value: agentId), URLQueryItem(name: "rel", value: rel)])
        return l.items
    }
    public func cleanCheck(agentId: String, rel: String) async throws -> GitCheck {
        try decode(GitCheck.self, try await send("POST", "/workspaces/clean/check", body: .object(["agent": .string(agentId), "rel": .string(rel)]), timeout: 60))
    }
    public func clean(agentId: String?, paths: [String]) async throws -> CleanLog {
        let body: JSONValue = .object(["agent": agentId.map { .string($0) } ?? .null, "paths": .array(paths.map { .string($0) })])
        return try decode(CleanLog.self, try await send("POST", "/workspaces/clean", body: body, timeout: 600))
    }
    public func cleanLog() async throws -> CleanLog { try await get("/workspaces/log") }
    public func leftRunning() async throws -> [String: [LeftProcess]] { try await get("/workspaces/processes") }
    public func stopLeftRunning(agentId: String) async throws { _ = try await send("POST", "/workspaces/\(agentId)/stop-processes", body: .object([:]), timeout: 30) }
    public func agentDevices() async throws -> [AgentDevice] {
        let l: AgentDevices = try await get("/simulator/devices")
        return l.devices
    }

    public func backUpNow() async throws -> BackupStatus {
        // A backup of a big database takes a while; the server answers once it's done.
        try decode(BackupStatus.self, try await send("POST", "/backups", body: [:], timeout: 120))
    }

    public func approveComment(taskId: String, commentId: Int64, text: String?) async throws {
        _ = try await send("POST", "/tasks/\(taskId)/comments/\(commentId)/approve", body: text.map { ["text": .string($0)] } ?? [:])
    }

    public func keepComment(taskId: String, commentId: Int64) async throws {
        _ = try await send("POST", "/tasks/\(taskId)/comments/\(commentId)/keep", body: [:])
    }

    public func activity(limit: Int, before: Date?) async throws -> [ActivityItem] {
        var q = [URLQueryItem(name: "limit", value: String(limit))]
        if let before { q.append(URLQueryItem(name: "before", value: JSONCoding.formatDate(before))) }
        return try await get("/activity", query: q)
    }

    public func usage() async throws -> PlanUsage { try await get("/usage") }

    /// What every agent's runs cost over the Usage week (`GET /api/agents/summary?totals=true`), in US dollars.
    /// Only the widgets read it, so it isn't part of `LoftAPI`.
    public func weekSpend() async throws -> Double {
        struct Totals: Decodable { struct Period: Decodable { var costUsd: Double }; var period: Period }
        let totals: Totals = try await get("/agents/summary", query: [URLQueryItem(name: "totals", value: "true")])
        return totals.period.costUsd
    }
    public func system() async throws -> SystemReading { try await get("/system") }
    public func agentSummaries() async throws -> [AgentSummary] { try await get("/agents/summary") }

    public func cleanVoice(text: String) async throws -> VoiceCleanResponse {
        try decode(VoiceCleanResponse.self, try await send("POST", "/voice/clean", body: ["text": .string(text)], timeout: 60))
    }

    public func briefVoice(text: String) async throws -> VoiceBriefResponse {
        try decode(VoiceBriefResponse.self, try await send("POST", "/voice/brief", body: ["text": .string(text)], timeout: LoftStore.briefTimeout))
    }

    public func repos() async throws -> [RepoSummary] { try await get("/repos") }
    public func repoGithub() async throws -> [String: RepoGithub] { try await get("/repos/github") }
    public func agentsTotals() async throws -> AgentsTotals { try await get("/agents/summary", query: [URLQueryItem(name: "totals", value: "true")]) }
    public func settingsView() async throws -> JSONValue { try await get("/settings") }
    public func restart() async throws { _ = try await send("POST", "/restart", body: [:]) }
    public func githubBudget() async throws -> JSONValue { try await get("/github/budget") }
    public func macAccess() async throws -> JSONValue { try decode(JSONValue.self, try await send("POST", "/system/mac-access", body: [:], timeout: 60)) }
    public func repoCommits(name: String) async throws -> [RepoCommit] {
        try await get("/repos/\(name.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed.subtracting(["/"])) ?? name)/commits")
    }
    public func pushStatus() async throws -> JSONValue { try await get("/push") }
    public func configurePush(_ changes: [String: JSONValue]) async throws -> JSONValue { try decode(JSONValue.self, try await send("PUT", "/push", body: .object(changes))) }
    public func pushSignIn() async throws -> JSONValue { try decode(JSONValue.self, try await send("POST", "/push/signin", body: [:])) }
    public func pushDevices() async throws -> [JSONValue] { try await get("/devices") }
    public func herdrStatus() async throws -> JSONValue { try await get("/herdr") }
    public func herdrSetup() async throws -> JSONValue { try decode(JSONValue.self, try await send("POST", "/herdr/setup", body: [:], timeout: 300)) }
    public func commands(agentId: String) async throws -> [AgentCommand] { try await get("/agents/\(agentId)/commands") }

    public func eventLines() async throws -> AsyncThrowingStream<String, Error> {
        var req = URLRequest(url: url("/events"))
        req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
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
    public let report: @Sendable (Double) -> Void
    public init(_ report: @escaping @Sendable (Double) -> Void) { self.report = report }

    public func urlSession(_ session: URLSession, task: URLSessionTask, didSendBodyData bytesSent: Int64, totalBytesSent: Int64,
                    totalBytesExpectedToSend: Int64) {
        guard totalBytesExpectedToSend > 0 else { return }
        report(Double(totalBytesSent) / Double(totalBytesExpectedToSend))
    }
}

/// Defaults for stand-ins written before grill mode (test mocks): no meta, no sessions.
extension LoftAPI {
    public func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?, images: [String]) async throws -> SendMessageResponse {
        guard images.isEmpty else { throw APIError.unsupported("Videos") }
        return try await sendMessage(agentId: agentId, text: text, clientId: clientId, meta: meta)
    }
    public func upload(agentId: String, file: URL, progress: @escaping @Sendable (Double) -> Void) async throws -> String { throw APIError.unsupported("Videos") }
    public func uploadURL(agentId: String, file: String) -> URL {
        baseURL.appendingPathComponent("api/agents/\(agentId)/uploads/\(file)")
    }
    public func sendMessage(agentId: String, text: String, clientId: String, meta: JSONValue?) async throws -> SendMessageResponse {
        try await sendMessage(agentId: agentId, text: text, clientId: clientId)
    }
    public func grillSession(agentId: String) async throws -> GrillSession? { nil }
    public func grillCommand(sessionId: String, command: GrillCommand, questionId: String?, clientId: String) async throws -> GrillSession? {
        throw APIError.unsupported("Grill mode")
    }
    public func grillPhrases() async throws -> GrillPhrases { throw APIError.unsupported("Grill mode") }
    public func sendTeamSetup(agentId: String, messageId: Int64) async throws -> SendMessageResponse { throw APIError.unsupported("Team setup") }
    public func startDelivery(agentId: String, messageId: Int64, repo: String, spec: Int, initGit: Bool) async throws -> SendMessageResponse { throw APIError.unsupported("Delivery") }
    public func issueDelivery(taskId: String) async throws -> IssueDelivery? { nil }
    public func issueDeliveryAction(taskId: String, _ action: IssueDeliveryAction) async throws -> IssueDeliveryReply { throw APIError.unsupported("Delivery") }
    public func publishTask(id: String) async throws -> HBTask { throw APIError.unsupported("Publishing to GitHub") }
    public func issueOpened(id: String) async throws {}
    public func mergeDelivery(repo: String, spec: Int) async throws -> String { throw APIError.unsupported("Delivery") }
    public func updateAgent(id: String, _ fields: [String: JSONValue]) async throws -> Agent { throw APIError.unsupported("Agent settings") }
    public func workspaceSize(agentId: String) async throws -> Int64 { throw APIError.unsupported("Workspace size") }
    public func deleteAgent(id: String, purgeFiles: Bool) async throws { throw APIError.unsupported("Deleting agents") }
    public func taskComments(id: String) async throws -> [TaskComment] { [] }
    public func addTaskComment(id: String, text: String, clientId: String, github: Bool?) async throws { throw APIError.unsupported("Comments") }
    public func serverSettings() async throws -> JSONValue { throw APIError.unsupported("Server settings") }
    public func putServerSettings(_ settings: JSONValue) async throws -> JSONValue { throw APIError.unsupported("Server settings") }
    public func backups() async throws -> BackupStatus { throw APIError.unsupported("Backups") }
    public func backUpNow() async throws -> BackupStatus { throw APIError.unsupported("Backups") }
    public func workspaceSizes() async throws -> WorkspaceSizes { throw APIError.unsupported("Disk usage") }
    public func cleanPreview(agentId: String?) async throws -> [CleanItem] { throw APIError.unsupported("Disk usage") }
    public func cleanFolders(agentId: String, rel: String) async throws -> [CleanItem] { throw APIError.unsupported("Disk usage") }
    public func cleanCheck(agentId: String, rel: String) async throws -> GitCheck { throw APIError.unsupported("Disk usage") }
    public func clean(agentId: String?, paths: [String]) async throws -> CleanLog { throw APIError.unsupported("Disk usage") }
    public func cleanLog() async throws -> CleanLog { throw APIError.unsupported("Disk usage") }
    public func leftRunning() async throws -> [String: [LeftProcess]] { throw APIError.unsupported("Disk usage") }
    public func stopLeftRunning(agentId: String) async throws { throw APIError.unsupported("Disk usage") }
    public func agentDevices() async throws -> [AgentDevice] { throw APIError.unsupported("Disk usage") }
    public func approveComment(taskId: String, commentId: Int64, text: String?) async throws { throw APIError.unsupported("Comments") }
    public func keepComment(taskId: String, commentId: Int64) async throws { throw APIError.unsupported("Comments") }
    public func briefVoice(text: String) async throws -> VoiceBriefResponse { throw APIError.unsupported("Briefs") }
    public func taskPlan(id: String) async throws -> TaskPlan { throw APIError.unsupported("Plans") }
    public func needsYou() async throws -> NeedsYouList { throw APIError.unsupported("Needs you") }
    /// A "Needs you" action as the server describes it: `method` + `path` (with the /api prefix) + `body`.
    public func needsYouRequest(method: String, path: String, body: JSONValue?) async throws { throw APIError.unsupported("Needs you") }
    public func registerDevice(_ d: DeviceRegistration) async throws { throw APIError.unsupported("Push") }
    public func unregisterDevice(token: String) async throws { throw APIError.unsupported("Push") }
    public func testPush(token: String) async throws { throw APIError.unsupported("Push") }
    public func updateDevice(token: String, _ fields: [String: JSONValue]) async throws { throw APIError.unsupported("Push") }
    public func repos() async throws -> [RepoSummary] { throw APIError.unsupported("Repos") }
    public func repoGithub() async throws -> [String: RepoGithub] { throw APIError.unsupported("Repos") }
    public func agentsTotals() async throws -> AgentsTotals { throw APIError.unsupported("Spend") }
    public func settingsView() async throws -> JSONValue { throw APIError.unsupported("Server settings") }
    public func restart() async throws { throw APIError.unsupported("Restart") }
    public func githubBudget() async throws -> JSONValue { throw APIError.unsupported("GitHub API") }
    public func macAccess() async throws -> JSONValue { throw APIError.unsupported("Mac access") }
    public func repoCommits(name: String) async throws -> [RepoCommit] { throw APIError.unsupported("Repos") }
    public func pushStatus() async throws -> JSONValue { throw APIError.unsupported("Push") }
    public func configurePush(_ changes: [String: JSONValue]) async throws -> JSONValue { throw APIError.unsupported("Push") }
    public func pushSignIn() async throws -> JSONValue { throw APIError.unsupported("Push") }
    public func pushDevices() async throws -> [JSONValue] { throw APIError.unsupported("Push") }
    public func herdrStatus() async throws -> JSONValue { throw APIError.unsupported("Herdr UI") }
    public func herdrSetup() async throws -> JSONValue { throw APIError.unsupported("Herdr UI") }
    public func commands(agentId: String) async throws -> [AgentCommand] { [] }
}
