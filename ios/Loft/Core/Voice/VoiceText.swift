import Foundation
import NaturalLanguage

/// Pure text helpers for voice sessions, kept free of audio so they can be tested.
enum VoiceText {
    /// Said at the end of a dictation to finish it. Matched case- and accent-insensitively, ignoring punctuation.
    static let endPhrases = [
        "i'm done", "im done", "i am done", "that's it", "thats it", "that's all", "over and out",
        "terminei", "acabei", "pronto", "é isso", "e isso", "é só isso", "e so isso",
    ]

    static func normalize(_ s: String) -> String {
        let folded = s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil)
            .replacingOccurrences(of: "’", with: "'")
        let kept = folded.unicodeScalars.map { CharacterSet.letters.contains($0) || CharacterSet.decimalDigits.contains($0) || $0 == "'" ? Character($0) : " " }
        return String(kept).split(separator: " ").joined(separator: " ")
    }

    /// If `text` ends with an end phrase, the text without it; otherwise nil.
    static func strippingEndPhrase(_ text: String) -> String? {
        let norm = normalize(text)
        for phrase in endPhrases.map(normalize) where norm == phrase || norm.hasSuffix(" " + phrase) {
            // Drop the same number of words from the original, keeping its punctuation and case.
            let count = phrase.split(separator: " ").count
            var words = text.split(whereSeparator: \.isWhitespace)
            words.removeLast(min(count, words.count))
            return words.joined(separator: " ").trimmingCharacters(in: CharacterSet(charactersIn: " ,;:-–—"))
        }
        return nil
    }

    /// Joins recognizer segments. Each restart of Apple's recognizer starts a new segment.
    static func join(_ segments: [String]) -> String {
        segments.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// What is worth reading aloud from an agent's Markdown reply: no code blocks, inline code
    /// markers, URLs, tables or Markdown syntax. Long replies are cut at a paragraph boundary.
    static func speakable(_ markdown: String, limit: Int = 900) -> (text: String, truncated: Bool) {
        var lines: [String] = []
        var inFence = false
        for raw in markdown.components(separatedBy: .newlines) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") || line.hasPrefix("~~~") {
                if !inFence { lines.append("(code omitted)") }
                inFence.toggle()
                continue
            }
            if inFence || line.hasPrefix("|") { continue }
            var l = line
            while let r = l.range(of: #"^(#{1,6}\s+|[-*+]\s+|\d+[.)]\s+|>\s*)"#, options: .regularExpression) { l.removeSubrange(r) }
            l = l.replacingOccurrences(of: #"\[([^\]]+)\]\([^)]+\)"#, with: "$1", options: .regularExpression)
            l = l.replacingOccurrences(of: #"https?://\S+"#, with: "a link", options: .regularExpression)
            l = l.replacingOccurrences(of: #"[*_`~]+"#, with: "", options: .regularExpression)
            lines.append(l)
        }
        let paragraphs = lines.split(separator: "", omittingEmptySubsequences: true).map { $0.joined(separator: " ") }
        var out = ""
        for p in paragraphs {
            if !out.isEmpty && out.count + p.count > limit {
                return (out, true)
            }
            out += (out.isEmpty ? "" : "\n\n") + p
        }
        if out.count > limit * 2, let cut = out.prefix(limit).lastIndex(where: { ".!?".contains($0) }) {
            return (String(out[...cut]), true)
        }
        return (out, false)
    }

    /// A reply split for reading the short way: its opening (the first paragraph, cut at a sentence when
    /// that's long) and the rest, nil when the opening is all there is. Both are already speakable.
    static func shortRead(_ markdown: String, limit: Int = 320) -> (text: String, rest: String?) {
        let full = speakable(markdown, limit: 100_000).text
        let paragraphs = full.components(separatedBy: "\n\n").filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        guard let first = paragraphs.first else { return (full, nil) }
        var opening = first
        var restOfFirst = ""
        if first.count > limit {
            var sentences: [String] = []
            first.enumerateSubstrings(in: first.startIndex..., options: .bySentences) { s, _, _, _ in
                if let s { sentences.append(s) }
            }
            var taken = 0
            var count = 0
            for s in sentences {
                if taken > 0 && count + s.count > limit { break }
                count += s.count
                taken += 1
            }
            opening = sentences.prefix(taken).joined().trimmingCharacters(in: .whitespaces)
            restOfFirst = sentences.dropFirst(taken).joined().trimmingCharacters(in: .whitespaces)
        }
        let rest = ([restOfFirst] + paragraphs.dropFirst()).filter { !$0.isEmpty }.joined(separator: "\n\n")
        return (opening, rest.isEmpty ? nil : rest)
    }

    /// The language a reply is mostly written in, for picking a voice.
    static func dominantLanguage(_ text: String) -> String? {
        let r = NLLanguageRecognizer()
        r.languageConstraints = [.english, .portuguese, .spanish, .french, .german, .italian]
        r.processString(text)
        return r.dominantLanguage?.rawValue
    }
}
