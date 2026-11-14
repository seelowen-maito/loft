import Foundation

/// Who Talk can send to: the To: row's chips and the More list (iPhone and Mac).
public enum TalkRecipients {
    /// Who a one-tap Talk starts with: whoever you talked to last, else the orchestrator; nil when neither is an agent
    /// Loft has (pick one then).
    public static func starter(lastAgentId: String?, orchestratorId: String?, agents: [Agent]) -> String? {
        [lastAgentId, orchestratorId].compactMap { $0 }.first { id in agents.contains { $0.id == id } }
    }

    /// The chips: `current`, then every Lead, then the others, each most recently run first. The others fill the row
    /// up to `limit` chips; Leads are all shown, even past it.
    public static func chips(_ agents: [Agent], current: String?, limit: Int = 5) -> [Agent] {
        let recent = agents.sorted { ($0.lastRunAt ?? .distantPast) > ($1.lastRunAt ?? .distantPast) }
        let first = recent.filter { $0.id == current }
        let leads = recent.filter { $0.id != current && $0.isLead }
        let others = recent.filter { $0.id != current && !$0.isLead }
        return first + leads + others.prefix(max(0, limit - first.count - leads.count))
    }

    /// A chip's name, with its group when another chip has the same name ("Lead · Dineout").
    public static func label(_ a: Agent, among shown: [Agent]) -> String {
        let twin = shown.contains { $0.id != a.id && $0.name == a.name }
        return twin && !a.team.isEmpty ? "\(a.name) · \(a.team)" : a.name
    }

    /// The agents matching `query`, in sections by group (A to Z, agents without one last), names A to Z.
    public static func sections(_ agents: [Agent], query: String) -> [(title: String, agents: [Agent])] {
        let q = query.trimmingCharacters(in: .whitespaces)
        let hits = agents.filter { a in
            q.isEmpty || ([a.name, a.roleLabel, a.group ?? ""] + a.allProjects).contains { $0.localizedCaseInsensitiveContains(q) }
        }
        let byGroup = Dictionary(grouping: hits) { ($0.group ?? "").trimmingCharacters(in: .whitespaces) }
        return byGroup.keys
            .sorted { ($0.isEmpty ? 1 : 0, $0.lowercased()) < ($1.isEmpty ? 1 : 0, $1.lowercased()) }
            .map { g in
                (g.isEmpty ? "No group" : g, byGroup[g]!.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending })
            }
    }
}

/// What a `loft://` link opened in the app does (iPhone and Mac).
public enum TalkLink {
    public enum Route: Equatable {
        /// The Talk widget's or control's signed link: start listening with this agent.
        case start(String)
        /// A signed Talk link with no one to start with: ask who.
        case pick
        /// Anything else, a Talk link nobody signed (another app, a web page) too: open it, mic off.
        case open(DeepLink)
    }

    /// nil for a link Loft doesn't know.
    public static func route(_ url: URL, key: String?, lastAgentId: String?, orchestratorId: String?, agents: [Agent]) -> Route? {
        guard let link = DeepLink(url: url) else { return nil }
        guard TalkKey.signs(url, key: key) else { return .open(link) }
        return TalkRecipients.starter(lastAgentId: lastAgentId, orchestratorId: orchestratorId, agents: agents).map(Route.start) ?? .pick
    }
}
