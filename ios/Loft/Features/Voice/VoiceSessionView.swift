import SwiftUI

/// Talk: dictate to one agent, then follow it. You talk (or hold the mic) and let go; the tidied text lands in that
/// agent's chat message box to read and send. Sending it there brings this screen back as a full-screen status:
/// Sent (with Undo) → Thinking → the reply read aloud. It only reads `VoiceSession` and calls its actions.
struct VoiceSessionView: View {
    @Environment(LoftStore.self) private var store
    @Environment(VoiceSession.self) private var voice
    @Environment(\.dynamicTypeSize) private var typeSize
    let dismiss: () -> Void

    /// After Send: the full-screen status, when there is one. A mic failure keeps the failure screen.
    private var status: VoiceStatus? {
        let s = VoiceStatus.of(voice.phase, undoPending: voice.autoSendAt != nil, posting: !store.pending(for: agentId).isEmpty)
        return s == .failed && voice.failedSend == nil ? nil : s
    }

    private var agentId: String { voice.agentId ?? "" }
    private var agentName: String { store.agent(agentId)?.name ?? "Agent" }

    var body: some View {
        VStack(spacing: 0) {
            // A full screen covers the root's banner, so it shows its own.
            if store.isDemo { DemoBanner() }
            header
            questionCard
            Group {
                if let status {
                    VoiceStatusView(status: status, dismiss: dismiss)
                } else if case .failed(let message) = voice.phase {
                    failed(message)
                } else if voice.phase == .review || voice.phase == .sending {
                    // On its way to the chat's message box (or an interview answer going out).
                    ProgressView()
                } else {
                    dictate
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(Theme.background.ignoresSafeArea())
        .onChange(of: voice.phase, initial: true) { _, p in
            // The tidied text goes to the chat's message box, to read there; sending it brings the status back.
            if p == .review, voice.autoSendAt == nil, let id = voice.agentId {
                let text = voice.cleaned ?? voice.raw
                voice.discard()
                VoicePresentation.shared.statusAgentId = id
                // Also for a chat that's already open, which keeps its own box.
                VoicePresentation.shared.returnedText = text
                VoicePresentation.shared.closeThenOpen(id, draft: text)
                dismiss()
            }
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
                Text(agentName).font(.headline).lineLimit(1).minimumScaleFactor(0.6)
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

    private var subtitle: String {
        switch voice.phase {
        case .idle, .preparing: "Getting ready…"
        case .listening: "Listening · \(Self.clock(voice.elapsed))"
        case .paused: "Paused · \(Self.clock(voice.elapsed))"
        case .cleaning: "Tidying up…"
        case .review: voice.autoSendAt != nil ? "Sending…" : "Review before sending"
        case .sending: "Sent"
        case .speaking: "Replying"
        case .replied: "Replied"
        case .failed: "Something went wrong"
        }
    }

    static func clock(_ t: TimeInterval) -> String {
        let s = Int(t.rounded(.down))
        return String(format: "%d:%02d", s / 60, s % 60)
    }

    // MARK: Dictate

    private var dictate: some View {
        VStack(spacing: 20) {
            RecentAgentsRow()
            Spacer(minLength: 0)
            if voice.phase == .cleaning {
                ProgressView("Tidying up…")
                Text(voice.raw).font(.body).foregroundStyle(.secondary).lineLimit(6).padding(.horizontal, Theme.gutter)
            } else {
                CaptionTranscript(committed: voice.committed, live: voice.live)
            }
            Spacer(minLength: 0)
            VoiceBars(level: voice.level, active: voice.phase == .listening).frame(height: 60).scaleEffect(0.6)
            HoldToTalkButton(listening: voice.phase == .listening) { Task { await voice.finish() } }
                .disabled(voice.raw.isEmpty || voice.phase == .cleaning)
            Text("Tap or let go when you're done: it lands in the message box to send.")
                .font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
                .padding(.horizontal, Theme.gutter).padding(.bottom, 12)
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
                .buttonStyle(.accent)
                .disabled(!store.canWrite)
                .accessibilityIdentifier("voiceRetrySend")
            } else {
                Button { Task { await voice.start(agentId: agentId) } } label: {
                    Text("Try again").frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.accent)
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
    /// Talk: the chat whose message box holds a dictation; sending it there opens the status screen.
    var statusAgentId: String?
    /// Talk: text Undo sends back to that chat's message box.
    var returnedText: String?

    /// The chat to open once the Talk screen is gone. Opening it while the screen closes switched tabs under the
    /// closing cover, and SwiftUI then dropped the close: the screen stayed up until you tapped Close.
    struct PendingChat: Equatable { var agentId: String; var draft: String? }
    var pendingChat: PendingChat?

    func closeThenOpen(_ agentId: String, draft: String? = nil) {
        pendingChat = PendingChat(agentId: agentId, draft: draft)
    }
}

struct VoicePresenter: ViewModifier {
    @Environment(VoiceSession.self) private var voice
    @Environment(Router.self) private var router
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
            .fullScreenCover(isPresented: $presentation.shown, onDismiss: {
                voice.close()
                // Now that the screen is gone, the chat it handed over to.
                if let p = presentation.pendingChat {
                    presentation.pendingChat = nil
                    router.openChat(p.agentId, draft: p.draft)
                }
            }) {
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
                        .buttonStyle(.accent)
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
