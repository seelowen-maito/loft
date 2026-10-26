import Foundation

// "Grill me" interviews: the agent asks one question at a time inside the normal chat.
// Contract v2 with the backend: questions, answers, summaries and status rows are
// ordinary chat messages tagged with `meta.grill`; the session state comes from
// `GET /agents/{id}/grill` and the `grill` server event; commands go to `POST /grill/{id}/command`.

enum GrillRole: String, Codable, Hashable {
    case question, answer, summary, note, status
    case startRequest = "start_request"
}

/// What kind of answer a question expects; the app picks chips, yes/no buttons or a keyboard from it.
enum GrillQuestionKind: String, Codable, Hashable {
    case freeText = "free_text"
    case singleChoice = "single_choice"
    case multiChoice = "multi_choice"
    case yesNo = "yes_no"
    case number
}

/// Commands the server understands. `repeat` and `next` never reach it: repeat re-reads the question
/// on the phone, and next is the same as skip (or just answering).
enum GrillCommand: String, Codable, Hashable, CaseIterable {
    case skip, back, pause, resume, end, summarize, decide
}

/// `meta.grill` on a chat message. Fields are optional because each role uses a different subset.
struct GrillMeta: Codable, Hashable {
    var sessionId: String?
    var role: GrillRole
    // question / answer
    var questionId: String?
    var index: Int?
    var total: Int?
    var topic: String?
    var kind: GrillQuestionKind?
    var options: [String]?
    var allowFreeText: Bool?
    /// Short plain text for reading aloud (no links or PR numbers); falls back to the message text.
    var speak: String?
    /// The question posted again (after back, resume or repeat), with the same question_id.
    var `repeat`: Bool?
    // summary
    var final: Bool?
    // status
    var state: String?
    var command: GrillCommand?
    var endReason: String?
    /// Set when the backend read an untagged message as a command, so the app can offer Undo.
    var taggedMessageId: Int64?

    init(sessionId: String?, role: GrillRole, questionId: String? = nil, index: Int? = nil, total: Int? = nil,
         topic: String? = nil, kind: GrillQuestionKind? = nil, options: [String]? = nil, allowFreeText: Bool? = nil,
         speak: String? = nil, final: Bool? = nil, state: String? = nil, command: GrillCommand? = nil,
         endReason: String? = nil, taggedMessageId: Int64? = nil) {
        self.sessionId = sessionId
        self.role = role
        self.questionId = questionId
        self.index = index
        self.total = total
        self.topic = topic
        self.kind = kind
        self.options = options
        self.allowFreeText = allowFreeText
        self.speak = speak
        self.final = final
        self.state = state
        self.command = command
        self.endReason = endReason
        self.taggedMessageId = taggedMessageId
    }

    /// Unknown roles, kinds or commands from a newer server decode as nil rather than failing the message.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        guard let role = (try? c.decode(String.self, forKey: .role)).flatMap(GrillRole.init(rawValue:)) else {
            throw DecodingError.dataCorruptedError(forKey: .role, in: c, debugDescription: "unknown grill role")
        }
        self.role = role
        sessionId = try? c.decode(String.self, forKey: .sessionId)
        questionId = try? c.decode(String.self, forKey: .questionId)
        index = try? c.decode(Int.self, forKey: .index)
        total = try? c.decode(Int.self, forKey: .total)
        topic = try? c.decode(String.self, forKey: .topic)
        kind = (try? c.decode(String.self, forKey: .kind)).flatMap(GrillQuestionKind.init(rawValue:))
        options = try? c.decode([String].self, forKey: .options)
        allowFreeText = try? c.decode(Bool.self, forKey: .allowFreeText)
        speak = try? c.decode(String.self, forKey: .speak)
        `repeat` = try? c.decode(Bool.self, forKey: .repeat)
        final = try? c.decode(Bool.self, forKey: .final)
        state = try? c.decode(String.self, forKey: .state)
        command = (try? c.decode(String.self, forKey: .command)).flatMap(GrillCommand.init(rawValue:))
        endReason = try? c.decode(String.self, forKey: .endReason)
        taggedMessageId = try? c.decode(Int64.self, forKey: .taggedMessageId)
    }

    /// The tag a user answer carries, so it lands on the question it was given for even if sent late.
    static func answer(to q: GrillMeta) -> JSONValue {
        var o: [String: JSONValue] = ["role": .string(GrillRole.answer.rawValue)]
        if let s = q.sessionId { o["session_id"] = .string(s) }
        if let id = q.questionId { o["question_id"] = .string(id) }
        if let i = q.index { o["index"] = .number(Double(i)) }
        return ["grill": .object(o)]
    }

    static let startRequest: JSONValue = ["grill": ["role": .string(GrillRole.startRequest.rawValue)]]
}

extension ChatMessage {
    /// The grill tag, when this message is part of an interview.
    var grill: GrillMeta? {
        guard let g = meta?["grill"], let data = try? JSONEncoder().encode(g) else { return nil }
        return try? JSONCoding.decoder.decode(GrillMeta.self, from: data)
    }
}

/// An interview's state, from `GET /agents/{id}/grill` and the `grill` server event.
struct GrillSession: Codable, Hashable {
    enum State: String, Codable, Hashable {
        /// Started; the agent is writing the first questions.
        case preparing, active, paused, ended
    }
    struct Current: Codable, Hashable {
        var questionId: String
        var index: Int
    }

    var sessionId: String
    var agentId: String?
    var state: State
    var index: Int?
    var total: Int?
    var topic: String?
    var startedAt: Date?
    var current: Current?
    var answered: Int?
    /// done | stopped | timeout, once ended.
    var endReason: String?

    /// Shown in chat (the status chip, the question's quick actions) until it ends.
    var isOpen: Bool { state != .ended }
}

/// `GET /api/grill/commands`: the phrases the backend matches, so the phone's matcher uses the same list.
struct GrillPhrases: Codable, Hashable {
    struct Phrases: Codable, Hashable {
        var en: [String]
        var pt: [String]
    }
    var version: String
    /// A spoken or typed message longer than this is never a command on its own.
    var maxWords: Int
    /// Words trimmed from the ends of an utterance ("um", "please"), never the middle.
    var fillers: [String]?
    /// Commands the app handles itself ("repeat").
    var clientOnly: [String]?
    var commands: [String: Phrases]

    init(version: String, maxWords: Int, fillers: [String]? = nil, clientOnly: [String]? = nil, commands: [String: Phrases]) {
        self.version = version
        self.maxWords = maxWords
        self.fillers = fillers
        self.clientOnly = clientOnly
        self.commands = commands
    }

    func phrases(for c: GrillCommand) -> [String] { phrases(forKey: c.rawValue) }

    /// Also covers client-only keys like "repeat".
    func phrases(forKey key: String) -> [String] {
        guard let p = commands[key] else { return [] }
        return p.en + p.pt
    }
}

/// `{ "session": … }` or `{ "session": null }`, the shape the grill endpoints answer with.
struct GrillSessionEnvelope: Decodable {
    var session: GrillSession?
}
