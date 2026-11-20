import AVFoundation
import XCTest
@testable import Loft
@testable import LoftKit

/// The guarded mic start: a route without a usable input throws instead of crashing, and starting again and
/// again never leaves two listeners.
@MainActor
final class MicrophoneTests: XCTestCase {
    /// An engine that, like `AVAudioEngine`, can't take a second tap or a tap whose format doesn't match the
    /// hardware: it records that as a crash.
    final class FakeEngine: MicEngine {
        var isRunning = false
        var micFormat = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1)!
        var hardwareFormat = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1)!
        var isVoiceProcessing = false
        var taps = 0
        var installs = 0
        var crashes: [String] = []

        func installMicTap(bufferSize: AVAudioFrameCount, format: AVAudioFormat, block: @escaping AVAudioNodeTapBlock) {
            if taps > 0 { crashes.append("second tap") }
            if format.channelCount == 0 || (!isVoiceProcessing && format.sampleRate != hardwareFormat.sampleRate) { crashes.append("invalid format") }
            taps += 1
            installs += 1
        }
        func removeMicTap() { taps = 0 }
        func prepare() {}
        func start() throws { isRunning = true }
        func stop() { isRunning = false }
    }

    private let engine = FakeEngine()
    private func start() throws { try Microphone.start(engine, bufferSize: 1024) { _, _ in } }

    func testPressingTheMicAgainAndAgainLeavesOneListener() throws {
        try start()
        try start()
        Microphone.stop(engine)
        try start()
        try start()
        XCTAssertEqual(engine.taps, 1)
        XCTAssertEqual(engine.installs, 4)
        XCTAssertTrue(engine.isRunning)
        XCTAssertEqual(engine.crashes, [])
    }

    func testARouteWithNoInputThrowsAClearMessage() {
        engine.micFormat = AVAudioFormat()
        engine.hardwareFormat = AVAudioFormat()
        XCTAssertThrowsError(try start()) {
            XCTAssertEqual($0.localizedDescription, "No microphone is available on this audio route.")
        }
        XCTAssertEqual(engine.installs, 0)
        XCTAssertFalse(engine.isRunning)
    }

    func testAFormatLeftOverFromTheOldRouteThrowsInsteadOfCrashing() {
        // Bluetooth call quality runs the hardware at 16 kHz while the input still reports the old 48 kHz.
        engine.hardwareFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
        XCTAssertThrowsError(try start()) {
            XCTAssertTrue($0.localizedDescription.contains("still switching audio route"))
        }
        XCTAssertEqual(engine.installs, 0)
        XCTAssertEqual(engine.crashes, [])
    }

    func testVoiceProcessingTakesItsOwnFormatAndLogsBoth() throws {
        // The listening reader turns voice processing on, and the input then delivers its processed format.
        engine.isVoiceProcessing = true
        engine.micFormat = AVAudioFormat(standardFormatWithSampleRate: 24_000, channels: 1)!
        engine.hardwareFormat = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1)!
        try Microphone.installTap(engine, bufferSize: 1024) { _, _ in }
        XCTAssertEqual(engine.installs, 1)
        XCTAssertEqual(engine.crashes, [])
        XCTAssertEqual(VoiceLog.shared.lines.last?.text, "mic formats: tap 24000 Hz 1 ch, hardware 48000 Hz 1 ch, voice processing on")
    }

    func testVoiceProcessingStillNeedsAUsableFormat() {
        engine.isVoiceProcessing = true
        engine.micFormat = AVAudioFormat()
        XCTAssertThrowsError(try start())
        XCTAssertEqual(engine.installs, 0)
    }

    private func startWhenSettled(stillWanted: @escaping () -> Bool = { true }) async throws {
        try await Microphone.startWhenSettled(engine, bufferSize: 1024, wait: .milliseconds(30), stillWanted: stillWanted) { _, _ in }
    }

    /// #371: the first start on a car's Bluetooth route catches it switching to call quality.
    func testAFirstStartWaitsForTheRouteToFinishSwitching() async throws {
        engine.hardwareFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
        let start = Task { try await startWhenSettled() }
        try await Task.sleep(for: .milliseconds(15))
        engine.micFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)! // it settles
        try await start.value
        XCTAssertEqual(engine.installs, 1)
        XCTAssertTrue(engine.isRunning)
        XCTAssertEqual(engine.crashes, [])
    }

    func testARouteThatNeverSettlesFailsAfterAFewTries() async {
        VoiceLog.shared.clear()
        engine.hardwareFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
        do {
            try await startWhenSettled()
            XCTFail("started on mismatched formats")
        } catch {
            XCTAssertTrue(error.localizedDescription.contains("still switching audio route"))
        }
        XCTAssertEqual(VoiceLog.shared.lines.filter { $0.text.hasPrefix("mic: the route is still switching") }.count, 2)
        XCTAssertEqual(engine.installs, 0)
    }

    func testAStartStoppedWhileWaitingForTheRouteGivesUp() async {
        engine.hardwareFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
        var wanted = true
        let start = Task { try await startWhenSettled(stillWanted: { wanted }) }
        try? await Task.sleep(for: .milliseconds(10))
        wanted = false // the mic button pressed off
        engine.micFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
        do {
            try await start.value
            XCTFail("started after the stop")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
        XCTAssertEqual(engine.installs, 0)
        XCTAssertFalse(engine.isRunning)
    }

    func testARestartOnTheNewRouteReplacesTheTap() throws {
        try start()
        engine.isRunning = false // the engine stops when the route changes
        engine.micFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
        engine.hardwareFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
        try start()
        XCTAssertEqual(engine.taps, 1)
        XCTAssertTrue(engine.isRunning)
        XCTAssertEqual(engine.crashes, [])
    }

    func testAStartOvertakenByAStopOrAnotherStartGivesUp() {
        var gate = StartGate()
        let first = gate.begin()
        gate.cancel() // the mic button pressed off while the first start waited
        let second = gate.begin() // and on again
        XCTAssertFalse(gate.isCurrent(first))
        XCTAssertTrue(gate.isCurrent(second))
        gate.cancel()
        XCTAssertFalse(gate.isCurrent(second))
    }
}
