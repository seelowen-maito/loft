import XCTest
@testable import Loft

final class RunTimerTests: XCTestCase {
    func testSecondsThenMinutesThenHours() {
        XCTAssertEqual(RunTimer.elapsed(0), "0s")
        XCTAssertEqual(RunTimer.elapsed(45.9), "45s")
        XCTAssertEqual(RunTimer.elapsed(60), "1m 0s")
        XCTAssertEqual(RunTimer.elapsed(143), "2m 23s")
        XCTAssertEqual(RunTimer.elapsed(3849), "1h 4m 9s")
        XCTAssertEqual(RunTimer.elapsed(-5), "0s", "a start in the future (clock skew)")
    }

    func testTheLiveTimerCountsFromTheServersRunStart() {
        let start = Date(timeIntervalSince1970: 1_800_000_000)
        XCTAssertEqual(RunTimer.elapsed(since: start, now: start.addingTimeInterval(143.4)), "2m 23s")
        XCTAssertEqual(RunTimer.elapsed(since: nil, now: .now), "0s")
    }

    func testAFinishedRunReadsDoneInDurationDayTime() {
        let end = Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 29, hour: 16, minute: 23))! // a Thursday
        XCTAssertEqual(RunTimer.done(143, endedAt: end, locale: Locale(identifier: "en_GB")), "Done in 2m 23s · Thu 16:23")
        let de = Locale(identifier: "de_DE")
        let day = end.formatted(.dateTime.weekday(.abbreviated).locale(de))
        XCTAssertNotEqual(day, "Thu", "the day follows the locale")
        XCTAssertTrue(RunTimer.done(5, endedAt: end, locale: de).hasPrefix("Done in 5s · \(day) "))
    }

    private func msg(_ kind: String, _ run: String?, _ at: TimeInterval, durationMs: Double? = nil) -> ChatMessage {
        ChatMessage(id: Int64(at), agentId: "a", runId: run, role: "system", kind: kind, content: "",
                    meta: durationMs.map { ["duration_ms": .number($0)] }, createdAt: Date(timeIntervalSince1970: 1_800_000_000 + at))
    }

    func testMeasuredFromRunStartToRunEndLikeTheLiveTimer() {
        let run = [msg("text", nil, 0), msg("run_start", "r1", 0), msg("text", "r1", 60), msg("run_end", "r1", 143, durationMs: 141_000)]
        XCTAssertEqual(RunTimer.lastFinished(run)?.duration, 143)
        XCTAssertEqual(RunTimer.lastFinished(run)?.endedAt, Date(timeIntervalSince1970: 1_800_000_143))
    }

    func testStoppedRunCountsFromRunStart() {
        XCTAssertEqual(RunTimer.lastFinished([msg("run_start", "r1", 0), msg("run_end", "r1", 30)])?.duration, 30)
    }

    func testDurationMsWhenTheRunStartIsNotLoaded() {
        XCTAssertEqual(RunTimer.lastFinished([msg("run_end", "r1", 143, durationMs: 5_000)])?.duration, 5)
    }

    func testNoLineAfterAClear() {
        XCTAssertNil(RunTimer.lastFinished([msg("run_start", "r1", 0), msg("run_end", "r1", 30, durationMs: 30_000), msg("clear", nil, 60)]))
    }

    func testNoDoneInAfterARunThatFailedWithoutARunEnd() {
        let run = [msg("run_start", "r1", 0), msg("run_end", "r1", 13, durationMs: 13_000),
                   msg("text", nil, 60), msg("run_start", "r2", 61), msg("error", "r2", 62)]
        XCTAssertNil(RunTimer.lastFinished(run), "not the previous run's line")
    }

    func testNoRunsNoLine() {
        XCTAssertNil(RunTimer.lastFinished([]))
        XCTAssertNil(RunTimer.lastFinished([msg("text", nil, 0)]))
    }
}
