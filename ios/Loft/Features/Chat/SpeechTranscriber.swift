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

    @ObservationIgnored private let engine = AVAudioEngine()
    @ObservationIgnored private var request: SFSpeechAudioBufferRecognitionRequest?
    @ObservationIgnored private var task: SFSpeechRecognitionTask?
    @ObservationIgnored private var interruption: NSObjectProtocol?
    @ObservationIgnored private var recognizer: SFSpeechRecognizer?
    /// The microphone tap feeds whichever request is current.
    @ObservationIgnored private let feed = RequestFeed()
    /// Text from requests that already ended in this dictation.
    @ObservationIgnored private var committed = ""
    /// Ignores callbacks from a request that's been replaced.
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var startup = StartupFailureGuard()
    @ObservationIgnored private var requestStarted = Date()

    private func send(_ e: SpeechEvent) {
        let next = SpeechMachine.reduce(state, e)
        if next != state { Log.speech.debug("speech \(String(describing: self.state), privacy: .public) → \(String(describing: next), privacy: .public)") }
        state = next
    }

    func start() async {
        send(.start)
        guard state == .requestingPermission else { return }
        transcript = ""
        // `-no-speech` (UI tests): recording at once, with no prompts, microphone or recognizer.
        if TestMode.noSpeech { send(.began); return }

        let auth = await withCheckedContinuation { c in SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0) } }
        guard auth == .authorized else {
            send(.denied("Speech recognition is off for Loft. Turn it on in Settings › Privacy & Security › Speech Recognition."))
            return
        }
        guard await AVAudioApplication.requestRecordPermission() else {
            send(.denied("Microphone access is off for Loft. Turn it on in Settings › Privacy & Security › Microphone."))
            return
        }
        // Stopped while the permission prompts were up.
        guard state == .requestingPermission else { return }

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
        committed = ""
        startup = StartupFailureGuard()

        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.record, mode: .measurement, options: .duckOthers)
            try session.setActive(true, options: .notifyOthersOnDeactivation)
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            guard format.channelCount > 0, format.sampleRate > 0 else {
                cleanUp()
                send(.unavailable("No microphone is available."))
                return
            }
            let feed = self.feed
            input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
                feed.append(buffer)
            }
            engine.prepare()
            try engine.start()
        } catch {
            cleanUp()
            send(.error("Couldn't start the microphone: \(error.localizedDescription)"))
            return
        }

        interruption = NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in
                guard let self, self.state == .recording else { return }
                self.cleanUp()
                self.send(.interrupted)
            }
        }
        beginRequest()
        send(.began)
        VoiceLog.shared.add("mic on (\(locale.identifier), on-device \(recognizer.supportsOnDeviceRecognition))")
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
        guard state.isRecording else { return }
        cleanUp()
        send(.stop)
    }

    private func cleanUp() {
        if engine.isRunning { engine.stop() }
        if !TestMode.noSpeech { engine.inputNode.removeTap(onBus: 0) }
        request?.endAudio()
        feed.request = nil
        generation += 1
        // Cancel rather than finish: nothing more is processed or sent once the user stops.
        task?.cancel()
        task = nil
        request = nil
        if let interruption { NotificationCenter.default.removeObserver(interruption) }
        interruption = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
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
