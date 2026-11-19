import AVFoundation
import XCTest
@testable import Loft
@testable import LoftKit

/// Talk picks the recipient from what you say (#343): naming an agent switches the To: row before anything is sent,
/// and the addressing words aren't in the message.
@MainActor
final class TalkRecipientTests: XCTestCase {
    /// Hears `text` when the dictation ends.
    private final class SayingRecognizer: SegmentRecognizer {
        let text: String
        init(_ text: String) { self.text = text }
        var onPartial: ((String) -> Void)?
        var onCommit: ((String) -> Void)?
        var onError: ((String) -> Void)?
        var inputFormat: AVAudioFormat? { nil }
        func prepare() async throws {}
        func start() throws {}
        func append(_ buffer: AVAudioPCMBuffer) {}
        func flush() async { onCommit?(text) }
        func cancel() {}
    }

    private func agent(_ id: String, _ name: String, role: String) -> Agent {
        var a = Agent(id: id, name: name, status: "idle")
        a.role = role
        return a
    }

    /// Talk with `id`, saying `text`, finished to review.
    private func say(_ text: String, picks: Bool = true) async throws -> VoiceSession {
        let api = MockAPI()
        api.agentList = [agent("lead", "Loft Lead", role: "lead"), agent("dev", "Loft Dev", role: "dev"), agent("mobile", "Loft mobile dev", role: "dev")]
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("t-\(UUID()).json")))
        try await store.refresh()
        let owner = AudioSessionOwner(AudioSessionOwnerTests.FakeSession(), attempts: 1, retryWait: .milliseconds(1), linger: .milliseconds(1), settle: .milliseconds(1))
        let session = VoiceSession(store: store, speaker: Speaker(audio: owner), audio: owner, engine: MicrophoneTests.FakeEngine())
        session.picksRecipient = picks
        session.prepareRecognizer = { SayingRecognizer(text) }
        addTeardownBlock { session.discard() }
        await session.start(agentId: "lead")
        await session.finish(.review)
        XCTAssertEqual(session.phase, .review)
        return session
    }

    func testNamingAnAgentSwitchesToItAndDropsTheWords() async throws {
        let s = try await say("Send this to Loft Dev, the build is red.")
        XCTAssertEqual(s.agentId, "dev")
        XCTAssertEqual(s.raw, "The build is red.")
        XCTAssertEqual(s.recipientNote, "To Loft Dev, from what you said.")
    }

    func testAnAmbiguousNameKeepsTheRecipientAndSaysSo() async throws {
        let s = try await say("Dev, can you fix the build?")
        XCTAssertEqual(s.agentId, "lead")
        XCTAssertEqual(s.raw, "Dev, can you fix the build?")
        XCTAssertEqual(s.recipientNote, "Didn't catch who: still to Loft Lead.")
    }

    func testNoNameChangesNothing() async throws {
        let s = try await say("The build is red.")
        XCTAssertEqual(s.agentId, "lead")
        XCTAssertEqual(s.raw, "The build is red.")
        XCTAssertNil(s.recipientNote)
    }

    /// Only the iPhone's Talk screen picks; the Mac's voice panel doesn't.
    func testOffItChangesNothing() async throws {
        let s = try await say("Send this to Loft Dev, the build is red.", picks: false)
        XCTAssertEqual(s.agentId, "lead")
        XCTAssertNil(s.recipientNote)
    }
}
