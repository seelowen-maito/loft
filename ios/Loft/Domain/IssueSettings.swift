import Foundation

/// Settings › GitHub issues (spec #65, global since #177): when comments on mirrored issues go to GitHub, and
/// whether new Loft issues are published there. Both live in the server's `/api/settings`.
struct IssueSettings: Equatable {
    /// ask (every comment waits for approval) | own (yours go straight, agents' wait) | all (everything goes straight)
    var commentApproval: String
    var autoPublish: Bool

    init(commentApproval: String, autoPublish: Bool) {
        self.commentApproval = commentApproval
        self.autoPublish = autoPublish
    }

    /// From `GET /api/settings`'s `settings` object; nil on a server without the GitHub issues settings.
    init?(_ settings: JSONValue) {
        guard let mode = settings["comment_approval_default"]?.stringValue else { return nil }
        commentApproval = ["own", "all"].contains(mode) ? mode : "ask"
        autoPublish = settings["auto_publish_all"]?.boolValue ?? false
    }

    /// The whole settings object to `PUT` back with these two changed. `PUT /api/settings` replaces everything, so
    /// the rest goes back as it came; `github_token` is dropped, since the server keeps the saved token when it's
    /// absent and the empty string it sends us would clear it.
    func applied(to settings: JSONValue) -> JSONValue {
        guard case .object(var o) = settings else { return settings }
        o["comment_approval_default"] = .string(commentApproval)
        o["auto_publish_all"] = .bool(autoPublish)
        o["github_token"] = nil
        return .object(o)
    }
}
