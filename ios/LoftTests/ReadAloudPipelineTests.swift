import XCTest
@testable import Loft
@testable import LoftKit

/// The read-aloud entry point: a reply is read once it's whole, and one reading never cuts off another by accident.
@MainActor
final class ReadAloudPipelineTests: XCTestCase {
    /// A speech engine that records what it read and plays until `finish()` (or `stop()`).
    final class FakeEngine {
        private(set) var spoken: [String] = []
        private(set) var stopped: [String] = []
        private var playing: (text: String, done: CheckedContinuation<Void, Never>)?
        var isSpeaking: Bool { playing != nil }

        func play(_ text: String) async -> String {
            spoken.append(text)
            await withCheckedContinuation { playing = (text, $0) }
            return text
        }

        func finish() { playing?.done.resume(); playing = nil }

        func stop() {
            guard let p = playing else { return }
            stopped.append(p.text)
            finish()
        }
    }

    /// A reply source that delivers a message over time: each step is (seconds after the start, the text so far).
    /// `nil` text: the run is still going, so the reply can't be whole yet.
    private func settled(_ steps: [(TimeInterval, String?)], until end: TimeInterval) -> (text: String, at: TimeInterval)? {
        var settle = ReplySettle()
        let start = Date(timeIntervalSince1970: 0)
        var t: TimeInterval = 0
        while t <= end {
            let current = steps.last { $0.0 <= t }?.1
            if let whole = settle.whole(current, at: start.addingTimeInterval(t)) { return (whole, t) }
            t += 0.25
        }
        return nil
    }

    func testTheSettleDelayIsAboutHalfASecondToASecond() {
        XCTAssertGreaterThanOrEqual(Speaker.settle, 0.5)
        XCTAssertLessThanOrEqual(Speaker.settle, 1)
        XCTAssertEqual(ReplySettle().delay, Speaker.settle)
    }

    func testAReplyInPiecesIsReadOnceWholeAfterTheSettleDelay() throws {
        let r = try XCTUnwrap(settled([(0, nil), (1, "First part."), (1.5, "First part.\n\nSecond part."),
                                       (2, "First part.\n\nSecond part.\n\nThird.")], until: 10))
        XCTAssertEqual(r.text, "First part.\n\nSecond part.\n\nThird.", "the whole reply, from the start")
        XCTAssertGreaterThanOrEqual(r.at, 2 + Speaker.settle, "not before the last piece has settled")
        XCTAssertLessThan(r.at, 2 + Speaker.settle + 0.5)
    }

    func testAPauseShorterThanTheSettleDelayDoesNotSplitTheReply() throws {
        let gap = Speaker.settle - 0.25
        let r = try XCTUnwrap(settled([(0, "One."), (gap, "One.\n\nTwo.")], until: 10))
        XCTAssertEqual(r.text, "One.\n\nTwo.")
    }

    func testAReplyEditedAfterItStartsIsReadAsEdited() throws {
        let r = try XCTUnwrap(settled([(0, "Hel"), (0.5, "Hello, the build"), (1, "Hello, the build passed.")], until: 10))
        XCTAssertEqual(r.text, "Hello, the build passed.")
        XCTAssertGreaterThanOrEqual(r.at, 1 + Speaker.settle)
    }

    func testAReplyWhileTheRunIsStillGoingIsNotRead() {
        // The run starts again before the reply settles: it waits for that run too.
        XCTAssertNil(settled([(0, "Looking."), (0.25, nil)], until: 10))
    }

    func testASecondReadingWaitsForTheFirstInsteadOfCuttingItOff() async {
        let engine = FakeEngine()
        let turns = ReadingTurns()
        let reply = Task { await turns.run { await engine.play("the reply") } }
        await waitUntil { engine.isSpeaking }
        let question = Task { await turns.run { await engine.play("the next question") } }
        try? await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(engine.spoken, ["the reply"], "the second waits its turn")
        engine.finish()
        await waitUntil { engine.spoken.count == 2 }
        engine.finish()
        let first = await reply.value, second = await question.value
        XCTAssertEqual(first, "the reply")
        XCTAssertEqual(second, "the next question")
        XCTAssertEqual(engine.stopped, [], "nothing was cut off")
    }

    func testStopDropsTheReadingsWaitingTheirTurn() async {
        let engine = FakeEngine()
        let turns = ReadingTurns()
        let reply = Task { await turns.run { await engine.play("the reply") } }
        await waitUntil { engine.isSpeaking }
        let ack = Task { await turns.run { await engine.play("on it") } }
        try? await Task.sleep(for: .milliseconds(50))
        // Stop (or a caller replacing the reading): the one playing stops, the waiting one never plays.
        turns.cancelAll()
        engine.stop()
        let first = await reply.value, dropped = await ack.value
        XCTAssertEqual(first, "the reply")
        XCTAssertNil(dropped)
        XCTAssertEqual(engine.spoken, ["the reply"])
        let next = await Task { await turns.run { "after stop" } }.value
        XCTAssertEqual(next, "after stop", "a new reading still plays after a stop")
    }

    private func waitUntil(_ condition: () -> Bool) async {
        for _ in 0..<200 where !condition() { try? await Task.sleep(for: .milliseconds(5)) }
        XCTAssertTrue(condition())
    }
}
