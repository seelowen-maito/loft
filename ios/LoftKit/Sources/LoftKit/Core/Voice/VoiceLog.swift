import Foundation

/// Every end-of-speech and auto-send decision (heard, pause, countdown, sent, held and why), kept on
/// the phone so a real-device test can be read back in Settings › Voice › Voice log. Also goes to the
/// system log (category "speech"). Holds no audio; dictated text is shortened.
@MainActor
@Observable
public final class VoiceLog {
    public static let shared = VoiceLog()

    public struct Line: Identifiable {
        public let id = UUID()
        public let at: Date
        public let text: String
    }

    public private(set) var lines: [Line] = []
    private let limit = 300

    public func add(_ text: String) {
        Log.speech.info("\(text, privacy: .public)")
        lines.append(Line(at: Date(), text: text))
        if lines.count > limit { lines.removeFirst(lines.count - limit) }
    }

    public func clear() { lines.removeAll() }

    /// The whole log as plain text, for copying into a bug report.
    public var plain: String {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss.SSS"
        return lines.map { "\(f.string(from: $0.at)) \($0.text)" }.joined(separator: "\n")
    }

    /// A dictation shortened for the log: its length and last few words.
    public static func clip(_ s: String) -> String {
        let words = s.split(whereSeparator: \.isWhitespace)
        let tail = words.suffix(5).joined(separator: " ")
        return "\(s.count) chars, “…\(tail)”"
    }
}
