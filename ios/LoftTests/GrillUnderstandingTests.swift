import XCTest
@testable import Loft

/// The phrase matcher decides first; the model (faked here) only reads what the matcher can't tell.
final class GrillUnderstandingTests: XCTestCase {
    private func read(_ text: String, model: String?, calls: Calls = Calls()) async -> GrillMatcher.Match? {
        await GrillUnderstanding.read(text, question: "Should the Pro plan include priority support?", phrases: nil) { _, _ in
            await calls.add()
            return model
        }
    }

    func testASureMatchNeverAsksTheModel() async {
        let calls = Calls()
        let m = await read("skip", model: "answer", calls: calls)
        XCTAssertEqual(m, GrillMatcher.Match(intent: .command(.skip), confidence: .sure))
        let n = await calls.count
        XCTAssertEqual(n, 0)
    }

    func testTheModelCanOnlyAsk() async {
        let m = await read("let's move on to the next one", model: "skip")
        XCTAssertEqual(m, GrillMatcher.Match(intent: .command(.skip), confidence: .ask), "a question, never a sure command")
        let r = await read("can you say that once more", model: "repeat")
        XCTAssertEqual(r?.intent, .repeatQuestion)
    }

    func testTheModelDismissesAQuestionOnlyInLongerReplies() async {
        // "next" first in a sentence makes the matcher ask; the model sees it's an answer.
        let long = await read("next quarter we can add it", model: "answer")
        XCTAssertNil(long)
        // In a short reply the matcher's phrase is the point: its question stays.
        let short = await read("tanto faz pra mim", model: "answer")
        XCTAssertEqual(short?.intent, .command(.decide))
        XCTAssertEqual(short?.confidence, .ask)
    }

    func testWithoutTheModelItsTheMatcher() async {
        let m = await read("next quarter we can add it", model: nil)
        XCTAssertEqual(m, GrillMatcher.match("next quarter we can add it"))
        let none = await GrillUnderstanding.read("let's move on", question: "", phrases: nil, model: nil)
        XCTAssertEqual(none, GrillMatcher.match("let's move on"))
    }

    func testLongAnswersDontWaitForTheModel() async {
        let calls = Calls()
        let text = "priority support with a four hour response time for teams above ten seats on the yearly plan"
        _ = await read(text, model: "skip", calls: calls)
        let n = await calls.count
        XCTAssertEqual(n, 0)
    }
}

private actor Calls {
    private(set) var count = 0
    func add() { count += 1 }
}
