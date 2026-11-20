import Foundation

/// An agent's settings being edited (the web's settings tab, `frontend/src/pages/agents/Inspector.tsx`), and what
/// to send to `PATCH /agents/{id}` when saved: only the fields that changed.
public struct AgentDraft: Equatable {
    public var name: String
    public var role: String
    public var group: String
    public var prompt: String
    public var model: String
    public var effort: String
    public var permissionMode: String
    public var intervalMinutes: Int
    public var freshContext: String

    public init(_ a: Agent) {
        name = a.name
        role = (a.role ?? "").isEmpty ? "custom" : a.role!
        group = a.group ?? ""
        prompt = a.prompt ?? ""
        model = a.model ?? ""
        effort = a.effort ?? ""
        permissionMode = a.permissionMode ?? "auto"
        intervalMinutes = a.intervalMinutes ?? 0
        freshContext = a.freshContext ?? "above_80k"
    }

    /// The fields that differ from `original`, by their API names.
    public func changes(from original: AgentDraft) -> [String: JSONValue] {
        var out: [String: JSONValue] = [:]
        func put(_ key: String, _ new: String, _ old: String) { if new != old { out[key] = .string(new) } }
        put("name", name.trimmingCharacters(in: .whitespaces), original.name)
        put("role", role, original.role)
        put("group", group.trimmingCharacters(in: .whitespaces), original.group)
        put("prompt", prompt, original.prompt)
        put("model", model, original.model)
        put("effort", effort, original.effort)
        put("permission_mode", permissionMode, original.permissionMode)
        put("fresh_context", freshContext, original.freshContext)
        if intervalMinutes != original.intervalMinutes { out["interval_minutes"] = .number(Double(intervalMinutes)) }
        return out
    }

    /// The choices the web offers.
    public static let models = [("sonnet", "Sonnet · balanced"), ("opus", "Opus · most capable"), ("fable", "Fable"), ("haiku", "Haiku · fastest")]
    public static let efforts = [("", "Default"), ("low", "low"), ("medium", "medium"), ("high", "high"), ("xhigh", "xhigh"), ("max", "max")]
    public static let intervals = [(0, "Manual only"), (15, "Every 15 min"), (60, "Hourly"), (360, "Every 6 hours"), (1440, "Daily")]
    public static let permissionModes = [("auto", "Auto"), ("default", "Ask"), ("acceptEdits", "Accept edits"), ("plan", "Plan only"), ("bypassPermissions", "Bypass")]
    public static let freshContexts = [("above_80k", "Above 80k (default)"), ("above_100k", "Above 100k"), ("above_120k", "Above 120k"),
                                       ("above_150k", "Above 150k"), ("above_180k", "Above 180k"), ("above_200k", "Above 200k"),
                                       ("always", "Always"), ("off", "Off")]
}
