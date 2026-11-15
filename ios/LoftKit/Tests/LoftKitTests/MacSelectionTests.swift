import XCTest
@testable import LoftKit

/// The Mac window's selection: the sidebar's row follows however the section changed, and a chat opened from code
/// (not a sidebar click) takes the cursor.
final class MacSelectionTests: XCTestCase {
    func testSectionFollowsGoAndOpenChat() {
        var s = MacSelection()
        s.go(.needsYou)
        XCTAssertEqual(s.section, .needsYou)
        s.go(.issues)
        XCTAssertEqual(s.section, .issues)
        s.openChat()
        XCTAssertEqual(s.section, .agents)
    }

    func testCommandNumbersMatchSidebarOrder() {
        XCTAssertEqual(MacSection.allCases.map(\.rawValue), [1, 2, 3, 4, 5])
        XCTAssertEqual(MacSection(rawValue: 3), .issues)
        XCTAssertEqual(MacSection.allCases.map(\.title), ["Overview", "Agents", "Issues", "Needs you", "Settings"])
    }

    func testLinksLandInTheirSection() {
        XCTAssertEqual(MacSection(.needsYou), .needsYou)
        XCTAssertEqual(MacSection(.needsYouItem("x")), .needsYou)
        XCTAssertEqual(MacSection(.agents), .agents)
        XCTAssertEqual(MacSection(.talk), .agents)
        XCTAssertEqual(MacSection(.agent("a")), .agents)
        XCTAssertEqual(MacSection(.overview), .overview)
        XCTAssertEqual(MacSection(.issue("1")), .issues)
    }

    func testOnlyOpenedChatsRequestTheComposer() {
        var s = MacSelection()
        XCTAssertEqual(s.composerRequests, 0)
        s.go(.agents) // a sidebar click or ⌘2
        XCTAssertEqual(s.composerRequests, 0)
        s.openChat()
        s.openChat() // the same chat again still asks
        XCTAssertEqual(s.composerRequests, 2)
    }

    func testARequestIsAnsweredOnce() {
        var s = MacSelection()
        XCTAssertFalse(s.takeComposerRequest())
        s.openChat()
        XCTAssertTrue(s.takeComposerRequest())
        XCTAssertFalse(s.takeComposerRequest()) // a later sidebar click shows a chat with no request
        s.go(.agents)
        XCTAssertFalse(s.takeComposerRequest())
        s.openChat()
        XCTAssertTrue(s.takeComposerRequest())
    }

    func testARequestForAChatThatNeverShowedIsDropped() {
        var s = MacSelection()
        s.openChat()
        s.go(.issues) // the chat never mounted
        s.go(.agents) // a later sidebar click
        XCTAssertFalse(s.takeComposerRequest())
    }

    func testOpeningAChatClearsTheNote() {
        var s = MacSelection()
        s.chatNote = (agentId: "a", text: "Herdr isn't linked")
        s.go(.issues)
        XCTAssertNotNil(s.chatNote) // a section change keeps it
        s.openChat()
        XCTAssertNil(s.chatNote)
    }

    func testComposerKeyboardRule() {
        // Opened from code: the cursor goes to the composer even if the sidebar had the keyboard.
        XCTAssertTrue(MacSelection.composerTakesKeyboard(requested: true, sidebarHasKeyboard: true))
        // A sidebar click alone: the sidebar keeps it.
        XCTAssertFalse(MacSelection.composerTakesKeyboard(requested: false, sidebarHasKeyboard: true))
        XCTAssertTrue(MacSelection.composerTakesKeyboard(requested: false, sidebarHasKeyboard: false))
    }
}
