import XCTest
@testable import LoftKit

/// A chat message's Markdown in blocks, as the web draws it (#486).
final class MarkdownBlocksTests: XCTestCase {
    typealias B = MarkdownBlock

    func testAReplyWithEveryKindOfBlock() {
        let md = """
        ## Plan

        Two **steps**, see [the PR](https://example.com).
        It wraps.

        - first
        - [x] done
          more about it
            - nested

        1. one
        2) two

        ```swift
        let x = 1
            indented()
        ```

        > quoted
        > - in a list

        | Name | State |
        |------|:-----:|
        | Dev | busy |

        ---
        """
        XCTAssertEqual(B.parse(md), [
            .heading(level: 2, text: "Plan"),
            .paragraph("Two **steps**, see [the PR](https://example.com).\nIt wraps."),
            .list(ordered: false, items: [
                .init(depth: 0, marker: "•", text: "first"),
                .init(depth: 0, marker: "•", checked: true, text: "done\nmore about it"),
                .init(depth: 1, marker: "•", text: "nested"),
            ]),
            .list(ordered: true, items: [.init(depth: 0, marker: "1.", text: "one"), .init(depth: 0, marker: "2.", text: "two")]),
            .code(language: "swift", text: "let x = 1\n    indented()"),
            .quote([.paragraph("quoted"), .list(ordered: false, items: [.init(depth: 0, marker: "•", text: "in a list")])]),
            .table(header: ["Name", "State"], rows: [["Dev", "busy"]]),
            .rule,
        ])
    }

    func testPlainTextIsOneParagraph() {
        XCTAssertEqual(B.parse("Hello *there*"), [.paragraph("Hello *there*")])
        XCTAssertEqual(B.parse(""), [])
    }

    func testAnUnclosedFenceRunsToTheEnd() {
        XCTAssertEqual(B.parse("```\na\n\nb"), [.code(language: "", text: "a\n\nb")])
    }

    func testNotABlockWhenItOnlyLooksLikeOne() {
        XCTAssertEqual(B.parse("#hashtag and *emphasis* and 2024. was a year"),
                       [.paragraph("#hashtag and *emphasis* and 2024. was a year")])
        XCTAssertEqual(B.parse("a | b without a divider"), [.paragraph("a | b without a divider")])
    }

    func testABlankLineBetweenItemsKeepsOneList() {
        XCTAssertEqual(B.parse("- a\n\n- b"), [.list(ordered: false, items: [.init(depth: 0, marker: "•", text: "a"), .init(depth: 0, marker: "•", text: "b")])])
    }
}
