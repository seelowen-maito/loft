import Foundation

/// The Mac's voice UI, the floating voice panel (spec #456) and the Talk sheet: what it shows as the shared voice session moves on, who it sends to,
/// and when to offer "Send to …?". Kept pure so its steps can be tested without a microphone.
public enum VoicePanel {
    public enum Step: Equatable {
        /// Opening the microphone.
        case starting
        case listening
        /// Loft is tidying the transcript up.
        case tidying
        /// Tidied: pick the recipient and send.
        case ready
        case sent
        case error(String, needsPermission: Bool)
    }

    /// `listened`: this panel has started listening. The shared session can still be sending or reading a reply from
    /// a Talk before it, and that is not this panel's send.
    public static func step(phase: VoiceSession.Phase, needsPermission: Bool, listened: Bool) -> Step {
        switch phase {
        case .idle: listened ? .error("Nothing was heard. Try again and speak a little closer.", needsPermission: false) : .starting
        case .preparing: .starting
        case .listening, .paused: .listening
        case .cleaning: .tidying
        case .review: listened ? .ready : .starting
        case .sending, .speaking, .replied: listened ? .sent : .starting
        case .failed(let message): .error(message, needsPermission: needsPermission)
        }
    }

    /// Who the panel sends to when it opens: the agent whose chat is open, else the last Talk's, else the Orchestrator
    /// (or the first agent). Agents that are gone are skipped.
    public static func recipient(openChat: String?, last: String?, agents: [Agent], orchestratorId: String?) -> String? {
        let ids = Set(agents.map(\.id))
        return [openChat, last, orchestratorId].compactMap { $0 }.first(where: ids.contains)
            ?? agents.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }.first?.id
    }

    /// Send while listening: once tidied, the text goes out at once, unless the on-device model points at another
    /// agent; then the panel waits on Ready, with "Send to …?", for you to pick.
    public static func sendsAtOnce(guess: Agent?, current: String?) -> Bool {
        suggestion(guess: guess, current: current) == nil
    }

    /// "Send to …?" for the on-device guess, unless it's already the recipient.
    public static func suggestion(guess: Agent?, current: String?) -> Agent? {
        guard let guess, guess.id != current else { return nil }
        return guess
    }
}
