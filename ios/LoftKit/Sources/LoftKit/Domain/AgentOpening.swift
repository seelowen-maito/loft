import Foundation

/// Where Enter, ⌘↵ and ⌥↵ open an agent in the Mac's quick search: the one place that decides, so the footer's
/// keys and what they do can't drift apart.
public enum AgentOpening: Equatable, Sendable {
    /// Its tab in Herdr UI.
    case herdr
    /// Its chat in the Mac app.
    case chat
    /// Its page in the browser.
    case browser

    /// - herdrReady: Herdr UI is linked on this Mac. Without it Herdr is never offered.
    /// - herdrFirst: Settings › Herdr UI says agents open there (Enter).
    /// - browserDefault: Settings › Quick search says Enter opens in the browser, not the app.
    /// ⌘ swaps Herdr and the Mac app's chat (just the chat when Herdr isn't ready); ⌥ is the other of
    /// browser and app; ⌘ wins when both are held.
    public static func choose(herdrReady: Bool, herdrFirst: Bool, browserDefault: Bool, command: Bool, option: Bool) -> AgentOpening {
        let herdrFirst = herdrReady && herdrFirst
        if command { return herdrReady && !herdrFirst ? .herdr : .chat }
        if option { return browserDefault ? .chat : .browser }
        if herdrFirst { return .herdr }
        return browserDefault ? .browser : .chat
    }
}
