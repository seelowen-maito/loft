import XCTest
@testable import Loft

final class MentionQueryTests: XCTestCase {
    private let agents = [Agent(id: "be", name: "Backend Agent", status: "idle"), Agent(id: "qa", name: "QA Agent", status: "idle"),
                          Agent(id: "hd", name: "Hosting Dev", status: "idle"), Agent(id: "orch", name: "Orchestrator", status: "idle")]

    func testAMentionIsBeingTypedOnlyAtAnAtThatStartsAWord() {
        XCTAssertEqual(MentionQuery.query(in: "@"), "")
        XCTAssertEqual(MentionQuery.query(in: "ask @Ho"), "ho")
        XCTAssertNil(MentionQuery.query(in: "mail sam@example"), "an email address isn't a mention")
        XCTAssertNil(MentionQuery.query(in: "@QA Agent which simulator"), "finished: a space after it")
        XCTAssertNil(MentionQuery.query(in: "no mention here"))
    }

    func testMatchesStartAnyWordAndLeaveOutThisChatsAgent() {
        XCTAssertEqual(MentionQuery.matches("dev", in: agents, excluding: "Orchestrator").map(\.name), ["Hosting Dev"])
        XCTAssertEqual(MentionQuery.matches("", in: agents, excluding: "Orchestrator").map(\.name), ["Backend Agent", "Hosting Dev", "QA Agent"])
        XCTAssertEqual(MentionQuery.matches("agent", in: agents, excluding: "Orchestrator").map(\.name), ["Backend Agent", "QA Agent"])
    }

    /// Every agent can be mentioned: the list used to stop at six, so with ten agents four could never be picked.
    func testEveryAgentIsOfferedNotJustTheFirstSix() {
        let many = (1...12).map { Agent(id: "a\($0)", name: String(format: "Agent %02d", $0), status: "idle") }
            + [Agent(id: "orch", name: "Orchestrator", status: "idle")]
        XCTAssertEqual(MentionQuery.matches("", in: many, excluding: "Orchestrator").count, 12)
        XCTAssertEqual(MentionQuery.matches("agent", in: many, excluding: "Orchestrator").map(\.name).last, "Agent 12")
    }

    func testPickingPutsTheExactNameTheBackendMatches() {
        XCTAssertEqual(MentionQuery.insert(agents[2], into: "can you check this @ho"), "can you check this @Hosting Dev ")
        XCTAssertEqual(MentionQuery.start(""), "@")
        XCTAssertEqual(MentionQuery.start("hi"), "hi @")
        XCTAssertEqual(MentionQuery.start("hi "), "hi @")
    }
}
