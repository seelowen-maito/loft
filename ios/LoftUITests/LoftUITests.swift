import XCTest

/// The iPhone's UI smoke suite: only the core flows that need a real app, against the in-app fixture server
/// (`-ui-testing`), so no live agents run. Everything else is a unit test (ios/README.md: "a UI test only for what a
/// unit test can't reach"). Each test is one launch with no animations, microphone, speech recognizer or on-device
/// model (`-no-animations -no-speech -no-ai`), and short waits: the fixture answers at once.
final class LoftUITests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-no-animations", "-no-speech", "-no-ai", "-AppleLanguages", "(en)"] + extra
        app.launch()
        return app
    }

    private func any(_ id: String, _ app: XCUIApplication) -> XCUIElement { app.descendants(matching: .any)[id].firstMatch }

    private func text(containing s: String, _ app: XCUIApplication) -> XCUIElement {
        app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", s)).firstMatch
    }

    /// Taps an agent's row in the Agents list, first scrolling it clear of the floating tab bar: on smaller
    /// phones the last rows sit under it, and a tap there lands on the tab bar.
    private func openAgent(_ name: String, _ app: XCUIApplication) {
        let row = app.buttons["agentRow-\(name)"].firstMatch
        XCTAssertTrue(row.appears(timeout: 5))
        let bar = app.tabBars.firstMatch
        for _ in 0..<4 where row.frame.maxY > bar.frame.minY - 8 {
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6))
            from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: -200)))
        }
        row.tap()
    }

    /// Launch and connect: the connect screen (with no saved address to prefill it: an empty `serverURL` in the
    /// arguments domain hides whatever an earlier run saved) takes the demo address, opens the app on sample data
    /// with its banner, and Exit goes back.
    func testLaunchAndConnectToTheDemo() {
        let app = XCUIApplication()
        app.launchArguments = ["-ui-onboarding", "-no-animations", "-no-speech", "-no-ai", "-serverURL", ""]
        app.launch()
        let field = app.textFields["serverAddressField"]
        XCTAssertTrue(field.appears(timeout: 5))
        XCTAssertFalse(app.buttons["testConnection"].isEnabled, "nothing to test without an address")
        field.tap()
        // Right after a launch the first tap can miss the focus.
        if !app.keyboards.firstMatch.appears(timeout: 2) { field.tap() }
        field.typeText("demo-loft-dev.get-loft.com")
        XCTAssertTrue(app.buttons["testConnection"].isEnabled)
        app.buttons["saveServer"].tap()
        XCTAssertTrue(any("demoBanner", app).appears(timeout: 5))
        XCTAssertTrue(app.tabBars.buttons["Overview"].exists, "the app, on sample data")
        app.buttons["exitDemo"].tap()
        XCTAssertTrue(field.appears(timeout: 5), "back to the connect screen")
        XCTAssertFalse(any("demoBanner", app).exists)
    }

    /// Each tab opens its screen.
    func testTabsOpenTheirScreens() {
        let app = launch()
        XCTAssertTrue(app.staticTexts["Connected"].appears(timeout: 5), "Overview first")
        app.tabBars.buttons["Agents"].tap()
        XCTAssertTrue(app.buttons["agentRow-QA Agent"].firstMatch.appears(timeout: 5))
        app.tabBars.buttons["Issues"].tap()
        XCTAssertTrue(app.segmentedControls["issuesViewSwitch"].appears(timeout: 5))
        app.tabBars.buttons["Chat"].tap()
        XCTAssertTrue(app.buttons["orchestratorConversation"].appears(timeout: 5))
        app.tabBars.buttons["Overview"].tap()
        XCTAssertTrue(app.staticTexts["Connected"].appears(timeout: 5))
    }

    /// Open an agent's chat from its screen and send a message: it shows in the chat and leaves the box.
    func testOpenAnAgentsChatAndSend() {
        let app = launch()
        app.tabBars.buttons["Agents"].tap()
        openAgent("QA Agent", app)
        app.buttons["chatWithAgent"].tap()
        let field = app.textFields["composerField"]
        XCTAssertTrue(field.appears(timeout: 5))
        field.tap()
        field.typeText("Run the smoke tests")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(app.staticTexts["Run the smoke tests"].appears(timeout: 5), "sent")
        XCTAssertFalse(app.buttons["retrySend"].exists)
        XCTAssertNotEqual(field.value as? String, "Run the smoke tests", "the box is empty again")
    }

    /// Deny a permission in the agent's chat and approve another on Overview: each goes once it's answered.
    func testApproveAndDenyAPermission() {
        let app = launch(["-fixture-risky-permission"])
        app.tabBars.buttons["Agents"].tap()
        openAgent("Backend Agent", app)
        app.buttons["chatWithAgent"].tap()
        let cards = app.otherElements.matching(identifier: "approvalCard")
        let risky = cards.containing(.any, identifier: "permissionRisks").firstMatch
        XCTAssertTrue(risky.appears(timeout: 5), "the push to main waits")
        risky.buttons["Deny"].tap()
        XCTAssertTrue(risky.disappears(timeout: 5), "the denied request goes")

        // The fixture's other request, on Overview's card.
        app.tabBars.buttons["Overview"].tap()
        let allow = app.buttons["Allow once"].firstMatch
        XCTAssertTrue(allow.appears(timeout: 5), "the other request still waits")
        allow.tap()
        XCTAssertTrue(allow.disappears(timeout: 5), "and goes once approved")
    }

    private func wait(_ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(5)
        while !done(), Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        return done()
    }
}
