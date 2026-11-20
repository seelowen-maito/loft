import AVFoundation
import Foundation

/// The one guarded way every listener (dictation, the Talk screen, the listening reader) starts the
/// microphone. `installTap` raises an Objective-C exception, which Swift can't catch, so the app aborts, when
/// the input has no usable format (no microphone on the route), when its format no longer matches the hardware
/// (a Bluetooth route switching to call quality changes the sample rate), or when the bus already has a tap.
/// These are checked first and become a `VoiceError` instead.
@MainActor
public enum Microphone {
    /// Starts `engine` with one tap on its input, replacing any tap already there, so starting twice never
    /// leaves two listeners.
    public static func start(_ engine: MicEngine, bufferSize: AVAudioFrameCount, block: @escaping AVAudioNodeTapBlock) throws {
        try installTap(engine, bufferSize: bufferSize, block: block)
        engine.prepare()
        do {
            try engine.start()
        } catch {
            stop(engine)
            throw error
        }
    }

    /// The first start on a route: a Bluetooth route can still be switching to call quality, so a format
    /// mismatch is retried a few times before it fails. Gives up with `CancellationError` once `stillWanted`
    /// says the start was overtaken (stopped, or started again) while it waited.
    public static func startWhenSettled(_ engine: MicEngine, bufferSize: AVAudioFrameCount, attempts: Int = 3,
                                 wait: Duration = .milliseconds(200), stillWanted: () -> Bool,
                                 block: @escaping AVAudioNodeTapBlock) async throws {
        var attempt = 1
        while true {
            do {
                return try start(engine, bufferSize: bufferSize, block: block)
            } catch is StillSwitching where attempt < attempts {
                VoiceLog.shared.add("mic: the route is still switching; retrying (\(attempt) of \(attempts - 1))")
                try await Task.sleep(for: wait)
                guard stillWanted() else { throw CancellationError() }
                attempt += 1
            }
        }
    }

    /// The input's format doesn't match the hardware yet: the route is switching.
    public struct StillSwitching: LocalizedError {
        public let errorDescription: String?
    }

    /// Stops the engine and puts one checked tap on its input, without starting it (for an engine that
    /// starts once it has something to play).
    public static func installTap(_ engine: MicEngine, bufferSize: AVAudioFrameCount, block: @escaping AVAudioNodeTapBlock) throws {
        stop(engine)
        let format = engine.micFormat
        let hardware = engine.hardwareFormat
        guard format.channelCount > 0, format.sampleRate > 0, hardware.sampleRate > 0 else {
            throw VoiceError("No microphone is available on this audio route.")
        }
        VoiceLog.shared.add("mic formats: tap \(describe(format)), hardware \(describe(hardware)), voice processing \(engine.isVoiceProcessing ? "on" : "off")")
        // With voice processing on (the listening reader), the input delivers its processed format, which can
        // differ from the hardware's, so only a usable format is required there.
        guard engine.isVoiceProcessing || format.sampleRate == hardware.sampleRate else {
            throw StillSwitching(errorDescription: "The microphone is still switching audio route (\(Int(format.sampleRate)) Hz, hardware \(Int(hardware.sampleRate)) Hz). Try again.")
        }
        engine.installMicTap(bufferSize: bufferSize, format: format, block: block)
    }

    private static func describe(_ f: AVAudioFormat) -> String { "\(Int(f.sampleRate)) Hz \(f.channelCount) ch" }

    /// Stops the engine and removes its tap. Safe to call when nothing is running.
    public static func stop(_ engine: MicEngine) {
        if engine.isRunning { engine.stop() }
        engine.removeMicTap()
    }
}

/// One start at a time: a start that is overtaken (stopped, or started again) while it waits for a permission
/// prompt or the audio gives up instead of starting a second listener.
public struct StartGate {
    private var current = 0
    private var open = false

    /// Begins a start; keep the number and check it after every wait.
    public mutating func begin() -> Int {
        current += 1
        open = true
        return current
    }

    /// Whether the start numbered `n` is still the one wanted.
    public func isCurrent(_ n: Int) -> Bool { open && n == current }

    /// A stop: any start still waiting gives up.
    public mutating func cancel() { open = false }
}

/// The parts of an `AVAudioEngine` the guarded start uses, so tests can swap in a fake.
public protocol MicEngine: AnyObject {
    var isRunning: Bool { get }
    /// The format the input delivers to a tap.
    var micFormat: AVAudioFormat { get }
    /// The microphone hardware's format on the current route.
    var hardwareFormat: AVAudioFormat { get }
    /// Whether the input runs Apple's voice processing (echo cancellation), as the listening reader's does.
    var isVoiceProcessing: Bool { get }
    func installMicTap(bufferSize: AVAudioFrameCount, format: AVAudioFormat, block: @escaping AVAudioNodeTapBlock)
    func removeMicTap()
    func prepare()
    func start() throws
    func stop()
}

extension AVAudioEngine: MicEngine {
    public var micFormat: AVAudioFormat { inputNode.outputFormat(forBus: 0) }
    public var hardwareFormat: AVAudioFormat { inputNode.inputFormat(forBus: 0) }
    public var isVoiceProcessing: Bool { inputNode.isVoiceProcessingEnabled }
    public func installMicTap(bufferSize: AVAudioFrameCount, format: AVAudioFormat, block: @escaping AVAudioNodeTapBlock) {
        inputNode.installTap(onBus: 0, bufferSize: bufferSize, format: format, block: block)
    }
    public func removeMicTap() { inputNode.removeTap(onBus: 0) }
}
