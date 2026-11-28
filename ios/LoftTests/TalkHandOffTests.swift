import XCTest
@testable import LoftKit

/// What the Mac's Talk sheet does as the voice session moves on, and where a denied permission leads (spec #382).
final class TalkHandOffTests: XCTestCase {
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
