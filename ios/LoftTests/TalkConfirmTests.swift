import XCTest
@testable import Loft

/// Talk's message box step: it asks after a pause (never sends by itself), a touch stops it, "send it" sends.
final class TalkConfirmTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1000)

    func testItAsksAfterAPauseAndNeverSendsByItself() {
        var s = TalkConfirm.State.counting(since: t0)
        s = TalkConfirm.reduce(s, .tick(t0.addingTimeInterval(2)))
        XCTAssertEqual(s, .counting(since: t0), "not yet")
        s = TalkConfirm.reduce(s, .tick(t0.addingTimeInterval(TalkConfirm.askAfter)))
        XCTAssertEqual(s, .asking)
        s = TalkConfirm.reduce(s, .tick(t0.addingTimeInterval(600)))
        XCTAssertEqual(s, .asking, "it asks and keeps asking: nothing goes without Send or “send it”")
        XCTAssertEqual(TalkConfirm.progress(since: t0, now: t0.addingTimeInterval(1)), 0.25, accuracy: 0.001)
    }

    func testWithTheSettingOffItDoesntAsk() {
        XCTAssertEqual(TalkConfirm.reduce(.counting(since: t0), .tick(t0.addingTimeInterval(60)), askAfterPause: false), .counting(since: t0))
    }

    func testATouchCancelsTheAsk() {
        XCTAssertEqual(TalkConfirm.reduce(.counting(since: t0), .touched), .editing)
        XCTAssertEqual(TalkConfirm.reduce(.asking, .touched), .editing)
        XCTAssertEqual(TalkConfirm.reduce(.editing, .heard(.send)), .editing, "once you're editing, nothing listens")
        XCTAssertEqual(TalkConfirm.reduce(.editing, .tick(t0.addingTimeInterval(60))), .editing)
    }

    func testSendEditAndCancel() {
        XCTAssertEqual(TalkConfirm.reduce(.counting(since: t0), .heard(.send)), .sending)
        XCTAssertEqual(TalkConfirm.reduce(.asking, .sendTapped), .sending)
        XCTAssertEqual(TalkConfirm.reduce(.asking, .heard(.edit)), .editing)
        XCTAssertEqual(TalkConfirm.reduce(.asking, .heard(.cancel)), .cancelled)
    }

    func testPhrases() {
        let agents = ["Loft Dev", "Orchestrator"]
        for p in ["send it", "Send.", "yes", "OK, send it", "send it please", "send it to Loft Dev", "Send it to loft dev.", "send to Orchestrator"] {
            XCTAssertEqual(TalkConfirm.command(in: p, agents: agents), .send, p)
        }
        XCTAssertEqual(TalkConfirm.command(in: "edit", agents: agents), .edit)
        XCTAssertEqual(TalkConfirm.command(in: "Cancel that", agents: agents), .cancel)
        for p in ["sender", "send it to the moon", "send it to Loft", "please send the invoice", "sending", ""] {
            XCTAssertNil(TalkConfirm.command(in: p, agents: agents), p)
        }
    }

    func testListeningOnlyWhileItWaitsAndWithTheSetting() {
        XCTAssertTrue(TalkConfirm.listens(.counting(since: t0), sayToSend: true))
        XCTAssertTrue(TalkConfirm.listens(.asking, sayToSend: true))
        XCTAssertFalse(TalkConfirm.listens(.asking, sayToSend: false))
        XCTAssertFalse(TalkConfirm.listens(.editing, sayToSend: true))
        XCTAssertFalse(TalkConfirm.listens(.sending, sayToSend: true))
    }
}
