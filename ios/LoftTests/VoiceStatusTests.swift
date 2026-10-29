import XCTest
@testable import Loft

/// Voice style E's status screen after a dictated message is sent: Sending → Thinking → Speaking → Replied.
final class VoiceStatusTests: XCTestCase {
    func testAMessageGoesSentThinkingSpeakingReplied() {
        var s: VoiceStatus? = .sending
        s = VoiceStatus.reduce(s!, .posted)
        XCTAssertEqual(s, .thinking)
        s = VoiceStatus.reduce(s!, .replyArrived(readAloud: true))
        XCTAssertEqual(s, .speaking)
        s = VoiceStatus.reduce(s!, .readThrough)
        XCTAssertEqual(s, .replied, "the reply stays on screen once read")
    }

    func testWithReadingOffTheReplyShowsAtOnce() {
        XCTAssertEqual(VoiceStatus.reduce(.thinking, .replyArrived(readAloud: false)), .replied)
    }

    func testStopAndReplay() {
        XCTAssertEqual(VoiceStatus.reduce(.speaking, .stop), .replied, "Stop keeps the reply, it doesn't close")
        XCTAssertEqual(VoiceStatus.reduce(.replied, .replay), .speaking)
        XCTAssertEqual(VoiceStatus.reduce(.thinking, .replay), .thinking, "nothing to replay yet")
    }

    func testUndoOnlyWhileSending() {
        XCTAssertNil(VoiceStatus.reduce(.sending, .undo), "Undo closes the status and the text goes back to the box")
        XCTAssertEqual(VoiceStatus.reduce(.thinking, .undo), .thinking, "too late once the agent has it")
    }

    func testAFailedSendCanBeRetried() {
        XCTAssertEqual(VoiceStatus.reduce(.sending, .postFailed), .failed)
        XCTAssertEqual(VoiceStatus.reduce(.failed, .retry), .sending)
        XCTAssertEqual(VoiceStatus.failed.title(agent: "Lead"), "Not sent")
    }

    func testTheScreenFollowsTheSession() {
        XCTAssertEqual(VoiceStatus.of(.review, undoPending: true, posting: false), .sending)
        XCTAssertNil(VoiceStatus.of(.review, undoPending: false, posting: false), "a review without Undo isn't a status")
        XCTAssertEqual(VoiceStatus.of(.sending, undoPending: false, posting: true), .sending)
        XCTAssertEqual(VoiceStatus.of(.sending, undoPending: false, posting: false), .thinking)
        XCTAssertEqual(VoiceStatus.of(.speaking, undoPending: false, posting: false), .speaking)
        XCTAssertEqual(VoiceStatus.of(.replied, undoPending: false, posting: false), .replied)
        XCTAssertEqual(VoiceStatus.of(.failed("x"), undoPending: false, posting: false), .failed)
        XCTAssertNil(VoiceStatus.of(.listening, undoPending: false, posting: false))
        XCTAssertEqual(VoiceStatus.thinking.title(agent: "Orchestrator"), "Orchestrator is thinking…")
        XCTAssertEqual(VoiceStatus.speaking.step, 2)
    }
}
