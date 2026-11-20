import XCTest
@testable import Loft
@testable import LoftKit

/// A chat's auto-send lives in the store, so it goes once, whether or not the chat is still open (#339).
@MainActor
final class VoiceAutoSendTests: XCTestCase {
    private func store() -> LoftStore {
        LoftStore(api: MockAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("vs-\(UUID()).json")))
    }

    func testSendsOnceWhenTheWindowEnds() async throws {
        let s = store()
        var sent = 0
        s.scheduleVoiceSend("Deploy the docs", to: "a1", in: 0.2) { sent += 1 }
        XCTAssertEqual(s.voiceAutoSend?.text, "Deploy the docs")
        try await Task.sleep(for: .seconds(0.5))
        XCTAssertEqual(sent, 1)
        XCTAssertNil(s.voiceAutoSend)
        s.sendVoiceNow()
        XCTAssertEqual(sent, 1, "nothing left to send")
    }

    func testBackFromTheBackgroundSendsAnOverdueOneOnlyOnce() async throws {
        let s = store()
        var sent = 0
        s.scheduleVoiceSend("Deploy", to: "a1", in: 0.3) { sent += 1 }
        s.sendOverdueVoiceSend()
        XCTAssertEqual(sent, 0, "still in its window")
        s.sendOverdueVoiceSend(now: Date().addingTimeInterval(1))
        XCTAssertEqual(sent, 1, "overdue on return: sent at once")
        try await Task.sleep(for: .seconds(0.6))
        XCTAssertEqual(sent, 1, "the timer finds nothing to send")
    }

    func testUndoSendsNothing() async throws {
        let s = store()
        var sent = 0
        s.scheduleVoiceSend("Deploy", to: "a1", in: 0.2) { sent += 1 }
        s.cancelVoiceSend()
        try await Task.sleep(for: .seconds(0.4))
        XCTAssertEqual(sent, 0)
        XCTAssertNil(s.voiceAutoSend)
    }

    func testTakingItOverKeepsOneSend() async throws {
        let s = store()
        var first = 0, second = 0
        s.scheduleVoiceSend("Deploy", to: "a1", in: 0.2) { first += 1 }
        // A composer coming back replaces the send with its own, for the time left.
        s.scheduleVoiceSend("Deploy", to: "a1", in: 0.2) { second += 1 }
        try await Task.sleep(for: .seconds(0.5))
        XCTAssertEqual([first, second], [0, 1])
    }

    func testAnotherChatsSendGoesFirstInsteadOfBeingDropped() async throws {
        let s = store()
        var first = 0, second = 0
        s.scheduleVoiceSend("Deploy", to: "a1", in: 0.2) { first += 1 }
        // Yes in chat A, leave, yes in chat B within A's window: A's goes now.
        s.scheduleVoiceSend("Rebuild", to: "a2", in: 0.2) { second += 1 }
        XCTAssertEqual(first, 1)
        XCTAssertEqual(s.voiceAutoSend?.agentId, "a2")
        try await Task.sleep(for: .seconds(0.5))
        XCTAssertEqual([first, second], [1, 1])
    }

    func testSwitchingOrForgettingTheServerCancelsIt() async throws {
        for next in [nil, MockAPI() as LoftAPI?] {
            let s = store()
            var sent = 0
            s.scheduleVoiceSend("Deploy", to: "a1", in: 0.2) { sent += 1 }
            s.configure(next)
            XCTAssertNil(s.voiceAutoSend)
            try await Task.sleep(for: .seconds(0.4))
            XCTAssertEqual(sent, 0, "nothing goes to another server")
        }
    }
}
