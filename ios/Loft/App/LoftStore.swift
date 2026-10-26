import Foundation
import Network
import Observation
import UserNotifications

enum ConnectionState: Equatable {
    case notConfigured
    case connecting
    /// Live: the event stream is open and a fresh snapshot was loaded.
    case connected
    /// Requests work but the event stream doesn't; data refreshes periodically.
    case polling
    case reconnecting(attempt: Int, error: APIError)

    var isUsable: Bool { self == .connected || self == .polling }

    var label: String {
        switch self {
        case .notConfigured: return "Not set up"
        case .connecting: return "Connecting…"
        case .connected: return "Connected"
        case .polling: return "Connected · no live updates"
        case .reconnecting(let attempt, let error):
            if case .server(let status, _) = error, status >= 500 { return "Server unavailable" }
            return attempt <= 1 ? "Reconnecting…" : "Disconnected"
        }
    }
}

/// The app's single source of client state. Everything here mirrors the server;
/// nothing is changed locally without the server confirming it.
@MainActor
@Observable
final class LoftStore {
    private(set) var api: LoftAPI?
    private(set) var connection: ConnectionState = .notConfigured
    private(set) var info: ServerInfo?
    private(set) var lastSync: Date?

    private(set) var agents: [Agent] = []
    private(set) var tasks: [HBTask] = []
    private(set) var permissions: [PermissionRequest] = []
    private(set) var activity: [ActivityItem] = []
    private(set) var activityHasMore = true
    /// Everything waiting on you, from GET /api/needs-you (servers with `needs_you`).
    private(set) var needsYou: NeedsYouList?
    /// A short note for the Needs you list ("Already done").
    var needsYouNotice: String?
    private(set) var pullRequests: [PullRequest] = []
    /// Plan views by parent task id, for the chat's plan cards. Kept here rather than in the card: the chat's lazy
    /// list drops a card's state when it scrolls out of view, and a card that comes back short and loads again moves
    /// the bottom-anchored list, which scrolls it out again, round and round.
    private(set) var plans: [String: TaskPlan] = [:]
    /// Claude plan usage; nil until loaded or when the server doesn't report it.
    private(set) var usage: PlanUsage?
    private(set) var usageLoadedAt: Date?
    /// The Mac's CPU temperature; nil until loaded or where the server can't read it.
    private(set) var system: SystemReading?
    /// Per-agent stats for the last 14 days, by agent id.
    private(set) var summaries: [String: AgentSummary] = [:]
    /// Loaded conversation history per agent, oldest first.
    private(set) var messages: [String: [ChatMessage]] = [:]
    private(set) var historyHasMore: [String: Bool] = [:]
    private(set) var pending: [PendingMessage] = []
    /// Open or recently ended "grill me" interviews, by agent id.
    private(set) var grillSessions: [String: GrillSession] = [:]
    /// The server's command phrases for on-device matching; the last list seen is kept for offline use.
    private(set) var grillPhrases: GrillPhrases? = {
        UserDefaults.standard.data(forKey: Prefs.grillPhrases).flatMap { try? JSONCoding.decoder.decode(GrillPhrases.self, from: $0) }
    }()

    var orchestratorId: String? {
        didSet { UserDefaults.standard.set(orchestratorId, forKey: Prefs.orchestratorId) }
    }

    let notifier: Notifier
    private let cache: SnapshotStore
    private var connectionTask: Task<Void, Never>?
    private var activityRefresh: Task<Void, Never>?
    private var statsTask: Task<Void, Never>?
    private var saveTask: Task<Void, Never>?
    private let pathMonitor = NWPathMonitor()
    private var lastPathSatisfied = true
    private(set) var isActive = false

    init(api: LoftAPI?, cache: SnapshotStore = .standard, notifier: Notifier = Notifier()) {
        self.cache = cache
        self.notifier = notifier
        self.orchestratorId = UserDefaults.standard.string(forKey: Prefs.orchestratorId)
        configure(api)
        pathMonitor.pathUpdateHandler = { [weak self] path in
            let ok = path.status == .satisfied
            Task { @MainActor in self?.networkChanged(satisfied: ok) }
        }
        pathMonitor.start(queue: .main)
    }

    // MARK: - Derived state

    var isLive: Bool { connection == .connected }
    var canWrite: Bool { connection.isUsable }
    var supportsTasks: Bool { info?.supports(.tasks) ?? false }
    var supportsGrill: Bool { info?.supports(.grill) ?? false }
    var supportsActivity: Bool { info?.supports(.activity) ?? false }

    // MARK: - Needs you

    var supportsNeedsYou: Bool { info?.supports(.needsYou) == true }

    func refreshNeedsYou() async {
        guard let api, supportsNeedsYou, let list = try? await api.needsYou() else { return }
        needsYou = list
        // The app icon's badge follows what needs you. Not awaited: refresh() runs this before the app counts
        // as connected, and a slow notification service (seen on a busy Simulator) held it at "Connecting…".
        let count = list.count
        Task.detached { try? await UNUserNotificationCenter.current().setBadgeCount(count) }
    }

    /// Runs a "Needs you" action (approve, deny, answer, retry, merge, restart, dismiss, skip), then refreshes.
    /// `text` is the user's words for answer and retry.
    /// Returns false when it had already been handled (404/409: answered elsewhere, or the item is gone): not an error.
    @discardableResult
    func perform(_ action: NeedsYouAction, text: String? = nil) async throws -> Bool {
        let api = try requireWrite()
        guard let method = action.method, let path = action.path else { return true }
        do {
            try await api.needsYouRequest(method: method, path: path, body: action.body(text: text, clientId: UUID().uuidString))
        } catch APIError.server(let status, _) where status == 404 || status == 409 {
            await refreshNeedsYou()
            return false
        }
        await refreshNeedsYou()
        return true
    }

    func agent(_ id: String?) -> Agent? { id.flatMap { id in agents.first { $0.id == id } } }
    func task(_ id: String?) -> HBTask? { id.flatMap { id in tasks.first { $0.id == id } } }

    /// Loads a plan; the card shows the last one it got until then. Unchanged plans aren't stored again.
    func loadPlan(_ taskId: String) async {
        guard let api, let p = try? await api.taskPlan(id: taskId), plans[taskId] != p else { return }
        plans[taskId] = p
    }

    var orchestrator: Agent? { agent(orchestratorId) }

    func pendingApprovals(for agentId: String? = nil) -> [PermissionRequest] {
        permissions.filter { $0.isPending && (agentId == nil || $0.agentId == agentId) }
    }

    func state(of agent: Agent) -> AgentState {
        AgentState(agent: agent, pendingApprovals: pendingApprovals(for: agent.id).count)
    }

    /// The task the agent is on now, if one was handed to it through Loft.
    func currentTask(for agentId: String) -> HBTask? {
        tasks.filter { $0.agentId == agentId && $0.state.isActive }
            .max { ($0.startedAt ?? $0.updatedAt) < ($1.startedAt ?? $1.updatedAt) }
    }

    func subtasks(of taskId: String) -> [HBTask] {
        tasks.filter { $0.parentId == taskId }.sorted { $0.createdAt < $1.createdAt }
    }

    var sortedAgents: [Agent] {
        agents.sorted {
            let a = state(of: $0).rank, b = state(of: $1).rank
            return a != b ? a < b : $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
    }

    var projects: [String] {
        Array(Set(agents.compactMap(\.project) + tasks.map(\.project)).filter { !$0.isEmpty })
            .sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    }

    func conversation(_ agentId: String) -> [ChatMessage] { messages[agentId] ?? [] }
    func pending(for agentId: String) -> [PendingMessage] { pending.filter { $0.agentId == agentId } }

    // MARK: - Configuration & lifecycle

    func configure(_ api: LoftAPI?) {
        connectionTask?.cancel()
        connectionTask = nil
        self.api = api
        resetData()
        guard let api else {
            connection = .notConfigured
            return
        }
        if let snap = cache.load(for: api.baseURL.absoluteString) {
            restore(snap)
        }
        connection = .connecting
        if isActive { startConnection() }
    }

    /// The app came to the foreground.
    func activate() {
        isActive = true
        if api != nil && connectionTask == nil { startConnection() }
    }

    /// The app went to the background: drop the stream (iOS would suspend it anyway) and save.
    func deactivate() {
        isActive = false
        connectionTask?.cancel()
        connectionTask = nil
        saveNow()
    }

    /// Drop the current attempt and connect again right away.
    func reconnect() {
        connectionTask?.cancel()
        connectionTask = nil
        if api != nil { startConnection() }
    }

    private func networkChanged(satisfied: Bool) {
        defer { lastPathSatisfied = satisfied }
        // Network came back (e.g. Tailscale reconnected): don't wait out the backoff.
        if satisfied && !lastPathSatisfied, isActive, case .reconnecting = connection {
            Log.sync.info("network is back; reconnecting")
            reconnect()
        }
    }

    private func startConnection() {
        guard let api else { return }
        connectionTask = Task { [weak self] in
            var backoff = Backoff()
            var failures = 0
            while !Task.isCancelled {
                guard let self else { return }
                do {
                    let lines = try await api.eventLines()
                    // The stream is open, so nothing after this point is missed. There is no replay of
                    // what happened while disconnected, so load a fresh snapshot rather than trust the cache.
                    try await self.refresh()
                    self.connection = .connected
                    backoff.reset()
                    failures = 0
                    var parser = SSEParser()
                    for try await line in lines {
                        if let data = parser.feed(line) { self.apply(eventData: data) }
                    }
                    throw APIError.unreachable("The event stream closed")
                } catch {
                    if Task.isCancelled { return }
                    let e = APIError.from(error)
                    if e == .cancelled { return }
                    failures += 1
                    Log.sync.error("connection attempt \(failures) failed: \(e.diagnostic, privacy: .public)")
                    // Requests may still work when only the stream is blocked (e.g. a proxy buffering SSE).
                    if failures >= 2, (try? await self.refresh()) != nil {
                        self.connection = .polling
                    } else {
                        self.connection = .reconnecting(attempt: failures, error: e)
                    }
                    try? await Task.sleep(for: .seconds(backoff.next()))
                }
            }
        }
    }

    // MARK: - Sync

    /// Load authoritative state from the server.
    func refresh() async throws {
        guard let api else { throw APIError.notConfigured }
        let info = try await api.info()
        async let agents = api.agents()
        async let permissions = api.permissions()
        async let prs = api.pullRequests()
        let tasks: [HBTask]? = info.supports(.tasks) ? try await api.tasks() : nil
        let activity: [ActivityItem]? = info.supports(.activity) ? try await api.activity(limit: 50, before: nil) : nil
        self.info = info
        self.agents = try await agents
        self.permissions = try await permissions
        self.pullRequests = (try? await prs) ?? self.pullRequests
        self.tasks = tasks ?? []
        if let activity {
            self.activity = activity
            self.activityHasMore = activity.count >= 50
        }
        lastSync = Date()
        refreshStatsSoon()
        if orchestratorId == nil || agent(orchestratorId) == nil {
            orchestratorId = Self.suggestOrchestrator(self.agents)?.id ?? orchestratorId
        }
        // Interviews are started from the Orchestrator chat; restore its state after a reconnect.
        if let o = orchestratorId { await refreshGrill(o) }
        if info.supports(.grill) { await loadGrillPhrases() }
        if info.supports(.needsYou) { await refreshNeedsYou() }
        scheduleSave()
    }

    /// An agent whose name says it coordinates the others. None when nothing fits: the user picks one.
    static func suggestOrchestrator(_ agents: [Agent]) -> Agent? {
        agents.first { $0.name.localizedCaseInsensitiveContains("orchestrator") }
    }

    /// Plan usage and per-agent stats. Not part of `refresh`: both are slower (usage asks Anthropic),
    /// and a failure here shouldn't make the connection look broken.
    func refreshStats() async {
        guard let api else { return }
        async let u = try? api.usage()
        async let s = try? api.agentSummaries()
        async let sys: SystemReading? = info?.supports(.system) == true ? (try? api.system()) : nil
        if let u = await u, u.windows.isEmpty == false || u.error != nil {
            usage = u
            usageLoadedAt = Date()
        }
        if let sys = await sys { system = sys }
        if let s = await s {
            summaries = Dictionary(s.map { ($0.agent.id, $0) }, uniquingKeysWith: { a, _ in a })
        }
    }

    /// Debounced, and at most every two minutes like the web UI's meter.
    private func refreshStatsSoon() {
        if let at = usageLoadedAt, Date().timeIntervalSince(at) < 120, !summaries.isEmpty { return }
        statsTask?.cancel()
        statsTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            await self?.refreshStats()
        }
    }

    /// Totals across agents for the Overview.
    var totals: (runs: Int, toolCalls: Int, prsMerged: Int, prsOpen: Int, daily: [(auto: Int, user: Int)]) {
        let all = Array(summaries.values)
        let days = all.map(\.days).max() ?? 14
        let daily = (0..<days).map { i in
            (all.reduce(0) { $0 + ($1.dailyAuto.indices.contains(i) ? $1.dailyAuto[i] : 0) },
             all.reduce(0) { $0 + ($1.dailyUser.indices.contains(i) ? $1.dailyUser[i] : 0) })
        }
        return (all.reduce(0) { $0 + $1.runs }, all.reduce(0) { $0 + $1.toolCalls },
                all.reduce(0) { $0 + $1.prs.merged }, all.reduce(0) { $0 + $1.prs.open }, daily)
    }

    func loadMoreActivity() async {
        guard let api, supportsActivity, activityHasMore, let last = activity.last else { return }
        do {
            let more = try await api.activity(limit: 50, before: last.at)
            let known = Set(activity.map(\.id))
            activity += more.filter { !known.contains($0.id) }
            activityHasMore = more.count >= 50
        } catch {
            Log.sync.error("activity paging failed: \(APIError.from(error).diagnostic, privacy: .public)")
        }
    }

    private func refreshActivitySoon() {
        guard supportsActivity else { return }
        activityRefresh?.cancel()
        activityRefresh = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1.5))
            guard let self, let api = self.api, !Task.isCancelled else { return }
            if let fresh = try? await api.activity(limit: 50, before: nil) {
                let freshIds = Set(fresh.map(\.id))
                let older = self.activity.filter { !freshIds.contains($0.id) && $0.at < (fresh.last?.at ?? .distantPast) }
                self.activity = fresh + older
            }
        }
    }

    // MARK: - Events

    func apply(eventData: String) {
        let event: ServerEvent
        do {
            event = try ServerEvent.decode(Data(eventData.utf8))
        } catch {
            Log.sync.error("bad event: \(String(describing: error), privacy: .public)")
            return
        }
        apply(event)
    }

    func apply(_ event: ServerEvent) {
        switch event {
        case .agentUpdated(let a):
            let before = agent(a.id)
            upsert(&agents, a)
            if let before, before.status != "error", a.status == "error" {
                notifier.post(.failure, title: "\(a.name)'s run failed", body: "Open Loft to see what happened.", whenActive: isActive)
            }
            if before?.status != a.status { refreshActivitySoon() }
        case .agentDeleted(let id):
            agents.removeAll { $0.id == id }
            messages[id] = nil
        case .message(let m):
            insert(m)
            if ["run_start", "run_end", "error", "permission"].contains(m.kind) || m.fromAgentName != nil {
                refreshActivitySoon()
            }
        case .permission(let p):
            let wasPending = permissions.first { $0.id == p.id }?.isPending ?? false
            upsert(&permissions, p)
            if p.isPending && !wasPending {
                let name = agent(p.agentId)?.name ?? "An agent"
                notifier.post(.approval, title: "\(name) needs approval", body: p.toolName, whenActive: isActive,
                              itemId: NeedsYouKey.permission(p.id), target: ["screen": "chat", "agent_id": p.agentId])
            }
        case .pullRequest(let pr):
            upsert(&pullRequests, pr)
        case .task(let t):
            let before = task(t.id)
            upsert(&tasks, t)
            if let before, before.status != t.status {
                switch t.state {
                case .done: notifier.post(.taskOutcome, title: "Task done", body: t.title, whenActive: isActive,
                                          target: ["screen": "task", "task_id": t.id])
                case .failed: notifier.post(.taskOutcome, title: "Task failed", body: t.title, whenActive: isActive,
                                            itemId: NeedsYouKey.task(t.id, "failed"), target: ["screen": "task", "task_id": t.id])
                case .blocked: notifier.post(.taskOutcome, title: "Task blocked", body: t.title, whenActive: isActive,
                                             itemId: NeedsYouKey.task(t.id, "blocked"), target: ["screen": "task", "task_id": t.id])
                default: break
                }
            }
            refreshActivitySoon()
        case .taskDeleted(let id):
            tasks.removeAll { $0.id == id }
        case .system(let r):
            system = r
        case .grill(let s):
            if let agentId = s.agentId ?? grillAgent(for: s.sessionId) { grillSessions[agentId] = s }
        case .needsYou(_, let ids):
            if ids != needsYou?.items.map(\.id) { Task { await refreshNeedsYou() } }
        case .other:
            break
        }
        scheduleSave()
    }

    private func upsert<T: Identifiable>(_ list: inout [T], _ item: T) {
        if let i = list.firstIndex(where: { $0.id == item.id }) {
            list[i] = item
        } else {
            list.append(item)
        }
    }

    /// Add a message once, in order. Clears the pending copy it confirms.
    private func insert(_ m: ChatMessage) {
        if let cid = m.clientId { pending.removeAll { $0.id == cid } }
        // Only track history for conversations that were opened; others load fresh when opened.
        guard var list = messages[m.agentId] else { return }
        guard !list.contains(where: { $0.id == m.id }) else { return }
        if let last = list.last, last.id > m.id {
            list.append(m)
            list.sort { $0.id < $1.id }
        } else {
            list.append(m)
        }
        if list.count > 600 { list.removeFirst(list.count - 600) }
        messages[m.agentId] = list
    }

    // MARK: - Conversations

    func loadConversation(_ agentId: String) async throws {
        guard let api else { throw APIError.notConfigured }
        let page = try await api.messages(agentId: agentId, limit: 100, before: nil)
        let existing = messages[agentId] ?? []
        let ids = Set(page.map(\.id))
        let firstNew = page.first?.id ?? .max
        // Keep older pages already loaded; the fresh page wins for anything it covers.
        messages[agentId] = existing.filter { $0.id < firstNew && !ids.contains($0.id) } + page
        if historyHasMore[agentId] == nil { historyHasMore[agentId] = page.count >= 100 }
        page.compactMap(\.clientId).forEach { cid in pending.removeAll { $0.id == cid } }
        scheduleSave()
    }

    func loadOlder(_ agentId: String) async throws {
        guard let api, let first = messages[agentId]?.first else { return }
        let page = try await api.messages(agentId: agentId, limit: 100, before: first.id)
        messages[agentId] = page + (messages[agentId] ?? [])
        historyHasMore[agentId] = page.count >= 100
    }

    func send(_ text: String, to agentId: String, meta: JSONValue? = nil) async {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        let p = PendingMessage(id: UUID().uuidString, agentId: agentId, text: text, createdAt: Date(), meta: meta)
        pending.append(p)
        await deliverUncancelled(p)
    }

    /// A send runs in its own task, so it never inherits the caller's cancellation: the mic turning off,
    /// a countdown finishing or a view going away mustn't turn a send into "Not sent: Cancelled".
    private func deliverUncancelled(_ p: PendingMessage) async {
        await Task { await deliver(p) }.value
    }

    /// The agent's text replies after message `id`, joined, once its run is over. Nil when
    /// `keepWaiting` turns false (the conversation was left or stopped) or after an hour.
    func waitForReply(agentId: String, after id: Int64, while keepWaiting: () -> Bool) async -> String? {
        var sawRun = false
        var lastCount = 0
        var settledTicks = 0
        for tick in 0..<(60 * 60 * 2) { // checked every half second
            guard keepWaiting() else { return nil }
            let a = agent(agentId)
            let running = a?.running == true || a?.status == "running"
            sawRun = sawRun || running
            let replies = conversation(agentId).filter { $0.id > id && $0.role == "assistant" && $0.kind == "text" }
            // A question from an interview comes straight from the server, without a run to wait for.
            let question = replies.contains { $0.grill?.role == .question }
            // When the run was never seen (its update missed), the reply counts once nothing new has come for 2 s,
            // instead of waiting a fixed 10 s, which was long enough to leave the chat and lose the reading.
            settledTicks = replies.count == lastCount ? settledTicks + 1 : 0
            lastCount = replies.count
            if !replies.isEmpty && (sawRun || question || settledTicks >= 4 || tick > 20) && !running {
                return replies.map(\.displayContent).joined(separator: "\n\n")
            }
            // Fetch history now and then, live or not: a message event missed during a reconnect or while in the
            // background would otherwise leave this waiting (and the reply unread) for an hour. The first fetch
            // comes after a second, so a quick reply whose event was missed isn't held back for five.
            if tick == 2 || tick % 10 == 9 { try? await loadConversation(agentId) }
            try? await Task.sleep(for: .milliseconds(500))
        }
        return nil
    }

    /// A dictation tidied up on the Mac (punctuation, filler words, obvious misrecognitions; same meaning and
    /// language) before it's sent. Falls back to the original when cleanup isn't available, fails or takes
    /// too long, so a dictation is never lost. Returns the text and what happened, for the voice log.
    /// "Make it a brief" is offered from this many words (the server answers "already short" below it).
    nonisolated static let briefMinWords = 80
    static let briefTimeout: TimeInterval = 20

    var supportsBrief: Bool { info?.supports(.voiceBrief) == true }

    /// A long dictation as a brief from the Mac, or nil and a note for the composer ("Already short", "Couldn't
    /// make a brief, kept your words"). The original is never lost: the composer keeps it either way.
    func brief(_ text: String) async -> (brief: String?, note: String, log: String) {
        guard let api, supportsBrief, canWrite else { return (nil, "Couldn't make a brief, kept your words", "not available here") }
        do {
            let r = try await api.briefVoice(text: text)
            let b = r.text.trimmingCharacters(in: .whitespacesAndNewlines)
            if r.briefed, !b.isEmpty { return (b, "", "brief by the Mac") }
            let why = r.reason ?? "no reason"
            return (nil, why == "already short" ? "Already short" : "Couldn't make a brief, kept your words",
                    why + (r.added.map { " (\($0))" } ?? ""))
        } catch {
            return (nil, "Couldn't make a brief, kept your words", APIError.from(error).localizedDescription)
        }
    }

    func cleanDictation(_ text: String) async -> (String, String) {
        guard let api, info?.supports(.voiceClean) == true, canWrite else { return (text, "not available here; sent as said") }
        let result = await withTaskGroup(of: VoiceCleanResponse?.self) { group in
            group.addTask { try? await api.cleanVoice(text: text) }
            group.addTask { try? await Task.sleep(for: .seconds(8)); return nil }
            let first = await group.next() ?? nil
            group.cancelAll()
            return first
        }
        guard let r = result, r.cleaned, !r.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return (text, result == nil ? "failed or timed out; sent as said" : "nothing to change")
        }
        return (r.text, "cleaned \(VoiceLog.clip(text)) → \(VoiceLog.clip(r.text))")
    }

    /// Retry a failed send. The same client id goes up, so the server drops it if the first try did land.
    func retry(_ p: PendingMessage) async {
        guard let i = pending.firstIndex(where: { $0.id == p.id }) else { return }
        pending[i].failure = nil
        await deliverUncancelled(pending[i])
    }

    func discard(_ p: PendingMessage) {
        pending.removeAll { $0.id == p.id }
        scheduleSave()
    }

    private func deliver(_ p: PendingMessage) async {
        do {
            guard canWrite else { throw APIError.offline }
            guard let api else { throw APIError.notConfigured }
            let r = try await api.sendMessage(agentId: p.agentId, text: p.text, clientId: p.id, meta: p.meta)
            if let m = r.message { insert(m) }
            pending.removeAll { $0.id == p.id }
        } catch {
            let e = APIError.from(error)
            if let i = pending.firstIndex(where: { $0.id == p.id }) { pending[i].failure = e.title }
        }
        scheduleSave()
    }

    // MARK: - Grill mode

    /// The agent's interview while it's preparing, active or paused.
    func openGrill(for agentId: String) -> GrillSession? {
        grillSessions[agentId].flatMap { $0.isOpen ? $0 : nil }
    }

    /// The question waiting for an answer: the session's current one, else the newest question in the
    /// session (the session event can arrive before or after the message). Nil once it's answered,
    /// including an answer still being sent, until the next question arrives.
    func currentGrillQuestion(for agentId: String) -> ChatMessage? {
        guard let s = openGrill(for: agentId), let list = messages[agentId] else { return nil }
        let inSession = list.filter { $0.grill?.sessionId == s.sessionId }
        let questions = inSession.filter { $0.grill?.role == .question }
        guard let q = questions.last(where: { $0.grill?.questionId == s.current?.questionId }) ?? questions.last,
              let qid = q.grill?.questionId else { return nil }
        let answered = inSession.contains { $0.id > q.id && $0.grill?.role == .answer && $0.grill?.questionId == qid }
            || pending.contains { $0.agentId == agentId && $0.meta?["grill"]?["question_id"]?.stringValue == qid && !$0.isFailed }
        return answered ? nil : q
    }

    func refreshGrill(_ agentId: String) async {
        guard supportsGrill, let api else { return }
        do {
            if let s = try await api.grillSession(agentId: agentId) {
                grillSessions[agentId] = s
            } else {
                grillSessions[agentId] = nil
            }
        } catch {
            // Keep what we had; the stale banner already says the data may be old.
            Log.sync.error("grill state: \(String(describing: error), privacy: .public)")
        }
    }

    /// Sends an answer tagged with the question it's for, through the normal (retryable) send queue.
    func answerGrill(_ text: String, to question: ChatMessage) async {
        guard let q = question.grill, q.role == .question else { return await send(text, to: question.agentId) }
        await send(text, to: question.agentId, meta: GrillMeta.answer(to: q))
        await catchUpGrill(question.agentId)
    }

    /// Skip, back, pause, resume, end, summarize or decide. The server confirms with a `status` message.
    func sendGrillCommand(_ command: GrillCommand, agentId: String) async throws {
        let api = try requireWrite()
        guard let s = openGrill(for: agentId) else { throw APIError.unsupported("No interview in progress") }
        let updated = try await api.grillCommand(sessionId: s.sessionId, command: command,
                                                 questionId: s.current?.questionId ?? currentGrillQuestion(for: agentId)?.grill?.questionId,
                                                 clientId: UUID().uuidString)
        if let updated { grillSessions[agentId] = updated }
        await catchUpGrill(agentId)
    }

    /// The next question and status rows normally arrive as events; fetch them anyway (it's one small
    /// request per answer) so a dropped event or a polling connection never leaves the chat waiting.
    private func catchUpGrill(_ agentId: String) async {
        await refreshGrill(agentId)
        try? await loadConversation(agentId)
    }

    /// Fetches the phrase list once per launch (it only changes with server updates) and caches it.
    func loadGrillPhrases() async {
        guard let api, supportsGrill, !grillPhrasesLoaded else { return }
        guard let p = try? await api.grillPhrases() else { return }
        grillPhrasesLoaded = true
        guard p != grillPhrases else { return }
        grillPhrases = p
        if let data = try? JSONCoding.encoder.encode(p) { UserDefaults.standard.set(data, forKey: Prefs.grillPhrases) }
    }

    @ObservationIgnored private var grillPhrasesLoaded = false

    private func grillAgent(for sessionId: String) -> String? {
        if let hit = grillSessions.first(where: { $0.value.sessionId == sessionId }) { return hit.key }
        return messages.first { $0.value.contains { $0.grill?.sessionId == sessionId } }?.key
    }

    // MARK: - Actions (each waits for the server and applies its answer)

    private func requireWrite() throws -> LoftAPI {
        guard let api else { throw APIError.notConfigured }
        guard canWrite else { throw APIError.offline }
        return api
    }

    private func requireTasks() throws -> LoftAPI {
        let api = try requireWrite()
        guard supportsTasks else { throw APIError.unsupported("Tasks") }
        return api
    }

    func decide(_ p: PermissionRequest, decision: String) async throws {
        try await requireWrite().decidePermission(id: p.id, decision: decision)
        // The server emits the updated request; mark it locally only once it confirmed.
        if let i = permissions.firstIndex(where: { $0.id == p.id }), permissions[i].isPending {
            permissions[i].status = decision == "deny" ? "denied" : (decision == "allow_always" ? "allowed_always" : "allowed_once")
        }
    }

    func stopRun(_ agentId: String) async throws { try await requireWrite().stopRun(agentId: agentId) }
    func restartRun(_ agentId: String) async throws {
        try await requireWrite().restartRun(agentId: agentId)
        // The agent_updated event normally follows; fetch anyway so the bar clears without a live stream.
        if let list = try? await api?.agents() { agents = list }
    }
    func runNow(_ agentId: String) async throws { try await requireWrite().runNow(agentId: agentId) }

    @discardableResult
    func createTask(_ fields: [String: JSONValue]) async throws -> HBTask {
        let t = try await requireTasks().createTask(fields)
        upsert(&tasks, t)
        return t
    }

    @discardableResult
    func updateTask(_ id: String, _ fields: [String: JSONValue]) async throws -> HBTask {
        let t = try await requireTasks().updateTask(id: id, fields)
        upsert(&tasks, t)
        return t
    }

    func deleteTask(_ id: String) async throws {
        try await requireTasks().deleteTask(id: id)
        tasks.removeAll { $0.id == id }
    }

    @discardableResult
    func executeTask(_ id: String, agentId: String?, note: String = "") async throws -> ExecuteResponse {
        let r = try await requireTasks().executeTask(id: id, agentId: agentId, note: note)
        upsert(&tasks, r.task)
        return r
    }

    /// Ask the orchestrator to plan a task and hand its parts to other agents.
    @discardableResult
    func delegateTask(_ id: String) async throws -> ExecuteResponse {
        guard let o = orchestrator else { throw APIError.server(status: 400, message: "Choose an orchestrator agent in Settings first.") }
        return try await executeTask(id, agentId: o.id, note: """
        Plan this task. Break it into subtasks with `mcp__loft__create_task` (parent_id set to this task, \
        depends_on where order matters) and hand each to the agent best suited for it with `start: true`. \
        Do parts yourself only when no other agent fits. Mark this task done when the subtasks are finished.
        """)
    }

    func cancelTask(_ id: String, stopRun: Bool) async throws {
        let r = try await requireTasks().cancelTask(id: id, stopRun: stopRun)
        upsert(&tasks, r.task)
    }

    // MARK: - Persistence

    private func resetData() {
        info = nil; lastSync = nil
        agents = []; tasks = []; permissions = []; activity = []; pullRequests = []
        usage = nil; usageLoadedAt = nil; summaries = [:]; system = nil
        messages = [:]; historyHasMore = [:]; pending = []
    }

    private func restore(_ s: Snapshot) {
        info = s.info
        lastSync = s.savedAt
        agents = s.agents
        tasks = s.tasks
        permissions = s.permissions
        activity = s.activity
        pullRequests = s.pullRequests
        messages = s.messages
        pending = s.unsent.map { var p = $0; p.failure = p.failure ?? "Not sent"; return p }
    }

    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            self?.saveNow()
        }
    }

    func saveNow() {
        guard let api, let lastSync else { return }
        cache.save(Snapshot(
            server: api.baseURL.absoluteString,
            savedAt: lastSync,
            info: info,
            agents: agents,
            tasks: tasks,
            permissions: permissions.filter { $0.isPending || $0.createdAt > Date().addingTimeInterval(-86400) },
            activity: Array(activity.prefix(100)),
            pullRequests: pullRequests,
            messages: messages.mapValues { Array($0.suffix(100)) },
            unsent: pending
        ))
    }

    func forgetServer() {
        // That Mac stops pushing to this phone (the request goes out before the address is dropped).
        let push = PushRegistrar.shared
        if push.registered, let api, let token = push.token { Task { try? await api.unregisterDevice(token: token) } }
        cache.clear()
        UserDefaults.standard.removeObject(forKey: Prefs.serverURL)
        configure(nil)
    }
}
