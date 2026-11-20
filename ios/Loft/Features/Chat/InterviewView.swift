import LoftKit
import SwiftUI

/// The open interview on its own, full screen over the chat: the question the voice is on as one big card, the
/// same card the chat shows. The chat underneath keeps reading each new question aloud. It closes by itself when
/// the interview ends; Close goes back to the chat with the interview still open. Commands are sent from here, so a
/// failure shows here and not in the chat's alert under the cover.
struct InterviewView: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let agent: Agent
    @State private var error: APIError?

    private var session: GrillSession? { store.openGrill(for: agent.id) }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let s = session, let t = s.topic, !t.isEmpty {
                        Text(t).font(.headline).foregroundStyle(.secondary)
                    }
                    content
                }
                .padding(Theme.gutter)
            }
            .scrollDismissesKeyboard(.interactively)
            .background(Theme.background)
            .navigationTitle("Interview")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") { dismiss() }.accessibilityIdentifier("interviewClose")
                }
            }
        }
        .errorAlert($error)
        // The chat underneath turns the mic off when the interview closes, however it ended.
        .onChange(of: session == nil) { _, ended in if ended { dismiss() } }
    }

    @ViewBuilder private var content: some View {
        if let s = session, let q = store.currentGrillQuestion(for: agent.id) {
            GrillQuestionCard(question: q, number: s.number(of: q.id) ?? 1, total: s.questions?.count ?? 1,
                              sessionId: s.sessionId, language: s.language, agent: agent, isCurrent: true,
                              command: command, roundActions: true)
                .id(q.id)
        } else if let s = session, s.hasOpenRound {
            // Every question has an answer: the round goes when you send it.
            VStack(alignment: .leading, spacing: 12) {
                Label("All questions answered", systemImage: "checkmark.circle").font(.title3.weight(.semibold))
                Button { command(.send) } label: {
                    Label("Send round", systemImage: "paperplane.fill").frame(minHeight: 32)
                }
                .buttonStyle(.accent)
                .disabled(!store.canWrite)
                .accessibilityIdentifier("grillSend")
            }
        } else {
            Label(session?.state == .preparing ? "Preparing questions…" : "Next round…", systemImage: "hourglass")
                .font(.title3)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("interviewWaiting")
        }
    }

    private func command(_ c: GrillCommand) {
        Task {
            do { try await store.sendGrillCommand(c, agentId: agent.id) } catch { self.error = APIError.from(error) }
        }
    }
}
