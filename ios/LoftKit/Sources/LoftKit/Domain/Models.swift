import Foundation

// Swift mirrors of the Loft backend's JSON (backend/src/db.rs, tasks.rs).
// The backend sends snake_case keys; `JSONCoding.decoder` converts them.

/// A prompt Loft sends again every interval until it's stopped.
public struct AgentLoop: Codable, Hashable {
    public var prompt: String
    public var intervalMinutes: Int
    public var nextAt: Date
}

/// A prompt Loft sends once, or at each due time of a repeat (`every`: "day" or "weekday" at `time`).
public struct AgentSchedule: Codable, Hashable, Identifiable {
    public var id: String
    public var prompt: String
    public var every: String? = nil
    public var time: String? = nil
    public var nextAt: Date

    /// "Every weekday 8:00 AM" or "Once", the time in the user's locale; short enough for one line.
    public var repeatLabel: String { repeatLabel() }

    func repeatLabel(locale: Locale = .current) -> String {
        guard let every else { return "Once" }
        return ["Every", every, Self.clock(time ?? "", locale: locale)].filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// The schedule's row: "Every weekday · next Mon Nov 16 8:00 AM". The repeat's time is left out when it is the
    /// next run's time, which is the usual case, so the row fits on one line.
    public func rowLabel(locale: Locale = .current, timeZone: TimeZone = .current) -> String {
        let next = Self.nextLabel(nextAt, locale: locale, timeZone: timeZone)
        guard let every else { return "Once · next \(next)" }
        let at = Self.clock(time ?? "", locale: locale)
        let same = at == Self.format(nextAt, "jmm", locale: locale, timeZone: timeZone)
        // Only the parts that exist: a schedule without a time gets no extra space.
        return ["Every", every, same ? "" : at].filter { !$0.isEmpty }.joined(separator: " ") + " · next \(next)"
    }

    /// "HH:MM" from the server as a time of day in the user's locale ("8:00 AM", "08:00"); as is when unreadable.
    static func clock(_ t: String, locale: Locale) -> String {
        let p = t.split(separator: ":")
        guard p.count == 2, let h = Int(p[0]), let m = Int(p[1]), (0..<24).contains(h), (0..<60).contains(m) else { return t }
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        guard let d = cal.date(from: DateComponents(year: 2000, month: 1, day: 1, hour: h, minute: m)) else { return t }
        return format(d, "jmm", locale: locale, timeZone: cal.timeZone)
    }

    private static func format(_ d: Date, _ template: String, locale: Locale, timeZone: TimeZone) -> String {
        let f = DateFormatter()
        f.locale = locale
        f.timeZone = timeZone
        f.setLocalizedDateFormatFromTemplate(template)
        return f.string(from: d)
    }

    /// A next-run time as "Mon Nov 16 8:00 AM" (en_US) or "Mon 16 Nov 08:00" (en_GB): the same short shape for a
    /// loop and a schedule, from the user's locale pieces (day and month order, 12 or 24 hour, language) without
    /// the commas and "at" a full format adds.
    public static func nextLabel(_ date: Date, locale: Locale = .current, timeZone: TimeZone = .current) -> String {
        ["EEE", "MMMd", "jmm"].map { format(date, $0, locale: locale, timeZone: timeZone) }.joined(separator: " ")
    }
}

public struct Agent: Codable, Identifiable, Hashable {
    public let id: String
    public var name: String
    public var project: String?
    public var model: String?
    public var effort: String?
    public var permissionMode: String?
    public var intervalMinutes: Int?
    /// Raw backend status: idle | running | waiting | error.
    public var status: String
    public var statusText: String?
    /// Whether a Claude process is running for this agent right now.
    public var running: Bool?
    public var lastRunAt: Date?
    public var nextRunAt: Date?
    public var createdAt: Date?
    public var totalCostUsd: Double?
    public var contextTokens: Int?
    public var prompt: String?
    /// The agent's sticker: a 100x100 SVG drawing (older servers don't send it).
    public var avatar: Avatar?
    /// What it does in its team: lead, dev, code_reviewer, qa, security or custom (nil from older servers).
    public var role: String? = nil
    /// When Claude last produced anything in the current run; nil when not running (servers with `run_watchdog`).
    public var lastOutputAt: Date? = nil
    /// Background tasks the agent is waiting on after it already replied (e.g. "Run the QA suite").
    public var waitingOn: [String]? = nil
    public var waitingSince: Date? = nil
    /// The Claude Code Session its next run resumes; nil after Clear (and from older servers).
    public var sessionId: String? = nil
    /// Fresh context for each Delivery ticket: "off", "above_80k" … "above_200k" or "always" (nil from older servers).
    public var freshContext: String? = nil
    /// Whether that setting applies: to every agent but a Lead that only coordinates (its team has a Dev).
    public var freshContextApplies: Bool? = nil
    /// Loft's own agent, which can't be deleted (nil from older servers).
    public var builtin: Bool? = nil
    /// Free-text name of its team ("Acme"); empty when it has no team (older servers don't send it).
    public var group: String? = nil
    /// The projects it works in, main first; `project` is the main one (older servers send only `project`).
    public var projects: [String]? = nil
    /// Its running `/loop`, and its `/schedule` entries (older servers send neither).
    public var loop: AgentLoop? = nil
    public var schedules: [AgentSchedule]? = nil

    /// Leads its team; it wears a captain hat.
    public var isLead: Bool { role == "lead" }

    /// Every role and how the user sees it, in the backend's order (`planning::ROLES`).
    public static let roles: [(id: String, label: String)] = [("lead", "Lead"), ("dev", "Dev"), ("code_reviewer", "Code reviewer"),
                                                       ("qa", "QA"), ("security", "Security"), ("custom", "Custom")]

    /// The label a role goes by: "Code reviewer", "Custom". No role (an older server) reads as Custom; an unknown one as it is.
    public static func roleLabel(_ role: String?) -> String {
        let r = (role ?? "").isEmpty ? "custom" : role!
        return roles.first { $0.id == r }?.label ?? r
    }

    /// The label it goes by, in place of the Title agents used to have.
    public var roleLabel: String { Agent.roleLabel(role) }

    /// Its team: the group; empty when it has none (backend `Agent::team`).
    public var team: String { (group ?? "").trimmingCharacters(in: .whitespaces) }

    /// The projects it works in, main first: `projects`, else the one `project` from an older server.
    public var allProjects: [String] {
        if let projects, !projects.isEmpty { return projects }
        return (project ?? "").isEmpty ? [] : [project!]
    }

    /// The order of every agent picker for an issue: those working in its `project` first, Devs first within each
    /// part, then by name (web `issuePickerOrder`). The first one is the default, so a hand-off goes to an agent that builds.
    public static func issuePickerOrder(_ agents: [Agent], project: String) -> [Agent] {
        let p = project.trimmingCharacters(in: .whitespaces)
        func rank(_ a: Agent) -> Int { (!p.isEmpty && a.allProjects.contains(p) ? 0 : 2) + (a.role == "dev" ? 0 : 1) }
        return agents.sorted {
            let a = rank($0), b = rank($1)
            return a != b ? a < b : $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
    }
}

/// One of an agent's past Sessions, from Claude Code's transcripts (`/agents/{id}/sessions`).
public struct PastSession: Codable, Identifiable, Hashable {
    public let id: String
    public var startedAt: Date
    public var firstMessage: String
    /// The agent that sent `firstMessage`, when another agent started the Session.
    public var fromAgent: String?
    public var contextTokens: Int?
}

/// An agent's past Sessions, newest first, and which one is its Session now.
public struct PastSessions: Codable, Hashable {
    public var sessions: [PastSession]
    public var current: String?
}

public struct Avatar: Codable, Hashable {
    public var svg: String
}

public struct ChatMessage: Codable, Identifiable, Hashable {
    public let id: Int64
    public let agentId: String
    public var runId: String?
    /// user | assistant | system
    public var role: String
    /// text | thinking | tool_use | tool_result | permission | error | notice | run_start | run_end | clear | compact | resume
    public var kind: String
    public var content: String
    public var meta: JSONValue?
    public var createdAt: Date

    /// Set when another agent sent this message rather than the user.
    public var fromAgentName: String? { meta?["from_agent"]?["name"]?.stringValue }
    public var fromAgentId: String? { meta?["from_agent"]?["id"]?.stringValue }
    /// An FYI from another agent: shown at once, read on the agent's next Run, without waking it (#112).
    public var isFYI: Bool { role == "user" && fromAgentName != nil && meta?["fyi"] == .bool(true) }
    /// Another agent's reply to an @-mention, copied into this chat (#92).
    public var isMentionReply: Bool { meta?["mention_reply"] == .bool(true) }
    /// The agents your message @-mentioned, and whether it went only to them (the chat's agent wasn't woken).
    public var mentionedNames: [String] {
        guard case .array(let a)? = meta?["mentions"] else { return [] }
        return a.compactMap { $0["name"]?.stringValue }
    }
    public var mentionsOnly: Bool { meta?["mentions_only"] == .bool(true) }
    /// The specs, issues and pull requests your message tagged with `#` that Loft knows (`meta.tags`).
    public var tags: [MessageTag] {
        guard case .array(let a)? = meta?["tags"] else { return [] }
        return a.compactMap { t in
            guard let tag = t["tag"]?.stringValue, let kind = t["kind"]?.stringValue, let title = t["title"]?.stringValue else { return nil }
            return MessageTag(tag: tag, kind: kind, title: title, url: t["url"]?.stringValue, taskId: t["task_id"]?.stringValue)
        }
    }
    public var clientId: String? { meta?["client_id"]?.stringValue }
    public var taskId: String? { meta?["task_id"]?.stringValue }
    public var toolName: String? { meta?["name"]?.stringValue }

    /// The task a message is about: from `meta.task_id`, or the `[Task <id>]` tag briefs and reports start with.
    public var referencedTaskId: String? { taskId ?? Self.splitTaskTag(content).id }
    /// Tasks this message is about, for chips: the attached one first, then task links in its text (#116); older
    /// messages, the task tag in their text.
    public var taskRefs: [String] {
        if case .array(let a)? = meta?["task_refs"] {
            let ids = a.compactMap(\.stringValue)
            if !ids.isEmpty { return ids }
        }
        return referencedTaskId.map { [$0] } ?? []
    }

    /// "brief" (a task handed to this agent) or "report" (a delegated subtask's outcome), set by Loft.
    public var taskRole: String? { meta?["task_role"]?.stringValue }
    public var isTaskBrief: Bool { taskRole == "brief" }

    /// The text to show: without task tags and the bracketed instructions Loft adds for the agent.
    public var displayContent: String {
        var text = Self.splitTaskTag(content).rest
        if let r = text.range(of: "\n\n[Report progress on this task") { text = String(text[..<r.lowerBound]) }
        // Older briefs mention other tasks inline ("This is a subtask of [Task <uuid>] Title.").
        text = text.replacingOccurrences(of: #"\[Task [0-9a-fA-F-]{36}\]\s*"#, with: "", options: .regularExpression)
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    public static func splitTaskTag(_ s: String) -> (id: String?, rest: String) {
        guard s.hasPrefix("[Task "), let close = s.firstIndex(of: "]") else { return (nil, s) }
        let id = s[s.index(s.startIndex, offsetBy: 6)..<close].trimmingCharacters(in: .whitespaces)
        guard UUID(uuidString: id) != nil else { return (nil, s) }
        return (id.lowercased(), String(s[s.index(after: close)...]).trimmingCharacters(in: .whitespaces))
    }
}

public struct PermissionRequest: Codable, Identifiable, Hashable {
    public let id: String
    public let agentId: String
    /// prompt (a blocked tool call) | request (the agent asked explicitly)
    public var source: String
    public var toolName: String
    public var input: JSONValue?
    public var reason: String?
    public var rule: String?
    /// pending | allowed_once | allowed_always | denied | expired
    public var status: String
    public var createdAt: Date

    public var isPending: Bool { status == "pending" }

    /// Whether Always allow can save something. A Kill's rule is only ever one Loft made for a program it can
    /// name (the backend checks it), so an empty rule means none.
    public var canAlwaysAllow: Bool {
        // A change to the user's checkout is only ever allowed once.
        toolName != "Checkout" && (toolName != "Kill" || !(rule ?? "").isEmpty)
    }

    /// The whole request as text, for the explainer: the command, the file, the URL, or the input as JSON.
    public var detail: String {
        if let cmd = input?["command"]?.stringValue { return cmd }
        if let path = input?["file_path"]?.stringValue { return path }
        if let url = input?["url"]?.stringValue { return url }
        guard let input, let d = try? JSONEncoder().encode(input), let s = String(data: d, encoding: .utf8) else { return toolName }
        return String(s.prefix(600))
    }

    /// What allowing it could do that you'd want to know first (decided in code, not by the model).
    public var risks: [PermissionRisk] { PermissionRisk.of(tool: toolName, input: input, rule: rule) }

    /// A one-line, human description of what is being asked, without dumping raw input.
    public var summary: String {
        if let cmd = input?["command"]?.stringValue { return "\(toolName): \(cmd)" }
        if let path = input?["file_path"]?.stringValue { return "\(toolName): \((path as NSString).lastPathComponent)" }
        if let perm = input?["permission"]?.stringValue { return "Grant \(perm)" }
        return toolName
    }
}

/// A `#` tag on a chat message, as the backend resolved it: it opens the Loft issue, else the GitHub page.
public struct MessageTag: Hashable, Identifiable {
    public let tag: String
    /// spec | issue | pr
    public let kind: String
    public let title: String
    public let url: String?
    public let taskId: String?
    public var id: String { tag }
}

public struct HBTask: Codable, Identifiable, Hashable {
    public let id: String
    public var title: String
    public var description: String
    /// todo (personal) | work (meant for an agent)
    public var kind: String
    public var status: String
    public var priority: String
    public var project: String
    public var agentId: String?
    public var parentId: String?
    public var dependsOn: [String]
    public var dueAt: Date?
    /// "user" or the id of the agent that delegated it.
    public var createdBy: String
    public var note: String
    public var summary: String
    public var error: String
    public var createdAt: Date
    public var updatedAt: Date
    public var startedAt: Date?
    public var completedAt: Date?
    /// Label names, as on GitHub. Older servers send none.
    public var labels: [String]? = nil
    /// The GitHub login of the person who owns it, or empty. Always a person, never an agent.
    public var owner: String? = nil
    /// The GitHub issue it mirrors; nil for a Loft-only issue.
    public var github: GitHubLink? = nil

    /// Where a mirrored Loft issue lives on GitHub (backend db.rs `GithubLink`).
    public struct GitHubLink: Codable, Hashable {
        /// owner/name
        public var repo: String
        public var number: Int
        public var url: String
        /// open | closed, as GitHub last said.
        public var state: String
        /// Its sub-issues on GitHub, when it has any.
        public var subTotal: Int?
        public var subDone: Int?

        /// "loft#42": the repo's name and the number, as a row shows it.
        public var shortName: String { "\(repo.split(separator: "/").last.map(String.init) ?? repo)#\(number)" }
        public var subIssues: String? {
            guard let t = subTotal, t > 0 else { return nil }
            return "\(subDone ?? 0)/\(t) sub-issues"
        }
    }

    public var ownerLogin: String? { owner.flatMap { $0.isEmpty ? nil : $0 } }

    public var state: TaskState { TaskState(raw: status) }
    public var priorityLevel: TaskPriority { TaskPriority(rawValue: priority) ?? .normal }
    public var isPersonalTodo: Bool { kind == "todo" }
    public var isDelegated: Bool { createdBy != "user" }
}

/// An entry in a Loft issue's thread (backend db.rs `TaskComment`): a comment, an agent's note, or a status change.
public struct TaskComment: Codable, Identifiable, Hashable {
    public let id: Int64
    public var taskId: String
    /// "user", an agent's id, or "github" for a comment that came from the GitHub issue.
    public var author: String
    /// comment | note | status
    public var kind: String
    public var text: String
    public var meta: JSONValue?
    public var createdAt: Date

    /// Where it stands with GitHub, on a mirrored issue: pending (waits for your approval), posted, kept
    /// (stays in Loft) or from_github. Nil when the issue isn't on GitHub.
    public var githubState: String? { meta?["github"]?["state"]?.stringValue }
    /// The GitHub login, or an agent's name, as the server put it.
    public var authorName: String? { meta?["author_name"]?.stringValue }
}

public struct ActivityItem: Codable, Identifiable, Hashable {
    public let id: String
    public var type: String
    public var at: Date
    public var agentId: String?
    public var taskId: String?
    public var messageId: Int64?
    public var summary: String
    /// info | success | attention | error
    public var severity: String
}

public struct PullRequest: Codable, Identifiable, Hashable {
    public let id: String
    public let agentId: String
    public var slug: String
    public var number: Int
    public var url: String
    public var title: String
    /// open | closed | merged
    public var state: String
    public var draft: Bool?
    public var createdAt: Date?
    /// Label names; the verdicts are tests:green, qa:passed, security:passed and their failures.
    public var labels: [String]?
}

public struct ServerInfo: Codable, Hashable {
    public var version: String?
    public var host: String?
    public var features: [String]?

    public func supports(_ feature: ServerFeature) -> Bool { features?.contains(feature.rawValue) ?? false }
}

public enum ServerFeature: String {
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
    /// Settings › Disk lists agents' Simulator devices.
    case agentDevices = "agent_devices"
    /// Settings › Disk lists what an idle agent left running, with Stop them; an agent is busy only in a run.
    case leftProcesses = "left_processes"
}

/// The Mac's CPU temperature (`GET /api/system`), sampled every 30 s on the Mac.
public struct SystemReading: Codable, Hashable {
    /// Hottest CPU die sensor, °C. Nil where it can't be read (Intel Macs).
    public var cpuTempC: Double?
    /// nominal | warm (≥ 85 °C) | hot (≥ 95 °C)
    public var level: String?
    public var sampledAt: Date?
    /// Active + wired + compressed, like Activity Monitor's "Memory Used". Nil from an older Loft.
    public var memUsedBytes: UInt64? = nil
    public var memTotalBytes: UInt64? = nil
    /// nominal | high (≥ 80 %) | critical (≥ 90 %), from memory used.
    public var memLevel: String? = nil
}

/// Claude plan usage (`GET /api/usage`), as Claude Code's /usage shows it.
public struct PlanUsage: Codable, Hashable {
    public struct Window: Codable, Hashable, Identifiable {
        /// five_hour (the "session") | seven_day (the "week") | others the plan reports
        public var key: String
        public var label: String
        public var percent: Double
        public var resetsAt: Date?
        public var id: String { key }
    }
    public var error: String?
    public var windows: [Window]

    public var session: Window? { windows.first { $0.key == "five_hour" } }
    public var week: Window? { windows.first { $0.key == "seven_day" } }
}

/// One agent's last 14 days (`GET /api/agents/summary`), as the web dashboard shows it.
public struct AgentSummary: Codable, Hashable, Identifiable {
    public struct PRCounts: Codable, Hashable { public var total: Int; public var open: Int; public var merged: Int }
    public struct Reply: Codable, Hashable { public var text: String; public var at: Date }

    public var agent: Agent
    public var days: Int
    /// Runs per local day, oldest first: scheduled wake-ups vs. ones you (or another agent) started.
    public var dailyAuto: [Int]
    public var dailyUser: [Int]
    public var toolCalls: Int
    public var userMessages: Int
    public var memories: Int
    public var pendingPermissions: Int
    public var lastError: String?
    public var lastReply: Reply?
    /// [[tool name, count], …]
    public var topTools: [JSONValue]
    public var prs: PRCounts
    /// Estimated at API list prices; nil from an older server.
    public var costUsd: Double? = nil

    public var id: String { agent.id }
    public var runs: Int { dailyAuto.reduce(0, +) + dailyUser.reduce(0, +) }
    public var topToolCounts: [(name: String, count: Int)] {
        topTools.compactMap { v in
            guard case .array(let a) = v, a.count == 2, let n = a[0].stringValue, let c = a[1].numberValue else { return nil }
            return (n, Int(c))
        }
    }
}

/// The context size warning, like the web's.
public enum ContextWindow {
    /// Past this many tokens in context a Session leaves the smart zone: the same limit grills compact at.
    public static let warnAt = 80_000

    /// Whether the context size shows in the warning color.
    public static func warns(_ tokens: Int) -> Bool { tokens >= warnAt }
}

public struct VoiceCleanResponse: Codable {
    public var text: String
    public var cleaned: Bool
    public var reason: String?
}

/// `POST /api/voice/brief` (#118): a long dictation as a short brief by Claude on the Mac. With `briefed: false`,
/// `text` is the original and `reason` says why ("already short", "brief added something", a timeout…).
public struct VoiceBriefResponse: Codable {
    public var text: String
    public var briefed: Bool
    public var reason: String?
    public var added: String?
}

public struct SendMessageResponse: Codable {
    public var ok: Bool
    public var queued: Bool?
    public var duplicate: Bool?
    public var message: ChatMessage?
    /// Start delivery on a plan whose project folder has no git: nothing started; ask before setting it up.
    public var needsGit: Bool? = nil
    public var dir: String? = nil
}

public struct ExecuteResponse: Codable {
    public var task: HBTask
    public var queued: Bool?
}

public struct CancelResponse: Codable {
    public var task: HBTask
    public var stopped: Bool?
}

/// Events from `GET /api/events` (server-sent events). Unknown types are kept as `.other` and ignored.
public enum ServerEvent {
    case agentUpdated(Agent)
    case agentDeleted(String)
    case message(ChatMessage)
    case permission(PermissionRequest)
    case pullRequest(PullRequest)
    case task(HBTask)
    case taskDeleted(String)
    case system(SystemReading)
    case grill(GrillSession)
    /// The "Needs you" list changed: refetch when `ids` differ from what's shown, or `new` (what hasn't been seen,
    /// the badge) from the list's.
    case needsYou(count: Int, new: Int? = nil, ids: [String])
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
    private struct NeedsYouBody: Decodable { let count: Int; let new: Int?; let ids: [String] }
    private struct DeliveryBody: Decodable { let repo: String; let spec: Int }

    public static func decode(_ data: Data) throws -> ServerEvent {
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
            return .needsYou(count: b.count, new: b.new, ids: b.ids)
        case "delivery":
            let b = try d.decode(DeliveryBody.self, from: data)
            return .delivery(repo: b.repo, spec: b.spec)
        default: return .other(type)
        }
    }
}
