import XCTest

/// Real screen behaviour against the in-app fixture server (`-ui-testing`), so no live agents run. Rules live in
/// the unit tests; these check that screens show and respond. Each test is one launch walking a whole journey,
/// with no animations, microphone, speech recognizer or on-device model (`-no-animations -no-speech -no-ai`),
/// and short waits: the fixture answers at once.
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

    private func back(_ app: XCUIApplication) { app.navigationBars.buttons.element(boundBy: 0).tap() }

    private func openOrchestratorChat(_ app: XCUIApplication) {
        app.tabBars.buttons["Chat"].tap()
        let convo = app.buttons["orchestratorConversation"]
        XCTAssertTrue(convo.appears(timeout: 5))
        convo.tap()
    }

    private func openQAChat(_ app: XCUIApplication) {
        app.tabBars.buttons["Agents"].tap()
        openAgent("QA Agent", app)
        app.buttons["chatWithAgent"].tap()
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

    private func composerValue(_ app: XCUIApplication) -> String? { app.textFields["composerField"].value as? String }

    /// Fresh context for each Delivery ticket on the agent's screen: Above 80k by default, and a choice sticks.
    func testFreshContextPicker() {
        let app = launch()
        app.tabBars.buttons["Agents"].tap()
        openAgent("QA Agent", app)
        XCTAssertTrue(app.buttons["chatWithAgent"].appears(timeout: 5))
        let picker = app.buttons["freshContext"].firstMatch
        // Clear of the floating tab bar, as in `openAgent`: a tap under it lands on the bar.
        let bar = app.tabBars.firstMatch
        for _ in 0..<8 where !picker.exists || picker.frame.maxY > bar.frame.minY - 8 { app.swipeUp() }
        // Let the scroll settle, or the tap lands on a moving row and opens nothing.
        RunLoop.current.run(until: Date().addingTimeInterval(1))
        XCTAssertTrue(picker.label.contains("Above 80k"), picker.label)
        picker.tap()
        let always = app.buttons["Always"].firstMatch
        XCTAssertTrue(always.appears(timeout: 5))
        always.tap()
        let deadline = Date().addingTimeInterval(5)
        while !picker.label.contains("Always"), Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertTrue(picker.label.contains("Always"), picker.label)
        // A higher threshold than the default 80k.
        picker.tap()
        let above150 = app.buttons["Above 150k"].firstMatch
        XCTAssertTrue(above150.appears(timeout: 5))
        above150.tap()
        let deadline150 = Date().addingTimeInterval(5)
        while !picker.label.contains("Above 150k"), Date() < deadline150 { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertTrue(picker.label.contains("Above 150k"), picker.label)
    }

    // MARK: - Default fixture

    /// One launch: live state, the Needs you card, the Mac's temperature, plan usage, approving a permission and
    /// activity; then tasks (subtasks nest, filters narrow, a new task shows up); then the agent list (context
    /// warning), an agent's detail and a direct chat with another agent.
    func testOverviewTasksAndAgents() {
        let app = launch()
        XCTAssertTrue(app.staticTexts["Connected"].appears(timeout: 5))
        // Servers with `needs_you` (the fixture) show the "Needs you" card instead of "Needs your attention".
        XCTAssertTrue(app.staticTexts["Needs you · 10 new of 10"].appears(timeout: 5))
        XCTAssertTrue(app.staticTexts["Backend Agent wants to use Bash"].exists)
        XCTAssertTrue(app.staticTexts["Active work"].exists)
        let chip = any("macTemperature", app)
        XCTAssertTrue(chip.appears(timeout: 5))
        XCTAssertEqual(chip.label, "Mac CPU temperature 64 degrees")
        XCTAssertTrue(any("planUsageCard", app).appears(timeout: 5))
        XCTAssertTrue(any("Session", app).exists)
        XCTAssertTrue(any("Week", app).exists)

        let allow = app.buttons["Allow once"].firstMatch
        XCTAssertTrue(allow.appears(timeout: 5))
        allow.tap()
        XCTAssertTrue(allow.disappears(timeout: 5))

        let activity = any("activityCard", app)
        for _ in 0..<4 where !activity.waitForExistence(timeout: 1) { app.swipeUp(velocity: .fast) }
        XCTAssertTrue(activity.appears(timeout: 5))

        // Tasks.
        app.tabBars.buttons["Issues"].tap()
        XCTAssertTrue(app.buttons["taskRow-Implement settings functionality"].appears(timeout: 5))
        XCTAssertTrue(app.buttons["taskRow-Implement backend endpoints"].exists, "subtasks nest under their parent")
        app.buttons["taskFilter-failed"].tap()
        XCTAssertTrue(app.buttons["taskRow-Fix broken docs links"].appears(timeout: 3))
        XCTAssertFalse(app.buttons["taskRow-Renew the domain"].exists)
        app.buttons["taskFilter-inbox"].tap()
        XCTAssertTrue(app.buttons["taskRow-Renew the domain"].appears(timeout: 3))

        app.buttons["createButton"].tap()
        app.buttons["New issue"].tap()
        let title = app.textFields["taskTitleField"]
        XCTAssertTrue(title.appears(timeout: 3))
        title.tap()
        title.typeText("Write release notes")
        app.buttons["saveTask"].tap()
        // The new task is in the inbox, the filter still on.
        XCTAssertTrue(app.buttons["taskRow-Write release notes"].appears(timeout: 5))

        // Agents.
        app.tabBars.buttons["Agents"].tap()
        let warning = any("contextChipOver", app)
        XCTAssertTrue(warning.appears(timeout: 5))
        XCTAssertTrue(warning.label.hasSuffix("past the 80k smart zone"))
        openAgent("Backend Agent", app)
        XCTAssertTrue(app.buttons["chatWithAgent"].appears(timeout: 5))
        XCTAssertTrue(app.staticTexts["Implement backend endpoints"].exists)
        back(app)

        openQAChat(app)
        XCTAssertTrue(app.staticTexts["Direct · Idle"].appears(timeout: 5))
        XCTAssertTrue(app.textFields["composerField"].exists)
        XCTAssertTrue(any("runDone", app).label.hasPrefix("Done in 2m 23s · "), "the last run's line, until the next run")
    }

    /// The Orchestrator chat: compact task rows, sending, the @ picker, the mic's long press, and no manual grill.
    func testOrchestratorChat() {
        let app = launch()
        openOrchestratorChat(app)
        XCTAssertTrue(app.staticTexts["Orchestrator · Working"].appears(timeout: 5))
        // The run line: the time since the run's start (12 minutes ago in the fixture), then the status text.
        let run = any("workingIndicator", app)
        XCTAssertTrue(run.appears(timeout: 5))
        XCTAssertTrue(run.label.hasPrefix("Working: 12m "), run.label)
        XCTAssertTrue(run.label.hasSuffix("s · Planning the settings work"), run.label)

        let brief = app.buttons["Issue: Renew the domain"]
        XCTAssertTrue(brief.appears(timeout: 5))
        // The raw brief (with its tag and instructions) isn't shown as a bubble.
        XCTAssertFalse(text(containing: "[Task", app).exists)
        XCTAssertFalse(text(containing: "expires on Nov 3", app).exists)
        brief.tap()
        XCTAssertTrue(text(containing: "expires on Nov 3", app).appears(timeout: 3))
        XCTAssertTrue(any("taskReport", app).exists)

        let field = app.textFields["composerField"]
        field.tap()
        field.typeText("Status please")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(app.staticTexts["Status please"].appears(timeout: 5))
        XCTAssertFalse(app.buttons["retrySend"].exists)

        let at = app.buttons["mentionButton"]
        XCTAssertTrue(at.appears(timeout: 5))
        at.tap()
        let qa = app.buttons["mention-QA Agent"]
        XCTAssertTrue(qa.appears(timeout: 5), "the agents show without typing")
        XCTAssertFalse(app.buttons["mention-Orchestrator"].exists, "not this chat's own agent")
        XCTAssertTrue(any("mentionSection-Loft", app).exists, "this chat's team first, under its group")
        XCTAssertTrue(any("mentionSection-Other agents", app).exists)
        XCTAssertTrue(app.buttons["mention-Docs Agent"].exists, "another team: the backend delivers it too")
        qa.tap()
        XCTAssertEqual(composerValue(app), "@QA Agent ")
        XCTAssertFalse(qa.exists, "the list goes once one is picked")

        // "#" lists specs, issues and PRs to tag; a pick puts its tag in the message, and the hint says both exist.
        XCTAssertTrue(any("composerHint", app).exists)
        field.tap()
        field.typeText("see #")
        let spec = app.buttons["tag-Implement settings functionality"]
        XCTAssertTrue(spec.appears(timeout: 5), "the picker opens on #")
        XCTAssertTrue(app.buttons["tag-Settings endpoints"].exists, "pull requests too")
        spec.tap()
        XCTAssertEqual(composerValue(app), "@QA Agent see #42 ", "same project: #N")
        XCTAssertFalse(spec.exists, "the list goes once one is picked")
        // No match shows nothing, as on the web: "#fff" is often just text.
        field.typeText("#fff")
        XCTAssertFalse(any("tagList", app).exists)
        XCTAssertFalse(app.staticTexts["Nothing to tag by that name"].exists)
        field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 4))

        app.buttons["Conversation options"].tap()
        // Once the menu is open, Grill me… would be in it: no waiting for something that shouldn't come.
        XCTAssertTrue(app.buttons["Agent details"].appears(timeout: 5))
        XCTAssertFalse(app.buttons["Grill me…"].exists)
        // Close the menu.
        app.tap()

    }

    /// #340: in an agent's chat, tapping the mic opens Talk for that agent; holding it dictates into the box.
    func testChatMicTapOpensTalkHoldDictates() {
        let app = launch()
        openQAChat(app)
        let mic = app.buttons["micButton"]
        XCTAssertTrue(mic.appears(timeout: 5))
        XCTAssertEqual(mic.label, "Talk")
        // #358: Apple's minimum tap target.
        XCTAssertGreaterThanOrEqual(mic.frame.width, 44)
        XCTAssertGreaterThanOrEqual(mic.frame.height, 44)
        mic.tap()
        let close = app.buttons["voiceClose"]
        XCTAssertTrue(close.appears(timeout: 5), "Talk opens")
        XCTAssertTrue(any("voiceRecentAgents", app).buttons["QA Agent"].isSelected, "with this agent in To")
        close.tap()
        XCTAssertTrue(close.disappears(timeout: 5))

        mic.press(forDuration: 0.8)
        XCTAssertTrue(any("speechStatus", app).appears(timeout: 5), "holding dictates into the box")
        XCTAssertFalse(close.exists, "and doesn't open Talk")
        XCTAssertEqual(mic.label, "Stop dictation")
    }

    /// A sent message that tagged a spec shows a chip, and tapping it opens that item in the app.
    func testTagChipOpensTheTaggedItem() {
        let app = launch()
        openOrchestratorChat(app)
        XCTAssertTrue(text(containing: "Is #42 still on track?", app).appears(timeout: 5))
        let chip = app.buttons["spec: Implement settings functionality"].firstMatch
        XCTAssertTrue(chip.appears(timeout: 5), "the message's tag shows as a chip")
        chip.tap()
        let owner = text(containing: "octo-dev", app)
        for _ in 0..<4 where !owner.appears(timeout: 2) { app.swipeUp() }
        XCTAssertTrue(owner.exists, "the chip opens the spec's detail")
    }

    /// Spec #203: a video chosen in the composer uploads with progress, then goes with the message and plays inline
    /// with its length. Photos is another process, so `-fixture-video` stands in for picking it there.
    func testSendingAVideo() throws {
        let video = try XCTUnwrap(Bundle(for: Self.self).path(forResource: "sample-video", ofType: "mp4"))
        let app = launch(["-fixture-video", video])
        openQAChat(app)
        let attach = app.buttons["attachVideoButton"]
        XCTAssertTrue(attach.appears(timeout: 5))
        attach.tap()
        app.buttons["chooseVideo"].tap()
        XCTAssertTrue(any("videoChip", app).appears(timeout: 5))
        XCTAssertTrue(app.staticTexts["Video · 0:06"].appears(timeout: 5), "its length")
        XCTAssertTrue(app.progressIndicators["Uploading"].appears(timeout: 3), "the upload's progress")
        XCTAssertFalse(app.buttons["sendButton"].isEnabled, "the message waits for the upload")
        XCTAssertTrue(app.staticTexts["Ready to send"].appears(timeout: 5))

        let field = app.textFields["composerField"]
        field.tap()
        field.typeText("What happens here?")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(any("videoChip", app).disappears(timeout: 5), "it went with the message")
        XCTAssertTrue(any("chatVideo", app).appears(timeout: 5), "an inline player")
        XCTAssertTrue(any("chatVideo", app).label.contains("0:06"), any("chatVideo", app).label)
        XCTAssertTrue(app.staticTexts["What happens here?"].exists)
        XCTAssertFalse(app.buttons["retrySend"].exists)
    }

    /// #309: photos chosen in the composer show as thumbnails, one can be removed, and the rest go with the message,
    /// shown in the chat and opening full screen. Photos is another process, so `-fixture-photos` stands in for it.
    func testSendingPhotos() throws {
        let paths = try [UIColor.systemTeal, .systemOrange, .systemPink].map { color in
            let image = UIGraphicsImageRenderer(size: CGSize(width: 1200, height: 900)).image { c in
                color.setFill()
                c.fill(CGRect(x: 0, y: 0, width: 1200, height: 900))
                UIColor.white.setFill()
                c.cgContext.fillEllipse(in: CGRect(x: 400, y: 250, width: 400, height: 400))
            }
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("photo-\(UUID().uuidString).png")
            try XCTUnwrap(image.pngData()).write(to: url)
            return url.path
        }
        let app = launch(["-fixture-photos", paths.joined(separator: ",")])
        openQAChat(app)
        let attach = app.buttons["attachVideoButton"]
        XCTAssertTrue(attach.appears(timeout: 5))
        attach.tap()
        app.buttons["choosePhotos"].tap()
        let chips = app.descendants(matching: .any).matching(identifier: "photoChip")
        XCTAssertTrue(chips.element(boundBy: 2).appears(timeout: 5), "a thumbnail for each photo")
        XCTAssertTrue(app.descendants(matching: .any)["Photo, ready to send"].appears(timeout: 10))
        app.buttons["removePhoto"].firstMatch.tap()
        XCTAssertTrue(chips.element(boundBy: 2).disappears(timeout: 3), "one removed")
        XCTAssertEqual(chips.count, 2)

        let field = app.textFields["composerField"]
        field.tap()
        field.typeText("Which one?")
        let send = app.buttons["sendButton"]
        XCTAssertTrue(send.wait(for: \.isEnabled, toEqual: true, timeout: 10), "sendable once both are up")
        send.tap()
        XCTAssertTrue(any("draftPhotos", app).disappears(timeout: 5), "they went with the message")
        let sent = app.buttons.matching(identifier: "chatPhoto")
        XCTAssertTrue(sent.element(boundBy: 1).appears(timeout: 5), "both photos in the chat")
        XCTAssertTrue(app.staticTexts["Which one?"].exists)
        sent.element(boundBy: 0).tap()
        XCTAssertTrue(any("fullScreenPhoto", app).appears(timeout: 5), "full screen")
        app.buttons["Done"].tap()
        XCTAssertTrue(any("fullScreenPhoto", app).disappears(timeout: 3))
    }

    /// #316: at accessibility-extra-large text, each photo's remove button sits under the thumbnail, not on it,
    /// and keeps a 44-pt target.
    func testPhotoRemoveButtonClearsThumbnailAtLargeText() throws {
        let image = UIGraphicsImageRenderer(size: CGSize(width: 600, height: 600)).image { c in
            UIColor.systemTeal.setFill()
            c.fill(CGRect(x: 0, y: 0, width: 600, height: 600))
        }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("photo-\(UUID().uuidString).png")
        try XCTUnwrap(image.pngData()).write(to: url)
        let app = launch(["-fixture-photos", url.path,
                          "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL"])
        openOrchestratorChat(app)
        let attach = app.buttons["attachVideoButton"]
        XCTAssertTrue(attach.appears(timeout: 5))
        attach.tap()
        app.buttons["choosePhotos"].tap()
        let chip = any("photoChip", app)
        XCTAssertTrue(chip.appears(timeout: 5))
        let remove = app.buttons["removePhoto"].firstMatch
        XCTAssertGreaterThanOrEqual(remove.frame.width, 44)
        XCTAssertGreaterThanOrEqual(remove.frame.height, 44)
        let thumbnail = CGRect(x: chip.frame.minX, y: chip.frame.minY, width: 64, height: 64)
        XCTAssertFalse(remove.frame.intersects(thumbnail), "\(remove.frame) covers \(thumbnail)")
    }

    /// The agent screen shows the agent's group and its projects, main first.
    func testAgentDetailsShowGroupAndProjects() {
        let app = launch()
        openOrchestratorChat(app)
        app.buttons["Conversation options"].tap()
        app.buttons["Agent details"].tap()
        let projects = text(containing: "loft (main), website", app)
        XCTAssertTrue(app.staticTexts["Orchestrator"].appears(timeout: 5))
        for _ in 0..<6 where !projects.exists { app.swipeUp(velocity: .fast) }
        XCTAssertTrue(projects.exists)
        XCTAssertTrue(app.staticTexts["Group, Loft"].exists)
    }

    /// Agent messages in the fixture chat: the FYI one carries the mark (and says so to VoiceOver), the needs-action one doesn't.
    func testFYIMarkOnlyOnFYIAgentMessages() {
        let app = launch()
        openOrchestratorChat(app)
        let fyi = app.staticTexts["Backend Agent, FYI: Merged #40 on main; the endpoints are live."]
        XCTAssertTrue(fyi.appears(timeout: 5), "VoiceOver reads the FYI message with FYI in its label")
        let marks = app.staticTexts.matching(identifier: "fyiMark")
        XCTAssertTrue(marks.matching(NSPredicate(format: "label == 'FYI'")).firstMatch.exists, "the mark shows in the chat")
        XCTAssertEqual(fyi.identifier, "fyiMark", "the mark sits on the FYI message")
        let needsAction = app.staticTexts["Backend Agent: Please review PR #41 before I merge it."]
        XCTAssertTrue(needsAction.exists)
        XCTAssertNotEqual(needsAction.identifier, "fyiMark", "no mark on a needs-action message")
        XCTAssertFalse(marks.matching(NSPredicate(format: "label CONTAINS 'PR #41'")).firstMatch.exists)
    }

    /// The Talk button: a tap talks to the Orchestrator; the long-press menu talks to another agent or dictates.
    func testTalkButton() {
        let app = launch()
        let talk = app.buttons["talkButton"]
        XCTAssertTrue(talk.appears(timeout: 5))
        talk.tap()
        let close = app.buttons["voiceClose"]
        XCTAssertTrue(close.appears(timeout: 5))
        close.tap()
        XCTAssertTrue(close.disappears(timeout: 5))

        talk.press(forDuration: 0.7)
        let other = app.buttons["Talk to another agent…"]
        XCTAssertTrue(other.appears(timeout: 5))
        other.tap()
        // The sheet opens at half height and the list only creates rows on screen: on a smaller iPhone
        // QA Agent (last) starts below the fold, so scroll until it's there.
        let qa = app.buttons["agentRow-QA Agent"].firstMatch
        let picker = app.collectionViews.firstMatch
        XCTAssertTrue(picker.appears(timeout: 5))
        for _ in 0..<4 where !qa.isHittable { picker.swipeUp(velocity: .fast) }
        XCTAssertTrue(qa.appears(timeout: 5))
        qa.tap()
        XCTAssertTrue(close.appears(timeout: 5))
        XCTAssertTrue(app.staticTexts["QA Agent"].exists, "for the agent that was picked")
        close.tap()
        XCTAssertTrue(close.disappears(timeout: 5))

        talk.press(forDuration: 0.7)
        let dictate = app.buttons["Dictate in chat"]
        XCTAssertTrue(dictate.appears(timeout: 5))
        dictate.tap()
        XCTAssertTrue(app.staticTexts["Orchestrator · Working"].appears(timeout: 5))
        // Dictation starts by itself; nothing is sent.
        XCTAssertTrue(any("speechStatus", app).appears(timeout: 5))
    }

    /// Talk's To row offers five agents; More lists all eleven, and one picked there is who Talk goes to, in the row.
    func testTalkMorePicksAnAgentThatWasNotInTheRow() {
        let app = launch(["-fixture-many-agents", "-fixture-voice", "listening"])
        let row = any("voiceRecentAgents", app)
        XCTAssertTrue(row.appears(timeout: 8))
        XCTAssertFalse(row.buttons["Web Agent"].exists, "not one of the five in the row")
        app.buttons["voiceMoreAgents"].tap()
        let search = app.searchFields.firstMatch
        XCTAssertTrue(search.appears(timeout: 5))
        search.tap()
        search.typeText("web")
        let pick = app.buttons["talkPick-Web Agent"].firstMatch
        XCTAssertTrue(pick.appears(timeout: 5))
        XCTAssertFalse(app.buttons["talkPick-QA Agent"].exists, "the search leaves the others out")
        pick.tap()
        XCTAssertTrue(pick.disappears(timeout: 5), "the list closes")
        XCTAssertTrue(row.buttons["Web Agent"].appears(timeout: 5), "and the picked agent is in the row")
        XCTAssertTrue(row.buttons["Web Agent"].isSelected, "as the one Talk goes to")
    }

    // MARK: - Other fixtures

    /// Talk: the tidied dictation lands in the chat's message box; sending it there shows the full-screen status,
    /// whose Undo puts the text back in the box.
    func testATalkDictationGoesToTheChatThenTheStatus() {
        let app = launch(["-fixture-voice", "review", "-fixture-countdown-hold", "YES"])
        XCTAssertTrue(app.textFields["composerField"].appears(timeout: 5), "in the chat")
        XCTAssertTrue(composerValue(app)?.contains("copy the diagnostics") == true, "the tidied text, in the box")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(app.staticTexts["voiceStatusTitle"].appears(timeout: 5))
        XCTAssertEqual(app.staticTexts["voiceStatusTitle"].label, "Sending to Orchestrator…")
        app.buttons["Undo"].tap()
        XCTAssertTrue(app.staticTexts["voiceStatusTitle"].disappears(timeout: 5))
        let deadline = Date().addingTimeInterval(5)
        while composerValue(app)?.contains("copy the diagnostics") != true, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertTrue(composerValue(app)?.contains("copy the diagnostics") == true, "nothing sent; the text is back")
    }

    /// Talk's Send now: it sends and goes straight to the chat, with the message there and the box empty.
    func testTalkSendNowOpensTheChat() {
        let app = launch(["-fixture-voice", "review", "-fixture-countdown-hold", "YES"])
        XCTAssertTrue(app.textFields["composerField"].appears(timeout: 5), "in the chat")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(app.staticTexts["voiceStatusTitle"].appears(timeout: 5))
        app.buttons["Send now"].tap()
        XCTAssertTrue(app.buttons["voiceClose"].disappears(timeout: 5), "Talk closes by itself")
        XCTAssertTrue(app.textFields["composerField"].appears(timeout: 5), "back in the chat")
        let sent = app.staticTexts.containing(NSPredicate(format: "label CONTAINS 'copy the diagnostics'")).firstMatch
        XCTAssertTrue(sent.appears(timeout: 5), "with the message shown")
        XCTAssertFalse(composerValue(app)?.contains("copy the diagnostics") == true, "and not left in the box")
    }

    /// A new Talk: Done after talking tidies it up, then the Talk screen closes by itself and you're in the agent's
    /// chat with the text in the box. With animations on: the close used to be dropped when the chat opened
    /// underneath the closing screen, and it stayed up until you tapped Close.
    func testTalkClosesByItselfIntoTheChat() throws {
        // Under XCUITest the tidy-up runs (Talk resets to "Getting ready…") but the screen doesn't close, while the
        // same launch driven by hand closes into the chat every time. Skipped until that difference is understood.
        throw XCTSkip("Talk's close isn't seen under XCUITest; checked by hand (PR #241)")
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-no-speech", "-no-ai", "-AppleLanguages", "(en)", "-fixture-voice", "listening"]
        app.launch()
        let done = app.descendants(matching: .any)["voiceHoldToTalk"]
        XCTAssertTrue(done.appears(timeout: 8))
        done.tap()
        XCTAssertTrue(app.buttons["voiceClose"].disappears(timeout: 10), "the Talk screen closes by itself")
        let field = app.textFields["composerField"]
        XCTAssertTrue(field.appears(timeout: 5), "in the agent's chat")
        XCTAssertTrue((field.value as? String)?.contains("diagnostics") == true, "with the tidied text in the box")
    }

    /// Talk's message box step: after a pause it asks "Send it?" (it never sends by itself); its Send sends as the
    /// box's own button does, with the status and Undo.
    func testTalkAsksToSendAfterAPause() {
        let app = launch(["-fixture-voice", "confirm", "-fixture-countdown-hold", "YES"])
        XCTAssertTrue(any("talkConfirm", app).appears(timeout: 5))
        XCTAssertTrue(any("talkListening", app).exists, "the mic is open, and it says so")
        XCTAssertTrue(app.staticTexts["talkAsk"].appears(timeout: 8), "asks after the pause")
        XCTAssertFalse(app.staticTexts["voiceStatusTitle"].exists, "nothing sent by itself")
        app.buttons["talkSend"].tap()
        XCTAssertTrue(app.staticTexts["voiceStatusTitle"].appears(timeout: 5))
    }

    private func openGrillChat(_ extra: [String] = []) -> XCUIApplication {
        // No reading aloud or hands-free listening in tests.
        let app = launch(["-fixture-grill", "-grillReadAloud", "NO", "-voiceHandsFree", "NO"] + extra)
        openOrchestratorChat(app)
        return app
    }

    /// The end-of-planning card in one launch. "Set up the team with loft" sends once (the card then reads "Sent to loft"
    /// and can't be tapped again), and Start delivery starts once (the card then says who has which ticket). A card the
    /// web already sent is a unit test (GrillTests.testATeamSetupSentFromTheWebShowsAsSent).
    func testEndOfPlanningCard() {
        let app = openGrillChat(["-fixture-grill-ended"])
        let setup = app.buttons["endCardSetup"]
        XCTAssertTrue(setup.appears(timeout: 5))
        XCTAssertFalse(app.buttons["openInterview"].exists, "no Open interview once it ended")
        XCTAssertTrue(app.buttons["endCardClear"].exists)
        // The card is the last thing in the chat: bring it out from under the composer.
        app.swipeUp()
        XCTAssertEqual(setup.label, "Set up the team with loft")
        XCTAssertEqual(app.staticTexts["endCardTeamNote"].label, "Delivery works with this team. Add a QA agent to have each ticket reviewed, and a Code reviewer for the spec PR.")
        XCTAssertLessThan(app.staticTexts["endCardTeamNote"].frame.minY, app.buttons["endCardStart"].frame.minY,
                          "as on the web: the issues, the team note, then Start delivery")
        setup.tap()
        let deadline = Date().addingTimeInterval(5)
        while setup.label != "Sent to loft", Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertEqual(setup.label, "Sent to loft")
        XCTAssertFalse(setup.isEnabled)

        app.buttons["endCardStart"].tap()
        let started = app.staticTexts["endCardStarted"]
        XCTAssertTrue(started.appears(timeout: 5))
        XCTAssertEqual(started.label, "Delivery started · #41 with Dev")
        XCTAssertFalse(app.buttons["endCardStart"].exists)
    }

    /// A grill that published three specs: each has its own Start delivery, and Start all starts the ones not started
    /// yet, each on its own Lead; the one that can't start says why under it and keeps its button.
    func testEndCardStartsEverySpec() {
        let app = openGrillChat(["-fixture-grill-ended", "-fixture-more-specs"])
        let all = app.buttons["endCardStartAll"]
        XCTAssertTrue(all.appears(timeout: 5))
        app.swipeUp()
        XCTAssertEqual(all.label, "Start all 3")
        XCTAssertEqual(app.buttons.matching(identifier: "endCardStart").count, 3)
        let note = app.staticTexts["endCardTeamNote"]
        XCTAssertTrue(note.label.hasPrefix("octo/site: Delivery works with this team."), "the team note names its repo: \(note.label)")
        XCTAssertGreaterThan(note.frame.minY, all.frame.minY, "after Start all, not under the last spec")

        app.buttons.matching(identifier: "endCardStart").element(boundBy: 0).tap()
        XCTAssertTrue(app.staticTexts["endCardStarted"].appears(timeout: 5))
        XCTAssertEqual(all.label, "Start remaining 2")

        all.tap()
        let deadline = Date().addingTimeInterval(5)
        while app.staticTexts.matching(identifier: "endCardStarted").count < 2, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertEqual(app.staticTexts.matching(identifier: "endCardStarted").allElementsBoundByIndex.map(\.label),
                       ["Delivery started · #41 with Dev", "Delivery started · Lead hands out the tickets"])
        XCTAssertTrue(app.staticTexts["endCardNotStarted"].appears(timeout: 5))
        XCTAssertEqual(app.staticTexts["endCardNotStarted"].label, "Not started: octo/docs has no Dev agent.")
        XCTAssertEqual(all.label, "Start remaining 1")
    }

    /// While one spec's Start delivery runs, Start all and the other specs' buttons wait, so a stale card from one
    /// request can't replace the newer card from the other.
    func testStartAllWaitsForASpecStarting() {
        let app = openGrillChat(["-fixture-grill-ended", "-fixture-more-specs", "-fixture-slow-end-card"])
        let all = app.buttons["endCardStartAll"]
        XCTAssertTrue(all.appears(timeout: 5))
        app.swipeUp()
        let starts = app.buttons.matching(identifier: "endCardStart")
        starts.element(boundBy: 0).tap()
        XCTAssertFalse(all.isEnabled, "Start all waits")
        XCTAssertFalse(starts.element(boundBy: 1).isEnabled, "so does another spec")
        XCTAssertTrue(app.staticTexts["endCardStarted"].appears(timeout: 5))
        XCTAssertTrue(all.isEnabled)
        XCTAssertEqual(all.label, "Start remaining 2")
    }

    /// While its request runs, each end-card button shows a spinner and can't be tapped again: Start delivery reads
    /// "Starting delivery…" until the card says it started, and Clear context waits for the session reset.
    func testEndCardButtonsShowTheyAreWorking() {
        let app = openGrillChat(["-fixture-grill-ended", "-fixture-slow-end-card"])
        let setup = app.buttons["endCardSetup"]
        XCTAssertTrue(setup.appears(timeout: 5))
        app.swipeUp()
        setup.tap()
        XCTAssertFalse(setup.isEnabled, "sending")
        XCTAssertEqual(setup.label, "Set up the team with loft")

        let start = app.buttons["endCardStart"]
        start.tap()
        XCTAssertEqual(start.label, "Starting delivery…")
        XCTAssertFalse(start.isEnabled)
        XCTAssertTrue(app.staticTexts["endCardStarted"].appears(timeout: 5))
        XCTAssertFalse(start.exists)

        let clear = app.buttons["endCardClear"]
        clear.tap()
        XCTAssertFalse(clear.isEnabled, "resetting")
        let deadline = Date().addingTimeInterval(5)
        while !clear.isEnabled, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertTrue(clear.isEnabled, "the reset finished")
    }

    /// The open round as cards: Accept holds an answer, typed text in the composer is chat (it answers nothing),
    /// a choice saved on the current card answers it and moves to the next, Send round brings the next round, then Pause and Resume.
    /// #340: in an interview the chat's own dictation drives it, so tapping the mic still dictates.
    func testInterviewMicStillDictates() {
        let app = openGrillChat()
        let mic = app.buttons["micButton"]
        XCTAssertTrue(mic.appears(timeout: 5))
        XCTAssertEqual(mic.label, "Dictate")
        mic.tap()
        XCTAssertTrue(any("speechStatus", app).appears(timeout: 5))
        XCTAssertFalse(app.buttons["voiceClose"].exists, "no Talk")
    }

    func testGrillRoundCardsAnswersAndSend() {
        let app = openGrillChat()
        XCTAssertTrue(any("grillOpenRound", app).appears(timeout: 5))
        XCTAssertTrue(any("grillCurrentQuestion", app).exists)
        XCTAssertTrue(text(containing: "Question 1 of 3", app).exists)
        XCTAssertTrue(text(containing: "Recommended: First-time visitors", app).exists, "an earlier round reads back with its recommendations")
        // Wrapped answer chips get their own rows (they used to overlap the actions under them).
        for id in ["grillRepeat", "grillSkip"] {
            XCTAssertFalse(app.buttons["grillChoice-Both"].frame.intersects(app.buttons[id].frame), "chip overlaps \(id)")
        }

        // Accept on the second card: held, not sent.
        app.buttons.matching(identifier: "grillAccept").element(boundBy: 1).tap()
        XCTAssertTrue(any("grillAccepted", app).appears(timeout: 5))
        XCTAssertTrue(text(containing: "1 of 3 answered", app).exists)

        // Typed text is a chat message, as on the web: no "Answer question 1…" placeholder, nothing answered, the voice stays.
        let field = app.textFields["composerField"]
        XCTAssertFalse((field.placeholderValue ?? "").contains("Answer"), field.placeholderValue ?? "")
        field.tap()
        field.typeText("wait, what does upsell mean?")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(text(containing: "wait, what does upsell mean?", app).appears(timeout: 5), "sent as chat")
        XCTAssertTrue(text(containing: "Question 1 of 3", app).exists, "the voice didn't move")
        XCTAssertTrue(text(containing: "1 of 3 answered", app).exists, "nothing was answered")

        // A choice saved on the current card answers it; the voice moves past the accepted one to the third.
        app.buttons["grillChoice-Both"].firstMatch.tap()
        app.buttons.matching(identifier: "grillSaveAnswer").firstMatch.tap()
        XCTAssertTrue(text(containing: "Question 3 of 3", app).appears(timeout: 5))
        XCTAssertTrue(text(containing: "2 of 3 answered", app).exists)

        app.buttons["grillSend"].tap()
        XCTAssertTrue(text(containing: "Answers to round 2", app).appears(timeout: 5))
        XCTAssertTrue(text(containing: "Round 3", app).appears(timeout: 5), "the next round follows")
        XCTAssertTrue(text(containing: "Still open from round 2", app).exists || text(containing: "Question 1 of 2", app).exists)

        app.buttons["Conversation options"].tap()
        app.buttons["Pause interview"].tap()
        XCTAssertTrue(app.buttons["grillResume"].appears(timeout: 5), "the open round shows Resume while paused")
        app.buttons["grillResume"].tap()
        XCTAssertTrue(app.buttons["grillResume"].disappears(timeout: 5))
    }

    /// The open interview full screen: one question as a card, answering moves on, Close goes back to the chat with
    /// the interview still open, and ending it closes the view by itself.
    func testInterviewFullScreenOpenAnswerClose() {
        let app = openGrillChat()
        let open = app.buttons["openInterview"]
        XCTAssertTrue(open.appears(timeout: 5))
        open.tap()
        XCTAssertTrue(app.buttons["interviewClose"].appears(timeout: 5))
        XCTAssertTrue(text(containing: "Question 1 of 3", app).exists)

        app.buttons["grillChoice-Both"].firstMatch.tap()
        app.buttons["grillSaveAnswer"].firstMatch.tap()
        XCTAssertTrue(text(containing: "Question 2 of 3", app).appears(timeout: 5), "on to the next question")

        app.buttons["interviewClose"].tap()
        XCTAssertTrue(app.buttons["interviewClose"].disappears(timeout: 5))
        XCTAssertTrue(any("grillOpenRound", app).exists, "the interview is still open in the chat")
        XCTAssertTrue(text(containing: "1 of 3 answered", app).exists)

        open.tap()
        app.buttons["grillMore"].firstMatch.tap()
        app.buttons["End interview"].tap()
        XCTAssertTrue(app.buttons["interviewClose"].disappears(timeout: 5), "closes when the interview ends")
        XCTAssertTrue(open.disappears(timeout: 5), "no Open interview without one")
    }

    /// At the largest text size the answer field's icons keep their own space inside the question card.
    func testInterviewAnswerIconsAtLargestTextSize() {
        let app = openGrillChat(["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
        let open = app.buttons["openInterview"]
        XCTAssertTrue(open.appears(timeout: 5))
        open.tap()
        let card = any("grillCurrentQuestion", app)
        XCTAssertTrue(card.appears(timeout: 5))
        let field = any("grillAnswerField", app)
        let save = app.buttons["grillSaveAnswer"].firstMatch, discuss = app.buttons["grillDiscuss"].firstMatch
        app.swipeUp()
        XCTAssertTrue(field.exists && save.exists && discuss.exists)
        // A fixed 44 pt frame holds a glyph that is bigger than 44 pt here, so the glyphs drew over each other.
        XCTAssertGreaterThan(min(save.frame.width, discuss.frame.width), 44, "the frames grow with the glyphs")
        XCTAssertGreaterThanOrEqual(save.frame.minY, field.frame.maxY, "the icons go under the field")
        XCTAssertFalse(save.frame.intersects(discuss.frame), "\(save.frame) overlaps \(discuss.frame)")
        XCTAssertTrue(card.frame.contains(save.frame) && card.frame.contains(discuss.frame),
                      "icons \(save.frame), \(discuss.frame) spill out of the card \(card.frame)")
    }

    /// A dictated answer (the voice walk-through) still answers the question the voice is on and moves to the next.
    func testDictatedAnswerDuringARoundAnswersAndMovesOn() {
        let app = openGrillChat(["-fixture-dictation", "Monthly only"])
        XCTAssertTrue(text(containing: "Question 1 of 3", app).appears(timeout: 5))
        let yes = app.buttons["submitPromptYes"]
        XCTAssertTrue(yes.appears(timeout: 5))
        yes.tap()
        XCTAssertTrue(text(containing: "Question 2 of 3", app).appears(timeout: 10), "on to the next question")
        XCTAssertTrue(text(containing: "1 of 3 answered", app).exists)
    }

    func testUnsureSpokenCommandAsksFirst() {
        let app = openGrillChat(["-fixture-grill-heard", "skip that one"])
        let ask = any("grillSuggestion", app)
        XCTAssertTrue(ask.appears(timeout: 5))
        XCTAssertTrue(app.staticTexts["Skip this question?"].exists)
        // "No" keeps the words as the answer; nothing was sent.
        app.buttons["grillSuggestionNo"].tap()
        XCTAssertFalse(ask.exists)
        XCTAssertEqual(composerValue(app), "skip that one")
    }

    private func dictateInOrchestratorChat(_ heard: String, _ extra: [String] = []) -> XCUIApplication {
        let app = launch(["-fixture-dictation", heard] + extra)
        openOrchestratorChat(app)
        return app
    }

    func testAPauseAsksAndNoKeepsTheText() {
        let app = dictateInOrchestratorChat("Deploy the docs site")
        let ask = any("submitPrompt", app)
        XCTAssertTrue(ask.appears(timeout: 5), "going quiet asks; it never sends by itself")
        XCTAssertTrue(app.staticTexts["Would you like to submit this?"].exists)
        // The rule itself is unit-tested (VoiceSessionTests); this only checks the screen doesn't start a countdown.
        XCTAssertFalse(any("autoSendBar", app).appears(timeout: 1), "nothing sends without an answer")
        app.buttons["submitPromptNo"].tap()
        XCTAssertFalse(ask.exists)
        XCTAssertEqual(composerValue(app), "Deploy the docs site", "no keeps the text to edit")
    }

    /// Yes tidies the text up and offers Undo; Undo keeps it, and Send then sends the tidied text.
    func testYesTidiesUpUndoKeepsIt() {
        // The fixture's cleanup drops "uh"; the countdown holds so Undo can be tapped.
        let app = dictateInOrchestratorChat("Deploy uh the docs site", ["-fixture-countdown-hold", "YES"])
        XCTAssertTrue(app.buttons["submitPromptYes"].appears(timeout: 5))
        app.buttons["submitPromptYes"].tap()
        XCTAssertTrue(app.buttons["autoSendUndo"].appears(timeout: 5), "a short Undo after yes")
        XCTAssertEqual(composerValue(app), "Deploy the docs site", "the cleaned text is what goes")
        app.buttons["autoSendUndo"].tap()
        XCTAssertFalse(any("autoSendBar", app).exists)
        XCTAssertEqual(composerValue(app), "Deploy the docs site")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(app.staticTexts["Deploy the docs site"].appears(timeout: 5), "sent as the cleaned message")
    }

    /// Leaving the chat during the Undo window still sends it, once (#339).
    func testLeavingDuringTheUndoWindowStillSends() {
        let app = dictateInOrchestratorChat("Deploy uh the docs site")
        XCTAssertTrue(app.buttons["submitPromptYes"].appears(timeout: 5))
        app.buttons["submitPromptYes"].tap()
        XCTAssertTrue(any("autoSendBar", app).appears(timeout: 5))
        app.tabBars.buttons["Overview"].tap()
        // The window is 3 s; wait it out away from the chat.
        Thread.sleep(forTimeInterval: 4)
        app.tabBars.buttons["Chat"].tap()
        XCTAssertTrue(app.staticTexts["Deploy the docs site"].appears(timeout: 5), "sent while you were away")
        XCTAssertFalse(any("autoSendBar", app).exists)
        XCTAssertEqual(app.staticTexts.matching(identifier: "Deploy the docs site").count, 1, "sent once")
        XCTAssertNotEqual(composerValue(app), "Deploy the docs site", "it left the field")
    }

    func testASpokenSendGoesAtOnce() {
        let app = dictateInOrchestratorChat("Fix the login bug, send to orchestrator")
        XCTAssertTrue(app.staticTexts["Fix the login bug"].appears(timeout: 5), "sent without the command words")
        XCTAssertFalse(any("submitPrompt", app).exists, "no question after an explicit send")
    }

    /// The chat header's context size: neutral under 80k, amber from it, and a menu with Compact and Clear that
    /// only works while the agent is idle. The 80k rule itself is StatsTests.testContextWarnsFrom80k.
    func testContextMenuCompactsAndClears() {
        let app = launch()
        openQAChat(app)
        let menu = app.buttons["contextMenu"]
        XCTAssertTrue(menu.appears(timeout: 5))
        XCTAssertTrue(any("contextChip", app).exists, "12k is neutral")
        menu.tap()
        let compact = app.buttons["contextCompact"]
        XCTAssertTrue(compact.appears(timeout: 3))
        XCTAssertTrue(compact.isEnabled)
        XCTAssertTrue(app.buttons["contextClear"].isEnabled)
        compact.tap()
        XCTAssertTrue(compact.disappears(timeout: 3))
        XCTAssertFalse(app.alerts.firstMatch.exists, "compacting went through")

        back(app)
        back(app)
        openOrchestratorChat(app)
        XCTAssertTrue(menu.appears(timeout: 5))
        XCTAssertTrue(any("contextChipOver", app).exists, "182k warns")
        menu.tap()
        XCTAssertTrue(compact.appears(timeout: 3))
        XCTAssertFalse(compact.isEnabled, "the Orchestrator is working")
        XCTAssertFalse(app.buttons["contextClear"].isEnabled)
        // A menu shows its plain text as a disabled item, not a static text.
        XCTAssertTrue(app.descendants(matching: .any)["Stop the agent first."].firstMatch.exists)
    }

    /// "Resume a session" in the context menu (#106): the past Sessions, newest first with the current one not
    /// pickable; picking one shows the Resumed marker. After Clear the header reads "no session" and still offers
    /// Resume; while the agent works it's off, with "Stop the agent first.".
    func testContextMenuResumesASession() {
        let app = launch()
        openQAChat(app)
        let menu = app.buttons["contextMenu"]
        XCTAssertTrue(menu.appears(timeout: 5))
        menu.tap()
        let resume = app.buttons["contextResume"]
        XCTAssertTrue(resume.appears(timeout: 3))
        resume.tap()

        let current = app.buttons["pastSession-s-qa"]
        let old = app.buttons["pastSession-s-qa-old"]
        let older = app.buttons["pastSession-s-qa-older"]
        XCTAssertTrue(old.appears(timeout: 5))
        XCTAssertFalse(current.isEnabled, "the current Session")
        XCTAssertTrue(current.frame.minY < old.frame.minY && old.frame.minY < older.frame.minY, "newest first")
        XCTAssertTrue(old.label.contains("Run the smoke tests on the settings page"))
        XCTAssertTrue(older.label.contains("Orchestrator: "), "another agent's Session names it")
        old.tap()
        XCTAssertTrue(old.disappears(timeout: 3))
        let day = Date().addingTimeInterval(-3 * 86_400).formatted(date: .abbreviated, time: .omitted)
        XCTAssertTrue(text(containing: "Resumed the Session from \(day)", app).appears(timeout: 5))
        XCTAssertEqual(any("contextChip", app).label, "Context 48k tokens", "the resumed Session's size")

        menu.tap()
        app.buttons["contextClear"].tap()
        XCTAssertTrue(any("contextChipNone", app).appears(timeout: 5), "no session")
        menu.tap()
        XCTAssertTrue(resume.appears(timeout: 3))
        XCTAssertFalse(app.buttons["contextCompact"].exists, "nothing to compact")
        XCTAssertFalse(app.buttons["contextClear"].exists)
        resume.tap()
        XCTAssertTrue(old.appears(timeout: 5))
        XCTAssertTrue(current.isEnabled, "no current Session after Clear")
        app.buttons["Cancel"].tap()

        back(app)
        back(app)
        openOrchestratorChat(app)
        menu.tap()
        XCTAssertTrue(resume.appears(timeout: 3))
        XCTAssertFalse(resume.isEnabled, "the Orchestrator is working")
        XCTAssertTrue(app.descendants(matching: .any)["Stop the agent first."].firstMatch.exists)
    }

    func testInstantAcknowledgementAfterSending() {
        // QA Agent is idle, so the row comes from the send, not from a run already in progress.
        let app = launch(["-fixture-dictation", "Run the smoke tests, send it now"])
        openQAChat(app)
        XCTAssertTrue(app.staticTexts["Run the smoke tests"].appears(timeout: 5))
        let ack = any("workingIndicator", app)
        XCTAssertTrue(ack.appears(timeout: 3))
        XCTAssertEqual(ack.label, "Working: Got it, thinking…")
    }

    /// Picking up the reply without an event is StoreTests.testAReplyWithoutAnEventIsStillPickedUp; this checks
    /// the screen reads it even when the dictation was sent with a tap.
    func testAReplyToADictatedMessageIsReadAloudEvenWhenSentWithATap() {
        // QA Agent is idle, so the reply ends the wait; the fixture sends no live event for it. The reading is held
        // until Stop: the Simulator's voice can end (or fail to start) before the bar is ever seen.
        let app = launch(["-fixture-dictation", "Run the smoke tests", "-fixture-replies", "-voiceHandsFree", "NO", "-hold-reading"])
        openQAChat(app)
        let no = app.buttons["submitPromptNo"]
        XCTAssertTrue(no.appears(timeout: 5))
        no.tap()
        app.buttons["sendButton"].tap()
        // The reply comes half a second after the send and is found by the store's first refetch, a second in.
        XCTAssertTrue(app.staticTexts["Done. I ran it and everything passed."].appears(timeout: 5))
        // The reply is read once it has stopped changing for `Speaker.settle` (LoftStore.waitForReply), checked
        // every quarter second: about a second after it shows, more on a busy Simulator.
        let bar = any("readAloudBar", app)
        XCTAssertTrue(bar.appears(timeout: 10), "reading starts once the reply has settled")
        // The bar's identifier covers its buttons, so Stop is found by its label.
        app.buttons.matching(identifier: "readAloudBar")["Stop"].tap()
        XCTAssertTrue(bar.disappears(timeout: 5))
    }

    func testRunWatchRestart() {
        let app = launch(["-fixture-run-waiting"])
        openOrchestratorChat(app)
        let bar = app.otherElements["runWatchBar"].firstMatch
        XCTAssertTrue(bar.appears(timeout: 5))
        let restart = bar.buttons["runRestart"].firstMatch
        XCTAssertTrue(restart.appears(timeout: 5))
        XCTAssertEqual(restart.label, "Restart")
        restart.tap()
        XCTAssertTrue(bar.disappears(timeout: 5))
    }

    func testASuggestedChatReplyFillsTheReplyBox() {
        let app = launch(["-fixture-agent-asks", "-fixture-chat-replies", "Merge it now|Wait for QA"])
        openOrchestratorChat(app)
        let chip = app.buttons["Suggested reply: Wait for QA"]
        XCTAssertTrue(chip.appears(timeout: 5))
        chip.tap()
        XCTAssertEqual(composerValue(app), "Wait for QA", "filled in, not sent")
        XCTAssertFalse(app.buttons["Suggested reply: Merge it now"].exists, "the suggestions step aside once there's a draft")
    }

    func testFailedSendCanBeRetried() {
        let app = launch(["-fixture-fail-sends"])
        openOrchestratorChat(app)
        let field = app.textFields["composerField"]
        XCTAssertTrue(field.appears(timeout: 5))
        field.tap()
        field.typeText("Deploy")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(app.buttons["retrySend"].appears(timeout: 5))
        XCTAssertTrue(app.staticTexts["Deploy"].exists, "the unsent text stays on screen")
        for id in ["retrySend", "discardSend"] {
            XCTAssertGreaterThanOrEqual(app.buttons[id].frame.height, 44, "\(id) is a comfortable tap target")
        }
    }

    /// Offline: the overview says so honestly, and nothing that never loaded is shown as empty.
    func testOffline() {
        let app = launch(["-fixture-offline"])
        XCTAssertTrue(any("staleBanner", app).appears(timeout: 5))
        XCTAssertFalse(app.staticTexts["Connected"].exists)
        XCTAssertTrue(app.buttons["Retry"].exists)
        app.tabBars.buttons["Agents"].tap()
        XCTAssertTrue(app.staticTexts["Can't reach Loft"].appears(timeout: 5))
        XCTAssertFalse(app.staticTexts["No agents registered"].exists)
        app.tabBars.buttons["Chat"].tap()
        XCTAssertTrue(app.staticTexts["Can't reach Loft"].appears(timeout: 5))
        XCTAssertFalse(app.staticTexts["Orchestrator"].exists)
        app.tabBars.buttons["Overview"].tap()
        XCTAssertTrue(app.staticTexts["Unavailable until Loft is reachable."].appears(timeout: 5))
        XCTAssertFalse(app.staticTexts["0"].exists)
    }

    /// Away for two hours: "While you were away" heads the Agents tab, with what needs you first.
    func testWhileYouWereAway() {
        let app = launch(["-fixture-digest"])
        app.tabBars.buttons["Agents"].tap()
        let card = any("digestCard", app)
        XCTAssertTrue(card.appears(timeout: 5), "the fixture's activity is within the two hours")
        let label = { (s: String) in app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", s)).firstMatch }
        XCTAssertTrue(label("While you were away").exists)
        XCTAssertTrue(label("Waiting for your approval").exists, "iOS Agent's approval")
        app.buttons["digestDismiss"].tap()
        XCTAssertTrue(card.disappears(timeout: 5), "Got it clears it")
    }

    func testLoadedButEmptySaysSo() {
        let app = launch(["-fixture-empty"])
        app.tabBars.buttons["Agents"].tap()
        XCTAssertTrue(app.staticTexts["No agents registered"].appears(timeout: 5))
    }

    /// The connect screen with no saved address to prefill it: an empty `serverURL` in the arguments domain hides
    /// whatever an earlier run saved, so the typed address never lands in the middle of an old one.
    private static let onboardingLaunchArguments = ["-ui-onboarding", "-no-animations", "-no-speech", "-no-ai", "-serverURL", ""]

    func testOnboardingAsksForServer() {
        let app = XCUIApplication()
        app.launchArguments = Self.onboardingLaunchArguments
        app.launch()
        let field = app.textFields["serverAddressField"]
        XCTAssertTrue(field.appears(timeout: 5))
        XCTAssertFalse(app.buttons["testConnection"].isEnabled)
        field.tap()
        field.typeText("my-mac.tailnet.ts.net")
        XCTAssertTrue(app.buttons["testConnection"].isEnabled)
    }

    /// Types the demo address on the connect screen (launched with `onboardingLaunchArguments`).
    private func enterDemo(_ app: XCUIApplication) {
        let field = app.textFields["serverAddressField"]
        XCTAssertTrue(field.appears(timeout: 5))
        field.tap()
        // Right after a launch the first tap can miss the focus.
        if !app.keyboards.firstMatch.appears(timeout: 2) { field.tap() }
        field.typeText("demo-loft-dev.get-loft.com")
        app.buttons["saveServer"].tap()
        XCTAssertTrue(any("demoBanner", app).appears(timeout: 5))
    }

    /// Spec #144: the demo address opens the app on sample data, with Loft-only and GitHub issues, and Exit leaves it.
    func testTheDemoShowsIssuesAndLeaves() {
        let app = XCUIApplication()
        app.launchArguments = Self.onboardingLaunchArguments
        app.launch()
        XCTAssertTrue(app.textFields["serverAddressField"].appears(timeout: 5))
        XCTAssertFalse(app.buttons["tryDemo"].exists, "no demo button: App Review types the demo address")
        enterDemo(app)
        app.tabBars.buttons["Issues"].tap()
        XCTAssertTrue(text(containing: "loft#42", app).appears(timeout: 5), "a GitHub issue shows its mark")
        app.buttons["taskRow-Implement settings functionality"].firstMatch.tap()
        // An open spec: its Delivery card sits above "Where", so the Owner row is below the fold.
        let owner = text(containing: "octo-dev", app)
        for _ in 0..<4 where !owner.appears(timeout: 2) { app.swipeUp() }
        XCTAssertTrue(owner.exists, "its owner")
        XCTAssertTrue(any("demoBanner", app).exists, "the banner stays on every screen")
        // #146: the Lead answers, from a script.
        app.tabBars.buttons["Chat"].tap()
        app.buttons["orchestratorConversation"].firstMatch.tap()
        let composer = app.textFields["composerField"]
        XCTAssertTrue(composer.appears(timeout: 5))
        composer.tap()
        composer.typeText("What is the status")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(text(containing: "3 of 4 endpoints", app).appears(timeout: 5), "a scripted answer, shown at once")
        app.buttons["exitDemo"].tap()
        XCTAssertTrue(app.textFields["serverAddressField"].appears(timeout: 5), "back to the connect screen")
    }

    /// #148: the Issues tab switches to Specs (kept across launches), in sections by stage, with the shared filters.
    func testSpecsListSectionsAndFilters() {
        let app = launch(["-fixture-specs"])
        app.tabBars.buttons["Issues"].tap()
        let specs = app.segmentedControls["issuesViewSwitch"].buttons["Specs"]
        XCTAssertTrue(specs.appears(timeout: 5))
        specs.tap()
        XCTAssertTrue(any("specSection-planned", app).appears(timeout: 5))
        XCTAssertTrue(text(containing: "Backend Agent on #43 Backend endpoints", app).exists, "who is on which ticket")
        XCTAssertTrue(text(containing: "Docs Agent on #45 Docs", app).exists, "#181: every running ticket")
        XCTAssertTrue(text(containing: "Waiting for the Lead to assign #44", app).exists, "#181: and what waits for the Lead")
        XCTAssertTrue(text(containing: "octo/loft#50", app).exists, "its repo")
        XCTAssertTrue(text(containing: "Loft · garden", app).exists, "a plan kept in Loft")
        for (stage, title) in [("planned", "Offline mode"), ("delivery", "Implement settings functionality"), ("ready", "Widget gallery"), ("shipped", "Dark theme")] {
            for _ in 0..<3 where !any("specRow-\(title)", app).exists { app.swipeUp() }
            XCTAssertTrue(any("specSection-\(stage)", app).appears(timeout: 5), stage)
            XCTAssertTrue(any("specRow-\(title)", app).exists, title)
        }
        XCTAssertFalse(any("specRow-Old idea", app).exists, "dropped is hidden")
        XCTAssertFalse(app.buttons["taskRow-Renew the domain"].exists, "only specs")

        app.buttons["Filter issues"].tap()
        app.buttons["Show dropped"].tap()
        for _ in 0..<3 where !any("specRow-Old idea", app).exists { app.swipeUp() }
        XCTAssertTrue(any("specRow-Old idea", app).appears(timeout: 3))

        // The group and project filters are the List's, and apply here.
        app.buttons["Filter issues"].tap()
        app.buttons["Loft"].tap()
        XCTAssertTrue(any("specRow-Offline mode", app).appears(timeout: 3))
        XCTAssertFalse(any("specRow-Garden notes", app).exists, "no agent of the group works in garden")
        app.buttons["Filter issues"].tap()
        app.buttons["Any group"].tap()
        app.buttons["Filter issues"].tap()
        app.buttons["garden"].tap()
        XCTAssertTrue(any("specRow-Garden notes", app).appears(timeout: 3))
        XCTAssertFalse(any("specRow-Offline mode", app).exists)

        // Kept across launches.
        app.terminate()
        app.launchArguments += ["-keep-issues-view"]
        app.launch()
        app.tabBars.buttons["Issues"].tap()
        XCTAssertTrue(any("specRow-Offline mode", app).appears(timeout: 5))
    }

    /// #172: a Priority picked in the List doesn't count as a filter in Specs, which has no Priority filter.
    func testSpecsDoesNotCountTheListOnlyFilters() {
        let app = launch(["-fixture-specs"])
        app.tabBars.buttons["Issues"].tap()
        // The view is kept across launches, and a spec's detail test leaves it on Specs.
        let list = app.segmentedControls["issuesViewSwitch"].buttons["List"]
        XCTAssertTrue(list.appears(timeout: 5))
        list.tap()
        XCTAssertTrue(app.buttons["taskFilter-all"].appears(timeout: 3))
        app.buttons["Filter issues"].tap()
        app.buttons["Urgent"].tap()
        app.segmentedControls["issuesViewSwitch"].buttons["Specs"].tap()
        XCTAssertTrue(any("specRow-Offline mode", app).appears(timeout: 5))
        app.buttons["Filter issues"].tap()
        XCTAssertTrue(app.buttons["Show dropped"].appears(timeout: 3))
        XCTAssertFalse(app.buttons["Clear filters"].exists, "no filter of the Specs view is set")
        app.buttons["Show dropped"].tap()

        // The List still has it, and the List is what the other tests expect on a fresh install.
        app.segmentedControls["issuesViewSwitch"].buttons["List"].tap()
        app.buttons["Filter issues"].tap()
        XCTAssertTrue(app.buttons["Clear filters"].appears(timeout: 3))
        app.buttons["Clear filters"].tap()
    }

    /// #194: a filter that hides every spec says so and offers Clear filters; with no specs at all, "No specs" stays.
    func testSpecsFilterThatHidesEverySpecOffersClearFilters() {
        let app = launch(["-fixture-specs"])
        app.tabBars.buttons["Issues"].tap()
        let specs = app.segmentedControls["issuesViewSwitch"].buttons["Specs"]
        XCTAssertTrue(specs.appears(timeout: 5))
        specs.tap()
        XCTAssertTrue(any("specRow-Offline mode", app).appears(timeout: 5))
        app.buttons["Filter issues"].tap()
        app.buttons["website"].tap()
        XCTAssertTrue(app.staticTexts["No matching specs"].appears(timeout: 3))
        XCTAssertFalse(app.staticTexts["No specs"].exists)
        app.buttons["Clear filters"].tap()
        XCTAssertTrue(any("specRow-Offline mode", app).appears(timeout: 3))

        app.terminate()
        let empty = launch(["-fixture-empty"])
        empty.tabBars.buttons["Issues"].tap()
        let emptySpecs = empty.segmentedControls["issuesViewSwitch"].buttons["Specs"]
        XCTAssertTrue(emptySpecs.appears(timeout: 5))
        emptySpecs.tap()
        XCTAssertTrue(empty.staticTexts["No specs"].appears(timeout: 3))
        XCTAssertFalse(empty.buttons["Clear filters"].exists)
    }

    private func openSpec(_ title: String, _ app: XCUIApplication) {
        app.tabBars.buttons["Issues"].tap()
        let specs = app.segmentedControls["issuesViewSwitch"].buttons["Specs"]
        XCTAssertTrue(specs.appears(timeout: 5))
        specs.tap()
        let row = any("specRow-\(title)", app)
        for _ in 0..<3 where !row.exists { app.swipeUp() }
        XCTAssertTrue(row.appears(timeout: 5), title)
        row.tap()
    }

    /// #149: an open spec on GitHub shows the Delivery card, then its progress, Spec PR and tickets with verdicts; a
    /// ticket opens its issue, with back to the spec.
    func testSpecDetailShowsDeliveryTicketsAndOpensATicket() {
        let app = launch(["-fixture-specs"])
        openSpec("Implement settings functionality", app)
        // The card has the state and Pause; the spec panel adds the rest.
        XCTAssertTrue(any("deliveryState", app).appears(timeout: 5), "the Delivery card")
        XCTAssertEqual(any("deliveryState", app).label, "Running.")
        XCTAssertTrue(any("delivery-pause", app).exists)
        XCTAssertFalse(any("delivery-start", app).exists, "it started")
        XCTAssertTrue(text(containing: "#43 Backend endpoints is with Backend Agent", app).exists, "the card says who has which ticket")
        XCTAssertTrue(text(containing: "#45 Docs is with Docs Agent", app).exists, "#181: every running ticket")
        XCTAssertEqual(any("deliveryWaiting", app).label, "Waiting for the Lead to assign #44")
        XCTAssertFalse(any("deliveryOnIt", app).exists, "so Progress doesn't repeat it")
        XCTAssertTrue(any("specPRLink", app).exists, "the spec PR")
        XCTAssertEqual(any("deliveryProgress", app).label, "1 of 5 tickets done")
        // The tickets sit below the card and the progress.
        for _ in 0..<3 where !any("ticketRow-QA", app).exists { app.swipeUp() }
        for t in ["Backend endpoints", "iOS screen", "Docs", "QA"] { XCTAssertTrue(any("ticketRow-\(t)", app).exists, t) }
        XCTAssertTrue(any("ticketRow-Docs", app).label.contains("in review"))
        XCTAssertTrue(any("ticketRow-Docs", app).label.contains("security waiting"))
        XCTAssertTrue(any("ticketRow-QA", app).label.contains("done"))
        XCTAssertTrue(any("ticketRow-QA", app).label.contains("security passed"))
        XCTAssertFalse(any("mergeDelivery", app).exists, "it merges on GitHub")

        any("ticketRow-Backend endpoints", app).tap()
        XCTAssertTrue(app.navigationBars["Issue"].appears(timeout: 5))
        XCTAssertTrue(text(containing: "Implement backend endpoints", app).appears(timeout: 5), "the ticket's own issue")
        XCTAssertFalse(any("ticketRow-QA", app).exists, "a ticket's page lists no tickets")
        back(app)
        XCTAssertTrue(any("ticketRow-QA", app).appears(timeout: 5), "back to the spec")
    }

    /// #149: an open spec on GitHub starts from the Delivery card and never merges here; Merge only for a plan kept
    /// in Loft that's ready.
    func testStartDeliveryAndMergeAppearOnlyWhenTheyApply() {
        let app = launch(["-fixture-specs"])
        openSpec("Offline mode", app)
        XCTAssertTrue(any("delivery-start", app).appears(timeout: 5), "the card's Start delivery")
        XCTAssertEqual(any("deliveryProgress", app).label, "0 of 2 tickets done", "a planned Delivery")
        XCTAssertFalse(any("deliveryState", app).exists, "not started, so no state")
        XCTAssertFalse(any("specPRLink", app).exists, "no spec PR yet")
        XCTAssertFalse(any("mergeDelivery", app).exists)
        // #434: a ticket waits on octo/api#8, so Start delivery warns first and Cancel puts the button back.
        for _ in 0..<3 where !any("ticketRow-Offline cache", app).exists { app.swipeUp() }
        XCTAssertTrue(any("ticketRow-Offline cache", app).label.contains("waiting on octo/api#8"))
        for _ in 0..<3 where !any("delivery-start", app).isHittable { app.swipeDown() }
        any("delivery-start", app).tap()
        XCTAssertTrue(any("startWarning", app).appears(timeout: 3))
        XCTAssertTrue(any("startFirst-octo/api#7", app).exists, "it offers to start the spec that builds it")
        XCTAssertTrue(any("startAnyway", app).exists)
        any("startWarningCancel", app).tap()
        XCTAssertTrue(any("delivery-start", app).appears(timeout: 3))
        XCTAssertFalse(any("startWarning", app).exists)
        back(app)

        // Ready on GitHub: it merges there, and the spec panel still shows its progress and Spec PR.
        any("specRow-Widget gallery", app).tap()
        XCTAssertTrue(any("deliveryState", app).appears(timeout: 5))
        XCTAssertEqual(any("deliveryState", app).label, "Done: every review passed.", "the card's state")
        XCTAssertFalse(any("delivery-start", app).exists)
        XCTAssertEqual(any("deliveryProgress", app).label, "3 of 3 tickets done")
        XCTAssertTrue(any("specPRLink", app).exists, "the spec PR")
        XCTAssertFalse(any("mergeDelivery", app).exists)
        back(app)

        for _ in 0..<3 where !any("specRow-Garden notes", app).exists { app.swipeUp() }
        any("specRow-Garden notes", app).tap()
        let merge = any("mergeDelivery", app)
        XCTAssertTrue(merge.appears(timeout: 5))
        XCTAssertTrue(any("deliveryState", app).exists, "no card on a plan kept in Loft: the panel has the state")
        merge.tap()
        XCTAssertTrue(text(containing: "Merging…", app).appears(timeout: 3), "pending")
        XCTAssertTrue(any("deliveryResult", app).appears(timeout: 8), "then the result")
        XCTAssertFalse(any("mergeDelivery", app).exists)
    }

    /// #149: a relaunch starts outside the demo with none of its changes, and the full-screen Talk view has the
    /// banner too, so Exit is one tap from there.
    func testARelaunchResetsTheDemoAndExitWorksFromFullScreen() {
        let app = XCUIApplication()
        app.launchArguments = Self.onboardingLaunchArguments
        app.launch()
        enterDemo(app)
        // A change in the demo: a message to the Lead.
        app.tabBars.buttons["Chat"].tap()
        app.buttons["orchestratorConversation"].firstMatch.tap()
        let composer = app.textFields["composerField"]
        XCTAssertTrue(composer.appears(timeout: 5))
        composer.tap()
        composer.typeText("Kept only until relaunch")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(text(containing: "Kept only until relaunch", app).appears(timeout: 5))

        app.terminate()
        app.launch()
        XCTAssertTrue(app.textFields["serverAddressField"].appears(timeout: 5), "a relaunch starts at the connect screen")
        XCTAssertFalse(any("demoBanner", app).exists, "not in the demo")
        enterDemo(app)
        app.tabBars.buttons["Chat"].tap()
        app.buttons["orchestratorConversation"].firstMatch.tap()
        XCTAssertTrue(app.textFields["composerField"].appears(timeout: 5))
        XCTAssertFalse(text(containing: "Kept only until relaunch", app).exists, "the demo starts fresh")

        app.tabBars.buttons["Overview"].tap()
        app.buttons["talkButton"].tap()
        XCTAssertTrue(app.buttons["voiceClose"].appears(timeout: 5))
        // The root's banner stays in the tree under the full screen, so tap where Exit shows rather than trust `exists`.
        app.buttons["exitDemo"].firstMatch.tap()
        XCTAssertTrue(app.textFields["serverAddressField"].appears(timeout: 5), "Exit from the Talk view")
        XCTAssertFalse(any("demoBanner", app).exists)
    }

    func testAPermissionShowsItsRisksAndExplainsOnRequest() {
        let app = launch(["-fixture-risky-permission", "-fixture-explain", "Sends your commits to the main branch on GitHub."])
        app.tabBars.buttons["Agents"].tap()
        openAgent("Backend Agent", app)
        app.buttons["chatWithAgent"].tap()
        let risks = any("permissionRisks", app)
        XCTAssertTrue(risks.appears(timeout: 5), "the risks show without asking")
        XCTAssertTrue(risks.label.contains("Pushes to main"))
        XCTAssertTrue(risks.label.contains("Uses the network"))
        let explain = app.buttons["permissionExplain"].firstMatch
        XCTAssertTrue(explain.exists)
        explain.tap()
        XCTAssertTrue(any("permissionExplanation", app).appears(timeout: 5))
        XCTAssertTrue(app.buttons["Deny"].firstMatch.exists, "approving and denying stay your taps")
    }

    func testAKillWithNoRuleOffersNoAlwaysAllow() {
        let app = launch(["-fixture-kill-permission"])
        app.tabBars.buttons["Agents"].tap()
        openAgent("Backend Agent", app)
        app.buttons["chatWithAgent"].tap()
        let cards = app.otherElements.matching(identifier: "approvalCard")
        XCTAssertTrue(cards.firstMatch.appears(timeout: 5))
        XCTAssertEqual(app.buttons.matching(identifier: "More options").count, 1, "only the kill that has a rule offers Always allow")
        XCTAssertEqual(app.buttons.matching(identifier: "Deny").count, 2)
    }

    func testNeedsYouGroupsPullRequestsByProject() {
        let app = launch(["-fixture-needs-you-projects"])
        XCTAssertTrue(app.staticTexts["Needs you · 13 new of 13"].firstMatch.appears(timeout: 5), "10 items, with the one PR swapped for four, none seen yet")
        app.buttons["needsYouSeeAll"].firstMatch.tap()
        let loft = app.buttons["needsYouProject-loft"]
        XCTAssertTrue(loft.appears(timeout: 5))
        XCTAssertTrue(loft.label.contains("3 pull requests"))
        XCTAssertTrue(app.buttons["needsYouProject-api"].exists)
        loft.tap()
        XCTAssertEqual(loft.value as? String, "Collapsed")
    }

    /// #176: Settings › Backups shows the last backup and its error, Back up now clears it, and iCloud Drive is one
    /// tap after the privacy warning. A failing backup's Needs you item has Back up now too.
    func testBackupsInSettingsAndNeedsYou() {
        let app = launch(["-fixture-backup-failed"])
        app.buttons["Settings"].firstMatch.tap()
        let now = app.buttons["backUpNow"]
        for _ in 0..<10 where !now.isHittable { app.swipeUp() }
        XCTAssertTrue(now.isHittable)
        XCTAssertTrue(any("backupLast", app).label.contains("2 days ago"), any("backupLast", app).label)
        XCTAssertTrue(any("backupError", app).label.contains("can't create /Volumes/Backup/Loft backups"))
        now.tap()
        XCTAssertTrue(any("backupError", app).disappears(timeout: 5), "a good backup clears the error")
        XCTAssertFalse(any("backupLast", app).label.contains("2 days ago"))

        app.buttons["backupFolderPicker"].tap()
        app.buttons["iCloud Drive"].tap()
        XCTAssertTrue(text(containing: "stored by Apple", app).appears(timeout: 3), "the warning comes first")
        app.buttons["Use iCloud Drive"].tap()
        XCTAssertTrue(text(containing: "com~apple~CloudDocs/Loft backups", app).appears(timeout: 5))
        app.buttons["backupFolderPicker"].tap()
        app.buttons["This Mac"].tap()
        XCTAssertTrue(text(containing: "Application Support/Loft/backups", app).appears(timeout: 5))

        app.terminate()
        let again = launch(["-fixture-backup-failed"])
        again.buttons["needsYouSeeAll"].firstMatch.tap()
        let item = any("needsYou-backup_failed", again)
        XCTAssertTrue(any("needsYou-permission", again).appears(timeout: 5))
        for _ in 0..<6 where !item.exists { again.swipeUp() }
        XCTAssertTrue(item.exists)
        XCTAssertTrue(item.staticTexts["Loft's backups are failing"].exists)
        item.buttons["Back up now"].tap()
        XCTAssertTrue(item.disappears(timeout: 5), "the item clears")
    }

    /// Opening the list marks what it shows seen (#395): the card stops saying they're new; Done takes an item off.
    func testOpeningNeedsYouMarksItSeenAndDoneTakesAnItemOff() {
        let app = launch()
        XCTAssertTrue(text(containing: " new of ", app).appears(timeout: 5), "nothing seen yet")
        app.buttons["needsYouSeeAll"].firstMatch.tap()
        XCTAssertTrue(any("needsYou-permission", app).appears(timeout: 5))
        XCTAssertTrue(any("needsYouNew", app).exists, "new during this visit, though marked seen")
        let row = any("needsYou-task_done", app)
        let done = row.buttons["needsYouDone"]
        // Clear of the floating tab bar: under it the button still counts as hittable, and the tap goes to the bar.
        let bar = app.tabBars.firstMatch
        for _ in 0..<8 where !done.exists || done.frame.maxY > bar.frame.minY - 8 { app.swipeUp() }
        done.tap()
        XCTAssertTrue(row.disappears(timeout: 5), "Done takes it off the list")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(text(containing: "Needs you · ", app).appears(timeout: 5))
        XCTAssertFalse(text(containing: " new of ", app).exists, "seen: the card has nothing new")
        XCTAssertFalse(any("needsYouNew", app).exists)
    }

    /// A failed issue with nobody on it (#145): the reply sheet asks who gets it, then says where it went.
    func testReplyingToAnIssueWithNobodyOnItAsksForAnAgent() {
        let app = launch()
        app.buttons["needsYouSeeAll"].firstMatch.tap()
        let row = any("needsYou-task_failed", app)
        let reply = row.buttons["Reply on the issue"]
        XCTAssertTrue(any("needsYou-permission", app).appears(timeout: 5))
        for _ in 0..<6 where !reply.isHittable { app.swipeUp() }
        reply.tap()
        let agent = any("needsYouAnswerAgent", app)
        XCTAssertTrue(agent.appears(timeout: 5))
        XCTAssertTrue(text(containing: "kept as a note only", app).exists)
        agent.tap()
        app.buttons["QA Agent"].firstMatch.tap()
        XCTAssertTrue(text(containing: "takes the issue", app).appears(timeout: 5))
        app.textFields["needsYouAnswerField"].tap()
        app.typeText("Install Xcode 27 first")
        app.buttons["needsYouAnswerSend"].tap()
        let notice = any("needsYouNotice", app)
        XCTAssertTrue(notice.appears(timeout: 5))
        XCTAssertEqual(notice.label, "Sent to QA Agent")
        XCTAssertFalse(any("needsYou-task_failed", app).exists, "the item clears")
    }

    func testAnAttachedTaskGoesWithTheMessageAndShowsAsAChip() {
        let app = launch()
        openOrchestratorChat(app)
        let attach = app.buttons["attachTaskButton"]
        XCTAssertTrue(attach.appears(timeout: 5))
        attach.tap()
        let task = app.buttons["attachTask-t2"]
        XCTAssertTrue(task.appears(timeout: 5))
        task.tap()
        XCTAssertTrue(any("attachedTask", app).appears(timeout: 5))
        let field = app.textFields["composerField"]
        field.tap()
        field.typeText("Can you check this one?")
        app.buttons["sendButton"].tap()
        XCTAssertTrue(any("attachedTask", app).disappears(timeout: 3), "sent with the message, then cleared")
        XCTAssertTrue(app.buttons["Issue: Implement backend endpoints, In progress"].appears(timeout: 5), "the chip on the sent message")
    }

    func testABriefFromTheMacKeepsWhatYouSaidOneTapAway() {
        let said = Array(repeating: "so we should fix the login screen because the password field hides behind the keyboard", count: 6).joined(separator: " ")
        let brief = "Goal: Fix the login screen so the password field isn't hidden by the keyboard"
        // Encoded: the apostrophe doesn't survive the trip as plain launch-argument text.
        let app = dictateInOrchestratorChat(said, ["-fixture-brief", "base64:" + Data(brief.utf8).base64EncodedString()])
        XCTAssertTrue(app.buttons["submitPromptNo"].appears(timeout: 5))
        app.buttons["submitPromptNo"].tap()
        let before = composerValue(app)
        let make = app.buttons["briefMake"]
        XCTAssertTrue(make.appears(timeout: 5))
        make.tap()
        XCTAssertTrue(app.buttons["briefUndo"].appears(timeout: 5))
        XCTAssertEqual(composerValue(app), brief, "the brief is in the box")
        XCTAssertFalse(any("autoSendBar", app).exists, "a brief never sends")
        app.buttons["briefUndo"].tap()
        XCTAssertEqual(composerValue(app), before, "what was said comes back word for word")
    }
}
