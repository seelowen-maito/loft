import SwiftUI
import UIKit

/// Agent avatars are small SVG drawings on a 100x100 canvas (backend `avatar.rs` validates them:
/// shapes, paths, groups, gradients and clip paths only). iOS can't draw SVG data at runtime, so
/// this renders that subset with Core Graphics, then wraps it in a sticker: a white die-cut border
/// grown from the silhouette plus a soft drop shadow, like the web's `Sticker`.
enum StickerRenderer {
    private static let cache = NSCache<NSString, UIImage>()

    /// Drawn over a Lead's art, inside the sticker's border; the same shapes as the web's `CAPTAIN_HAT`.
    static let captainHat = #"<g transform="rotate(-9 50 6)"><path d="M17 3 C15 -11 85 -11 83 3 L75 12 H25 Z" fill="#ffffff"/><path d="M20 6 L25 12 H75 L80 6 Z" fill="#dfe3ec"/><rect x="24" y="10" width="52" height="8" rx="2" fill="#1f2b5e"/><rect x="24" y="12.5" width="52" height="2" fill="#f2b632"/><path d="M24 17 H76 C73 26 27 26 24 17 Z" fill="#17171c"/><circle cx="50" cy="3" r="5.5" fill="#f2b632"/><circle cx="50" cy="3" r="2.4" fill="#1f2b5e"/></g>"#

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

// MARK: - SVG subset

/// A parsed avatar SVG: a tree of groups and shapes plus the gradients and clip paths they reference.
struct SVGDocument {
    fileprivate var root = SVGNode(name: "svg")
    fileprivate var defs: [String: SVGNode] = [:]

    init?(_ source: String) {
        guard let data = source.data(using: .utf8) else { return nil }
        let builder = SVGTreeBuilder()
        let parser = XMLParser(data: data)
        parser.delegate = builder
        guard parser.parse(), let root = builder.root, root.name == "svg" else { return nil }
        self.root = root
        root.walk { node in
            if let id = node.attrs["id"] { defs[id] = node }
        }
    }

    var drawsSomething: Bool {
        var found = false
        root.walk { if SVGNode.shapes.contains($0.name) { found = true } }
        return found
    }

    func draw(in cg: CGContext) {
        draw(root, in: cg, style: .initial)
    }

    private func draw(_ node: SVGNode, in cg: CGContext, style inherited: SVGStyle) {
        if ["defs", "clipPath", "mask", "linearGradient", "radialGradient", "stop"].contains(node.name) { return }
        let style = inherited.applying(node.attrs)
        cg.saveGState()
        defer { cg.restoreGState() }
        if let t = node.attrs["transform"] { cg.concatenate(SVGParse.transform(t)) }
        if let clip = SVGParse.reference(node.attrs["clip-path"]), let clipNode = defs[clip] {
            let path = CGMutablePath()
            clipNode.walk { child in
                guard let shape = SVGParse.shape(child) else { return }
                let t = child.attrs["transform"].map(SVGParse.transform) ?? .identity
                path.addPath(shape, transform: t)
            }
            cg.addPath(path)
            cg.clip(using: clipNode.attrs["clip-rule"] == "evenodd" ? .evenOdd : .winding)
        }
        let groupOpacity = node.attrs["opacity"].flatMap(Double.init) ?? 1
        if groupOpacity < 1 {
            cg.setAlpha(groupOpacity)
            cg.beginTransparencyLayer(auxiliaryInfo: nil)
        }
        if let path = SVGParse.shape(node) {
            fill(path, style: style, in: cg)
            stroke(path, style: style, in: cg)
        }
        for child in node.children { draw(child, in: cg, style: style) }
        if groupOpacity < 1 { cg.endTransparencyLayer() }
    }

    private func fill(_ path: CGPath, style: SVGStyle, in cg: CGContext) {
        guard let paint = style.fill, paint != "none" else { return }
        let rule: CGPathFillRule = style.fillRule == "evenodd" ? .evenOdd : .winding
        if let id = SVGParse.reference(paint) {
            guard let gradient = defs[id] else { return }
            cg.saveGState()
            cg.addPath(path)
            cg.clip(using: rule)
            drawGradient(gradient, bounds: path.boundingBoxOfPath, opacity: style.fillOpacity, in: cg)
            cg.restoreGState()
        } else if let color = SVGParse.color(paint) {
            cg.setFillColor(color.copy(alpha: color.alpha * style.fillOpacity) ?? color)
            cg.addPath(path)
            cg.fillPath(using: rule)
        }
    }

    private func stroke(_ path: CGPath, style: SVGStyle, in cg: CGContext) {
        guard let paint = style.stroke, paint != "none", style.strokeWidth > 0 else { return }
        cg.setLineWidth(style.strokeWidth)
        cg.setLineCap(style.lineCap)
        cg.setLineJoin(style.lineJoin)
        cg.setMiterLimit(style.miterLimit)
        if let id = SVGParse.reference(paint) {
            guard let gradient = defs[id] else { return }
            cg.saveGState()
            cg.addPath(path)
            cg.replacePathWithStrokedPath()
            cg.clip()
            drawGradient(gradient, bounds: path.boundingBoxOfPath, opacity: style.strokeOpacity, in: cg)
            cg.restoreGState()
        } else if let color = SVGParse.color(paint) {
            cg.setStrokeColor(color.copy(alpha: color.alpha * style.strokeOpacity) ?? color)
            cg.addPath(path)
            cg.strokePath()
        }
    }

    private func drawGradient(_ node: SVGNode, bounds: CGRect, opacity: CGFloat, in cg: CGContext) {
        // Stops may live on a gradient this one inherits from with href; the validator doesn't allow
        // href, so stops are always local.
        var colors: [CGColor] = []
        var locations: [CGFloat] = []
        for stop in node.children where stop.name == "stop" {
            let color = SVGParse.color(stop.attrs["stop-color"] ?? "#000") ?? UIColor.black.cgColor
            let a = (stop.attrs["stop-opacity"].flatMap(Double.init) ?? 1) * opacity
            colors.append(color.copy(alpha: color.alpha * a) ?? color)
            locations.append(SVGParse.number(stop.attrs["offset"]) ?? 0)
        }
        guard !colors.isEmpty,
              let gradient = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: colors as CFArray, locations: locations)
        else { return }
        let userSpace = node.attrs["gradientUnits"] == "userSpaceOnUse"
        if !userSpace {
            cg.translateBy(x: bounds.minX, y: bounds.minY)
            cg.scaleBy(x: max(bounds.width, 0.001), y: max(bounds.height, 0.001))
        }
        if let t = node.attrs["gradientTransform"] { cg.concatenate(SVGParse.transform(t)) }
        let n = { (key: String, fallback: CGFloat) in SVGParse.number(node.attrs[key]) ?? fallback }
        let options: CGGradientDrawingOptions = [.drawsBeforeStartLocation, .drawsAfterEndLocation]
        if node.name == "radialGradient" {
            let (cx, cy, r) = (n("cx", 0.5 * (userSpace ? 100 : 1)), n("cy", 0.5 * (userSpace ? 100 : 1)), n("r", 0.5 * (userSpace ? 100 : 1)))
            let (fx, fy) = (n("fx", cx), n("fy", cy))
            cg.drawRadialGradient(gradient, startCenter: CGPoint(x: fx, y: fy), startRadius: 0, endCenter: CGPoint(x: cx, y: cy), endRadius: r, options: options)
        } else {
            let start = CGPoint(x: n("x1", 0), y: n("y1", 0))
            let end = CGPoint(x: n("x2", userSpace ? 100 : 1), y: n("y2", 0))
            cg.drawLinearGradient(gradient, start: start, end: end, options: options)
        }
    }
}

fileprivate final class SVGNode {
    static let shapes: Set<String> = ["path", "circle", "ellipse", "rect", "polygon", "polyline", "line"]
    let name: String
    var attrs: [String: String] = [:]
    var children: [SVGNode] = []
    init(name: String, attrs: [String: String] = [:]) {
        self.name = name
        self.attrs = attrs
    }
    func walk(_ visit: (SVGNode) -> Void) {
        visit(self)
        for c in children { c.walk(visit) }
    }
}

fileprivate final class SVGTreeBuilder: NSObject, XMLParserDelegate {
    var root: SVGNode?
    private var stack: [SVGNode] = []

    func parser(_ parser: XMLParser, didStartElement name: String, namespaceURI: String?, qualifiedName: String?, attributes: [String: String] = [:]) {
        let node = SVGNode(name: name, attrs: attributes)
        if let parent = stack.last { parent.children.append(node) } else { root = node }
        stack.append(node)
    }

    func parser(_ parser: XMLParser, didEndElement name: String, namespaceURI: String?, qualifiedName: String?) {
        stack.removeLast()
    }
}

/// Paint properties that inherit down the tree (opacity doesn't; it's applied per element).
fileprivate struct SVGStyle {
    var fill: String? = "#000"
    var fillOpacity: CGFloat = 1
    var fillRule = "nonzero"
    var stroke: String?
    var strokeOpacity: CGFloat = 1
    var strokeWidth: CGFloat = 1
    var lineCap: CGLineCap = .butt
    var lineJoin: CGLineJoin = .miter
    var miterLimit: CGFloat = 4

    static let initial = SVGStyle()

    func applying(_ a: [String: String]) -> SVGStyle {
        var s = self
        if let v = a["fill"] { s.fill = v }
        if let v = SVGParse.number(a["fill-opacity"]) { s.fillOpacity = v }
        if let v = a["fill-rule"] { s.fillRule = v }
        if let v = a["stroke"] { s.stroke = v }
        if let v = SVGParse.number(a["stroke-opacity"]) { s.strokeOpacity = v }
        if let v = SVGParse.number(a["stroke-width"]) { s.strokeWidth = v }
        if let v = SVGParse.number(a["stroke-miterlimit"]) { s.miterLimit = v }
        switch a["stroke-linecap"] {
        case "round": s.lineCap = .round
        case "square": s.lineCap = .square
        case "butt": s.lineCap = .butt
        default: break
        }
        switch a["stroke-linejoin"] {
        case "round": s.lineJoin = .round
        case "bevel": s.lineJoin = .bevel
        case "miter": s.lineJoin = .miter
        default: break
        }
        return s
    }
}

enum SVGParse {
    static func number(_ s: String?) -> CGFloat? {
        guard var s = s?.trimmingCharacters(in: .whitespaces), !s.isEmpty else { return nil }
        var percent = false
        if s.hasSuffix("%") { percent = true; s.removeLast() }
        if s.hasSuffix("px") { s.removeLast(2) }
        guard let v = Double(s) else { return nil }
        return CGFloat(percent ? v / 100 : v)
    }

    /// `url(#id)` → `id`.
    static func reference(_ s: String?) -> String? {
        guard let s = s?.trimmingCharacters(in: .whitespaces), s.hasPrefix("url(#"), s.hasSuffix(")") else { return nil }
        return String(s.dropFirst(5).dropLast())
    }

    static func color(_ raw: String) -> CGColor? {
        let s = raw.trimmingCharacters(in: .whitespaces).lowercased()
        if s.hasPrefix("#") {
            var hex = String(s.dropFirst())
            if hex.count == 3 || hex.count == 4 { hex = hex.map { "\($0)\($0)" }.joined() }
            guard hex.count == 6 || hex.count == 8, let v = UInt64(hex, radix: 16) else { return nil }
            let (r, g, b, a): (UInt64, UInt64, UInt64, UInt64) = hex.count == 8
                ? (v >> 24 & 0xff, v >> 16 & 0xff, v >> 8 & 0xff, v & 0xff)
                : (v >> 16 & 0xff, v >> 8 & 0xff, v & 0xff, 0xff)
            return CGColor(srgbRed: CGFloat(r) / 255, green: CGFloat(g) / 255, blue: CGFloat(b) / 255, alpha: CGFloat(a) / 255)
        }
        if s.hasPrefix("rgb") {
            let parts = s.drop { $0 != "(" }.dropFirst().prefix { $0 != ")" }.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
            guard parts.count >= 3 else { return nil }
            let c = parts.prefix(3).map { p -> CGFloat in
                p.hasSuffix("%") ? (number(p) ?? 0) : (Double(p).map { CGFloat($0) / 255 } ?? 0)
            }
            let a = parts.count > 3 ? (number(parts[3]) ?? 1) : 1
            return CGColor(srgbRed: c[0], green: c[1], blue: c[2], alpha: a)
        }
        let named: [String: UInt32] = [
            "black": 0x000000, "white": 0xffffff, "red": 0xff0000, "green": 0x008000, "blue": 0x0000ff,
            "yellow": 0xffff00, "orange": 0xffa500, "pink": 0xffc0cb, "purple": 0x800080, "gray": 0x808080,
            "grey": 0x808080, "brown": 0xa52a2a, "gold": 0xffd700, "cyan": 0x00ffff, "magenta": 0xff00ff,
        ]
        if s == "transparent" { return CGColor(srgbRed: 0, green: 0, blue: 0, alpha: 0) }
        guard let v = named[s] else { return nil }
        return CGColor(srgbRed: CGFloat(v >> 16 & 0xff) / 255, green: CGFloat(v >> 8 & 0xff) / 255, blue: CGFloat(v & 0xff) / 255, alpha: 1)
    }

    static func numbers(_ s: String) -> [CGFloat] {
        var scanner = NumberScanner(s)
        var out: [CGFloat] = []
        while let n = scanner.next() { out.append(n) }
        return out
    }

    static func transform(_ s: String) -> CGAffineTransform {
        var t = CGAffineTransform.identity
        var rest = Substring(s)
        while let open = rest.firstIndex(of: "("), let close = rest[open...].firstIndex(of: ")") {
            let name = rest[..<open].trimmingCharacters(in: CharacterSet(charactersIn: " ,\n\t"))
            let v = numbers(String(rest[rest.index(after: open)..<close]))
            var m = CGAffineTransform.identity
            switch name {
            case "translate" where !v.isEmpty: m = CGAffineTransform(translationX: v[0], y: v.count > 1 ? v[1] : 0)
            case "scale" where !v.isEmpty: m = CGAffineTransform(scaleX: v[0], y: v.count > 1 ? v[1] : v[0])
            case "rotate" where !v.isEmpty:
                let r = CGAffineTransform(rotationAngle: v[0] * .pi / 180)
                m = v.count >= 3
                    ? CGAffineTransform(translationX: -v[1], y: -v[2]).concatenating(r).concatenating(CGAffineTransform(translationX: v[1], y: v[2]))
                    : r
            case "skewX" where !v.isEmpty: m = CGAffineTransform(a: 1, b: 0, c: tan(v[0] * .pi / 180), d: 1, tx: 0, ty: 0)
            case "skewY" where !v.isEmpty: m = CGAffineTransform(a: 1, b: tan(v[0] * .pi / 180), c: 0, d: 1, tx: 0, ty: 0)
            case "matrix" where v.count == 6: m = CGAffineTransform(a: v[0], b: v[1], c: v[2], d: v[3], tx: v[4], ty: v[5])
            default: break
            }
            // SVG lists apply right to left: the rightmost transform touches the points first.
            t = m.concatenating(t)
            rest = rest[rest.index(after: close)...]
        }
        return t
    }

    fileprivate static func shape(_ node: SVGNode) -> CGPath? {
        let a = node.attrs
        let n = { (key: String) in number(a[key]) ?? 0 }
        switch node.name {
        case "path": return a["d"].map(path)
        case "circle":
            let r = n("r")
            return r > 0 ? CGPath(ellipseIn: CGRect(x: n("cx") - r, y: n("cy") - r, width: 2 * r, height: 2 * r), transform: nil) : nil
        case "ellipse":
            let (rx, ry) = (n("rx"), n("ry"))
            return rx > 0 && ry > 0 ? CGPath(ellipseIn: CGRect(x: n("cx") - rx, y: n("cy") - ry, width: 2 * rx, height: 2 * ry), transform: nil) : nil
        case "rect":
            let rect = CGRect(x: n("x"), y: n("y"), width: n("width"), height: n("height"))
            guard rect.width > 0, rect.height > 0 else { return nil }
            var rx = number(a["rx"]), ry = number(a["ry"])
            if rx == nil { rx = ry }
            if ry == nil { ry = rx }
            let cw = min(rx ?? 0, rect.width / 2), ch = min(ry ?? 0, rect.height / 2)
            return cw > 0 && ch > 0 ? CGPath(roundedRect: rect, cornerWidth: cw, cornerHeight: ch, transform: nil) : CGPath(rect: rect, transform: nil)
        case "line":
            let p = CGMutablePath()
            p.move(to: CGPoint(x: n("x1"), y: n("y1")))
            p.addLine(to: CGPoint(x: n("x2"), y: n("y2")))
            return p
        case "polygon", "polyline":
            let v = numbers(a["points"] ?? "")
            guard v.count >= 4 else { return nil }
            let p = CGMutablePath()
            p.move(to: CGPoint(x: v[0], y: v[1]))
            for i in stride(from: 2, to: v.count - 1, by: 2) { p.addLine(to: CGPoint(x: v[i], y: v[i + 1])) }
            if node.name == "polygon" { p.closeSubpath() }
            return p
        default: return nil
        }
    }

    /// Path data: all commands, absolute and relative, including smooth curves and arcs.
    static func path(_ d: String) -> CGPath {
        let p = CGMutablePath()
        var s = NumberScanner(d)
        var current = CGPoint.zero, start = CGPoint.zero
        var lastControl: CGPoint?
        var lastCommand: Character = " "
        while let cmd = s.command(repeating: lastCommand) {
            let rel = cmd.isLowercase
            let base = rel ? current : .zero
            func pt() -> CGPoint? {
                guard let x = s.next(), let y = s.next() else { return nil }
                return CGPoint(x: base.x + x, y: base.y + y)
            }
            var control: CGPoint?
            switch cmd.uppercased().first! {
            case "M":
                guard let q = pt() else { return p }
                p.move(to: q)
                current = q; start = q
                lastCommand = rel ? "l" : "L"  // further pairs are implicit lineto
                lastControl = nil
                continue
            case "L":
                guard let q = pt() else { return p }
                p.addLine(to: q); current = q
            case "H":
                guard let x = s.next() else { return p }
                current = CGPoint(x: rel ? current.x + x : x, y: current.y); p.addLine(to: current)
            case "V":
                guard let y = s.next() else { return p }
                current = CGPoint(x: current.x, y: rel ? current.y + y : y); p.addLine(to: current)
            case "C":
                guard let c1 = pt(), let c2 = pt(), let q = pt() else { return p }
                p.addCurve(to: q, control1: c1, control2: c2); current = q; control = c2
            case "S":
                let c1 = reflect(lastControl, about: current, if: "CS".contains(lastCommand.uppercased()))
                guard let c2 = pt(), let q = pt() else { return p }
                p.addCurve(to: q, control1: c1, control2: c2); current = q; control = c2
            case "Q":
                guard let c = pt(), let q = pt() else { return p }
                p.addQuadCurve(to: q, control: c); current = q; control = c
            case "T":
                let c = reflect(lastControl, about: current, if: "QT".contains(lastCommand.uppercased()))
                guard let q = pt() else { return p }
                p.addQuadCurve(to: q, control: c); current = q; control = c
            case "A":
                guard let rx = s.next(), let ry = s.next(), let rot = s.next(), let large = s.flag(), let sweep = s.flag(), let q = pt() else { return p }
                addArc(to: p, from: current, to: q, rx: rx, ry: ry, rotation: rot, large: large, sweep: sweep)
                current = q
            case "Z":
                p.closeSubpath(); current = start
            default: return p
            }
            lastControl = control
            lastCommand = cmd
        }
        return p
    }

    private static func reflect(_ c: CGPoint?, about p: CGPoint, if applies: Bool) -> CGPoint {
        guard applies, let c else { return p }
        return CGPoint(x: 2 * p.x - c.x, y: 2 * p.y - c.y)
    }

    /// SVG endpoint arc → center parameterisation (SVG 1.1 appendix F.6), drawn as an ellipse segment.
    private static func addArc(to p: CGMutablePath, from p0: CGPoint, to p1: CGPoint, rx: CGFloat, ry: CGFloat, rotation: CGFloat, large: Bool, sweep: Bool) {
        var rx = abs(rx), ry = abs(ry)
        guard rx > 0, ry > 0, p0 != p1 else { p.addLine(to: p1); return }
        let phi = rotation * .pi / 180
        let (cosP, sinP) = (cos(phi), sin(phi))
        let dx = (p0.x - p1.x) / 2, dy = (p0.y - p1.y) / 2
        let x1 = cosP * dx + sinP * dy, y1 = -sinP * dx + cosP * dy
        let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry)
        if lambda > 1 { rx *= sqrt(lambda); ry *= sqrt(lambda) }
        let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1
        let den = rx * rx * y1 * y1 + ry * ry * x1 * x1
        var coef = sqrt(max(0, num / den))
        if large == sweep { coef = -coef }
        let cx1 = coef * rx * y1 / ry, cy1 = -coef * ry * x1 / rx
        let cx = cosP * cx1 - sinP * cy1 + (p0.x + p1.x) / 2
        let cy = sinP * cx1 + cosP * cy1 + (p0.y + p1.y) / 2
        func angle(_ ux: CGFloat, _ uy: CGFloat, _ vx: CGFloat, _ vy: CGFloat) -> CGFloat {
            atan2(ux * vy - uy * vx, ux * vx + uy * vy)
        }
        let theta = angle(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry)
        var delta = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry)
        if !sweep && delta > 0 { delta -= 2 * .pi }
        if sweep && delta < 0 { delta += 2 * .pi }
        // Draw on a unit circle mapped onto the rotated ellipse.
        let t = CGAffineTransform(translationX: cx, y: cy).rotated(by: phi).scaledBy(x: rx, y: ry)
        p.addRelativeArc(center: .zero, radius: 1, startAngle: theta, delta: delta, transform: t)
    }
}

/// Reads numbers and command letters out of path data like `M10-5.5.5 1e2z`.
struct NumberScanner {
    private let chars: [Character]
    private var i = 0

    init(_ s: String) { chars = Array(s) }

    private mutating func skipSeparators() {
        while i < chars.count, chars[i].isWhitespace || chars[i] == "," { i += 1 }
    }

    /// The next command letter, or `repeating` when the data continues with numbers (implicit repeat).
    mutating func command(repeating last: Character) -> Character? {
        skipSeparators()
        guard i < chars.count else { return nil }
        let c = chars[i]
        if c.isLetter, c != "e", c != "E" { i += 1; return c }
        return last == " " || last.uppercased() == "Z" ? nil : last
    }

    /// Arc flags are single digits that may run together (`a5 5 0 011 1`).
    mutating func flag() -> Bool? {
        skipSeparators()
        guard i < chars.count, chars[i] == "0" || chars[i] == "1" else { return nil }
        defer { i += 1 }
        return chars[i] == "1"
    }

    mutating func next() -> CGFloat? {
        skipSeparators()
        let start = i
        if i < chars.count, chars[i] == "-" || chars[i] == "+" { i += 1 }
        var sawDot = false, sawDigit = false
        while i < chars.count {
            let c = chars[i]
            if c.isNumber { sawDigit = true; i += 1 }
            else if c == ".", !sawDot { sawDot = true; i += 1 }
            else { break }
        }
        if sawDigit, i < chars.count, chars[i] == "e" || chars[i] == "E" {
            var j = i + 1
            if j < chars.count, chars[j] == "-" || chars[j] == "+" { j += 1 }
            if j < chars.count, chars[j].isNumber {
                i = j
                while i < chars.count, chars[i].isNumber { i += 1 }
            }
        }
        guard sawDigit, let v = Double(String(chars[start..<i])) else { i = start; return nil }
        return CGFloat(v)
    }
}
