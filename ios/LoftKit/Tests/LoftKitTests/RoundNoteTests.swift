import XCTest
@testable import LoftKit

/// "Anything else?" by voice: what counts as "no", and the prompt in the session's language.
final class RoundNoteTests: XCTestCase {
    func testNoSendsTheRoundWithoutANote() {
        for said in ["No", "no.", "Nope", "No, thanks.", "No thank you", "Nothing else", "That's all", "um, no", "Não", "não, obrigado",
                     "Nada", "Nada mais", "Só isso", "É isso", "Não, é só isso", "pode mandar"] {
            XCTAssertTrue(RoundNote.declines(said), said)
        }
    }

    func testAnythingElseIsTheNote() {
        for said in ["What about refunds?", "No refunds for yearly plans", "Não esquece dos reembolsos", "nothing about pricing yet",
                     "Também quero falar de preços", "Send the invoices monthly", ""] {
            XCTAssertFalse(RoundNote.declines(said), said)
        }
    }

    func testPromptFollowsTheSessionLanguage() {
        XCTAssertEqual(RoundNote.prompt(language: "pt"), "Algo a mais?")
        XCTAssertEqual(RoundNote.prompt(language: "pt-BR"), "Algo a mais?")
        XCTAssertEqual(RoundNote.prompt(language: "en"), "Anything else?")
        XCTAssertEqual(RoundNote.prompt(language: nil), "Anything else?")
    }

    func testANoteBelongsToItsRound() {
        XCTAssertEqual(RoundNote.key(GrillSession(sessionId: "g1", agentId: nil, state: .active, round: 2)), "g1:2")
        XCTAssertNil(RoundNote.key(GrillSession(sessionId: "g1", agentId: nil, state: .preparing)))
    }
}
