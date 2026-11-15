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
}
