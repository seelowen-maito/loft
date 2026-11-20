import XCTest
@testable import Loft
@testable import LoftKit

final class MentionQueryTests: XCTestCase {
    private func names(_ q: String, in agents: [Agent], excluding name: String) -> [String] {
        MentionQuery.sections(q, in: agents, excluding: name).flatMap(\.agents).map(\.name)
    }

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
        XCTAssertEqual(names("dev", in: agents, excluding: "Orchestrator"), ["Hosting Dev"])
        XCTAssertEqual(names("", in: agents, excluding: "Orchestrator"), ["Backend Agent", "Hosting Dev", "QA Agent"])
        XCTAssertEqual(names("agent", in: agents, excluding: "Orchestrator"), ["Backend Agent", "QA Agent"])
    }

    /// Every agent can be mentioned: the list used to stop at six, so with ten agents four could never be picked.
    func testEveryAgentIsOfferedNotJustTheFirstSix() {
        let many = (1...12).map { Agent(id: "a\($0)", name: String(format: "Agent %02d", $0), status: "idle") }
            + [Agent(id: "orch", name: "Orchestrator", status: "idle")]
        XCTAssertEqual(names("", in: many, excluding: "Orchestrator").count, 12)
        XCTAssertEqual(names("agent", in: many, excluding: "Orchestrator").last, "Agent 12")
    }

    /// The backend delivers a mention to any agent, its own team's first: the chat agent's team (its group) comes
    /// first, then every other agent. Agents without a group have no team, so no team mates.
    func testTheChatsTeamComesFirstThenEveryOtherAgent() {
        let team = [Agent(id: "orch", name: "Orchestrator", project: "loft", status: "idle", group: "loft "),
                    Agent(id: "qa", name: "QA Agent", project: "imole", status: "idle", group: "loft"),
                    Agent(id: "web", name: "Web Dev", project: "imole", status: "idle"),
                    Agent(id: "free", name: "Helper", status: "idle"),
                    Agent(id: "api", name: "API Dev", project: "api", status: "idle", group: "Dineout", projects: ["api"]),
                    Agent(id: "pos", name: "POS Dev", project: "pos", status: "idle", group: " Dineout", projects: ["pos", "api"])]
        let sections = { (name: String) in MentionQuery.sections("", in: team, excluding: name).map { "\($0.title): \($0.agents.map(\.name))" } }
        XCTAssertEqual(sections("Orchestrator"), [#"loft: ["QA Agent"]"#, #"Other agents: ["API Dev", "Helper", "POS Dev", "Web Dev"]"#])
        XCTAssertEqual(sections("API Dev"), [#"Dineout: ["POS Dev"]"#, #"Other agents: ["Helper", "Orchestrator", "QA Agent", "Web Dev"]"#],
                       "the group is the team, whatever the main project")
        XCTAssertEqual(sections("Helper"), [#"Other agents: ["API Dev", "Orchestrator", "POS Dev", "QA Agent", "Web Dev"]"#], "no team mates")
        XCTAssertEqual(sections("Web Dev"), [#"Other agents: ["API Dev", "Helper", "Orchestrator", "POS Dev", "QA Agent"]"#],
                       "a main project without a group is no team")
        XCTAssertEqual(sections("Unknown"), [#"Agents: ["API Dev", "Helper", "Orchestrator", "POS Dev", "QA Agent", "Web Dev"]"#],
                       "chat's agent not loaded: one list")
        XCTAssertEqual(MentionQuery.sections("zz", in: team, excluding: "Orchestrator").count, 0)
    }

    func testPickingPutsTheExactNameTheBackendMatches() {
        XCTAssertEqual(MentionQuery.insert(agents[2], into: "can you check this @ho"), "can you check this @Hosting Dev ")
        XCTAssertEqual(MentionQuery.start(""), "@")
        XCTAssertEqual(MentionQuery.start("hi"), "hi @")
        XCTAssertEqual(MentionQuery.start("hi "), "hi @")
    }
}
