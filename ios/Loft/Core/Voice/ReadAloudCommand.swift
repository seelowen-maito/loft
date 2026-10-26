import Foundation

/// What you can say while a reply or question is being read aloud. Only a short, clear command on
/// its own counts (the grill matcher's "sure" rule), so talking to someone nearby doesn't stop it.
enum ReadAloudCommand: Equatable {
    /// "stop", "para": stop reading; in a conversation, also end it.
    case stop
    /// "skip", "next", "pula", "próxima": stop reading and move on (the next question, or your turn).
    case skip
    /// "repeat", "de novo": read it again from the start.
    case repeatIt
    /// "more", "mais": read the rest of a reply that was read short.
    case more

    static let morePhrases = ["more", "tell me more", "read more", "go on", "continue", "keep going", "the rest",
                              "mais", "fala mais", "conta mais", "le mais", "continua", "continuar", "o resto"]

    /// The command in `text`, or nil when it's anything else.
    static func match(_ text: String, phrases: GrillPhrases? = nil) -> ReadAloudCommand? {
        let words = VoiceText.normalize(text).split(separator: " ").map(String.init)
        let core = words.drop { GrillMatcher.fillers.contains($0) }.reversed().drop { GrillMatcher.fillers.contains($0) }.reversed()
        let said = core.joined(separator: " ")
        guard !said.isEmpty else { return nil }
        if morePhrases.map(VoiceText.normalize).contains(said) { return .more }
        guard let m = GrillMatcher.match(text, phrases: phrases), m.confidence == .sure else { return nil }
        switch m.intent {
        case .command(.end), .command(.pause): return .stop
        case .command(.skip): return .skip
        case .repeatQuestion: return .repeatIt
        default: return nil
        }
    }

    /// A recognized word and when it was said, from the start of listening.
    struct Word: Equatable {
        let text: String
        let at: TimeInterval
        let duration: TimeInterval
    }

    /// The command said last, on its own: the words after the latest pause, if there are no more than a few.
    /// The transcript keeps everything heard since reading started, so a command is looked for at its end.
    static func latest(in words: [Word], pause: TimeInterval = 0.5, maxWords: Int = 4, phrases: GrillPhrases? = nil) -> ReadAloudCommand? {
        guard var i = words.indices.last else { return nil }
        while i > words.startIndex {
            let gap = words[i].at - (words[i - 1].at + words[i - 1].duration)
            if gap >= pause { break }
            i -= 1
        }
        let tail = words[i...]
        guard tail.count <= maxWords else { return nil }
        return match(tail.map(\.text).joined(separator: " "), phrases: phrases)
    }
}
