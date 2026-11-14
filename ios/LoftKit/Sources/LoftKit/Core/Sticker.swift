import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// An agent's avatar SVG as a sticker, drawn with Core Graphics alone so the Mac can use it too: the art inside a white
/// die-cut border grown from its silhouette, with a soft drop shadow, like the web's `Sticker`.
public enum Sticker {
    /// Drawn over a Lead's art, inside the sticker's border; the same shapes as the web's `CAPTAIN_HAT`.
    public static let captainHat = ##"<g transform="rotate(-9 50 6)"><path d="M17 3 C15 -11 85 -11 83 3 L75 12 H25 Z" fill="#ffffff"/><path d="M20 6 L25 12 H75 L80 6 Z" fill="#dfe3ec"/><rect x="24" y="10" width="52" height="8" rx="2" fill="#1f2b5e"/><rect x="24" y="12.5" width="52" height="2" fill="#f2b632"/><path d="M24 17 H76 C73 26 27 26 24 17 Z" fill="#17171c"/><circle cx="50" cy="3" r="5.5" fill="#f2b632"/><circle cx="50" cy="3" r="2.4" fill="#1f2b5e"/></g>"##

    /// The SVG with the captain hat drawn last; the agent's own SVG is never changed.
    public static func withHat(_ svg: String) -> String {
        guard let end = svg.range(of: "</svg>", options: .backwards) else { return svg }
        return svg.replacingCharacters(in: end, with: captainHat + "</svg>")
    }

    /// The sticker as a PNG `pixels` wide, or nil if the SVG can't be read. A Lead's wears the captain hat.
    public static func png(svg: String, pixels: Int, lead: Bool = false) -> Data? {
        guard let art = SVGDocument(svg), art.drawsSomething, let doc = lead ? SVGDocument(withHat(svg)) : art,
              let image = image(doc, pixels: pixels) else { return nil }
        let data = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(dest, image, nil)
        return CGImageDestinationFinalize(dest) ? data as Data : nil
    }

    /// The art's 100 units sit inside a 128-unit canvas (14 each side) so the border and shadow have room.
    private static func image(_ doc: SVGDocument, pixels: Int) -> CGImage? {
        let size = CGFloat(pixels), unit = size / 128
        func context() -> CGContext? {
            CGContext(data: nil, width: pixels, height: pixels, bitsPerComponent: 8, bytesPerRow: 0,
                      space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        }
        let full = CGRect(x: 0, y: 0, width: size, height: size)
        // SVG's y runs down: flip, then place the art.
        guard let artCtx = context(), let paperCtx = context(), let out = context() else { return nil }
        artCtx.translateBy(x: 0, y: size)
        artCtx.scaleBy(x: unit, y: -unit)
        artCtx.translateBy(x: 14, y: 14)
        doc.draw(in: artCtx)
        guard let art = artCtx.makeImage() else { return nil }
        // Silhouette: the art's alpha, filled white.
        paperCtx.draw(art, in: full)
        paperCtx.setBlendMode(.sourceIn)
        paperCtx.setFillColor(CGColor(gray: 1, alpha: 1))
        paperCtx.fill(full)
        guard let paper = paperCtx.makeImage() else { return nil }
        // Shadow under the whole sticker (y up here, so a negative offset falls below), then the border grown by
        // stamping the silhouette in a ring, then the art on top.
        let border = 3.4 * unit
        out.saveGState()
        out.setShadow(offset: CGSize(width: 0, height: -2 * unit), blur: 4.4 * unit, color: CGColor(gray: 0, alpha: 0.32))
        out.beginTransparencyLayer(auxiliaryInfo: nil)
        for step in 0..<16 {
            let a = CGFloat(step) / 16 * 2 * .pi
            out.draw(paper, in: full.offsetBy(dx: cos(a) * border, dy: sin(a) * border))
        }
        out.draw(paper, in: full)
        out.endTransparencyLayer()
        out.restoreGState()
        out.draw(art, in: full)
        return out.makeImage()
    }
}
