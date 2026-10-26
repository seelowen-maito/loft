import XCTest
@testable import Loft

final class ReadAloudCommandTests: XCTestCase {
    func testCommandsOnTheirOwn() {
        XCTAssertEqual(ReadAloudCommand.match("Stop"), .stop)
        XCTAssertEqual(ReadAloudCommand.match("ok, stop please"), .stop)
        XCTAssertEqual(ReadAloudCommand.match("Para"), .stop)
        XCTAssertEqual(ReadAloudCommand.match("chega"), .stop)
        XCTAssertEqual(ReadAloudCommand.match("wait"), .stop)
        XCTAssertEqual(ReadAloudCommand.match("Skip"), .skip)
        XCTAssertEqual(ReadAloudCommand.match("next"), .skip)
        XCTAssertEqual(ReadAloudCommand.match("Pula"), .skip)
        XCTAssertEqual(ReadAloudCommand.match("Próxima"), .skip)
        XCTAssertEqual(ReadAloudCommand.match("repeat"), .repeatIt)
        XCTAssertEqual(ReadAloudCommand.match("de novo"), .repeatIt)
        XCTAssertEqual(ReadAloudCommand.match("more"), .more)
        XCTAssertEqual(ReadAloudCommand.match("Conta mais"), .more)
        XCTAssertEqual(ReadAloudCommand.match("continue"), .more, "continue reads on, it doesn't resume an interview")
    }

    func testAnythingElseIsIgnored() {
        XCTAssertNil(ReadAloudCommand.match(""))
        XCTAssertNil(ReadAloudCommand.match("stop the deploy on staging"))
        XCTAssertNil(ReadAloudCommand.match("the next one is better"))
        XCTAssertNil(ReadAloudCommand.match("I don't know"), "deciding is an answer, not a read-aloud command")
        XCTAssertNil(ReadAloudCommand.match("skop"), "a near miss asks in the grill; while reading it's ignored")
    }

    private func words(_ spec: [(String, TimeInterval)]) -> [ReadAloudCommand.Word] {
        spec.map { ReadAloudCommand.Word(text: $0.0, at: $0.1, duration: 0.3) }
    }

    func testLatestLooksAfterTheLastPause() {
        // Someone talking, a pause, then "stop".
        let w = words([("so", 0), ("the", 0.3), ("build", 0.6), ("is", 0.9), ("green", 1.2), ("stop", 3.0)])
        XCTAssertEqual(ReadAloudCommand.latest(in: w), .stop)
        // "Skip" running straight on from other words isn't on its own.
        let run = words([("please", 0), ("skip", 0.3), ("the", 0.6), ("tests", 0.9)])
        XCTAssertNil(ReadAloudCommand.latest(in: run))
        XCTAssertNil(ReadAloudCommand.latest(in: words([("we", 0), ("should", 0.3), ("stop", 0.6)])))
        XCTAssertEqual(ReadAloudCommand.latest(in: words([("ok", 0), ("next", 0.35)])), .skip)
        XCTAssertNil(ReadAloudCommand.latest(in: []))
    }
}
