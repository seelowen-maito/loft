import SwiftUI

/// Full-screen, hands-free conversation with one agent: listen → review → send → hear the reply.
/// It only reads `VoiceSession` and calls its actions; the engine owns the mic, the end phrases and
/// the spoken confirmation. Nothing is sent without an explicit Send (a tap, or "send" in hands-free mode).
struct VoiceSessionView: View {
    @Environment(LoftStore.self) private var store
    @Environment(VoiceSession.self) private var voice
    @Environment(Router.self) private var router
    @Environment(\.dynamicTypeSize) private var typeSize
    @AppStorage(VoiceVariant.key) private var variantRaw = VoiceVariant.current.rawValue
    let dismiss: () -> Void

    private var variant: VoiceVariant { VoiceVariant(rawValue: variantRaw.lowercased()) ?? .current }

    @State private var showOriginal = false
    @State private var draft = ""
    @State private var confirmDiscard = false
    @FocusState private var editing: Bool

    private var agentId: String { voice.agentId ?? "" }
    private var agentName: String { store.agent(agentId)?.name ?? "Agent" }

    var body: some View {
        VStack(spacing: 0) {
            header
            questionCard
            Group {
                switch voice.phase {
                case .idle, .preparing, .listening, .paused: listening
                case .cleaning: cleaning
                case .review: review
                case .sending: working
                case .speaking: speaking
                case .failed(let message): failed(message)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(Theme.background.ignoresSafeArea())
        .onChange(of: voice.phase, initial: true) { _, p in
            if p == .review { loadDraft() }
            // Voice style B: once it's sent, the chat takes over (your message, then "thinking…" and the reply).
            if p == .sending, variant == .b, let id = voice.agentId {
                router.openChat(id)
                dismiss()
            }
        }
        .confirmationDialog("Discard what you said?", isPresented: $confirmDiscard, titleVisibility: .visible) {
            Button("Discard", role: .destructive) { voice.discard(); dismiss() }
        } message: {
            Text("The transcript is deleted and nothing is sent.")
        }
        .sensoryFeedback(.selection, trigger: voice.phase)
        .task(id: agentId) {
            // Opened from the Overview the chat may not have loaded yet: fetch the interview and its question.
            guard !agentId.isEmpty else { return }
            await store.refreshGrill(agentId)
            if store.conversation(agentId).isEmpty { try? await store.loadConversation(agentId) }
        }
        .onChange(of: store.openGrill(for: agentId) == nil) { was, closed in
            // However the interview ended, the mic goes off and stays off.
            if closed, !was { voice.interviewEnded() }
        }
    }

    // MARK: Interview question

    /// The question waiting for an answer, as the same card the chat shows. What you say next answers it.
    @ViewBuilder private var questionCard: some View {
        if let q = store.currentGrillQuestion(for: agentId), let s = store.openGrill(for: agentId), let agent = store.agent(agentId) {
            let card = GrillQuestionCard(question: q, number: s.number(of: q.id) ?? 1, total: s.questions?.count ?? 1,
                                         sessionId: s.sessionId, language: s.language, agent: agent, isCurrent: true,
                                         command: { c in
                                             if c == .end { voice.interviewEnded() }
                                             Task { try? await store.sendGrillCommand(c, agentId: agentId) }
                                         },
                                         choose: { choice in Task { await voice.answerWithChoice(choice) } }, roundActions: true)
                .padding(.horizontal, Theme.gutter)
                .padding(.bottom, 8)
            Group {
                if typeSize.isAccessibilitySize {
                    // With its choices and five actions the card is taller than the screen at these sizes:
                    // it scrolls in its own area so the header and the conversation stay in view.
                    ScrollView { card }
                        .scrollIndicators(.visible)
                        .frame(maxHeight: 220)
                } else {
                    // Long questions collapse inside the card, so it stays a card's height.
                    card
                }
            }
            .accessibilityIdentifier("voiceQuestionCard")
        }
    }

    // MARK: Header

    private var header: some View {
        HStack {
            Button(action: dismiss) {
                Image(systemName: "xmark").font(.body.weight(.semibold)).frame(width: 44, height: 44)
            }
            .accessibilityLabel("Close")
            .accessibilityHint("Your dictation is kept until you send or discard it.")
            .accessibilityIdentifier("voiceClose")
            Spacer()
            VStack(spacing: 2) {
                if variant == .c { recipientChip } else {
                    Text(agentName).font(.headline).lineLimit(1).minimumScaleFactor(0.6)
                }
                Text(subtitle).font(.caption.monospacedDigit()).foregroundStyle(.secondary).lineLimit(1).minimumScaleFactor(0.6)
            }
            .accessibilityElement(children: .combine)
            Spacer()
            Group {
                if voice.handsFree {
                    Image(systemName: "ear")
                        .font(.footnote.weight(.semibold))
                        .padding(6)
                        .background(Theme.accent.opacity(0.18), in: Circle())
                        .foregroundStyle(Theme.accent)
                        .accessibilityLabel("Hands-free on")
                }
            }
            .frame(width: 44, height: 44)
        }
        .padding(.horizontal, 8)
    }

    /// Voice style C: who this goes to, changeable before it's sent.
    private var recipientChip: some View {
        Menu {
            ForEach(store.agents) { a in
                Button { voice.switchAgent(a.id) } label: {
                    if a.id == agentId { Label(a.name, systemImage: "checkmark") } else { Text(a.name) }
                }
            }
        } label: {
            HStack(spacing: 4) {
                Text("To").foregroundStyle(.secondary)
                Text(agentName).fontWeight(.semibold)
                Image(systemName: "chevron.down").font(.caption2.weight(.bold))
            }
            .font(.subheadline)
            .padding(.horizontal, 12).padding(.vertical, 6)
            .background(Theme.accent.opacity(0.15), in: Capsule())
        }
        .disabled(voice.phase == .sending || voice.phase == .speaking)
        .accessibilityLabel("To \(agentName)")
        .accessibilityHint("Choose who this message goes to.")
        .accessibilityIdentifier("voiceRecipient")
    }

    private var subtitle: String {
        switch voice.phase {
        case .idle, .preparing: "Getting ready…"
        case .listening: "Listening · \(Self.clock(voice.elapsed))"
        case .paused: "Paused · \(Self.clock(voice.elapsed))"
        case .cleaning: "Tidying up…"
        case .review: voice.autoSendAt != nil ? "Sending…" : "Review before sending"
        case .sending: "Sent"
        case .speaking: "Replying"
        case .failed: "Something went wrong"
        }
    }

    static func clock(_ t: TimeInterval) -> String {
        let s = Int(t.rounded(.down))
        return String(format: "%d:%02d", s / 60, s % 60)
    }

    // MARK: Listening

    private var listening: some View {
        let preparing = voice.phase == .preparing || voice.phase == .idle
        // At accessibility sizes everything competes for the height: no orb, the transcript gives way, icon-only buttons.
        let ax = typeSize.isAccessibilitySize
        return VStack(spacing: ax ? 8 : 16) {
            if !ax {
                Spacer(minLength: 0)
                Group {
                    let tint = voice.phase == .listening ? Theme.accent : Color.secondary
                    switch variant {
                    case .current: VoiceOrb(level: voice.level, active: voice.phase == .listening, tint: tint).frame(maxWidth: 220)
                    case .a: VoiceBars(level: voice.level, active: voice.phase == .listening, tint: tint)
                    case .b: VoicePulse(level: voice.level, active: voice.phase == .listening, tint: tint)
                    case .c: VoiceLine(level: voice.level, active: voice.phase == .listening, tint: tint).padding(.horizontal, Theme.gutter)
                    }
                }
                .opacity(preparing ? 0.45 : 1)
            }
            Spacer(minLength: 0)
            if preparing {
                ProgressView("Getting ready…").accessibilityIdentifier("voicePreparing")
                Spacer(minLength: 0)
            } else {
                switch variant {
                case .current:
                    transcript
                        .frame(maxHeight: 280)
                        .fixedSize(horizontal: false, vertical: !ax && voice.raw.count < 200)
                case .a: CaptionTranscript(committed: voice.committed, live: voice.live)
                case .b: BubbleTranscript(text: voice.raw, agentName: agentName)
                case .c: LiveCaptions(committed: voice.committed, live: voice.live)
                }
                if voice.phase == .paused {
                    Label("Paused. Tap resume to keep talking.", systemImage: "pause.circle")
                        .font(.footnote).foregroundStyle(.secondary)
                } else {
                    Text(hint)
                        .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
                        .lineLimit(ax ? 2 : nil)
                        .accessibilityIdentifier("voiceFinishHint")
                        .accessibilityIdentifier("voiceHint")
                }
            }
            HStack(spacing: 16) {
                Button {
                    voice.phase == .paused ? voice.resume() : voice.pause()
                } label: {
                    Label(voice.phase == .paused ? "Resume" : "Pause",
                          systemImage: voice.phase == .paused ? "play.fill" : "pause.fill")
                        .frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.bordered)
                .disabled(preparing)
                .accessibilityIdentifier("voicePause")
                Button {
                    Task { await voice.finish() }
                } label: {
                    Label("Done", systemImage: "checkmark").frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.borderedProminent)
                .disabled(preparing || voice.raw.isEmpty)
                .accessibilityIdentifier("voiceDone")
            }
            .labelStyle(ax ? AnyLabelStyle(.iconOnly) : AnyLabelStyle(.titleAndIcon))
            .controlSize(.large)
            .padding(.horizontal, Theme.gutter)
            .padding(.bottom, 12)
        }
    }

    private var hint: String {
        switch variant {
        case .current, .c: "Take your time. Say “send” when you're done; after a quiet moment I'll ask."
        case .a: "Say “send” or tap Done. It's tidied up and goes after 3 seconds, with Undo."
        case .b: "Say “send” or tap Done. It goes to the chat as you said it."
        }
    }

    private var transcript: some View {
        ScrollViewReader { proxy in
            ScrollView {
                Group {
                    if voice.raw.isEmpty {
                        Text("Start talking…").foregroundStyle(.tertiary)
                    } else {
                        Text(voice.committed) + Text(voice.committed.isEmpty || voice.live.isEmpty ? "" : " ")
                            + Text(voice.live).foregroundStyle(.secondary)
                    }
                }
                .font(.title3)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, Theme.gutter)
                .id("end")
                .accessibilityIdentifier("voiceTranscript")
            }
            .defaultScrollAnchor(.bottom)
            .onChange(of: voice.raw) { _, _ in
                withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo("end", anchor: .bottom) }
            }
        }
        .mask(LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .black, location: 0.12),
                                     .init(color: .black, location: 1)], startPoint: .top, endPoint: .bottom))
    }

    // MARK: Cleaning

    private var cleaning: some View {
        VStack(spacing: 20) {
            VoiceOrb(level: 0.2, active: true).frame(maxWidth: 140)
            ProgressView("Tidying up what you said…")
            Text(voice.raw).font(.body).foregroundStyle(.secondary).lineLimit(6)
                .padding(.horizontal, Theme.gutter)
        }
    }

    // MARK: Review

    private func loadDraft() {
        draft = showOriginal ? voice.raw : (voice.cleaned ?? voice.raw)
    }

    @ViewBuilder private var review: some View {
        // At accessibility sizes the bar, the draft and three big buttons don't fit: let it all scroll.
        if typeSize.isAccessibilitySize {
            ScrollView { reviewContent }.scrollBounceBehavior(.basedOnSize)
        } else {
            reviewContent
        }
    }

    private var reviewContent: some View {
        VStack(spacing: 12) {
            if voice.autoSendAt != nil {
                AutoSendBar(recipient: agentName, undo: { voice.cancelAutoSend() }, sendNow: {
                    editing = false
                    let text = draft
                    Task { await voice.send(text) }
                })
            }
            if voice.cleaned != nil {
                Picker("Version", selection: $showOriginal) {
                    Text("Cleaned").tag(false)
                    Text("Original").tag(true)
                }
                .pickerStyle(.segmented)
                .onChange(of: showOriginal) { _, _ in loadDraft() }
                .accessibilityIdentifier("voiceVersion")
            }
            if let note = voice.cleanupNote {
                Label(note, systemImage: "info.circle")
                    .font(.footnote).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("voiceCleanupNote")
            }
            TextEditor(text: $draft)
                .font(.title3)
                .focused($editing)
                .scrollContentBackground(.hidden)
                .padding(10)
                .background(Theme.card, in: RoundedRectangle(cornerRadius: Theme.radius, style: .continuous))
                .frame(minHeight: typeSize.isAccessibilitySize ? 160 : 100)
                .accessibilityLabel("Message to \(agentName)")
                .accessibilityIdentifier("voiceDraft")
            if voice.awaitingVoiceConfirm {
                Label("\(VoiceSend.question(for: draft)) Say yes or no, or keep talking.", systemImage: "waveform")
                    .font(.footnote.weight(.medium)).foregroundStyle(Theme.accent)
                    .accessibilityIdentifier("voiceConfirmHint")
            }
            VStack(spacing: 10) {
                Button {
                    editing = false
                    let text = draft
                    Task { await voice.send(text) }
                } label: {
                    Label(typeSize.isAccessibilitySize ? "Send" : "Send to \(agentName)", systemImage: "arrow.up.circle.fill")
                        .frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.borderedProminent)
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !store.canWrite)
                .accessibilityIdentifier("voiceSend")
                AnyLayout(typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(spacing: 10)) : AnyLayout(HStackLayout(spacing: 10))) {
                    Button { voice.resume() } label: {
                        Label("Keep talking", systemImage: "mic").fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, minHeight: 30)
                    }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("voiceKeepTalking")
                    Button(role: .destructive) { confirmDiscard = true } label: {
                        Label("Discard", systemImage: "trash").frame(maxWidth: .infinity, minHeight: 30)
                    }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("voiceDiscard")
                }
            }
            .controlSize(.large)
        }
        .padding(.horizontal, Theme.gutter)
        .padding(.bottom, 12)
    }

    // MARK: Agent replying

    @ViewBuilder private var working: some View {
        if variant == .a || variant == .c { VoiceProgress(agentId: agentId) } else { workingOrb }
    }

    private var workingOrb: some View {
        VStack(spacing: 20) {
            VoiceOrb(level: 0.35, active: true, tint: Theme.working).frame(maxWidth: 160)
            Text("\(agentName) is working…").font(.title3.weight(.semibold))
            Text(voice.readReplies ? "The reply will be read aloud when it arrives." : "The reply will appear here when it arrives.")
                .font(.footnote).foregroundStyle(.secondary)
        }
        .padding(.horizontal, Theme.gutter)
        .accessibilityElement(children: .combine)
    }

    private var speaking: some View {
        VStack(spacing: 16) {
            VoiceOrb(level: 0.5, active: true, tint: Theme.working).frame(maxWidth: typeSize.isAccessibilitySize ? 56 : 120)
            ScrollView {
                Text(voice.reply ?? "")
                    .font(.title3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, Theme.gutter)
                    .accessibilityIdentifier("voiceReply")
            }
            .scrollIndicators(.visible)
            .scrollBounceBehavior(.basedOnSize)
            if voice.handsFree && !typeSize.isAccessibilitySize {
                Label("Listening again after the reply", systemImage: "arrow.trianglehead.2.clockwise")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            (typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(spacing: 10)) : AnyLayout(HStackLayout(spacing: 12))) {
                if voice.handsFree {
                    // Your turn now, without waiting for the end of the reply.
                    Button { voice.speaker.interrupt(.skip) } label: {
                        Label("Skip", systemImage: "forward.fill").frame(maxWidth: .infinity, minHeight: 30)
                    }
                    .buttonStyle(.bordered)
                    .accessibilityHint("Stops reading and listens for you")
                    .accessibilityIdentifier("voiceSkipSpeaking")
                }
                Button { voice.speaker.interrupt(.stop) } label: {
                    Label("Stop", systemImage: "stop.fill").frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.borderedProminent)
                .accessibilityHint("Stops reading and ends the conversation")
                .accessibilityIdentifier("voiceStopSpeaking")
            }
            .controlSize(.large)
            .padding(.horizontal, Theme.gutter)
            .padding(.bottom, 12)
        }
    }

    // MARK: Failure

    private func failed(_ message: String) -> some View {
        VStack(spacing: 12) {
            Spacer(minLength: 0)
            EmptyState(symbol: "mic.slash", title: "Voice stopped", message: message)
            Spacer(minLength: 0)
            if voice.failedSend != nil {
                // The message is kept; one tap sends it again.
                Button { Task { await voice.retrySend() } } label: {
                    Label("Retry sending", systemImage: "arrow.clockwise").frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.borderedProminent)
                .disabled(!store.canWrite)
                .accessibilityIdentifier("voiceRetrySend")
            } else {
                Button { Task { await voice.start(agentId: agentId) } } label: {
                    Text("Try again").frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("voiceRetry")
            }
            if voice.failureNeedsPermission, let url = URL(string: UIApplication.openSettingsURLString) {
                Link(destination: url) { Text("Open Settings").frame(maxWidth: .infinity, minHeight: 30) }
                    .buttonStyle(.bordered)
            }
            if !voice.raw.isEmpty {
                Button { Task { await voice.finish() } } label: {
                    Text("Review what was heard").frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.bordered)
            }
        }
        .controlSize(.large)
        .padding(.horizontal, Theme.gutter)
        .padding(.bottom, 12)
    }
}

/// Presents the voice screen whenever a session becomes active (from any entry point, a restored
/// draft or a fixture). Closing it pauses the session and keeps the draft.
@MainActor @Observable final class VoicePresentation {
    static let shared = VoicePresentation()
    var shown = false
}

struct VoicePresenter: ViewModifier {
    @Environment(VoiceSession.self) private var voice
    @Bindable private var presentation = VoicePresentation.shared
    private var shown: Bool {
        get { presentation.shown }
        nonmutating set { presentation.shown = newValue }
    }

    func body(content: Content) -> some View {
        content
            .onChange(of: voice.phase, initial: true) { old, new in
                if new != .idle && (old == .idle || !shown) { shown = true }
            }
            .fullScreenCover(isPresented: $presentation.shown, onDismiss: { voice.close() }) {
                VoiceSessionView { shown = false }
            }
    }
}

extension View {
    func voicePresenter() -> some View { modifier(VoicePresenter()) }
}

/// Offers to pick up a dictation that wasn't sent: one closed during review, or one saved before a crash.
struct VoiceDraftBanner: View {
    @Environment(LoftStore.self) private var store
    @Environment(VoiceSession.self) private var voice
    private var presentation: VoicePresentation { .shared }
    @State private var saved: VoiceSession.Draft?

    private var visible: Bool {
        !presentation.shown && (voice.phase == .review || (voice.phase == .idle && saved != nil))
    }
    private var agentName: String {
        store.agent(voice.agentId ?? saved?.agentId ?? "")?.name ?? "an agent"
    }

    var body: some View {
        Group {
            if visible {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(alignment: .top, spacing: 12) {
                        Image(systemName: "waveform").font(.title3).foregroundStyle(Theme.accent)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Unsent voice message for \(agentName)").font(.subheadline.weight(.semibold))
                            if let at = saved?.savedAt {
                                RelativeTime(date: at).font(.footnote).foregroundStyle(.secondary)
                            }
                        }
                    }
                    .accessibilityElement(children: .combine)
                    HStack {
                        Button {
                            if voice.phase == .review { presentation.shown = true } else { Task { await voice.restoreDraft() } }
                        } label: {
                            Text("Review").frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("voiceDraftReview")
                        Button(role: .destructive) {
                            voice.discard()
                            saved = nil
                        } label: {
                            Text("Discard").frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                        .accessibilityIdentifier("voiceDraftDiscard")
                    }
                    .controlSize(.small)
                }
                .padding(14)
                .background(Theme.card, in: RoundedRectangle(cornerRadius: Theme.radius, style: .continuous))
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("voiceDraftBanner")
            }
        }
        .onAppear { saved = VoiceSession.savedDraft() }
        .onChange(of: voice.phase) { _, _ in saved = VoiceSession.savedDraft() }
        .onChange(of: presentation.shown) { _, _ in saved = VoiceSession.savedDraft() }
    }
}

/// Picks a label style at run time (icon-only at accessibility sizes, where words don't fit side by side).
struct AnyLabelStyle: LabelStyle {
    private let make: (Configuration) -> AnyView
    init(_ style: some LabelStyle) { make = { AnyView(style.makeBody(configuration: $0)) } }
    func makeBody(configuration: Configuration) -> some View { make(configuration) }
}
