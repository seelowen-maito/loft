import AVFoundation
import Foundation
import Speech

enum SpeechState: Equatable {
    case idle
    case requestingPermission
    case recording
    case denied(String)
    case unavailable(String)
    case failed(String)

    var isRecording: Bool { self == .recording || self == .requestingPermission }

    var message: String? {
        switch self {
        case .idle: nil
        case .requestingPermission: "Starting dictation…"
        case .recording: "Listening… say “send” when you're done."
        case .denied(let m), .unavailable(let m), .failed(let m): m
        }
    }
}

enum SpeechEvent: Equatable {
    case start
    case denied(String)
    case unavailable(String)
    case began
    case finished
    case error(String)
    case stop
    case interrupted
}

/// The dictation state machine, kept pure so it can be tested without audio.
enum SpeechMachine {
    static func reduce(_ s: SpeechState, _ e: SpeechEvent) -> SpeechState {
        switch (s, e) {
        case (.recording, .start), (.requestingPermission, .start): s
        case (_, .start): .requestingPermission
        case (.requestingPermission, .denied(let m)): .denied(m)
        case (.requestingPermission, .unavailable(let m)): .unavailable(m)
        case (.requestingPermission, .error(let m)): .failed(m)
        case (.requestingPermission, .began): .recording
        case (.recording, .finished), (.recording, .stop), (.requestingPermission, .stop): .idle
        case (.recording, .interrupted): .failed("Dictation stopped because another app took the microphone.")
        case (.recording, .error(let m)): .failed(m)
        // Anything else (late callbacks after a stop) leaves the state alone.
        default: s
        }
    }
}

/// Live speech-to-text with the Speech framework. Prefers on-device recognition;
/// with "On-device only" set it never sends audio to Apple's servers. Audio capture
/// stops the moment dictation stops, and the result only fills the composer.
@MainActor
@Observable
final class SpeechTranscriber {
    private(set) var state: SpeechState = .idle
    private(set) var transcript = ""

    @ObservationIgnored private let engine: MicEngine
    @ObservationIgnored private let audio: AudioSessionOwner
    @ObservationIgnored private var request: SFSpeechAudioBufferRecognitionRequest?
    @ObservationIgnored private var task: SFSpeechRecognitionTask?
    /// The audio held from `AudioSessionOwner` while the mic is on.
    @ObservationIgnored private var lease: UUID?
    @ObservationIgnored private var recognizer: SFSpeechRecognizer?
    /// The microphone tap feeds whichever request is current.
    @ObservationIgnored private let feed = RequestFeed()
    /// Text from requests that already ended in this dictation.
    @ObservationIgnored private var committed = ""
    /// Ignores callbacks from a request that's been replaced.
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var startup = StartupFailureGuard()
    @ObservationIgnored private var requestStarted = Date()
    /// A quick on/off/on of the mic button: the start that was overtaken gives up.
    @ObservationIgnored private var gate = StartGate()

    init(audio: AudioSessionOwner? = nil, engine: MicEngine = AVAudioEngine()) {
        self.audio = audio ?? .shared
        self.engine = engine
    }

    private func send(_ e: SpeechEvent) {
        let next = SpeechMachine.reduce(state, e)
        if next != state { Log.speech.debug("speech \(String(describing: self.state), privacy: .public) → \(String(describing: next), privacy: .public)") }
        state = next
    }

    func start() async {
        send(.start)
        guard state == .requestingPermission else { return }
        let attempt = gate.begin()
        transcript = ""
        // `-no-speech` (UI tests): recording at once, with no prompts, microphone or recognizer.
        if TestMode.noSpeech { send(.began); return }

        do {
            try await VoiceSession.requestPermissions()
        } catch {
            send(.denied(error.localizedDescription))
            return
        }
        // Stopped (or stopped and started again) while the permission prompts were up.
        guard gate.isCurrent(attempt), state == .requestingPermission else { return }

        let locale = UserDefaults.standard.string(forKey: Prefs.speechLocale).map(Locale.init(identifier:)) ?? Locale.current
        guard let recognizer = SFSpeechRecognizer(locale: locale) else {
            send(.unavailable("Dictation doesn't support \(locale.localizedString(forIdentifier: locale.identifier) ?? locale.identifier). Pick another language in Settings."))
            return
        }
        guard recognizer.isAvailable else {
            send(.unavailable("Speech recognition isn't available right now. You can still type."))
            return
        }
        let onDeviceOnly = UserDefaults.standard.bool(forKey: Prefs.speechOnDeviceOnly)
        if onDeviceOnly && !recognizer.supportsOnDeviceRecognition {
            send(.unavailable("On-device dictation isn't available for this language. Turn off “On-device only” in Settings to use Apple's servers."))
            return
        }

        self.recognizer = recognizer
        await listen(attempt)
    }

    /// `start` with the permission and recognizer checks skipped: takes the audio and turns the mic on. For
    /// tests, which have no recognizer, so no request is made.
    func startSkippingChecks() async {
        send(.start)
        guard state == .requestingPermission else { return }
        await listen(gate.begin())
    }

    private func listen(_ attempt: Int) async {
        committed = ""
        startup = StartupFailureGuard()

        do {
            let lease = try await audio.acquire(.listening, by: "dictation", onRouteChange: { [weak self] in
                self?.routeChanged()
            }) { [weak self] loss in self?.lost(loss) }
            // Stopped (or started again) while the audio was being handed over.
            guard gate.isCurrent(attempt), state == .requestingPermission else {
                audio.release(lease)
                return
            }
            self.lease = lease
            let feed = self.feed
            try await Microphone.startWhenSettled(engine, bufferSize: 1024, stillWanted: { [weak self] in
                self.map { $0.gate.isCurrent(attempt) && $0.state == .requestingPermission } ?? false
            }) { buffer, _ in feed.append(buffer) }
        } catch {
            // A start that was overtaken leaves the newer one alone.
            guard gate.isCurrent(attempt) else { return }
            // Stopped, or the audio was lost, while the mic waited for the route: that path already said so.
            if error is CancellationError { return }
            cleanUp()
            send(.error("Couldn't start the microphone: \(error.localizedDescription)"))
            return
        }

        beginRequest()
        send(.began)
        VoiceLog.shared.add("mic on (\(recognizer?.locale.identifier ?? "no recognizer"), on-device \(recognizer?.supportsOnDeviceRecognition ?? false))")
    }

    /// Losing the audio (read-aloud, a call, another app) ends the dictation, keeping what was heard.
    func lost(_ loss: AudioSessionOwner.Loss) {
        guard state.isRecording else { return }
        lease = nil
        cleanUp()
        switch loss {
        case .handedOver: send(.stop)
        case .interrupted: send(.interrupted)
        case .noMicrophone: send(.error("Dictation stopped: this audio route has no microphone."))
        }
    }

    private func startMic() throws {
        let feed = self.feed
        try Microphone.start(engine, bufferSize: 1024) { buffer, _ in feed.append(buffer) }
    }

    /// The route settled on another one (the car's Bluetooth switching to call quality): the engine stopped
    /// with it, so start the mic again there, keeping the recognition request.
    func routeChanged() {
        guard state == .recording || state == .requestingPermission, lease != nil else { return }
        do {
            try startMic()
            VoiceLog.shared.add("mic restarted on the new route")
        } catch {
            VoiceLog.shared.add("mic couldn't restart on the new route: \(error.localizedDescription)")
            cleanUp()
            send(.error("Dictation stopped: \(error.localizedDescription)"))
        }
    }

    /// A fresh recognition request on the running microphone.
    private func beginRequest() {
        guard let recognizer else { return }
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.shouldReportPartialResults = true
        req.requiresOnDeviceRecognition = recognizer.supportsOnDeviceRecognition
        req.addsPunctuation = true
        request = req
        feed.request = req
        generation += 1
        requestStarted = Date()
        let gen = generation
        task = recognizer.recognitionTask(with: req) { [weak self] result, error in
            let text = result?.bestTranscription.formattedString
            let isFinal = result?.isFinal ?? false
            let message = error?.localizedDescription
            Task { @MainActor in self?.handle(gen: gen, text: text, isFinal: isFinal, error: message) }
        }
    }

    private func handle(gen: Int, text: String?, isFinal: Bool, error: String?) {
        // Results that arrive after a stop are dropped, so they can't overwrite edits.
        guard state == .recording, gen == generation else { return }
        if let text { transcript = VoiceText.join([committed, text]) }
        guard isFinal || error != nil else { return }
        // The recognizer ends a request by itself after a short silence (on the iPhone often within a
        // second or two) or at its time limit. That isn't the end of what you're saying: keep the text
        // and carry on with a new request. Only the composer's pause rule, a command or Stop end dictation.
        let heard = !(text ?? "").isEmpty
        if startup.record(failed: error != nil, heard: heard, lasted: Date().timeIntervalSince(requestStarted)) {
            VoiceLog.shared.add("recognizer keeps failing (\(error ?? "?")): mic off")
            cleanUp()
            send(transcript.isEmpty ? .error("Didn't catch that. \(error ?? "")") : .finished)
            return
        }
        committed = transcript
        VoiceLog.shared.add(isFinal ? "recognizer finalized a segment; still listening" : "recognizer ended (\(error ?? "")); still listening")
        task?.cancel()
        beginRequest()
    }

    func stop() {
        gate.cancel()
        guard state.isRecording else { return }
        cleanUp()
        send(.stop)
    }

    private func cleanUp() {
        if !TestMode.noSpeech { Microphone.stop(engine) }
        request?.endAudio()
        feed.request = nil
        generation += 1
        // Cancel rather than finish: nothing more is processed or sent once the user stops.
        task?.cancel()
        task = nil
        request = nil
        if let lease { audio.release(lease) }
        lease = nil
    }
}

/// The current request, read from the audio thread.
final class RequestFeed: @unchecked Sendable {
    private let lock = NSLock()
    private var _request: SFSpeechAudioBufferRecognitionRequest?
    var request: SFSpeechAudioBufferRecognitionRequest? {
        get { lock.lock(); defer { lock.unlock() }; return _request }
        set { lock.lock(); _request = newValue; lock.unlock() }
    }
    func append(_ buffer: AVAudioPCMBuffer) { request?.append(buffer) }
}
