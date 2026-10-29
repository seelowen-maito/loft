import AVFoundation
import Foundation
import Speech
import UIKit

/// A hands-free conversation with one agent: listen for as long as you talk,
/// clean the dictation up on the Mac, let you review it, send it on an explicit
/// "send", then read the agent's reply aloud and, in hands-free mode, listen again.
@MainActor
@Observable
final class VoiceSession {
    enum Phase: Equatable {
        case idle
        case preparing
        case listening
        case paused
        /// Finishing the transcript and running the cleanup pass.
        case cleaning
        /// The big preview: nothing is sent until `send` is called.
        case review
        /// Sent; waiting for the agent to answer.
        case sending
        case speaking
        /// Voice style E: the reply has been read (or arrived, with reading off) and stays on screen.
        case replied
        case failed(String)
    }

    private(set) var phase: Phase = .idle
    /// In `.failed`: true when the fix is allowing access in Loft's page in iOS Settings, so the UI can offer to open it.
    private(set) var failureNeedsPermission = false
    private(set) var agentId: String?
    /// Finished segments, joined.
    private(set) var committed = ""
    /// What's being heard right now.
    private(set) var live = ""
    /// Microphone level, 0…1, about 20 times a second.
    private(set) var level: Float = 0
    private(set) var elapsed: TimeInterval = 0
    /// When the message went out, for the status screen's timer.
    private(set) var sentAt: Date?
    /// The cleaned-up text, set in `.review`. Nil when cleanup failed or isn't available: show `raw`.
    private(set) var cleaned: String?
    private(set) var cleanupNote: String?
    /// In hands-free review, listening for "send" / "continue" / "discard".
    private(set) var awaitingVoiceConfirm = false
    /// The message that couldn't be sent, for "Retry sending" on the failure screen.
    private(set) var failedSend: PendingMessage?
    /// The agent's answer, once it arrives.
    private(set) var reply: String?
    /// Set while a finished dictation waits to go out by itself ("Sending… Undo"); nil otherwise.
    private(set) var autoSendAt: Date?

    var raw: String { VoiceText.join([committed, live]) }
    /// Waiting for, or reading, the answer to what was just sent: the Talk screen reads the next question itself.
    var readsReplies: Bool { phase == .sending || phase == .speaking }
    var isActive: Bool { ![.idle].contains(phase) && !isFailed }
    var isFailed: Bool { if case .failed = phase { return true } else { return false } }
    var endPhrases: [String] { VoiceText.endPhrases }

    var handsFree: Bool {
        get { UserDefaults.standard.bool(forKey: Prefs.voiceHandsFree) }
        set { UserDefaults.standard.set(newValue, forKey: Prefs.voiceHandsFree) }
    }
    var readReplies: Bool {
        get { UserDefaults.standard.bool(forKey: Prefs.voiceReadReplies) }
        set { UserDefaults.standard.set(newValue, forKey: Prefs.voiceReadReplies) }
    }

    let speaker: Speaker
    @ObservationIgnored private let store: LoftStore
    @ObservationIgnored private let engine = AVAudioEngine()
    @ObservationIgnored private var recognizer: SegmentRecognizer?
    @ObservationIgnored private var segments: [String] = []
    @ObservationIgnored private var started: Date?
    @ObservationIgnored private var accumulated: TimeInterval = 0
    @ObservationIgnored private var clock: Timer?
    @ObservationIgnored private var interruption: NSObjectProtocol?
    @ObservationIgnored private var lastLevelAt = Date.distantPast
    @ObservationIgnored private var confirmText = ""
    /// When the answer to "Would you like to submit this?" last changed, and when the question was asked.
    @ObservationIgnored private var confirmHeardAt = Date()
    @ObservationIgnored private var askedAt = Date()
    @ObservationIgnored private var confirmWatch: Task<Void, Never>?
    /// "Would you like to submit this?": once per quiet spell (VoiceSend.Prompt).
    @ObservationIgnored private var prompt = VoiceSend.Prompt(startedAt: Date())
    /// The transcript already checked for a command, and the one at the latest gap ("…[pause] send").
    @ObservationIgnored private var evaluated = ""
    @ObservationIgnored private var pauseBase = ""
    @ObservationIgnored private var lastWords = ""
    /// When something was last heard, for ending the dictation on a pause.
    @ObservationIgnored private var lastHeard = Date()
    @ObservationIgnored private var autoSend: Task<Void, Never>?

    init(store: LoftStore, speaker: Speaker? = nil) {
        self.store = store
        self.speaker = speaker ?? Speaker()
    }

    // MARK: - Listening

    func start(agentId: String) async {
        guard phase == .idle || isFailed || phase == .review || phase == .speaking || phase == .replied else { return }
        speaker.stop()
        failureNeedsPermission = false
        failedSend = nil
        self.agentId = agentId
        if phase != .review {
            segments = []
            committed = ""
            accumulated = 0
            elapsed = 0
        }
        live = ""
        cleaned = nil
        cleanupNote = nil
        reply = nil
        awaitingVoiceConfirm = false
        await listen()
    }

    private func listen() async {
        phase = .preparing
        do {
            if !TestMode.noSpeech { try await Self.requestPermissions() }
            let locale = UserDefaults.standard.string(forKey: Prefs.speechLocale).map(Locale.init(identifier:)) ?? Locale.current
            let r = Recognizers.make(locale: locale)
            try await r.prepare()
            r.onPartial = { [weak self] text in self?.heard(partial: text) }
            r.onCommit = { [weak self] text in self?.heard(commit: text) }
            r.onError = { [weak self] message in self?.fail(message) }
            recognizer = r
            lastHeard = Date()
            prompt = VoiceSend.Prompt(startedAt: lastHeard)
            evaluated = ""
            pauseBase = raw
            if !TestMode.noSpeech { try startAudio(feeding: r) }
            try r.start()
            phase = .listening
            started = Date()
            startClock()
        } catch {
            stopAudio()
            let v = error as? VoiceError
            fail(v?.message ?? error.localizedDescription, needsPermission: v?.isPermission ?? false)
        }
    }

    func pause() {
        guard phase == .listening else { return }
        phase = .paused
        accumulated += started.map { Date().timeIntervalSince($0) } ?? 0
        started = nil
        clock?.invalidate()
        stopAudio()
        let r = recognizer
        recognizer = nil
        Task { await r?.flush(); saveDraft() }
    }

    func resume() {
        guard phase == .paused || phase == .review else { return }
        awaitingVoiceConfirm = false
        cleaned = nil
        Task { await listen() }
    }

    /// Why listening ended: "send" was said, "review" (or Done) was asked for, or the quiet spell came
    /// and Loft asks "Would you like to submit this?". Nothing is sent just because you went quiet.
    enum FinishReason { case send, review, ask }

    /// Stop listening, clean the transcript up on the Mac, then send it ("send"), show it to edit
    /// ("review", Done), or show it and ask whether to submit it (after a quiet spell).
    func finish(_ reason: FinishReason = .review) async {
        guard phase == .listening || phase == .paused else { return }
        if phase == .listening {
            accumulated += started.map { Date().timeIntervalSince($0) } ?? 0
        }
        started = nil
        clock?.invalidate()
        stopAudio()
        phase = .cleaning
        let r = recognizer
        recognizer = nil
        await r?.flush()
        live = ""
        // A command at the very end of what was said isn't part of the message.
        let reading = VoiceSend.command(in: committed, sincePause: pauseBase)
        if reading.command != nil || reading.saidToFinish { setCommitted(reading.text) }
        saveDraft()
        guard !raw.isEmpty else {
            phase = .idle
            return
        }
        await clean()
        phase = .review
        switch reason {
        case .send where store.canWrite:
            VoiceLog.shared.add("talk: heard “send”: sending the cleaned text")
            await send(cleaned ?? raw)
        case .send, .review:
            VoiceLog.shared.add("talk: kept for review (\(reason == .send ? "offline" : "review or Done"))")
        case .ask:
            await askToSubmit()
        }
    }

    /// "Would you like to submit this?" aloud, over the cleaned text, then listen for yes / no / more.
    private func askToSubmit() async {
        let q = VoiceSend.question(for: cleaned ?? raw)
        VoiceLog.shared.add("talk: asked “\(q)”")
        if UIAccessibility.isVoiceOverRunning {
            UIAccessibility.post(notification: .announcement, argument: q)
            try? await Task.sleep(for: .seconds(2))
        } else {
            await speaker.speak(q)
        }
        guard phase == .review, autoSendAt == nil else { return }
        await listenForConfirm()
    }

    private func scheduleAutoSend() {
        autoSendAt = Date().addingTimeInterval(VoiceSend.undoWindow)
        autoSend?.cancel()
        autoSend = Task { [weak self] in
            try? await Task.sleep(for: .seconds(VoiceSend.undoWindow))
            guard let self, !Task.isCancelled, self.phase == .review, self.autoSendAt != nil else { return }
            VoiceLog.shared.add("talk: countdown over: sending")
            self.autoSendAt = nil
            // `send` cancels any pending countdown: this one is done, so let go of it first rather than
            // cancelling the task we're running in (its request then failed as "Cancelled").
            self.autoSend = nil
            await self.send(self.cleaned ?? self.raw)
        }
    }

    /// A suggested answer tapped on the question card: it goes out as the answer, whatever was being said is dropped.
    func answerWithChoice(_ text: String) async {
        guard agentId != nil else { return }
        VoiceLog.shared.add("talk: answer chip tapped")
        autoSend?.cancel()
        autoSend = nil
        autoSendAt = nil
        clock?.invalidate()
        stopConfirm()
        stopAudio()
        recognizer?.cancel()
        recognizer = nil
        speaker.stop()
        live = ""
        phase = .review
        await send(text)
    }

    /// The interview ended (End, a timeout, the agent): mic and reading off, and nothing reopens them.
    func interviewEnded() {
        guard isActive else { return }
        VoiceLog.shared.add("talk: interview ended: mic off")
        discard()
    }

    /// Voice style E: who the dictation goes to, picked on the To: row. Remembered for the next Talk.
    func switchAgent(_ id: String) {
        guard phase != .sending, phase != .speaking else { return }
        agentId = id
        UserDefaults.standard.set(id, forKey: "voiceLastAgent")
    }

    /// Voice style E: a dictated message sent from the chat's message box goes out after a short Undo, with the
    /// full-screen status (Sent, Thinking, the reply read aloud) instead of only a bubble in the chat.
    func sendWithStatus(_ text: String, to agentId: String) {
        guard phase == .idle || phase == .replied || isFailed else { return }
        self.agentId = agentId
        segments = []
        committed = ""
        live = ""
        reply = nil
        failedSend = nil
        cleaned = text
        // The countdown first, so the screen already sees it when the phase changes (no Undo: E's review means "to the chat").
        scheduleAutoSend()
        phase = .review
        VoiceLog.shared.add("talk: sending from the chat with the status screen (style E)")
    }

    /// Voice style E: read the reply again.
    func replay() async {
        guard phase == .replied, let text = reply else { return }
        phase = .speaking
        _ = await speaker.readReply(text, isActive: { phase == .speaking })
        if phase == .speaking { phase = .replied }
    }

    /// Voice style E: stop reading; the reply stays on screen.
    func stopReading() {
        guard phase == .speaking else { return }
        phase = .replied
        speaker.stop()
    }

    /// Undo an automatic send: the text stays for review and editing.
    func cancelAutoSend() {
        VoiceLog.shared.add("talk: Undo tapped: not sent, kept to edit")
        autoSend?.cancel()
        autoSend = nil
        autoSendAt = nil
    }

    /// The cleaned text goes in `cleaned`; if cleanup isn't possible, the original stays (never lost).
    private func clean() async {
        let (text, how) = await store.cleanDictation(raw)
        VoiceLog.shared.add("talk: cleanup: \(how)")
        if text != raw { cleaned = text } else if !how.hasPrefix("nothing") { cleanupNote = "Couldn't tidy this up; showing what was heard." }
    }

    private func heard(partial: String) {
        if awaitingVoiceConfirm { return confirmHeard(partial) }
        live = partial
        noteWords()
    }

    private func heard(commit: String) {
        if awaitingVoiceConfirm { return confirmHeard(commit) }
        lastHeard = Date()
        segments.append(commit)
        committed = VoiceText.join(segments)
        live = ""
        saveDraft()
        noteWords()
    }

    /// New words (not the recognizer rewriting punctuation) restart the quiet spell.
    private func noteWords() {
        let words = VoiceText.normalize(raw)
        guard words != lastWords else { return }
        lastWords = words
        lastHeard = Date()
        prompt.heardWords(at: lastHeard)
    }

    private func setCommitted(_ text: String) {
        segments = [text]
        committed = text
    }

    /// Twice a second while listening: act on a settled command, and ask after a quiet spell.
    private func tick() async {
        guard phase == .listening else { return }
        let now = Date()
        let quiet = now.timeIntervalSince(lastHeard)
        if raw != evaluated, !raw.isEmpty, quiet >= VoiceSend.settle {
            evaluated = raw
            let r = VoiceSend.command(in: raw, sincePause: pauseBase)
            switch r.command {
            case .send: return await finish(.send)
            case .review: return await finish(.review)
            case .stop, .discard:
                VoiceLog.shared.add("talk: heard “\(r.command == .stop ? "stop" : "cancel")”: nothing sent")
                return discard()
            case .startOver:
                VoiceLog.shared.add("talk: heard “start over”: cleared")
                segments = []
                committed = ""
                live = ""
                evaluated = ""
                pauseBase = ""
                clearDraft()
                return
            case nil:
                if r.saidToFinish {
                    VoiceLog.shared.add("talk: heard an end phrase: asking now")
                    prompt.didAsk()
                    return await finish(.ask)
                }
                if VoiceSend.endsOpen(raw) {
                    prompt.leftOpen()
                    VoiceLog.shared.add("talk: stopped mid-sentence: asking after \(Int(VoiceSend.askAfterOpenEnding)) s")
                }
            }
        }
        if quiet >= VoiceSend.utteranceGap { pauseBase = raw }
        if raw.isEmpty, quiet >= VoiceSend.noSpeechTimeout {
            VoiceLog.shared.add("talk: nothing heard for \(Int(VoiceSend.noSpeechTimeout)) s: paused")
            return pause()
        }
        if prompt.shouldAsk(hasText: !raw.isEmpty, now: now) {
            VoiceLog.shared.add("talk: quiet \(String(format: "%g", prompt.wait)) s after \(VoiceLog.clip(raw)): asking")
            prompt.didAsk()
            await finish(.ask)
        }
    }

    // MARK: - Hands-free confirm

    /// Listen briefly for a one- or two-word command while the review is on screen.
    private func listenForConfirm() async {
        awaitingVoiceConfirm = true
        confirmText = ""
        askedAt = Date()
        confirmHeardAt = askedAt
        confirmWatch?.cancel()
        confirmWatch = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(250))
                guard let self, self.awaitingVoiceConfirm else { return }
                await self.checkAnswer()
            }
        }
        do {
            let locale = UserDefaults.standard.string(forKey: Prefs.speechLocale).map(Locale.init(identifier:)) ?? Locale.current
            let r = Recognizers.make(locale: locale)
            try await r.prepare()
            r.onPartial = { [weak self] t in self?.confirmHeard(t) }
            r.onCommit = { [weak self] t in self?.confirmHeard(t) }
            recognizer = r
            try startAudio(feeding: r)
            try r.start()
        } catch {
            // Tapping still works.
            awaitingVoiceConfirm = false
            stopAudio()
        }
    }

    private func confirmHeard(_ text: String) {
        guard awaitingVoiceConfirm, phase == .review else { return }
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty, VoiceText.normalize(t) != VoiceText.normalize(confirmText) else { return }
        confirmText = t
        confirmHeardAt = Date()
        // More than a few words is the message going on.
        if VoiceText.normalize(t).split(separator: " ").count > 3 { carryOn(with: t) }
    }

    /// The answer, once the words have settled; nothing for a long while leaves the text to edit.
    private func checkAnswer() async {
        guard awaitingVoiceConfirm, phase == .review else { return }
        if confirmText.isEmpty {
            if Date().timeIntervalSince(askedAt) >= VoiceSend.noSpeechTimeout {
                VoiceLog.shared.add("talk: no answer for \(Int(VoiceSend.noSpeechTimeout)) s: mic off, kept, nothing sent")
                stopConfirm()
            }
            return
        }
        guard Date().timeIntervalSince(confirmHeardAt) >= VoiceSend.settle else { return }
        let t = confirmText
        let answer = await OnDeviceFeatures.submitAnswer(t, question: VoiceSend.question(for: cleaned ?? raw))
        // More was said while the model thought: the next round decides.
        guard awaitingVoiceConfirm, phase == .review, confirmText == t else { return }
        switch answer {
        case .yes:
            VoiceLog.shared.add("talk: answer yes (“\(t)”): sending in \(Int(VoiceSend.undoWindow)) s with Undo")
            stopConfirm()
            scheduleAutoSend()
        case .no:
            VoiceLog.shared.add("talk: answer no (“\(t)”): kept to edit")
            stopConfirm()
        case .discard:
            VoiceLog.shared.add("talk: answer cancel: nothing sent")
            stopConfirm()
            discard()
        case nil:
            carryOn(with: t)
        }
    }

    /// Not an answer: back to listening, with those words added to the (original) message.
    private func carryOn(with words: String) {
        VoiceLog.shared.add("talk: question dismissed, more words: \(VoiceLog.clip(words))")
        stopConfirm()
        segments.append(words)
        committed = VoiceText.join(segments)
        resume()
    }

    private func stopConfirm() {
        awaitingVoiceConfirm = false
        confirmWatch?.cancel()
        confirmWatch = nil
        stopAudio()
        recognizer?.cancel()
        recognizer = nil
    }

    // MARK: - Sending and the reply

    /// The only way anything leaves the phone: an explicit send of the reviewed text.
    func send(_ text: String) async {
        guard phase == .review, let agentId else { return }
        autoSend?.cancel()
        autoSend = nil
        autoSendAt = nil
        stopConfirm()
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        phase = .sending
        sentAt = Date()
        let before = store.conversation(agentId).last?.id ?? 0
        // While a round waits, what you say answers the question read last (same as typing it in the chat); the next
        // question of the round is read straight away, and after the last one the round goes out.
        if let q = store.currentGrillQuestion(for: agentId) {
            VoiceLog.shared.add("talk: sent as the answer to question \(store.openGrill(for: agentId)?.number(of: q.id) ?? 0)")
            do {
                try await store.answerCurrentGrill(text, agentId: agentId)
            } catch {
                let e = APIError.from(error)
                phase = .failed(e.isRoundNotSent ? "\(e.title). \(e.suggestion)" : "Not sent: \(e.title). Say it again or answer on the card.")
                return
            }
            if let next = store.currentGrillQuestion(for: agentId), next.id != q.id {
                return await afterSent(agentId: agentId, before: before, next: next)
            }
        } else {
            await store.send(text, to: agentId)
        }
        if let failed = store.pending(for: agentId).last(where: { $0.text == text && $0.isFailed }) {
            return sendFailed(failed)
        }
        VoiceLog.shared.add("talk: sent \(VoiceLog.clip(text))")
        await afterSent(agentId: agentId, before: before)
    }

    /// "Retry sending" on the failure screen: the same message (same client id, so it can't arrive twice).
    func retrySend() async {
        guard let p = failedSend, let agentId else { return }
        failedSend = nil
        phase = .sending
        VoiceLog.shared.add("talk: retrying the send")
        let before = store.conversation(agentId).last?.id ?? 0
        await store.retry(p)
        if let failed = store.pending(for: agentId).first(where: { $0.id == p.id && $0.isFailed }) {
            return sendFailed(failed)
        }
        await afterSent(agentId: agentId, before: before)
    }

    private func sendFailed(_ failed: PendingMessage) {
        VoiceLog.shared.add("talk: send failed: \(failed.failure ?? "unknown error"); kept to retry")
        failedSend = failed
        phase = .failed("Not sent: \(failed.failure ?? "unknown error"). It's kept in the chat to retry.")
    }

    /// Sent: wait for the answer, read it (or the next interview question), and listen again when hands-free.
    /// `next`: the next question of the same round, read without waiting for anything.
    private func afterSent(agentId: String, before: Int64, next: GrillQuestion? = nil) async {
        clearDraft()
        segments = []
        committed = ""
        var next = next
        var answer = ""
        if next == nil {
            VoiceLog.shared.add("talk: waiting for the reply")
            guard let r = await store.waitForReply(agentId: agentId, after: before, while: { phase == .sending }) else {
                VoiceLog.shared.add("talk: stopped waiting for the reply")
                phase = .idle
                return
            }
            answer = r
            VoiceLog.shared.add("talk: reply arrived: \(VoiceLog.clip(answer))")
            // A new round: its first question, once the session has caught up with the round's message.
            if store.openGrill(for: agentId) != nil { await store.refreshGrill(agentId) }
            next = store.grillRoundMessage(for: agentId).flatMap { $0.id > before ? store.currentGrillQuestion(for: agentId) : nil }
        }
        // A question is read in its short spoken form with its recommendation, like in the chat; its card shows it.
        reply = next == nil ? answer : nil
        let s = store.openGrill(for: agentId)
        let spoken = next.map { $0.spoken(number: s?.number(of: $0.id) ?? 1, of: s?.questions?.count ?? 1, language: s?.language) } ?? answer
        if readReplies {
            phase = .speaking
            let id = next != nil ? store.grillRoundMessage(for: agentId)?.id : store.conversation(agentId).last { $0.role == "assistant" && $0.kind == "text" }?.id
            VoiceLog.shared.add("talk: reading the \(next == nil ? "reply" : "next question")")
            let ended = await speaker.readReply(spoken, messageId: id, isActive: { phase == .speaking })
            VoiceLog.shared.add(speaker.lastFailure.map { "talk: not read: \($0)" } ?? "talk: read \(ended.map { "until “\($0)”" } ?? "through")")
            if VoiceVariant.selected == .e, next == nil {
                // Style E: the reply stays on screen with Replay, Reply by voice, Open chat and Done.
                if phase == .speaking { phase = .replied }
                return
            }
            if ended == .stop, phase == .speaking {
                // "Stop" while the reply is read ends the conversation.
                phase = .idle
                return
            }
        } else {
            VoiceLog.shared.add("talk: not read, “Read replies aloud” is off")
        }
        guard phase == .speaking || phase == .sending else { return } // stopped meanwhile
        if VoiceVariant.selected == .e, next == nil {
            phase = .replied
            return
        }
        if handsFree {
            phase = .idle
            await start(agentId: agentId)
        } else {
            phase = .idle
        }
    }

    func stopSpeaking() {
        speaker.stop()
    }

    /// Throw away the dictation and stop everything.
    func discard() {
        autoSend?.cancel()
        autoSend = nil
        autoSendAt = nil
        clock?.invalidate()
        stopConfirm()
        stopAudio()
        recognizer?.cancel()
        recognizer = nil
        speaker.stop()
        segments = []
        committed = ""
        live = ""
        cleaned = nil
        reply = nil
        elapsed = 0
        accumulated = 0
        clearDraft()
        phase = .idle
    }

    /// Leave the session (e.g. closing the screen) without losing the draft.
    func close() {
        // Leaving mid-countdown keeps the text for review instead of sending behind your back.
        if autoSendAt != nil { autoSend?.cancel(); autoSend = nil; autoSendAt = nil }
        if phase == .listening || phase == .paused {
            pause()
        }
        speaker.stop()
        stopConfirm()
        if phase != .review { phase = .idle }
    }

    private func fail(_ message: String, needsPermission: Bool = false) {
        failureNeedsPermission = needsPermission
        clock?.invalidate()
        stopAudio()
        recognizer?.cancel()
        recognizer = nil
        saveDraft()
        phase = .failed(message)
    }

    // MARK: - Audio

    private func startAudio(feeding r: SegmentRecognizer) throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .spokenAudio, options: [.defaultToSpeaker, .allowBluetoothHFP, .duckOthers])
        try session.setActive(true, options: .notifyOthersOnDeactivation)
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.channelCount > 0, format.sampleRate > 0 else { throw VoiceError("No microphone is available.") }
        input.removeTap(onBus: 0)
        input.installTap(onBus: 0, bufferSize: 2048, format: format) { [weak self] buffer, _ in
            r.append(buffer)
            let level = Self.rms(buffer)
            DispatchQueue.main.async { self?.setLevel(level) }
        }
        engine.prepare()
        try engine.start()
        if interruption == nil {
            interruption = NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] n in
                let began = (n.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt).flatMap(AVAudioSession.InterruptionType.init) == .began
                Task { @MainActor in if began { self?.pause() } }
            }
        }
    }

    private func stopAudio() {
        if engine.isRunning { engine.stop() }
        if !TestMode.noSpeech { engine.inputNode.removeTap(onBus: 0) }
        level = 0
    }

    private func setLevel(_ l: Float) {
        // ~20 Hz is plenty for an animation.
        guard Date().timeIntervalSince(lastLevelAt) > 0.05 else { return }
        lastLevelAt = Date()
        level = l
    }

    nonisolated static func rms(_ buffer: AVAudioPCMBuffer) -> Float {
        guard let data = buffer.floatChannelData?[0], buffer.frameLength > 0 else { return 0 }
        var sum: Float = 0
        for i in 0..<Int(buffer.frameLength) { sum += data[i] * data[i] }
        let rms = sqrt(sum / Float(buffer.frameLength))
        // Map roughly -50…0 dB to 0…1.
        let db = 20 * log10(max(rms, 1e-6))
        return max(0, min(1, (db + 50) / 50))
    }

    private func startClock() {
        clock?.invalidate()
        clock = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self, let s = self.started else { return }
                self.elapsed = self.accumulated + Date().timeIntervalSince(s)
                await self.tick()
            }
        }
    }

    static func requestPermissions() async throws {
        let auth = await withCheckedContinuation { c in SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0) } }
        guard auth == .authorized else {
            throw VoiceError("Speech recognition is off for Loft. Turn it on in Settings › Privacy & Security › Speech Recognition.", isPermission: true)
        }
        guard await AVAudioApplication.requestRecordPermission() else {
            throw VoiceError("Microphone access is off for Loft. Turn it on in Settings › Privacy & Security › Microphone.", isPermission: true)
        }
    }

    // MARK: - Draft

    struct Draft: Codable, Equatable {
        var agentId: String
        var text: String
        var savedAt: Date
    }

    private static let draftKey = "voiceDraft"

    private func saveDraft() {
        guard let agentId, !raw.isEmpty else { return }
        let d = Draft(agentId: agentId, text: raw, savedAt: Date())
        if let data = try? JSONEncoder().encode(d) { UserDefaults.standard.set(data, forKey: Self.draftKey) }
    }

    private func clearDraft() { UserDefaults.standard.removeObject(forKey: Self.draftKey) }

    /// A dictation that was never sent (the app was closed or crashed mid-session).
    static func savedDraft() -> Draft? {
        UserDefaults.standard.data(forKey: draftKey).flatMap { try? JSONDecoder().decode(Draft.self, from: $0) }
    }

    /// Pick up a saved draft in the review step.
    func restoreDraft() async {
        guard let d = Self.savedDraft(), phase == .idle else { return }
        agentId = d.agentId
        setCommitted(d.text)
        live = ""
        phase = .cleaning
        await clean()
        phase = .review
    }

    #if DEBUG
    /// For UI work and screenshots without a microphone (`-fixture-voice <phase>`).
    func applyFixture(_ name: String, agentId: String) {
        self.agentId = agentId
        segments = ["So I want the settings screen to show the server version and the last sync time,"]
        committed = VoiceText.join(segments)
        live = "and add a button to copy the diagnostics"
        level = 0.55
        elapsed = 83
        switch name {
        case "paused": phase = .paused
        case "preparing": phase = .preparing
        case "cleaning":
            live = ""
            phase = .cleaning
        case "review":
            live = ""
            cleaned = "I want the settings screen to show the server version and the last sync time, and add a button to copy the diagnostics."
            phase = .review
            awaitingVoiceConfirm = handsFree
        case "autosend":
            live = ""
            phase = .review
            autoSendAt = Date().addingTimeInterval(3600)
        case "sent":
            // Style E's status screen: the message on its way, with Undo.
            live = ""
            cleaned = "I want the settings screen to show the server version and the last sync time, and add a button to copy the diagnostics."
            phase = .review
            autoSendAt = Date().addingTimeInterval(3600)
        case "sending", "thinking":
            sentAt = Date().addingTimeInterval(-74)
            phase = .sending
        case "replied":
            reply = "Done. The settings screen now shows the server version and last sync, and there's a Copy diagnostics button."
            phase = .replied
        case "failed-send":
            failedSend = PendingMessage(id: "fixture", agentId: agentId, text: "Add a copy diagnostics button", createdAt: Date(), failure: "The Mac can't be reached")
            phase = .failed("Not sent: The Mac can't be reached. It's kept in the chat to retry.")
        case "speaking":
            reply = "Done. The settings screen now shows the server version and last sync, and there's a Copy diagnostics button."
            phase = .speaking
        case "failed":
            failureNeedsPermission = true
            phase = .failed("Microphone access is off for Loft. Turn it on in Settings › Privacy & Security › Microphone.")
        case "failed-startup":
            failureNeedsPermission = false
            phase = .failed("Speech recognition couldn't start on this device (Failed to initialize recognizer). Check that dictation is on and its language is downloaded in Settings › General › Keyboard.")
        default: phase = .listening
        }
    }
    #endif
}
