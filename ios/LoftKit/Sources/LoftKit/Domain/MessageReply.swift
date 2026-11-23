import Foundation

/// A quoted message (spec #572): the composer's quote before sending, and the one a sent reply carries in
/// `meta.reply_to`. `author` is nil for the user.
public struct MessageQuote: Hashable, Sendable {
    public let id: Int64
    public let author: String?
    public let excerpt: String

    public init(id: Int64, author: String?, excerpt: String) {
        self.id = id
        self.author = author
        self.excerpt = excerpt
    }

    /// As the server cuts it: one line, 200 characters and an ellipsis.
    public static let excerptChars = 200

    /// Who wrote it, as shown: the user is "You".
    public var authorLabel: String { author ?? "You" }

    /// The quote of `m` in `agentName`'s chat, as the composer shows it before sending.
    public init(of m: ChatMessage, agentName: String) {
        let line = m.content.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        id = m.id
        author = m.fromAgentName ?? (m.role == "user" ? nil : agentName)
        excerpt = line.count > Self.excerptChars ? String(line.prefix(Self.excerptChars)) + "…" : line
    }

    /// The meta to send a message with while the composer quotes `quote`: `reply_to` (the quoted id), which
    /// the API client sends as the request's `reply_to`, next to anything else (an attached issue).
    public static func meta(_ meta: JSONValue?, replyingTo quote: MessageQuote?) -> JSONValue? {
        guard let quote else { return meta }
        var o: [String: JSONValue] = if case .object(let o)? = meta { o } else { [:] }
        o["reply_to"] = .number(Double(quote.id))
        return .object(o)
    }
}

extension ChatMessage {
    /// Only chat messages can be quoted (as the server rules): text from the user, another agent or the agent.
    public var isQuotable: Bool { kind == "text" && (role == "user" || role == "assistant") }

    /// The message this one replies to, from `meta.reply_to`.
    public var replyQuote: MessageQuote? {
        guard let r = meta?["reply_to"], let id = r["id"]?.numberValue else { return nil }
        return MessageQuote(id: Int64(id), author: r["author"]?.stringValue, excerpt: r["excerpt"]?.stringValue ?? "")
    }
}
