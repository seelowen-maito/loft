import Foundation

/// What was said while an interview question waits, read by the phrase matcher first and, only when the
/// matcher can't tell, by the on-device model. The matcher decides alone when it's sure, so clear commands
/// stay instant. The model can only turn an unclear reply into a question ("Skip this question?") or
/// dismiss one; it never runs a command by itself.
enum GrillUnderstanding {
    /// Longer replies are answers; the model isn't asked (it would only add delay).
    static let maxWordsForModel = 12
    /// In replies this short, the model can't dismiss the matcher's question.
    static let maxWordsMatcherKeeps = 4

    /// `model`: (reply, question) → one of answer/skip/back/pause/resume/end/summarize/decide/repeat, or nil
    /// when the model isn't available, is too slow or fails.
    static func read(_ text: String, question: String, phrases: GrillPhrases?,
                     model: ((String, String) async -> String?)?) async -> GrillMatcher.Match? {
        let m = GrillMatcher.match(text, phrases: phrases)
        if m?.confidence == .sure { return m }
        let words = VoiceText.normalize(text).split(separator: " ").count
        guard let model, words <= maxWordsForModel, let label = await model(text, question) else { return m }
        guard let intent = intent(label) else {
            // "It's an answer." In a short reply the matcher's phrase is the whole point ("tanto faz pra mim"),
            // so its question stays; in a longer one it's usually a word in a sentence ("next quarter we…").
            return words <= maxWordsMatcherKeeps ? m : nil
        }
        return GrillMatcher.Match(intent: intent, confidence: .ask)
    }

    static func intent(_ label: String) -> GrillMatcher.Intent? {
        if label == "repeat" { return .repeatQuestion }
        return GrillCommand(rawValue: label).map { .command($0) }
    }

    /// The evaluation label for a reading: the intent's name, or "answer".
    static func label(_ m: GrillMatcher.Match?) -> String {
        switch m?.intent {
        case .none: "answer"
        case .repeatQuestion: "repeat"
        case .command(let c): c.rawValue
        }
    }
}
