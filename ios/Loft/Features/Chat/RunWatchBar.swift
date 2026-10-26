import SwiftUI

/// Above the composer while a run looks stuck: what it's waiting on (or how long it's been quiet),
/// with Restart, which stops the run and keeps the queued messages.
struct RunWatchBar: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dynamicTypeSize) private var typeSize
    let agentId: String
    @State private var restarting = false
    @State private var error: APIError?

    var body: some View {
        // Re-evaluated every 30 s so "· 14 min" keeps counting without new events.
        TimelineView(.periodic(from: .now, by: 30)) { context in
            if store.info?.supports(.runWatchdog) == true, let agent = store.agent(agentId),
               let watch = RunWatch.of(agent, newestMessage: store.conversation(agentId).last?.createdAt, now: context.date) {
                bar(watch)
            }
        }
        .errorAlert($error)
    }

    private func bar(_ watch: RunWatch) -> some View {
        // At accessibility sizes it shares the space with the Sending and read-aloud bars: one short line.
        let ax = typeSize.isAccessibilitySize
        return HStack(spacing: 10) {
            Label(ax ? watch.shortText : watch.text, systemImage: { if case .waiting = watch { "hourglass" } else { "exclamationmark.triangle" } }())
                .font(.footnote)
                .foregroundStyle(.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
                .layoutPriority(1)
                .accessibilityLabel(watch.text)
            Button {
                Task {
                    restarting = true
                    defer { restarting = false }
                    do { try await store.restartRun(agentId) } catch { self.error = APIError.from(error) }
                }
            } label: {
                // At accessibility sizes the word took half the bar and squeezed the status: an icon (VoiceOver says Restart).
                if restarting {
                    ProgressView().controlSize(.small)
                } else if ax {
                    Image(systemName: "arrow.clockwise").accessibilityLabel("Restart")
                } else {
                    Text("Restart").lineLimit(1).fixedSize()
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(restarting || !store.canWrite)
            .accessibilityHint("Stops this run. Queued messages are kept.")
            .accessibilityIdentifier("runRestart")
        }
        .padding(ax ? 6 : 10)
        // Tinted like the chat's "blocked" cards, so it stands out in light mode too.
        .background(Theme.attention.opacity(0.14), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay { RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(Theme.attention.opacity(0.35), lineWidth: 1) }
        .padding(.horizontal, Theme.gutter)
        .padding(.top, 6)
        .background(.bar)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("runWatchBar")
    }
}
