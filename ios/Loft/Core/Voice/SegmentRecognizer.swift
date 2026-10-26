import AVFoundation
import Foundation
import Speech

/// Continuous speech recognition delivered as segments. A segment is committed
/// whenever the underlying recognizer finalizes text (its ~1 minute limit, a long
/// pause, or a pause in the session); the next one starts without a gap.
protocol SegmentRecognizer: AnyObject {
    /// Text heard since the last commit. Replaces the previous partial.
    var onPartial: ((String) -> Void)? { get set }
    /// A finished segment, in order.
    var onCommit: ((String) -> Void)? { get set }
    var onError: ((String) -> Void)? { get set }

    /// Checks support and downloads models if needed. Throws a user-facing message.
    func prepare() async throws
    /// The audio format `append` expects, or nil for the microphone's own.
    var inputFormat: AVAudioFormat? { get }
    func start() throws
    /// Called from the audio thread.
    func append(_ buffer: AVAudioPCMBuffer)
    /// Ends the current segment and commits it (used for pause and finish).
    func flush() async
    func cancel()
}

struct VoiceError: LocalizedError {
    let message: String
    /// Fixed by allowing access in Loft's page in iOS Settings (microphone, speech recognition).
    let isPermission: Bool
    var errorDescription: String? { message }
    init(_ message: String, isPermission: Bool = false) {
        self.message = message
        self.isPermission = isPermission
    }
}

enum Recognizers {
    static func make(locale: Locale) -> SegmentRecognizer {
        if TestMode.noSpeech { return SilentRecognizer() }
        if #available(iOS 26.0, *), Speech.SpeechTranscriber.isAvailable {
            return AnalyzerSegmentRecognizer(locale: locale)
        }
        return ChainedSegmentRecognizer(locale: locale)
    }
}

/// `-no-speech` (UI tests): starts at once and never hears anything.
final class SilentRecognizer: SegmentRecognizer {
    var onPartial: ((String) -> Void)?
    var onCommit: ((String) -> Void)?
    var onError: ((String) -> Void)?
    var inputFormat: AVAudioFormat? { nil }
    func prepare() async throws {}
    func start() throws {}
    func append(_ buffer: AVAudioPCMBuffer) {}
    func flush() async {}
    func cancel() {}
}

/// Tells a recognizer that can't start apart from a pause. A request that fails within moments
/// without hearing anything, several times in a row, means the recognizer is broken (no model,
/// can't initialize); restarting would loop forever.
struct StartupFailureGuard {
    static let limit = 3
    static let quick: TimeInterval = 2
    private(set) var consecutive = 0

    /// Returns true when it's time to give up.
    mutating func record(failed: Bool, heard: Bool, lasted: TimeInterval) -> Bool {
        if heard { consecutive = 0; return false }
        guard failed, lasted < Self.quick else { return false }
        consecutive += 1
        return consecutive >= Self.limit
    }
}

// MARK: - iOS 17–18: chained SFSpeechRecognizer requests

/// Apple caps one recognition request at about a minute and ends it after a long
/// pause, so this rolls over to a fresh request before that, and again whenever
/// one ends by itself. The microphone tap never stops; buffers always go to the
/// newest request, so no audio is lost at the seams. Rollover waits for a short
/// pause after `segmentDuration`, so a word isn't cut in half between two requests;
/// if you never pause, it happens anyway shortly before the limit.
final class ChainedSegmentRecognizer: SegmentRecognizer {
    var onPartial: ((String) -> Void)?
    var onCommit: ((String) -> Void)?
    var onError: ((String) -> Void)?
    var inputFormat: AVAudioFormat? { nil }

    /// Roll over at the first pause after this long…
    let segmentDuration: TimeInterval
    /// …or after this long regardless (Apple stops a request at about a minute).
    let hardLimit: TimeInterval
    /// A pause: this long below `quietLevel`.
    private let pauseLength: TimeInterval = 0.25
    private let quietLevel: Float = 0.25
    /// Audio-thread state, under `lock`.
    private var segmentStart = Date()
    private var quietSince: Date?
    private var rolloverRequested = false

    private let recognizer: SFSpeechRecognizer?
    private let lock = NSLock()
    private var current: (id: Int, request: SFSpeechAudioBufferRecognitionRequest)?
    private var tasks: [Int: SFSpeechRecognitionTask] = [:]
    /// Latest text of every segment not yet committed, by id.
    private var texts: [Int: String] = [:]
    private var done: Set<Int> = []
    private var nextId = 0
    private var nextToCommit = 0
    private var rollover: Timer?
    private var stopped = false
    private var startup = StartupFailureGuard()
    private var segmentStartedAt: [Int: Date] = [:]

    init(locale: Locale, segmentDuration: TimeInterval = 40, hardLimit: TimeInterval? = nil) {
        recognizer = SFSpeechRecognizer(locale: locale)
        self.segmentDuration = segmentDuration
        self.hardLimit = hardLimit ?? min(55, segmentDuration + 15)
    }

    func prepare() async throws {
        guard let recognizer else { throw VoiceError("Dictation doesn't support this language. Pick another one in Settings.") }
        guard recognizer.isAvailable else { throw VoiceError("Speech recognition isn't available right now.") }
        guard recognizer.supportsOnDeviceRecognition else {
            throw VoiceError("Long sessions need on-device recognition, which this language doesn't have on this iPhone.")
        }
    }

    func start() throws {
        stopped = false
        startup = StartupFailureGuard()
        beginSegment()
    }

    func append(_ buffer: AVAudioPCMBuffer) {
        let level = VoiceSession.rms(buffer)
        let now = Date()
        lock.lock()
        let request = current?.request
        let age = now.timeIntervalSince(segmentStart)
        if level < quietLevel { quietSince = quietSince ?? now } else { quietSince = nil }
        let paused = quietSince.map { now.timeIntervalSince($0) >= pauseLength } ?? false
        let due = !rolloverRequested && request != nil && ((age >= segmentDuration && paused) || age >= hardLimit)
        if due { rolloverRequested = true }
        lock.unlock()
        request?.append(buffer)
        if due { DispatchQueue.main.async { [weak self] in self?.rollOver() } }
    }

    private func beginSegment() {
        guard let recognizer, !stopped else { return }
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.shouldReportPartialResults = true
        // On-device has no daily quota and keeps audio on the phone.
        req.requiresOnDeviceRecognition = true
        req.addsPunctuation = true
        let id = nextId
        nextId += 1
        texts[id] = ""
        segmentStartedAt[id] = Date()
        lock.lock()
        current = (id, req)
        segmentStart = Date()
        quietSince = nil
        rolloverRequested = false
        lock.unlock()
        tasks[id] = recognizer.recognitionTask(with: req) { [weak self] result, error in
            let text = result?.bestTranscription.formattedString
            let final = result?.isFinal ?? false
            let message = error?.localizedDescription
            DispatchQueue.main.async { self?.handle(id: id, text: text, isFinal: final, error: message) }
        }
        // Backstop if audio stops arriving (rollover is otherwise driven by `append`).
        rollover?.invalidate()
        rollover = Timer.scheduledTimer(withTimeInterval: hardLimit, repeats: false) { [weak self] _ in
            self?.rollOver()
        }
    }

    /// End the current request (it delivers its final text) and keep listening on a new one.
    private func rollOver() {
        lock.lock()
        let old = current
        lock.unlock()
        beginSegment()
        old?.request.endAudio()
    }

    private func handle(id: Int, text: String?, isFinal: Bool, error: String?) {
        guard texts[id] != nil else { return }
        if let text { texts[id] = text }
        if isFinal || error != nil {
            done.insert(id)
            tasks[id] = nil
            let heard = !(texts[id] ?? "").isEmpty
            let lived = Date().timeIntervalSince(segmentStartedAt.removeValue(forKey: id) ?? Date())
            // A request that fails at once without hearing anything (no model, recognizer can't start) isn't
            // a pause: after a few in a row, stop and report it instead of restarting forever.
            if let error, !heard { Log.speech.info("segment \(id) failed: \(error, privacy: .public)") }
            if startup.record(failed: error != nil, heard: heard, lasted: lived) {
                cancel()
                onError?("Speech recognition couldn't start on this device (\(error ?? "unknown error")). Check that dictation is on and its language is downloaded in Settings › General › Keyboard.")
                return
            }
            // The request ended by itself (a long pause, or the limit): carry on with a new one.
            lock.lock()
            let wasCurrent = current?.id == id
            lock.unlock()
            if wasCurrent && !stopped { beginSegment() }
        }
        emit()
    }

    /// Commit finished segments in order, then report what's still in flight as the partial.
    private func emit() {
        while done.contains(nextToCommit) {
            let t = texts.removeValue(forKey: nextToCommit) ?? ""
            done.remove(nextToCommit)
            nextToCommit += 1
            if !t.trimmingCharacters(in: .whitespaces).isEmpty { onCommit?(t) }
        }
        onPartial?(VoiceText.join(texts.keys.sorted().compactMap { texts[$0] }))
    }

    func flush() async {
        rollover?.invalidate()
        let old = lock.withLock {
            defer { current = nil }
            return current
        }
        old?.request.endAudio()
        // Wait (briefly) for every outstanding segment to deliver its final text.
        for _ in 0..<40 where !texts.isEmpty {
            try? await Task.sleep(for: .milliseconds(100))
        }
        // Whatever didn't finalize in time is committed as heard.
        for id in texts.keys { done.insert(id) }
        tasks.values.forEach { $0.cancel() }
        tasks.removeAll()
        emit()
    }

    func cancel() {
        stopped = true
        rollover?.invalidate()
        tasks.values.forEach { $0.cancel() }
        tasks.removeAll()
        texts.removeAll()
        done.removeAll()
        lock.lock()
        current = nil
        lock.unlock()
    }
}

// MARK: - iOS 26: SpeechAnalyzer (built for long-form audio, fully on-device)

@available(iOS 26.0, *)
final class AnalyzerSegmentRecognizer: SegmentRecognizer {
    var onPartial: ((String) -> Void)?
    var onCommit: ((String) -> Void)?
    var onError: ((String) -> Void)?
    private(set) var inputFormat: AVAudioFormat?

    private let requested: Locale
    private var locale: Locale?
    private var transcriber: Speech.SpeechTranscriber?
    private var analyzer: SpeechAnalyzer?
    private var input: AsyncStream<AnalyzerInput>.Continuation?
    private var resultsTask: Task<Void, Never>?
    private let lock = NSLock()
    private var converter: AVAudioConverter?

    init(locale: Locale) { requested = locale }

    func prepare() async throws {
        guard let locale = await Speech.SpeechTranscriber.supportedLocale(equivalentTo: requested) else {
            throw VoiceError("Long sessions don't support \(requested.identifier) yet. Pick another language in Settings.")
        }
        self.locale = locale
        let t = Self.transcriber(locale)
        if await AssetInventory.status(forModules: [t]) != .installed {
            if let request = try await AssetInventory.assetInstallationRequest(supporting: [t]) {
                try await request.downloadAndInstall()
            }
        }
        inputFormat = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [t])
    }

    private static func transcriber(_ locale: Locale) -> Speech.SpeechTranscriber {
        Speech.SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [])
    }

    func start() throws {
        guard let locale else { throw VoiceError("Speech recognition wasn't prepared.") }
        // A fresh module per analyzer run: resume after a pause starts a new one.
        let transcriber = Self.transcriber(locale)
        self.transcriber = transcriber
        let (stream, continuation) = AsyncStream<AnalyzerInput>.makeStream()
        lock.lock()
        input = continuation
        lock.unlock()
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        self.analyzer = analyzer
        resultsTask = Task { @MainActor [weak self] in
            do {
                for try await r in transcriber.results {
                    let text = String(r.text.characters)
                    if r.isFinal { self?.onCommit?(text); self?.onPartial?("") } else { self?.onPartial?(text) }
                }
            } catch {
                self?.onError?(error.localizedDescription)
            }
        }
        Task {
            do { try await analyzer.start(inputSequence: stream) } catch { await MainActor.run { self.onError?(error.localizedDescription) } }
        }
    }

    func append(_ buffer: AVAudioPCMBuffer) {
        lock.lock()
        let input = self.input
        lock.unlock()
        guard let input else { return }
        input.yield(AnalyzerInput(buffer: convert(buffer) ?? buffer))
    }

    /// The analyzer wants its own format (usually 16 kHz mono); the mic delivers 44.1/48 kHz.
    private func convert(_ buffer: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
        guard let target = inputFormat, buffer.format != target else { return nil }
        if converter == nil || converter?.inputFormat != buffer.format {
            converter = AVAudioConverter(from: buffer.format, to: target)
        }
        guard let converter else { return nil }
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * target.sampleRate / buffer.format.sampleRate) + 1
        guard let out = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return nil }
        var fed = false
        var err: NSError?
        converter.convert(to: out, error: &err) { _, status in
            if fed { status.pointee = .noDataNow; return nil }
            fed = true
            status.pointee = .haveData
            return buffer
        }
        return err == nil ? out : nil
    }

    func flush() async {
        let input = lock.withLock {
            defer { self.input = nil }
            return self.input
        }
        input?.finish()
        try? await analyzer?.finalizeAndFinishThroughEndOfInput()
        await resultsTask?.value
        resultsTask = nil
        analyzer = nil
    }

    func cancel() {
        lock.lock()
        input?.finish()
        input = nil
        lock.unlock()
        resultsTask?.cancel()
        let a = analyzer
        analyzer = nil
        Task { await a?.cancelAndFinishNow() }
    }
}
