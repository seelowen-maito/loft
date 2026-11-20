import LoftKit
import SwiftUI
import UIKit

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
