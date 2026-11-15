import XCTest
@testable import LoftKit

final class AgentCommandsTests: XCTestCase {
    private let list = [
        AgentCommand(name: "/clear", description: "Clear the conversation.", tag: "loft"),
        AgentCommand(name: "/loop", description: "Repeat a prompt.", tag: "loft"),
        AgentCommand(name: "/model", description: "Show or switch the model.", tag: "loft"),
        AgentCommand(name: "/doctor", description: "Check the install.", tag: "terminal", reason: "Only in the terminal app."),
        AgentCommand(name: "/review", description: "Review a pull request.", tag: "passthrough"),
        AgentCommand(name: "/ship", description: "Clear the deck and ship.", tag: "passthrough"),
    ]

    func testTheMenuOpensOnALeadingSlashWordOnly() {
        XCTAssertEqual(CommandQuery.query(in: "/"), "")
        XCTAssertEqual(CommandQuery.query(in: "/Rev"), "rev")
        XCTAssertNil(CommandQuery.query(in: "/review 12"))
        XCTAssertNil(CommandQuery.query(in: "see /review"))
        XCTAssertNil(CommandQuery.query(in: "hello"))
    }

    func testNamesThatStartWithTheQueryComeBeforeNamesAndDescriptionsThatHaveIt() {
        XCTAssertEqual(CommandQuery.matches(list, "").count, list.count)
        XCTAssertEqual(CommandQuery.matches(list, "cl").map(\.name), ["/clear", "/ship"], "ship matches by its description")
        XCTAssertEqual(CommandQuery.matches(list, "o").map(\.name), ["/loop", "/model", "/doctor", "/clear"], "names with an o, then the description that has one")
        XCTAssertEqual(CommandQuery.matches(list, "zzz"), [])
    }

    func testPickingLeavesRoomForArguments() {
        XCTAssertEqual(CommandQuery.insert(list[1]), "/loop ")
    }

    func testSendingARoutesLikeTheWeb() {
        func act(_ t: String) -> CommandQuery.Action? { CommandQuery.action(for: t, commands: list) }
        XCTAssertEqual(act("/clear"), .clear)
        XCTAssertEqual(act("/loop 10m check"), .loop("10m check"))
        XCTAssertEqual(act("/model opus"), .info(name: "model", arg: "opus"))
        XCTAssertEqual(act("/review 12"), .chat("/review 12"))
        XCTAssertEqual(act("  /Ship "), .chat("/ship"))
        XCTAssertEqual(act("/doctor"), .terminal("/doctor can't run here. Only in the terminal app."))
        XCTAssertEqual(act("/nope"), .unknown("/nope"))
        XCTAssertNil(act("/usr/bin is odd"))
        XCTAssertNil(act("hello"))
        XCTAssertNil(CommandQuery.action(for: "/clear", commands: []), "no list yet: a plain message")
    }

    func testLoopTakesAnIntervalAndAPrompt() {
        XCTAssertEqual(CommandQuery.parseLoop("10m check the deploy")?.interval, "10m")
        XCTAssertEqual(CommandQuery.parseLoop("2H go")?.prompt, "go")
        XCTAssertNil(CommandQuery.parseLoop("10m"))
        XCTAssertNil(CommandQuery.parseLoop("soon go"))
    }

    func testAnAgentDecodesItsLoopAndSchedules() throws {
        let json = """
        {"id":"a1","name":"Dev","status":"idle",
         "loop":{"prompt":"check","interval_minutes":120,"next_at":"2026-10-12T10:00:00+00:00"},
         "schedules":[{"id":"s1","prompt":"report","every":"weekday","time":"09:30","next_at":"2026-10-13T08:30:00+00:00"},
                      {"id":"s2","prompt":"once","next_at":"2026-10-13T08:30:00Z"}]}
        """
        let a = try JSONCoding.decoder.decode(Agent.self, from: Data(json.utf8))
        XCTAssertEqual(a.loop?.intervalMinutes, 120)
        XCTAssertEqual(a.schedules?.map(\.id), ["s1", "s2"])
        XCTAssertEqual(a.schedules?[0].every, "weekday")
        XCTAssertNil(a.schedules?[1].every)
        let old = try JSONCoding.decoder.decode(Agent.self, from: Data(#"{"id":"a1","name":"Dev","status":"idle"}"#.utf8))
        XCTAssertNil(old.loop)
    }

    func testScheduleRowsAreShortInTheUsersLocale() throws {
        let json = #"{"id":"s1","prompt":"p","every":"weekday","time":"08:00","next_at":"2026-11-16T08:00:00Z"}"#
        let s = try JSONCoding.decoder.decode(AgentSchedule.self, from: Data(json.utf8))
        let utc = TimeZone(identifier: "UTC")!
        let us = Locale(identifier: "en_US"), de = Locale(identifier: "de_DE")
        XCTAssertEqual(s.repeatLabel(locale: us), "Every weekday 8:00 AM".replacingOccurrences(of: " AM", with: "\u{202F}AM"))
        XCTAssertEqual(s.repeatLabel(locale: de), "Every weekday 08:00")
        XCTAssertEqual(AgentSchedule(id: "o", prompt: "p", nextAt: s.nextAt).repeatLabel, "Once")
        XCTAssertEqual(AgentSchedule.nextLabel(s.nextAt, locale: us, timeZone: utc), "Mon Nov 16 8:00\u{202F}AM", "month first in en_US, no comma, no \"at\"")
        let germanNext = AgentSchedule.nextLabel(s.nextAt, locale: de, timeZone: utc)
        XCTAssertTrue(germanNext.hasPrefix("Mo") && germanNext.contains("16") && germanNext.hasSuffix("08:00"), germanNext)
        XCTAssertEqual(AgentSchedule.clock("8:00", locale: us), "8:00\u{202F}AM", "8:00 and 08:00 read the same")
        XCTAssertEqual(AgentSchedule.clock("bad", locale: us), "bad")
    }

    func testNextRunKeepsEachLocalesDayAndMonthOrder() {
        let d = Date(timeIntervalSince1970: 1794816000) // Mon 16 Nov 2026 08:00 UTC
        let utc = TimeZone(identifier: "UTC")!
        let label = { (id: String) in AgentSchedule.nextLabel(d, locale: Locale(identifier: id), timeZone: utc) }
        XCTAssertEqual(label("en_US"), "Mon Nov 16 8:00\u{202F}AM")
        XCTAssertEqual(label("en_GB"), "Mon 16 Nov 08:00")
        let de = label("de_DE")
        XCTAssertTrue(de.hasPrefix("Mo") && de.contains("16. Nov") && de.hasSuffix("08:00"), de)
    }

    func testTheRowDropsTheRepeatTimeWhenItIsTheNextRunsTime() throws {
        let json = #"{"id":"s1","prompt":"p","every":"weekday","time":"08:00","next_at":"2026-11-16T08:00:00Z"}"#
        let s = try JSONCoding.decoder.decode(AgentSchedule.self, from: Data(json.utf8))
        let us = Locale(identifier: "en_US"), utc = TimeZone(identifier: "UTC")!
        XCTAssertEqual(s.rowLabel(locale: us, timeZone: utc), "Every weekday · next Mon Nov 16 8:00\u{202F}AM")
        // The phone in another zone: the times differ, so the repeat time stays.
        let row = s.rowLabel(locale: us, timeZone: TimeZone(identifier: "America/New_York")!)
        XCTAssertTrue(row.hasPrefix("Every weekday 8:00\u{202F}AM · next "), row)
        var once = s; once.every = nil
        XCTAssertTrue(once.rowLabel(locale: us, timeZone: utc).hasPrefix("Once · next "))
    }
}
