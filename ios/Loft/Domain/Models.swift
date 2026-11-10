import Foundation

// Swift mirrors of the Loft backend's JSON (backend/src/db.rs, tasks.rs).
// The backend sends snake_case keys; `JSONCoding.decoder` converts them.

struct Agent: Codable, Identifiable, Hashable {
    let id: String
    var name: String
    var title: String?
    var project: String?
    var model: String?
    var effort: String?
    var permissionMode: String?
    var intervalMinutes: Int?
    /// Raw backend status: idle | running | waiting | error.
    var status: String
    var statusText: String?
    /// Whether a Claude process is running for this agent right now.
    var running: Bool?
    var lastRunAt: Date?
    var nextRunAt: Date?
    var createdAt: Date?
    var totalCostUsd: Double?
    var contextTokens: Int?
    var prompt: String?
    /// The agent's sticker: a 100x100 SVG drawing (older servers don't send it).
    var avatar: Avatar?
    /// What it does in its team: lead, dev, code_reviewer, qa, security or custom (nil from older servers).
    var role: String? = nil
    /// When Claude last produced anything in the current run; nil when not running (servers with `run_watchdog`).
    var lastOutputAt: Date? = nil
    /// Background tasks the agent is waiting on after it already replied (e.g. "Run the QA suite").
    var waitingOn: [String]? = nil
    var waitingSince: Date? = nil
    /// The Claude Code Session its next run resumes; nil after Clear (and from older servers).
    var sessionId: String? = nil
    /// Fresh context for each Delivery ticket: "off", "above_80k" … "above_200k" or "always" (nil from older servers).
    var freshContext: String? = nil
    /// Whether that setting applies: to every agent but a Lead that only coordinates (its team has a Dev).
    var freshContextApplies: Bool? = nil
    /// Loft's own agent, which can't be deleted (nil from older servers).
    var builtin: Bool? = nil
    /// Free-text name of its team ("Acme"); empty when it has no team (older servers don't send it).
    var group: String? = nil
    /// The projects it works in, main first; `project` is the main one (older servers send only `project`).
    var projects: [String]? = nil

    /// Leads its team; it wears a captain hat.
    var isLead: Bool { role == "lead" }

    /// Its team: the group; empty when it has none (backend `Agent::team`).
    var team: String { (group ?? "").trimmingCharacters(in: .whitespaces) }

    /// The projects it works in, main first: `projects`, else the one `project` from an older server.
    var allProjects: [String] {
        if let projects, !projects.isEmpty { return projects }
        return (project ?? "").isEmpty ? [] : [project!]
    }
}

/// One of an agent's past Sessions, from Claude Code's transcripts (`/agents/{id}/sessions`).
struct PastSession: Codable, Identifiable, Hashable {
    let id: String
    var startedAt: Date
    var firstMessage: String
    /// The agent that sent `firstMessage`, when another agent started the Session.
    var fromAgent: String?
    var contextTokens: Int?
}

/// An agent's past Sessions, newest first, and which one is its Session now.
struct PastSessions: Codable, Hashable {
    var sessions: [PastSession]
    var current: String?
}

struct Avatar: Codable, Hashable {
    var svg: String
}

struct ChatMessage: Codable, Identifiable, Hashable {
    let id: Int64
    let agentId: String
    var runId: String?
    /// user | assistant | system
    var role: String
    /// text | thinking | tool_use | tool_result | permission | error | notice | run_start | run_end | clear | compact | resume
    var kind: String
    var content: String
    var meta: JSONValue?
    var createdAt: Date

    /// Set when another agent sent this message rather than the user.
    var fromAgentName: String? { meta?["from_agent"]?["name"]?.stringValue }
    var fromAgentId: String? { meta?["from_agent"]?["id"]?.stringValue }
    /// An FYI from another agent: shown at once, read on the agent's next Run, without waking it (#112).
    var isFYI: Bool { role == "user" && fromAgentName != nil && meta?["fyi"] == .bool(true) }
    /// Another agent's reply to an @-mention, copied into this chat (#92).
    var isMentionReply: Bool { meta?["mention_reply"] == .bool(true) }
    /// The agents your message @-mentioned, and whether it went only to them (the chat's agent wasn't woken).
    var mentionedNames: [String] {
        guard case .array(let a)? = meta?["mentions"] else { return [] }
        return a.compactMap { $0["name"]?.stringValue }
    }
    var mentionsOnly: Bool { meta?["mentions_only"] == .bool(true) }
    /// The specs, issues and pull requests your message tagged with `#` that Loft knows (`meta.tags`).
    var tags: [MessageTag] {
        guard case .array(let a)? = meta?["tags"] else { return [] }
        return a.compactMap { t in
            guard let tag = t["tag"]?.stringValue, let kind = t["kind"]?.stringValue, let title = t["title"]?.stringValue else { return nil }
            return MessageTag(tag: tag, kind: kind, title: title, url: t["url"]?.stringValue, taskId: t["task_id"]?.stringValue)
        }
    }
    var clientId: String? { meta?["client_id"]?.stringValue }
    var taskId: String? { meta?["task_id"]?.stringValue }
    var toolName: String? { meta?["name"]?.stringValue }
    var permissionRequestId: String? { meta?["request_id"]?.stringValue }

    /// The task a message is about: from `meta.task_id`, or the `[Task <id>]` tag briefs and reports start with.
    var referencedTaskId: String? { taskId ?? Self.splitTaskTag(content).id }
    /// Tasks this message is about, for chips: the attached one first, then task links in its text (#116); older
    /// messages, the task tag in their text.
    var taskRefs: [String] {
        if case .array(let a)? = meta?["task_refs"] {
            let ids = a.compactMap(\.stringValue)
            if !ids.isEmpty { return ids }
        }
        return referencedTaskId.map { [$0] } ?? []
    }

    /// "brief" (a task handed to this agent) or "report" (a delegated subtask's outcome), set by Loft.
    var taskRole: String? { meta?["task_role"]?.stringValue }
    var isTaskBrief: Bool { taskRole == "brief" }

    /// The text to show: without task tags and the bracketed instructions Loft adds for the agent.
    var displayContent: String {
        var text = Self.splitTaskTag(content).rest
        if let r = text.range(of: "\n\n[Report progress on this task") { text = String(text[..<r.lowerBound]) }
        // Older briefs mention other tasks inline ("This is a subtask of [Task <uuid>] Title.").
        text = text.replacingOccurrences(of: #"\[Task [0-9a-fA-F-]{36}\]\s*"#, with: "", options: .regularExpression)
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    static func splitTaskTag(_ s: String) -> (id: String?, rest: String) {
        guard s.hasPrefix("[Task "), let close = s.firstIndex(of: "]") else { return (nil, s) }
        let id = s[s.index(s.startIndex, offsetBy: 6)..<close].trimmingCharacters(in: .whitespaces)
        guard UUID(uuidString: id) != nil else { return (nil, s) }
        return (id.lowercased(), String(s[s.index(after: close)...]).trimmingCharacters(in: .whitespaces))
    }
}

struct PermissionRequest: Codable, Identifiable, Hashable {
    let id: String
    let agentId: String
    /// prompt (a blocked tool call) | request (the agent asked explicitly)
    var source: String
    var toolName: String
    var input: JSONValue?
    var reason: String?
    var rule: String?
    /// pending | allowed_once | allowed_always | denied | expired
    var status: String
    var createdAt: Date

    var isPending: Bool { status == "pending" }

    /// The whole request as text, for the explainer: the command, the file, the URL, or the input as JSON.
    var detail: String {
        if let cmd = input?["command"]?.stringValue { return cmd }
        if let path = input?["file_path"]?.stringValue { return path }
        if let url = input?["url"]?.stringValue { return url }
        guard let input, let d = try? JSONEncoder().encode(input), let s = String(data: d, encoding: .utf8) else { return toolName }
        return String(s.prefix(600))
    }

    /// What allowing it could do that you'd want to know first (decided in code, not by the model).
    var risks: [PermissionRisk] { PermissionRisk.of(tool: toolName, input: input, rule: rule) }

    /// A one-line, human description of what is being asked, without dumping raw input.
    var summary: String {
        if let cmd = input?["command"]?.stringValue { return "\(toolName): \(cmd)" }
        if let path = input?["file_path"]?.stringValue { return "\(toolName): \((path as NSString).lastPathComponent)" }
        if let perm = input?["permission"]?.stringValue { return "Grant \(perm)" }
        return toolName
    }
}

/// A `#` tag on a chat message, as the backend resolved it: it opens the Loft issue, else the GitHub page.
struct MessageTag: Hashable, Identifiable {
    let tag: String
    /// spec | issue | pr
    let kind: String
    let title: String
    let url: String?
    let taskId: String?
    var id: String { tag }
}

struct HBTask: Codable, Identifiable, Hashable {
    let id: String
    var title: String
    var description: String
    /// todo (personal) | work (meant for an agent)
    var kind: String
    var status: String
    var priority: String
    var project: String
    var agentId: String?
    var parentId: String?
    var dependsOn: [String]
    var dueAt: Date?
    /// "user" or the id of the agent that delegated it.
    var createdBy: String
    var note: String
    var summary: String
    var error: String
    var createdAt: Date
    var updatedAt: Date
    var startedAt: Date?
    var completedAt: Date?
    /// Label names, as on GitHub. Older servers send none.
    var labels: [String]? = nil
    /// The GitHub login of the person who owns it, or empty. Always a person, never an agent.
    var owner: String? = nil
    /// The GitHub issue it mirrors; nil for a Loft-only issue.
    var github: GitHubLink? = nil

    /// Where a mirrored Loft issue lives on GitHub (backend db.rs `GithubLink`).
    struct GitHubLink: Codable, Hashable {
        /// owner/name
        var repo: String
        var number: Int
        var url: String
        /// open | closed, as GitHub last said.
        var state: String
        /// Its sub-issues on GitHub, when it has any.
        var subTotal: Int?
        var subDone: Int?

        /// "loft#42": the repo's name and the number, as a row shows it.
        var shortName: String { "\(repo.split(separator: "/").last.map(String.init) ?? repo)#\(number)" }
        var subIssues: String? {
            guard let t = subTotal, t > 0 else { return nil }
            return "\(subDone ?? 0)/\(t) sub-issues"
        }
    }

    var ownerLogin: String? { owner.flatMap { $0.isEmpty ? nil : $0 } }

    var state: TaskState { TaskState(raw: status) }
    var priorityLevel: TaskPriority { TaskPriority(rawValue: priority) ?? .normal }
    var isPersonalTodo: Bool { kind == "todo" }
    var isDelegated: Bool { createdBy != "user" }
}

/// An entry in a Loft issue's thread (backend db.rs `TaskComment`): a comment, an agent's note, or a status change.
struct TaskComment: Codable, Identifiable, Hashable {
    let id: Int64
    var taskId: String
    /// "user", an agent's id, or "github" for a comment that came from the GitHub issue.
    var author: String
    /// comment | note | status
    var kind: String
    var text: String
    var meta: JSONValue?
    var createdAt: Date

    /// Where it stands with GitHub, on a mirrored issue: pending (waits for your approval), posted, kept
    /// (stays in Loft) or from_github. Nil when the issue isn't on GitHub.
    var githubState: String? { meta?["github"]?["state"]?.stringValue }
    /// The GitHub login, or an agent's name, as the server put it.
    var authorName: String? { meta?["author_name"]?.stringValue }
}

struct ActivityItem: Codable, Identifiable, Hashable {
    let id: String
    var type: String
    var at: Date
    var agentId: String?
    var taskId: String?
    var messageId: Int64?
    var summary: String
    /// info | success | attention | error
    var severity: String
}

struct PullRequest: Codable, Identifiable, Hashable {
    let id: String
    let agentId: String
    var slug: String
    var number: Int
    var url: String
    var title: String
    /// open | closed | merged
    var state: String
    var draft: Bool?
    var createdAt: Date?
    /// Label names; the verdicts are tests:green, qa:passed, security:passed and their failures.
    var labels: [String]?
}

struct ServerInfo: Codable, Hashable {
    var version: String?
    var host: String?
    var features: [String]?

    func supports(_ feature: ServerFeature) -> Bool { features?.contains(feature.rawValue) ?? false }
}

enum ServerFeature: String {
    case tasks, activity
    case idempotentMessages = "idempotent_messages"
    case grill
    case voiceClean = "voice_clean"
    case voiceBrief = "voice_brief"
    case system
    case runWatchdog = "run_watchdog"
    case plans
    case needsYou = "needs_you"
    case push
    case backups
    case workspaces
    /// Clean lists the workspace for the user to tick what goes, and logs it.
    case cleanPicker = "clean_picker"
}

/// The Mac's CPU temperature (`GET /api/system`), sampled every 30 s on the Mac.
struct SystemReading: Codable, Hashable {
    /// Hottest CPU die sensor, °C. Nil where it can't be read (Intel Macs).
    var cpuTempC: Double?
    /// nominal | warm (≥ 85 °C) | hot (≥ 95 °C)
    var level: String?
    var sampledAt: Date?
}

/// Claude plan usage (`GET /api/usage`), as Claude Code's /usage shows it.
struct PlanUsage: Codable, Hashable {
    struct Window: Codable, Hashable, Identifiable {
        /// five_hour (the "session") | seven_day (the "week") | others the plan reports
        var key: String
        var label: String
        var percent: Double
        var resetsAt: Date?
        var id: String { key }
    }
    var error: String?
    var windows: [Window]

    var session: Window? { windows.first { $0.key == "five_hour" } }
    var week: Window? { windows.first { $0.key == "seven_day" } }
}

/// One agent's last 14 days (`GET /api/agents/summary`), as the web dashboard shows it.
struct AgentSummary: Codable, Hashable, Identifiable {
    struct PRCounts: Codable, Hashable { var total: Int; var open: Int; var merged: Int }
    struct Reply: Codable, Hashable { var text: String; var at: Date }

    var agent: Agent
    var days: Int
    /// Runs per local day, oldest first: scheduled wake-ups vs. ones you (or another agent) started.
    var dailyAuto: [Int]
    var dailyUser: [Int]
    var toolCalls: Int
    var userMessages: Int
    var memories: Int
    var pendingPermissions: Int
    var lastError: String?
    var lastReply: Reply?
    /// [[tool name, count], …]
    var topTools: [JSONValue]
    var prs: PRCounts

    var id: String { agent.id }
    var runs: Int { dailyAuto.reduce(0, +) + dailyUser.reduce(0, +) }
    var topToolCounts: [(name: String, count: Int)] {
        topTools.compactMap { v in
            guard case .array(let a) = v, a.count == 2, let n = a[0].stringValue, let c = a[1].numberValue else { return nil }
            return (n, Int(c))
        }
    }
}

/// The context size warning, like the web's.
enum ContextWindow {
    /// Past this many tokens in context a Session leaves the smart zone: the same limit grills compact at.
    static let warnAt = 80_000

    /// Whether the context size shows in the warning color.
    static func warns(_ tokens: Int) -> Bool { tokens >= warnAt }
}

struct VoiceCleanResponse: Codable {
    var text: String
    var cleaned: Bool
    var reason: String?
}

/// `POST /api/voice/brief` (#118): a long dictation as a short brief by Claude on the Mac. With `briefed: false`,
/// `text` is the original and `reason` says why ("already short", "brief added something", a timeout…).
struct VoiceBriefResponse: Codable {
    var text: String
    var briefed: Bool
    var reason: String?
    var added: String?
}

struct SendMessageResponse: Codable {
    var ok: Bool
    var queued: Bool?
    var duplicate: Bool?
    var message: ChatMessage?
    /// Start delivery on a plan whose project folder has no git: nothing started; ask before setting it up.
    var needsGit: Bool? = nil
    var dir: String? = nil
}

struct ExecuteResponse: Codable {
    var task: HBTask
    var queued: Bool?
}

struct CancelResponse: Codable {
    var task: HBTask
    var stopped: Bool?
}

/// Events from `GET /api/events` (server-sent events). Unknown types are kept as `.other` and ignored.
enum ServerEvent {
    case agentUpdated(Agent)
    case agentDeleted(String)
    case message(ChatMessage)
    case permission(PermissionRequest)
    case pullRequest(PullRequest)
    case task(HBTask)
    case taskDeleted(String)
    case system(SystemReading)
    case grill(GrillSession)
    /// The "Needs you" list changed: refetch when `ids` differ from what's shown.
    case needsYou(count: Int, ids: [String])
    /// The Delivery of spec (or standalone ticket) `repo#spec` changed: a card showing it reloads.
    case delivery(repo: String, spec: Int)
    case other(String)

    private struct Envelope: Decodable { let type: String }
    private struct AgentBody: Decodable { let agent: Agent }
    private struct AgentDeletedBody: Decodable { let agentId: String }
    private struct MessageBody: Decodable { let message: ChatMessage }
    private struct PermissionBody: Decodable { let request: PermissionRequest }
    private struct PRBody: Decodable { let pr: PullRequest }
    private struct TaskBody: Decodable { let task: HBTask }
    private struct TaskDeletedBody: Decodable { let taskId: String }
    private struct SystemBody: Decodable { let system: SystemReading }
    private struct GrillBody: Decodable { let session: GrillSession }
    private struct NeedsYouBody: Decodable { let count: Int; let ids: [String] }
    private struct DeliveryBody: Decodable { let repo: String; let spec: Int }

    static func decode(_ data: Data) throws -> ServerEvent {
        let d = JSONCoding.decoder
        let type = try d.decode(Envelope.self, from: data).type
        switch type {
        case "agent_updated": return .agentUpdated(try d.decode(AgentBody.self, from: data).agent)
        case "agent_deleted": return .agentDeleted(try d.decode(AgentDeletedBody.self, from: data).agentId)
        case "message": return .message(try d.decode(MessageBody.self, from: data).message)
        case "permission": return .permission(try d.decode(PermissionBody.self, from: data).request)
        case "pull_request": return .pullRequest(try d.decode(PRBody.self, from: data).pr)
        case "task": return .task(try d.decode(TaskBody.self, from: data).task)
        case "task_deleted": return .taskDeleted(try d.decode(TaskDeletedBody.self, from: data).taskId)
        case "system": return .system(try d.decode(SystemBody.self, from: data).system)
        case "grill": return .grill(try d.decode(GrillBody.self, from: data).session)
        case "needs_you":
            let b = try d.decode(NeedsYouBody.self, from: data)
            return .needsYou(count: b.count, ids: b.ids)
        case "delivery":
            let b = try d.decode(DeliveryBody.self, from: data)
            return .delivery(repo: b.repo, spec: b.spec)
        default: return .other(type)
        }
    }
}
