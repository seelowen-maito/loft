import AVFoundation
import Foundation
import UIKit

/// Reads agent replies aloud with Apple's on-device voices, in the language each
/// reply is written in. Code, links and Markdown are left out.
@MainActor
@Observable
final class Speaker: NSObject {
    private(set) var isSpeaking = false
    /// The text being read now, for showing alongside the audio.
    private(set) var current: String?
    /// The id of the message being read, so its bubble can show a stop button.
    private(set) var messageId: Int64?
    /// Part of a hands-free conversation: it can be stopped or skipped, by voice or with the bar's buttons.
    private(set) var interruptible = false
    /// Read short, with the rest waiting for "more" (the bar shows a More button).
    private(set) var hasMore = false
    /// A message that should have been read aloud but couldn't be, for "Couldn't read aloud · Retry" on it.
    private(set) var unreadMessageId: Int64?
    /// Why the last `speak` produced no speech; nil when it was heard (or stopped on purpose).
    private(set) var lastFailure: String?
    @ObservationIgnored private var synthStarted = false
    /// `stop()` was called while speaking: silence then isn't a failure.
    @ObservationIgnored private var stoppedOnPurpose = false
    /// The interview's extra command phrases, for recognizing them while reading.
    @ObservationIgnored var phrases: GrillPhrases?

    @ObservationIgnored private let synth = AVSpeechSynthesizer()
    @ObservationIgnored private var finished: CheckedContinuation<Void, Never>?
    @ObservationIgnored private var reader: InterruptibleReader?
    /// How the current reading was cut short, returned by `speak`.
    @ObservationIgnored private var endedBy: ReadAloudCommand?

    override init() {
        super.init()
        synth.delegate = self
    }

    func speak(_ message: ChatMessage) {
        Task { await speak(message.displayContent, messageId: message.id) }
    }

    /// Speaks and returns once done (or stopped). With `interruptible`, it listens meanwhile (when that's
    /// on in Settings and VoiceOver isn't running) and returns the command that cut it short, if any.
    @discardableResult
    func speak(_ markdown: String, messageId: Int64? = nil, interruptible: Bool = false, hasMore: Bool = false) async -> ReadAloudCommand? {
        stop()
        let (text, truncated) = VoiceText.speakable(markdown)
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        let locale = UserDefaults.standard.string(forKey: Prefs.speechLocale).map(Locale.init(identifier:)) ?? Locale.current
        let listens = interruptible && UserDefaults.standard.bool(forKey: Prefs.voiceInterrupt) && !UIAccessibility.isVoiceOverRunning
            && InterruptibleReader.canListen(locale: locale)
        let spoken = hasMore ? text + "\n\n" + Self.askForMore(text, byVoice: listens) : truncated ? text + "\n\n" + Self.moreInChat(text) : text
        lastFailure = nil
        stoppedOnPurpose = false
        current = text
        self.messageId = messageId
        self.interruptible = interruptible
        self.hasMore = hasMore
        endedBy = nil
        isSpeaking = true
        if TestMode.holdReading {
            await withCheckedContinuation { c in finished = c }
            return endedBy
        }
        let u = AVSpeechUtterance(string: spoken)
        u.voice = Self.voice(for: spoken)
        u.prefersAssistiveTechnologySettings = true
        if listens {
            let r = InterruptibleReader()
            r.phrases = phrases
            r.onCommand = { [weak self] c in self?.interrupt(c) }
            reader = r
            do {
                // After "Say more for the rest", a moment to say it.
                try await r.play(u, locale: locale, listenAfter: hasMore ? 3 : 0)
                reader = nil
                if r.startedAudio || endedBy != nil || !isSpeaking {
                    let e = endedBy
                    if r.startedAudio, unreadMessageId == messageId { unreadMessageId = nil }
                    done()
                    return e
                }
                // Nothing was heard: don't count silence as "read". Read it the plain way below.
                VoiceLog.shared.add("read-aloud: listening reader played nothing (\(r.failure ?? "unknown")); reading plainly")
            } catch {
                // Couldn't listen: read it the plain way; the bar's buttons still work.
                VoiceLog.shared.add("read-aloud: listening reader failed (\(error.localizedDescription)); reading plainly")
                r.stop()
                reader = nil
            }
        }
        // The mic (the composer or the Talk screen) may still be letting go of the audio: wait for it, briefly.
        if let problem = await activateSession() {
            return failed(messageId, "the audio wasn't available (\(problem))")
        }
        synthStarted = false
        await withCheckedContinuation { c in
            finished = c
            synth.speak(u)
        }
        if !synthStarted && endedBy == nil && !stoppedOnPurpose {
            return failed(messageId, "the voice didn't start")
        }
        if unreadMessageId == messageId { unreadMessageId = nil }
        return endedBy
    }

    private func failed(_ id: Int64?, _ why: String) -> ReadAloudCommand? {
        lastFailure = why
        if let id { unreadMessageId = id }
        VoiceLog.shared.add("read-aloud: couldn't read: \(why)")
        done()
        return nil
    }

    /// A reply in a hands-free conversation: the short version first when that's on, "more" (said or tapped)
    /// for the rest, "repeat" from the start. Returns `.stop` or `.skip` when one cut it short, nil when read through.
    @discardableResult
    func readReply(_ markdown: String, messageId: Int64? = nil, isActive: () -> Bool) async -> ReadAloudCommand? {
        var parts: (text: String, rest: String?) = UserDefaults.standard.bool(forKey: Prefs.voiceShortRead) ? VoiceText.shortRead(markdown) : (markdown, nil)
        // A long reply: the on-device model's few sentences instead of its opening; "more" then reads it all.
        if parts.rest != nil, let spoken = await OnDeviceFeatures.spokenVersion(of: markdown) {
            parts = (spoken, VoiceText.speakable(markdown, limit: 100_000).text)
        }
        var next: (text: String, rest: String?)? = parts
        while let part = next, isActive() {
            next = nil
            switch await speak(part.text, messageId: messageId, interruptible: true, hasMore: part.rest != nil) {
            case .repeatIt: next = parts
            case .more: next = part.rest.map { ($0, nil) }
            case .stop: return .stop
            case .skip: return .skip
            case nil: break
            }
        }
        return nil
    }

    #if DEBUG
    /// `-fixture-read-aloud YES`: the reading state without audio (the Simulator's voice ends at once), held
    /// until Skip/Stop/More is tapped, for screenshots and VoiceOver checks.
    func holdFixture(text: String, messageId: Int64?, hasMore: Bool) {
        current = text
        self.messageId = messageId
        interruptible = true
        self.hasMore = hasMore
        isSpeaking = true
    }
    #endif

    /// Cut the reading short with a command (said, or tapped on the bar); `speak` returns it.
    func interrupt(_ c: ReadAloudCommand) {
        guard isSpeaking else { return }
        endedBy = c
        stop()
    }

    func stop() {
        if isSpeaking { stoppedOnPurpose = true }
        if synth.isSpeaking { synth.stopSpeaking(at: .immediate) }
        reader?.stop()
        reader = nil
        done()
    }

    private func done() {
        isSpeaking = false
        interruptible = false
        hasMore = false
        current = nil
        messageId = nil
        finished?.resume()
        finished = nil
    }

    /// Makes the audio session ready for speech, retrying for up to a second while the mic lets go of it.
    /// Returns the error when it never became available.
    private func activateSession() async -> String? {
        let s = AVAudioSession.sharedInstance()
        var last: String?
        for attempt in 0..<5 {
            do {
                // Keep recording possible: a voice session listens again right after.
                if s.category != .playAndRecord {
                    try s.setCategory(.playback, mode: .spokenAudio, options: [.duckOthers])
                }
                try s.setActive(true)
                if attempt > 0 { VoiceLog.shared.add("read-aloud: audio ready after \(attempt * 200) ms") }
                return nil
            } catch {
                last = error.localizedDescription
                if attempt == 0 { VoiceLog.shared.add("read-aloud: waiting for the audio (\(error.localizedDescription))") }
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
        return last
    }

    /// Reads a short sample in one voice, for the voice picker.
    func preview(_ voice: AVSpeechSynthesisVoice) async {
        stop()
        _ = await activateSession()
        isSpeaking = true
        let u = AVSpeechUtterance(string: VoiceChoice.sample(for: voice.language))
        u.voice = voice
        await withCheckedContinuation { c in
            finished = c
            synth.speak(u)
        }
    }

    private static func askForMore(_ text: String, byVoice: Bool) -> String {
        let pt = VoiceText.dominantLanguage(text) == "pt"
        return byVoice ? (pt ? "Diga “mais” para ouvir o resto." : "Say “more” for the rest.")
            : (pt ? "Toque em Mais para ouvir o resto." : "Tap More for the rest.")
    }

    private static func moreInChat(_ text: String) -> String {
        VoiceText.dominantLanguage(text) == "pt" ? "O resto está no chat." : "The rest is in the chat."
    }

    /// The voice chosen in Settings when it speaks the text's language; otherwise the best installed
    /// voice for that language, preferring the user's region (pt-BR vs pt-PT) and Premium/Enhanced quality.
    static func voice(for text: String) -> AVSpeechSynthesisVoice? {
        let lang = VoiceText.dominantLanguage(text) ?? Locale.current.language.languageCode?.identifier ?? "en"
        let preferredRegion = (UserDefaults.standard.string(forKey: Prefs.speechLocale).map(Locale.init(identifier:)) ?? Locale.current)
            .region?.identifier
        let installed = AVSpeechSynthesisVoice.speechVoices()
        let chosen = VoiceChoice.pick(language: lang, region: preferredRegion,
                                      chosenId: UserDefaults.standard.string(forKey: Prefs.speechVoice),
                                      voices: installed.map(VoiceChoice.init))
        return chosen.flatMap { id in installed.first { $0.identifier == id } } ?? AVSpeechSynthesisVoice(language: lang)
    }
}

extension Speaker: AVSpeechSynthesizerDelegate {
    nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didStart u: AVSpeechUtterance) {
        Task { @MainActor in self.synthStarted = true }
    }

    nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish u: AVSpeechUtterance) {
        Task { @MainActor in self.done() }
    }

    nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didCancel u: AVSpeechUtterance) {
        Task { @MainActor in self.done() }
    }
}

/// An installed voice reduced to what voice selection needs, so the logic is testable without real voices.
struct VoiceChoice: Hashable {
    enum Quality: Int, Comparable {
        case standard, enhanced, premium
        static func < (a: Self, b: Self) -> Bool { a.rawValue < b.rawValue }
        var label: String {
            switch self {
            case .premium: "Premium"
            case .enhanced: "Enhanced"
            case .standard: "Default"
            }
        }
    }

    let identifier: String
    let name: String
    /// BCP 47, e.g. "pt-BR".
    let language: String
    let quality: Quality
    /// Sound-effect voices (Bells, Bubbles, Boing…): never picked automatically, hidden in the picker.
    let isNovelty: Bool

    init(identifier: String, name: String, language: String, quality: Quality, isNovelty: Bool = false) {
        self.identifier = identifier
        self.name = name
        self.language = language
        self.quality = quality
        self.isNovelty = isNovelty
    }

    init(_ v: AVSpeechSynthesisVoice) {
        self.init(identifier: v.identifier, name: v.name, language: v.language,
                  quality: v.quality == .premium ? .premium : v.quality == .enhanced ? .enhanced : .standard,
                  isNovelty: v.voiceTraits.contains(.isNoveltyVoice))
    }

    /// "pt" for "pt-BR".
    var languageCode: String { String(language.prefix { $0 != "-" && $0 != "_" }) }

    /// The voice to read `language` text with: the chosen one if it's installed and speaks that
    /// language (a Portuguese reply isn't read by an English voice), else the automatic pick.
    /// Nil when nothing installed speaks the language.
    static func pick(language: String, region: String?, chosenId: String?, voices: [VoiceChoice]) -> String? {
        if let chosenId, !chosenId.isEmpty, let chosen = voices.first(where: { $0.identifier == chosenId }),
           chosen.languageCode == language {
            return chosen.identifier
        }
        return automatic(language: language, region: region, voices: voices)?.identifier
    }

    /// Best quality first, then the preferred region; ties keep the system's order.
    static func automatic(language: String, region: String?, voices: [VoiceChoice]) -> VoiceChoice? {
        func score(_ v: VoiceChoice) -> Int {
            let q = v.quality == .premium ? 20 : v.quality == .enhanced ? 10 : 0
            let r = region.map { v.language.hasSuffix($0) } == true ? 5 : 0
            return q + r
        }
        return voices.filter { $0.languageCode == language && !$0.isNovelty }.max { score($0) < score($1) }
    }

    /// The picker's sections: one per language, the user's own language first and the rest by name;
    /// within a language, Premium, then Enhanced, then Default, then by name.
    static func grouped(_ voices: [VoiceChoice], firstLanguage: String) -> [(language: String, voices: [VoiceChoice])] {
        let byLang = Dictionary(grouping: voices, by: \.languageCode)
        let name = { (code: String) in Locale.current.localizedString(forLanguageCode: code) ?? code }
        return byLang.keys
            .sorted { a, b in
                if (a == firstLanguage) != (b == firstLanguage) { return a == firstLanguage }
                return name(a).localizedCaseInsensitiveCompare(name(b)) == .orderedAscending
            }
            .map { code in
                (code, byLang[code]!.sorted {
                    $0.quality != $1.quality ? $0.quality > $1.quality
                        : $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
                })
            }
    }

    /// A short line to preview a voice, in its own language.
    static func sample(for language: String) -> String {
        switch String(language.prefix(2)) {
        case "pt": "Olá! Vou ler as respostas dos seus agentes com esta voz."
        case "es": "¡Hola! Leeré las respuestas de tus agentes con esta voz."
        case "fr": "Bonjour ! Je lirai les réponses de vos agents avec cette voix."
        case "de": "Hallo! Ich lese die Antworten deiner Agenten mit dieser Stimme vor."
        case "it": "Ciao! Leggerò le risposte dei tuoi agenti con questa voce."
        case "en": "Hi! I'll read your agents' replies in this voice."
        default: Locale(identifier: language).localizedString(forLanguageCode: String(language.prefix(2))) ?? "Loft"
        }
    }
}
