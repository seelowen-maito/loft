import XCTest
@testable import Loft

final class RunWatchTests: XCTestCase {
    private let now = Date()
    private func agent(running: Bool = true, waiting: [String]? = nil, since: Double? = nil, output: Double? = nil) -> Agent {
        Agent(id: "a", name: "A", status: running ? "running" : "idle", running: running,
              lastOutputAt: output.map { now.addingTimeInterval(-$0 * 60) },
              waitingOn: waiting, waitingSince: since.map { now.addingTimeInterval(-$0 * 60) })
    }

    func testWaitingOnBackgroundWork() {
        let w = RunWatch.of(agent(waiting: ["Run the QA suite", "Build docs"], since: 14, output: 14), newestMessage: nil, now: now)
        XCTAssertEqual(w, .waiting(first: "Run the QA suite", more: 1, minutes: 14))
        XCTAssertEqual(w?.text, "Waiting on a background task: Run the QA suite (+1 more) · 14 min")
        XCTAssertEqual(w?.shortText, "Waiting\u{00A0}· 14\u{00A0}min")
        XCTAssertEqual(RunWatch.silent(minutes: 23).shortText, "Quiet\u{00A0}· 23\u{00A0}min")
        XCTAssertEqual(RunWatch.of(agent(waiting: ["Run the QA suite"], since: 2), newestMessage: nil, now: now)?.text,
                       "Waiting on a background task: Run the QA suite · 2 min", "shown at once, not only after 10 min")
    }

    func testSilentAfterTenMinutes() {
        XCTAssertEqual(RunWatch.of(agent(output: 23), newestMessage: nil, now: now), .silent(minutes: 23))
        XCTAssertNil(RunWatch.of(agent(output: 9), newestMessage: nil, now: now))
        // A newer chat message counts as activity even if lastOutputAt lags.
        XCTAssertNil(RunWatch.of(agent(output: 30), newestMessage: now.addingTimeInterval(-60), now: now))
        XCTAssertEqual(RunWatch.of(agent(output: 30), newestMessage: now.addingTimeInterval(-12 * 60), now: now), .silent(minutes: 12))
    }

    func testNothingWhenIdleOrUnknown() {
        XCTAssertNil(RunWatch.of(agent(running: false, waiting: ["x"], since: 30, output: 30), newestMessage: nil, now: now))
        XCTAssertNil(RunWatch.of(agent(waiting: []), newestMessage: nil, now: now), "older server or nothing known: no bar")
    }
}
