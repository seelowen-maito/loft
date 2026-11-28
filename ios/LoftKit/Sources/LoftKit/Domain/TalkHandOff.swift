import Foundation

/// When the Mac's Talk sheet and voice panel drop a dictation, and where a denied permission leads (spec #382).
public enum TalkHandOff {
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
