import Foundation

/// Which Needs you items the Mac app shows a notification for (spec #382): an agent that needs you (a question or a
/// permission) and a PR ready to merge, the same events the iPhone gets a push for. Items already on the list when
/// the app first sees it are only remembered, so launching doesn't flood.
public struct NeedsYouAlerts {
    public static let types: Set<String> = ["permission", "question", "grill_round", "pr_ready"]

    private var seen: Set<String> = []
    private var primed = false

    public init() {}

    /// The items to notify about for this list; the first list only sets the baseline. Every call marks the list as
    /// seen, also when the caller posts nothing because the window is in front: an item the user saw there doesn't
    /// notify later when the app goes to the background. `seen` keeps only the current list, so an item that left the
    /// list and comes back notifies again.
    public mutating func fresh(in list: NeedsYouList) -> [NeedsYouItem] {
        let new = list.items.filter { !seen.contains($0.id) }
        seen = Set(list.items.map(\.id))
        defer { primed = true }
        return primed ? new.filter { Self.types.contains($0.type) } : []
    }

    /// The Dock badge: the Needs you count, none when it's empty or the list hasn't loaded.
    public static func badge(for list: NeedsYouList?) -> String? {
        guard let n = list?.count, n > 0 else { return nil }
        return String(n)
    }

    /// Where a click on the item's notification leads, in a push's `target` shape.
    public static func target(of item: NeedsYouItem) -> [String: String]? {
        if let t = item.openAction?.target {
            if let url = t.url { return ["url": url] }
            if let id = t.agentId { return ["screen": t.screen ?? "chat", "agent_id": id] }
        }
        if let id = item.agent?.id { return ["screen": "chat", "agent_id": id] }
        return ["screen": "needs_you"]
    }
}
