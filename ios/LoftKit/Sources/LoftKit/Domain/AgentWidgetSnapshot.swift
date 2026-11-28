import Foundation

/// What the One agent widget shows (spec #462): the picked agent's sticker, name and status, the ticket it's on, its
/// last message and its run time.
public struct AgentWidgetSnapshot: Equatable {
    public enum Phase: Equatable {
        /// The app hasn't connected to a Loft yet.
        case notConnected
        /// Connected, but Loft didn't answer.
        case unreachable
        /// Nobody picked an agent yet.
        case notChosen
        /// The picked agent was deleted.
        case gone
        case ready
    }

    /// The last message is cut to this many characters (the widget also cuts it to its lines).
    public static let messageLimit = 140

    public var phase: Phase
    public var agentId = ""
    public var name = ""
    public var avatarSVG: String?
    public var state = AgentState.unknown
    /// "#465 One-agent widget": the issue it's working on, nil when none.
    public var ticket: String?
    /// Its last reply on one line, cut to `messageLimit`; nil when it hasn't said anything yet.
    public var lastMessage: String?
    /// While it works: when the run started, for a live timer.
    public var runStart: Date?
    /// When it doesn't: "Done in 2m 23s · Thu 16:23" for its last run, nil when unknown.
    public var lastRun: String?

    public init(phase: Phase, agentId: String = "", name: String = "") {
        self.phase = phase
        self.agentId = agentId
        self.name = name
    }

    /// `name` is what the widget's configuration remembers, shown when the agent is gone.
    public init(agentId: String, name: String, agents: [Agent], tasks: [HBTask], messages: [ChatMessage], permissions: [PermissionRequest]) {
        guard let agent = agents.first(where: { $0.id == agentId }) else {
            self.init(phase: .gone, agentId: agentId, name: name)
            return
        }
        self.init(phase: .ready, agentId: agentId, name: agent.name)
        avatarSVG = agent.avatar?.svg
        state = AgentState(agent: agent, pendingApprovals: permissions.filter { $0.isPending && $0.agentId == agentId }.count)
        ticket = tasks.filter { $0.agentId == agentId && $0.status == "in_progress" }.max { $0.updatedAt < $1.updatedAt }
            .map { t in t.github.map { "#\($0.number) \(t.title)" } ?? t.title }
        // Its own words: not another agent's reply copied into its chat.
        lastMessage = messages.last { $0.role == "assistant" && $0.kind == "text" && $0.fromAgentId == nil
            && !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .map { Self.oneLine($0.content) }
        if state == .working {
            runStart = agent.lastRunAt
        } else {
            lastRun = RunTimer.lastFinished(messages).map { RunTimer.done($0.duration, endedAt: $0.endedAt) }
        }
    }

    /// The text's words on one line without bold or code marks, cut to `messageLimit` with an ellipsis.
    static func oneLine(_ text: String) -> String {
        let words = text.replacingOccurrences(of: "**", with: "").replacingOccurrences(of: "`", with: "")
            .split(whereSeparator: \.isWhitespace).joined(separator: " ")
        return words.count > messageLimit ? String(words.prefix(messageLimit - 1)) + "…" : words
    }

    /// Reads the agent from Loft at the address the app connected to.
    public static func load(agentId: String?, name: String) async -> AgentWidgetSnapshot {
        guard let server = SharedServer.address else { return AgentWidgetSnapshot(phase: .notConnected) }
        guard let agentId else { return AgentWidgetSnapshot(phase: .notChosen) }
        let api = LiveLoftAPI(baseURL: server)
        return await load(agentId: agentId, name: name, agents: api.agents, tasks: api.tasks,
                          messages: { try await api.messages(agentId: agentId, limit: 50, before: nil) }, permissions: api.permissions,
                          names: .standard)
    }

    /// The agents are required; the tasks, messages and permissions are extras, so one failing leaves its line out
    /// instead of making Loft look unreachable.
    static func load(agentId: String, name: String, agents: () async throws -> [Agent], tasks: () async throws -> [HBTask],
                     messages: () async throws -> [ChatMessage], permissions: () async throws -> [PermissionRequest],
                     names: UserDefaults? = nil) async -> AgentWidgetSnapshot {
        // The widget remembers each agent's name it showed, since the configuration loses it once the agent is deleted.
        let key = "agentWidget.name.\(agentId)"
        let name = name.isEmpty ? names?.string(forKey: key) ?? "" : name
        do {
            async let list = agents()
            async let issues = try? tasks()
            async let chat = try? messages()
            async let asks = try? permissions()
            let s = AgentWidgetSnapshot(agentId: agentId, name: name, agents: try await list, tasks: await issues ?? [],
                                        messages: await chat ?? [], permissions: await asks ?? [])
            if s.phase == .ready { names?.set(s.name, forKey: key) }
            return s
        } catch {
            return AgentWidgetSnapshot(phase: .unreachable, agentId: agentId, name: name)
        }
    }
}
