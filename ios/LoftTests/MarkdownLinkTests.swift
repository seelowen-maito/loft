import XCTest
@testable import Loft

/// #635: links in your own (green) bubble are underlined so they still read as links in the bubble's text colour.
final class MarkdownLinkTests: XCTestCase {
    private func linkRuns(_ a: AttributedString) -> [AttributedString.Runs.Run] { a.runs.filter { $0.link != nil } }

    func testOwnBubbleUnderlinesLinks() {
        let a = Markdown.render("see [the PR](https://github.com/o/r/pull/1) now", underlineLinks: true)
        let links = linkRuns(a)
        XCTAssertEqual(links.count, 1)
        XCTAssertEqual(links.first?.underlineStyle, .single)
        XCTAssertNil(a.runs.first?.underlineStyle, "plain text isn't underlined")
    }

    func testOwnBubbleUnderlinesBareURL() {
        let a = Markdown.render("appeared again: https://example.com/a/b opening the order", underlineLinks: true)
        XCTAssertEqual(linkRuns(a).first?.underlineStyle, .single)
    }

    func testAgentBubbleLinksUnchanged() {
        let a = Markdown.render("see [the PR](https://github.com/o/r/pull/1) now")
        XCTAssertEqual(linkRuns(a).count, 1)
        XCTAssertTrue(a.runs.allSatisfy { $0.underlineStyle == nil })
    }
}
