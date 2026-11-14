import AppKit
import Carbon.HIToolbox
import LoftKit
import SwiftUI

/// A small always-on-top panel that takes keys without bringing Loft to the front. It stays when you click elsewhere,
/// and hands its keys to `onKey` first.
private final class VoicePanelWindow: NSPanel {
    var onKey: ((NSEvent) -> Bool)?

    init() {
        super.init(contentRect: NSRect(origin: .zero, size: VoicePanelView.size), styleMask: [.borderless, .nonactivatingPanel],
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

/// The floating voice panel (spec #456): ⇧⌥Space or the menu bar opens it over any app and it starts listening. It
/// shows the same `VoiceCard` as the chat's Talk sheet: Send (↵) has Loft tidy the text up and sends it, Edit in chat
/// opens it in the agent's chat, and Cancel (esc) stops and sends nothing.
@MainActor
final class VoicePanelController {
    let flow: VoiceFlow
    private let store: LoftStore
    private let nav: MacNav
    /// Opens or raises the main window, for Edit in chat; set by the menu bar item, which is there for the app's whole life.
    var showWindow: (() -> Void)?
    /// Off while drawing screenshots, so Sent stays on screen.
    var closesWhenSent = true
    /// On while drawing screenshots of More.
    var opensMore = false
    private var panel: VoicePanelWindow?
    private var binding: ShortcutBinding!

    init(store: LoftStore, nav: MacNav, voice: VoiceSession) {
        self.store = store
        self.nav = nav
        flow = VoiceFlow(voice: voice, store: store, listensAgain: false)
        binding = ShortcutBinding(.voicePanel) { [weak self] in self?.toggle() }
        #if DEBUG
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-render-voice-panel"), i + 2 < args.count {
            Task { await renderVoicePanel(self, store: store, fixture: args[i + 1], to: args[i + 2]) }
        }
        if let i = args.firstIndex(of: "-show-voice-panel"), i + 1 < args.count {
            Task { await showVoicePanel(self, store: store, fixture: args[i + 1]) }
        }
        if let i = args.firstIndex(of: "-render-talk-sheet"), i + 2 < args.count {
            Task { await renderTalkSheet(store: store, voice: voice, fixture: args[i + 1], to: args[i + 2]) }
        }
        #endif
    }

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
        let to = VoicePanel.recipient(openChat: nav.section == .agents ? nav.agentId : nil,
                                      last: UserDefaults.standard.string(forKey: "voiceLastAgent"),
                                      agents: store.agents, orchestratorId: store.orchestratorId)
        guard let to else { return }
        Task { await flow.start(agentId: to) }
    }

    /// Cancel or esc: stops listening and sends nothing. After a send the session goes on reading the reply.
    func close() {
        panel?.orderOut(nil)
        flow.cancel()
    }

    /// Edit in chat: the main window opens on the agent's chat with the text in its message box.
    func toChat(_ agentId: String, _ text: String) {
        panel?.orderOut(nil)
        nav.openChat(agentId)
        nav.voiceDraft = (agentId, text)
        showWindow?()
        NSApp.activate(ignoringOtherApps: true)
    }

    /// The step moved on: Sent shows for a moment, then the panel gets out of the way.
    func stepChanged(_ step: VoicePanel.Step) {
        guard step == .sent, closesWhenSent else { return }
        Task {
            try? await Task.sleep(for: .seconds(1.5))
            if flow.step == .sent { panel?.orderOut(nil) }
        }
    }

    private func makePanel() -> VoicePanelWindow {
        let p = VoicePanelWindow()
        p.contentView = NSHostingView(rootView: VoicePanelView(controller: self).environment(store).tint(Theme.accent))
        p.onKey = { [weak self] in self?.handle($0) ?? false }
        panel = p
        return p
    }

    /// ↵ sends, esc cancels: Loft stays in the background, so the panel takes these keys itself.
    private func handle(_ e: NSEvent) -> Bool {
        switch Int(e.keyCode) {
        case kVK_Return, kVK_ANSI_KeypadEnter:
            guard flow.canSend else { return false }
            Task { await flow.send() }
        case kVK_Escape: close()
        default: return false
        }
        return true
    }
}

/// The panel's card: the shared `VoiceCard` on the web's page colour, floating with a shadow.
struct VoicePanelView: View {
    static let size = CGSize(width: 520, height: 400)
    let controller: VoicePanelController

    var body: some View {
        VoiceCard(flow: controller.flow, cancel: controller.close, toChat: controller.toChat, picking: controller.opensMore)
            .padding(20)
            .background(WebTheme.canvas, in: RoundedRectangle(cornerRadius: WebTheme.panelRadius))
            .overlay(RoundedRectangle(cornerRadius: WebTheme.panelRadius).strokeBorder(WebTheme.line))
            .shadow(color: .black.opacity(0.35), radius: 18, y: 8)
            .padding(20)
            .frame(width: Self.size.width, height: Self.size.height)
            .onChange(of: controller.flow.step) { _, s in controller.stepChanged(s) }
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
/// `-show-voice-panel <fixture>`: shows the panel in one of `VoiceSession`'s fixture phases (listening, cleaning,
/// review, sending, failed) and leaves it open, for UI tests without a microphone.
@MainActor
func showVoicePanel(_ c: VoicePanelController, store: LoftStore, fixture: String) async {
    for _ in 0..<80 where store.agents.isEmpty { try? await Task.sleep(for: .milliseconds(100)) }
    c.closesWhenSent = false
    c.show(listening: false)
    c.flow.voice.applyFixture(fixture, agentId: store.orchestratorId ?? store.agents.first?.id ?? "")
    c.flow.listened = true
    if fixture == "review" { c.flow.suggested = store.agents.first { $0.id != store.orchestratorId } }
}

/// `-render-voice-panel <fixture> <file.png>`: the panel as `-show-voice-panel` shows it, drawn to a PNG, then quits.
/// For screenshots without a microphone. `<fixture>+more` also opens More, drawn over the panel where it pops up.
@MainActor
func renderVoicePanel(_ c: VoicePanelController, store: LoftStore, fixture: String, to file: String) async {
    c.opensMore = fixture.hasSuffix("+more")
    await showVoicePanel(c, store: store, fixture: fixture.replacingOccurrences(of: "+more", with: ""))
    try? await Task.sleep(for: .seconds(1.5))
    // The panel, then any popover window on top of it, each at its place on screen.
    let windows = NSApp.windows.filter { $0.isVisible && ($0 is NSPanel || $0.className.contains("Popover")) }
        .sorted { ($0 is NSPanel ? 0 : 1) < ($1 is NSPanel ? 0 : 1) }
    guard let frame = windows.map(\.frame).reduce(nil, { $0?.union($1) ?? $1 }) else { return NSApp.terminate(nil) }
    let image = NSImage(size: frame.size, flipped: false) { _ in
        for w in windows {
            guard let view = w.contentView, let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { continue }
            view.cacheDisplay(in: view.bounds, to: rep)
            rep.draw(in: NSRect(x: w.frame.minX - frame.minX, y: w.frame.minY - frame.minY, width: w.frame.width, height: w.frame.height))
        }
        return true
    }
    if let tiff = image.tiffRepresentation, let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) {
        try? png.write(to: URL(fileURLWithPath: file))
    }
    NSApp.terminate(nil)
}
#endif
