import Foundation

/// Suggested replies under an agent's message that asks you something.
enum ChatReplies {
    /// The message ends on a question: its last sentence, outside code, ends with a question mark ("Why did it
    /// fail? A missing key." answers its own question).
    static func asksSomething(_ text: String) -> Bool {
        let prose = text.components(separatedBy: "```").enumerated().filter { $0.offset % 2 == 0 }.map(\.element).joined(separator: "\n")
        let paragraphs = prose.components(separatedBy: "\n\n").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        guard let last = paragraphs.last else { return false }
        return last.trimmingCharacters(in: CharacterSet(charactersIn: " *_)\"”'")).hasSuffix("?")
    }

    private static let yesNo: Set<String> = ["yes", "no", "yeah", "nope", "sim", "não", "nao"]

    /// The question offers a choice ("merge now or wait for QA?"), so a bare "yes" or "no" means nothing.
    static func offersAChoice(_ text: String) -> Bool {
        let n = " " + VoiceText.normalize(text) + " "
        return n.contains(" or ") || n.contains(" ou ")
    }

    /// The model's usable replies, minus a bare yes or no to an either/or question. Fewer than two: none shown.
    static func shown(_ replies: [String], for text: String) -> [String] {
        let kept = offersAChoice(text) ? replies.filter { !yesNo.contains(VoiceText.normalize($0)) } : replies
        return kept.count >= 2 ? kept : []
    }
}
