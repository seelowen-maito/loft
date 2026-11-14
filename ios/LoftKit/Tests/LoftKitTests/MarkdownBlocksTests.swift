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
            .paragraph("Two **steps**, see [the PR](https://example.com). It wraps."),
            .list(ordered: false, items: [
                .init(depth: 0, marker: "•", text: "first"),
                .init(depth: 0, marker: "•", checked: true, text: "done more about it"),
                .init(depth: 1, marker: "•", text: "nested"),
            ]),
            .list(ordered: true, items: [.init(depth: 0, marker: "1.", text: "one"), .init(depth: 0, marker: "2.", text: "two")]),
            .code(language: "swift", text: "let x = 1\n    indented()"),
            .quote([.paragraph("quoted"), .list(ordered: false, items: [.init(depth: 0, marker: "•", text: "in a list")])]),
            .table(header: ["Name", "State"], alignments: [.leading, .center], rows: [["Dev", "busy"]]),
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

    func testAFencedCommandInsideANumberedStepStaysInTheStep() {
        let md = "1. Build\n2. Install\n   ```sh\n   xcrun simctl install booted Loft.app\n\n   - not an item\n   ```\n3. Launch"
        XCTAssertEqual(B.parse(md), [.list(ordered: true, items: [
            .init(depth: 0, marker: "1.", text: "Build"),
            .init(depth: 0, marker: "2.", text: "Install",
                  blocks: [.code(language: "sh", text: "xcrun simctl install booted Loft.app\n\n- not an item")]),
            .init(depth: 0, marker: "3.", text: "Launch"),
        ])])
    }

    func testAnIndentedParagraphAfterABlankLineStaysInTheItem() {
        XCTAssertEqual(B.parse("- a\n\n  more\n- b"), [.list(ordered: false, items: [
            .init(depth: 0, marker: "•", text: "a", blocks: [.paragraph("more")]),
            .init(depth: 0, marker: "•", text: "b"),
        ])])
    }

    func testLinesOfAParagraphJoinUnlessTheyEndInABreak() {
        XCTAssertEqual(B.parse("One soft\nbreak line here.\nHard  \nbreak\\\nhere"),
                       [.paragraph("One soft break line here. Hard\nbreak\nhere")])
    }

    func testTableColumnsKeepTheirAlignment() {
        XCTAssertEqual(B.parse("|a|b|c|\n|:--|:-:|--:|\n|1|2|3|"),
                       [.table(header: ["a", "b", "c"], alignments: [.leading, .center, .trailing], rows: [["1", "2", "3"]])])
    }

    /// A line of 40,000 `>` once overflowed the stack (Security on #486): quotes stop nesting at `maxDepth`.
    func testDeeplyNestedQuoteMarkersStopNesting() {
        let start = Date()
        var blocks = B.parse(String(repeating: ">", count: 40_000) + " hi")
        var depth = 0
        while case .quote(let inner) = blocks.first { blocks = inner; depth += 1 }
        XCTAssertEqual(depth, B.maxDepth)
        XCTAssertEqual(blocks, [.paragraph(String(repeating: ">", count: 40_000 - B.maxDepth) + " hi")])
        XCTAssertLessThan(Date().timeIntervalSince(start), 1)
    }

    /// A line of 40,000 `- ` or `1. ` made one list per marker and overflowed the stack (Security on #486).
    func testDeeplyNestedListMarkersStopNesting() {
        for marker in ["- ", "1. "] {
            let start = Date()
            var blocks = B.parse(String(repeating: marker, count: 40_000) + "hi")
            var depth = 0
            var last: B.ListItem?
            while case .list(_, let items) = blocks.first, let item = items.first { last = item; blocks = item.blocks; depth += 1 }
            XCTAssertEqual(depth, B.maxDepth, marker)
            XCTAssertEqual(last?.text, String(repeating: marker, count: 40_000 - B.maxDepth) + "hi")
            XCTAssertLessThan(Date().timeIntervalSince(start), 1, marker)
        }
    }
}
