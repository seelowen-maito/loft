import AppKit
import XCTest
@testable import LoftKit

/// The menu bar's house mark (#516): a 16pt template image of the web's pixel house.
final class LoftMarkTests: XCTestCase {
    func testIsATemplateAtMenuBarSize() {
        let image = LoftMark.menuBarImage()
        XCTAssertTrue(image.isTemplate)
        XCTAssertEqual(image.size, NSSize(width: 16, height: 16))
    }

    func testDrawsTheHouse() throws {
        let rep = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 16, pixelsHigh: 16, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
        LoftMark.menuBarImage().draw(in: NSRect(x: 0, y: 0, width: 16, height: 16))
        NSGraphicsContext.restoreGraphicsState()
        // colorAt counts y from the top, like the rows.
        func filled(_ x: Int, _ y: Int) -> Bool { (rep.colorAt(x: x * 2, y: y * 2)?.alphaComponent ?? 0) > 0.5 }
        for (y, row) in LoftMark.rows.enumerated() {
            for (x, c) in row.enumerated() {
                XCTAssertEqual(filled(x, y), c == "#", "pixel \(x),\(y)")
            }
        }
    }
}
