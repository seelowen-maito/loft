import XCTest
@testable import LoftKit

/// Talk picks the recipient from what you say (#343): matched on the phone against names, roles and groups.
final class TalkAddressTests: XCTestCase {
    private func agent(_ id: String, _ name: String, role: String? = nil, group: String? = nil) -> Agent {
        var a = Agent(id: id, name: name, status: "idle")
        a.role = role
        a.group = group
        return a
    }

    private lazy var agents = [
        agent("loft", "Loft"),
        agent("loft-lead", "Loft Lead", role: "lead", group: "Loft"),
        agent("loft-dev", "Loft Dev", role: "dev", group: "Loft"),
        agent("mobile", "Loft mobile dev", role: "dev", group: "Loft"),
        agent("dine-lead", "Lead", role: "lead", group: "Dineout"),
        agent("qa", "QA", role: "qa"),
    ]

    private func read(_ text: String) -> TalkAddress.Reading { TalkAddress.read(text, agents: agents) }

    func testSendThisToAnAgentPicksItAndDropsTheWords() {
        XCTAssertEqual(read("Send this to the Dineout Lead, the build is red again."), .agent(id: "dine-lead", text: "The build is red again."))
        XCTAssertEqual(read("send this to Loft Dev: add a retry button"), .agent(id: "loft-dev", text: "Add a retry button"))
        XCTAssertEqual(read("This is for Loft mobile dev. Fix the To row."), .agent(id: "mobile", text: "Fix the To row."))
        XCTAssertEqual(read("Send this to QA please check the login"), .agent(id: "qa", text: "Please check the login"))
    }

    func testANameAtTheEndPicksIt() {
        XCTAssertEqual(read("The build is red again, send this to Loft Lead."), .agent(id: "loft-lead", text: "The build is red again"))
    }

    func testStartingWithANamePicksIt() {
        XCTAssertEqual(read("Loft Dev, can you look at the logs?"), .agent(id: "loft-dev", text: "Can you look at the logs?"))
        XCTAssertEqual(read("QA can you run the suite"), .agent(id: "qa", text: "Can you run the suite"))
        XCTAssertEqual(read("Loft will you restart"), .agent(id: "loft", text: "Will you restart"))
        XCTAssertEqual(read("QA please run the suite"), .agent(id: "qa", text: "Please run the suite"))
    }

    func testTheLongestNameWins() {
        // "Loft Lead" and the Dineout "Lead" both answer to "lead"; the group says which.
        XCTAssertEqual(read("Send this to the Loft Lead: ship it"), .agent(id: "loft-lead", text: "Ship it"))
    }

    func testAnAmbiguousNameIsUnclear() {
        XCTAssertEqual(read("Send this to the Lead, ship it"), .unclear, "two Leads")
        XCTAssertEqual(read("Dev, can you fix it"), .unclear, "two Devs")
    }

    func testAnUnknownNameIsUnclear() {
        XCTAssertEqual(read("Send this to Bob, ship it"), .unclear)
        XCTAssertEqual(read("Fix it and send this to Bob."), .unclear)
    }

    func testNoNameLeavesTheMessage() {
        XCTAssertEqual(read("Add a retry button to the settings screen."), .none)
        XCTAssertEqual(read(""), .none)
        // A name that's only part of the sentence, not who it's said to.
        XCTAssertEqual(read("QA found two bugs in the login."), .none)
        // "will", "can", "would" also start plain statements: only a question ("… can you") or "please" addresses.
        XCTAssertEqual(read("Loft will need a restart after this update."), .none)
        XCTAssertEqual(read("Loft can crash when the disk is full."), .none)
        XCTAssertEqual(read("QA would be faster with more runners."), .none)
        XCTAssertEqual(read("Please send this to the customer and then tell me what they said about it."), .none)
    }
}
