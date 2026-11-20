import AVFoundation
import XCTest
@testable import Loft
@testable import LoftKit

/// The speaker as a holder of the audio: a wait that another request overtakes ends quietly, and the voice
/// preview doesn't play when the audio is refused.
@MainActor
final class SpeakerTests: XCTestCase {
    private var fake: AudioSessionOwnerTests.FakeSession!
    private var owner: AudioSessionOwner!
    private var speaker: Speaker!

    override func setUp() async throws {
        fake = AudioSessionOwnerTests.FakeSession()
        owner = AudioSessionOwner(fake, attempts: 5, retryWait: .milliseconds(50), linger: .milliseconds(20), settle: .milliseconds(30))
        speaker = Speaker(audio: owner)
        VoiceLog.shared.clear()
    }

    func testAWaitOvertakenByAnotherRequestIsNotAFailure() async throws {
        fake.refusals = 2 // the reading waits while the audio is refused
        let reading = Task { await speaker.speak("Hello there.", messageId: 7) }
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertTrue(speaker.isSpeaking, "still waiting for the audio")
        _ = try await owner.acquire(.listening, by: "dictation") { _ in }
        _ = await reading.value
        XCTAssertFalse(speaker.isSpeaking)
        XCTAssertNil(speaker.lastFailure, "a superseded wait isn't reported")
        XCTAssertNil(speaker.unreadMessageId, "nor marks the message unread")
        XCTAssertEqual(owner.heldFor, .listening)
    }

    func testAPreviewDoesNotPlayWhenTheAudioIsRefused() async throws {
        fake.refusals = 100
        let voice = try XCTUnwrap(AVSpeechSynthesisVoice(language: "en-US"))
        await speaker.preview(voice)
        XCTAssertFalse(speaker.isSpeaking)
        XCTAssertTrue(VoiceLog.shared.plain.contains("read-aloud: couldn't preview the voice: The audio wasn't available"))
    }
}
