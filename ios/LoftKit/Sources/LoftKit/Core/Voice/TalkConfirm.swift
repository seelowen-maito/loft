import Foundation

/// Talk's message box step: the tidied text waits in the chat's box. After a few untouched seconds Loft asks
/// "Send it?" (never sends by itself), and "send it" said aloud sends it as the Send button does. Pure, so it's tested.
public enum TalkConfirm {
    public enum State: Equatable {
        /// Waiting, with the countdown ring, since this moment.
        case counting(since: Date)
        /// "Send it?" with Send and Edit.
        case asking
        /// You touched it: it's an ordinary message box now, nothing asks or listens.
        case editing
        case sending
        /// "Cancel": the text is thrown away.
        case cancelled
    }

    public enum Command: Equatable { case send, edit, cancel }

    public enum Event: Equatable {
        case tick(Date)
        /// Typing in the box, or tapping Edit.
        case touched
        case heard(Command)
        case sendTapped
    }

    /// Untouched this long, it asks.
    public static let askAfter: TimeInterval = 4

    public static func reduce(_ s: State, _ e: Event, askAfterPause: Bool = true) -> State {
        switch (s, e) {
        case (.counting(let since), .tick(let now)):
            askAfterPause && now.timeIntervalSince(since) >= askAfter ? .asking : s
        case (.counting, .touched), (.asking, .touched), (.counting, .heard(.edit)), (.asking, .heard(.edit)): .editing
        case (.counting, .heard(.send)), (.asking, .heard(.send)), (.counting, .sendTapped), (.asking, .sendTapped): .sending
        case (.counting, .heard(.cancel)), (.asking, .heard(.cancel)): .cancelled
        default: s
        }
    }

    /// How far the ring has gone (0…1).
    public static func progress(since: Date, now: Date) -> Double { min(1, max(0, now.timeIntervalSince(since) / askAfter)) }

    /// The mic stays open this long at most, then the buttons are the way.
    public static let listenFor: TimeInterval = 60

    /// Listening for "send it" makes sense only while it waits, with the setting on, the app in front, and not
    /// for longer than `listenFor`.
    public static func listens(_ s: State, sayToSend: Bool, active: Bool = true, listenedFor: TimeInterval = 0) -> Bool {
        guard sayToSend, active, listenedFor < listenFor else { return false }
        switch s {
        case .counting, .asking: return true
        default: return false
        }
    }

    private static let fillers: Set<String> = ["ok", "okay", "um", "uh", "please", "now", "so", "yeah"]

    /// The command in what was said, matched on whole words only ("sender" isn't "send"), on the device, with no
    /// model. `agents`: names that may follow "send it to" ("send it to Loft Dev"). A bare "send", "yes" or "go ahead"
    /// only answers the "Send it?" question (`asking`): before it, a bystander's "yes" mustn't send.
    public static func command(in heard: String, agents: [String], asking: Bool = true) -> Command? {
        let words = VoiceText.normalize(heard).split(separator: " ").map(String.init)
        let core = Array(words.drop { fillers.contains($0) }.reversed().drop { fillers.contains($0) }.reversed())
        guard !core.isEmpty else { return nil }
        let said = core.joined(separator: " ")
        if ["send it", "yes send it", "send it now"].contains(said) { return .send }
        if asking, ["send", "yes", "yes send", "send now", "go ahead"].contains(said) { return .send }
        for name in agents.map(VoiceText.normalize) where !name.isEmpty {
            if said == "send it to \(name)" || said == "send to \(name)" || said == "send this to \(name)" { return .send }
        }
        if ["edit", "edit it", "wait", "let me edit", "no"].contains(said) { return .edit }
        if ["cancel", "cancel it", "cancel that", "discard", "discard it"].contains(said) { return .cancel }
        return nil
    }
}
