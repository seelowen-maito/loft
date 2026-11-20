import XCTest
@testable import LoftKit

final class AskQuestionTests: XCTestCase {
    private let input: JSONValue = ["questions": .array([
        ["question": "Which store?", "header": "Store", "options": .array([["label": "B2", "description": "cheap"], ["label": "S3"]])],
        ["question": "Regions?", "multiSelect": true, "options": .array([["label": "EU"], ["label": "US"], ["label": "Asia"]])],
        ["header": "no question text: dropped"],
        ["question": "Anything else?", "options": .array([["label": .number(3)]])],
    ])]

    func testReadsOnlyStringText() {
        let qs = AskQuestion.of(input)
        XCTAssertEqual(qs.map(\.question), ["Which store?", "Regions?", "Anything else?"])
        XCTAssertEqual(qs[0].options, [.init(label: "B2", description: "cheap"), .init(label: "S3", description: nil)])
        XCTAssertFalse(qs[0].multiSelect)
        XCTAssertTrue(qs[1].multiSelect)
        XCTAssertEqual(qs[2].options, [], "a label that isn't text is dropped")
        XCTAssertEqual(AskQuestion.of(["command": "ls"]), [])
    }

    func testNothingToSendWhenNoQuestionsParse() {
        XCTAssertNil(AskQuestion.answers(AskQuestion.of(["questions": .array([["header": "no text"]])]), picks: [:]))
    }

    func testTheServersChatHintIsDroppedOnlyForQuestions() throws {
        let list = try JSONCoding.decoder.decode(NeedsYouList.self, from: Data(FixtureNeedsYou.json.utf8))
        var item = try XCTUnwrap(list.items.first { $0.id == "permission:p1" })
        func ask(_ id: String, _ source: String) -> PermissionRequest {
            PermissionRequest(id: id, agentId: "qa", source: source, toolName: "AskUserQuestion", input: nil, reason: nil, rule: nil,
                              status: "pending", createdAt: Date())
        }
        let perms = [ask("p1", "prompt"), ask("q1", "questions")]
        XCTAssertFalse(item.asksQuestions(perms), "the first ask (may it ask?) is an ordinary permission")
        item = NeedsYouItem(id: "permission:q1", type: item.type, urgency: 3, agent: nil, taskId: nil, title: "", reason: "", text: "",
                            createdAt: nil, ageSeconds: nil, links: nil, actions: [], project: nil, pr: nil)
        XCTAssertTrue(item.asksQuestions(perms))
        XCTAssertFalse(item.asksQuestions([]), "not loaded: the server's line stays")
    }

    func testNothingToSendUntilEveryQuestionHasAnAnswer() {
        let qs = Array(AskQuestion.of(input).prefix(2))
        XCTAssertNil(AskQuestion.answers(qs, picks: [:]))
        XCTAssertNil(AskQuestion.answers(qs, picks: [0: .init(chosen: ["B2"])]))
        XCTAssertNil(AskQuestion.answers(qs, picks: [0: .init(chosen: ["B2"]), 1: .init(other: "   ")]), "blank text is no answer")
    }

    /// The same picks give the same `answers` as the web's answersOf: labels in pick order, then the typed text, joined by ", ".
    func testAnswersMatchTheWeb() {
        let qs = Array(AskQuestion.of(input).prefix(2))
        var store = AskQuestion.Pick().toggling("S3", multi: false).toggling("B2", multi: false)
        var regions = AskQuestion.Pick().toggling("Asia", multi: true).toggling("EU", multi: true).toggling("US", multi: true).toggling("US", multi: true)
        regions = regions.typing(" Mars ", multi: true)
        XCTAssertEqual(AskQuestion.answers(qs, picks: [0: store, 1: regions]), ["Which store?": "B2", "Regions?": "Asia, EU, Mars"])
        // Single choice: typing replaces the pick, and picking again clears the typed text.
        store = store.typing("Local disk", multi: false)
        XCTAssertEqual(store, .init(chosen: [], other: "Local disk"))
        XCTAssertEqual(store.toggling("S3", multi: false), .init(chosen: ["S3"], other: ""))
        // Picking the chosen option again clears it.
        XCTAssertEqual(AskQuestion.Pick(chosen: ["B2"]).toggling("B2", multi: false).chosen, [])
    }
}
