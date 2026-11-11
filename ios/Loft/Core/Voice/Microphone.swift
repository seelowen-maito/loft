import AVFoundation
import Foundation

/// The one guarded way every listener (dictation, the Talk screen, the listening reader) starts the
/// microphone. `installTap` raises an Objective-C exception, which Swift can't catch, so the app aborts, when
/// the input has no usable format (no microphone on the route), when its format no longer matches the hardware
/// (a Bluetooth route switching to call quality changes the sample rate), or when the bus already has a tap.
/// These are checked first and become a `VoiceError` instead.
enum Microphone {
    /// Starts `engine` with one tap on its input, replacing any tap already there, so starting twice never
    /// leaves two listeners.
    static func start(_ engine: MicEngine, bufferSize: AVAudioFrameCount, block: @escaping AVAudioNodeTapBlock) throws {
        try installTap(engine, bufferSize: bufferSize, block: block)
        engine.prepare()
        do {
            try engine.start()
        } catch {
            stop(engine)
            throw error
        }
    }

    /// Stops the engine and puts one checked tap on its input, without starting it (for an engine that
    /// starts once it has something to play).
    static func installTap(_ engine: MicEngine, bufferSize: AVAudioFrameCount, block: @escaping AVAudioNodeTapBlock) throws {
        stop(engine)
        let format = engine.micFormat
        let hardware = engine.hardwareFormat
        guard format.channelCount > 0, format.sampleRate > 0, hardware.sampleRate > 0 else {
            throw VoiceError("No microphone is available on this audio route.")
        }
        guard format.sampleRate == hardware.sampleRate else {
            throw VoiceError("The microphone is still switching audio route (\(Int(format.sampleRate)) Hz, hardware \(Int(hardware.sampleRate)) Hz). Try again.")
        }
        engine.installMicTap(bufferSize: bufferSize, format: format, block: block)
    }

    /// Stops the engine and removes its tap. Safe to call when nothing is running.
    static func stop(_ engine: MicEngine) {
        if engine.isRunning { engine.stop() }
        engine.removeMicTap()
    }
}

/// One start at a time: a start that is overtaken (stopped, or started again) while it waits for a permission
/// prompt or the audio gives up instead of starting a second listener.
struct StartGate {
    private var current = 0
    private var open = false

    /// Begins a start; keep the number and check it after every wait.
    mutating func begin() -> Int {
        current += 1
        open = true
        return current
    }

    /// Whether the start numbered `n` is still the one wanted.
    func isCurrent(_ n: Int) -> Bool { open && n == current }

    /// A stop: any start still waiting gives up.
    mutating func cancel() { open = false }
}

/// The parts of an `AVAudioEngine` the guarded start uses, so tests can swap in a fake.
protocol MicEngine: AnyObject {
    var isRunning: Bool { get }
    /// The format the input delivers to a tap.
    var micFormat: AVAudioFormat { get }
    /// The microphone hardware's format on the current route.
    var hardwareFormat: AVAudioFormat { get }
    func installMicTap(bufferSize: AVAudioFrameCount, format: AVAudioFormat, block: @escaping AVAudioNodeTapBlock)
    func removeMicTap()
    func prepare()
    func start() throws
    func stop()
}

extension AVAudioEngine: MicEngine {
    var micFormat: AVAudioFormat { inputNode.outputFormat(forBus: 0) }
    var hardwareFormat: AVAudioFormat { inputNode.inputFormat(forBus: 0) }
    func installMicTap(bufferSize: AVAudioFrameCount, format: AVAudioFormat, block: @escaping AVAudioNodeTapBlock) {
        inputNode.installTap(onBus: 0, bufferSize: bufferSize, format: format, block: block)
    }
    func removeMicTap() { inputNode.removeTap(onBus: 0) }
}
