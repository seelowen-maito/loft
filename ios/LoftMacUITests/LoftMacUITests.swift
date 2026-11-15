import XCTest

/// Loft for Mac on fixture data (`-ui-testing`): the agents list, sending with @ and #, Overview, Needs you
/// and Settings with Clean.
final class LoftMacUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
    }

    override func tearDown() {
        app.terminate()
    }

    private func launch(_ args: [String]) {
        app.launchArguments = args
        app.launch()
        app.activate()
    }

    private func shot(_ name: String) {
        let a = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }

    /// First launch: the address screen says so when nothing answers there, and keeps the address unsaved.
    func testWrongAddressSaysLoftIsNotReachable() {
        launch(["-ui-onboarding"])
        let field = app.textFields["serverAddressField"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        XCTAssertEqual(field.value as? String, "http://localhost:7420")
        field.click()
        field.typeKey("a", modifierFlags: .command)
        field.typeText("http://127.0.0.1:9\r")
        XCTAssertTrue(app.staticTexts["connectError"].waitForExistence(timeout: 20))
        shot("Can't reach Loft")
        XCTAssertTrue(field.exists, "Still on the address screen")
    }

    /// Leads first, then groups; an agent's settings save.
    func testAgentsLeadsFirstAndSettingsSave() {
        launch(["-ui-testing", "-fixture-lead"])
        let list = app.outlines["agentsList"]
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        let leads = list.staticTexts["Leads"], loft = list.staticTexts["Loft"]
        XCTAssertTrue(leads.waitForExistence(timeout: 10))
        XCTAssertTrue(loft.exists)
        XCTAssertLessThan(leads.frame.minY, loft.frame.minY, "Leads come before the groups")

        list.cells.containing(.any, identifier: "agentRow-Backend Agent").firstMatch.click()
        app.buttons["agentSettingsButton"].click()
        let name = app.textFields["settingsName"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.click()
        name.typeKey("a", modifierFlags: .command)
        name.typeText("API Agent")
        app.buttons["settingsSave"].click()
        XCTAssertTrue(list.descendants(matching: .any)["agentRow-API Agent"].waitForExistence(timeout: 5))
        shot("Agents and settings")
    }

    /// @ and # pick from lists as on the web, and the message is sent with both tags.
    func testSendWithMentionAndTag() {
        launch(["-ui-testing"])
        let list = app.outlines["agentsList"]
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        list.cells.containing(.any, identifier: "agentRow-Orchestrator").firstMatch.click()
        // Another agent's handover is incoming, not one of the user's own messages.
        let handover = app.descendants(matching: .any).matching(identifier: "agentMessage")
            .matching(NSPredicate(format: "label BEGINSWITH %@", "From Backend Agent")).firstMatch
        XCTAssertTrue(handover.waitForExistence(timeout: 10))

        let field = app.textFields["composerField"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.click()
        field.typeText("@Bac")
        let mention = app.buttons["mention-Backend Agent"]
        XCTAssertTrue(mention.waitForExistence(timeout: 5))
        shot("@ picker")
        mention.click()
        XCTAssertEqual(field.value as? String, "@Backend Agent ")

        field.typeText("please check #")
        XCTAssertTrue(app.otherElements["suggestions"].waitForExistence(timeout: 5) || app.scrollViews["suggestions"].exists)
        shot("# picker")
        // Return picks the first suggestion while the list shows.
        field.typeText("\r")
        let withTag = field.value as? String ?? ""
        XCTAssertTrue(withTag.hasPrefix("@Backend Agent please check #"), withTag)
        XCTAssertNotEqual(withTag, "@Backend Agent please check #")

        field.typeText("\r")
        let sent = app.descendants(matching: .any).matching(identifier: "userMessage").matching(NSPredicate(format: "label CONTAINS %@", "@Backend Agent please check #")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10))
        XCTAssertEqual(field.value as? String, "")
        shot("Sent with @ and #")
    }

    /// Settings › Apple Intelligence on the fixture model status: unavailable says why and disables the switches;
    /// available enables them.
    func testAppleIntelligenceSettingsFollowTheModelStatus() {
        launch(["-ui-testing", "-fixture-ai-status", "off"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("5", modifierFlags: .command)
        let status = app.staticTexts["onDeviceModelStatus"]
        XCTAssertTrue(status.waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any)["onDeviceModelStatus"].exists)
        XCTAssertFalse(app.checkBoxes["aiRouteToggle"].isEnabled)
        shot("Apple Intelligence off")
        app.terminate()
        launch(["-ui-testing", "-fixture-ai-status", "available"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("5", modifierFlags: .command)
        XCTAssertTrue(app.checkBoxes["aiRouteToggle"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.checkBoxes["aiRouteToggle"].isEnabled)
        XCTAssertTrue(app.checkBoxes["aiChatRepliesToggle"].isEnabled)
        shot("Apple Intelligence available")
    }

    /// Clicking the chat's mic opens Talk with that agent in To:; Cancel returns to the chat.
    func testMicOpensTalkWithTheAgent() {
        launch(["-ui-testing", "-no-speech"])
        let list = app.outlines["agentsList"]
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        list.cells.containing(.any, identifier: "agentRow-Orchestrator").firstMatch.click()
        let mic = app.descendants(matching: .any)["micButton"]
        XCTAssertTrue(mic.waitForExistence(timeout: 10))
        mic.click()
        XCTAssertTrue(app.buttons["talkCancel"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["talkTo-Orchestrator"].exists, "the agent is in To:")
        shot("Talk")
        app.buttons["talkCancel"].click()
        XCTAssertTrue(app.textFields["composerField"].waitForExistence(timeout: 5))
    }

    /// ⌘N opens New message, and picking an agent opens its chat.
    func testNewMessageShortcut() {
        launch(["-ui-testing"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("5", modifierFlags: .command)
        XCTAssertTrue(app.textFields["serverAddressField"].waitForExistence(timeout: 5), "⌘5 opens Settings")
        app.typeKey("n", modifierFlags: .command)
        let search = app.textFields["recipientSearch"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.typeText("QA\r")
        XCTAssertTrue(app.textFields["composerField"].waitForExistence(timeout: 5))
        XCTAssertEqual(app.textFields["composerField"].placeholderValue, "Message QA Agent")
    }

    /// ⌘1: the web's home summary, and a card per agent.
    func testOverviewShowsSummary() {
        launch(["-ui-testing"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("1", modifierFlags: .command)
        let stats = app.descendants(matching: .any)["overviewStats"]
        XCTAssertTrue(stats.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["REPOSITORIES"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any)["agentCard-Backend Agent"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "Loft for Mac")).firstMatch
            .waitForExistence(timeout: 10), "Open pull requests list the fixture's PR")
        shot("Overview")
    }

    /// Overview has the web's Activity and Most active panels and the GitHub token note; a repo opens its detail with
    /// languages, contributors and recent commits.
    func testOverviewActivityAndRepoDetail() {
        launch(["-ui-testing"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("1", modifierFlags: .command)
        XCTAssertTrue(app.descendants(matching: .any)["overviewActivity"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["MOST ACTIVE · 30D"].exists)
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "value BEGINSWITH %@", "peak ")).firstMatch.exists)
        shot("Overview activity")
        let loft = app.descendants(matching: .any)["mostActive-loft"]
        XCTAssertTrue(loft.waitForExistence(timeout: 5))
        loft.click()
        XCTAssertTrue(app.staticTexts["Your local developer command center."].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Merge main into the spec branch"].waitForExistence(timeout: 5), "Recent commits load")
        XCTAssertTrue(app.staticTexts["Loft mobile dev"].exists, "Contributors")
        XCTAssertTrue(app.staticTexts["Swift"].exists, "Languages")
        shot("Repo detail")
        app.typeKey(.escape, modifierFlags: [])
        // "site" can't be read on GitHub and the fixture has a saved token, so no token note.
        XCTAssertFalse(app.descendants(matching: .any)["githubLimited"].exists)
    }

    /// Needs you behind another app: the window is inactive and the accent buttons still read (the screenshot shows it).
    func testNeedsYouInactiveWindow() {
        launch(["-ui-testing", "-no-ai"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("4", modifierFlags: .command)
        XCTAssertTrue(app.descendants(matching: .any)["needsYou-question:1"].waitForExistence(timeout: 10))
        XCUIApplication(bundleIdentifier: "com.apple.finder").activate()
        sleep(1)
        XCTAssertTrue(app.buttons["Allow once"].firstMatch.exists)
        shot("Needs you inactive")
    }

    /// ⌘4: a question's Reply opens a sheet; Send answers it and the item goes.
    func testNeedsYouReply() {
        // The on-device model's summaries would move the row under the click; off, as on a Mac without it.
        launch(["-ui-testing", "-no-ai"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("4", modifierFlags: .command)
        let row = app.descendants(matching: .any)["needsYou-question:1"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Run the suite"].exists)
        shot("Needs you")
        row.buttons["needsYouAnswer"].click()
        let field = app.textViews["needsYouAnswerField"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.click()
        field.typeText("B2, please")
        shot("Needs you reply")
        app.buttons["needsYouAnswerSend"].click()
        XCTAssertTrue(app.descendants(matching: .any)["needsYouNotice"].waitForExistence(timeout: 5))
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: row)
        waitForExpectations(timeout: 10)
    }

    /// Settings › Disk › Check space: tick one item, Delete selected, and the log says what was freed.
    func testCleanRun() {
        launch(["-ui-testing"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("5", modifierFlags: .command)
        let check = app.buttons["checkSpace"]
        XCTAssertTrue(check.waitForExistence(timeout: 10))
        XCTAssertTrue(check.isEnabled)
        check.click()
        XCTAssertTrue(app.descendants(matching: .any)["left-a2"].waitForExistence(timeout: 10), "Loft Reviewer's leftover processes show")
        let tick = app.checkBoxes["tick-test-run.log"]
        XCTAssertTrue(tick.waitForExistence(timeout: 10))
        XCTAssertFalse(app.checkBoxes["tick-loft"].firstMatch.isEnabled, "A main clone that isn't checked can't be ticked")
        tick.click()
        XCTAssertEqual(app.staticTexts["cleanSummary"].value as? String ?? app.staticTexts["cleanSummary"].label, "1 selected · 12 MB")
        shot("Clean")
        app.buttons["cleanConfirm"].click()
        let summary = app.staticTexts["cleanLogSummary"]
        XCTAssertTrue(summary.waitForExistence(timeout: 10))
        let text = summary.value as? String ?? summary.label
        XCTAssertTrue(text.hasPrefix("Freed 12 MB · 1 removed"), text)
        shot("Clean log")
        app.buttons["cleanDone"].click()
        XCTAssertTrue(app.descendants(matching: .any)["lastClean"].waitForExistence(timeout: 10), "Disk shows the run")
    }

    /// Settings: a change shows Save, and saving keeps it.
    func testSettingsSave() {
        launch(["-ui-testing"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("5", modifierFlags: .command)
        let toggle = app.descendants(matching: .any)["messagingRulesToggle"].firstMatch
        XCTAssertTrue(toggle.waitForExistence(timeout: 10))
        XCTAssertEqual(toggle.value as? Int, 1)
        toggle.click()
        let save = app.buttons["settingsSaveAll"]
        XCTAssertTrue(save.waitForExistence(timeout: 5))
        save.click()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: save)
        waitForExpectations(timeout: 10)
        app.typeKey("1", modifierFlags: .command)
        app.typeKey("5", modifierFlags: .command)
        XCTAssertTrue(toggle.waitForExistence(timeout: 10))
        XCTAssertEqual(toggle.value as? Int, 0, "Saved off, and loaded again off")
        shot("Settings")
    }

    /// Settings has the web's Herdr UI and Push notifications: Set up links Herdr here, a phone gets a test push and
    /// can be removed.
    func testSettingsHerdrAndPush() {
        launch(["-ui-testing"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("5", modifierFlags: .command)
        let open = app.buttons["herdrOpen"]
        XCTAssertTrue(open.waitForExistence(timeout: 10))
        open.click()
        let setUp = app.buttons["herdrSetUp"]
        XCTAssertTrue(setUp.waitForExistence(timeout: 5))
        setUp.click()
        XCTAssertTrue(app.staticTexts["herdrResult"].waitForExistence(timeout: 10))
        XCTAssertTrue((app.staticTexts["herdrHere"].value as? String ?? "").contains("shows Loft's agents"))
        XCTAssertEqual(app.buttons["herdrSetUp"].label, "Set up again")
        shot("Settings Herdr UI")

        let test = app.buttons["pushTest-iPhone"]
        XCTAssertTrue(test.waitForExistence(timeout: 10))
        test.click()
        let msg = app.staticTexts["pushMessage"]
        XCTAssertTrue(msg.waitForExistence(timeout: 5))
        XCTAssertTrue((msg.value as? String ?? msg.label).hasPrefix("Sent to iPhone"))
        XCTAssertEqual(app.staticTexts["pushPath"].value as? String, "Using the Loft relay")
        shot("Settings Push notifications")
        app.buttons["Remove this phone"].click()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: test)
        waitForExpectations(timeout: 10)
    }

    /// The Issues board: the Issues column first, All / Issues / Specs, a spec's page with its tickets, and Start
    /// delivery on a spec that waits for one.
    func testIssuesBoardFilterOpensASpecAndStartsDelivery() {
        launch(["-ui-testing", "-fixture-specs"])
        XCTAssertTrue(app.outlines["agentsList"].waitForExistence(timeout: 10))
        app.typeKey("3", modifierFlags: .command)
        let issues = app.descendants(matching: .any)["boardColumn-issues"]
        let planned = app.descendants(matching: .any)["boardColumn-planned"]
        XCTAssertTrue(issues.waitForExistence(timeout: 10))
        XCTAssertTrue(planned.exists)
        XCTAssertLessThan(issues.frame.minX, planned.frame.minX, "the Issues column comes first")
        XCTAssertTrue(card("issueCard-Renew the domain").exists, "an issue that is no spec's ticket")
        XCTAssertFalse(card("issueCard-Implement backend endpoints").exists, "a spec's ticket stays with its spec")
        XCTAssertTrue(card("specCard-Offline mode").exists)
        shot("Issues board")

        let type = app.radioGroups["boardType"]
        type.radioButtons["Specs"].click()
        XCTAssertTrue(issues.waitForNonExistence(timeout: 5), "Specs hides the Issues column")
        XCTAssertTrue(card("specCard-Implement settings functionality").exists)
        type.radioButtons["Issues"].click()
        XCTAssertTrue(planned.waitForNonExistence(timeout: 5), "Issues hides the spec columns")
        XCTAssertTrue(card("issueCard-Renew the domain").exists)
        type.radioButtons["All"].click()

        // A spec in Delivery: its state, who has which ticket, its progress and its tickets.
        card("specCard-Implement settings functionality").click()
        XCTAssertTrue(card("deliveryState").waitForExistence(timeout: 10))
        XCTAssertEqual(card("deliveryProgress").label, "1 of 5 tickets done")
        for t in ["Backend endpoints", "iOS screen", "Docs", "QA"] { XCTAssertTrue(card("ticketRow-\(t)").exists, t) }
        XCTAssertFalse(card("delivery-start").exists, "it started")
        shot("Spec page")
        app.typeKey("[", modifierFlags: .command)

        // A spec that waits: Start delivery starts it.
        XCTAssertTrue(card("specCard-Offline mode").waitForExistence(timeout: 5))
        card("specCard-Offline mode").click()
        let start = app.buttons["delivery-start"]
        XCTAssertTrue(start.waitForExistence(timeout: 10))
        start.click()
        XCTAssertTrue(card("deliveryRunning").waitForExistence(timeout: 10), "the card says who has the first ticket")
        XCTAssertFalse(start.exists)
        shot("Delivery started")
    }

    private func card(_ id: String) -> XCUIElement { app.descendants(matching: .any)[id].firstMatch }
}
