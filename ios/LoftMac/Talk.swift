import AppKit
import LoftKit
import SwiftUI

/// Talk on the Mac (spec #382), from the chat's mic: the shared voice card in a sheet. Send hands the message to the
/// agent and goes back to its chat; Edit in chat puts the tidied text in that chat's message box to read and send.
struct TalkSheet: View {
    @Environment(LoftStore.self) private var store
    @Environment(VoiceSession.self) private var voice
    @Environment(\.dismiss) private var dismiss
    let agentId: String
    /// The chat to show now, and the tidied text for its message box when it wasn't sent.
    let done: (_ agentId: String, _ text: String?) -> Void
    @State private var flow: VoiceFlow?

    var body: some View {
        Group {
            if let flow {
                VoiceCard(flow: flow, cancel: { flow.cancel(); dismiss() }) { id, text in
                    done(id, text)
                    dismiss()
                }
                .onChange(of: flow.step) { _, step in
                    // The session goes on: it reads the reply.
                    guard step == .sent else { return }
                    done(voice.agentId ?? agentId, nil)
                    dismiss()
                }
            }
        }
        .padding(20)
        .frame(width: 520, height: 400)
        .background(WebTheme.canvas)
        .task {
            let f = VoiceFlow(voice: voice, store: store, listensAgain: true)
            flow = f
            await f.start(agentId: agentId)
        }
        .onDisappear { flow?.cancel() }
    }
}

/// The steps of one dictation in the Talk sheet or the voice panel, on the app's one `VoiceSession` (LoftKit's
/// pipeline: one audio owner, one reader): listen, have Loft tidy it up, maybe "Send to …?", then send.
@MainActor
@Observable
final class VoiceFlow {
    let voice: VoiceSession
    @ObservationIgnored private let store: LoftStore
    /// After a send the session listens again for the reply's answer (the Talk sheet), or stops (the panel).
    @ObservationIgnored private let listensAgain: Bool
    /// This dictation has started listening (see `VoicePanel.step`).
    var listened = false
    /// "Send to …?" from the on-device model, once the text is tidied.
    var suggested: Agent?

    init(voice: VoiceSession, store: LoftStore, listensAgain: Bool) {
        self.voice = voice
        self.store = store
        self.listensAgain = listensAgain
    }

    var step: VoicePanel.Step { VoicePanel.step(phase: voice.phase, needsPermission: voice.failureNeedsPermission, listened: listened) }
    /// Send and Edit in chat have something to work on.
    var canSend: Bool { step == .ready || (step == .listening && !voice.raw.isEmpty) }

    func start(agentId: String) async {
        // One microphone: whatever the session was doing (an earlier dictation, a reply being read) stops.
        if voice.phase != .idle { voice.discard() }
        voice.listensAgain = listensAgain
        listened = false
        suggested = nil
        await voice.start(agentId: agentId)
        if voice.phase == .listening { listened = true }
    }

    /// While listening, Loft tidies the text up and it goes out, unless the on-device model points at another agent:
    /// then it waits on Ready with "Send to …?". On Ready it goes out.
    func send() async {
        switch step {
        case .listening:
            guard !voice.raw.isEmpty else { return }
            await voice.finish(.review)
            guard step == .ready else { return }
            let guess = await routeGuess()
            guard step == .ready else { return }
            if VoicePanel.sendsAtOnce(guess: guess, current: voice.agentId) {
                await voice.send(voice.cleaned ?? voice.raw)
            } else {
                suggested = guess
            }
        case .ready:
            await voice.send(voice.cleaned ?? voice.raw)
        default:
            break
        }
    }

    /// The tidied text for the chat's message box. The dictation ends here.
    func editInChat() async -> String? {
        if step == .listening {
            guard !voice.raw.isEmpty else { return nil }
            await voice.finish(.review)
        }
        guard step == .ready else { return nil }
        let text = voice.cleaned ?? voice.raw
        voice.discard()
        return text
    }

    func pick(_ agentId: String) {
        voice.switchAgent(agentId)
        if suggested?.id == agentId { suggested = nil }
    }

    /// Cancel or esc: nothing is sent. After a send the session goes on reading the reply.
    func cancel() {
        if TalkHandOff.discardsOnClose(voice.phase) { voice.discard() }
        listened = false
    }

    func retry() async {
        if voice.failedSend != nil {
            await voice.retrySend()
        } else if let id = voice.agentId {
            await start(agentId: id)
        }
    }

    private func routeGuess() async -> Agent? {
        #if canImport(FoundationModels)
        if #available(macOS 26.0, *) {
            return await OnDeviceFeatures.routeGuess(for: voice.cleaned ?? voice.raw, agents: store.agents, orchestratorId: store.orchestratorId)
        }
        #endif
        return nil
    }
}

/// The voice UI the Talk sheet and the floating voice panel share: the step, To: with "Send to …?", the live
/// transcript over the level bars, and Cancel (esc), Edit in chat and Send (↵).
struct VoiceCard: View {
    @Environment(LoftStore.self) private var store
    let flow: VoiceFlow
    let cancel: () -> Void
    /// Edit in chat: the agent and the tidied text, for its chat's message box.
    let toChat: (_ agentId: String, _ text: String) -> Void
    @State private var picking: Bool
    private var voice: VoiceSession { flow.voice }

    /// `picking` opens More from the start, for screenshots.
    init(flow: VoiceFlow, cancel: @escaping () -> Void, toChat: @escaping (_ agentId: String, _ text: String) -> Void, picking: Bool = false) {
        self.flow = flow
        self.cancel = cancel
        self.toChat = toChat
        _picking = State(initialValue: picking)
    }

    var body: some View {
        let step = flow.step
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Image(systemName: Self.symbol(step)).foregroundStyle(Self.tint(step)).accessibilityHidden(true)
                Text(step == .sent ? "Sent to \(store.agent(voice.agentId)?.name ?? "the agent")" : Self.title(step))
                    .font(.headline)
                    .accessibilityIdentifier("talkState")
            }
            toRow(step)
            content(step).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            footer(step)
        }
        .foregroundStyle(WebTheme.fg)
        .onChange(of: voice.phase) { _, p in if p == .listening { flow.listened = true } }
    }

    // MARK: To: row and "Send to …?"

    private func toRow(_ step: VoicePanel.Step) -> some View {
        let current = voice.agentId
        let shown = TalkRecipients.chips(store.agents, current: current)
        return HStack(spacing: 8) {
            Text("To").foregroundStyle(WebTheme.fg3)
            // As many whole chips as fit, most recent first; the rest are under More.
            ViewThatFits(in: .horizontal) {
                ForEach(Array(stride(from: shown.count, through: 0, by: -1)), id: \.self) { k in
                    HStack {
                        if let s = flow.suggested, step == .ready {
                            Button { flow.pick(s.id) } label: { Label("Send to \(s.name)?", systemImage: "arrow.turn.up.right") }
                                .buttonStyle(.plain).foregroundStyle(Theme.accent)
                                .accessibilityIdentifier("talkSuggestion")
                        }
                        ForEach(shown.prefix(k)) { a in
                            Button { flow.pick(a.id) } label: {
                                Text(TalkRecipients.label(a, among: shown)).fontWeight(a.id == current ? .semibold : .regular)
                                    .padding(.horizontal, 10).padding(.vertical, 4)
                                    .background(a.id == current ? Theme.accent.opacity(0.25) : WebTheme.raised, in: Capsule())
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("talkTo-\(a.name)")
                        }
                    }
                    .fixedSize()
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button("More") { picking = true }.fixedSize().accessibilityIdentifier("talkMore")
                .popover(isPresented: $picking) { TalkMore(voice: voice, isPresented: $picking) { flow.pick($0) } }
        }
        .disabled(step == .sent || step == .tidying)
    }

    // MARK: The step's content

    @ViewBuilder
    private func content(_ step: VoicePanel.Step) -> some View {
        switch step {
        case .starting, .listening:
            VStack(spacing: 14) {
                transcript(step)
                Spacer(minLength: 0)
                TalkBars(level: voice.level, active: step == .listening)
                Text(step == .listening ? "Listening · \(Int(voice.elapsed))s" : " ").font(.caption.monospacedDigit()).foregroundStyle(WebTheme.fg3)
            }
        case .tidying:
            VStack(alignment: .leading, spacing: 10) {
                Text(voice.raw).font(.title3.weight(.medium)).foregroundStyle(WebTheme.fg3).lineLimit(5).truncationMode(.head)
                ProgressView().controlSize(.small)
            }
        case .ready, .sent:
            ScrollView {
                Text(voice.cleaned ?? voice.raw).font(.title3.weight(.medium))
                    .frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
            }
            .accessibilityIdentifier("talkText")
        case .error(let reason, let needsPermission):
            VStack(spacing: 12) {
                Spacer(minLength: 0)
                Text(reason).multilineTextAlignment(.center).foregroundStyle(WebTheme.fg2).accessibilityIdentifier("talkError")
                HStack {
                    if needsPermission {
                        Button("Open System Settings") { NSWorkspace.shared.open(TalkPermission.settingsURL(for: reason)) }
                            .accessibilityIdentifier("talkOpenSettings")
                    }
                    // A failed send keeps the message; one click sends it again.
                    Button(voice.failedSend != nil ? "Retry sending" : "Try again") { Task { await flow.retry() } }
                        .accessibilityIdentifier("talkRetry")
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity)
        }
    }

    private func transcript(_ step: VoicePanel.Step) -> some View {
        Group {
            if voice.committed.isEmpty && voice.live.isEmpty {
                Text(step == .starting ? "Getting ready…" : "Start talking…").foregroundStyle(WebTheme.fg3)
            } else {
                Text(voice.committed) + Text(voice.committed.isEmpty || voice.live.isEmpty ? "" : " ") + Text(voice.live).foregroundStyle(Theme.accent)
            }
        }
        .font(.title3.weight(.medium))
        .lineLimit(5)
        .truncationMode(.head)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier("talkTranscript")
    }

    // MARK: Buttons

    private func footer(_ step: VoicePanel.Step) -> some View {
        HStack {
            Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
                .help("Cancel (esc): nothing is sent")
                .accessibilityIdentifier("talkCancel")
            Spacer()
            switch step {
            case .starting, .listening, .tidying, .ready:
                Button("Edit in chat") {
                    let id = voice.agentId
                    Task { if let text = await flow.editInChat(), let id { toChat(id, text) } }
                }
                .disabled(!flow.canSend)
                .accessibilityIdentifier("talkEdit")
                Button { Task { await flow.send() } } label: { Label("Send", systemImage: "arrow.up.circle.fill") }
                    .buttonStyle(.accent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!flow.canSend)
                    .accessibilityIdentifier("talkSend")
            case .sent, .error:
                EmptyView()
            }
        }
    }

    private static func title(_ s: VoicePanel.Step) -> String {
        switch s {
        case .starting: "Getting ready"
        case .listening: "Listening"
        case .tidying: "Tidying up…"
        case .ready: "Ready to send"
        case .sent: "Sent"
        case .error(_, let permission): permission ? "Loft can't use the microphone" : "Voice stopped"
        }
    }

    private static func symbol(_ s: VoicePanel.Step) -> String {
        switch s {
        case .starting, .listening: "mic.fill"
        case .tidying: "wand.and.stars"
        case .ready: "text.bubble"
        case .sent: "checkmark.circle.fill"
        case .error: "mic.slash"
        }
    }

    private static func tint(_ s: VoicePanel.Step) -> Color {
        switch s {
        case .error: WebTheme.bad
        case .sent: WebTheme.good
        default: Theme.accent
        }
    }
}

/// Where System Settings turns access back on, for what the failure says is off.
enum TalkPermission {
    static func settingsURL(for message: String) -> URL {
        URL(string: "x-apple.systempreferences:com.apple.preference.security?\(TalkHandOff.privacyPane(for: message))")!
    }
}

/// Bars that rise with the voice.
struct TalkBars: View {
    var level: Float
    var active: Bool

    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<21, id: \.self) { i in
                let centre = 1 - abs(CGFloat(i) - 10) / 12
                Capsule()
                    .fill(Theme.accent.opacity(0.55 + 0.45 * centre))
                    .frame(width: 5, height: 6 + (active ? 44 * CGFloat(min(max(level, 0), 1)) * centre : 0))
            }
        }
        .frame(height: 56)
        .animation(.easeOut(duration: 0.1), value: level)
        .accessibilityHidden(true)
    }
}

/// Talk's More: every agent by group, searchable. It takes the session from its card, not the environment: the
/// floating voice panel doesn't put one there (#491).
private struct TalkMore: View {
    @Environment(LoftStore.self) private var store
    let voice: VoiceSession
    @Binding var isPresented: Bool
    let pick: (String) -> Void
    @State private var query = ""

    var body: some View {
        VStack(spacing: 8) {
            TextField("Name, title, group or project", text: $query).textFieldStyle(.roundedBorder)
                .accessibilityIdentifier("talkMoreSearch")
            List {
                ForEach(TalkRecipients.sections(store.agents, query: query), id: \.title) { section in
                    Section(section.title) {
                        ForEach(section.agents) { a in
                            Button {
                                pick(a.id)
                                isPresented = false
                            } label: {
                                HStack { Text(a.name); Spacer(); if a.id == voice.agentId { Image(systemName: "checkmark") } }.contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("talkMoreRow-\(a.name)")
                        }
                    }
                }
            }
            .frame(height: 260)
        }
        .padding(12)
        .frame(width: 300)
    }
}

#if DEBUG
/// `-render-talk-sheet <fixture> <file.png>`: the Talk sheet's card in one of `VoiceSession`'s fixture phases, as
/// `-render-voice-panel` draws the panel, to a PNG, then quits. For side-by-side screenshots without a microphone.
@MainActor
func renderTalkSheet(store: LoftStore, voice: VoiceSession, fixture: String, to file: String) async {
    for _ in 0..<80 where store.agents.isEmpty { try? await Task.sleep(for: .milliseconds(100)) }
    let flow = VoiceFlow(voice: voice, store: store, listensAgain: true)
    voice.applyFixture(fixture, agentId: store.orchestratorId ?? store.agents.first?.id ?? "")
    flow.listened = true
    if fixture == "review" { flow.suggested = store.agents.first { $0.id != store.orchestratorId } }
    let view = NSHostingView(rootView: VoiceCard(flow: flow, cancel: {}, toChat: { _, _ in })
        .padding(20).frame(width: 520, height: 400).background(WebTheme.canvas)
        .environment(store).tint(Theme.accent))
    let window = NSWindow(contentRect: NSRect(x: 200, y: 200, width: 520, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView = view
    window.makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
    try? await Task.sleep(for: .seconds(1.5))
    if let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
        view.cacheDisplay(in: view.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: file))
    }
    NSApp.terminate(nil)
}
#endif
