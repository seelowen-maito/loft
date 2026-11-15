import XCTest
@testable import LoftKit

/// Enter, ⌘↵ and ⌥↵ on an agent in quick search, in both "Open agents in" settings.
final class AgentOpeningTests: XCTestCase {
    private func open(ready: Bool = true, herdrFirst: Bool, browser: Bool = false, command: Bool = false, option: Bool = false) -> AgentOpening {
        AgentOpening.choose(herdrReady: ready, herdrFirst: herdrFirst, browserDefault: browser, command: command, option: option)
    }

    func testHerdrFirst() {
        XCTAssertEqual(open(herdrFirst: true), .herdr)
        XCTAssertEqual(open(herdrFirst: true, command: true), .chat)
        XCTAssertEqual(open(herdrFirst: true, option: true), .browser, "⌥↵ is no longer ignored")
        XCTAssertEqual(open(herdrFirst: true, browser: true, option: true), .chat, "the other of browser and app")
    }

    func testChatFirstWithHerdrReady() {
        XCTAssertEqual(open(herdrFirst: false), .chat)
        XCTAssertEqual(open(herdrFirst: false, browser: true), .browser)
        XCTAssertEqual(open(herdrFirst: false, command: true), .herdr)
        XCTAssertEqual(open(herdrFirst: false, option: true), .browser)
    }

    func testHerdrNotLinkedIsNeverOffered() {
        XCTAssertEqual(open(ready: false, herdrFirst: true), .chat)
        XCTAssertEqual(open(ready: false, herdrFirst: true, command: true), .chat)
        XCTAssertEqual(open(ready: false, herdrFirst: true, option: true), .browser)
    }

    func testCommandWinsOverOption() {
        XCTAssertEqual(open(herdrFirst: true, command: true, option: true), .chat)
        XCTAssertEqual(open(herdrFirst: false, command: true, option: true), .herdr)
    }
}
