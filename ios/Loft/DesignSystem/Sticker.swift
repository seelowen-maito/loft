import LoftKit
import SwiftUI
import UIKit

/// Agent avatars are small SVG drawings on a 100x100 canvas (backend `avatar.rs` validates them:
/// shapes, paths, groups, gradients and clip paths only). iOS can't draw SVG data at runtime, so
/// this renders that subset with Core Graphics, then wraps it in a sticker: a white die-cut border
/// grown from the silhouette plus a soft drop shadow, like the web's `Sticker`.
enum StickerRenderer {
    private static let cache = NSCache<NSString, UIImage>()

    /// Drawn over a Lead's art, inside the sticker's border; the same shapes as the web's `CAPTAIN_HAT`.
    static let captainHat = ##"<g transform="rotate(-9 50 6)"><path d="M17 3 C15 -11 85 -11 83 3 L75 12 H25 Z" fill="#ffffff"/><path d="M20 6 L25 12 H75 L80 6 Z" fill="#dfe3ec"/><rect x="24" y="10" width="52" height="8" rx="2" fill="#1f2b5e"/><rect x="24" y="12.5" width="52" height="2" fill="#f2b632"/><path d="M24 17 H76 C73 26 27 26 24 17 Z" fill="#17171c"/><circle cx="50" cy="3" r="5.5" fill="#f2b632"/><circle cx="50" cy="3" r="2.4" fill="#1f2b5e"/></g>"##

    /// The sticker at `points` wide, or nil if the SVG can't be read (callers fall back to initials).
    /// A Lead's sticker wears the captain hat; the agent's own SVG is never changed.
    static func image(svg: String, points: CGFloat, scale: CGFloat, lead: Bool = false) -> UIImage? {
        let key = "\(svg.hashValue)@\(points)x\(scale)\(lead ? "+lead" : "")" as NSString
        if let hit = cache.object(forKey: key) { return hit }
        guard let art = SVGDocument(svg), art.drawsSomething, let doc = lead ? SVGDocument(withHat(svg)) : art else { return nil }
        let image = sticker(doc, points: points, scale: scale)
        cache.setObject(image, forKey: key)
        return image
    }

    /// The hat on its own, as an avatar SVG: worn over the initials fallback.
    static let hatOnly = #"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">"# + captainHat + "</svg>"

    private static func withHat(_ svg: String) -> String {
        guard let end = svg.range(of: "</svg>", options: .backwards) else { return svg }
        return svg.replacingCharacters(in: end, with: captainHat + "</svg>")
    }

    /// Same layout as the web: the art's 100 units sit inside a 128-unit canvas (14 each side)
    /// so the border and shadow have room.
    private static func sticker(_ doc: SVGDocument, points: CGFloat, scale: CGFloat) -> UIImage {
        let unit = points / 128
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        let size = CGSize(width: points, height: points)
        let art = UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            let cg = ctx.cgContext
            cg.translateBy(x: 14 * unit, y: 14 * unit)
            cg.scaleBy(x: unit, y: unit)
            doc.draw(in: cg)
        }
        // Silhouette: the art's alpha, filled white.
        let paper = UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            art.draw(at: .zero)
            UIColor.white.setFill()
            // The renderer context's plain fill(_:) resets to normal blending, which would paint the
            // whole square; sourceIn keeps only the art's alpha.
            ctx.fill(CGRect(origin: .zero, size: size), blendMode: .sourceIn)
        }
        return UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            let cg = ctx.cgContext
            let border = 3.4 * unit
            // Shadow under the whole sticker, then the border grown by stamping the silhouette in a ring.
            cg.saveGState()
            cg.setShadow(offset: CGSize(width: 0, height: 2 * unit), blur: 4.4 * unit, color: UIColor.black.withAlphaComponent(0.32).cgColor)
            cg.beginTransparencyLayer(auxiliaryInfo: nil)
            for step in 0..<16 {
                let a = CGFloat(step) / 16 * 2 * .pi
                paper.draw(at: CGPoint(x: cos(a) * border, y: sin(a) * border))
            }
            paper.draw(at: .zero)
            cg.endTransparencyLayer()
            cg.restoreGState()
            art.draw(at: .zero)
        }
    }
}

/// An agent's avatar as a sticker; nil `svg` or an unreadable one shows nothing (use a fallback).
struct StickerImage: View {
    let svg: String
    let size: CGFloat
    var lead = false
    @Environment(\.displayScale) private var displayScale

    var body: some View {
        if let image = StickerRenderer.image(svg: svg, points: size, scale: displayScale, lead: lead) {
            Image(uiImage: image).resizable().frame(width: size, height: size)
        }
    }
}
