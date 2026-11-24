import Foundation

/// A chat message kept in Saved (`/api/saved`, spec #675): a copy of its text with a title, a tag and where it came from.
public struct SavedItem: Codable, Identifiable, Hashable, Sendable {
    public let id: Int64
    public var title: String
    public var tag: String
    /// The message as Markdown.
    public var content: String
    /// The agent it was saved from, and its name then: the agent may be gone.
    public let agentId: String
    public let agentName: String
    public let messageId: Int64
    public let createdAt: Date
    public var updatedAt: Date?
    /// Only on `GET /saved/{id}`: the message it came from is still in its agent's chat.
    public var sourceExists: Bool?

    public init(id: Int64, title: String, tag: String, content: String, agentId: String, agentName: String, messageId: Int64,
                createdAt: Date, updatedAt: Date? = nil, sourceExists: Bool? = nil) {
        self.id = id
        self.title = title
        self.tag = tag
        self.content = content
        self.agentId = agentId
        self.agentName = agentName
        self.messageId = messageId
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.sourceExists = sourceExists
    }
}

/// `GET /saved`: the items, newest first, and every tag in use.
public struct SavedList: Codable, Sendable {
    public var items: [SavedItem]
    public var tags: [String]

    public init(items: [SavedItem], tags: [String]) {
        self.items = items
        self.tags = tags
    }
}

/// Saved's rules, as the web's frontend/src/lib/saved.ts.
public enum Saved {
    /// The tags the Save form offers; any other text works too.
    public static let suggestedTags = ["Tutorial", "Good to know", "Decision"]

    /// Only chat text, yours or the agent's, can be saved; not a cancelled message.
    public static func canSave(_ m: ChatMessage) -> Bool {
        m.isQuotable && !m.isCancelled && !m.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// The Save form's title for a message: its first line without Markdown's heading, quote or list mark, cut at a word.
    public static func defaultTitle(_ content: String, chars: Int = 80) -> String {
        let mark = #/^\s*(#{1,6}\s+|>\s*|[-*+]\s+|\d+[.)]\s+)/#
        let line = content.split(separator: "\n", omittingEmptySubsequences: false)
            .map { $0.replacing(mark, with: "", maxReplacements: 1).trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? ""
        guard line.count > chars else { return line }
        let cut = String(line.prefix(chars))
        let head = cut.lastIndex(of: " ").map { cut[..<$0] }.flatMap { $0.isEmpty ? nil : $0 } ?? Substring(cut)
        return head.trimmingCharacters(in: .whitespaces) + "…"
    }

    /// The tag filter's choices: the suggested tags, then the others in use, each once whatever its case.
    public static func tagOptions(_ used: [String]) -> [String] {
        var seen = Set<String>()
        return (suggestedTags + used).filter {
            let k = $0.trimmingCharacters(in: .whitespaces).lowercased()
            return !k.isEmpty && seen.insert(k).inserted
        }
    }

    /// The tag filter once the list is back: a tag no item has any more (its last one was deleted) goes back to All,
    /// so the filter never sits on a chip that's gone (#681).
    public static func keptFilter(_ tag: String, tags: [String]) -> String {
        tag.isEmpty || tagOptions(tags).contains(tag) ? tag : ""
    }

    /// The list's preview of an item's content: plain words, without Markdown's marks.
    public static func excerpt(_ content: String, chars: Int = 160) -> String {
        // A table's separator row and a horizontal rule aren't words.
        // One character class, not `\s*\|?\s*`: two `\s*` in a row backtrack in quadratic time on a long run of spaces (#681).
        let rule = #/^[\s|]*:?-{3,}/#
        let text = content.split(separator: "\n", omittingEmptySubsequences: false).filter { $0.firstMatch(of: rule) == nil }.joined(separator: "\n")
        let words = text.replacing(#/[`*_>#|\[\]]/#, with: " ").split(whereSeparator: \.isWhitespace).joined(separator: " ")
        return words.count > chars ? String(words.prefix(chars)) + "…" : words
    }
}
