import XCTest
@testable import Loft

/// Voice style D's turn-taking: silence ends your turn, the reply is read, talking over it is your turn again.
final class VoiceLoopTests: XCTestCase {
    func testAFullTurnGoesListeningThinkingSpeakingAndBack() {
        var s = VoiceLoop.State.listening
        s = VoiceLoop.reduce(s, .quiet(2, hasText: true, endsOpen: false))
        XCTAssertEqual(s, .thinking, "a pause after words ends the turn")
        s = VoiceLoop.reduce(s, .replyArrived)
        XCTAssertEqual(s, .speaking)
        s = VoiceLoop.reduce(s, .replyRead)
        XCTAssertEqual(s, .listening, "after the reply it listens again")
    }

    func testSilenceEndsTheTurnOnlyAfterWordsAndLongEnough() {
        XCTAssertEqual(VoiceLoop.reduce(.listening, .quiet(1.0, hasText: true, endsOpen: false)), .listening, "a breath isn't the end")
        XCTAssertEqual(VoiceLoop.reduce(.listening, .quiet(VoiceLoop.endOfTurn, hasText: true, endsOpen: false)), .thinking)
        XCTAssertEqual(VoiceLoop.reduce(.listening, .quiet(30, hasText: false, endsOpen: false)), .listening, "nothing said: nothing sent")
    }

    func testASentenceLeftOpenWaitsLonger() {
        XCTAssertEqual(VoiceLoop.reduce(.listening, .quiet(2, hasText: true, endsOpen: true)), .listening, "“…and” — more is coming")
        XCTAssertEqual(VoiceLoop.reduce(.listening, .quiet(VoiceLoop.endOfTurnOpen, hasText: true, endsOpen: true)), .thinking)
        XCTAssertEqual(VoiceLoop.endProgress(quiet: 0.8, endsOpen: false), 0.5, accuracy: 0.01)
        XCTAssertEqual(VoiceLoop.endProgress(quiet: 9, endsOpen: false), 1)
    }

    func testTalkingOverTheReplyIsYourTurn() {
        XCTAssertEqual(VoiceLoop.reduce(.speaking, .userSpoke(words: 3)), .listening)
        XCTAssertEqual(VoiceLoop.reduce(.speaking, .userSpoke(words: 1)), .speaking, "one stray word doesn't cut in")
        XCTAssertEqual(VoiceLoop.reduce(.thinking, .userSpoke(words: 5)), .thinking, "nothing to cut in on while it thinks")
    }

    func testEndStopsFromAnywhere() {
        for s in [VoiceLoop.State.listening, .thinking, .speaking] {
            XCTAssertEqual(VoiceLoop.reduce(s, .end), .ended)
        }
    }

    func testTheIndicatorFollowsTheSession() {
        XCTAssertEqual(VoiceLoop.state(for: .listening), .listening)
        XCTAssertEqual(VoiceLoop.state(for: .sending), .thinking)
        XCTAssertEqual(VoiceLoop.state(for: .speaking), .speaking)
        XCTAssertEqual(VoiceLoop.state(for: .idle), .ended)
    }
}
