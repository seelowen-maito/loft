#if os(iOS)
import UIKit
#else
import AppKit
#endif
import SwiftUI

/// Loft's palette, shared by the iPhone and Mac apps. The web's colours are the source of truth
/// (frontend/src/index.css); these follow them in light and dark mode.
public enum Theme {
    /// The web's green: lime on dark, a deeper green on light, where lime is too faint on white.
    /// The apps' AccentColor assets hold the same two values, for the system's own tint.
    public static let accent = dynamic(light: rgb(0x4F, 0x6A, 0x26), dark: rgb(0xC8, 0xF2, 0x6D))
    /// Text and icons on an accent fill: near-black on the lime, white on the deeper green.
    public static let onAccent = dynamic(light: .white, dark: PlatformColor(red: 0.03, green: 0.04, blue: 0.04, alpha: 1))
    public static let working = Color(red: 0.30, green: 0.82, blue: 0.52)
    public static let attention = Color(red: 1.00, green: 0.72, blue: 0.25)
    public static let failure = Color(red: 1.00, green: 0.42, blue: 0.40)
    /// Text in those colours on a pale tint of themselves (risk labels): the same in dark mode, darker in light
    /// mode, where the bright amber and red read faintly on white (about 5:1 instead of 1.6:1 and 2.5:1).
    public static let attentionInk = dynamic(light: PlatformColor(red: 0.60, green: 0.36, blue: 0.00, alpha: 1),
                                             dark: PlatformColor(red: 1.00, green: 0.72, blue: 0.25, alpha: 1))
    public static let failureInk = dynamic(light: PlatformColor(red: 0.76, green: 0.18, blue: 0.16, alpha: 1),
                                           dark: PlatformColor(red: 1.00, green: 0.42, blue: 0.40, alpha: 1))
    public static let neutral = Color.secondary

    #if os(iOS)
    typealias PlatformColor = UIColor
    #else
    typealias PlatformColor = NSColor
    #endif

    private static func rgb(_ r: Int, _ g: Int, _ b: Int) -> PlatformColor {
        PlatformColor(red: CGFloat(r) / 255, green: CGFloat(g) / 255, blue: CGFloat(b) / 255, alpha: 1)
    }

    private static func dynamic(light: PlatformColor, dark: PlatformColor) -> Color {
        #if os(iOS)
        Color(uiColor: UIColor { $0.userInterfaceStyle == .dark ? dark : light })
        #else
        Color(nsColor: NSColor(name: nil) { $0.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light })
        #endif
    }
}

extension AgentState {
    public var color: Color {
        switch self {
        case .working: Theme.working
        case .waitingApproval: Theme.attention
        case .failed: Theme.failure
        case .idle, .unknown: Theme.neutral
        }
    }
}

extension TaskState {
    public var color: Color {
        switch self {
        case .inProgress: Theme.working
        case .blocked, .waitingApproval: Theme.attention
        case .failed: Theme.failure
        case .done: Theme.accent
        case .inbox, .ready, .cancelled, .unknown: Theme.neutral
        }
    }
}

extension ConnectionState {
    public var color: Color {
        switch self {
        case .connected: Theme.working
        case .polling, .connecting: Theme.attention
        case .reconnecting(let attempt, _): attempt <= 1 ? Theme.attention : Theme.failure
        case .notConfigured: Theme.neutral
        }
    }
}

/// The filled accent button (`.borderedProminent`) with a label that stays readable on the lime. On the Mac an inactive
/// window greys the fill, so the dark label is only forced while the window is active.
public struct AccentButtonStyle: PrimitiveButtonStyle {
    public init() {}

    public func makeBody(configuration: Configuration) -> some View {
        AccentButton(configuration: configuration)
    }
}

private struct AccentButton: View {
    let configuration: PrimitiveButtonStyleConfiguration
    #if os(macOS)
    @Environment(\.controlActiveState) private var active
    private var onLime: Bool { active == .key || active == .active }
    #else
    private let onLime = true
    #endif

    var body: some View {
        Button(configuration).buttonStyle(.borderedProminent).foregroundStyle(onLime ? AnyShapeStyle(Theme.onAccent) : AnyShapeStyle(.primary))
    }
}

extension PrimitiveButtonStyle where Self == AccentButtonStyle {
    public static var accent: AccentButtonStyle { AccentButtonStyle() }
}
