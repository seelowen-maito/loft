import Foundation

/// Voice style D's turn-taking, like a voice assistant: your turn ends on a short silence and goes out, the reply is
/// read, talking over it makes it your turn again, and after it you're listening again. Pure, so it's unit-tested.
enum VoiceLoop {
    enum State: Equatable {
        case listening, thinking, speaking, ended
    }

    enum Event: Equatable {
        /// The mic is listening and it's been `quiet` seconds since the last new word.
        case quiet(TimeInterval, hasText: Bool, endsOpen: Bool)
        case replyArrived
        case replyRead
        /// Words heard while the reply is read.
        case userSpoke(words: Int)
        case end
    }

    /// Silence that ends your turn after some words. Short, like a conversation…
    static let endOfTurn: TimeInterval = 1.6
    /// …and longer when the last word leaves the sentence open ("…and", "…porque").
    static let endOfTurnOpen: TimeInterval = 3
    /// Words that count as talking over the reply: one stray word (or a cough heard as one) doesn't.
    static let bargeInWords = 2

    static func reduce(_ s: State, _ e: Event) -> State {
        switch (s, e) {
        case (_, .end): .ended
        case (.listening, .quiet(let q, let hasText, let open)):
            hasText && q >= (open ? endOfTurnOpen : endOfTurn) ? .thinking : .listening
        case (.thinking, .replyArrived): .speaking
        case (.speaking, .replyRead): .listening
        case (.speaking, .userSpoke(let n)): n >= bargeInWords ? .listening : .speaking
        default: s
        }
    }

    /// The state a voice session's phase shows as, for the big indicator.
    static func state(for phase: VoiceSession.Phase) -> State {
        switch phase {
        case .idle, .failed: .ended
        case .preparing, .listening, .paused: .listening
        case .cleaning, .review, .sending: .thinking
        case .speaking: .speaking
        }
    }

    /// How far the silence is towards ending the turn (0…1), for the ring that fills as you pause.
    static func endProgress(quiet: TimeInterval, endsOpen: Bool) -> Double {
        min(1, max(0, quiet / (endsOpen ? endOfTurnOpen : endOfTurn)))
    }
}
