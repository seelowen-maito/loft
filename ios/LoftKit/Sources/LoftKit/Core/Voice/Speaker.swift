import AVFoundation
import Foundation

/// Reads agent replies aloud with Apple's on-device voices, in the language each
/// reply is written in. Code, links and Markdown are left out.
///
/// The one way into reading aloud: every caller goes through `speak` or `readReply`. One reading plays at a
/// time; a new request waits for the one playing unless it asks to `replace` it, and `stop()` drops them all.
@MainActor
@Observable
public final class Speaker: NSObject {
    public private(set) var isSpeaking = false
    /// The text being read now, for showing alongside the audio.
    public private(set) var current: String?
    /// The id of the message being read, so its bubble can show a stop button.
    public private(set) var messageId: Int64?
    /// Part of a hands-free conversation: it can be stopped or skipped, by voice or with the bar's buttons.
    public private(set) var interruptible = false
    /// Read short, with the rest waiting for "more" (the bar shows a More button).
    public private(set) var hasMore = false
    /// A message that should have been read aloud but couldn't be, for "Couldn't read aloud · Retry" on it.
    public private(set) var unreadMessageId: Int64?
    /// Why the last `speak` produced no speech; nil when it was heard (or stopped on purpose).
    public private(set) var lastFailure: String?
    @ObservationIgnored private var synthStarted = false
    /// `stop()` was called while speaking: silence then isn't a failure.
    @ObservationIgnored private var stoppedOnPurpose = false
    /// The interview's extra command phrases, for recognizing them while reading.
    @ObservationIgnored public var phrases: GrillPhrases?

    /// How long a reply must stop changing before it's read: the one setting for "the whole reply is in".
    public nonisolated static let settle: TimeInterval = 0.8

    @ObservationIgnored private let synth = AVSpeechSynthesizer()
    @ObservationIgnored private let turns = ReadingTurns()
    /// The utterance playing now: the end of an earlier one (it arrives late) mustn't end this reading.
    @ObservationIgnored private var utterance: ObjectIdentifier?
    @ObservationIgnored private var finished: CheckedContinuation<Void, Never>?
    @ObservationIgnored private var reader: InterruptibleReader?
    /// The audio held from `AudioSessionOwner` while the plain voice reads.
    @ObservationIgnored private var lease: UUID?
    /// How the current reading was cut short, returned by `speak`.
    @ObservationIgnored private var endedBy: ReadAloudCommand?
    @ObservationIgnored private let audio: AudioSessionOwner

    init(audio: AudioSessionOwner? = nil) {
        self.audio = audio ?? .shared
        super.init()
        synth.delegate = self
    }

    /// A message's speak button (or Retry): it replaces whatever is being read.
    public func speak(_ message: ChatMessage) {
        Task { await speak(message.displayContent, messageId: message.id, replace: true) }
    }

    /// Speaks and returns once done (or stopped). With `interruptible`, it listens meanwhile (when that's
    /// on in Settings and VoiceOver isn't running) and returns the command that cut it short, if any.
    /// It waits for a reading in progress to end first, unless `replace` stops it.
    @discardableResult
    public func speak(_ markdown: String, messageId: Int64? = nil, interruptible: Bool = false, hasMore: Bool = false,
               replace: Bool = false) async -> ReadAloudCommand? {
        await turn(replace: replace) {
            await self.play(markdown, messageId: messageId, interruptible: interruptible, hasMore: hasMore)
        }
    }

    private func turn(replace: Bool, _ work: @escaping @MainActor () async -> ReadAloudCommand?) async -> ReadAloudCommand? {
        if replace { stop() } else if isSpeaking { VoiceLog.shared.add("read-aloud: waiting for the reading in progress") }
        return await turns.run(work) ?? nil
    }

    private func play(_ markdown: String, messageId: Int64?, interruptible: Bool, hasMore: Bool) async -> ReadAloudCommand? {
        let (text, truncated) = VoiceText.speakable(markdown)
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        let locale = UserDefaults.standard.string(forKey: Prefs.speechLocale).map(Locale.init(identifier:)) ?? Locale.current
        let listens = interruptible && UserDefaults.standard.bool(forKey: Prefs.voiceInterrupt) && !Platform.isVoiceOverRunning
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
        let problem = await takeAudio()
        // Stopped while the audio was being handed over, or another request took it before this one held it:
        // ended on purpose, not a failure to report.
        guard isSpeaking else { done(); return endedBy }
        if let problem { return failed(messageId, problem) }
        synthStarted = false
        await withCheckedContinuation { c in
            finished = c
            utterance = ObjectIdentifier(u)
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
    public func readReply(_ markdown: String, messageId: Int64? = nil, replace: Bool = false,
                   isActive: @escaping @MainActor () -> Bool) async -> ReadAloudCommand? {
        await turn(replace: replace) { await self.playReply(markdown, messageId: messageId, isActive: isActive) }
    }

    private func playReply(_ markdown: String, messageId: Int64?, isActive: () -> Bool) async -> ReadAloudCommand? {
        var parts: (text: String, rest: String?) = UserDefaults.standard.bool(forKey: Prefs.voiceShortRead) ? VoiceText.shortRead(markdown) : (markdown, nil)
        // A long reply: the on-device model's few sentences instead of its opening; "more" then reads it all.
        if parts.rest != nil, let spoken = await OnDeviceFeatures.spokenVersion(of: markdown) {
            parts = (spoken, VoiceText.speakable(markdown, limit: 100_000).text)
        }
        var next: (text: String, rest: String?)? = parts
        while let part = next, isActive() {
            next = nil
            switch await play(part.text, messageId: messageId, interruptible: true, hasMore: part.rest != nil) {
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
    public func holdFixture(text: String, messageId: Int64?, hasMore: Bool) {
        current = text
        self.messageId = messageId
        interruptible = true
        self.hasMore = hasMore
        isSpeaking = true
    }
    #endif

    /// Cut the reading short with a command (said, or tapped on the bar); `speak` returns it.
    public func interrupt(_ c: ReadAloudCommand) {
        guard isSpeaking else { return }
        endedBy = c
        stopPlaying()
    }

    /// Stops the reading and drops the ones waiting their turn.
    public func stop() {
        turns.cancelAll()
        stopPlaying()
    }

    private func stopPlaying() {
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
        utterance = nil
        if let lease { audio.release(lease) }
        lease = nil
        finished?.resume()
        finished = nil
    }

    /// Takes the audio from the owner for reading aloud (the mic hands it over). Returns why it couldn't.
    private func takeAudio() async -> String? {
        do {
            lease = try await audio.acquire(.speaking, by: "read-aloud") { [weak self] _ in self?.stopPlaying() }
            return nil
        } catch {
            return (error as? VoiceError)?.message ?? error.localizedDescription
        }
    }

    /// Reads a short sample in one voice, for the voice picker. A tap there is explicit, so it isn't queued
    /// through `turns`: `stop()` drops the readings waiting and cuts the one playing.
    public func preview(_ voice: AVSpeechSynthesisVoice) async {
        stop()
        if let problem = await takeAudio() {
            VoiceLog.shared.add("read-aloud: couldn't preview the voice: \(problem)")
            return
        }
        isSpeaking = true
        let u = AVSpeechUtterance(string: VoiceChoice.sample(for: voice.language))
        u.voice = voice
        await withCheckedContinuation { c in
            finished = c
            utterance = ObjectIdentifier(u)
            synth.speak(u)
        }
    }

    /// The synthesizer's end of `u`: only the utterance playing now ends the reading.
    private func ended(_ u: ObjectIdentifier) {
        guard u == utterance else { return VoiceLog.shared.add("read-aloud: ignored the end of an earlier reading") }
        done()
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
    public static func voice(for text: String) -> AVSpeechSynthesisVoice? {
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
    public nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didStart u: AVSpeechUtterance) {
        Task { @MainActor in self.synthStarted = true }
    }

    public nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish u: AVSpeechUtterance) {
        let id = ObjectIdentifier(u)
        Task { @MainActor in self.ended(id) }
    }

    public nonisolated func speechSynthesizer(_ s: AVSpeechSynthesizer, didCancel u: AVSpeechUtterance) {
        let id = ObjectIdentifier(u)
        Task { @MainActor in self.ended(id) }
    }
}

/// One reading at a time, in the order asked. `cancelAll` drops the ones still waiting (the one playing is
/// stopped by its owner); they return nil without running.
@MainActor
public final class ReadingTurns {
    private var tail: Task<Void, Never>?
    private var generation = 0

    public func run<T>(_ work: @escaping @MainActor () async -> T) async -> T? {
        let mine = generation
        let previous = tail
        let task = Task { @MainActor () -> T? in
            await previous?.value
            guard mine == self.generation else { return nil }
            return await work()
        }
        tail = Task { _ = await task.value }
        return await task.value
    }

    public func cancelAll() { generation += 1 }
}

/// A reply counts as whole once it has stopped changing for `delay`: pieces and edits restart the wait.
public struct ReplySettle {
    public var delay: TimeInterval = Speaker.settle
    private var text: String?
    private var since = Date.distantPast

    public init(delay: TimeInterval = Speaker.settle) { self.delay = delay }

    /// `candidate`: the reply as it stands now, nil while it can't be whole yet (the run still going).
    /// Returns it once it has been the same for `delay`.
    public mutating func whole(_ candidate: String?, at now: Date) -> String? {
        if candidate != text {
            text = candidate
            since = now
            return nil
        }
        guard let text, now.timeIntervalSince(since) >= delay else { return nil }
        return text
    }
}

/// An installed voice reduced to what voice selection needs, so the logic is testable without real voices.
public struct VoiceChoice: Hashable {
    public enum Quality: Int, Comparable {
        case standard, enhanced, premium
        public static func < (a: Self, b: Self) -> Bool { a.rawValue < b.rawValue }
        public var label: String {
            switch self {
            case .premium: "Premium"
            case .enhanced: "Enhanced"
            case .standard: "Default"
            }
        }
    }

    public let identifier: String
    public let name: String
    /// BCP 47, e.g. "pt-BR".
    public let language: String
    public let quality: Quality
    /// Sound-effect voices (Bells, Bubbles, Boing…): never picked automatically, hidden in the picker.
    public let isNovelty: Bool

    public init(identifier: String, name: String, language: String, quality: Quality, isNovelty: Bool = false) {
        self.identifier = identifier
        self.name = name
        self.language = language
        self.quality = quality
        self.isNovelty = isNovelty
    }

    public init(_ v: AVSpeechSynthesisVoice) {
        self.init(identifier: v.identifier, name: v.name, language: v.language,
                  quality: v.quality == .premium ? .premium : v.quality == .enhanced ? .enhanced : .standard,
                  isNovelty: v.voiceTraits.contains(.isNoveltyVoice))
    }

    /// "pt" for "pt-BR".
    public var languageCode: String { String(language.prefix { $0 != "-" && $0 != "_" }) }

    /// The voice to read `language` text with: the chosen one if it's installed and speaks that
    /// language (a Portuguese reply isn't read by an English voice), else the automatic pick.
    /// Nil when nothing installed speaks the language.
    public static func pick(language: String, region: String?, chosenId: String?, voices: [VoiceChoice]) -> String? {
        if let chosenId, !chosenId.isEmpty, let chosen = voices.first(where: { $0.identifier == chosenId }),
           chosen.languageCode == language {
            return chosen.identifier
        }
        return automatic(language: language, region: region, voices: voices)?.identifier
    }

    /// Best quality first, then the preferred region; ties keep the system's order.
    public static func automatic(language: String, region: String?, voices: [VoiceChoice]) -> VoiceChoice? {
        func score(_ v: VoiceChoice) -> Int {
            let q = v.quality == .premium ? 20 : v.quality == .enhanced ? 10 : 0
            let r = region.map { v.language.hasSuffix($0) } == true ? 5 : 0
            return q + r
        }
        return voices.filter { $0.languageCode == language && !$0.isNovelty }.max { score($0) < score($1) }
    }

    /// The picker's sections: one per language, the user's own language first and the rest by name;
    /// within a language, Premium, then Enhanced, then Default, then by name.
    public static func grouped(_ voices: [VoiceChoice], firstLanguage: String) -> [(language: String, voices: [VoiceChoice])] {
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
    public static func sample(for language: String) -> String {
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
