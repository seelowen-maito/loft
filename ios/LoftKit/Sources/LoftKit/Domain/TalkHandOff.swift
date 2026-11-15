import Foundation

/// What the Mac's Talk sheet does as the voice session moves on (spec #382). Kept pure so the sheet's hand-off can be
/// tested without a microphone.
public enum TalkHandOff {
    public enum Outcome: Equatable {
        /// Still listening, tidying up or failed: the sheet stays.
        case stay
        /// Tidied and kept: its text goes to the chat's message box, to read and send.
        case textToChat
        /// Sent (by Send, or by a spoken "send"): back to the chat, which shows the message; the session reads the reply.
        case sentToChat
    }

    /// `listened`: this sheet has started listening. The shared session can still be `.replied` or `.speaking` from a
    /// Talk before it, and that is not this sheet's send.
    public static func outcome(phase: VoiceSession.Phase, autoSendAt: Date?, listened: Bool) -> Outcome {
        guard listened else { return .stay }
        return switch phase {
        case .review: autoSendAt == nil ? .textToChat : .stay
        case .sending, .speaking, .replied: .sentToChat
        default: .stay
        }
    }

    /// Phases in which closing the sheet throws the dictation away. After a send the session goes on reading the reply.
    public static func discardsOnClose(_ phase: VoiceSession.Phase) -> Bool {
        return switch phase {
        case .preparing, .listening, .paused, .cleaning, .review: true
        default: false
        }
    }

    /// The Privacy pane in System Settings that turns the failing access back on.
    public static func privacyPane(for message: String) -> String {
        message.localizedCaseInsensitiveContains("speech") ? "Privacy_SpeechRecognition" : "Privacy_Microphone"
    }
}
