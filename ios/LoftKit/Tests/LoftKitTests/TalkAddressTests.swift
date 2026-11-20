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
        agent("design", "Designer", group: ""),
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
    }

    func testAnAmbiguousNameAtTheEndIsUnclear() {
        XCTAssertEqual(read("Ship it, send this to the Lead."), .unclear, "two Leads")
    }

    /// At the end, "send this to …" or "this is for …" that names no agent is only part of the message: no note.
    func testWordsThatArentANameAtTheEndLeaveTheMessage() {
        XCTAssertEqual(read("Please send this to the customer"), .none)
        XCTAssertEqual(read("Tell them this is for the demo"), .none)
        XCTAssertEqual(read("Fix it and send this to Bob."), .none)
    }

    /// "Loft" is an agent and the start of "Loft Dev" and "Loft mobile dev": the longest wins, at either end.
    func testLoftAsASharedPrefix() {
        XCTAssertEqual(read("It's red, send this to Loft"), .agent(id: "loft", text: "It's red"))
        XCTAssertEqual(read("It's red, send this to Loft Dev"), .agent(id: "loft-dev", text: "It's red"))
        XCTAssertEqual(read("It's red, send this to Loft mobile dev."), .agent(id: "mobile", text: "It's red"))
        XCTAssertEqual(read("Loft, can you restart?"), .agent(id: "loft", text: "Can you restart?"))
        XCTAssertEqual(read("Loft mobile dev, can you look?"), .agent(id: "mobile", text: "Can you look?"))
    }

    /// An agent with an empty group answers to its name only, not to " Designer".
    func testAnAgentWithAnEmptyGroup() {
        XCTAssertEqual(read("Send this to Designer, the icons are blurry."), .agent(id: "design", text: "The icons are blurry."))
        XCTAssertEqual(read("The icons are blurry, this is for the Designer."), .agent(id: "design", text: "The icons are blurry"))
    }

    /// Only the name: the recipient switches, and the message is empty.
    func testOnlyANameLeavesAnEmptyMessage() {
        XCTAssertEqual(read("Send this to QA"), .agent(id: "qa", text: ""))
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
