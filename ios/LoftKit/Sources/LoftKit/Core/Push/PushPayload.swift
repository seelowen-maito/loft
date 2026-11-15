import Foundation

/// Where a notification leads when tapped, read from its `userInfo` (a push's custom keys, or a local
/// notification's). Kept pure so routing and de-duplication can be tested without notifications.
public enum PushDestination: Equatable {
    case chat(agentId: String)
    case task(taskId: String)
    case needsYou
    case url(URL)

    /// Push payload: `{ aps: {…}, needs_you_id, type, target: {screen, agent_id?, task_id?, url?} }`. `loftHost` is
    /// the host of the Loft the app is connected to: a `url` target opens only when it's trusted (`opens(_:loftHost:)`),
    /// and any other link just opens the app, like a push with no target.
    public static func from(_ info: [AnyHashable: Any], loftHost: String? = nil) -> PushDestination? {
        if let t = info["target"] as? [String: Any] {
            if let s = t["url"] as? String { return link(s, loftHost: loftHost).map(PushDestination.url) }
            if (t["screen"] as? String) == "task", let id = t["task_id"] as? String { return .task(taskId: id) }
            // The summary push a phone gets at the rate limit.
            if (t["screen"] as? String) == "needs_you" { return .needsYou }
            if let id = t["agent_id"] as? String { return .chat(agentId: id) }
        }
        if info["needs_you_id"] != nil { return .needsYou }
        // A test push ("type": "test") has no target: tapping it just opens the app.
        return nil
    }

    /// Whether a push's link may be opened: `https` on github.com (the pull request and issue links Loft sends) or on
    /// the connected Loft's host. A stranger's relay key must not be able to open any page on the phone.
    public static func opens(_ url: URL, loftHost: String?) -> Bool {
        guard url.scheme?.lowercased() == "https", let host = url.host()?.lowercased() else { return false }
        return host == "github.com" || host == loftHost?.lowercased()
    }

    /// A link from the server (a Needs you Open, an Overview row) as a URL only when `opens(_:loftHost:)` trusts it;
    /// nil otherwise, so the app does nothing with it.
    public static func link(_ s: String?, loftHost: String?) -> URL? {
        guard let u = s.flatMap(URL.init(string:)), opens(u, loftHost: loftHost) else { return nil }
        return u
    }

    /// A Needs you Open link: what `link(_:loftHost:)` allows, plus `http`, `https` and `vnc` on the connected Loft's
    /// host or its tailnet host (`ServerInfo.host`), which the server builds Settings and Screen Sharing links from.
    public static func needsYouLink(_ s: String?, loftHost: String?, tailnetHost: String?) -> URL? {
        if let u = link(s, loftHost: loftHost) { return u }
        guard let u = s.flatMap(URL.init(string:)), let scheme = u.scheme?.lowercased(), ["http", "https", "vnc"].contains(scheme),
              let host = u.host()?.lowercased(), host == loftHost?.lowercased() || host == tailnetHost?.lowercased() else { return nil }
        return u
    }

    /// The relay portal page Sign in opens: only `https` on the relay's own host (`relay_url`), since the link comes
    /// from the server's reply. Nil otherwise.
    public static func portal(_ link: String?, relay: String) -> URL? {
        guard let url = link.flatMap(URL.init(string:)), url.scheme?.lowercased() == "https", let host = url.host()?.lowercased(),
              host == URL(string: relay)?.host()?.lowercased() else { return nil }
        return url
    }

    /// The "Needs you" item a notification is about, so a push and a local notification for the same item show once.
    public static func itemId(_ info: [AnyHashable: Any]) -> String? { info["needs_you_id"] as? String }
}

/// The switches in Settings › Push. Each covers one or more of the server's kinds, which are the "Needs you"
/// item types one to one (docs/push.md).
public enum PushKind: String, CaseIterable, Identifiable {
    case permission, approval, blockedFailed = "blocked_failed", questions, stuck, prReady = "pr_ready", prReview = "pr_review",
         deliveryStalled = "delivery_stalled", deliveryDone = "delivery_done"

    public var id: String { rawValue }
    public var label: String {
        switch self {
        case .permission: "Permission requests"
        case .approval: "Issues waiting for approval"
        case .blockedFailed: "Blocked or failed issues"
        case .questions: "Questions and interviews"
        case .stuck: "Stuck agents"
        case .prReady: "PRs ready to merge"
        case .prReview: "PRs waiting for your review"
        case .deliveryStalled: "Delivery stalled"
        case .deliveryDone: "Delivery done"
        }
    }
    /// The server's kinds this switch turns on.
    public var serverKinds: [String] {
        switch self {
        case .permission: ["permission"]
        case .approval: ["task_approval"]
        case .blockedFailed: ["task_blocked", "task_failed"]
        case .questions: ["question", "grill_round"]
        case .stuck: ["stuck"]
        case .prReady: ["pr_ready"]
        case .prReview: ["pr_review"]
        case .deliveryStalled: ["delivery_stalled"]
        case .deliveryDone: ["delivery_done"]
        }
    }
    /// On unless turned off, except PRs (the server's default leaves them out too).
    public var onByDefault: Bool { self != .prReady && self != .prReview }
    public var prefKey: String { "push.\(rawValue)" }

    public func isOn(_ d: UserDefaults = .standard) -> Bool { d.object(forKey: prefKey) == nil ? onByDefault : d.bool(forKey: prefKey) }
}

/// The keys local notifications use for the same items as the server's "Needs you" ids, so a push and a local
/// notification about one item aren't both shown.
public enum NeedsYouKey {
    public static func permission(_ id: String) -> String { "permission:\(id)" }
    public static func task(_ id: String, _ status: String) -> String { "task:\(id):\(status)" }
}
