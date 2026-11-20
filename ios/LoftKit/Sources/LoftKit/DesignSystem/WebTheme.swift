#if os(macOS)
import AppKit
import SwiftUI

/// The web's design tokens (frontend/src/index.css `@theme`) for the Mac app, so it and the browser look like one
/// product. Dark is the web's own palette; light, which the web doesn't have, keeps the same warm greys inverted.
public enum WebTheme {
    /// `ink-950`: the page.
    public static let canvas = dynamic(light: 0xF6F5F1, dark: 0x08090A)
    /// `ink-900`: panels and cards.
    public static let surface = dynamic(light: 0xFFFFFF, dark: 0x0D0E10)
    /// `ink-850`: inputs and the composer.
    public static let field = dynamic(light: 0xFBFAF7, dark: 0x121316)
    /// `ink-800`: buttons and raised rows.
    public static let raised = dynamic(light: 0xEEEDE8, dark: 0x17191C)
    /// `fg`, `fg-2`, `fg-3`: text, secondary text, labels.
    public static let fg = dynamic(light: 0x17191C, dark: 0xECEAE4)
    public static let fg2 = dynamic(light: 0x55554F, dark: 0xA9A8A3)
    public static let fg3 = dynamic(light: 0x7A7A75, dark: 0x6E6E6B)
    /// `line`, `line-strong`: hairline borders.
    public static let line = Color(nsColor: NSColor(name: nil) {
        isDark($0) ? NSColor(white: 1, alpha: 0.07) : NSColor(white: 0, alpha: 0.08)
    })
    public static let lineStrong = Color(nsColor: NSColor(name: nil) {
        isDark($0) ? NSColor(white: 1, alpha: 0.12) : NSColor(white: 0, alpha: 0.14)
    })
    /// `good`, `warn`, `bad`, `info`: states, always next to an icon or a label.
    public static let good = Color(hex: 0x7FD99A)
    public static let warn = Color(hex: 0xF2C46D)
    public static let bad = Color(hex: 0xF27D6D)
    public static let info = Color(hex: 0x8CB8FF)
    /// `merged`: merged pull requests and closed issues.
    public static let merged = Color(hex: 0xB99BFF)

    /// Card and panel corner radius (`rounded-2xl`), and the smaller one for rows and inputs (`rounded-xl`).
    public static let panelRadius: CGFloat = 16
    public static let rowRadius: CGFloat = 12

    static func isDark(_ a: NSAppearance) -> Bool { a.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua }

    private static func dynamic(light: Int, dark: Int) -> Color {
        Color(nsColor: NSColor(name: nil) { NSColor(hex: isDark($0) ? dark : light) })
    }
}

/// Settings › Appearance: Dark (the default, as on the web), Light or System. Kept in `Prefs.appearance`, which the
/// iPhone app uses for the same choice.
public enum AppearanceChoice: String, CaseIterable, Identifiable, Sendable {
    case dark, light, system

    public var id: String { rawValue }
    public var label: String { rawValue.capitalized }

    public init(stored: String?) { self = stored.flatMap(Self.init(rawValue:)) ?? .dark }

    /// The appearance the app takes; nil follows the system.
    public var nsAppearance: NSAppearance? {
        switch self {
        case .dark: NSAppearance(named: .darkAqua)
        case .light: NSAppearance(named: .aqua)
        case .system: nil
        }
    }
}

extension NSColor {
    convenience init(hex: Int) {
        self.init(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
    }
}

extension Color {
    init(hex: Int) { self.init(nsColor: NSColor(hex: hex)) }
}
#endif
