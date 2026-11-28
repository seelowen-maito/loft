import XCTest
@testable import Loft
@testable import LoftKit

/// The wrapper's fallbacks, without the model (these run anywhere; the model's quality is measured by ios/Evaluations).
final class OnDeviceModelTests: XCTestCase {
    func testUnavailableFallsBackWithoutCallingTheModel() async {
        let called = Flag()
        let o = await OnDeviceModel.run("test", available: false, fallback: { "fallback" }) {
            await called.set()
            return "model"
        }
        XCTAssertEqual(o.value, "fallback")
        XCTAssertFalse(o.usedModel)
        let wasCalled = await called.value
        XCTAssertFalse(wasCalled)
    }

    func testTheModelsAnswerIsUsedWhenItComesInTime() async {
        let o = await OnDeviceModel.run("test", timeout: .seconds(1), available: true, fallback: { "fallback" }) { "model" }
        XCTAssertEqual(o.value, "model")
        XCTAssertTrue(o.usedModel)
    }

    func testASlowModelFallsBackOnTime() async {
        let start = Date()
        let o = await OnDeviceModel.run("test", timeout: .milliseconds(200), available: true, fallback: { "fallback" }) {
            try await Task.sleep(for: .seconds(5))
            return "model"
        }
        XCTAssertEqual(o.value, "fallback")
        XCTAssertLessThan(Date().timeIntervalSince(start), 1.5, "the voice path never waits for a slow model")
        if case .fallback(_, let why) = o { XCTAssertTrue(why.contains("longer")) } else { XCTFail() }
    }

    func testAnErrorFallsBack() async {
        struct Boom: Error {}
        let o = await OnDeviceModel.run("test", available: true, fallback: { 1 }) { throw Boom() }
        XCTAssertEqual(o.value, 1)
        XCTAssertFalse(o.usedModel)
    }

    func testStatusWording() {
        XCTAssertEqual(OnDeviceModel.Status.available.text, "Available")
        XCTAssertTrue(OnDeviceModel.Status.appleIntelligenceOff.text.contains("Turned off"))
        XCTAssertFalse(OnDeviceModel.Status.deviceNotEligible.isAvailable)
        #if !os(macOS)
        XCTAssertEqual(OnDeviceModel.Status.needsNewerSystem.text, "Needs iOS 26 or later")
        #endif
    }

    /// The Mac features (summaries, chat replies, "Send to…?") are the shared ones behind Settings' switches: switched
    /// off (as they are while the model can't run) they give nothing, and Loft works as before. The real model's
    /// answers need the Mac itself (listed in the spec PR).
    @MainActor
    func testTheMacFeaturesGiveNothingWhenTheirSwitchIsOff() async {
        let keys = [Prefs.aiNeedsYou, Prefs.aiChatReplies, Prefs.aiRoute]
        let saved = keys.map { UserDefaults.standard.object(forKey: $0) }
        defer { zip(keys, saved).forEach { UserDefaults.standard.set($1, forKey: $0) } }
        keys.forEach { UserDefaults.standard.set(false, forKey: $0) }
        let item = NeedsYouItem(id: "q", type: "question", urgency: 2, agent: nil, taskId: nil, title: "Merge?", reason: "", text: "Shall I merge it now?",
                                createdAt: nil, ageSeconds: nil, links: nil, actions: [], project: nil, pr: nil)
        let assist = await OnDeviceFeatures.needsYouAssist(item)
        XCTAssertNil(assist)
        let replies = await OnDeviceFeatures.chatReplies(to: "Shall I merge it now?")
        XCTAssertTrue(replies.isEmpty)
        if #available(iOS 26.0, macOS 26.0, *) {
            let guess = await OnDeviceFeatures.routeGuess(for: "fix the login", agents: [], orchestratorId: nil)
            XCTAssertNil(guess)
        }
    }

    private func status(_ reason: String?, languageSupported: Bool = true) -> OnDeviceModel.Status {
        OnDeviceModel.status(unavailableReason: reason, languageSupported: languageSupported, language: "Icelandic")
    }

    func testKnownReasonsMapToTheirOwnWords() {
        XCTAssertEqual(status(nil), .available)
        XCTAssertEqual(status("appleIntelligenceNotEnabled"), .appleIntelligenceOff)
        XCTAssertEqual(status("deviceNotEligible"), .deviceNotEligible)
        XCTAssertEqual(status("modelNotReady"), .modelNotReady)
        XCTAssertEqual(status("modelNotReady").text, "Getting ready (downloading)")
    }

    func testAnUnknownReasonIsShownAsTheSystemWroteItNeverAsADownload() {
        let s = status("someNewReason")
        XCTAssertEqual(s, .unavailable("someNewReason"))
        XCTAssertEqual(s.text, "Unavailable (someNewReason)")
        XCTAssertFalse(s.text.contains("downloading"))
        XCTAssertFalse(s.isAvailable)
    }

    func testAnUnsupportedLanguageSaysSoInsteadOfDownloading() {
        XCTAssertEqual(status("modelNotReady", languageSupported: false), .languageNotSupported("Icelandic"))
        XCTAssertEqual(status("modelNotReady", languageSupported: false).text, "Not available in Icelandic")
        // A clearer reason from the system still wins, and an available model stays available.
        XCTAssertEqual(status("appleIntelligenceNotEnabled", languageSupported: false), .appleIntelligenceOff)
        XCTAssertEqual(status(nil, languageSupported: false), .available)
    }
}

private actor Flag {
    private(set) var value = false
    func set() { value = true }
}
