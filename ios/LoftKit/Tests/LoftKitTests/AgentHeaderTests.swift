import XCTest
@testable import LoftKit

/// The Mac chat header and agents list: what they say, as the web's (the actions are in LoftTests' AgentHeaderActionsTests).
final class AgentHeaderTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    private func agent(interval: Int? = 30, next: TimeInterval? = 12 * 60, quiet: [QuietWindow]? = nil, session: String? = "s1",
                       ctx: Int? = 84_000) -> Agent {
        var a = Agent(id: "qa", name: "QA", status: "idle")
        a.intervalMinutes = interval
        a.nextRunAt = next.map { now.addingTimeInterval($0) }
        a.quietHours = quiet
        a.sessionId = session
        a.contextTokens = ctx
        return a
    }

    func testScheduleReadsLikeTheWeb() {
        XCTAssertEqual(AgentHeader.schedule(agent(), now: now), "every 30m · next in 12m")
        XCTAssertEqual(AgentHeader.schedule(agent(interval: 120, next: -3 * 3600), now: now), "every 2h · next: any moment")
        XCTAssertEqual(AgentHeader.schedule(agent(next: 0), now: now), "every 30m · next: any moment")
        XCTAssertEqual(AgentHeader.schedule(agent(next: 30), now: now), "every 30m · next: any moment")
        XCTAssertEqual(AgentHeader.schedule(agent(interval: 1440, next: nil), now: now), "every 1d")
        XCTAssertEqual(AgentHeader.schedule(agent(interval: 0), now: now), "manual")
        XCTAssertEqual(AgentHeader.schedule(agent(interval: nil), now: now), "manual")
    }

    func testWorkingHoursMarkTheScheduleCustomAndSayWhenItWorks() {
        let a = agent(quiet: [QuietWindow(days: [0, 1, 2, 3, 4], start: "22:00", end: "07:00"), QuietWindow(days: [5, 6], start: "00:00", end: "00:00")])
        XCTAssertEqual(AgentHeader.schedule(a, now: now), "every 30m · custom · next in 12m")
        XCTAssertEqual(AgentHeader.quietHours(a), "Works Mon–Fri 22:00–07:00; Sat, Sun all day")
        XCTAssertNil(AgentHeader.quietHours(agent()))
        XCTAssertEqual(AgentHeader.quietHours(agent(quiet: [QuietWindow(days: [], start: "00:00", end: "00:00")])), "Never wakes on its own")
        XCTAssertEqual(AgentHeader.days([0, 2, 3, 4]), "Mon, Wed–Fri")
    }

    func testContextSize() {
        XCTAssertEqual(AgentHeader.context(agent()), "context 84k")
        XCTAssertEqual(AgentHeader.context(agent(ctx: nil)), "context —")
        XCTAssertEqual(AgentHeader.context(agent(session: nil, ctx: nil)), "no session")
        // An older server sends no Session id; a known size still means there is one.
        XCTAssertEqual(AgentHeader.context(agent(session: nil, ctx: 4200)), "context 4.2k")
        XCTAssertEqual(AgentHeader.tokens(950), "950")
        XCTAssertTrue(ContextWindow.warns(84_000))
        XCTAssertFalse(ContextWindow.warns(79_999))
    }

    func testTheListShowsTheSizeOnlyWithASession() {
        XCTAssertEqual(AgentHeader.listedContext(agent()), 84_000)
        XCTAssertNil(AgentHeader.listedContext(agent(session: nil, ctx: 84_000)))
        XCTAssertNil(AgentHeader.listedContext(agent(ctx: nil)))
    }
}
