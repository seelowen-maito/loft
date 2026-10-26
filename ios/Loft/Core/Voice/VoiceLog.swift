import Foundation
import SwiftUI

/// Every end-of-speech and auto-send decision (heard, pause, countdown, sent, held and why), kept on
/// the phone so a real-device test can be read back in Settings › Voice › Voice log. Also goes to the
/// system log (category "speech"). Holds no audio; dictated text is shortened.
@MainActor
@Observable
final class VoiceLog {
    static let shared = VoiceLog()

    struct Line: Identifiable {
        let id = UUID()
        let at: Date
        let text: String
    }

    private(set) var lines: [Line] = []
    private let limit = 300

    func add(_ text: String) {
        Log.speech.info("\(text, privacy: .public)")
        lines.append(Line(at: Date(), text: text))
        if lines.count > limit { lines.removeFirst(lines.count - limit) }
    }

    func clear() { lines.removeAll() }

    /// The whole log as plain text, for copying into a bug report.
    var plain: String {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss.SSS"
        return lines.map { "\(f.string(from: $0.at)) \($0.text)" }.joined(separator: "\n")
    }

    /// A dictation shortened for the log: its length and last few words.
    static func clip(_ s: String) -> String {
        let words = s.split(whereSeparator: \.isWhitespace)
        let tail = words.suffix(5).joined(separator: " ")
        return "\(s.count) chars, “…\(tail)”"
    }
}

/// Settings › Voice › Voice log.
struct VoiceLogView: View {
    private var log: VoiceLog { .shared }

    var body: some View {
        List {
            if log.lines.isEmpty {
                Text("Nothing yet. Dictate something and come back.").foregroundStyle(.secondary)
            }
            ForEach(log.lines.reversed()) { line in
                VStack(alignment: .leading, spacing: 2) {
                    Text(line.at, format: .dateTime.hour().minute().second())
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                    Text(line.text).font(.footnote)
                }
                .accessibilityElement(children: .combine)
            }
        }
        .navigationTitle("Voice log")
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button("Copy") { UIPasteboard.general.string = log.plain }
                    .disabled(log.lines.isEmpty)
                Button("Clear", role: .destructive) { log.clear() }
                    .disabled(log.lines.isEmpty)
            }
        }
        .accessibilityIdentifier("voiceLog")
    }
}
