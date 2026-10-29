import AVFoundation
import Foundation
import Speech

/// Reads an utterance aloud while listening for a spoken command. The speech is rendered into
/// buffers and played through the same audio engine as the microphone, with voice processing on,
/// so the iPhone's echo cancellation removes Loft's own voice from what the recognizer hears.
/// Recognition stays on the phone when the language supports it; nothing heard is kept.
@MainActor
final class InterruptibleReader {
    /// The command heard, once; reading is stopped by whoever handles it.
    var onCommand: ((ReadAloudCommand) -> Void)?
    var phrases: GrillPhrases?
    /// Voice style D: any words said over the reading (not only a command) end it, as `.skip`: it's your turn.
    var bargeIn = false

    private let synth = AVSpeechSynthesizer()
    private var engine: AVAudioEngine?
    private let player = AVAudioPlayerNode()
    private var converter: AVAudioConverter?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var finished: CheckedContinuation<Void, Never>?
    private var connected = false
    private var scheduled = 0
    private var played = 0
    private var rendered = false
    private var heard = false
    /// Audio actually went out. When it didn't (the engine wouldn't start, nothing was rendered), the caller
    /// reads it the plain way instead of treating the silence as "read through".
    private(set) var startedAudio = false
    /// Why it ended without audio, for the voice log.
    private(set) var failure: String?
    /// Silence played after the speech, so a command said right after it ("more") is still heard.
    private var tail: TimeInterval = 0
    private var lastFormat: AVAudioFormat?

    /// Whether listening can work here: permissions already given (never prompted mid-reply) and a recognizer
    /// that respects "On-device only".
    static func canListen(locale: Locale) -> Bool {
        guard !TestMode.noSpeech, SFSpeechRecognizer.authorizationStatus() == .authorized,
              AVAudioApplication.shared.recordPermission == .granted,
              let r = SFSpeechRecognizer(locale: locale), r.isAvailable else { return false }
        return r.supportsOnDeviceRecognition || !UserDefaults.standard.bool(forKey: Prefs.speechOnDeviceOnly)
    }

    /// Plays `utterance` and returns when it's done or stopped. Throws if the audio couldn't start, so the
    /// caller can read it the plain way instead.
    func play(_ utterance: AVSpeechUtterance, locale: Locale, listenAfter: TimeInterval = 0) async throws {
        tail = listenAfter
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker, .allowBluetoothHFP, .duckOthers])
        try session.setActive(true)
        guard session.isInputAvailable else { throw VoiceError("No microphone is available.") }
        let engine = AVAudioEngine()
        self.engine = engine
        try engine.inputNode.setVoiceProcessingEnabled(true)
        engine.attach(player)
        try startRecognition(locale: locale, engine: engine)
        // If no audio has started within a few seconds, give up so the caller can read it plainly.
        let watchdog = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(3))
            guard let self, !Task.isCancelled, !self.startedAudio, self.finished != nil else { return }
            self.failure = self.failure ?? "no audio after 3 s"
            self.stop()
        }
        await withCheckedContinuation { c in
            finished = c
            synth.write(utterance) { [weak self] buffer in
                guard let pcm = buffer as? AVAudioPCMBuffer else { return }
                DispatchQueue.main.async { self?.received(pcm) }
            }
        }
        watchdog.cancel()
    }

    func stop() {
        synth.stopSpeaking(at: .immediate)
        player.stop()
        task?.cancel()
        task = nil
        request?.endAudio()
        request = nil
        if let engine {
            if engine.isRunning { engine.stop() }
            engine.inputNode.removeTap(onBus: 0)
        }
        engine = nil
        finished?.resume()
        finished = nil
    }

    private func received(_ pcm: AVAudioPCMBuffer) {
        guard finished != nil, let engine else { return }
        // An empty buffer marks the end of the speech.
        guard pcm.frameLength > 0 else {
            if tail > 0, connected, let format = lastFormat,
               let silence = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(tail * format.sampleRate)) {
                silence.frameLength = silence.frameCapacity
                for ch in 0..<Int(format.channelCount) {
                    silence.floatChannelData?[ch].update(repeating: 0, count: Int(silence.frameLength))
                }
                schedule(silence)
            }
            rendered = true
            finishIfDone()
            return
        }
        guard let buffer = float(pcm) else {
            failure = failure ?? "couldn't convert the speech audio"
            return
        }
        if !connected {
            engine.connect(player, to: engine.mainMixerNode, format: buffer.format)
            do {
                engine.prepare()
                try engine.start()
            } catch {
                Log.speech.info("read-aloud engine didn't start: \(error.localizedDescription, privacy: .public)")
                failure = "engine didn't start: \(error.localizedDescription)"
                stop()
                return
            }
            player.play()
            connected = true
            startedAudio = true
        }
        lastFormat = buffer.format
        schedule(buffer)
    }

    private func schedule(_ buffer: AVAudioPCMBuffer) {
        scheduled += 1
        player.scheduleBuffer(buffer) { [weak self] in
            DispatchQueue.main.async {
                guard let self else { return }
                self.played += 1
                self.finishIfDone()
            }
        }
    }

    private func finishIfDone() {
        if rendered && played >= scheduled { stop() }
    }

    /// The player wants float samples; the synthesizer may hand out 16-bit ones.
    private func float(_ pcm: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
        if pcm.format.commonFormat == .pcmFormatFloat32 { return pcm }
        guard let target = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: pcm.format.sampleRate,
                                         channels: pcm.format.channelCount, interleaved: false) else { return nil }
        if converter?.inputFormat != pcm.format { converter = AVAudioConverter(from: pcm.format, to: target) }
        guard let converter, let out = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: pcm.frameLength) else { return nil }
        do { try converter.convert(to: out, from: pcm) } catch { return nil }
        return out
    }

    private func startRecognition(locale: Locale, engine: AVAudioEngine) throws {
        guard let recognizer = SFSpeechRecognizer(locale: locale) else { return }
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        // installTap raises an Objective-C exception (which Swift can't catch, so the app aborts) when the mic
        // has no usable format: no microphone, or another engine still holding it. Read plainly instead.
        guard format.channelCount > 0, format.sampleRate > 0 else {
            throw VoiceError("The microphone isn't available (\(format.channelCount) channels at \(Int(format.sampleRate)) Hz).")
        }
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.shouldReportPartialResults = true
        req.requiresOnDeviceRecognition = recognizer.supportsOnDeviceRecognition
        req.taskHint = .confirmation
        request = req
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
            req.append(buffer)
        }
        task = recognizer.recognitionTask(with: req) { [weak self] result, _ in
            guard let result else { return }
            let words = result.bestTranscription.segments.map {
                ReadAloudCommand.Word(text: $0.substring, at: $0.timestamp, duration: $0.duration)
            }
            DispatchQueue.main.async { self?.check(words) }
        }
    }

    private func check(_ words: [ReadAloudCommand.Word]) {
        guard !heard, finished != nil else { return }
        if bargeIn, ReadAloudCommand.latest(in: words, phrases: phrases) == nil,
           VoiceLoop.reduce(.speaking, .userSpoke(words: words.count)) == .listening {
            heard = true
            return onCommand?(.skip) ?? ()
        }
        guard let c = ReadAloudCommand.latest(in: words, phrases: phrases) else { return }
        heard = true
        onCommand?(c)
    }
}
