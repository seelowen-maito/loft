import SwiftUI

// "Grill me" interviews render inline in the normal chat: each round the agent posts is a card in the
// message stream. The round waiting for answers shows a card per question with its recommendation, Accept,
// Decide for me and an answer field, and a Send round action; earlier rounds show their questions read-only.
// Commands come back as small status rows.

/// A round the agent posted (`meta.grill.role == "round"`). The open one is fed by the live session.
struct GrillRoundCard: View {
    @Environment(LoftStore.self) private var store
    let message: ChatMessage
    let agent: Agent
    /// The question the voice is on, which carries Repeat · Skip · Back.
    let currentQuestionId: String?
    let command: (GrillCommand) -> Void
    @State private var confirmEnd = false

    private var g: GrillMeta { message.grill ?? GrillMeta(sessionId: nil, role: .round) }
    private var session: GrillSession? { store.openGrill(for: agent.id) }
    /// This round is the one waiting for answers.
    private var open: GrillSession? {
        guard let s = session, s.sessionId == g.sessionId, s.round == g.round, s.hasOpenRound else { return nil }
        return s
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 4) {
                Label("Round \(g.round ?? 1)", systemImage: "questionmark.bubble")
                    .foregroundStyle(open == nil ? Color.secondary : Theme.accent)
                if let t = g.topic, !t.isEmpty { Text("· \(t)").foregroundStyle(.secondary).lineLimit(1) }
            }
            .font(.caption.weight(.semibold))
            if let s = open, let qs = s.questions {
                ForEach(Array(qs.enumerated()), id: \.element.id) { i, q in
                    GrillQuestionCard(question: q, number: i + 1, total: qs.count, sessionId: s.sessionId, agent: agent,
                                      isCurrent: q.id == currentQuestionId, command: command)
                }
                footer(s, qs)
            } else {
                ForEach(Array((g.questions ?? []).enumerated()), id: \.element.id) { i, q in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(Markdown.render("\(i + 1). \(q.text)")).font(.subheadline)
                        if !q.recommendation.isEmpty {
                            Text("Recommended: \(q.recommendation)").font(.footnote).foregroundStyle(.secondary).padding(.leading, 14)
                        }
                    }
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(Theme.accent.opacity(open == nil ? 0.15 : 0.45), lineWidth: 1)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(open == nil ? "grillRound" : "grillOpenRound")
        .confirmationDialog("End the interview?", isPresented: $confirmEnd, titleVisibility: .visible) {
            Button("End", role: .destructive) { command(.end) }
        } message: {
            Text("\(agent.name) will sum up what you covered.")
        }
    }

    /// "2 of 3 answered", Send round, and Pause · Summary so far · End; Resume while paused.
    private func footer(_ s: GrillSession, _ qs: [GrillQuestion]) -> some View {
        let answered = qs.filter { $0.state != .open }.count
        return VStack(alignment: .leading, spacing: 8) {
            Divider()
            Text("\(answered) of \(qs.count) answered\(answered < qs.count ? ". Unanswered ones come back next round." : "")")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("grillProgress")
            HStack(spacing: 8) {
                Button { command(.send) } label: {
                    Label("Send round", systemImage: "paperplane.fill").frame(minHeight: 32)
                }
                .buttonStyle(.borderedProminent)
                .disabled(!store.canWrite)
                .accessibilityIdentifier("grillSend")
                if s.state == .paused {
                    Button { command(.resume) } label: { Label("Resume", systemImage: "play.fill").frame(minHeight: 32) }
                        .buttonStyle(.bordered)
                        .disabled(!store.canWrite)
                        .accessibilityIdentifier("grillResume")
                }
                Spacer(minLength: 0)
                Menu {
                    if s.state != .paused {
                        Button { command(.pause) } label: { Label("Pause", systemImage: "pause") }
                    }
                    Button { command(.summarize) } label: { Label("Summary so far", systemImage: "text.alignleft") }
                    Button(role: .destructive) { confirmEnd = true } label: { Label("End interview", systemImage: "stop") }
                } label: {
                    Image(systemName: "ellipsis.circle").font(.title3).frame(width: 44, height: 44)
                }
                .disabled(!store.canWrite)
                .accessibilityLabel("More interview actions")
                .accessibilityIdentifier("grillMore")
            }
        }
    }
}

/// One question of the open round: the recommendation with Accept and Decide for me, chips for its options, and a
/// field for any other answer, saved as a decision or as a comment to discuss. The question the voice is on
/// also has Repeat · Skip · Back.
struct GrillQuestionCard: View {
    @Environment(LoftStore.self) private var store
    @Environment(Speaker.self) private var speaker
    @Environment(\.dynamicTypeSize) private var typeSize
    let question: GrillQuestion
    let number: Int
    let total: Int
    let sessionId: String
    let agent: Agent
    let isCurrent: Bool
    let command: (GrillCommand) -> Void
    /// A chip tapped on the Talk screen answers at once by voice; in the chat it fills the answer field.
    var choose: ((String) -> Void)?
    /// Pause · Summary so far · End on the card itself (the Talk screen, which shows no round footer).
    var roundActions = false
    @State private var draft = ""
    @State private var busy = false
    @State private var error: APIError?

    private var q: GrillQuestion { question }
    /// The answer is the user's own words (not the recommendation accepted or left to the agent).
    private var written: Bool { (q.state == .answered && !q.accepted) || q.state == .discuss }
    private var saved: Bool { written && q.answer == draft.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if isCurrent {
                Label("Question \(number) of \(total) · answer by voice or below", systemImage: "waveform")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Theme.accent)
            } else if let r = q.fromRound {
                Text("Still open from round \(r)").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            }
            CollapsibleText(text: "\(number). \(q.text)").font(.body.weight(.medium))
            if !q.recommendation.isEmpty { recommendation }
            if let o = q.options, !o.isEmpty { chips(o) }
            answerField
            if isCurrent { voiceActions }
            if let error { Text(error.title).font(.footnote).foregroundStyle(Theme.failureInk) }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.background.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Theme.accent.opacity(isCurrent ? 0.6 : 0), lineWidth: 1.5)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(isCurrent ? "grillCurrentQuestion" : "grillQuestion")
        .onAppear { if written { draft = q.answer ?? "" } }
        // Answers given elsewhere (the web, by voice) show here too.
        .onChange(of: q.answer) { _, a in if written { draft = a ?? "" } }
    }

    private var recommendation: some View {
        let layout = typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 6)) : AnyLayout(FlowLayout(spacing: 6))
        return layout {
            Label { Text("Recommended: ").foregroundStyle(.secondary) + Text(q.recommendation) } icon: {
                Image(systemName: "sparkles").foregroundStyle(Theme.accent)
            }
            .font(.subheadline)
            .accessibilityElement(children: .combine)
            if q.accepted {
                Label("Accepted", systemImage: "checkmark").font(.footnote.weight(.semibold)).foregroundStyle(.green)
                    .frame(minHeight: 44)
                    .accessibilityIdentifier("grillAccepted")
            } else {
                Button { answer("Yes") } label: { Label("Accept", systemImage: "checkmark").frame(minHeight: 32) }
                    .buttonStyle(.bordered)
                    .disabled(busy || !store.canWrite)
                    .accessibilityHint("Accepts the recommendation as your answer")
                    .accessibilityIdentifier("grillAccept")
            }
            if q.state == .decided {
                Label("Left to the agent", systemImage: "dice").font(.footnote.weight(.semibold)).foregroundStyle(.green)
                    .frame(minHeight: 44)
            } else {
                Button { run { try await store.sendGrillCommand(.decide, agentId: agent.id, questionId: q.id) } } label: {
                    Label("Decide for me", systemImage: "dice").frame(minHeight: 32)
                }
                .buttonStyle(.borderless)
                .disabled(busy || !store.canWrite)
                .accessibilityHint("The agent's recommendation becomes the decision")
                .accessibilityIdentifier("grillDecide")
            }
        }
    }

    private func chips(_ options: [String]) -> some View {
        let layout = typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8)) : AnyLayout(FlowLayout(spacing: 8))
        return layout {
            ForEach(options, id: \.self) { c in
                Button { if let choose { choose(c) } else { draft = c } } label: {
                    Text(c)
                        .font(.subheadline)
                        .multilineTextAlignment(.leading)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .frame(minHeight: 44)
                        .frame(maxWidth: typeSize.isAccessibilitySize ? .infinity : nil, alignment: .leading)
                        .background(Theme.accent.opacity(0.12), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityHint(choose == nil ? "Puts this answer in the field" : "Answers with this")
                .accessibilityIdentifier("grillChoice-\(c)")
            }
        }
    }

    /// Your own answer, saved as a decision (Return) or as a comment to discuss.
    private var answerField: some View {
        HStack(spacing: 6) {
            TextField(q.accepted || q.state == .decided ? "Or answer differently…" : "Your answer, or a comment…", text: $draft, axis: .vertical)
                .lineLimit(1...4)
                .textFieldStyle(.roundedBorder)
                .submitLabel(.done)
                .onSubmit { answer(draft) }
                .accessibilityLabel("Answer to question \(number)")
                .accessibilityIdentifier("grillAnswerField")
            let empty = draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            if saved && q.state == .answered {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green).frame(width: 44, height: 44)
                    .accessibilityLabel("Saved")
            } else {
                Button { answer(draft) } label: { Image(systemName: "arrow.turn.down.left").frame(width: 44, height: 44) }
                    .disabled(busy || empty || !store.canWrite)
                    .accessibilityLabel("Save as your answer")
                    .accessibilityIdentifier("grillSaveAnswer")
            }
            if saved && q.state == .discuss {
                Image(systemName: "bubble.left.and.bubble.right.fill").foregroundStyle(Theme.accent).frame(width: 44, height: 44)
                    .accessibilityLabel("To discuss")
            } else {
                Button { answer(draft, discuss: true) } label: { Image(systemName: "bubble.left.and.bubble.right").frame(width: 44, height: 44) }
                    .disabled(busy || empty || !store.canWrite)
                    .accessibilityLabel("Discuss instead of deciding")
                    .accessibilityHint("The agent explains and asks again next round")
                    .accessibilityIdentifier("grillDiscuss")
            }
        }
    }

    /// Repeat · Skip · Back on the question the voice is on.
    private var voiceActions: some View {
        HStack(spacing: 0) {
            action("Repeat", "speaker.wave.2", id: "grillRepeat", enabled: true) {
                Task { await speaker.speak(q.spoken(number: number, of: total)) }
            }
            action("Skip", "forward", id: "grillSkip", enabled: store.canWrite) { command(.skip) }
            action("Back", "backward", id: "grillBack", enabled: number > 1) { command(.back) }
            if roundActions {
                Menu {
                    Button { command(store.openGrill(for: agent.id)?.state == .paused ? .resume : .pause) } label: {
                        store.openGrill(for: agent.id)?.state == .paused ? Label("Resume", systemImage: "play") : Label("Pause", systemImage: "pause")
                    }
                    Button { command(.send) } label: { Label("Send round now", systemImage: "paperplane") }
                    Button { command(.summarize) } label: { Label("Summary so far", systemImage: "text.alignleft") }
                    Button(role: .destructive) { command(.end) } label: { Label("End interview", systemImage: "stop") }
                } label: {
                    Label("More", systemImage: "ellipsis.circle").padding(.horizontal, 10).frame(minHeight: 44).contentShape(Rectangle())
                }
                .disabled(!store.canWrite)
                .accessibilityLabel("More interview actions")
                .accessibilityIdentifier("grillMore")
            }
        }
        .font(.footnote.weight(.semibold))
    }

    private func action(_ title: String, _ symbol: String, id: String, enabled: Bool, _ run: @escaping () -> Void) -> some View {
        Button(action: run) {
            Label(title, systemImage: symbol)
                .padding(.horizontal, 10)
                .frame(minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .disabled(!enabled)
        .accessibilityIdentifier(id)
    }

    private func answer(_ text: String, discuss: Bool = false) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        run { try await store.answerGrill(t, questionId: q.id, agentId: agent.id, discuss: discuss) }
    }

    private func run(_ f: @escaping () async throws -> Void) {
        busy = true
        error = nil
        Task {
            do { try await f() } catch { self.error = APIError.from(error) }
            busy = false
        }
    }
}

/// "Question 2 left to the agent", "Paused", "Grill ended": a small centered row, like a divider.
struct GrillStatusRow: View {
    let message: ChatMessage

    private var symbol: String {
        let g = message.grill
        switch g?.command {
        case .send: return "paperplane"
        case .skip: return "forward"
        case .back: return "backward"
        case .decide: return "dice"
        case .summarize: return "text.alignleft"
        default: break
        }
        switch g?.state {
        case "started": return "flame"
        case "paused": return "pause.circle"
        case "resumed": return "play.circle"
        case "ended": return "flag.checkered"
        default: return "info.circle"
        }
    }

    var body: some View {
        Label(message.displayContent, systemImage: symbol)
            .font(.caption)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(Theme.card.opacity(0.7), in: Capsule())
            .frame(maxWidth: .infinity)
            .accessibilityIdentifier("grillStatus")
    }
}

/// Lays children out left to right and wraps onto new lines, like text.
struct FlowLayout: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        // Take the whole offered width: answering with the narrower content width made the parent
        // place us in less room than we measured for, so rows wrapped again and overlapped what follows.
        let width = proposal.width.map { $0.isFinite ? $0 : nil } ?? nil
        return CGSize(width: width ?? (rows.map(\.width).max() ?? 0), height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for i in row.indices {
                let size = row.sizes[i - row.indices.lowerBound]
                subviews[i].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(width: size.width, height: size.height))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var indices: Range<Int>
        var sizes: [CGSize]
        var width: CGFloat { sizes.map(\.width).reduce(0, +) }
        var height: CGFloat { sizes.map(\.height).max() ?? 0 }
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var start = 0
        var sizes: [CGSize] = []
        var x: CGFloat = 0
        for (i, sub) in subviews.enumerated() {
            // A child wider than the line gets the whole line and wraps its own text.
            var size = sub.sizeThatFits(.unspecified)
            if size.width > width { size = sub.sizeThatFits(ProposedViewSize(width: width, height: nil)) }
            if !sizes.isEmpty, x + spacing + size.width > width {
                rows.append(Row(indices: start..<i, sizes: sizes))
                start = i; sizes = []; x = 0
            }
            x += (sizes.isEmpty ? 0 : spacing) + size.width
            sizes.append(size)
        }
        if !sizes.isEmpty { rows.append(Row(indices: start..<subviews.count, sizes: sizes)) }
        return rows
    }
}

/// "Skip this question?" for a spoken command the phone wasn't sure about.
struct GrillSuggestionBar: View {
    @Environment(\.dynamicTypeSize) private var typeSize
    let intent: GrillMatcher.Intent
    let yes: () -> Void
    let no: () -> Void

    private var question: String {
        switch intent {
        case .repeatQuestion: "Repeat the question?"
        case .command(.send): "Send the round now?"
        case .command(.skip): "Skip this question?"
        case .command(.back): "Go back to the previous question?"
        case .command(.pause): "Pause the interview?"
        case .command(.resume): "Resume the interview?"
        case .command(.end): "End the interview?"
        case .command(.summarize): "Hear a summary so far?"
        case .command(.decide): "Let the agent decide this one?"
        }
    }

    var body: some View {
        // The question on its own line and the buttons under it, so "No, it's my answer" has room for one line.
        VStack(alignment: .leading, spacing: 8) {
            Label(question, systemImage: "questionmark.circle")
                .font(.subheadline.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
            // Full-width and stacked at accessibility sizes, Yes first.
            let ax = typeSize.isAccessibilitySize
            (ax ? AnyLayout(VStackLayout(spacing: 8)) : AnyLayout(HStackLayout(spacing: 8))) {
                if !ax { Spacer(minLength: 0) }
                if ax { yesButton }
                Button(action: no) {
                    Text("No, it's my answer").multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: ax ? .infinity : nil)
                }
                .buttonStyle(.bordered)
                .accessibilityIdentifier("grillSuggestionNo")
                if !ax { yesButton }
            }
            .controlSize(.regular)
        }
        .padding(10)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("grillSuggestion")
    }

    private var yesButton: some View {
        Button(action: yes) {
            Text("Yes").frame(maxWidth: typeSize.isAccessibilitySize ? .infinity : nil)
        }
        .buttonStyle(.borderedProminent)
        .accessibilityIdentifier("grillSuggestionYes")
    }
}
