import LoftKit
import SwiftUI

/// The open grill round, over the composer: how many of its questions have an answer, "Anything else?" (the Round
/// note: ideas or questions the round doesn't cover, which the agent asks about next round) and Send round, which sends
/// the round with it. The questions are answered on the web, the iPhone or by voice; after the last one the voice asks
/// "Anything else?", and the bar shows it.
struct RoundBar: View {
    @Environment(LoftStore.self) private var store
    let agentId: String
    @State private var sending = false
    @State private var error: String?

    var body: some View {
        if let s = store.openGrill(for: agentId), s.hasOpenRound, let qs = s.questions {
            let answered = qs.filter { $0.state != .open }.count
            let asked = store.grillAsksNote(for: agentId)
            let hasNote = !store.grillNote(for: agentId).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Label("Round \(s.round ?? 1)", systemImage: "questionmark.bubble").foregroundStyle(Theme.accent)
                    Text("\(answered) of \(qs.count) answered").foregroundStyle(WebTheme.fg3)
                    Spacer()
                }
                .font(.system(size: 12, weight: .medium))
                Label(asked ? "Anything else? · answer by voice or below" : "Anything else?", systemImage: asked ? "waveform" : "text.bubble")
                    .font(.system(size: 12.5))
                    .foregroundStyle(asked ? Theme.accent : WebTheme.fg)
                HStack(alignment: .bottom, spacing: 8) {
                    TextField("Ideas or questions this round doesn't cover: the agent asks about them next round", text: Binding(
                        get: { store.grillNote(for: agentId) },
                        set: { store.setGrillNote($0, agentId: agentId) }
                    ), axis: .vertical)
                        .lineLimit(1...5)
                        .textFieldStyle(.roundedBorder)
                        .accessibilityLabel("Round note: anything else?")
                        .accessibilityIdentifier("grillNote")
                    Button(action: send) {
                        Label(hasNote ? "Send round with note" : "Send round", systemImage: "paperplane.fill")
                    }
                    .buttonStyle(.accent)
                    .disabled(sending || !store.canWrite)
                    .accessibilityIdentifier("grillSend")
                }
                if let error { Text(error).font(.caption).foregroundStyle(Theme.failureInk) }
            }
            .padding(10)
            .background(WebTheme.fg.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Theme.accent.opacity(asked ? 0.6 : 0.25), lineWidth: 1)
            }
            .padding(.horizontal, 12).padding(.top, 8)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("roundBar")
        }
    }

    private func send() {
        sending = true
        error = nil
        Task {
            do { try await store.sendGrillCommand(.send, agentId: agentId) } catch { self.error = APIError.from(error).title }
            sending = false
        }
    }
}
