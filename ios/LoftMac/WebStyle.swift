import AppKit
import LoftKit
import SwiftUI

/// The web's component styles (frontend/src/index.css `@layer components`) for the Mac's screens.
extension View {
    /// `.panel`: a card on the page, with a rounded hairline border.
    func panel(radius: CGFloat = WebTheme.panelRadius) -> some View {
        background(WebTheme.surface, in: RoundedRectangle(cornerRadius: radius, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: radius, style: .continuous).strokeBorder(WebTheme.line))
    }

    /// `.eyebrow`: a small mono, upper-case label over a group, `fg3` unless it carries a state.
    func eyebrow(_ color: Color = WebTheme.fg3) -> some View {
        font(.system(size: 10.5, design: .monospaced)).textCase(.uppercase).tracking(1.4).foregroundStyle(color)
    }

    /// `.chip`: a mono pill with a hairline border, tinted when it carries a state.
    func chip(_ tint: Color? = nil) -> some View {
        font(.system(size: 11, design: .monospaced))
            .padding(.horizontal, 8).padding(.vertical, 2)
            .foregroundStyle(tint ?? WebTheme.fg2)
            .background((tint ?? .clear).opacity(0.1), in: Capsule())
            .overlay(Capsule().strokeBorder(tint?.opacity(0.3) ?? WebTheme.line))
    }

    /// A screen on the web's page colour, with its text colours.
    func webPage() -> some View {
        scrollContentBackground(.hidden).background(WebTheme.canvas).foregroundStyle(WebTheme.fg)
    }
}

/// Applies Settings › Appearance to every window, the menu bar's included; Dark unless the user picked otherwise.
@MainActor
enum Appearance {
    static func apply(_ stored: String? = UserDefaults.standard.string(forKey: Prefs.appearance)) {
        NSApp.appearance = AppearanceChoice(stored: stored).nsAppearance
    }
}
