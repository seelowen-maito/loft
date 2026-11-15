import Foundation

/// "@ho" being typed at the end of the message: the agents to offer, and the text once one is picked. The same
/// rule as the web composer, and the mention text the backend matches (`@<exact name> `, see backend mentions.rs).
public enum MentionQuery {
    /// What follows an `@` that starts a word at the end of `text`, or nil when no mention is being typed.
    public static func query(in text: String) -> String? {
        guard let at = text.lastIndex(of: "@") else { return nil }
        let before = text[..<at]
        guard before.isEmpty || before.last?.isWhitespace == true else { return nil }
        let q = text[text.index(after: at)...]
        guard !q.contains(where: { $0.isWhitespace || $0 == "@" }) else { return nil }
        return String(q).lowercased()
    }

    /// Agents whose name starts with the query (any word of it, so "dev" finds "Web Dev"), not the chat's own, in
    /// sections: the chat agent's team first, then every other agent. All of them: the picker scrolls, so no agent
    /// is left out however many there are. The backend delivers a mention to any agent, its own team's first.
    public static func sections(_ q: String, in agents: [Agent], excluding name: String) -> [(title: String, agents: [Agent])] {
        let team = agents.first { $0.name == name }?.team
        let hits = agents.filter { a in
            a.name != name && (q.isEmpty || a.name.lowercased().hasPrefix(q) || a.name.lowercased().split(separator: " ").contains { $0.hasPrefix(q) })
        }
        .sorted { ($0.name.lowercased().hasPrefix(q) ? 0 : 1, $0.name) < ($1.name.lowercased().hasPrefix(q) ? 0 : 1, $1.name) }
        guard let team else { return hits.isEmpty ? [] : [("Agents", hits)] }
        let ours = team.isEmpty ? [] : hits.filter { $0.team == team }, rest = hits.filter { team.isEmpty || $0.team != team }
        return [(team, ours), ("Other agents", rest)].filter { !$0.agents.isEmpty }
    }

    /// `text` with the `@…` being typed replaced by the agent's exact name and a space.
    public static func insert(_ agent: Agent, into text: String) -> String {
        guard let at = text.lastIndex(of: "@") else { return text + "@\(agent.name) " }
        return String(text[..<at]) + "@\(agent.name) "
    }

    /// Text with a fresh `@` at the end (the composer's @ button), with a space before it when needed.
    public static func start(_ text: String) -> String {
        text.isEmpty || text.last?.isWhitespace == true ? text + "@" : text + " @"
    }
}
