import Foundation
import os

public enum Log {
    private static let subsystem = Bundle.main.bundleIdentifier ?? "loft"
    public static let net = Logger(subsystem: subsystem, category: "network")
    public static let sync = Logger(subsystem: subsystem, category: "sync")
    public static let speech = Logger(subsystem: subsystem, category: "speech")
}

/// The last data loaded from the server, so the app opens instantly and stays
/// browsable offline. Always shown as stale until a fresh sync succeeds.
public struct Snapshot: Codable {
    public var server: String
    public var savedAt: Date
    public var info: ServerInfo?
    public var agents: [Agent]
    public var tasks: [HBTask]
    public var permissions: [PermissionRequest]
    public var activity: [ActivityItem]
    public var pullRequests: [PullRequest]
    /// Recent history of conversations that were opened, per agent.
    public var messages: [String: [ChatMessage]]
    /// Messages that never reached the server; kept so they can be retried by hand.
    public var unsent: [PendingMessage]
}

public struct SnapshotStore {
    public var url: URL

    public static var standard: SnapshotStore {
        let dir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        return SnapshotStore(url: dir.appendingPathComponent("loft-snapshot.json"))
    }

    public func load(for server: String) -> Snapshot? {
        guard let data = try? Data(contentsOf: url),
              let s = try? JSONCoding.decoder.decode(Snapshot.self, from: data),
              s.server == server else { return nil }
        return s
    }

    public func save(_ s: Snapshot) {
        do {
            let data = try JSONCoding.encoder.encode(s)
            try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            Log.sync.error("snapshot save failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    public func clear() { try? FileManager.default.removeItem(at: url) }
}

/// A message the user sent that the server hasn't confirmed yet.
public struct PendingMessage: Codable, Identifiable, Hashable {
    /// Sent to the server as `client_id`, so a retry is never delivered twice.
    public let id: String
    public let agentId: String
    public var text: String
    public var createdAt: Date
    public var failure: String?
    /// Sent with the message (a grill answer's tag); nil for plain messages.
    public var meta: JSONValue?
    /// Uploads sent with it (a video), by the server's names.
    public var images: [String]? = nil

    public var isFailed: Bool { failure != nil }
}

/// Launch switches for UI tests. Debug builds only; the real app never sees them.
public enum TestMode {
    /// `-no-animations`: no UIKit or SwiftUI animations, so a UI test never waits for one to finish (the
    /// "animations complete" waits behind the idle timeouts) and screens settle at once.
    public static let noAnimations = flag("-no-animations")
    /// `-no-speech`: no microphone, speech recognizer or permission prompts. Talking and dictation start at once
    /// and hear nothing, so a UI test never stalls on "Failed to initialize recognizer" in the Simulator.
    public static let noSpeech = flag("-no-speech")
    /// `-no-ai`: the on-device model counts as off, so no UI test waits on it (`-fixture-ai-status` still wins).
    public static let noAI = flag("-no-ai")
    /// `-hold-reading`: reading aloud plays no audio and holds its state (the Skip/Stop bar) until Skip or Stop,
    /// so a UI test sees it however soon the Simulator's voice would end, or fail to start.
    public static let holdReading = flag("-hold-reading")

    private static func flag(_ name: String) -> Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains(name)
        #else
        false
        #endif
    }
}

/// App preferences (UserDefaults). Nothing secret is stored here.
public enum Prefs {
    public static let serverURL = "serverURL"
    public static let orchestratorId = "orchestratorAgentId"
    public static let appearance = "appearance"
    public static let speechLocale = "speechLocale"
    public static let speechOnDeviceOnly = "speechOnDeviceOnly"
    /// The server's grill command phrases (JSON), cached for offline matching.
    public static let grillPhrases = "grillPhrases"
    /// AVSpeechSynthesisVoice identifier for reading replies aloud; empty means automatic.
    public static let speechVoice = "speechVoice"
    public static let notifyApprovals = "notifyApprovals"
    public static let notifyTaskOutcomes = "notifyTaskOutcomes"
    public static let notifyFailures = "notifyFailures"
    public static let voiceHandsFree = "voiceHandsFree"
    public static let voiceReadReplies = "voiceReadReplies"
    /// Read each new interview question aloud (its short `speak` version).
    public static let grillReadAloud = "grillReadAloud"
    /// While a reply or question is read in a hands-free conversation, listen for "stop" / "skip" / "repeat".
    public static let voiceInterrupt = "voiceInterrupt"
    /// Read the opening of a long reply first; "more" reads the rest.
    public static let voiceShortRead = "voiceShortRead"
    /// Push: quiet hours on/off and their "HH:mm" bounds (per-kind switches are `PushKind.prefKey`).
    public static let pushQuietHours = "pushQuietHours"
    public static let pushQuietStart = "pushQuietStart"
    public static let pushQuietEnd = "pushQuietEnd"
    /// Settings › Apple Intelligence: use the on-device model for voice commands the phrase lists can't tell.
    public static let aiCommands = "aiCommands"
    /// Settings › Apple Intelligence: with "Read long replies short", the on-device model writes the short version.
    public static let aiShortSpoken = "aiShortSpoken"
    /// One-line summaries and suggested replies in "Needs you".
    public static let aiNeedsYou = "aiNeedsYou"
    /// The Mac's quick search shortcut (`MacShortcut`'s raw value; "off" turns it off).
    public static let quickSearchShortcut = "quickSearchShortcut"
    /// The Mac's voice panel shortcut (`MacShortcut`'s raw value; "off" turns it off).
    public static let voicePanelShortcut = "voicePanelShortcut"
    /// Where Enter opens a quick search result on the Mac: "app" or "browser" (⌥Enter opens it in the other).
    public static let quickSearchOpensIn = "quickSearchOpensIn"
    /// The Mac puts Loft's agents, issues and specs in Spotlight; off clears them from it.
    public static let spotlight = "spotlight"
    /// When the app was last in front: the digest covers what happened since.
    public static let digestLastLooked = "digestLastLooked"
    /// A one-line first reply from the on-device model right after you send to the Orchestrator.
    public static let aiQuickAck = "aiQuickAck"
    /// "Send to <agent>?" when a message to the Orchestrator looks meant for one agent.
    public static let aiRoute = "aiRoute"
    /// "Make it a brief" under a long dictation (Claude on the Mac, `voice_brief`).
    public static let voiceBrief = "voiceBrief"
    /// Talk: "Send it?" after the tidied text has waited untouched in the message box for a few seconds.
    public static let talkAskToSend = "talkAskToSend"
    /// Talk: while the text waits in the message box, "send it" said aloud sends it.
    public static let talkSayToSend = "talkSayToSend"
    /// Suggested one-tap replies under an agent's question in a chat.
    public static let aiChatReplies = "aiChatReplies"
    /// "What does this do?" on a permission card.
    public static let aiExplain = "aiExplain"

    public static func registerDefaults() {
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
            talkAskToSend: true,
            talkSayToSend: true,
            aiChatReplies: true,
            aiExplain: true,
            quickSearchShortcut: "option-space",
            voicePanelShortcut: "shift-option-space",
            quickSearchOpensIn: "app",
            spotlight: true,
        ])
    }
}

extension Date {
    /// "now", "4m", "3h", "2d", or a short date.
    public var shortAgo: String {
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
    public func durationSince(_ now: Date = Date()) -> String {
        let s = max(0, Int(now.timeIntervalSince(self)))
        if s < 60 { return "\(s)s" }
        if s < 3600 { return "\(s / 60)m" }
        return String(format: "%dh %02dm", s / 3600, (s % 3600) / 60)
    }
}
