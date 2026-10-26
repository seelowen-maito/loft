import SwiftUI

/// "Suggested" replies from the on-device model: dashed, so they read as suggestions rather than actions. A tap
/// fills in the reply; you send it.
struct SuggestedReplies: View {
    let replies: [String]
    let pick: (String) -> Void

    var body: some View {
        HStack(spacing: 6) {
            Text("Suggested").font(.caption2).foregroundStyle(.secondary)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(replies, id: \.self) { r in
                        Button { pick(r) } label: {
                            Text(r).font(.footnote).padding(.horizontal, 10).padding(.vertical, 5)
                                .overlay(Capsule().strokeBorder(Theme.accent.opacity(0.5), style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
                                .frame(minHeight: 44)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(Theme.accent)
                        .accessibilityLabel("Suggested reply: \(r)")
                        .accessibilityHint("Fills in the reply; you send it")
                    }
                }
            }
        }
    }
}
