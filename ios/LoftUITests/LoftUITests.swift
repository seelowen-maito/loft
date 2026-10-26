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
        let row = app.buttons["agentRow-QA Agent"].firstMatch
        XCTAssertTrue(row.appears(timeout: 5))
        row.tap()
        app.buttons["chatWithAgent"].tap()
    }

    private func composerValue(_ app: XCUIApplication) -> String? { app.textFields["composerField"].value as? String }

    // MARK: - Default fixture

    /// One launch: live state, the Needs you card, the Mac's temperature, plan usage, approving a permission and
    /// activity; then tasks (subtasks nest, filters narrow, a new task shows up); then the agent list (context
    /// warning), an agent's detail and a direct chat with another agent.
    func testOverviewTasksAndAgents() {
        let app = launch()
        XCTAssertTrue(app.staticTexts["Connected"].appears(timeout: 5))
        // Servers with `needs_you` (the fixture) show the "Needs you" card instead of "Needs your attention".
        XCTAssertTrue(app.staticTexts["Needs you · 8"].appears(timeout: 5))
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
        for _ in 0..<4 where !activity.exists { app.swipeUp(velocity: .fast) }
        XCTAssertTrue(activity.appears(timeout: 5))

        // Tasks.
        app.tabBars.buttons["Tasks"].tap()
        XCTAssertTrue(app.buttons["taskRow-Implement settings functionality"].appears(timeout: 5))
        XCTAssertTrue(app.buttons["taskRow-Implement backend endpoints"].exists, "subtasks nest under their parent")
        app.buttons["taskFilter-failed"].tap()
        XCTAssertTrue(app.buttons["taskRow-Fix broken docs links"].appears(timeout: 3))
        XCTAssertFalse(app.buttons["taskRow-Renew the domain"].exists)
        app.buttons["taskFilter-inbox"].tap()
        XCTAssertTrue(app.buttons["taskRow-Renew the domain"].appears(timeout: 3))

        app.buttons["createButton"].tap()
        app.buttons["New task"].tap()
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
        XCTAssertTrue(warning.label.hasSuffix("over the 200k window"))
        app.buttons["agentRow-Backend Agent"].firstMatch.tap()
        XCTAssertTrue(app.buttons["chatWithAgent"].appears(timeout: 5))
        XCTAssertTrue(app.staticTexts["Implement backend endpoints"].exists)
        back(app)

        openQAChat(app)
        XCTAssertTrue(app.staticTexts["Direct · Idle"].appears(timeout: 5))
        XCTAssertTrue(app.textFields["composerField"].exists)
    }

    /// The Orchestrator chat: compact task rows, sending, the @ picker, the mic's long press, and no manual grill.
    func testOrchestratorChat() {
        let app = launch()
        openOrchestratorChat(app)
        XCTAssertTrue(app.staticTexts["Orchestrator · Working"].appears(timeout: 5))

        let brief = app.buttons["Task: Renew the domain"]
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
        qa.tap()
        XCTAssertEqual(composerValue(app), "@QA Agent ")
        XCTAssertFalse(qa.exists, "the list goes once one is picked")

        app.buttons["Conversation options"].tap()
        // Once the menu is open, Grill me… would be in it: no waiting for something that shouldn't come.
        XCTAssertTrue(app.buttons["Agent details"].appears(timeout: 5))
        XCTAssertFalse(app.buttons["Grill me…"].exists)
        // Close the menu.
        app.tap()

        let mic = app.buttons["micButton"]
        XCTAssertTrue(mic.appears(timeout: 5))
        mic.press(forDuration: 0.8)
        XCTAssertTrue(app.buttons["voiceClose"].appears(timeout: 5))
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

    // MARK: - Other fixtures

    func testClosingReviewKeepsDraftOnOverview() {
        let app = launch(["-fixture-voice", "review"])
        XCTAssertTrue(app.buttons["voiceSend"].appears(timeout: 5))
        app.buttons["voiceClose"].tap()
        let review = app.buttons["voiceDraftReview"]
        XCTAssertTrue(review.appears(timeout: 5))
        review.tap()
        XCTAssertTrue(app.buttons["voiceSend"].appears(timeout: 5))
        app.buttons["voiceClose"].tap()
        let discard = app.buttons["voiceDraftDiscard"]
        XCTAssertTrue(discard.appears(timeout: 5))
        discard.tap()
        XCTAssertTrue(discard.disappears(timeout: 5))
    }

    private func openGrillChat(_ extra: [String] = []) -> XCUIApplication {
        // No reading aloud or hands-free listening in tests.
        let app = launch(["-fixture-grill", "-grillReadAloud", "NO", "-voiceHandsFree", "NO"] + extra)
        openOrchestratorChat(app)
        return app
    }

    /// The interview card: a choice answers it, then Skip, Pause and Resume.
    func testGrillCardAnswersAndQuickActions() {
        let app = openGrillChat()
        let current = any("grillCurrentQuestion", app)
        XCTAssertTrue(current.appears(timeout: 5))
        XCTAssertTrue(current.label.contains("Question 3 of 8"))
        XCTAssertFalse(any("grillChip", app).exists, "no interview mode bar: the card is the interview")
        XCTAssertTrue(text(containing: "Skipped question 2", app).exists)
        // Wrapped answer chips get their own rows (they used to overlap the actions under them).
        for id in ["grillRepeat", "grillSkip", "grillMore"] {
            XCTAssertFalse(app.buttons["grillChoice-Only on yearly"].frame.intersects(app.buttons[id].frame), "chip overlaps \(id)")
        }
        // A choice fills the composer; nothing is sent until Send.
        app.buttons["grillChoice-Only on yearly"].tap()
        XCTAssertEqual(composerValue(app), "Only on yearly")
        app.buttons["sendButton"].tap()
        let next = app.descendants(matching: .any).matching(identifier: "grillCurrentQuestion")
            .containing(NSPredicate(format: "label CONTAINS 'Question 4'")).firstMatch
        XCTAssertTrue(next.appears(timeout: 5), "the next question follows the answer")

        app.buttons["grillSkip"].tap()
        XCTAssertTrue(text(containing: "Skipped question 4", app).appears(timeout: 5))
        app.buttons["Conversation options"].tap()
        app.buttons["Pause interview"].tap()
        XCTAssertTrue(app.buttons["grillResume"].appears(timeout: 5), "the waiting question shows Resume while paused")
        app.buttons["grillResume"].tap()
        XCTAssertTrue(app.buttons["grillSkip"].appears(timeout: 5))
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

    func testASpokenSendGoesAtOnce() {
        let app = dictateInOrchestratorChat("Fix the login bug, send to orchestrator")
        XCTAssertTrue(app.staticTexts["Fix the login bug"].appears(timeout: 5), "sent without the command words")
        XCTAssertFalse(any("submitPrompt", app).exists, "no question after an explicit send")
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
        // QA Agent is idle, so the reply ends the wait; the fixture sends no live event for it.
        let app = launch(["-fixture-dictation", "Run the smoke tests", "-fixture-replies", "-voiceHandsFree", "NO"])
        openQAChat(app)
        let no = app.buttons["submitPromptNo"]
        XCTAssertTrue(no.appears(timeout: 5))
        no.tap()
        app.buttons["sendButton"].tap()
        // The reply comes half a second after the send and is found by the store's first refetch, a second in.
        XCTAssertTrue(app.staticTexts["Done. I ran it and everything passed."].appears(timeout: 5))
        XCTAssertTrue(any("readAloudBar", app).appears(timeout: 5), "reading starts soon after the reply shows")
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

    func testOnboardingAsksForServer() {
        let app = XCUIApplication()
        app.launchArguments = ["-ui-onboarding", "-no-animations", "-no-speech", "-no-ai"]
        app.launch()
        let field = app.textFields["serverAddressField"]
        XCTAssertTrue(field.appears(timeout: 5))
        XCTAssertFalse(app.buttons["testConnection"].isEnabled)
        field.tap()
        field.typeText("my-mac.tailnet.ts.net")
        XCTAssertTrue(app.buttons["testConnection"].isEnabled)
    }

    func testAPermissionShowsItsRisksAndExplainsOnRequest() {
        let app = launch(["-fixture-risky-permission", "-fixture-explain", "Sends your commits to the main branch on GitHub."])
        app.tabBars.buttons["Agents"].tap()
        let row = app.buttons["agentRow-Backend Agent"].firstMatch
        XCTAssertTrue(row.appears(timeout: 5))
        row.tap()
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

    func testNeedsYouGroupsPullRequestsByProject() {
        let app = launch(["-fixture-needs-you-projects"])
        XCTAssertTrue(app.staticTexts["Needs you · 11"].firstMatch.appears(timeout: 5), "8 items, with the one PR swapped for four")
        app.buttons["needsYouSeeAll"].firstMatch.tap()
        let loft = app.buttons["needsYouProject-loft"]
        XCTAssertTrue(loft.appears(timeout: 5))
        XCTAssertTrue(loft.label.contains("3 pull requests"))
        XCTAssertTrue(app.buttons["needsYouProject-api"].exists)
        loft.tap()
        XCTAssertEqual(loft.value as? String, "Collapsed")
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
        XCTAssertTrue(app.buttons["Task: Implement backend endpoints, In progress"].appears(timeout: 5), "the chip on the sent message")
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
