import Foundation

/// The Mac window's sections, in sidebar order; ⌘1 to ⌘5 open them.
public enum MacSection: Int, CaseIterable, Identifiable, Sendable {
    case overview = 1, agents, issues, needsYou, settings

    public var id: Int { rawValue }

    public var title: String {
        switch self {
        case .overview: "Overview"
        case .agents: "Agents"
        case .issues: "Issues"
        case .needsYou: "Needs you"
        case .settings: "Settings"
        }
    }

    public var symbol: String {
        switch self {
        case .overview: "square.grid.2x2"
        case .agents: "person.3"
        case .issues: "checklist"
        case .needsYou: "bell.badge"
        case .settings: "gearshape"
        }
    }

    /// The section a link lands in.
    public init(_ link: DeepLink) {
        switch link {
        case .needsYou, .needsYouItem: self = .needsYou
        case .agents, .talk, .agent: self = .agents
        case .overview: self = .overview
        case .issue: self = .issues
        }
    }
}

/// Which section the window shows and the sidebar highlights (one value, so they can't differ: a click, ⌘1–⌘5, a link
/// or an opened chat all set it), and when a chat's composer takes the keyboard.
public struct MacSelection: Sendable {
    public var section: MacSection = .agents
    /// Counts the chats opened from code, and how many of them a composer has answered.
    public private(set) var composerRequests = 0
    private var composerAnswered = 0

    public init() {}

    /// ⌘1–⌘5, a link or a menu: shows the section and highlights its row.
    public mutating func go(_ section: MacSection) {
        self.section = section
        composerAnswered = composerRequests  // a request for a chat that never showed doesn't wait for a later click
    }
    /// A line for the top of that agent's chat: why it opened here and not in Herdr.
    public var chatNote: (agentId: String, text: String)?

    /// Spotlight, quick search, ⌘N, a notification: shows Agents, and asks for the cursor in the composer.
    public mutating func openChat() {
        chatNote = nil  // it was for the open that fell back, not for this one
        section = .agents
        composerRequests += 1
    }

    /// A composer asks whether a chat was opened from code since one last answered; answering clears it.
    public mutating func takeComposerRequest() -> Bool {
        defer { composerAnswered = composerRequests }
        return composerRequests != composerAnswered
    }

    /// A chat's composer takes the keyboard when the chat shows, unless the sidebar has it (↑/↓ there keep moving
    /// between sections), and always when the chat was opened from code.
    /// - requested: `takeComposerRequest()`.
    public static func composerTakesKeyboard(requested: Bool, sidebarHasKeyboard: Bool) -> Bool {
        requested || !sidebarHasKeyboard
    }
}
