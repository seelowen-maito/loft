import XCTest
@testable import LoftKit

/// What the Mac's Talk sheet does as the voice session moves on, and where a denied permission leads (spec #382).
final class TalkHandOffTests: XCTestCase {
    func testTheSheetStaysWhileListeningTidyingOrFailed() {
        for p: VoiceSession.Phase in [.idle, .preparing, .listening, .paused, .cleaning, .failed("x")] {
            XCTAssertEqual(TalkHandOff.outcome(phase: p, autoSendAt: nil, listened: true), .stay, "\(p)")
        }
    }

    func testTidiedTextGoesToTheChatBox() {
        XCTAssertEqual(TalkHandOff.outcome(phase: .review, autoSendAt: nil, listened: true), .textToChat)
        XCTAssertEqual(TalkHandOff.outcome(phase: .review, autoSendAt: Date(), listened: true), .stay, "a countdown send isn't ours to take")
    }

    /// Send, or a spoken "send", takes the sheet to the chat; it no longer sits on "Start talking…".
    func testASentMessageTakesTheSheetToTheChat() {
        for p: VoiceSession.Phase in [.sending, .speaking, .replied] {
            XCTAssertEqual(TalkHandOff.outcome(phase: p, autoSendAt: nil, listened: true), .sentToChat, "\(p)")
        }
    }

    /// Talk opened again while the last one's reply is still being read (or after): not a send of this sheet.
    func testASheetOpenedOnAnOldSendStaysOpen() {
        for p: VoiceSession.Phase in [.sending, .speaking, .replied, .review] {
            XCTAssertEqual(TalkHandOff.outcome(phase: p, autoSendAt: nil, listened: false), .stay, "\(p)")
        }
    }

    func testClosingKeepsTheReplyBeingReadButDropsADictation() {
        XCTAssertFalse(TalkHandOff.discardsOnClose(.speaking))
        XCTAssertFalse(TalkHandOff.discardsOnClose(.sending))
        XCTAssertTrue(TalkHandOff.discardsOnClose(.listening))
        XCTAssertTrue(TalkHandOff.discardsOnClose(.review))
    }

    func testTheRightPrivacyPaneOpens() {
        XCTAssertEqual(TalkHandOff.privacyPane(for: "Microphone access is off for Loft."), "Privacy_Microphone")
        XCTAssertEqual(TalkHandOff.privacyPane(for: "Speech recognition is off for Loft."), "Privacy_SpeechRecognition")
    }
}
