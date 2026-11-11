import AVFoundation
import XCTest
@testable import Loft

/// The one owner of the audio session: the mic and the speaker never hold it together, a refused activation
/// is retried, and an interruption ends whoever holds it.
@MainActor
final class AudioSessionOwnerTests: XCTestCase {
    /// An audio session that records every call and can refuse activation or have no microphone.
    final class FakeSession: AudioSessionAPI {
        var events: [String] = []
        var category: AVAudioSession.Category = .soloAmbient
        var isInputAvailable = true
        var isOtherAudioPlaying = false
        var route: String { "in fake, out fake" }
        /// How many activations to refuse before accepting one.
        var refusals = 0

        func setCategory(_ category: AVAudioSession.Category, mode: AVAudioSession.Mode, options: AVAudioSession.CategoryOptions) throws {
            self.category = category
            events.append("category \(category.rawValue)")
        }

        func setActive(_ active: Bool) throws {
            if active && refusals > 0 {
                refusals -= 1
                events.append("refused")
                throw NSError(domain: NSOSStatusErrorDomain, code: 561_017_449) // '!pri': insufficient priority
            }
            events.append(active ? "on" : "off")
        }
    }

    private var fake: FakeSession!
    private var owner: AudioSessionOwner!

    override func setUp() async throws {
        fake = FakeSession()
        owner = AudioSessionOwner(fake, attempts: 3, retryWait: .milliseconds(1), linger: .milliseconds(20))
        VoiceLog.shared.clear()
    }

    private func log() -> String { VoiceLog.shared.plain }

    func testTheMicHandsTheAudioToTheSpeaker() async throws {
        _ = try await owner.acquire(.listening, by: "dictation") { _ in self.fake.events.append("dictation let go") }
        XCTAssertEqual(owner.heldFor, .listening)
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        XCTAssertEqual(owner.heldFor, .speaking)
        XCTAssertEqual(fake.events.filter { $0 != "category AVAudioSessionCategoryPlayAndRecord" },
                       ["on", "dictation let go", "on"], "the mic lets go before the speaker turns the audio on")
        XCTAssertTrue(log().contains("handing over from dictation (listening) to read-aloud (speaking)"))
    }

    func testTheSpeakerHandsTheAudioToTheMic() async throws {
        var loss: AudioSessionOwner.Loss?
        _ = try await owner.acquire(.speaking, by: "read-aloud") { loss = $0 }
        XCTAssertEqual(fake.category, .playback)
        _ = try await owner.acquire(.listening, by: "Talk") { _ in }
        XCTAssertEqual(loss, .handedOver)
        XCTAssertEqual(owner.heldFor, .listening)
        XCTAssertEqual(fake.category, .playAndRecord)
        XCTAssertTrue(log().contains("handing over from read-aloud (speaking) to Talk (listening)"))
    }

    func testAReadingRightAfterTheMicStopsStartsWithoutTurningTheSessionOff() async throws {
        let mic = try await owner.acquire(.listening, by: "dictation") { _ in }
        owner.release(mic)
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        try await Task.sleep(for: .milliseconds(60))
        XCTAssertEqual(owner.heldFor, .speaking)
        XCTAssertFalse(fake.events.contains("off"), "a quick hand-over keeps the session on")
    }

    func testTheSessionTurnsOffOnceNobodyHoldsIt() async throws {
        let id = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        owner.release(id)
        XCTAssertNil(owner.heldFor)
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(fake.events.last, "off")
    }

    func testARefusedActivationIsRetried() async throws {
        fake.refusals = 2
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        XCTAssertEqual(owner.heldFor, .speaking)
        XCTAssertEqual(fake.events.filter { $0 == "refused" }.count, 2)
        XCTAssertTrue(log().contains("audio: refused for read-aloud"))
        XCTAssertTrue(log().contains("read-aloud holds it (speaking"))
    }

    func testARefusalThatLastsFailsAndTheNextRequestWorks() async throws {
        fake.refusals = 3
        do {
            _ = try await owner.acquire(.listening, by: "Talk") { _ in }
            XCTFail("should be refused")
        } catch {
            XCTAssertTrue(error.localizedDescription.contains("wasn't available"))
        }
        XCTAssertNil(owner.heldFor)
        _ = try await owner.acquire(.listening, by: "Talk") { _ in }
        XCTAssertEqual(owner.heldFor, .listening)
    }

    func testNoMicrophoneRefusesListeningButNotSpeaking() async throws {
        fake.isInputAvailable = false
        do {
            _ = try await owner.acquire(.listening, by: "dictation") { _ in }
            XCTFail("should be refused")
        } catch {
            XCTAssertEqual(error.localizedDescription, "No microphone is available.")
        }
        XCTAssertNil(owner.heldFor)
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        XCTAssertEqual(owner.heldFor, .speaking)
    }

    func testAnInterruptionEndsWhoeverHoldsItAndStartingAgainWorks() async throws {
        var loss: AudioSessionOwner.Loss?
        let id = try await owner.acquire(.speaking, by: "read-aloud") { loss = $0 }
        owner.interrupted(began: true)
        XCTAssertEqual(loss, .interrupted)
        XCTAssertNil(owner.heldFor)
        owner.release(id) // the holder's own cleanup after losing it changes nothing
        owner.interrupted(began: false)
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        XCTAssertEqual(owner.heldFor, .speaking)
        XCTAssertTrue(log().contains("interrupted; read-aloud (speaking) ends"))
    }

    func testAReleaseFromAnEarlierHolderDoesNotEndTheNewOne() async throws {
        let old = try await owner.acquire(.listening, by: "dictation") { _ in }
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        owner.release(old)
        XCTAssertEqual(owner.heldFor, .speaking)
    }
}
