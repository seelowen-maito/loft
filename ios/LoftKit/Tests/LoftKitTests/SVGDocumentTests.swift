import XCTest
@testable import LoftKit

/// Avatar SVGs past the size or nesting limit aren't read at all.
final class SVGDocumentTests: XCTestCase {
    private func svg(_ inner: String) -> String { #"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">"# + inner + "</svg>" }
    private func nested(_ groups: Int) -> String {
        svg(String(repeating: "<g>", count: groups) + #"<circle cx="50" cy="50" r="40"/>"# + String(repeating: "</g>", count: groups))
    }

    func testUpTo64KBIsReadAndLargerIsNot() {
        let circle = #"<circle cx="50" cy="50" r="40"/>"#
        let fits = svg(String(repeating: circle, count: (SVGDocument.maxBytes - svg("").utf8.count) / circle.utf8.count))
        XCTAssertLessThanOrEqual(fits.utf8.count, 64 * 1024)
        XCTAssertNotNil(SVGDocument(fits))
        XCTAssertNil(SVGDocument(fits + String(repeating: " ", count: 64 * 1024 - fits.utf8.count + 1)))
    }

    func testUpTo64LevelsIsReadAndDeeperIsNot() {
        // The <svg> itself is the first level, the circle the last.
        XCTAssertNotNil(SVGDocument(nested(62)))
        XCTAssertNil(SVGDocument(nested(63)))
        XCTAssertNil(SVGDocument(nested(10_000)))
    }
}
