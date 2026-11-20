import Foundation

/// One command of an agent's "/" menu, as `GET /api/agents/{id}/commands` lists it: the same list the web and herdr show.
public struct AgentCommand: Codable, Identifiable, Hashable, Sendable {
    public let name: String
    public let description: String
    /// `passthrough` (sent to the CLI as the message), `loft` (Loft runs it) or `terminal` (can't run here).
    public let tag: String
    public let reason: String?
    public var id: String { name }
    public var isTerminal: Bool { tag == "terminal" }

    public init(name: String, description: String, tag: String, reason: String? = nil) {
        self.name = name
        self.description = description
        self.tag = tag
        self.reason = reason
    }
}

/// "/rev" being typed as the message's first word: the commands to offer, and what sending a command does.
/// The same rules as the web composer (frontend lib/commands.ts).
public enum CommandQuery {
    /// What follows a leading `/` while it is the only word typed, or nil when no command is being typed.
    public static func query(in text: String) -> String? {
        let t = text.drop { $0.isWhitespace }
        guard t.first == "/", !t.contains(where: \.isWhitespace) else { return nil }
        return String(t.dropFirst()).lowercased()
    }

    /// Names that start with the query first, then names that contain it, then descriptions that do.
    public static func matches(_ list: [AgentCommand], _ q: String) -> [AgentCommand] {
        let q = q.lowercased()
        func rank(_ c: AgentCommand) -> Int {
            let n = c.name.dropFirst().lowercased()
            return n.hasPrefix(q) ? 0 : n.contains(q) ? 1 : c.description.lowercased().contains(q) ? 2 : 3
        }
        return list.enumerated().map { ($0.offset, $0.element, rank($0.element)) }
            .filter { $0.2 < 3 }
            .sorted { ($0.2, $0.0) < ($1.2, $1.0) }
            .map(\.1)
    }

    /// The text once a command is picked: it waits for its arguments, so Enter sends it.
    public static func insert(_ c: AgentCommand) -> String { c.name + " " }

    /// What sending a message that starts with a command does.
    public enum Action: Equatable {
        case chat(String)
        case clear
        case compact(String)
        case resume
        case code
        case grill(String)
        case loop(String)
        case schedule(String)
        case info(name: String, arg: String)
        case terminal(String)
        case unknown(String)
    }

    private static let info: Set<String> = ["/model", "/effort", "/cost", "/status", "/login"]

    /// The action for `text`, or nil when it is an ordinary message. With no list loaded yet, everything is a message.
    public static func action(for text: String, commands: [AgentCommand]) -> Action? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.hasPrefix("/"), !commands.isEmpty else { return nil }
        let parts = t.split(maxSplits: 1, whereSeparator: \.isWhitespace)
        let word = parts[0].lowercased()
        let rest = parts.count > 1 ? parts[1].trimmingCharacters(in: .whitespacesAndNewlines) : ""
        guard let c = commands.first(where: { $0.name == word }) else {
            // "/usr/bin is odd" is text; a bare "/nope" is a mistyped command.
            return rest.isEmpty ? .unknown(word) : nil
        }
        switch word {
        case "/clear": return .clear
        case "/compact": return .compact(rest)
        case "/resume": return .resume
        case "/code": return .code
        case "/grill": return .grill(rest)
        case "/loop": return .loop(rest)
        case "/schedule": return .schedule(rest)
        case _ where info.contains(word): return .info(name: String(word.dropFirst()), arg: rest)
        default:
            if c.isTerminal { return .terminal("\(c.name) can't run here. \(c.reason ?? "Claude Code runs it only in its own terminal app.")") }
            return .chat(rest.isEmpty ? word : "\(word) \(rest)")
        }
    }

    /// What follows `/loop`: `stop`, or an interval (10m, 2h, 1d) and the prompt to repeat; nil when it's neither.
    public static func parseLoop(_ rest: String) -> (interval: String, prompt: String)? {
        let parts = rest.trimmingCharacters(in: .whitespacesAndNewlines).split(maxSplits: 1, whereSeparator: \.isWhitespace)
        guard parts.count == 2, parts[0].count >= 2, let unit = parts[0].last?.lowercased(), "mhd".contains(unit),
              parts[0].dropLast().allSatisfy(\.isNumber) else { return nil }
        return (parts[0].lowercased(), parts[1].trimmingCharacters(in: .whitespacesAndNewlines))
    }
}
