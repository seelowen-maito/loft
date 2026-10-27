import Foundation

/// Tells a spoken interview command ("skip", "pula", "say that again") from an answer, on the phone.
/// Same idea as the voice review's send/continue/discard: only a short utterance that is a command
/// and nothing else runs straight away; a near miss or a command at the start of a longer sentence
/// asks first; anything else is an answer.
enum GrillMatcher {
    /// What a spoken command does. `repeatQuestion`, skip ("next") and back stay on the phone; "yes" is an answer
    /// (it accepts the recommendation).
    enum Intent: Hashable {
        case command(GrillCommand)
        case repeatQuestion
    }

    enum Confidence: Hashable {
        /// Run it.
        case sure
        /// Ask "Skip this question?" before running it; "no" keeps the words as the answer.
        case ask
    }

    struct Match: Hashable {
        let intent: Intent
        let confidence: Confidence
    }

    /// The intents and their keys in the phrase catalog, in priority order for ambiguous near-matches.
    static let order: [(Intent, String)] = [
        (.repeatQuestion, "repeat"), (.command(.send), "send"), (.command(.skip), "skip"), (.command(.back), "back"),
        (.command(.pause), "pause"), (.command(.resume), "resume"), (.command(.end), "end"),
        (.command(.summarize), "summarize"), (.command(.decide), "decide"),
    ]

    /// The server's catalog as the app ships it (`Resources/grill_phrases.json`, a copy of `GET /grill/commands`
    /// the backend's tests keep equal), so matching works offline and before the server's list loads.
    static let bundled: GrillPhrases? = Bundle.main.url(forResource: "grill_phrases", withExtension: "json")
        .flatMap { try? Data(contentsOf: $0) }
        .flatMap { try? JSONCoding.decoder.decode(GrillPhrases.self, from: $0) }

    /// Words that don't change what was meant at the start or end: "um, skip please" is still skip.
    /// Only trimmed at the ends: "um" is a real word in "faz um resumo".
    static let fillers: Set<String> = ["um", "uh", "uhm", "hmm", "hm", "ok", "okay", "please", "so", "well", "just", "and",
                                       "e", "entao", "tipo", "ah", "eh", "por", "favor", "bom", "ta", "pode"]

    /// The intent of `text`, or nil when it's an answer.
    static func match(_ text: String, phrases: GrillPhrases? = nil) -> Match? {
        let words = folded(text).split(separator: " ").map(String.init)
        guard !words.isEmpty else { return nil }
        let fill = fillers.union(((phrases?.fillers ?? []) + (bundled?.fillers ?? [])).map(folded))
        let core = Array(words.drop { fill.contains($0) }.reversed().drop { fill.contains($0) }.reversed())
        guard !core.isEmpty else { return nil }
        let maxWords = phrases?.maxWords ?? bundled?.maxWords ?? 6
        let table = merged(phrases)

        // 1. The whole (short) utterance is a command.
        if core.count <= maxWords {
            let said = core.joined(separator: " ")
            if let hit = table.first(where: { $0.phrases.contains(said) }) { return Match(intent: hit.intent, confidence: .sure) }
            // A misheard word ("skop", "proxma"): ask.
            for entry in table {
                for p in entry.phrases where isNear(core, p.split(separator: " ").map(String.init)) {
                    return Match(intent: entry.intent, confidence: .ask)
                }
            }
            // A command inside a short utterance ("skip that one", "I really don't know"): ask.
            // A one-word command only counts when it comes first ("next month" asks; "the next one" doesn't).
            for entry in table {
                for p in entry.phrases {
                    let pw = p.split(separator: " ").map(String.init)
                    if pw.count == 1 ? core.first == pw[0] : containsWords(core, p) {
                        return Match(intent: entry.intent, confidence: .ask)
                    }
                }
            }
            return nil
        }
        // 2. A longer sentence that opens with a command ("skip, I don't care about that"): ask.
        // "I don't know" opening a sentence is the start of an answer ("I don't know if it scales").
        for entry in table where entry.intent != .command(.decide) {
            for p in entry.phrases {
                let pw = p.split(separator: " ").map(String.init)
                if core.count <= 10, core.starts(with: pw), endsClause(text, after: pw.count) {
                    return Match(intent: entry.intent, confidence: .ask)
                }
            }
        }
        return nil
    }

    // MARK: Helpers

    private struct Entry { let intent: Intent; let phrases: Set<String> }

    /// The bundled catalog plus the server's (an older or newer server may list other phrases).
    private static func merged(_ server: GrillPhrases?) -> [Entry] {
        order.map { intent, key in
            let all = (bundled?.phrases(forKey: key) ?? []) + (server?.phrases(forKey: key) ?? [])
            return Entry(intent: intent, phrases: Set(all.map(folded)))
        }
    }

    /// `VoiceText.normalize` without apostrophes, as the server normalises its phrases ("don't" is "dont").
    private static func folded(_ s: String) -> String {
        VoiceText.normalize(s).replacingOccurrences(of: "'", with: "")
    }

    /// Same number of words, each equal or one edit away (only for words of 4+ letters).
    private static func isNear(_ a: [String], _ b: [String]) -> Bool {
        guard a.count == b.count, a != b else { return false }
        return zip(a, b).allSatisfy { x, y in x == y || (min(x.count, y.count) >= 4 && levenshtein(x, y) <= 1) }
    }

    private static func containsWords(_ words: [String], _ phrase: String) -> Bool {
        let p = phrase.split(separator: " ").map(String.init)
        guard p.count <= words.count else { return false }
        return (0...(words.count - p.count)).contains { Array(words[$0..<$0 + p.count]) == p }
    }

    /// True when the original text has a pause mark (comma, period, dash) right after the first `n` words.
    private static func endsClause(_ text: String, after n: Int) -> Bool {
        var seen = 0
        var inWord = false
        for ch in text {
            if ch.isLetter || ch.isNumber || ch == "'" || ch == "’" {
                if !inWord { inWord = true }
            } else {
                if inWord { seen += 1; inWord = false }
                if seen == n { return [",", ".", ";", ":", "-", "–", "—", "!"].contains(ch) }
                if seen > n { return false }
            }
        }
        return false
    }

    static func levenshtein(_ a: String, _ b: String) -> Int {
        let a = Array(a), b = Array(b)
        guard !a.isEmpty else { return b.count }
        guard !b.isEmpty else { return a.count }
        var prev = Array(0...b.count)
        for i in 1...a.count {
            var cur = [i] + Array(repeating: 0, count: b.count)
            for j in 1...b.count {
                cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] == b[j - 1] ? 0 : 1))
            }
            prev = cur
        }
        return prev[b.count]
    }
}
