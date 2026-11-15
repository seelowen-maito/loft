import XCTest

/// Launching Loft for Mac on fixture data (`-ui-testing`) and the helpers both Mac UI suites share.
class MacUITestCase: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
    }

    override func tearDown() {
        app.terminate()
    }

    func launch(_ args: [String]) {
        // No Needs you notifications: their permission prompt takes the focus mid-test.
        app.launchArguments = args + ["-no-alerts"]
        app.launch()
        app.activate()
        // macOS can restore Loft with its window closed, leaving only the menu bar item: Go › Agents (⌘2) opens it.
        if !app.windows.firstMatch.waitForExistence(timeout: 5) { app.typeKey("2", modifierFlags: .command) }
    }

    func shot(_ name: String) {
        let a = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }

    func card(_ id: String) -> XCUIElement { app.descendants(matching: .any)[id].firstMatch }

    func sidebarRow(_ title: String) -> XCUIElement {
        app.outlines.cells.containing(.any, identifier: "section-\(title)").firstMatch
    }
}

/// The Mac's UI smoke suite: only the core flows that need a real window, focus or the system. Everything else is a
/// unit test (ios/README.md: "a UI test only for what a unit test can't reach"). These take over the Mac's screen:
/// run them (`ios/scripts/test.sh --mac-ui`) only when nobody is using it.
final class LoftMacUITests: MacUITestCase {
    /// #484: the demo address opens the bundled demo with its banner, and Exit goes back to the address screen.
    func testDemoAddressOpensTheDemoAndExitLeavesIt() {
        launch(["-ui-onboarding"])
        let field = app.textFields["serverAddressField"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        field.click()
        field.typeKey("a", modifierFlags: .command)
        field.typeText("demo-loft-dev.get-loft.com\r")
        XCTAssertTrue(card("demoBanner").waitForExistence(timeout: 10))
        XCTAssertTrue(app.descendants(matching: .any)["section-Agents"].exists, "the main window, on sample data")
        shot("Demo")
        app.buttons["exitDemo"].click()
        XCTAssertTrue(field.waitForExistence(timeout: 10), "back on the address screen")
        XCTAssertFalse(card("demoBanner").exists)
    }

    /// #487: Shift-Return adds a line without sending; Return sends the message with its line breaks.
    func testShiftReturnAddsLineAndReturnSends() {
        launch(["-ui-testing"])
        let list = app.scrollViews["agentsList"]
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        list.buttons["agentRow-Orchestrator"].click()
        let field = app.textFields["composerField"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.click()
        field.typeText("first line")
        field.typeKey(.return, modifierFlags: .shift)
        field.typeText("second line")
        XCTAssertEqual(field.value as? String, "first line\nsecond line", "Shift-Return adds a line and doesn't send")
        let sent = app.descendants(matching: .any).matching(identifier: "userMessage")
            .matching(NSPredicate(format: "label CONTAINS %@", "first line")).firstMatch
        XCTAssertFalse(sent.exists)
        shot("Multi-line before sending")

        field.typeText("\r")
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        XCTAssertTrue((sent.label).contains("first line\nsecond line"), sent.label)
        XCTAssertEqual(field.value as? String, "")
        shot("Multi-line sent")
    }

    /// #468: a click on each sidebar item shows its section, ⌘1–⌘5 go to each, and the arrow keys move between them.
    func testSidebarClicksOpenEachSection() {
        launch(["-ui-testing", "-no-ai"])
        XCTAssertTrue(app.scrollViews["agentsList"].waitForExistence(timeout: 10))
        let screens = [
            ("Overview", "overviewStats"), ("Issues", "boardColumn-issues"), ("Needs you", "needsYou-question:1"),
            ("Settings", "messagingRulesToggle"), ("Agents", "agentsList"),
        ]
        for (title, screen) in screens {
            card("section-\(title)").click()
            XCTAssertTrue(card(screen).waitForExistence(timeout: 5), "a click on \(title) shows it")
            XCTAssertTrue(sidebarRow(title).isSelected, "\(title) is highlighted")
        }
        shot("Sidebar Agents")
        for (key, (title, screen)) in zip(["1", "3", "4", "5", "2"], screens) {
            app.typeKey(key, modifierFlags: .command)
            XCTAssertTrue(card(screen).waitForExistence(timeout: 5), "⌘\(key) shows \(title)")
            XCTAssertTrue(sidebarRow(title).isSelected, "\(title) is highlighted")
        }
        card("section-Agents").click()
        app.typeKey(.downArrow, modifierFlags: [])
        XCTAssertTrue(card("boardColumn-issues").waitForExistence(timeout: 5), "↓ from Agents goes to Issues")
        shot("Sidebar Issues")
    }

    /// Quick search: the panel opens over the main window, typing finds an agent, and Return opens its chat there.
    /// A synthesized ⌥Space goes to the app, not through the global hot key, so `-show-quick-search` opens it at
    /// launch as the hot key does.
    func testQuickSearchOpensAnAgentsChat() {
        // In the app, whatever Settings › Quick search was left at on this Mac.
        launch(["-ui-testing", "-no-ai", "-show-quick-search", "-quickSearchOpensIn", "app"])
        let field = app.textFields["quickSearchField"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "the shortcut opens the panel")
        field.typeText("qa")
        let row = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "QA Agent")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 5), "the agent is found")
        shot("Quick search")
        field.typeText("\r")
        XCTAssertTrue(field.waitForNonExistence(timeout: 5), "the panel closes")
        let composer = app.textFields["composerField"]
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        XCTAssertEqual(composer.placeholderValue, "Message QA Agent", "the agent's chat opens")
    }
}
