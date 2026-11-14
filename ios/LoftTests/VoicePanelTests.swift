import XCTest
@testable import LoftKit

/// The Mac's floating voice panel: its steps as the voice session moves on, who it sends to, and "Send to …?".
final class VoicePanelTests: XCTestCase {
    private let agents = [Agent(id: "dev", name: "Loft mobile dev", status: "idle"), Agent(id: "qa", name: "Loft Mobile QA", status: "idle"),
                          Agent(id: "orch", name: "Orchestrator", status: "idle")]

    private func step(_ p: VoiceSession.Phase, listened: Bool = true, permission: Bool = false) -> VoicePanel.Step {
        VoicePanel.step(phase: p, needsPermission: permission, listened: listened)
    }

    func testListeningTidyingReadySent() {
        XCTAssertEqual(step(.preparing), .starting)
        XCTAssertEqual(step(.listening), .listening)
        XCTAssertEqual(step(.paused), .listening)
        XCTAssertEqual(step(.cleaning), .tidying)
        XCTAssertEqual(step(.review), .ready)
        for p: VoiceSession.Phase in [.sending, .speaking, .replied] { XCTAssertEqual(step(p), .sent, "\(p)") }
    }

    /// Opened while the last Talk's reply is still on its way or being read: not this panel's send.
    func testAnOldSendIsNotThisPanels() {
        for p: VoiceSession.Phase in [.idle, .review, .sending, .speaking, .replied] {
            XCTAssertEqual(step(p, listened: false), .starting, "\(p)")
        }
    }

    func testErrorsKeepTheirReason() {
        XCTAssertEqual(step(.failed("Microphone access is off for Loft."), permission: true),
                       .error("Microphone access is off for Loft.", needsPermission: true))
        XCTAssertEqual(step(.failed("The Mac can't be reached")), .error("The Mac can't be reached", needsPermission: false))
        guard case .error(let why, false) = step(.idle) else { return XCTFail("nothing heard is an error") }
        XCTAssertTrue(why.contains("Nothing was heard"))
    }

    func testItSendsToTheOpenChatFirst() {
        XCTAssertEqual(VoicePanel.recipient(openChat: "qa", last: "dev", agents: agents, orchestratorId: "orch"), "qa")
        XCTAssertEqual(VoicePanel.recipient(openChat: nil, last: "dev", agents: agents, orchestratorId: "orch"), "dev")
        XCTAssertEqual(VoicePanel.recipient(openChat: nil, last: nil, agents: agents, orchestratorId: "orch"), "orch")
    }

    func testAgentsThatAreGoneAreSkipped() {
        XCTAssertEqual(VoicePanel.recipient(openChat: "gone", last: "gone too", agents: agents, orchestratorId: "orch"), "orch")
        XCTAssertEqual(VoicePanel.recipient(openChat: nil, last: nil, agents: agents, orchestratorId: nil), "dev", "first by name")
        XCTAssertNil(VoicePanel.recipient(openChat: "dev", last: nil, agents: [], orchestratorId: nil))
    }

    func testSendToIsOfferedOnlyForSomeoneElse() {
        XCTAssertEqual(VoicePanel.suggestion(guess: agents[1], current: "dev")?.id, "qa")
        XCTAssertNil(VoicePanel.suggestion(guess: agents[1], current: "qa"))
        XCTAssertNil(VoicePanel.suggestion(guess: nil, current: "qa"))
    }

    /// Send while listening goes straight out, unless the on-device model thinks someone else should get it.
    func testSendWaitsOnlyForAnotherRecipient() {
        XCTAssertTrue(VoicePanel.sendsAtOnce(guess: nil, current: "dev"))
        XCTAssertTrue(VoicePanel.sendsAtOnce(guess: agents[0], current: "dev"))
        XCTAssertFalse(VoicePanel.sendsAtOnce(guess: agents[1], current: "dev"))
    }
}
