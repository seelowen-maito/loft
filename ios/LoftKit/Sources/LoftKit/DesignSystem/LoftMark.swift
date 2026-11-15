#if canImport(AppKit)
import AppKit

/// Loft's mark: the web's 8×8 pixel house (frontend `Logo`), drawn for the Mac menu bar (#516).
public enum LoftMark {
    static let rows = ["...##...", "..####..", ".######.", "########", ".#....#.", ".#.##.#.", ".#.##.#.", ".######."]

    /// A 16pt template image, 2pt per pixel so it stays crisp; the menu bar tints it for light, dark and wallpaper bars.
    public static func menuBarImage() -> NSImage {
        let cell: CGFloat = 2
        let image = NSImage(size: NSSize(width: 8 * cell, height: 8 * cell), flipped: true) { _ in
            NSColor.black.setFill()
            for (y, row) in rows.enumerated() {
                for (x, c) in row.enumerated() where c == "#" {
                    NSRect(x: CGFloat(x) * cell, y: CGFloat(y) * cell, width: cell, height: cell).fill()
                }
            }
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "Loft"
        return image
    }
}
#endif
