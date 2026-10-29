import Foundation

/// Voice style E, after you send: the full-screen status, one step at a time. Pure, so it's unit-tested.
/// Sending (with Undo) → Thinking (the agent works) → Speaking (the reply read aloud) → Replied (the reply stays).
enum VoiceStatus: Equatable {
    case sending, thinking, speaking, replied, failed

    enum Event: Equatable {
        case undo
        case posted
        case postFailed
        case replyArrived(readAloud: Bool)
        case readThrough
        case stop
        case replay
        case retry
    }

    /// The next step, or nil when the status screen closes (Undo: the text goes back to the message box).
    static func reduce(_ s: VoiceStatus, _ e: Event) -> VoiceStatus? {
        switch (s, e) {
        case (.sending, .undo): nil
        case (.sending, .posted): .thinking
        case (.sending, .postFailed), (.thinking, .postFailed): .failed
        case (.thinking, .replyArrived(let read)): read ? .speaking : .replied
        case (.speaking, .readThrough), (.speaking, .stop): .replied
        case (.replied, .replay): .speaking
        case (.failed, .retry): .sending
        default: s
        }
    }

    /// Where a voice session is, as a status step. `undoPending`: the Undo countdown runs; `posting`: the message is
    /// on its way to the server. Nil while it's listening or idle.
    static func of(_ phase: VoiceSession.Phase, undoPending: Bool, posting: Bool) -> VoiceStatus? {
        switch phase {
        case .review: undoPending ? .sending : nil
        case .sending: posting ? .sending : .thinking
        case .speaking: .speaking
        case .replied: .replied
        case .failed: .failed
        default: nil
        }
    }

    /// The step lit on the Sent → Thinking → Reply strip.
    var step: Int {
        switch self {
        case .sending: 0
        case .thinking: 1
        case .speaking, .replied: 2
        case .failed: 0
        }
    }

    func title(agent: String) -> String {
        switch self {
        case .sending: "Sending to \(agent)…"
        case .thinking: "\(agent) is thinking…"
        case .speaking: "\(agent) replies"
        case .replied: "\(agent) replied"
        case .failed: "Not sent"
        }
    }
}
