import Foundation

/// iPhone Talk: who a dictation is addressed to, from its first words ("Send this to the Dineout Lead, …",
/// "Loft Dev, can you …") or its last ("… send this to QA"). Matched on the phone against the agents' names, roles
/// and groups; the longest match wins, and a tie is no one.
public enum TalkAddress {
    public enum Reading: Equatable {
        /// Not addressed to anyone: the message stays as it was.
        case none
        /// Addressed to one agent: who, and the message without the addressing words.
        case agent(id: String, text: String)
        /// Addressed to a name that is no agent's, or more than one's: the recipient and the message stay as they were.
        case unclear
    }

    /// The words that address a message to someone ("send this to …").
    private static let leads: [[String]] = [["send", "this", "to"], ["this", "is", "for"]]
    /// Words that, followed by "you" right after a name the message starts with, show it's said to that agent
    /// ("Dev can you …"). On their own they also start plain statements ("Loft will need a restart").
    private static let asks: Set<String> = ["can", "could", "would", "will"]

    public static func read(_ text: String, agents: [Agent]) -> Reading {
        let words = text.split(whereSeparator: \.isWhitespace)
        let norm = words.map { VoiceText.normalize(String($0)) }
        let names = phrases(agents)

        // "Send this to <agent>, …"
        if let start = lead(in: norm, at: 0) {
            guard let (id, end) = match(norm, at: start, names) else { return .unclear }
            return .agent(id: id, text: rest(words[end...]))
        }
        // "… send this to <agent>": the name ends the message.
        for s in stride(from: norm.count - 1, to: 0, by: -1) {
            guard let start = lead(in: norm, at: s) else { continue }
            guard let (id, end) = match(norm, at: start, names), end == norm.count else {
                // A name of up to four words can follow; later in the message it's only words ("send this to them and …").
                if norm.count - start > 4 { continue }
                return .unclear
            }
            return .agent(id: id, text: String(words[..<s].joined(separator: " ")).trimmingCharacters(in: trim))
        }
        // "<agent>, …" or "<agent> can you …"
        guard let longest = longest(norm, at: 0, names) else { return .none }
        let end = longest.length
        guard end < words.count, words[end - 1].last.map({ ",:".contains($0) }) == true || asksAt(norm, end) else { return .none }
        guard longest.ids.count == 1 else { return .unclear }
        return .agent(id: longest.ids[0], text: rest(words[end...]))
    }

    /// "can you", "will you" … or "please" at `i`.
    private static func asksAt(_ norm: [String], _ i: Int) -> Bool {
        norm[i] == "please" || (asks.contains(norm[i]) && i + 1 < norm.count && norm[i + 1] == "you")
    }

    private static let trim = CharacterSet(charactersIn: " ,;:-–—")

    /// Where the name starts when one of `leads` (and maybe "the") is at `i`; nil when none is.
    private static func lead(in norm: [String], at i: Int) -> Int? {
        for l in leads where norm.count >= i + l.count && Array(norm[i..<(i + l.count)]) == l {
            let after = i + l.count
            return after < norm.count && norm[after] == "the" ? after + 1 : after
        }
        return nil
    }

    /// The one agent named at `i`, and where its name ends; nil when it's no one's or more than one's.
    private static func match(_ norm: [String], at i: Int, _ names: [(id: String, words: [String])]) -> (String, Int)? {
        guard let l = longest(norm, at: i, names), l.ids.count == 1 else { return nil }
        return (l.ids[0], i + l.length)
    }

    /// The agents whose longest phrase starts at `i`, with that phrase's length in words.
    private static func longest(_ norm: [String], at i: Int, _ names: [(id: String, words: [String])]) -> (ids: [String], length: Int)? {
        var best = 0
        var ids: [String] = []
        for n in names where norm.count >= i + n.words.count && Array(norm[i..<(i + n.words.count)]) == n.words {
            if n.words.count > best { best = n.words.count; ids = [] }
            if n.words.count == best, !ids.contains(n.id) { ids.append(n.id) }
        }
        return best == 0 ? nil : (ids, best)
    }

    /// What each agent can be called: its name, its group and name ("Dineout Lead"), and its role ("QA").
    private static func phrases(_ agents: [Agent]) -> [(id: String, words: [String])] {
        agents.flatMap { a -> [(id: String, words: [String])] in
            var all = [a.name]
            if !a.team.isEmpty { all.append("\(a.team) \(a.name)") }
            if let r = a.role, !r.isEmpty, r != "custom" { all.append(a.roleLabel) }
            return all.map { (a.id, VoiceText.normalize($0).split(separator: " ").map(String.init)) }.filter { !$0.words.isEmpty }
        }
    }

    /// The message after the addressing words, starting with a capital.
    private static func rest(_ words: ArraySlice<Substring>) -> String {
        let s = words.joined(separator: " ").trimmingCharacters(in: trim)
        return s.prefix(1).uppercased() + s.dropFirst()
    }
}
