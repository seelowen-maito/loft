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
        // No Needs you notifications (their permission prompt takes the focus mid-test), and no restored windows.
        app.launchArguments = args + ["-no-alerts", "-ApplePersistenceIgnoreState", "YES"]
        app.launch()
        app.activate()
        // macOS can restore Loft with its window closed, leaving only the menu bar item: Go › Agents (⌘2) opens it.
        if !app.windows.firstMatch.waitForExistence(timeout: 5) { app.typeKey("2", modifierFlags: .command) }
        // The split view saves a collapsed sidebar in the app's defaults, kept from one test run to the next, so
        // show it here rather than clear those defaults.
        let show = app.toolbars.buttons["Show Sidebar"]
        if show.exists { show.click() }
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

    /// #668: a queued message shows Queued · Cancel; Cancel turns it into Cancelled · Copy to box, and Copy to box puts
    /// it back in the composer.
    func testCancelAQueuedMessageThenCopyItToTheBox() {
        launch(["-ui-testing", "-no-ai", "-fixture-queued"])
        let list = app.scrollViews["agentsList"]
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        list.buttons["agentRow-Orchestrator"].click()
        let cancel = app.buttons["cancelQueued"]
        XCTAssertTrue(cancel.waitForExistence(timeout: 5), "the queued message offers Cancel")
        let copies = app.buttons.matching(identifier: "copyToBox")
        XCTAssertEqual(copies.count, 1, "only the already cancelled message offers Copy to box")
        shot("Queued")
        cancel.click()
        XCTAssertTrue(cancel.waitForNonExistence(timeout: 5), "Cancel goes once it's cancelled")
        XCTAssertEqual(copies.count, 2, "the cancelled message offers Copy to box")
        shot("Cancelled")
        copies.element(boundBy: 1).click()
        let field = app.textFields["composerField"]
        XCTAssertEqual(field.value as? String, "Also add a dark mode toggle to the settings screen.")
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
    /// The test's own shortcut, ⌃⌥⌘K, leaves ⌥Space to the Loft open on this Mac.
    func testQuickSearchOpensAnAgentsChat() {
        launch(["-ui-testing", "-no-ai", "-quickSearchShortcut", "control-option-command-40", "-quickSearchOpensIn", "app"])
        app.typeKey("k", modifierFlags: [.control, .option, .command])
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

    /// #659: "Start focus" opens the length popover with the last length highlighted; ↓ moves, Esc closes. It never
    /// starts a session, which would hide the apps on this Mac.
    func testStartFocusOpensTheLengthPopover() {
        launch(["-ui-testing", "-no-ai", "-focusMinutes", "50"])
        let start = card("startFocus")
        XCTAssertTrue(start.waitForExistence(timeout: 10))
        start.click()
        XCTAssertTrue(card("length-25").waitForExistence(timeout: 5), "the popover opens")
        XCTAssertTrue(card("length-90").exists)
        XCTAssertTrue(card("customLength").exists)
        XCTAssertTrue(card("length-50").isSelected, "the last length is highlighted")
        shot("Focus length popover")
        app.typeKey(.downArrow, modifierFlags: [])
        XCTAssertTrue(card("length-90").isSelected, "↓ moves the highlight")
        app.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(card("length-25").waitForNonExistence(timeout: 5), "Esc closes it")
        XCTAssertFalse(card("focusCountdown").exists, "no session started")
        // Focus for… (⌥⇧⌘F) with the sidebar collapsed shows the sidebar and the same popover.
        app.toolbars.buttons["Hide Sidebar"].click()
        XCTAssertTrue(start.waitForNonExistence(timeout: 5))
        app.typeKey("f", modifierFlags: [.command, .shift, .option])
        XCTAssertTrue(card("length-25").waitForExistence(timeout: 5), "Focus for… opens the popover")
        XCTAssertTrue(start.exists, "under the sidebar, shown again")
        app.typeKey(.escape, modifierFlags: [])
    }

    /// #659: a custom last length starts in the Custom field. Hovering a preset doesn't move the highlight (so Return
    /// can't start it), and ↑ leaves the field.
    func testTheCustomFieldKeepsTheKeyboardUntilUp() {
        launch(["-ui-testing", "-no-ai", "-focusMinutes", "37"])
        let start = card("startFocus")
        XCTAssertTrue(start.waitForExistence(timeout: 10))
        start.click()
        let field = app.textFields["customLength"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, "37")
        XCTAssertTrue(card("length-25").exists)
        card("length-25").hover()
        XCTAssertFalse(card("length-25").isSelected, "hover doesn't take the highlight")
        XCTAssertTrue(field.hasKeyboardFocus, "the field keeps the keyboard")
        app.typeKey(.upArrow, modifierFlags: [])
        XCTAssertTrue(card("length-90").isSelected, "↑ leaves Custom for 90")
        XCTAssertFalse(field.hasKeyboardFocus)
        app.typeKey(.escape, modifierFlags: [])
        XCTAssertTrue(card("length-25").waitForNonExistence(timeout: 5))
    }
}

extension XCUIElement {
    var hasKeyboardFocus: Bool { (value(forKey: "hasKeyboardFocus") as? Bool) ?? false }
}
