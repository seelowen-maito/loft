import AppKit
import SwiftUI
import XCTest
@testable import LoftKit

/// The Usage widget (#489): full, partial and offline readings, and its states drawn on the Mac, dark and light.
/// `WIDGET_SHOTS=<folder>` also writes each drawing there as a PNG, for screenshots.
final class UsageWidgetTests: XCTestCase {
    struct Down: Error {}
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    private func defaults() throws -> UserDefaults {
        let d = try XCTUnwrap(UserDefaults(suiteName: "UsageWidgetTests"))
        d.removePersistentDomain(forName: "UsageWidgetTests")
        return d
    }

    func testAFullReadingShowsTheSessionWeekMemoryAndTemperature() async throws {
        let api = FixtureAPI()
        let s = await UsageWidgetSnapshot.load(usage: api.usage, system: api.system, weekSpend: { 12.4 }, now: now)
        XCTAssertEqual(s.phase, .ready)
        XCTAssertEqual(UsageWidgetSnapshot.percentText(s.session), "42%")
        XCTAssertEqual(UsageWidgetSnapshot.percentText(s.week), "18%")
        XCTAssertNotNil(s.session?.resetsAt)
        XCTAssertEqual(s.memoryText, "21 of 32 GB")
        XCTAssertEqual(s.tempText, "64°")
        XCTAssertEqual(s.tempLevel, "nominal")
        XCTAssertEqual(s.spendText, "$12.40")
        XCTAssertEqual(s.takenAt, now)
    }

    func testTheMacsReadingDecodesItsMemory() throws {
        let json = #"{"cpu_temp_c":91.2,"level":"warm","mem_used_bytes":30064771072,"mem_total_bytes":34359738368,"mem_level":"high","sampled_at":null}"#
        let r = try JSONCoding.decoder.decode(SystemReading.self, from: Data(json.utf8))
        let s = UsageWidgetSnapshot(usage: nil, system: r, weekSpend: nil, at: now)
        XCTAssertEqual(s.memoryText, "28 of 32 GB")
        XCTAssertEqual(s.memLevel, "high")
        XCTAssertEqual(s.tempLevel, "warm")
    }

    /// No login, no sensor, an older Loft without `/system` or memory: a dash, never 0.
    func testWhatLoftDoesntHaveIsADash() async throws {
        let s = await UsageWidgetSnapshot.load(usage: { PlanUsage(error: "not logged in", windows: []) }, system: { throw Down() },
                                               weekSpend: { throw Down() }, now: now)
        XCTAssertEqual(s.phase, .ready, "one of the two answering is enough")
        XCTAssertNil(s.session)
        XCTAssertEqual(UsageWidgetSnapshot.percentText(s.session), "—")
        XCTAssertEqual(s.memoryText, "—")
        XCTAssertEqual(s.tempText, "—")
        XCTAssertNil(s.spendText)

        let noSensor = UsageWidgetSnapshot(usage: nil, system: SystemReading(cpuTempC: nil, level: nil, sampledAt: nil), weekSpend: nil, at: now)
        XCTAssertEqual(noSensor.tempText, "—")
        XCTAssertEqual(noSensor.memoryText, "—", "an older Loft without memory")
    }

    func testOfflineShowsTheLastReadingWithItsTime() async throws {
        let last = try defaults()
        let api = FixtureAPI()
        let live = await UsageWidgetSnapshot.load(usage: api.usage, system: api.system, weekSpend: { 3 }, last: last, now: now)
        let offline = await UsageWidgetSnapshot.load(usage: { throw Down() }, system: { throw Down() }, weekSpend: { throw Down() },
                                                     last: last, now: now.addingTimeInterval(3600))
        XCTAssertEqual(offline.phase, .unreachable)
        XCTAssertEqual(offline.takenAt, now, "the reading's own time, not now")
        XCTAssertEqual(offline.session, live.session)
        XCTAssertEqual(offline.memoryText, live.memoryText)

        let never = await UsageWidgetSnapshot.load(usage: { throw Down() }, system: { throw Down() }, weekSpend: { throw Down() },
                                                   last: try defaults(), now: now)
        XCTAssertEqual(never.phase, .unreachable)
        XCTAssertNil(never.takenAt, "no reading yet: the plain offline note")
    }

    func testTheResetTimeIsShort() {
        XCTAssertEqual(UsageWidgetSnapshot.resetText(now.addingTimeInterval(2 * 3600 + 14 * 60 + 30), now: now), "Resets in 2h 14m")
        XCTAssertEqual(UsageWidgetSnapshot.resetText(now.addingTimeInterval(3 * 86400 + 23 * 3600), now: now), "Resets in 3d 23h")
        XCTAssertEqual(UsageWidgetSnapshot.resetText(now.addingTimeInterval(25 * 60), now: now), "Resets in 25m")
        XCTAssertEqual(UsageWidgetSnapshot.resetText(now.addingTimeInterval(30), now: now), "Resets in under a minute")
        XCTAssertEqual(UsageWidgetSnapshot.resetText(now.addingTimeInterval(-3 * 3600), now: now), "Reset has passed", "after a long offline stretch")
        XCTAssertEqual(UsageWidgetSnapshot.resetText(now, now: now), "Reset has passed")
        XCTAssertEqual(UsageWidgetSnapshot.resetText(nil, now: now), "Reset time unknown")
    }

    /// `/usage` failing while `/system` answers keeps the last plan numbers, also for a later offline state.
    func testOnlyUsageFailingKeepsTheLastPlanNumbers() async throws {
        let last = try defaults()
        let api = FixtureAPI()
        let live = await UsageWidgetSnapshot.load(usage: api.usage, system: api.system, weekSpend: { 3 }, last: last, now: now)
        let partial = await UsageWidgetSnapshot.load(usage: { throw Down() }, system: api.system, weekSpend: { 3 }, last: last,
                                                     now: now.addingTimeInterval(600))
        XCTAssertEqual(partial.phase, .ready)
        XCTAssertEqual(partial.session, live.session)
        XCTAssertEqual(partial.week, live.week)
        let offline = await UsageWidgetSnapshot.load(usage: { throw Down() }, system: { throw Down() }, weekSpend: { throw Down() },
                                                     last: last, now: now.addingTimeInterval(1200))
        XCTAssertEqual(UsageWidgetSnapshot.percentText(offline.session), "42%", "not a dash")

        let loggedOut = await UsageWidgetSnapshot.load(usage: { PlanUsage(error: "not logged in", windows: []) }, system: api.system,
                                                       weekSpend: { 3 }, last: last, now: now)
        XCTAssertNil(loggedOut.session, "Loft answering that it can't read the plan is a dash, not the old number")
    }

    func testATapOpensTheOverview() {
        XCTAssertEqual(DeepLink(url: DeepLink.overview.url), .overview)
    }

    @MainActor
    func testTheAppReloadsTheWidgetOnlyWhenThePlansNumbersChange() async {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        var reloads = 0
        store.reloadUsageWidget = { reloads += 1 }
        await store.refreshStats()
        XCTAssertEqual(reloads, 1)
        await store.refreshStats()
        XCTAssertEqual(reloads, 1, "the same numbers (the fixture's reset times move) don't reload it")
    }

    @MainActor
    func testEveryStateDrawsInDarkAndLight() throws {
        var hot = UsageWidgetSnapshot.sample
        hot.session = .init(percent: 93, resetsAt: .now.addingTimeInterval(1500))
        hot.week = .init(percent: 78, resetsAt: .now.addingTimeInterval(2 * 86400))
        hot.memUsedBytes = 29 << 30
        hot.memLevel = "high"
        hot.cpuTempC = 96
        hot.tempLevel = "hot"
        var offline = UsageWidgetSnapshot.sample
        offline.phase = .unreachable
        offline.takenAt = .now.addingTimeInterval(-3 * 3600)
        var partial = UsageWidgetSnapshot(usage: nil, system: SystemReading(cpuTempC: nil, level: nil, sampledAt: nil), weekSpend: nil, at: .now)
        partial.phase = .ready
        let states: [(String, UsageWidgetSnapshot)] = [
            ("ready", .sample), ("hot", hot), ("offline", offline), ("dashes", partial),
            ("never-read", UsageWidgetSnapshot(phase: .unreachable)), ("not-connected", UsageWidgetSnapshot(phase: .notConnected)),
        ]
        var seen: [Data] = []
        for (name, snapshot) in states {
            for (size, width) in [(UsageWidgetView.Size.small, 158.0), (.medium, 338.0)] {
                for scheme in [ColorScheme.dark, .light] {
                    let view = UsageWidgetView(snapshot, size: size).padding(16).frame(width: width, height: 158)
                        .background(scheme == .dark ? Color(white: 0.11) : .white).environment(\.colorScheme, scheme)
                    let renderer = ImageRenderer(content: view)
                    renderer.scale = 2
                    let tiff = try XCTUnwrap(renderer.nsImage?.tiffRepresentation, name)
                    let png = try XCTUnwrap(NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]))
                    seen.append(png)
                    if let dir = ProcessInfo.processInfo.environment["WIDGET_SHOTS"] {
                        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                        let file = "mac-usage-\(name)-\(size == .small ? "small" : "medium")-\(scheme == .dark ? "dark" : "light").png"
                        try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent(file))
                    }
                }
            }
        }
        XCTAssertEqual(Set(seen).count, seen.count, "no two states draw the same")
    }
}
