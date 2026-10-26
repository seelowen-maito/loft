import SwiftUI

// "Grill me" interviews render inline in the normal chat: questions are cards in the message
// stream, the waiting question carries its answer chips and quick actions, commands come back as
// small status rows, and a one-line chip above the composer says an interview is going.

/// An interviewer's question. The waiting one shows answer chips and Repeat · Skip · Back · I don't know.
struct GrillQuestionRow: View {
    @Environment(LoftStore.self) private var store
    @Environment(Speaker.self) private var speaker
    @Environment(\.dynamicTypeSize) private var typeSize
    let message: ChatMessage
    let agent: Agent
    /// True for the question waiting for an answer.
    let isCurrent: Bool
    /// Puts a suggested answer in the composer; the user still taps Send.
    let choose: (String) -> Void
    let command: (GrillCommand) -> Void
    @State private var confirmEnd = false

    private var g: GrillMeta { message.grill ?? GrillMeta(sessionId: nil, role: .question) }
    private var paused: Bool { store.openGrill(for: agent.id)?.state == .paused }

    private var caption: String {
        guard let i = g.index else { return "Question" }
        return g.total.map { "Question \(i) of \($0)" } ?? "Question \(i)"
    }

    /// Suggested answers: the agent's, or Yes / No for a yes-no question without any.
    private var choices: [String] {
        if let o = g.options, !o.isEmpty { return o }
        return g.kind == .yesNo ? ["Yes", "No"] : []
    }

    var body: some View {
        HStack(alignment: .bottom) {
            VStack(alignment: .leading, spacing: 10) {
                Label(caption, systemImage: "questionmark.bubble")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Theme.accent)
                CollapsibleText(text: message.displayContent)
                    .font(.body.weight(.medium))
                if isCurrent {
                    if paused {
                        pausedRow
                    } else {
                        if !choices.isEmpty { choiceChips }
                        actions
                    }
                }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(Theme.accent.opacity(isCurrent ? 0.45 : 0.15), lineWidth: 1)
            }
            Spacer(minLength: typeSize.isAccessibilitySize ? 0 : 24)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(caption). \(message.displayContent)\(choices.isEmpty ? "" : ". Suggested answers: " + choices.joined(separator: ", "))")
        .accessibilityIdentifier(isCurrent ? "grillCurrentQuestion" : "grillQuestion")
        .confirmationDialog("End the interview?", isPresented: $confirmEnd, titleVisibility: .visible) {
            Button("End", role: .destructive) { command(.end) }
        } message: {
            Text("\(agent.name) will sum up what you covered.")
        }
    }

    private var choiceChips: some View {
        let layout = typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8)) : AnyLayout(FlowLayout(spacing: 8))
        return layout {
            ForEach(choices, id: \.self) { c in
                Button { choose(c) } label: {
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
                .accessibilityHint("Puts this answer in the message field")
                .accessibilityIdentifier("grillChoice-\(c)")
            }
        }
    }

    /// Repeat · Skip · Back · I don't know · More: one row when it fits, else two fixed rows, else a column
    /// (fixed rows keep the spacing even; a wrapping layout left a tall gap between them).
    private var actions: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 0) { repeatButton; skipButton; backButton; decideButton; moreMenu }
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 0) { repeatButton; skipButton; backButton }
                HStack(spacing: 0) { decideButton; moreMenu }
            }
            VStack(alignment: .leading, spacing: 0) { repeatButton; skipButton; backButton; decideButton; moreMenu }
        }
        .font(.footnote.weight(.semibold))
    }

    private var repeatButton: some View {
        action("Repeat", "speaker.wave.2", id: "grillRepeat", enabled: true) {
            Task { await speaker.speak(g.speak ?? message.displayContent, messageId: message.id) }
        }
    }
    private var skipButton: some View { action("Skip", "forward", id: "grillSkip", enabled: store.canWrite) { command(.skip) } }
    private var backButton: some View {
        action("Back", "backward", id: "grillBack", enabled: store.canWrite && (g.index ?? 1) > 1) { command(.back) }
    }
    private var decideButton: some View { action("I don't know", "dice", id: "grillDecide", enabled: store.canWrite) { command(.decide) } }

    private var moreMenu: some View {
        Menu {
            Button { command(.pause) } label: { Label("Pause", systemImage: "pause") }
            Button { command(.summarize) } label: { Label("Summary so far", systemImage: "text.alignleft") }
            Button(role: .destructive) { confirmEnd = true } label: { Label("End interview", systemImage: "stop") }
        } label: {
            Label("More", systemImage: "ellipsis.circle")
                .foregroundStyle(Theme.accent)
                .padding(.horizontal, 10)
                .frame(minHeight: 44)
                .contentShape(Rectangle())
        }
        .disabled(!store.canWrite)
        .accessibilityLabel("More interview actions")
        .accessibilityIdentifier("grillMore")
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

    /// "Paused" is already on the status row and the chip; the card just offers the way back.
    private var pausedRow: some View {
        HStack(spacing: 8) {
            Button { command(.resume) } label: {
                Label("Resume interview", systemImage: "play.fill").frame(minHeight: 44)
            }
            .buttonStyle(.borderless)
            .disabled(!store.canWrite)
            .accessibilityIdentifier("grillResume")
        }
        .font(.footnote.weight(.semibold))
    }
}

/// "Skipped question 3", "Paused", "Ended": a small centered row, like a divider.
struct GrillStatusRow: View {
    let message: ChatMessage

    private var symbol: String {
        let g = message.grill
        switch g?.command {
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
