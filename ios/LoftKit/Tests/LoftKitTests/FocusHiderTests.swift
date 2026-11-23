import XCTest
@testable import LoftKit

final class FocusHiderTests: XCTestCase {
    private final class FakeApp: FocusRunningApp {
        let bundleIdentifier: String?
        let isRegularApp: Bool
        var isHidden: Bool
        var isTerminated = false

        init(_ bundleId: String?, regular: Bool = true, hidden: Bool = false) {
            bundleIdentifier = bundleId
            isRegularApp = regular
            isHidden = hidden
        }

        func hide() -> Bool { isHidden = true; return false }  // false, like the sandboxed app's
        func unhide() -> Bool { isHidden = false; return false }
    }

    private let allowList = FocusAllowList(apps: [FocusApp(bundleId: "com.apple.dt.Xcode", name: "Xcode")])

    private func hide(_ apps: [FakeApp], in hider: inout FocusHider<FakeApp>, allowList: FocusAllowList? = nil) {
        for app in apps { hider.hide(app, unless: allowList ?? self.allowList, loftBundleId: "loft") }
    }

    func testASessionHidesOnlyAppsOffTheList() {
        let safari = FakeApp("com.apple.Safari"), xcode = FakeApp("com.apple.dt.Xcode"), loft = FakeApp("loft")
        let agent = FakeApp("com.example.agent", regular: false), noId = FakeApp(nil)
        var hider = FocusHider<FakeApp>()
        hide([safari, xcode, loft, agent, noId], in: &hider)
        XCTAssertTrue(safari.isHidden)
        XCTAssertFalse(xcode.isHidden || loft.isHidden || agent.isHidden || noId.isHidden)
        XCTAssertEqual(hider.bundleIds, ["com.apple.Safari"])
    }

    func testEndingShowsWhatItHidButNotWhatTheUserHad() {
        let safari = FakeApp("com.apple.Safari"), mail = FakeApp("com.apple.mail", hidden: true)
        var hider = FocusHider<FakeApp>()
        hide([safari, mail], in: &hider)
        hide([safari], in: &hider)  // it came forward and was hidden again: still listed once
        XCTAssertEqual(hider.bundleIds, ["com.apple.Safari"])
        hider.unhideAll()
        XCTAssertFalse(safari.isHidden)
        XCTAssertTrue(mail.isHidden)
        XCTAssertTrue(hider.hidden.isEmpty)
    }

    func testAQuitAppIsLeftAlone() {
        let safari = FakeApp("com.apple.Safari")
        var hider = FocusHider<FakeApp>()
        hide([safari], in: &hider)
        safari.isTerminated = true
        hider.unhideAll()
        XCTAssertTrue(safari.isHidden)
    }

    func testAllowingAnAppShowsItAtOnce() {
        let safari = FakeApp("com.apple.Safari"), notes = FakeApp("com.apple.Notes")
        var hider = FocusHider<FakeApp>()
        hide([safari, notes], in: &hider)
        hider.unhide(bundleId: "com.apple.Safari")
        XCTAssertFalse(safari.isHidden)
        XCTAssertTrue(notes.isHidden)
        XCTAssertEqual(hider.bundleIds, ["com.apple.Notes"])
    }

    func testRemovingAnAppFromTheListHidesItAtOnce() {
        let xcode = FakeApp("com.apple.dt.Xcode")
        var hider = FocusHider<FakeApp>()
        hide([xcode], in: &hider)
        XCTAssertFalse(xcode.isHidden)
        var list = allowList
        list.remove("com.apple.dt.Xcode")
        hide([xcode], in: &hider, allowList: list)
        XCTAssertTrue(xcode.isHidden)
        XCTAssertEqual(hider.bundleIds, ["com.apple.dt.Xcode"])
    }

    func testALaunchAfterACrashShowsTheSavedApps() {
        let safari = FakeApp("com.apple.Safari", hidden: true), mail = FakeApp("com.apple.mail", hidden: true)
        FocusHider.unhide(saved: ["com.apple.Safari"], in: [safari, mail])
        XCTAssertFalse(safari.isHidden)
        XCTAssertTrue(mail.isHidden)
    }

    func testDuringASessionOnlyAgentsAndIssuesOpen() {
        XCTAssertEqual(FocusSession.shown(.overview, focusing: true), .agents)
        XCTAssertEqual(FocusSession.shown(.needsYou, focusing: true), .agents)
        XCTAssertEqual(FocusSession.shown(.issues, focusing: true), .issues)
        XCTAssertEqual(FocusSession.shown(.overview, focusing: false), .overview)
    }
}
