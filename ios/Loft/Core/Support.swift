import Foundation
import os

enum Log {
    private static let subsystem = Bundle.main.bundleIdentifier ?? "loft"
    static let net = Logger(subsystem: subsystem, category: "network")
    static let sync = Logger(subsystem: subsystem, category: "sync")
    static let speech = Logger(subsystem: subsystem, category: "speech")
}

/// The last data loaded from the server, so the app opens instantly and stays
/// browsable offline. Always shown as stale until a fresh sync succeeds.
struct Snapshot: Codable {
    var server: String
    var savedAt: Date
    var info: ServerInfo?
    var agents: [Agent]
    var tasks: [HBTask]
    var permissions: [PermissionRequest]
    var activity: [ActivityItem]
    var pullRequests: [PullRequest]
    /// Recent history of conversations that were opened, per agent.
    var messages: [String: [ChatMessage]]
    /// Messages that never reached the server; kept so they can be retried by hand.
    var unsent: [PendingMessage]
}

struct SnapshotStore {
    var url: URL

    static var standard: SnapshotStore {
        let dir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        return SnapshotStore(url: dir.appendingPathComponent("loft-snapshot.json"))
    }

    func load(for server: String) -> Snapshot? {
        guard let data = try? Data(contentsOf: url),
              let s = try? JSONCoding.decoder.decode(Snapshot.self, from: data),
              s.server == server else { return nil }
        return s
    }

    func save(_ s: Snapshot) {
        do {
            let data = try JSONCoding.encoder.encode(s)
            try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            Log.sync.error("snapshot save failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    func clear() { try? FileManager.default.removeItem(at: url) }
}

/// A message the user sent that the server hasn't confirmed yet.
struct PendingMessage: Codable, Identifiable, Hashable {
    /// Sent to the server as `client_id`, so a retry is never delivered twice.
    let id: String
    let agentId: String
    var text: String
    var createdAt: Date
    var failure: String?
    /// Sent with the message (a grill answer's tag); nil for plain messages.
    var meta: JSONValue?

    var isFailed: Bool { failure != nil }
}

/// Launch switches for UI tests. Debug builds only; the real app never sees them.
enum TestMode {
    /// `-no-animations`: no UIKit or SwiftUI animations, so a UI test never waits for one to finish (the
    /// "animations complete" waits behind the idle timeouts) and screens settle at once.
    static let noAnimations = flag("-no-animations")
    /// `-no-speech`: no microphone, speech recognizer or permission prompts. Talking and dictation start at once
    /// and hear nothing, so a UI test never stalls on "Failed to initialize recognizer" in the Simulator.
    static let noSpeech = flag("-no-speech")
    /// `-no-ai`: the on-device model counts as off, so no UI test waits on it (`-fixture-ai-status` still wins).
    static let noAI = flag("-no-ai")

    private static func flag(_ name: String) -> Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains(name)
        #else
        false
        #endif
    }
}

/// App preferences (UserDefaults). Nothing secret is stored here.
enum Prefs {
    static let serverURL = "serverURL"
    static let orchestratorId = "orchestratorAgentId"
    static let appearance = "appearance"
    static let speechLocale = "speechLocale"
    static let speechOnDeviceOnly = "speechOnDeviceOnly"
    /// The server's grill command phrases (JSON), cached for offline matching.
    static let grillPhrases = "grillPhrases"
    /// AVSpeechSynthesisVoice identifier for reading replies aloud; empty means automatic.
    static let speechVoice = "speechVoice"
    static let notifyApprovals = "notifyApprovals"
    static let notifyTaskOutcomes = "notifyTaskOutcomes"
    static let notifyFailures = "notifyFailures"
    static let voiceHandsFree = "voiceHandsFree"
    static let voiceReadReplies = "voiceReadReplies"
    /// Read each new interview question aloud (its short `speak` version).
    static let grillReadAloud = "grillReadAloud"
    /// While a reply or question is read in a hands-free conversation, listen for "stop" / "skip" / "repeat".
    static let voiceInterrupt = "voiceInterrupt"
    /// Read the opening of a long reply first; "more" reads the rest.
    static let voiceShortRead = "voiceShortRead"
    /// Push: quiet hours on/off and their "HH:mm" bounds (per-kind switches are `PushKind.prefKey`).
    static let pushQuietHours = "pushQuietHours"
    static let pushQuietStart = "pushQuietStart"
    static let pushQuietEnd = "pushQuietEnd"
    /// Settings › Apple Intelligence: use the on-device model for voice commands the phrase lists can't tell.
    static let aiCommands = "aiCommands"
    /// Settings › Apple Intelligence: with "Read long replies short", the on-device model writes the short version.
    static let aiShortSpoken = "aiShortSpoken"
    /// One-line summaries and suggested replies in "Needs you".
    static let aiNeedsYou = "aiNeedsYou"
    /// When the app was last in front: the digest covers what happened since.
    static let digestLastLooked = "digestLastLooked"
    /// A one-line first reply from the on-device model right after you send to the Orchestrator.
    static let aiQuickAck = "aiQuickAck"
    /// "Send to <agent>?" when a message to the Orchestrator looks meant for one agent.
    static let aiRoute = "aiRoute"
    /// "Make it a brief" under a long dictation (Claude on the Mac, `voice_brief`).
    static let voiceBrief = "voiceBrief"
    /// Suggested one-tap replies under an agent's question in a chat.
    static let aiChatReplies = "aiChatReplies"
    /// "What does this do?" on a permission card.
    static let aiExplain = "aiExplain"

    static func registerDefaults() {
        UserDefaults.standard.register(defaults: [
            appearance: "dark",
            speechOnDeviceOnly: false,
            notifyApprovals: true,
            notifyTaskOutcomes: true,
            notifyFailures: true,
            voiceHandsFree: true,
            voiceReadReplies: true,
            grillReadAloud: true,
            voiceInterrupt: true,
            voiceShortRead: true,
            aiCommands: true,
            aiShortSpoken: true,
            aiNeedsYou: true,
            aiQuickAck: true,
            aiRoute: true,
            voiceBrief: true,
            aiChatReplies: true,
            aiExplain: true,
        ])
    }
}

extension Date {
    /// "now", "4m", "3h", "2d", or a short date.
    var shortAgo: String {
        let s = -timeIntervalSinceNow
        switch s {
        case ..<45: return "now"
        case ..<3600: return "\(Int(s / 60) + (s.truncatingRemainder(dividingBy: 60) > 30 ? 1 : 0))m"
        case ..<86400: return "\(Int(s / 3600))h"
        case ..<(86400 * 7): return "\(Int(s / 86400))d"
        default: return formatted(.dateTime.month(.abbreviated).day())
        }
    }

    /// Elapsed time, e.g. "12m" or "1h 05m".
    func durationSince(_ now: Date = Date()) -> String {
        let s = max(0, Int(now.timeIntervalSince(self)))
        if s < 60 { return "\(s)s" }
        if s < 3600 { return "\(s / 60)m" }
        return String(format: "%dh %02dm", s / 3600, (s % 3600) / 60)
    }
}
