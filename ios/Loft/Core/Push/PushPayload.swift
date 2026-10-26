import Foundation

/// Where a notification leads when tapped, read from its `userInfo` (a push's custom keys, or a local
/// notification's). Kept pure so routing and de-duplication can be tested without notifications.
enum PushDestination: Equatable {
    case chat(agentId: String)
    case task(taskId: String)
    case needsYou
    case url(URL)

    /// Push payload: `{ aps: {…}, needs_you_id, type, target: {screen, agent_id?, task_id?, url?} }`.
    static func from(_ info: [AnyHashable: Any]) -> PushDestination? {
        if let t = info["target"] as? [String: Any] {
            if let s = t["url"] as? String, let u = URL(string: s), u.scheme == "https" { return .url(u) }
            if (t["screen"] as? String) == "task", let id = t["task_id"] as? String { return .task(taskId: id) }
            // The summary push a phone gets at the rate limit.
            if (t["screen"] as? String) == "needs_you" { return .needsYou }
            if let id = t["agent_id"] as? String { return .chat(agentId: id) }
        }
        if info["needs_you_id"] != nil { return .needsYou }
        // A test push ("type": "test") has no target: tapping it just opens the app.
        return nil
    }

    /// The "Needs you" item a notification is about, so a push and a local notification for the same item show once.
    static func itemId(_ info: [AnyHashable: Any]) -> String? { info["needs_you_id"] as? String }
}

/// The switches in Settings › Push. Each covers one or more of the server's kinds, which are the "Needs you"
/// item types one to one (docs/push.md).
enum PushKind: String, CaseIterable, Identifiable {
    case permission, approval, blockedFailed = "blocked_failed", questions, stuck, prReady = "pr_ready", prReview = "pr_review"

    var id: String { rawValue }
    var label: String {
        switch self {
        case .permission: "Permission requests"
        case .approval: "Tasks waiting for approval"
        case .blockedFailed: "Blocked or failed tasks"
        case .questions: "Questions and interviews"
        case .stuck: "Stuck agents"
        case .prReady: "PRs ready to merge"
        case .prReview: "PRs waiting for your review"
        }
    }
    /// The server's kinds this switch turns on.
    var serverKinds: [String] {
        switch self {
        case .permission: ["permission"]
        case .approval: ["task_approval"]
        case .blockedFailed: ["task_blocked", "task_failed"]
        case .questions: ["question", "grill_question"]
        case .stuck: ["stuck"]
        case .prReady: ["pr_ready"]
        case .prReview: ["pr_review"]
        }
    }
    /// On unless turned off, except PRs (the server's default leaves them out too).
    var onByDefault: Bool { self != .prReady && self != .prReview }
    var prefKey: String { "push.\(rawValue)" }

    func isOn(_ d: UserDefaults = .standard) -> Bool { d.object(forKey: prefKey) == nil ? onByDefault : d.bool(forKey: prefKey) }
}

/// The keys local notifications use for the same items as the server's "Needs you" ids, so a push and a local
/// notification about one item aren't both shown.
enum NeedsYouKey {
    static func permission(_ id: String) -> String { "permission:\(id)" }
    static func task(_ id: String, _ status: String) -> String { "task:\(id):\(status)" }
}
