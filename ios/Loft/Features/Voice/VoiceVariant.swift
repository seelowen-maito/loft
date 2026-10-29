import SwiftUI

/// Temporary: three takes on the Talk screen to compare (Settings › Voice style, or `-voiceVariant a|b|c`).
/// One is kept and the rest deleted once picked.
/// - A "Bars": rising bars, a caption-style transcript, and Done sends the tidied text after a 3 s Undo (no review screen).
/// - B "Chat": a pulsing ring, the transcript as the message bubble, and Done sends what was heard straight into the chat.
/// - C "Direct": a voice line, the transcript as live captions, and a To: chip to talk to any agent instead of the Lead.
/// - D "Conversation" (round 2, like Grok or ChatGPT voice): a short silence ends your turn and sends it, the reply is
///   read with captions, talking over it is your turn, and it listens again. One big state ring, one End button.
/// - E "Dictate" (round 2, like Wispr Flow): talk (or hold the mic) and let go; the tidied text lands in the chat's
///   message box, to read and send with one tap. A To: row with recent agents.
enum VoiceVariant: String, CaseIterable {
    case current, a, b, c, d, e

    static let key = "voiceVariant"
    static var selected: VoiceVariant { VoiceVariant(rawValue: UserDefaults.standard.string(forKey: key)?.lowercased() ?? "") ?? .current }

    var title: String {
        switch self {
        case .current: "Current"
        case .a: "A · Bars, send with Undo"
        case .b: "B · Bubble, send into the chat"
        case .c: "C · Captions, talk to any agent"
        case .d: "D · Conversation, hands-free"
        case .e: "E · Dictate into the chat"
        }
    }
}

// MARK: - Animations

/// A: bars that rise with the voice, taller in the middle, each moving at its own pace.
struct VoiceBars: View {
    var level: Float
    var active: Bool
    var tint: Color = Theme.accent

    var body: some View {
        TimelineView(.animation(paused: !active || TestMode.noAnimations)) { t in
            let phase = t.date.timeIntervalSinceReferenceDate
            let l = active ? CGFloat(min(max(level, 0), 1)) : 0
            HStack(spacing: 5) {
                ForEach(0..<21, id: \.self) { i in
                    let centre = 1 - abs(CGFloat(i) - 10) / 12
                    let wobble = 0.35 + 0.65 * abs(sin(phase * (2.2 + Double(i % 5) * 0.7) + Double(i)))
                    Capsule()
                        .fill(tint.opacity(0.55 + 0.45 * centre))
                        .frame(width: 6, height: 8 + 110 * l * centre * wobble)
                }
            }
            .frame(height: 130)
        }
        .animation(.easeOut(duration: 0.1), value: level)
        .accessibilityHidden(true)
    }
}

/// B: a mic disc with rings that swell with the voice.
struct VoicePulse: View {
    var level: Float
    var active: Bool
    var tint: Color = Theme.accent

    var body: some View {
        let l = active ? CGFloat(min(max(level, 0), 1)) : 0
        ZStack {
            ForEach(0..<3, id: \.self) { i in
                Circle()
                    .stroke(tint.opacity(0.45 - Double(i) * 0.12), lineWidth: 2)
                    .scaleEffect(0.55 + l * (0.18 + CGFloat(i) * 0.14) + CGFloat(i) * 0.08)
            }
            Circle().fill(tint.gradient).scaleEffect(0.5)
            Image(systemName: "mic.fill").font(.system(size: 34, weight: .semibold)).foregroundStyle(.white)
        }
        .animation(.spring(duration: 0.25), value: level)
        .aspectRatio(1, contentMode: .fit)
        .frame(maxWidth: 180)
        .accessibilityHidden(true)
    }
}

/// C: a soft line that waves with the voice, three strands slightly out of step.
struct VoiceLine: View {
    var level: Float
    var active: Bool
    var tint: Color = Theme.accent

    var body: some View {
        TimelineView(.animation(paused: !active || TestMode.noAnimations)) { t in
            let phase = t.date.timeIntervalSinceReferenceDate
            let l = active ? CGFloat(min(max(level, 0), 1)) : 0
            Canvas { ctx, size in
                for strand in 0..<3 {
                    var p = Path()
                    let amp = size.height / 2 * (0.08 + 0.9 * l) * (1 - CGFloat(strand) * 0.28)
                    for x in stride(from: 0, through: size.width, by: 2) {
                        let u = x / size.width
                        // Fades to flat at both ends.
                        let envelope = sin(.pi * u)
                        let y = size.height / 2 + amp * envelope * sin(u * 4 * .pi + phase * (3 + Double(strand)) + Double(strand))
                        x == 0 ? p.move(to: CGPoint(x: x, y: y)) : p.addLine(to: CGPoint(x: x, y: y))
                    }
                    ctx.stroke(p, with: .color(tint.opacity(1 - Double(strand) * 0.3)), lineWidth: strand == 0 ? 3 : 1.5)
                }
            }
            .frame(height: 120)
        }
        .animation(.easeOut(duration: 0.1), value: level)
        .accessibilityHidden(true)
    }
}

// MARK: - Transcripts

/// A: the last few lines in large type, the words still being recognised in the accent colour.
struct CaptionTranscript: View {
    var committed: String
    var live: String

    var body: some View {
        Group {
            if committed.isEmpty && live.isEmpty {
                Text("Start talking…").foregroundStyle(.tertiary)
            } else {
                Text(committed) + Text(committed.isEmpty || live.isEmpty ? "" : " ") + Text(live).foregroundStyle(Theme.accent)
            }
        }
        .font(.title2.weight(.medium))
        .lineLimit(4)
        .truncationMode(.head)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, Theme.gutter)
        .accessibilityIdentifier("voiceTranscript")
    }
}

/// B: the message as it will look in the chat, growing as you talk.
struct BubbleTranscript: View {
    var text: String
    var agentName: String

    var body: some View {
        VStack(alignment: .trailing, spacing: 6) {
            ScrollView {
                Text(text.isEmpty ? "Start talking…" : text)
                    .font(.body)
                    .foregroundStyle(text.isEmpty ? Color.white.opacity(0.7) : .white)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(14)
            }
            .defaultScrollAnchor(.bottom)
            .scrollBounceBehavior(.basedOnSize)
            .fixedSize(horizontal: false, vertical: text.count < 160)
            .frame(maxHeight: 220)
            .background(Theme.accent.gradient, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            Text("To \(agentName), as you say it").font(.caption).foregroundStyle(.secondary)
        }
        .padding(.leading, 48)
        .padding(.horizontal, Theme.gutter)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("voiceTranscript")
    }
}

/// C: live captions. The sentence being said is large; the ones before it fade above.
struct LiveCaptions: View {
    var committed: String
    var live: String

    var body: some View {
        let earlier = committed
        let now = live.isEmpty ? "" : live
        VStack(alignment: .leading, spacing: 10) {
            if !earlier.isEmpty {
                Text(earlier)
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                    .truncationMode(.head)
            }
            Text(now.isEmpty ? (earlier.isEmpty ? "Start talking…" : "…") : now)
                .font(.title.weight(.semibold))
                .foregroundStyle(now.isEmpty ? .tertiary : .primary)
                .lineLimit(3)
                .truncationMode(.head)
                .contentTransition(.opacity)
                .animation(.easeOut(duration: 0.15), value: now)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, Theme.gutter)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("voiceTranscript")
    }
}

// MARK: - After sending

/// A and C: what happens after Send, in two steps. "Sending to …" until the server has the message,
/// then "… is thinking" with the agent's live status, so it's always clear who has it and what they're doing.
struct VoiceProgress: View {
    @Environment(LoftStore.self) private var store
    let agentId: String

    var body: some View {
        let agent = store.agent(agentId)
        let name = agent?.name ?? "Agent"
        let sending = !store.pending(for: agentId).isEmpty
        VStack(spacing: 18) {
            HStack(spacing: 10) {
                step("Sent", done: !sending, active: sending)
                Rectangle().fill(.quaternary).frame(width: 24, height: 2)
                step("Thinking", done: false, active: !sending)
                Rectangle().fill(.quaternary).frame(width: 24, height: 2)
                step("Reply", done: false, active: false)
            }
            Text(sending ? "Sending to \(name)…" : "\(name) is thinking…").font(.title3.weight(.semibold))
            if !sending, let status = agent?.statusText, !status.isEmpty {
                Label(status, systemImage: "gearshape.2").font(.footnote).foregroundStyle(.secondary).lineLimit(2)
            }
            if !sending, let since = agent?.lastRunAt {
                RelativeTime(date: since).font(.caption.monospacedDigit()).foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, Theme.gutter)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("voiceProgress")
    }

    private func step(_ title: String, done: Bool, active: Bool) -> some View {
        VStack(spacing: 4) {
            ZStack {
                Circle().fill(done ? Theme.accent : active ? Theme.working.opacity(0.25) : Color.secondary.opacity(0.15))
                if done {
                    Image(systemName: "checkmark").font(.caption.weight(.bold)).foregroundStyle(.white)
                } else if active {
                    ProgressView().controlSize(.mini)
                }
            }
            .frame(width: 26, height: 26)
            Text(title).font(.caption2).foregroundStyle(active || done ? .primary : .secondary)
        }
    }
}

// MARK: - D: conversation

/// The one thing D shows: a big ring that says whose turn it is. Listening: it fills as you pause, and the turn ends
/// when it's full. Thinking: it turns while the agent works. Speaking: it pulses with the reply.
struct VoiceStateRing: View {
    var state: VoiceLoop.State
    var level: Float
    /// 0…1 of the silence that ends your turn.
    var progress: Double

    private var tint: Color {
        switch state {
        case .listening: Theme.accent
        case .thinking: Theme.attention
        case .speaking: Theme.working
        case .ended: .secondary
        }
    }
    private var title: String {
        switch state {
        case .listening: "Listening"
        case .thinking: "Thinking"
        case .speaking: "Speaking"
        case .ended: "Ended"
        }
    }

    var body: some View {
        let l = CGFloat(min(max(level, 0), 1))
        ZStack {
            Circle().stroke(tint.opacity(0.18), lineWidth: 10)
            if state == .listening {
                Circle().trim(from: 0, to: progress)
                    .stroke(tint, style: StrokeStyle(lineWidth: 10, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .animation(.linear(duration: 0.25), value: progress)
            } else if state == .thinking {
                TimelineView(.animation(paused: TestMode.noAnimations)) { t in
                    Circle().trim(from: 0, to: 0.25)
                        .stroke(tint, style: StrokeStyle(lineWidth: 10, lineCap: .round))
                        .rotationEffect(.degrees(t.date.timeIntervalSinceReferenceDate * 240))
                }
            }
            Circle().fill(tint.gradient)
                .scaleEffect(0.62 + (state == .listening || state == .speaking ? l * 0.18 : 0))
                .animation(.spring(duration: 0.2), value: level)
            Text(title).font(.title2.weight(.bold)).foregroundStyle(.white)
        }
        .frame(width: 220, height: 220)
        .accessibilityElement()
        .accessibilityLabel(title)
        .accessibilityIdentifier("voiceStateRing")
    }
}

// MARK: - E: dictate

/// E's To: row: whoever it goes to now, then the agents that worked most recently. One tap switches.
struct RecentAgentsRow: View {
    @Environment(LoftStore.self) private var store
    @Environment(VoiceSession.self) private var voice

    var body: some View {
        let current = voice.agentId
        let recent = store.agents.sorted { ($0.lastRunAt ?? .distantPast) > ($1.lastRunAt ?? .distantPast) }
        let shown = (recent.filter { $0.id == current } + recent.filter { $0.id != current }).prefix(5)
        ScrollView(.horizontal) {
            HStack(spacing: 8) {
                Text("To").font(.subheadline).foregroundStyle(.secondary)
                ForEach(Array(shown)) { a in
                    Button { voice.switchAgent(a.id) } label: {
                        Text(a.name).font(.subheadline.weight(a.id == current ? .semibold : .regular))
                            .padding(.horizontal, 12).padding(.vertical, 6)
                            .background(a.id == current ? Theme.accent.opacity(0.25) : Color.secondary.opacity(0.12), in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(a.id == current ? .isSelected : [])
                }
            }
            .padding(.horizontal, Theme.gutter)
        }
        .scrollIndicators(.hidden)
        .accessibilityIdentifier("voiceRecentAgents")
    }
}

/// E's mic: you're already talking when it opens. Let go of it after holding, or tap it, and you're done.
struct HoldToTalkButton: View {
    var listening: Bool
    var done: () -> Void
    @GestureState private var pressed = false

    var body: some View {
        Image(systemName: listening ? "waveform" : "mic.fill")
            .font(.system(size: 34, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 96, height: 96)
            .background(Theme.accent.gradient, in: Circle())
            .scaleEffect(pressed ? 1.12 : 1)
            .animation(.spring(duration: 0.2), value: pressed)
            .gesture(DragGesture(minimumDistance: 0).updating($pressed) { _, s, _ in s = true }.onEnded { _ in done() })
            .accessibilityElement()
            .accessibilityLabel("Done talking")
            .accessibilityHint("Your words are tidied up and put in the chat's message box.")
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { done() }
            .accessibilityIdentifier("voiceHoldToTalk")
    }
}
