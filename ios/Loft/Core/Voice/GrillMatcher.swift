import Foundation

/// Tells a spoken interview command ("skip", "pula", "say that again") from an answer, on the phone.
/// Same idea as the voice review's send/continue/discard: only a short utterance that is a command
/// and nothing else runs straight away; a near miss or a command at the start of a longer sentence
/// asks first; anything else is an answer.
enum GrillMatcher {
    /// What a spoken command does. `repeatQuestion` stays on the phone; "next" is the server's skip.
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

    /// Built in, so matching works offline and before the server's list loads. The server's list
    /// (`GET /grill/commands`) is added on top. In priority order for ambiguous near-matches.
    static let builtIn: [(Intent, [String])] = [
        (.repeatQuestion, ["repeat", "repeat that", "say that again", "again", "what was the question", "come again",
                          "repete", "repita", "de novo", "fala de novo", "qual era a pergunta", "nao entendi"]),
        (.command(.skip), ["skip", "skip it", "skip this", "skip this one", "skip this question", "next", "next question", "pass",
                          "pula", "pular", "pula essa", "proxima", "proxima pergunta", "passa"]),
        (.command(.back), ["go back", "back", "previous", "previous question", "last question",
                          "volta", "voltar", "anterior", "pergunta anterior"]),
        (.command(.pause), ["pause", "hold on", "wait", "pause the interview",
                           "pausa", "pausar", "espera", "peraí", "perai"]),
        (.command(.resume), ["resume", "continue", "carry on", "keep going", "go on",
                            "continua", "continuar", "retoma", "retomar", "segue"]),
        (.command(.end), ["stop", "end", "end it", "end the interview", "stop the interview", "i'm finished", "im finished", "that's enough", "thats enough",
                         "para", "parar", "chega", "encerra", "encerrar", "terminar", "acabou"]),
        (.command(.summarize), ["summarize", "summarise", "sum up", "recap", "where are we", "summary",
                               "resume tudo", "resumo", "resume ai", "onde estamos", "faz um resumo"]),
        (.command(.decide), ["i don't know", "i dont know", "don't know", "dont know", "no idea", "you decide", "your call", "decide for me", "up to you", "you choose",
                            "nao sei", "sei la", "decide voce", "voce decide", "tanto faz", "escolhe voce", "voce escolhe"]),
    ]

    /// Words that don't change what was meant at the start or end: "um, skip please" is still skip.
    /// Only trimmed at the ends: "um" is a real word in "faz um resumo".
    static let fillers: Set<String> = ["um", "uh", "uhm", "hmm", "hm", "ok", "okay", "please", "so", "well", "just", "and",
                                       "e", "entao", "tipo", "ah", "eh", "por", "favor", "bom", "ta", "pode"]

    /// The intent of `text`, or nil when it's an answer.
    static func match(_ text: String, phrases: GrillPhrases? = nil) -> Match? {
        let words = VoiceText.normalize(text).split(separator: " ").map(String.init)
        guard !words.isEmpty else { return nil }
        let fill = fillers.union((phrases?.fillers ?? []).map(VoiceText.normalize))
        let core = Array(words.drop { fill.contains($0) }.reversed().drop { fill.contains($0) }.reversed())
        guard !core.isEmpty else { return nil }
        let maxWords = phrases?.maxWords ?? 4
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

    private static func merged(_ server: GrillPhrases?) -> [Entry] {
        builtIn.map { intent, list in
            var all = Set(list.map(VoiceText.normalize))
            if let server {
                let key: String
                switch intent {
                case .command(let c): key = c.rawValue
                case .repeatQuestion: key = "repeat"
                }
                all.formUnion(server.phrases(forKey: key).map(VoiceText.normalize))
            }
            return Entry(intent: intent, phrases: all)
        }
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
