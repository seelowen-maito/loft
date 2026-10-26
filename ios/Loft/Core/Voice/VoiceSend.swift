import Foundation

/// What a dictation asks for, and when to ask. Nothing is ever sent because you went quiet: a spoken
/// "send" sends, "review" keeps the text to edit, and after a few seconds of quiet the app asks
/// "Would you like to submit this?" while it keeps listening. Shared by the chat's mic, the Talk
/// screen and interview answers.
enum VoiceSend {
    /// Quiet this long after some words: ask "Would you like to submit this?".
    static let askAfter: TimeInterval = 5
    /// …and when the dictation stopped on a word a sentence doesn't end on ("…and", "…porque").
    static let askAfterOpenEnding: TimeInterval = 8
    /// Words count once the recognizer has settled this long ("send" can be the start of "sending…").
    static let settle: TimeInterval = 0.8
    /// A gap this long separates utterances, so a command said after it counts as said on its own.
    static let utteranceGap: TimeInterval = 1.5
    /// With nothing said at all (or no answer to the question), the mic turns itself off after this long.
    static let noSpeechTimeout: TimeInterval = 20
    /// How long "Sending…" waits with Undo after you said yes to the question.
    static var undoWindow: TimeInterval {
        #if DEBUG
        // `-fixture-countdown-hold YES`: the bar stays up, for screenshots and VoiceOver checks.
        if UserDefaults.standard.bool(forKey: "fixture-countdown-hold") { return 3600 }
        #endif
        return 3
    }

    enum Command: Equatable {
        case send
        /// Stop listening and keep the text to edit.
        case review
        /// Ends the hands-free conversation: nothing is sent and the mic stays off.
        case stop
        /// Throw the dictation away and stop listening.
        case discard
        /// Throw the dictation away and keep listening for a new one.
        case startOver
    }

    /// A dictation so far: an explicit command, if one was said, and the text without it.
    struct Reading: Equatable {
        var command: Command?
        var text: String
        /// A command or an end phrase ("that's it", "pronto") finished it: act (or ask) now.
        var saidToFinish: Bool
    }

    // Said on their own (nothing else in the utterance), these are commands. One short word is
    // fine here: "send" alone can't be part of a message.
    static let sendAlone = ["send", "send it", "send now", "send to orchestrator", "send it to the orchestrator", "yes send",
                            "submit", "submit it", "submit this", "go ahead", "go ahead and send", "send that",
                            "envia", "enviar", "envie", "manda", "mandar", "pode mandar", "pode enviar", "envia agora",
                            "manda pro orquestrador", "envia pro orquestrador", "envia para o orquestrador", "manda ai", "pode ir"]
    static let reviewAlone = ["review", "review it", "review first", "review before sending", "let me review", "wait let me review",
                              "let me check", "let me edit", "hold it",
                              "revisar", "revisa", "revisar antes", "quero revisar", "deixa eu revisar", "deixa eu ver", "deixa eu editar"]
    static let stopAlone = ["stop", "stop listening", "stop it", "goodbye", "bye",
                            "para", "parar", "pare", "chega", "tchau"]
    static let discardAlone = ["cancel", "cancel that", "cancel it", "discard", "discard it", "discard that", "delete that", "scrap that",
                               "never mind", "nevermind", "forget it",
                               "cancela", "cancelar", "descarta", "descartar", "apaga", "apaga isso", "esquece", "deixa pra la"]
    static let startOverAlone = ["start over", "start again", "restart", "let me start over",
                                 "comeca de novo", "recomeca", "recomecar", "vou recomecar"]
    // At the end of a longer dictation only distinctive phrases count, so "…and send the invoice"
    // or "…then review it" stay part of the message.
    static let sendAtEnd = ["send to orchestrator", "send it to the orchestrator", "send it now", "send now", "submit it now", "go ahead and send it",
                            "manda pro orquestrador", "envia pro orquestrador", "envia para o orquestrador", "pode enviar", "pode mandar"]
    static let reviewAtEnd = ["review before sending", "let me review", "wait let me review",
                              "revisar antes", "quero revisar", "deixa eu revisar"]

    /// Words dropped from the start or end of a command ("ok, send", "send please").
    private static let fillers: Set<String> = ["ok", "okay", "um", "uh", "please", "now", "then", "so", "ta", "entao", "por", "favor", "agora"]

    /// What `text` asks for once dictation ends.
    static func read(_ text: String) -> Reading {
        let words = VoiceText.normalize(text).split(separator: " ").map(String.init)
        let core = Array(words.drop { fillers.contains($0) }.reversed().drop { fillers.contains($0) }.reversed())
        let joined = core.joined(separator: " ")
        if !joined.isEmpty, sendAlone.map(VoiceText.normalize).contains(joined) {
            return Reading(command: .send, text: "", saidToFinish: true)
        }
        if !joined.isEmpty, reviewAlone.map(VoiceText.normalize).contains(joined) {
            return Reading(command: .review, text: "", saidToFinish: true)
        }
        if !joined.isEmpty, stopAlone.map(VoiceText.normalize).contains(joined) {
            return Reading(command: .stop, text: "", saidToFinish: true)
        }
        if !joined.isEmpty, discardAlone.map(VoiceText.normalize).contains(joined) {
            return Reading(command: .discard, text: "", saidToFinish: true)
        }
        if !joined.isEmpty, startOverAlone.map(VoiceText.normalize).contains(joined) {
            return Reading(command: .startOver, text: "", saidToFinish: true)
        }
        for (command, phrases) in [(Command.send, sendAtEnd), (.review, reviewAtEnd)] {
            if let rest = strip(phrases, from: text) {
                return Reading(command: command, text: rest, saidToFinish: true)
            }
        }
        if let rest = VoiceText.strippingEndPhrase(text) {
            return Reading(command: nil, text: rest, saidToFinish: true)
        }
        return Reading(command: nil, text: text.trimmingCharacters(in: .whitespacesAndNewlines), saidToFinish: false)
    }

    /// What the words said last ask for. A command counts when it's the whole utterance after the latest
    /// gap (`sincePause`: the transcript as it was then) or a distinctive phrase ending the whole dictation.
    /// The returned text is the whole dictation without the command words.
    static func command(in transcript: String, sincePause base: String) -> Reading {
        if let added = added(since: base, now: transcript) {
            let alone = read(added)
            if let c = alone.command, alone.text.isEmpty {
                return Reading(command: c, text: base.trimmingCharacters(in: .whitespacesAndNewlines), saidToFinish: true)
            }
        }
        return read(transcript)
    }

    /// The words in `now` that came after `base` (what was said since then), or nil when nothing new was
    /// said: the recognizer rewriting punctuation or a word isn't new speech.
    static func added(since base: String, now: String) -> String? {
        let count = { (s: String) in s.split(whereSeparator: \.isWhitespace).filter { !VoiceText.normalize(String($0)).isEmpty }.count }
        let extra = count(now) - count(base)
        guard extra > 0 else { return nil }
        let words = now.split(whereSeparator: \.isWhitespace).filter { !VoiceText.normalize(String($0)).isEmpty }
        return words.suffix(extra).joined(separator: " ")
    }

    /// `text` without a trailing phrase from `phrases` (and a filler or two around it), keeping the
    /// rest's punctuation and case; nil when it doesn't end with one.
    private static func strip(_ phrases: [String], from text: String) -> String? {
        // Words, without bare punctuation ("—").
        let all = text.split(whereSeparator: \.isWhitespace).map(String.init).filter { !VoiceText.normalize($0).isEmpty }
        // As said first ("…send it now": "now" is part of the phrase), then without trailing fillers ("…send to orchestrator, please").
        var trimmed = all
        while let last = trimmed.last, fillers.contains(VoiceText.normalize(last)) { trimmed.removeLast() }
        for words in [all, trimmed] {
            if let rest = strip(phrases, fromWords: words) { return rest }
        }
        return nil
    }

    private static func strip(_ phrases: [String], fromWords words: [String]) -> String? {
        let norm = words.map(VoiceText.normalize)
        for p in phrases {
            let pw = VoiceText.normalize(p).split(separator: " ").map(String.init)
            guard norm.count > pw.count, Array(norm.suffix(pw.count)) == pw else { continue }
            var rest = Array(words.dropLast(pw.count))
            while let last = rest.last, fillers.contains(VoiceText.normalize(last)) { rest.removeLast() }
            let out = rest.joined(separator: " ").trimmingCharacters(in: CharacterSet(charactersIn: " ,;:-–—"))
            return out.isEmpty ? nil : out
        }
        return nil
    }

    // MARK: - "Would you like to submit this?"

    /// The question, in the dictation's language.
    static func question(for text: String) -> String {
        VoiceText.dominantLanguage(text) == "pt" ? "Quer enviar isto?" : "Would you like to submit this?"
    }

    enum Answer: Equatable {
        case yes
        case no
        /// "Cancel": throw it away.
        case discard
    }

    static let yesWords = ["yes", "yeah", "yep", "yup", "sure", "yes please", "yes send", "yes send it", "please do", "do it",
                           "sim", "sim manda", "sim envia", "pode", "claro", "isso", "manda", "envia"] + sendAlone
    static let noWords = ["no", "nope", "not yet", "no thanks", "no wait", "wait", "hold on", "not now",
                          "nao", "ainda nao", "agora nao", "espera", "peraí", "perai"] + reviewAlone

    /// The answer to the question, or nil when it's more of the message (or anything else).
    static func answer(_ text: String) -> Answer? {
        let words = VoiceText.normalize(text).split(separator: " ").map(String.init)
        let said = words.drop { fillers.contains($0) }.reversed().drop { fillers.contains($0) }.reversed().joined(separator: " ")
        guard !said.isEmpty else { return nil }
        if yesWords.map(VoiceText.normalize).contains(said) { return .yes }
        if noWords.map(VoiceText.normalize).contains(said) { return .no }
        if discardAlone.map(VoiceText.normalize).contains(said) { return .discard }
        return nil
    }

    /// When to ask: once after `askAfter` of quiet following new words, and not again until more words come.
    struct Prompt: Equatable {
        private(set) var lastWordsAt: Date
        private(set) var asked = false
        /// How long this quiet spell waits before asking: `askAfter`, or `askAfterOpenEnding`.
        private(set) var wait = VoiceSend.askAfter

        init(startedAt: Date) { lastWordsAt = startedAt }

        mutating func heardWords(at date: Date) {
            lastWordsAt = date
            asked = false
            wait = VoiceSend.askAfter
        }

        /// The words so far stop mid-sentence: wait longer before asking. Only when to ask, never a send.
        mutating func leftOpen() { wait = VoiceSend.askAfterOpenEnding }

        func shouldAsk(hasText: Bool, now: Date) -> Bool {
            hasText && !asked && now.timeIntervalSince(lastWordsAt) >= wait
        }

        mutating func didAsk() { asked = true }
    }

    /// Words a sentence doesn't end on, per language. Only ones that can't close one: English "do", "so", "on",
    /// "no" can ("what should I do", "I think so", "turn it on", "the answer is no"). Measured on 48 dictations,
    /// this catches half of the unfinished ones and none of the finished. Apple's on-device model was tried for
    /// the rest and called all 48 complete, so it isn't used.
    private static let openEnglish: Set<String> = [
        "and", "but", "or", "because", "with", "the", "a", "an", "of", "for", "to", "which", "if", "when", "that's",
        "is", "are", "was", "should", "would", "could", "will", "can", "um", "uh", "my", "our", "your", "their",
    ]
    private static let openPortuguese: Set<String> = [
        "e", "mas", "ou", "porque", "com", "para", "pra", "pro", "o", "a", "os", "as", "de", "do", "da", "dos", "das",
        "no", "na", "nos", "nas", "pelo", "pela", "um", "uma", "que", "quando", "se", "é", "deveria", "vai", "meu", "minha",
    ]

    /// The dictation stops mid-sentence: a trailing comma or colon, or a last word like "and" or "porque".
    static func endsOpen(_ text: String) -> Bool {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return false }
        if t.hasSuffix(",") || t.hasSuffix(":") || t.hasSuffix("…") || t.hasSuffix("...") { return true }
        let last = t.lowercased().split(whereSeparator: { !$0.isLetter && $0 != "'" }).last.map(String.init) ?? ""
        return (VoiceText.dominantLanguage(t) == "pt" ? openPortuguese : openEnglish).contains(last)
    }
}
