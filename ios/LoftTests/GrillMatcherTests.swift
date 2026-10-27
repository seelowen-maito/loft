import XCTest
@testable import Loft

final class GrillMatcherTests: XCTestCase {
    private func intent(_ s: String, _ phrases: GrillPhrases? = nil) -> GrillMatcher.Intent? { GrillMatcher.match(s, phrases: phrases)?.intent }
    private func sure(_ s: String) -> GrillMatcher.Intent? {
        guard let m = GrillMatcher.match(s), m.confidence == .sure else { return nil }
        return m.intent
    }
    private func asks(_ s: String) -> GrillMatcher.Intent? {
        guard let m = GrillMatcher.match(s), m.confidence == .ask else { return nil }
        return m.intent
    }

    func testEnglishCommandsRunStraightAway() {
        XCTAssertEqual(sure("Skip."), .command(.skip))
        XCTAssertEqual(sure("Next question"), .command(.skip), "next is the server's skip")
        XCTAssertEqual(sure("go back"), .command(.back))
        XCTAssertEqual(sure("Say that again?"), .repeatQuestion)
        XCTAssertEqual(sure("hold on"), .command(.pause))
        XCTAssertEqual(sure("Carry on"), .command(.resume))
        XCTAssertEqual(sure("End the interview"), .command(.end))
        XCTAssertEqual(sure("Recap"), .command(.summarize))
        XCTAssertEqual(sure("I don't know"), .command(.decide))
        XCTAssertEqual(sure("I dont know."), .command(.decide))
        XCTAssertEqual(sure("you decide"), .command(.decide))
    }

    func testPortugueseCommands() {
        XCTAssertEqual(sure("Pula"), .command(.skip))
        XCTAssertEqual(sure("Próxima pergunta"), .command(.skip))
        XCTAssertEqual(sure("volta"), .command(.back))
        XCTAssertEqual(sure("Repete, por favor"), .repeatQuestion)
        XCTAssertEqual(sure("pausa"), .command(.pause))
        XCTAssertEqual(sure("Continua"), .command(.resume))
        XCTAssertEqual(sure("Chega"), .command(.end))
        XCTAssertEqual(sure("Resume tudo"), .command(.summarize), "PT \"resume\" with \"tudo\" is a summary, not resume")
        XCTAssertEqual(sure("Não sei"), .command(.decide))
        XCTAssertEqual(sure("Tanto faz"), .command(.decide))
    }

    func testFillersDontChangeTheCommand() {
        XCTAssertEqual(sure("Um, skip please"), .command(.skip))
        XCTAssertEqual(sure("ok, então pula"), .command(.skip))
        XCTAssertNil(GrillMatcher.match("um, uh"), "only fillers is nothing")
        // Fillers are trimmed at the ends only: "um" is a real word here.
        XCTAssertEqual(sure("faz um resumo"), .command(.summarize))
        XCTAssertNil(GrillMatcher.match(""))
    }

    func testNearMissesAndEmbeddedCommandsAsk() {
        XCTAssertEqual(asks("skop"), .command(.skip), "misheard")
        XCTAssertEqual(asks("proxma pergunta"), .command(.skip))
        XCTAssertEqual(asks("skip that one"), .command(.skip))
        XCTAssertEqual(asks("I really don't know"), .command(.decide))
        // A longer sentence opening with a command, then a pause mark.
        XCTAssertEqual(asks("Skip, I don't care about pricing tiers at all"), .command(.skip))
    }

    func testAnswersStayAnswers() {
        XCTAssertNil(intent("Mostly first-time visitors."))
        XCTAssertNil(intent("Only on yearly"))
        XCTAssertNil(intent("Yes"))
        XCTAssertNil(intent("I don't know if Postgres scales for this, but let's try it"), "I don't know inside a sentence is an answer")
        XCTAssertNil(intent("We should go back to the old pricing page and copy the layout"), "no pause mark after the command words")
        XCTAssertNil(intent("Skip the free tier and keep monthly and yearly plans for everyone"))
        XCTAssertNil(intent("the next release"), "a one-word command counts only as the first word")
    }

    func testServerPhrasesAreAdded() {
        let p = GrillPhrases(version: "1", maxWords: 4, commands: ["skip": .init(en: ["moving on"], pt: ["partiu"])])
        XCTAssertEqual(GrillMatcher.match("Moving on", phrases: p), .init(intent: .command(.skip), confidence: .sure))
        XCTAssertEqual(GrillMatcher.match("partiu", phrases: p)?.intent, .command(.skip))
        XCTAssertNil(GrillMatcher.match("moving on"), "not built in")
        let withRepeat = GrillPhrases(version: "1", maxWords: 4, fillers: ["bom dia"], clientOnly: ["repeat"],
                                      commands: ["repeat": .init(en: ["once more"], pt: ["outra vez"])])
        XCTAssertEqual(GrillMatcher.match("Outra vez", phrases: withRepeat)?.intent, .repeatQuestion)
        XCTAssertNil(GrillMatcher.match("Outra vez", phrases: p), "same version, other catalog: not the cached one")
    }

    func testLevenshtein() {
        XCTAssertEqual(GrillMatcher.levenshtein("skip", "skop"), 1)
        XCTAssertEqual(GrillMatcher.levenshtein("", "abc"), 3)
        XCTAssertEqual(GrillMatcher.levenshtein("pausa", "pausa"), 0)
    }
}
