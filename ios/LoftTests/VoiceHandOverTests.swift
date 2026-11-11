import AVFoundation
import XCTest
@testable import Loft

/// The callers of `AudioSessionOwner`: dictation and Talk let go when read-aloud takes the audio, and restart
/// their mic when the route settles on another one. The permission prompts and the real recognizers aren't
/// covered here: dictation starts past its checks, and Talk with a silent recognizer.
@MainActor
final class VoiceHandOverTests: XCTestCase {
    private typealias FakeSession = AudioSessionOwnerTests.FakeSession
    private typealias FakeEngine = MicrophoneTests.FakeEngine

    private var fake: FakeSession!
    private var owner: AudioSessionOwner!
    private var engine: FakeEngine!

    override func setUp() async throws {
        fake = FakeSession()
        owner = AudioSessionOwner(fake, attempts: 3, retryWait: .milliseconds(1), linger: .milliseconds(20), settle: .milliseconds(30))
        engine = FakeEngine()
        VoiceLog.shared.clear()
    }

    private func waitFor(_ what: String, _ condition: () -> Bool) async throws {
        for _ in 0..<100 where !condition() { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertTrue(condition(), what)
    }

    private func readAloudTakesTheAudio() async throws {
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
    }

    private func routeSettlesOnTheCar() async throws {
        fake.route = "in BluetoothHFP, out BluetoothHFP"
        owner.routeChanged(reason: "new device")
        try await Task.sleep(for: .milliseconds(100))
    }

    /// A Bluetooth route still switching to call quality: the first start retries.
    private func routeStillSwitching() {
        engine.hardwareFormat = AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1)!
    }

    // MARK: - Dictation

    func testReadAloudTakingTheAudioEndsTheDictation() async throws {
        let dictation = SpeechTranscriber(audio: owner, engine: engine)
        await dictation.startSkippingChecks()
        XCTAssertEqual(dictation.state, .recording)
        XCTAssertTrue(engine.isRunning)

        try await readAloudTakesTheAudio()
        XCTAssertEqual(dictation.state, .idle, "a hand-over ends the dictation without an error")
        XCTAssertFalse(engine.isRunning)
        XCTAssertEqual(engine.taps, 0)
        XCTAssertEqual(owner.heldFor, .speaking)
    }

    func testTheDictationMicRestartsWhenTheRouteSettlesOnAnotherOne() async throws {
        let dictation = SpeechTranscriber(audio: owner, engine: engine)
        await dictation.startSkippingChecks()
        XCTAssertEqual(engine.installs, 1)

        try await routeSettlesOnTheCar()
        XCTAssertEqual(engine.installs, 2, "the mic started again on the new route")
        XCTAssertEqual(engine.taps, 1)
        XCTAssertTrue(engine.isRunning)
        XCTAssertEqual(dictation.state, .recording)
        XCTAssertTrue(VoiceLog.shared.plain.contains("mic restarted on the new route"))
    }

    func testLosingTheAudioWhileTheFirstDictationStartRetriesEndsQuietly() async throws {
        routeStillSwitching()
        let dictation = SpeechTranscriber(audio: owner, engine: engine)
        let start = Task { await dictation.startSkippingChecks() }
        try await waitFor("the first start is retrying") { VoiceLog.shared.plain.contains("still switching; retrying") }

        try await readAloudTakesTheAudio()
        await start.value
        XCTAssertEqual(dictation.state, .idle, "no \"Couldn't start the microphone\" after a quiet stop")
        XCTAssertEqual(engine.installs, 0, "the retry gave up instead of starting the mic")
        XCTAssertEqual(owner.heldFor, .speaking)
    }

    // MARK: - Talk

    private func talk() -> VoiceSession {
        let store = LoftStore(api: MockAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("v-\(UUID()).json")))
        let session = VoiceSession(store: store, speaker: Speaker(audio: owner), audio: owner, engine: engine)
        session.prepareRecognizer = { SilentRecognizer() }
        addTeardownBlock { session.discard() }
        return session
    }

    func testReadAloudTakingTheAudioPausesTalk() async throws {
        let session = talk()
        await session.start(agentId: "a1")
        XCTAssertEqual(session.phase, .listening)
        XCTAssertTrue(engine.isRunning)

        try await readAloudTakesTheAudio()
        XCTAssertEqual(session.phase, .paused)
        XCTAssertFalse(engine.isRunning)
        XCTAssertEqual(engine.taps, 0)
        XCTAssertEqual(owner.heldFor, .speaking)
    }

    func testTalksMicRestartsWhenTheRouteSettlesOnAnotherOne() async throws {
        let session = talk()
        await session.start(agentId: "a1")
        XCTAssertEqual(engine.installs, 1)

        try await routeSettlesOnTheCar()
        XCTAssertEqual(engine.installs, 2, "the mic started again on the new route")
        XCTAssertEqual(engine.taps, 1)
        XCTAssertTrue(engine.isRunning)
        XCTAssertEqual(session.phase, .listening)
        XCTAssertTrue(VoiceLog.shared.plain.contains("Talk: mic restarted on the new route"))
    }

    func testLosingTheAudioWhileTalksFirstStartRetriesPauses() async throws {
        routeStillSwitching()
        let session = talk()
        let start = Task { await session.start(agentId: "a1") }
        try await waitFor("the first start is retrying") { VoiceLog.shared.plain.contains("still switching; retrying") }

        try await readAloudTakesTheAudio()
        await start.value
        XCTAssertEqual(session.phase, .paused, "paused, as losing the audio while listening does, not failed")
        XCTAssertEqual(engine.installs, 0, "the retry gave up instead of starting the mic")
        XCTAssertEqual(owner.heldFor, .speaking)
    }
}
