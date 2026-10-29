import SwiftUI

/// "Resume a session" from the chat's context menu: the agent's past Sessions, newest first; picking one makes it
/// the agent's Session, as the web's /resume does.
struct ResumeSheet: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let agentId: String
    @State private var list: PastSessions?
    @State private var error: APIError?
    @State private var busy: String?

    private var running: Bool {
        guard let a = store.agent(agentId) else { return false }
        return store.state(of: a) == .working || a.running == true
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    if running {
                        Text("Stop the agent first.").foregroundStyle(Theme.attention)
                    }
                    if let error {
                        Text(error.title).foregroundStyle(.red)
                    }
                    if let list {
                        if list.sessions.isEmpty {
                            Text("No past Sessions for this agent yet.").foregroundStyle(.secondary)
                        }
                        ForEach(list.sessions) { s in
                            let current = s.id == list.current
                            Button { resume(s.id) } label: { row(s, current: current) }
                                .disabled(running || current || busy != nil)
                                .accessibilityIdentifier("pastSession-\(s.id)")
                        }
                    } else if error == nil {
                        HStack(spacing: 8) {
                            ProgressView()
                            Text("Reading the transcripts…").foregroundStyle(.secondary)
                        }
                    }
                } footer: {
                    Text("The agent's next run picks up where that Session left off. Its whole context goes back into the prompt cache at full price, so a big one costs more.")
                }
            }
            .navigationTitle("Resume a session")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
        .task {
            do { list = try await store.pastSessions(agentId) } catch { self.error = APIError.from(error) }
        }
    }

    private func row(_ s: PastSession, current: Bool) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            if busy == s.id { ProgressView() } else { Image(systemName: "clock.arrow.circlepath").foregroundStyle(.secondary) }
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(s.startedAt.formatted(date: .abbreviated, time: .shortened))
                    if current { Text("current").foregroundStyle(Theme.accent) }
                    Spacer()
                    if let n = s.contextTokens {
                        Text("ctx \(ContextChip.short(n))").monospacedDigit()
                            .foregroundStyle(ContextWindow.warns(n) ? Theme.attention : .secondary)
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                (Text(s.fromAgent.map { "\($0): " } ?? "").foregroundStyle(.secondary) + Text(s.firstMessage))
                    .font(.subheadline)
                    .foregroundStyle(.primary)
                    .lineLimit(2)
            }
        }
    }

    private func resume(_ id: String) {
        busy = id
        error = nil
        Task {
            do {
                try await store.resumeSession(agentId, sessionId: id)
                dismiss()
            } catch {
                self.error = APIError.from(error)
                busy = nil
            }
        }
    }
}
