import XCTest
@testable import LoftKit

final class FocusSessionTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000_000)

    func testASessionEndsAfterItsLength() {
        let s = FocusSession(minutes: 25, from: start)
        XCTAssertEqual(s.secondsLeft(at: start), 25 * 60)
        XCTAssertFalse(s.isOver(at: start.addingTimeInterval(25 * 60 - 1)))
        XCTAssertTrue(s.isOver(at: start.addingTimeInterval(25 * 60)))
        XCTAssertEqual(s.secondsLeft(at: start.addingTimeInterval(26 * 60)), 0)
    }

    func testTheCountdownRoundsUpPartSeconds() {
        let s = FocusSession(minutes: 25, from: start)
        XCTAssertEqual(s.countdown(at: start), "25:00")
        XCTAssertEqual(s.countdown(at: start.addingTimeInterval(0.4)), "25:00")
        XCTAssertEqual(s.countdown(at: start.addingTimeInterval(1)), "24:59")
        XCTAssertEqual(s.countdown(at: start.addingTimeInterval(25 * 60 + 5)), "0:00")
    }

    func testLongSessionsShowHours() {
        XCTAssertEqual(FocusSession(minutes: 90, from: start).countdown(at: start.addingTimeInterval(1)), "1:29:59")
    }

    func testTheDockLabelIsMinutesThenSeconds() {
        let s = FocusSession(minutes: 25, from: start)
        XCTAssertEqual(s.dockLabel(at: start), "25m")
        XCTAssertEqual(s.dockLabel(at: start.addingTimeInterval(30)), "25m")  // 24:30 left still reads 25m
        XCTAssertEqual(s.dockLabel(at: start.addingTimeInterval(24 * 60)), "1m")
        XCTAssertEqual(s.dockLabel(at: start.addingTimeInterval(24 * 60 + 18)), "42s")
    }

    func testProgressGoesFromZeroToOne() {
        let s = FocusSession(minutes: 50, from: start)
        XCTAssertEqual(s.progress(at: start.addingTimeInterval(-5)), 0)
        XCTAssertEqual(s.progress(at: start.addingTimeInterval(25 * 60)), 0.5, accuracy: 0.0001)
        XCTAssertEqual(s.progress(at: start.addingTimeInterval(60 * 60)), 1)
    }

    func testCustomLengthsAreClamped() {
        XCTAssertEqual(FocusSession(minutes: 0, from: start).secondsLeft(at: start), 60)
        XCTAssertEqual(FocusSession(minutes: 10_000, from: start).secondsLeft(at: start), 480 * 60)
    }

    func testTheSidebarShowsOnlyAgentsAndIssues() {
        XCTAssertEqual(FocusSession.sections, [.agents, .issues])
    }

    func testTheLastDurationIsRemembered() {
        XCTAssertEqual(FocusDuration.remembered(0), 25)  // nothing saved yet
        XCTAssertEqual(FocusDuration.remembered(50), 50)
        XCTAssertEqual(FocusDuration.remembered(37), 37)
        XCTAssertEqual(FocusDuration.remembered(-3), 1)
    }
}

final class FocusAllowListTests: XCTestCase {
    private let loft = "dev.loft.mac"
    private var list: FocusAllowList { .starting(loftBundleId: loft) }

    func testItStartsWithLoftAndTheCodingTools() {
        XCTAssertEqual(list.apps.map(\.name), ["Loft", "Xcode", "Terminal", "iTerm", "Visual Studio Code", "Cursor", "Simulator"])
        XCTAssertTrue(list.allows(loft))
        XCTAssertTrue(list.allows("com.apple.dt.Xcode"))
    }

    func testAppsOffTheListAreHidden() {
        XCTAssertTrue(list.hides(bundleId: "com.apple.Safari", isRegularApp: true, loftBundleId: loft))
        XCTAssertFalse(list.hides(bundleId: "com.apple.Terminal", isRegularApp: true, loftBundleId: loft))
    }

    func testBackgroundAppsAndAppsWithoutABundleIdStay() {
        XCTAssertFalse(list.hides(bundleId: "com.example.menubar", isRegularApp: false, loftBundleId: loft))
        XCTAssertFalse(list.hides(bundleId: nil, isRegularApp: true, loftBundleId: loft))
    }

    func testLoftNeverHidesItself() {
        var l = list
        l.remove(loft)
        XCTAssertFalse(l.hides(bundleId: loft, isRegularApp: true, loftBundleId: loft))
    }

    func testAddingAndRemovingApps() {
        var l = list
        l.add(FocusApp(bundleId: "com.apple.Safari", name: "Safari"))
        l.add(FocusApp(bundleId: "com.apple.Safari", name: "Safari"))
        XCTAssertEqual(l.apps.filter { $0.bundleId == "com.apple.Safari" }.count, 1)
        XCTAssertFalse(l.hides(bundleId: "com.apple.Safari", isRegularApp: true, loftBundleId: loft))
        l.remove("com.apple.dt.Xcode")
        XCTAssertTrue(l.hides(bundleId: "com.apple.dt.Xcode", isRegularApp: true, loftBundleId: loft))
    }

    func testTheListIsSavedAndLoaded() {
        var l = list
        l.remove("com.apple.Terminal")
        XCTAssertEqual(FocusAllowList.load(l.saved, loftBundleId: loft), l)
        // Nothing saved, or something unreadable: the starting list.
        XCTAssertEqual(FocusAllowList.load(nil, loftBundleId: loft), list)
        XCTAssertEqual(FocusAllowList.load(Data("nope".utf8), loftBundleId: loft), list)
        // An emptied list stays empty.
        XCTAssertEqual(FocusAllowList.load(FocusAllowList(apps: []).saved, loftBundleId: loft).apps, [])
    }
}
