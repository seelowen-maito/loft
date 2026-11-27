import Foundation

/// A typed message's place in the queue (#665, #668): still waiting for the agent's current run to end, or
/// cancelled before the agent read it. Shown in the message's footer, before its time.
public enum QueueMark: Equatable, Sendable {
    case queued
    case cancelled

    /// The mark on a message still waiting for the agent's current run to end.
    public static let queuedText = "Queued until this run ends"
    /// The mark on a message the user cancelled while it waited: the agent never read it.
    public static let cancelledText = "Cancelled, not sent to the agent"
    /// What a Cancel the run beat to the message says.
    public static let tooLateText = "The agent already took this message"

    /// `m`'s mark in `agent`'s chat: queued while the agent's `queued_messages` holds it, cancelled once
    /// `meta.cancelled` says so, else none.
    public init?(_ m: ChatMessage, in agent: Agent) {
        if agent.queuedMessages?.contains(m.id) == true { self = .queued }
        else if m.isCancelled { self = .cancelled }
        else { return nil }
    }

    /// The message box after Copy to box on `m`: its text, after any draft.
    public static func copiedToBox(_ draft: String, _ m: ChatMessage) -> String {
        draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? m.displayContent : "\(draft)\n\n\(m.displayContent)"
    }
}

extension ChatMessage {
    /// Cancelled while it waited in the queue: the agent never read it (`meta.cancelled`).
    public var isCancelled: Bool { meta?["cancelled"] == .bool(true) }
}
