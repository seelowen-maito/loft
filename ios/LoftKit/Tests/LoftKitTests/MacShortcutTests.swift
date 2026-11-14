import XCTest
@testable import LoftKit

/// The Mac's recorded shortcuts: how they're stored, what the old fixed list reads as, and which are refused.
final class MacShortcutTests: XCTestCase {
    private let controlCommandSpace = MacShortcut(keyCode: 49, modifiers: MacShortcut.control | MacShortcut.command)

    func testStoredFormRoundTrips() {
        XCTAssertEqual(controlCommandSpace.stored, "control-command-49")
        XCTAssertEqual(MacShortcut(stored: "control-command-49"), controlCommandSpace)
        let f5 = MacShortcut(keyCode: 96, modifiers: 0)
        XCTAssertEqual(f5.stored, "96")
        XCTAssertEqual(MacShortcut.read(f5.stored, default: nil), f5)
    }

    func testOffAndMissing() {
        XCTAssertEqual(MacShortcut.stored(nil), "off")
        XCTAssertNil(MacShortcut.read("off", default: .quickSearchDefault))
        XCTAssertEqual(MacShortcut.read(nil, default: .quickSearchDefault), .quickSearchDefault)
        XCTAssertEqual(MacShortcut.read("hyper-k", default: .voicePanelDefault), .voicePanelDefault)
        XCTAssertNil(MacShortcut(stored: ""))
    }

    func testTheOldListStillReads() {
        let o = MacShortcut.option, s = MacShortcut.shift, c = MacShortcut.command, ct = MacShortcut.control
        XCTAssertEqual(MacShortcut.read("option-space", default: nil), MacShortcut(keyCode: 49, modifiers: o))
        XCTAssertEqual(MacShortcut.read("shift-option-space", default: nil), MacShortcut(keyCode: 49, modifiers: s | o))
        XCTAssertEqual(MacShortcut.read("option-command-space", default: nil), MacShortcut(keyCode: 49, modifiers: o | c))
        XCTAssertEqual(MacShortcut.read("control-option-space", default: nil), MacShortcut(keyCode: 49, modifiers: ct | o))
        XCTAssertNil(MacShortcut.read("off", default: .quickSearchDefault))
        XCTAssertEqual(MacShortcut.read("option-command-space", default: nil)?.label, "⌥⌘Space")
    }

    func testLabelsUseTheMacsOrder() {
        XCTAssertEqual(controlCommandSpace.label, "⌃⌘Space")
        XCTAssertEqual(MacShortcut(keyCode: 40, modifiers: MacShortcut.command | MacShortcut.shift | MacShortcut.option | MacShortcut.control).label, "⌃⌥⇧⌘K")
        XCTAssertEqual(MacShortcut.voicePanelDefault.label, "⌥⇧Space")
        XCTAssertEqual(MacShortcut(keyCode: 122, modifiers: 0).label, "F1")
        XCTAssertEqual(MacShortcut(keyCode: 200, modifiers: MacShortcut.option).label, "⌥Key 200")
    }

    func testAModifierIsNeededUnlessItsAFunctionKey() {
        XCTAssertEqual(MacShortcut(keyCode: 0, modifiers: 0).refusal(taken: []), "A needs ⌘, ⌥ or ⌃, unless it's a function key.")
        XCTAssertNil(MacShortcut(keyCode: 109, modifiers: 0).refusal(taken: []))
        XCTAssertNil(MacShortcut(keyCode: 109, modifiers: MacShortcut.shift).refusal(taken: []))
        XCTAssertNil(MacShortcut(keyCode: 0, modifiers: MacShortcut.control).refusal(taken: []))
    }

    func testShiftAloneIsNotEnough() {
        XCTAssertEqual(MacShortcut(keyCode: 0, modifiers: MacShortcut.shift).refusal(taken: []), "⇧A needs ⌘, ⌥ or ⌃, unless it's a function key.")
        XCTAssertEqual(MacShortcut(keyCode: 18, modifiers: MacShortcut.shift).refusal(taken: []), "⇧1 needs ⌘, ⌥ or ⌃, unless it's a function key.")
        XCTAssertNil(MacShortcut(keyCode: 0, modifiers: MacShortcut.shift | MacShortcut.option).refusal(taken: []))
    }

    func testStandardMenuShortcutsAreRefused() {
        let c = MacShortcut.command
        XCTAssertEqual(MacShortcut(keyCode: 12, modifiers: c).refusal(taken: []), "⌘Q is already Loft's Quit shortcut.")
        XCTAssertEqual(MacShortcut(keyCode: 8, modifiers: c).refusal(taken: []), "⌘C is already Loft's Copy shortcut.")
        XCTAssertEqual(MacShortcut(keyCode: 43, modifiers: c).refusal(taken: []), "⌘, is already Loft's Settings shortcut.")
        XCTAssertEqual(MacShortcut(keyCode: 6, modifiers: MacShortcut.shift | c).refusal(taken: []), "⇧⌘Z is already Loft's Redo shortcut.")
        for code: UInt32 in [13, 4, 46, 9, 7, 6, 0, 3] { XCTAssertNotNil(MacShortcut(keyCode: code, modifiers: c).refusal(taken: []), "\(code)") }
        XCTAssertNil(MacShortcut(keyCode: 12, modifiers: MacShortcut.control | c).refusal(taken: []))
    }

    func testTakenCombinationsAreRefused() {
        XCTAssertEqual(MacShortcut.quickSearchDefault.refusal(taken: [(.quickSearchDefault, "quick search")]),
                       "⌥Space is already Loft's quick search shortcut.")
        XCTAssertEqual(controlCommandSpace.refusal(taken: [(.quickSearchDefault, "quick search")], system: [controlCommandSpace]),
                       "macOS already uses ⌃⌘Space. Record another.")
        XCTAssertNil(controlCommandSpace.refusal(taken: [(.quickSearchDefault, "quick search")], system: []))
    }
}
