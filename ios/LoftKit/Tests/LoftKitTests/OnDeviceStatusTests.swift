import XCTest
@testable import LoftKit

/// What the Mac's Settings › Apple Intelligence says about the model (#469).
final class OnDeviceStatusTests: XCTestCase {
    func testGettingReadySaysWhereToCheckOnThisMac() {
        let s = OnDeviceModel.status(unavailableReason: "modelNotReady", languageSupported: true)
        XCTAssertEqual(s.text, "Getting ready (downloading)")
        XCTAssertEqual(s.hint, "macOS downloads the model itself. Check System Settings › Apple Intelligence & Siri; this turns on by itself once it's ready.")
    }

    func testAnUnsupportedLanguagePointsAtTheLanguageSetting() {
        let s = OnDeviceModel.status(unavailableReason: "modelNotReady", languageSupported: false, language: "Icelandic")
        XCTAssertEqual(s.hint, "Check the language in System Settings › Apple Intelligence & Siri.")
    }

    func testStatusesThatSayWhereToLookHaveNoHint() {
        XCTAssertNil(OnDeviceModel.Status.available.hint)
        XCTAssertNil(OnDeviceModel.Status.appleIntelligenceOff.hint)
        XCTAssertNil(OnDeviceModel.Status.deviceNotEligible.hint)
    }

    func testRawAvailabilityShowsTheSystemsOwnAnswer() {
        // On a Mac without the model it names the reason; with it, "available". Either way the locale is there.
        XCTAssertTrue(OnDeviceModel.rawAvailability.contains("supported:"), OnDeviceModel.rawAvailability)
    }
}
