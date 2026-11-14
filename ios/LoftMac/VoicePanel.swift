import AppKit
import Carbon.HIToolbox
import LoftKit
import SwiftUI

/// A small always-on-top panel that takes keys without bringing Loft to the front. It stays when you click elsewhere,
/// and hands its keys to `onKey` first.
private final class VoicePanelWindow: NSPanel {
    var onKey: ((NSEvent) -> Bool)?

    init() {
        super.init(contentRect: NSRect(x: 0, y: 0, width: 460, height: 300), styleMask: [.borderless, .nonactivatingPanel],
                   backing: .buffered, defer: true)
        isFloatingPanel = true
        level = .floating
        hidesOnDeactivate = false
        isMovableByWindowBackground = true
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
    }

    override var canBecomeKey: Bool { true }

    override func sendEvent(_ event: NSEvent) {
        if event.type == .keyDown, onKey?(event) == true { return }
        super.sendEvent(event)
    }
}

/// The floating voice panel (spec #456): ⇧⌥Space or the menu bar opens it over any app and it starts listening. Done
/// has Loft tidy the text up, the on-device model may offer "Send to …?", and Send delivers it. Esc or × stops and
/// sends nothing. It drives the app's one `VoiceSession`, as the Talk sheet does.
@MainActor
@Observable
final class VoicePanelController {
    let voice: VoiceSession
    @ObservationIgnored private let store: LoftStore
    @ObservationIgnored private let nav: MacNav
    /// This opening has started listening (see `VoicePanel.step`).
    var listened = false
    /// "Send to …?" from the on-device model, once the text is tidied.
    var suggested: Agent?
    /// Off while drawing screenshots, so Sent stays on screen.
    @ObservationIgnored var closesWhenSent = true
    @ObservationIgnored private var panel: VoicePanelWindow?
    @ObservationIgnored private var binding: ShortcutBinding!

    init(store: LoftStore, nav: MacNav, voice: VoiceSession) {
        self.store = store
        self.nav = nav
        self.voice = voice
        binding = ShortcutBinding(.voicePanel) { [weak self] in self?.toggle() }
        #if DEBUG
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-render-voice-panel"), i + 2 < args.count {
            Task { await renderVoicePanel(self, store: store, fixture: args[i + 1], to: args[i + 2]) }
        }
        #endif
    }

    var step: VoicePanel.Step { VoicePanel.step(phase: voice.phase, needsPermission: voice.failureNeedsPermission, listened: listened) }
    var shortcutLabel: String? { binding.current?.label }

    func toggle() {
        if panel?.isVisible == true { close() } else { show() }
    }

    func show(listening: Bool = true) {
        let panel = panel ?? makePanel()
        if !panel.isVisible, let screen = NSScreen.screens.first(where: { $0.frame.contains(NSEvent.mouseLocation) }) ?? NSScreen.main {
            // Top centre, out of the way of what you're looking at.
            let f = screen.visibleFrame
            panel.setFrameTopLeftPoint(NSPoint(x: f.midX - panel.frame.width / 2, y: f.maxY - 40))
        }
        panel.makeKeyAndOrderFront(nil)
        if listening { listen() }
    }

    /// A fresh dictation to the open chat's agent, or the last one's.
    func listen() {
        // One microphone: whatever the session was doing (a Talk, an old reply being read) stops.
        if voice.phase != .idle { voice.discard() }
        voice.listensAgain = false
        listened = false
        suggested = nil
        let to = VoicePanel.recipient(openChat: nav.section == .agents ? nav.agentId : nil,
                                      last: UserDefaults.standard.string(forKey: "voiceLastAgent"),
                                      agents: store.agents, orchestratorId: store.orchestratorId)
        guard let to else { return }
        Task {
            await voice.start(agentId: to)
            if voice.phase == .listening { listened = true }
        }
    }

    /// Esc or ×: stops listening and sends nothing. After a send the session goes on reading the reply.
    func close() {
        panel?.orderOut(nil)
        if TalkHandOff.discardsOnClose(voice.phase) { voice.discard() }
        listened = false
    }

    func done() {
        guard step == .listening, !voice.raw.isEmpty else { return }
        Task {
            await voice.finish(.review)
            await suggest()
        }
    }

    func send() {
        guard step == .ready else { return }
        Task {
            await voice.send(voice.cleaned ?? voice.raw)
        }
    }

    func pick(_ agentId: String) {
        voice.switchAgent(agentId)
        if suggested?.id == agentId { suggested = nil }
    }

    /// The step moved on: Sent shows for a moment, then the panel gets out of the way.
    func stepChanged(_ step: VoicePanel.Step) {
        guard step == .sent, closesWhenSent else { return }
        Task {
            try? await Task.sleep(for: .seconds(1.5))
            if self.step == .sent { panel?.orderOut(nil) }
        }
    }

    private func suggest() async {
        guard step == .ready else { return }
        #if canImport(FoundationModels)
        if #available(macOS 26.0, *) {
            let guess = await OnDeviceFeatures.routeGuess(for: voice.cleaned ?? voice.raw, agents: store.agents, orchestratorId: store.orchestratorId)
            if step == .ready { suggested = VoicePanel.suggestion(guess: guess, current: voice.agentId) }
        }
        #endif
    }

    private func makePanel() -> VoicePanelWindow {
        let p = VoicePanelWindow()
        p.contentView = NSHostingView(rootView: VoicePanelView(controller: self).environment(store).tint(Theme.accent))
        p.onKey = { [weak self] in self?.handle($0) ?? false }
        panel = p
        return p
    }

    /// ↵ finishes, then sends; esc closes.
    private func handle(_ e: NSEvent) -> Bool {
        switch Int(e.keyCode) {
        case kVK_Return, kVK_ANSI_KeypadEnter:
            if step == .listening { done() } else if step == .ready { send() } else { return false }
        case kVK_Escape: close()
        default: return false
        }
        return true
    }
}

struct VoicePanelView: View {
    @Environment(LoftStore.self) private var store
    let controller: VoicePanelController
    private var voice: VoiceSession { controller.voice }

    var body: some View {
        let step = controller.step
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: Self.symbol(step)).foregroundStyle(Self.tint(step)).accessibilityHidden(true)
                Text(step == .sent ? "Sent to \(store.agent(voice.agentId)?.name ?? "the agent")" : Self.title(step)).font(.headline).accessibilityIdentifier("voicePanelState")
                Spacer()
                Button { controller.close() } label: { Image(systemName: "xmark").padding(4).contentShape(Rectangle()) }
                    .buttonStyle(.plain).foregroundStyle(WebTheme.fg3)
                    .help("Close (esc): nothing is sent")
                    .accessibilityLabel("Close")
                    .accessibilityIdentifier("voicePanelClose")
            }
            toRow(step)
            content(step).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            footer(step)
        }
        .padding(16)
        .background(WebTheme.canvas, in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(WebTheme.fg3.opacity(0.25)))
        .shadow(color: .black.opacity(0.35), radius: 18, y: 8)
        .padding(20)
        .frame(width: 460, height: 300)
        .onChange(of: step) { _, s in controller.stepChanged(s) }
    }

    // MARK: To: row and "Send to …?"

    @ViewBuilder
    private func toRow(_ step: VoicePanel.Step) -> some View {
        let current = voice.agentId
        HStack(spacing: 8) {
            Text("To").foregroundStyle(WebTheme.fg3)
            Picker("To", selection: Binding(get: { current }, set: { if let id = $0 { controller.pick(id) } })) {
                ForEach(store.sortedAgents) { Text($0.name).tag(Optional($0.id)) }
            }
            .labelsHidden()
            .fixedSize()
            .disabled(step == .sent || step == .tidying)
            .accessibilityIdentifier("voicePanelRecipient")
            if let s = controller.suggested, step == .ready {
                Button { controller.pick(s.id) } label: { Label("Send to \(s.name)?", systemImage: "arrow.turn.up.right") }
                    .buttonStyle(.plain).foregroundStyle(Theme.accent)
                    .accessibilityIdentifier("voicePanelSuggestion")
            }
        }
    }

    // MARK: The step's content

    @ViewBuilder
    private func content(_ step: VoicePanel.Step) -> some View {
        switch step {
        case .starting:
            Text("Getting ready…").foregroundStyle(WebTheme.fg3)
        case .listening:
            VStack(alignment: .leading, spacing: 10) {
                transcript
                Spacer(minLength: 0)
                TalkBars(level: voice.level, active: voice.phase == .listening).frame(maxWidth: .infinity)
            }
        case .tidying:
            VStack(alignment: .leading, spacing: 10) {
                Text(voice.raw).foregroundStyle(WebTheme.fg3).lineLimit(5).truncationMode(.head)
                ProgressView().controlSize(.small)
            }
        case .ready, .sent:
            ScrollView {
                Text(voice.cleaned ?? voice.raw).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
            }
            .accessibilityIdentifier("voicePanelText")
        case .error(let reason, _):
            Text(reason).foregroundStyle(WebTheme.fg2).accessibilityIdentifier("voicePanelError")
        }
    }

    private var transcript: some View {
        Group {
            if voice.committed.isEmpty && voice.live.isEmpty {
                Text("Start talking…").foregroundStyle(WebTheme.fg3)
            } else {
                Text(voice.committed) + Text(voice.committed.isEmpty || voice.live.isEmpty ? "" : " ") + Text(voice.live).foregroundStyle(Theme.accent)
            }
        }
        .font(.title3.weight(.medium))
        .lineLimit(3)
        .truncationMode(.head)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityIdentifier("voicePanelTranscript")
    }

    // MARK: Buttons

    @ViewBuilder
    private func footer(_ step: VoicePanel.Step) -> some View {
        HStack {
            switch step {
            case .listening:
                Text("Listening · \(Int(voice.elapsed))s").font(.caption.monospacedDigit()).foregroundStyle(WebTheme.fg3)
                Spacer()
                Button("Done") { controller.done() }.buttonStyle(.accent).disabled(voice.raw.isEmpty)
                    .accessibilityIdentifier("voicePanelDone")
            case .ready:
                Text("↵ Send   esc Discard").font(.caption.monospaced()).foregroundStyle(WebTheme.fg3)
                Spacer()
                Button { controller.send() } label: { Label("Send", systemImage: "arrow.up.circle.fill") }.buttonStyle(.accent)
                    .accessibilityIdentifier("voicePanelSend")
            case .error(let reason, let needsPermission):
                Spacer()
                if needsPermission {
                    Button("Open System Settings") { NSWorkspace.shared.open(TalkPermission.settingsURL(for: reason)) }
                        .accessibilityIdentifier("voicePanelOpenSettings")
                }
                if voice.failedSend != nil {
                    Button("Retry sending") { Task { await voice.retrySend() } }.buttonStyle(.accent)
                } else {
                    Button("Try again") { controller.listen() }.buttonStyle(.accent).accessibilityIdentifier("voicePanelRetry")
                }
            case .starting, .tidying, .sent:
                Spacer()
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

/// Settings › Voice panel: its shortcut, or none.
struct VoicePanelSettings: View {
    var body: some View {
        Section("Voice panel") {
            ShortcutRow(.voicePanel)
            Text("Opens a floating panel from any app that listens, has Loft tidy up what you said, and sends it to the agent you pick. Also in the menu bar.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

#if DEBUG
/// `-render-voice-panel <fixture> <file.png>`: shows the panel in one of `VoiceSession`'s fixture phases (listening,
/// cleaning, review, sending, failed), draws it to a PNG, then quits. For screenshots without a microphone.
@MainActor
func renderVoicePanel(_ c: VoicePanelController, store: LoftStore, fixture: String, to file: String) async {
    for _ in 0..<80 where store.agents.isEmpty { try? await Task.sleep(for: .milliseconds(100)) }
    c.closesWhenSent = false
    c.show(listening: false)
    c.voice.applyFixture(fixture, agentId: store.orchestratorId ?? store.agents.first?.id ?? "")
    c.listened = true
    if fixture == "review" { c.suggested = store.agents.first { $0.id != store.orchestratorId } }
    try? await Task.sleep(for: .seconds(1.5))
    if let view = NSApp.windows.first(where: { $0 is NSPanel && $0.isVisible })?.contentView,
       let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
        view.cacheDisplay(in: view.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: file))
    }
    NSApp.terminate(nil)
}
#endif
