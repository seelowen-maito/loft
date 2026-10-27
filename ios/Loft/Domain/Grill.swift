import Foundation

// "Grill me" interviews in rounds (ADR 0001): the agent posts every question it can ask now, each with a
// recommendation that "yes" accepts. Loft holds the answers until the round is sent, which wakes the agent
// once with all of them. Rounds, sent answers, summaries and status rows are ordinary chat messages tagged
// with `meta.grill`; the session state (the latest round with its answers so far) comes from
// `GET /agents/{id}/grill` and the `grill` server event; commands go to `POST /grill/{id}/command`.

enum GrillRole: String, Codable, Hashable {
    /// A round the agent posted; `answers` is the user's answers once it was sent; `command` a typed command.
    case round, answers, answer, command, summary, status
    case startRequest = "start_request"
}

/// What kind of answer a question expects; the card shows its options as chips.
enum GrillQuestionKind: String, Codable, Hashable {
    case freeText = "free_text"
    case singleChoice = "single_choice"
    case multiChoice = "multi_choice"
    case yesNo = "yes_no"
    case number
}

/// Commands from the cards, the menus and the voice. `skip` and `back` step through a round by voice and stay
/// on the phone (the server ignores them); `repeat` re-reads the question and never leaves the phone.
enum GrillCommand: String, Codable, Hashable, CaseIterable {
    case send, skip, back, pause, resume, end, summarize, decide
}

/// One question of a round. A round message carries the questions as posted; the session carries them with
/// the user's answers so far.
struct GrillQuestion: Codable, Hashable, Identifiable {
    /// discuss: the answer is a comment, not a decision; decided: left to the agent, its recommendation stands;
    /// carried: sent unanswered, asked again next round.
    enum State: String, Codable, Hashable {
        case open, answered, discuss, decided, carried
    }

    var id: String
    var text: String
    /// Short plain version for reading aloud (no links or PR numbers).
    var speak: String?
    var kind: GrillQuestionKind?
    var options: [String]?
    /// The agent's proposed answer; "yes" accepts it.
    var recommendation: String
    var state: State
    var answer: String?
    /// The answer is the recommendation, accepted as-is.
    var accepted: Bool
    /// Carried over unanswered: the round it was first asked in.
    var fromRound: Int?

    init(id: String, text: String, speak: String? = nil, kind: GrillQuestionKind? = nil, options: [String]? = nil,
         recommendation: String, state: State = .open, answer: String? = nil, accepted: Bool = false, fromRound: Int? = nil) {
        self.id = id
        self.text = text
        self.speak = speak
        self.kind = kind
        self.options = options
        self.recommendation = recommendation
        self.state = state
        self.answer = answer
        self.accepted = accepted
        self.fromRound = fromRound
    }

    /// Unknown kinds or states from a newer server decode as nil or open rather than failing the round.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        text = (try? c.decode(String.self, forKey: .text)) ?? ""
        speak = try? c.decode(String.self, forKey: .speak)
        kind = (try? c.decode(String.self, forKey: .kind)).flatMap(GrillQuestionKind.init(rawValue:))
        options = try? c.decode([String].self, forKey: .options)
        recommendation = (try? c.decode(String.self, forKey: .recommendation)) ?? ""
        state = (try? c.decode(String.self, forKey: .state)).flatMap(State.init(rawValue:)) ?? .open
        answer = try? c.decode(String.self, forKey: .answer)
        accepted = (try? c.decode(Bool.self, forKey: .accepted)) ?? false
        fromRound = try? c.decode(Int.self, forKey: .fromRound)
    }

    /// What is read aloud: the question in its short form, then the recommendation that "yes" accepts.
    func spoken(number: Int, of total: Int) -> String {
        let q = speak.flatMap { $0.isEmpty ? nil : $0 } ?? VoiceText.speakable(text).text
        let head = total > 1 ? "Question \(number) of \(total). " : ""
        return recommendation.isEmpty ? head + q : "\(head)\(q) Recommended: \(recommendation)."
    }
}

/// The end of a planning grill: the spec and tickets it published, the message Clear context leaves in the
/// composer, and the roles the project's team lacks, with the message that asks `loft` to fill them.
struct EndCard: Codable, Hashable {
    struct Issue: Codable, Hashable {
        var number: Int
        var title: String
        var url: URL
    }
    struct Setup: Codable, Hashable {
        var agentId: String
        var message: String
    }
    var repo: String
    var spec: Issue?
    var tickets: [Issue]
    var nextMessage: String
    /// "dev", "reviewer".
    var missingRoles: [String]
    var setup: Setup?

    /// "Dev or reviewer", as in "The team has no Dev or reviewer yet."
    var missingText: String {
        missingRoles.map { $0 == "dev" ? "Dev" : $0 }.joined(separator: " or ")
    }
}

/// `meta.grill` on a chat message. Fields are optional because each role uses a different subset.
struct GrillMeta: Codable, Hashable {
    var sessionId: String?
    var role: GrillRole
    // round / answers
    var round: Int?
    var topic: String?
    var questions: [GrillQuestion]?
    // status ("decided" names its question)
    var questionId: String?
    var state: String?
    var command: GrillCommand?
    var endReason: String?
    /// Set when the backend read an untagged message as a command.
    var taggedMessageId: Int64?
    // summary
    var final: Bool?
    /// Where the notes file was written in the agent's workspace (an agent without a repo).
    var notesPath: String?
    /// The end-of-planning card, on the final summary of a grill that published a spec or tickets.
    var endCard: EndCard?

    init(sessionId: String?, role: GrillRole, round: Int? = nil, topic: String? = nil, questions: [GrillQuestion]? = nil,
         questionId: String? = nil, state: String? = nil, command: GrillCommand? = nil, endReason: String? = nil,
         taggedMessageId: Int64? = nil, final: Bool? = nil, notesPath: String? = nil, endCard: EndCard? = nil) {
        self.sessionId = sessionId
        self.role = role
        self.round = round
        self.topic = topic
        self.questions = questions
        self.questionId = questionId
        self.state = state
        self.command = command
        self.endReason = endReason
        self.taggedMessageId = taggedMessageId
        self.final = final
        self.notesPath = notesPath
        self.endCard = endCard
    }

    /// Unknown roles, kinds or commands from a newer server decode as nil rather than failing the message.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        guard let role = (try? c.decode(String.self, forKey: .role)).flatMap(GrillRole.init(rawValue:)) else {
            throw DecodingError.dataCorruptedError(forKey: .role, in: c, debugDescription: "unknown grill role")
        }
        self.role = role
        sessionId = try? c.decode(String.self, forKey: .sessionId)
        round = try? c.decode(Int.self, forKey: .round)
        topic = try? c.decode(String.self, forKey: .topic)
        questions = try? c.decode([GrillQuestion].self, forKey: .questions)
        questionId = try? c.decode(String.self, forKey: .questionId)
        state = try? c.decode(String.self, forKey: .state)
        command = (try? c.decode(String.self, forKey: .command)).flatMap(GrillCommand.init(rawValue:))
        endReason = try? c.decode(String.self, forKey: .endReason)
        taggedMessageId = try? c.decode(Int64.self, forKey: .taggedMessageId)
        final = try? c.decode(Bool.self, forKey: .final)
        notesPath = try? c.decode(String.self, forKey: .notesPath)
        endCard = try? c.decode(EndCard.self, forKey: .endCard)
    }

    /// The tag an answer carries, so it lands on the question it was given for. With `discuss`, the text is a
    /// comment for the agent instead of a decision.
    static func answer(sessionId: String, questionId: String, discuss: Bool = false) -> JSONValue {
        var o: [String: JSONValue] = ["role": .string(GrillRole.answer.rawValue), "session_id": .string(sessionId), "question_id": .string(questionId)]
        if discuss { o["discuss"] = .bool(true) }
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

/// An interview's state, from `GET /agents/{id}/grill` and the `grill` server event: the latest round with
/// its answers so far.
struct GrillSession: Codable, Hashable {
    enum State: String, Codable, Hashable {
        /// Started; the agent is writing the first round.
        case preparing, active, paused, ended
    }

    var sessionId: String
    var agentId: String?
    var state: State
    var topic: String?
    var startedAt: Date?
    /// The latest round's number, its size and how many of its questions have an answer.
    var round: Int?
    var total: Int?
    var answered: Int?
    /// The latest round was sent; the agent is working on the next one.
    var waiting: Bool?
    var questions: [GrillQuestion]?
    /// done | stopped | timeout | superseded, once ended.
    var endReason: String?

    init(sessionId: String, agentId: String?, state: State, topic: String? = nil, startedAt: Date? = nil, round: Int? = nil,
         total: Int? = nil, answered: Int? = nil, waiting: Bool? = nil, questions: [GrillQuestion]? = nil, endReason: String? = nil) {
        self.sessionId = sessionId
        self.agentId = agentId
        self.state = state
        self.topic = topic
        self.startedAt = startedAt
        self.round = round
        self.total = total
        self.answered = answered
        self.waiting = waiting
        self.questions = questions
        self.endReason = endReason
    }

    /// Shown in chat (the round cards, the menu) until it ends.
    var isOpen: Bool { state != .ended }

    /// A round waits for the user's answers (paused included: answering picks it up again).
    var hasOpenRound: Bool {
        isOpen && state != .preparing && waiting != true && !(questions ?? []).isEmpty
    }

    /// 1-based position of a question in the open round.
    func number(of questionId: String) -> Int? {
        questions?.firstIndex { $0.id == questionId }.map { $0 + 1 }
    }
}

/// Reading a round aloud one question at a time: where the voice goes after each answer, skip or "I don't know".
/// The round is sent after its last question, with any skipped ones carried into the next round.
enum GrillWalk {
    enum Step: Equatable {
        case ask(String)
        case send
    }

    /// Where reading starts: the first question still open, or nil when every one has an answer.
    static func start(_ questions: [GrillQuestion]) -> String? {
        questions.first { $0.state == .open }?.id
    }

    /// After the question `id`: the next open question after it, else send the round.
    static func next(after id: String, in questions: [GrillQuestion]) -> Step {
        guard let i = questions.firstIndex(where: { $0.id == id }) else { return start(questions).map(Step.ask) ?? .send }
        return questions[(i + 1)...].first { $0.state == .open }.map { .ask($0.id) } ?? .send
    }

    /// "Back": the question before `id`, answered or not, so its answer can be changed.
    static func back(from id: String, in questions: [GrillQuestion]) -> String? {
        guard let i = questions.firstIndex(where: { $0.id == id }), i > 0 else { return nil }
        return questions[i - 1].id
    }
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
    /// Commands the app handles itself ("repeat", "skip", "back").
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
