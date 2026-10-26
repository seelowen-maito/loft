import XCTest
@testable import Loft

final class ShortReadTests: XCTestCase {
    func testShortReplyIsReadWhole() {
        let r = VoiceText.shortRead("Done. The docs site is deployed.")
        XCTAssertEqual(r.text, "Done. The docs site is deployed.")
        XCTAssertNil(r.rest)
    }

    func testOpeningParagraphFirst() {
        let r = VoiceText.shortRead("Deployed the docs site.\n\n## Details\n- Built in 40 s\n- Cache cleared\n\nSee [the PR](https://x.y/1).")
        XCTAssertEqual(r.text, "Deployed the docs site.")
        let rest = try! XCTUnwrap(r.rest)
        XCTAssertTrue(rest.contains("Built in 40 s"))
        XCTAssertTrue(rest.contains("See the PR."), "the rest is speakable too: no Markdown or URLs")
        XCTAssertFalse(rest.contains("https"))
    }

    func testLongFirstParagraphIsCutAtASentence() {
        let sentence = "This sentence is about sixty characters long, give or take. "
        let r = VoiceText.shortRead(String(repeating: sentence, count: 10), limit: 130)
        XCTAssertEqual(r.text, (sentence + sentence).trimmingCharacters(in: .whitespaces))
        XCTAssertTrue(r.rest?.hasPrefix("This sentence") == true)
    }

    func testOneLongSentenceIsKeptWhole() {
        let long = String(repeating: "word ", count: 100) + "end."
        let r = VoiceText.shortRead(long, limit: 50)
        XCTAssertEqual(r.text, long.trimmingCharacters(in: .whitespaces))
        XCTAssertNil(r.rest)
    }
}
