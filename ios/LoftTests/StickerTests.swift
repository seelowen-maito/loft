import XCTest
@testable import Loft

final class StickerTests: XCTestCase {
    /// The backend's generated shape: a shaded body via clipPath, hat, googly eyes, stroked mouth.
    let generated = ##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><clipPath id="body"><path d="M50 12 C76 12 90 30 90 54 C90 78 74 92 50 92 C26 92 10 78 10 54 C10 30 24 12 50 12 Z"/></clipPath></defs><g fill="#d94f8a"><path d="M50 12 C76 12 90 30 90 54 C90 78 74 92 50 92 C26 92 10 78 10 54 C10 30 24 12 50 12 Z"/></g><g clip-path="url(#body)"><g fill="#ff7eb6" transform="translate(-1 -5)"><path d="M50 12 C76 12 90 30 90 54 C90 78 74 92 50 92 C26 92 10 78 10 54 C10 30 24 12 50 12 Z"/></g></g><ellipse cx="38.0" cy="46.0" rx="12.5" ry="14.0" fill="#fff"/><ellipse cx="34.0" cy="41.8" rx="5.2" ry="7.2" transform="rotate(-10 34.0 41.8)" fill="#17171c"/><path d="M43.5 66.0 Q50 72.0 56.5 66.0" fill="none" stroke="#17171c" stroke-width="4.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##

    private func pixel(_ image: UIImage, x: CGFloat, y: CGFloat) -> (r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat) {
        let cg = image.cgImage!
        let px = Int(x * image.scale), py = Int(y * image.scale)
        var data = [UInt8](repeating: 0, count: 4)
        let ctx = CGContext(data: &data, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        ctx.draw(cg, in: CGRect(x: -px, y: py - cg.height + 1, width: cg.width, height: cg.height))
        return (CGFloat(data[0]) / 255, CGFloat(data[1]) / 255, CGFloat(data[2]) / 255, CGFloat(data[3]) / 255)
    }

    func testRendersBackendStickerWithBorder() throws {
        let image = try XCTUnwrap(StickerRenderer.image(svg: generated, points: 128, scale: 1))
        XCTAssertEqual(image.size, CGSize(width: 128, height: 128))
        // 128pt canvas = 1pt per unit, art offset by 14. The body near the bottom is the base pink.
        let body = pixel(image, x: 14 + 70, y: 14 + 70)
        XCTAssertEqual(body.a, 1, accuracy: 0.01)
        XCTAssertGreaterThan(body.r, 0.9)
        // The eye white.
        let eye = pixel(image, x: 14 + 44, y: 14 + 52)
        XCTAssertGreaterThan(min(eye.r, eye.g, eye.b), 0.95)
        // Just outside the body (left edge at x=10): the white die-cut border.
        let border = pixel(image, x: 14 + 8, y: 14 + 54)
        XCTAssertGreaterThan(border.a, 0.9)
        XCTAssertGreaterThan(min(border.r, border.g, border.b), 0.95)
        // The corner stays transparent.
        XCTAssertLessThan(pixel(image, x: 1, y: 1).a, 0.05)
    }

    func testUnreadableOrEmptyFallsBack() {
        XCTAssertNil(StickerRenderer.image(svg: "<svg/>", points: 40, scale: 2))
        XCTAssertNil(StickerRenderer.image(svg: "not svg", points: 40, scale: 2))
        XCTAssertNil(StickerRenderer.image(svg: #"<svg viewBox="0 0 100 100"><defs/></svg>"#, points: 40, scale: 2))
    }

    func testFixtureStickersAllRender() {
        for (id, svg) in FixtureStickers.byAgent {
            XCTAssertNotNil(StickerRenderer.image(svg: svg, points: 60, scale: 2), id)
        }
    }

    func testNumbersAndFlags() {
        XCTAssertEqual(SVGParse.numbers("10-5.5.5 1e2,-.25"), [10, -5.5, 0.5, 100, -0.25])
        // Arc flags written together: rx ry rot large sweep x y.
        let arc = SVGParse.path("M0 0a5 5 0 0110 0")
        XCTAssertEqual(arc.currentPoint.x, 10, accuracy: 0.001)
        XCTAssertEqual(arc.currentPoint.y, 0, accuracy: 0.001)
    }

    func testPathCommands() {
        // Relative lines with implicit repeats after M, H/V, and close back to the start.
        let p = SVGParse.path("m10 10 10 0 0 10h-5v5z")
        XCTAssertEqual(p.currentPoint, CGPoint(x: 10, y: 10))
        XCTAssertEqual(p.boundingBoxOfPath, CGRect(x: 10, y: 10, width: 10, height: 15))
        // A half-circle arc reaches the far side of its ellipse.
        let a = SVGParse.path("M10 50 A40 40 0 0 1 90 50")
        XCTAssertEqual(a.boundingBoxOfPath.minY, 10, accuracy: 0.5)
        XCTAssertEqual(a.boundingBoxOfPath.maxY, 50, accuracy: 0.5)
    }

    func testTransforms() {
        let t = SVGParse.transform("translate(10 5) scale(2)")
        XCTAssertEqual(CGPoint(x: 1, y: 1).applying(t), CGPoint(x: 12, y: 7))
        let r = SVGParse.transform("rotate(90 50 50)")
        let q = CGPoint(x: 60, y: 50).applying(r)
        XCTAssertEqual(q.x, 50, accuracy: 0.001)
        XCTAssertEqual(q.y, 60, accuracy: 0.001)
    }

    func testColors() {
        XCTAssertEqual(SVGParse.color("#fff")?.components, [1, 1, 1, 1])
        XCTAssertEqual(SVGParse.color("#ff000080")?.alpha ?? 0, 128.0 / 255, accuracy: 0.001)
        XCTAssertEqual(SVGParse.color("rgb(255, 0, 0)")?.components, [1, 0, 0, 1])
        XCTAssertNotNil(SVGParse.color("white"))
        XCTAssertNil(SVGParse.color("url(#g)"))
    }

    func testAgentDecodesAvatar() throws {
        let json = #"{"id":"a","name":"A","status":"idle","avatar":{"svg":"<svg/>"}}"#
        let a = try JSONCoding.decoder.decode(Agent.self, from: Data(json.utf8))
        XCTAssertEqual(a.avatar?.svg, "<svg/>")
    }
}
