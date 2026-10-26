import XCTest
@testable import Loft

final class ChatRepliesTests: XCTestCase {
    func testOnlyAMessageThatEndsOnAQuestionGetsReplies() {
        XCTAssertTrue(ChatReplies.asksSomething("Tests are green. Should I merge it now or wait for QA?"))
        XCTAssertTrue(ChatReplies.asksSomething("Here's what I found.\n\nWant me to open a PR?"))
        XCTAssertTrue(ChatReplies.asksSomething("Faço o merge agora?"))
        XCTAssertFalse(ChatReplies.asksSomething("Why did it fail? A missing key. I added it and the build is green."), "it answered itself")
        XCTAssertFalse(ChatReplies.asksSomething("Should I merge?\n\nNever mind, I merged it."))
        XCTAssertFalse(ChatReplies.asksSomething("```swift\nlet x = y ?? z\n```"), "a ? in code isn't a question")
        XCTAssertFalse(ChatReplies.asksSomething(""))
    }

    func testABareYesOrNoIsDroppedForAnEitherOrQuestion() {
        let q = "Should I merge it now or wait for QA?"
        XCTAssertEqual(ChatReplies.shown(["Yes", "No", "Later"], for: q), [], "only Later is left: one isn't a choice")
        XCTAssertEqual(ChatReplies.shown(["Merge now", "Wait for QA"], for: q), ["Merge now", "Wait for QA"])
        XCTAssertEqual(ChatReplies.shown(["Yes", "No"], for: "Should I retry the failed run?"), ["Yes", "No"])
        XCTAssertEqual(ChatReplies.shown(["Sim", "Não", "Depois"], for: "Faço o merge agora ou espero?"), [])
    }
}
