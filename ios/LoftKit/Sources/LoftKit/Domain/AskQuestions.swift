import Foundation

/// An agent's multiple-choice questions (Claude Code's AskUserQuestion), asked as a `questions` request once it may
/// ask them. The picks go back as the tool's `answers`, keyed by question text, the same way the web sends them
/// (frontend/src/lib/questions.ts).
public struct AskQuestion: Equatable {
    public struct Option: Equatable {
        public var label: String
        public var description: String?
    }

    /// One question's state on the card: the options picked, and what's typed under "Something else".
    public struct Pick: Equatable {
        public var chosen: [String] = []
        public var other = ""
        public init(chosen: [String] = [], other: String = "") { self.chosen = chosen; self.other = other }

        /// A single-choice question keeps only the option (picking it again clears it); a multi-choice one toggles it.
        public func toggling(_ label: String, multi: Bool) -> Pick {
            let has = chosen.contains(label)
            if multi { return Pick(chosen: has ? chosen.filter { $0 != label } : chosen + [label], other: other) }
            return Pick(chosen: has ? [] : [label], other: has ? other : "")
        }

        /// Typed text replaces a single-choice pick and joins a multi-choice one.
        public func typing(_ text: String, multi: Bool) -> Pick {
            Pick(chosen: multi || text.trimmingCharacters(in: .whitespaces).isEmpty ? chosen : [], other: text)
        }

        /// The answer text: the chosen labels, then the typed text, joined by ", ".
        var answer: String { (chosen + [other.trimmingCharacters(in: .whitespacesAndNewlines)]).filter { !$0.isEmpty }.joined(separator: ", ") }
    }

    public var question: String
    public var header: String?
    public var multiSelect: Bool
    public var options: [Option]

    /// The questions in a request's input, keeping only string text: the input comes from the agent.
    public static func of(_ input: JSONValue?) -> [AskQuestion] {
        guard case .array(let qs)? = input?["questions"] else { return [] }
        return qs.compactMap { q in
            guard let text = q["question"]?.stringValue else { return nil }
            let options: [Option]
            if case .array(let os)? = q["options"] {
                options = os.compactMap { o in o["label"]?.stringValue.map { Option(label: $0, description: o["description"]?.stringValue) } }
            } else { options = [] }
            return AskQuestion(question: text, header: q["header"]?.stringValue, multiSelect: q["multiSelect"]?.boolValue == true, options: options)
        }
    }

    /// The `answers` to send, or nil until every question has one.
    public static func answers(_ qs: [AskQuestion], picks: [Int: Pick]) -> [String: String]? {
        var out: [String: String] = [:]
        for (i, q) in qs.enumerated() {
            let a = picks[i]?.answer ?? ""
            if a.isEmpty { return nil }
            out[q.question] = a
        }
        return out
    }
}
