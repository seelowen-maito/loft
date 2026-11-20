import LoftKit
import SwiftUI

/// The Talk screen's parts: the voice bars, the live captions, the To: row, the mic you hold, and the full-screen
/// status after Send.

// MARK: - Animations

/// Bars that rise with the voice, taller in the middle, each moving at its own pace.
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

// MARK: - Transcripts

/// The last few lines in large type, the words still being recognised in the accent colour.
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

// MARK: - After sending

/// After Send: the whole screen says where the message is. Sent (with Undo) → Thinking, with the agent's own
/// status line and a timer → the reply, read aloud, as a big bubble. The chat is one tap away at every step.
struct VoiceStatusView: View {
    @Environment(LoftStore.self) private var store
    @Environment(VoiceSession.self) private var voice
    let status: VoiceStatus
    let dismiss: () -> Void

    private var agentId: String { voice.agentId ?? "" }

    var body: some View {
        let agent = store.agent(agentId)
        VStack(spacing: 18) {
            Spacer(minLength: 0)
            steps
            Text(status.title(agent: agent?.name ?? "Agent"))
                .font(.title.weight(.bold))
                .multilineTextAlignment(.center)
                .accessibilityIdentifier("voiceStatusTitle")
            detail(agent)
            Spacer(minLength: 0)
            buttons
        }
        .padding(.horizontal, Theme.gutter)
        .padding(.bottom, 12)
        .sensoryFeedback(.impact(weight: .medium), trigger: status)
        .sensoryFeedback(.success, trigger: status == .replied)
        .sensoryFeedback(.error, trigger: status == .failed)
    }

    // MARK: Steps

    private var steps: some View {
        HStack(spacing: 10) {
            step("Sent", index: 0)
            Rectangle().fill(.quaternary).frame(width: 28, height: 2)
            step("Thinking", index: 1)
            Rectangle().fill(.quaternary).frame(width: 28, height: 2)
            step("Reply", index: 2)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(status == .failed ? "Not sent" : ["Sending", "Thinking", "Reply"][status.step])
        .accessibilityIdentifier("voiceStatusSteps")
    }

    private func step(_ title: String, index: Int) -> some View {
        let done = index < status.step || (index == 2 && status == .replied)
        let active = index == status.step && !done
        let failed = status == .failed && index == 0
        return VStack(spacing: 6) {
            ZStack {
                Circle().fill(failed ? Theme.failure : done ? Theme.accent : active ? Theme.working.opacity(0.25) : Color.secondary.opacity(0.15))
                if failed {
                    Image(systemName: "xmark").font(.callout.weight(.bold)).foregroundStyle(.white)
                } else if done {
                    Image(systemName: "checkmark").font(.callout.weight(.bold)).foregroundStyle(Theme.onAccent)
                } else if active && status == .speaking {
                    Image(systemName: "waveform").font(.callout.weight(.bold)).foregroundStyle(Theme.working)
                        .symbolEffect(.variableColor.iterative, isActive: !TestMode.noAnimations)
                } else if active {
                    ProgressView().controlSize(.small)
                }
            }
            .frame(width: 40, height: 40)
            Text(title).font(.subheadline).foregroundStyle(active || done ? .primary : .secondary)
        }
    }

    // MARK: Detail

    @ViewBuilder private func detail(_ agent: Agent?) -> some View {
        switch status {
        case .sending, .thinking:
            if status == .thinking, let s = agent?.statusText, !s.isEmpty {
                Label(s, systemImage: "gearshape.2").font(.body).foregroundStyle(.secondary).lineLimit(2)
                    .accessibilityIdentifier("voiceStatusLine")
            } else if status == .sending, let text = voice.cleaned {
                Text("“\(text)”").font(.body).foregroundStyle(.secondary).lineLimit(4).multilineTextAlignment(.center)
            }
            if let at = voice.sentAt, status == .thinking {
                TimelineView(.periodic(from: at, by: 1)) { ctx in
                    Text(VoiceSessionView.clock(ctx.date.timeIntervalSince(at)))
                        .font(.callout.monospacedDigit()).foregroundStyle(.tertiary)
                }
                .accessibilityLabel("Waiting")
            }
        case .speaking, .replied:
            ScrollView {
                Text(voice.reply ?? "")
                    .font(.title3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
                    .accessibilityIdentifier("voiceReply")
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(maxHeight: 340)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            if status == .speaking {
                VoiceBars(level: 0.45, active: true, tint: Theme.working).frame(height: 40).scaleEffect(0.4)
            }
        case .failed:
            if case .failed(let message) = voice.phase {
                Text(message).font(.body).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
        }
    }

    // MARK: Buttons

    private var buttons: some View {
        VStack(spacing: 10) {
            switch status {
            case .sending:
                AutoSendBar(recipient: store.agent(agentId)?.name ?? "Agent", deadline: voice.autoSendAt ?? Date(), undo: undo, sendNow: {
                    Task { await voice.send(voice.cleaned ?? voice.raw) }
                })
            case .thinking:
                EmptyView()
            case .speaking:
                big("Stop", "stop.fill", id: "voiceStop") { voice.stopReading() }
            case .replied:
                big("Replay", "play.fill", id: "voiceReplay") { Task { await voice.replay() } }
            case .failed:
                big("Retry", "arrow.clockwise", id: "voiceRetrySend") { Task { await voice.retrySend() } }
                    .disabled(!store.canWrite)
            }
            if status == .speaking || status == .replied {
                small("Reply by voice", "mic.fill", id: "voiceReplyByVoice") {
                    voice.stopReading()
                    Task { await voice.start(agentId: agentId) }
                }
            }
            HStack(spacing: 10) {
                small("Open chat", "bubble.left.and.bubble.right", id: "voiceOpenChat", action: openChat)
                // While it can still be undone, Undo is the way out.
                if status != .sending {
                    small("Done", "checkmark", id: "voiceStatusDone") {
                        voice.speaker.stop()
                        voice.discard()
                        dismiss()
                    }
                }
            }
        }
        .controlSize(.large)
    }

    private func big(_ title: String, _ symbol: String, id: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { Label(title, systemImage: symbol).frame(maxWidth: .infinity, minHeight: 30) }
            .buttonStyle(.accent)
            .accessibilityIdentifier(id)
    }

    private func small(_ title: String, _ symbol: String, id: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: symbol).frame(maxWidth: .infinity, minHeight: 30)
        }
        .buttonStyle(.bordered)
        .accessibilityIdentifier(id)
    }

    /// Undo: nothing goes; the text is back in the chat's message box.
    private func undo() {
        let text = voice.cleaned ?? voice.raw
        voice.cancelAutoSend()
        voice.discard()
        // Sending it again from the box brings the status back.
        VoicePresentation.shared.statusAgentId = agentId
        VoicePresentation.shared.returnedText = text
        VoicePresentation.shared.closeThenOpen(agentId, draft: text)
        dismiss()
    }

    /// The chat, where the same message, "thinking…" and the reply show. Mid-countdown it sends first.
    private func openChat() {
        let id = agentId
        if voice.autoSendAt != nil {
            let text = voice.cleaned ?? voice.raw
            voice.cancelAutoSend()
            Task { await store.send(text, to: id) }
        }
        voice.speaker.stop()
        voice.discard()
        VoicePresentation.shared.closeThenOpen(id)
        dismiss()
    }
}

// MARK: - E: dictate

/// E's To: row: whoever it goes to now, then every Lead, then the agents that worked most recently. One tap
/// switches; More lists every agent.
struct RecentAgentsRow: View {
    @Environment(LoftStore.self) private var store
    @Environment(VoiceSession.self) private var voice
    @State private var picking = false

    static func chips(_ agents: [Agent], current: String?, limit: Int = 5) -> [Agent] { TalkRecipients.chips(agents, current: current, limit: limit) }

    static func label(_ a: Agent, among shown: [Agent]) -> String { TalkRecipients.label(a, among: shown) }

    var body: some View {
        let current = voice.agentId
        let shown = Self.chips(store.agents, current: current)
        // More stays at the end of the row, outside the scrolling chips, so it's always on screen.
        HStack(spacing: 8) {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    Text("To").font(.subheadline).foregroundStyle(.secondary)
                    ForEach(shown) { a in
                        Button { voice.switchAgent(a.id) } label: {
                            Text(Self.label(a, among: shown)).font(.subheadline.weight(a.id == current ? .semibold : .regular))
                                .padding(.horizontal, 12).padding(.vertical, 6)
                                .background(a.id == current ? Theme.accent.opacity(0.25) : Color.secondary.opacity(0.12), in: Capsule())
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(a.id == current ? .isSelected : [])
                    }
                }
                .padding(.leading, Theme.gutter)
            }
            .scrollIndicators(.hidden)
            .accessibilityIdentifier("voiceRecentAgents")
            Button { picking = true } label: {
                Label("More", systemImage: "ellipsis").font(.subheadline)
                    .padding(.horizontal, 12).padding(.vertical, 6)
                    .background(Color.secondary.opacity(0.12), in: Capsule())
            }
            .buttonStyle(.plain)
            .fixedSize()
            .accessibilityLabel("More agents")
            .accessibilityHint("Lists every agent, to pick who this goes to.")
            .accessibilityIdentifier("voiceMoreAgents")
        }
        .padding(.trailing, Theme.gutter)
        .sheet(isPresented: $picking) {
            TalkAgentPicker().presentationDetents([.medium, .large])
        }
    }
}

/// Talk's More: every agent, by group, searchable by name, title, group or project. Picking one switches Talk to it.
struct TalkAgentPicker: View {
    @Environment(LoftStore.self) private var store
    @Environment(VoiceSession.self) private var voice
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""

    static func sections(_ agents: [Agent], query: String) -> [(title: String, agents: [Agent])] { TalkRecipients.sections(agents, query: query) }

    var body: some View {
        NavigationStack {
            let sections = Self.sections(store.agents, query: query)
            List {
                ForEach(sections, id: \.title) { section in
                    Section(section.title) {
                        ForEach(section.agents) { a in
                            let current = a.id == voice.agentId
                            Button {
                                voice.switchAgent(a.id)
                                dismiss()
                            } label: {
                                HStack {
                                    AgentRow(agent: a)
                                    if current { Image(systemName: "checkmark").foregroundStyle(Theme.accent).accessibilityHidden(true) }
                                }
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityAddTraits(current ? .isSelected : [])
                            .accessibilityHint(current ? "Talk goes to this agent now." : "Talk goes to this agent.")
                            .accessibilityIdentifier("talkPick-\(a.name)")
                        }
                    }
                }
            }
            .overlay { if sections.isEmpty { ContentUnavailableView.search(text: query) } }
            .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Name, title, group or project")
            .navigationTitle("Talk to…")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
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
            .foregroundStyle(Theme.onAccent)
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
