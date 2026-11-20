import AVFoundation
import XCTest
@testable import Loft
@testable import LoftKit

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
        var route = "in MicrophoneBuiltIn, out Speaker"
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
        owner = AudioSessionOwner(fake, attempts: 3, retryWait: .milliseconds(1), linger: .milliseconds(20), settle: .milliseconds(30))
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

    /// Every start, stop and hand-over in the voice log says the route, the session category and who holds it.
    func testTheLogShowsRouteCategoryAndHolderAtEachStep() async throws {
        _ = try await owner.acquire(.listening, by: "dictation") { _ in }
        let reading = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        owner.release(reading)
        try await Task.sleep(for: .milliseconds(60))
        _ = try await owner.acquire(.listening, by: "Talk") { _ in }
        owner.interrupted(began: true)
        let lines = log().split(separator: "\n").filter { $0.contains("audio:") }
        XCTAssertGreaterThanOrEqual(lines.count, 6)
        for line in lines {
            XCTAssertTrue(line.contains("route in MicrophoneBuiltIn, out Speaker"), "no route: \(line)")
            XCTAssertTrue(line.contains("category "), "no category: \(line)")
            XCTAssertTrue(line.contains("held by "), "no holder: \(line)")
        }
        XCTAssertTrue(log().contains("handing over from dictation (listening) to read-aloud (speaking) [route in MicrophoneBuiltIn, out Speaker, category PlayAndRecord, held by dictation (listening)]"))
        XCTAssertTrue(log().contains("read-aloud let go [route in MicrophoneBuiltIn, out Speaker, category PlayAndRecord, held by nobody]"))
        XCTAssertTrue(log().contains("session off"))
    }

    func testAReleaseFromAnEarlierHolderDoesNotEndTheNewOne() async throws {
        let old = try await owner.acquire(.listening, by: "dictation") { _ in }
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        owner.release(old)
        XCTAssertEqual(owner.heldFor, .speaking)
    }

    // MARK: - Route changes

    private let car = "in BluetoothHFP, out BluetoothHFP"

    /// The car's Bluetooth switching to call quality when the mic starts reports several changes in a row.
    private func flap(_ routes: [String]) {
        for r in routes {
            fake.route = r
            owner.routeChanged(reason: "category change")
        }
    }

    func testAFlappingRouteRestartsTheMicOnceItSettles() async throws {
        var restarts = 0
        var lost: AudioSessionOwner.Loss?
        _ = try await owner.acquire(.listening, by: "dictation", onRouteChange: { restarts += 1 }) { lost = $0 }
        flap(["in BluetoothHFP, out BluetoothA2DPOutput", car, "in BluetoothHFP, out BluetoothA2DPOutput", car])
        XCTAssertEqual(restarts, 0, "nothing happens while the route is still changing")
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(restarts, 1, "one restart once the route has settled")
        XCTAssertNil(lost)
        XCTAssertEqual(owner.heldFor, .listening, "the mic stays on")
        XCTAssertEqual(fake.events.filter { $0 == "on" || $0 == "off" }, ["on"], "the session never turns off and on")
        XCTAssertTrue(log().contains("route settled on \(car); dictation restarts the mic"))
        // Nothing more changes: no more restarts.
        try await Task.sleep(for: .milliseconds(60))
        XCTAssertEqual(restarts, 1)
    }

    func testARouteThatComesBackChangesNothing() async throws {
        var restarts = 0
        _ = try await owner.acquire(.listening, by: "Talk", onRouteChange: { restarts += 1 }) { _ in }
        let start = fake.route
        flap([car, start])
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(restarts, 0)
        XCTAssertEqual(owner.heldFor, .listening)
        XCTAssertTrue(log().contains("route settled back"))
    }

    func testARouteWithNoMicrophoneEndsTheListenerWithAReason() async throws {
        var lost: AudioSessionOwner.Loss?
        var restarts = 0
        _ = try await owner.acquire(.listening, by: "dictation", onRouteChange: { restarts += 1 }) { lost = $0 }
        fake.isInputAvailable = false
        flap(["in none, out BluetoothA2DPOutput"])
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(lost, .noMicrophone)
        XCTAssertEqual(restarts, 0)
        XCTAssertNil(owner.heldFor)
        XCTAssertTrue(log().contains("no microphone on in none, out BluetoothA2DPOutput; dictation ends"))
        // Reading aloud still works on that route.
        _ = try await owner.acquire(.speaking, by: "read-aloud") { _ in }
        XCTAssertEqual(owner.heldFor, .speaking)
    }

    func testARouteChangeWhileSpeakingLeavesTheReadingAlone() async throws {
        var lost: AudioSessionOwner.Loss?
        fake.isInputAvailable = false
        _ = try await owner.acquire(.speaking, by: "read-aloud") { lost = $0 }
        flap([car])
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertNil(lost)
        XCTAssertEqual(owner.heldFor, .speaking)
    }

    func testClosingTheChatDuringARouteChangeThenReopeningIt() async throws {
        var calls = 0
        let old = try await owner.acquire(.listening, by: "dictation", onRouteChange: { calls += 1 }) { _ in calls += 1 }
        flap([car])
        owner.release(old) // the chat closes before the route settles
        var restarts = 0
        _ = try await owner.acquire(.listening, by: "dictation", onRouteChange: { restarts += 1 }) { _ in }
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(calls, 0, "the closed chat's mic is never called back")
        XCTAssertEqual(restarts, 0, "the reopened chat started on the settled route")
        XCTAssertEqual(owner.heldFor, .listening)
    }
}
