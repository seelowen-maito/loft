import AppKit
import SwiftUI
import XCTest
@testable import LoftKit

/// The shared palette follows the web (frontend/src/index.css), not macOS blue.
final class ThemeTests: XCTestCase {
    private func hex(_ color: Color, _ appearance: NSAppearance.Name) -> String {
        var out = ""
        NSAppearance(named: appearance)!.performAsCurrentDrawingAppearance {
            let c = NSColor(color).usingColorSpace(.sRGB)!
            out = String(format: "%02X%02X%02X", Int((c.redComponent * 255).rounded()),
                         Int((c.greenComponent * 255).rounded()), Int((c.blueComponent * 255).rounded()))
        }
        return out
    }

    func testAccentIsTheWebsLimeOnDarkAndDeeperGreenOnLight() {
        XCTAssertEqual(hex(Theme.accent, .darkAqua), "C8F26D")
        XCTAssertEqual(hex(Theme.accent, .aqua), "4F6A26")
    }

    func testAccentAssetsMatchTheTheme() throws {
        // The apps' AccentColor assets tint system controls; they must hold the same two values.
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("../../..").standardized
        for app in ["Loft/Resources", "LoftMac"] {
            let url = root.appendingPathComponent("\(app)/Assets.xcassets/AccentColor.colorset/Contents.json")
            let json = try String(contentsOf: url, encoding: .utf8)
            XCTAssertTrue(json.contains(#""red":"0x4F","green":"0x6A","blue":"0x26""#), app)
            XCTAssertTrue(json.contains(#""red":"0xC8","green":"0xF2","blue":"0x6D""#), app)
        }
    }

    func testStatusColoursMatchTheIPhoneAndTheWeb() {
        XCTAssertEqual(AgentState.working.color, Theme.working)
        XCTAssertEqual(AgentState.waitingApproval.color, Theme.attention)
        XCTAssertEqual(AgentState.failed.color, Theme.failure)
        XCTAssertEqual(AgentState.idle.color, Theme.neutral)
        // The Mac takes the web's good, warn and bad (the iPhone keeps 4CD185, FFB840, FF6B66).
        XCTAssertEqual(hex(Theme.working, .aqua), "7FD99A")
        XCTAssertEqual(hex(Theme.attention, .aqua), "F2C46D")
        XCTAssertEqual(hex(Theme.failure, .aqua), "F27D6D")
    }

    func testMacSurfacesAndTextAreTheWebsInkAndFgOnDark() {
        XCTAssertEqual(hex(WebTheme.canvas, .darkAqua), "08090A")
        XCTAssertEqual(hex(WebTheme.surface, .darkAqua), "0D0E10")
        XCTAssertEqual(hex(WebTheme.field, .darkAqua), "121316")
        XCTAssertEqual(hex(WebTheme.raised, .darkAqua), "17191C")
        XCTAssertEqual(hex(WebTheme.fg, .darkAqua), "ECEAE4")
        XCTAssertEqual(hex(WebTheme.fg2, .darkAqua), "A9A8A3")
        XCTAssertEqual(hex(WebTheme.fg3, .darkAqua), "6E6E6B")
    }

    func testLightModeInvertsTheSurfacesAndText() {
        XCTAssertEqual(hex(WebTheme.canvas, .aqua), "F6F5F1")
        XCTAssertEqual(hex(WebTheme.fg, .aqua), "17191C")
    }

    func testAppearanceIsDarkByDefaultAndKeepsTheChoice() {
        XCTAssertEqual(AppearanceChoice(stored: nil), .dark)
        XCTAssertEqual(AppearanceChoice(stored: "nonsense"), .dark)
        XCTAssertEqual(AppearanceChoice(stored: "light"), .light)
        XCTAssertEqual(AppearanceChoice(stored: "system").nsAppearance, nil)
        XCTAssertEqual(AppearanceChoice.dark.nsAppearance?.name, .darkAqua)
        XCTAssertEqual(AppearanceChoice.allCases.map(\.label), ["Dark", "Light", "System"])
        // Saved under the key the iPhone app uses, registered as dark.
        let defaults = UserDefaults(suiteName: "ThemeTests")!
        defaults.removePersistentDomain(forName: "ThemeTests")
        defaults.register(defaults: [Prefs.appearance: "dark"])
        XCTAssertEqual(AppearanceChoice(stored: defaults.string(forKey: Prefs.appearance)), .dark)
        defaults.set(AppearanceChoice.light.rawValue, forKey: Prefs.appearance)
        XCTAssertEqual(AppearanceChoice(stored: defaults.string(forKey: Prefs.appearance)), .light)
    }

    /// WCAG contrast of `color` against white, in light mode.
    private func contrastOnWhite(_ color: Color) -> Double {
        var l = 0.0
        NSAppearance(named: .aqua)!.performAsCurrentDrawingAppearance {
            let c = NSColor(color).usingColorSpace(.sRGB)!
            let lin = { (v: CGFloat) -> Double in let v = Double(v); return v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
            l = 0.2126 * lin(c.redComponent) + 0.7152 * lin(c.greenComponent) + 0.0722 * lin(c.blueComponent)
        }
        return 1.05 / (l + 0.05)
    }

    func testWidgetAmberTextIsReadableOnWhiteAndUnchangedInDark() {
        for amber in [UsageTone.usage(80), UsageTone.level("warm"), WidgetTheme.attention] {
            XCTAssertGreaterThanOrEqual(contrastOnWhite(amber), 4.5)
            // Dark keeps the bright amber: on the Mac that is the web's `warn` (#456).
            XCTAssertEqual(hex(amber, .darkAqua), hex(Theme.attention, .darkAqua))
        }
        XCTAssertLessThan(contrastOnWhite(Theme.attention), 4.5, "the bright amber is why")
    }

    func testWidgetRedTextIsReadableOnWhiteAndUnchangedInDark() {
        for red in [UsageTone.usage(95), UsageTone.level("hot"), UsageTone.level("critical"), WidgetTheme.failure] {
            XCTAssertGreaterThanOrEqual(contrastOnWhite(red), 4.5)
            // Dark keeps the bright red: on the Mac that is the web's `bad` (#456).
            XCTAssertEqual(hex(red, .darkAqua), hex(Theme.failure, .darkAqua))
        }
        XCTAssertLessThan(contrastOnWhite(Theme.failure), 4.5, "the bright red is why")
    }
}
