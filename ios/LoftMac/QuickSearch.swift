import AppKit
import Carbon.HIToolbox
import LoftKit
import SwiftUI

/// The global shortcuts Settings offers for quick search (⌥Space by default) and the voice panel (⇧⌥Space).
enum MacShortcut: String, CaseIterable, Identifiable {
    case optionSpace = "option-space", shiftOptionSpace = "shift-option-space", optionCommandSpace = "option-command-space",
         controlOptionSpace = "control-option-space", off

    var id: String { rawValue }

    init(stored: String?, default fallback: MacShortcut = .optionSpace) { self = stored.flatMap(Self.init(rawValue:)) ?? fallback }

    var label: String {
        switch self {
        case .optionSpace: "⌥Space"
        case .shiftOptionSpace: "⇧⌥Space"
        case .optionCommandSpace: "⌥⌘Space"
        case .controlOptionSpace: "⌃⌥Space"
        case .off: "Off"
        }
    }

    /// The Carbon modifiers with Space, or nil when off.
    fileprivate var modifiers: UInt32? {
        switch self {
        case .optionSpace: UInt32(optionKey)
        case .shiftOptionSpace: UInt32(shiftKey | optionKey)
        case .optionCommandSpace: UInt32(optionKey | cmdKey)
        case .controlOptionSpace: UInt32(controlKey | optionKey)
        case .off: nil
        }
    }
}

/// The shortcuts that couldn't be registered, by hot key id: Loft's other shortcut has it, or macOS doesn't allow it.
/// Settings shows it under the picker. Another app holding the same keys isn't an error to Carbon: only one of the apps
/// gets the presses.
@MainActor
@Observable
final class HotKeyFailures {
    static let shared = HotKeyFailures()
    var byID: [UInt32: MacShortcut] = [:]
}

/// A system-wide shortcut through Carbon's hot keys: it works from any app and needs no Accessibility permission.
/// Each one has its own `id`, and passes the others' presses on.
final class GlobalHotKey {
    static let quickSearch: UInt32 = 1, voicePanel: UInt32 = 2

    private var ref: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private let id: UInt32
    private let action: () -> Void

    init(id: UInt32, action: @escaping () -> Void) {
        self.id = id
        self.action = action
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, event, me in
            guard let event, let me else { return OSStatus(eventNotHandledErr) }
            let hotKey = Unmanaged<GlobalHotKey>.fromOpaque(me).takeUnretainedValue()
            var pressed = EventHotKeyID()
            GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID), nil,
                              MemoryLayout<EventHotKeyID>.size, nil, &pressed)
            guard pressed.id == hotKey.id else { return OSStatus(eventNotHandledErr) }
            DispatchQueue.main.async { hotKey.action() }
            return noErr
        }, 1, &spec, Unmanaged.passUnretained(self).toOpaque(), &handler)
    }

    @MainActor
    func set(_ shortcut: MacShortcut) {
        if let ref { UnregisterEventHotKey(ref) }
        ref = nil
        HotKeyFailures.shared.byID[id] = nil
        guard let modifiers = shortcut.modifiers else { return }
        let status = RegisterEventHotKey(UInt32(kVK_Space), modifiers, EventHotKeyID(signature: OSType(0x4C_4F_46_54), id: id),
                                         GetApplicationEventTarget(), 0, &ref)
        if status != noErr {
            ref = nil
            HotKeyFailures.shared.byID[id] = shortcut
        }
    }
}

/// A floating panel that takes typing without bringing Loft to the front, like Spotlight. It closes when it loses
/// the keyboard (a click outside), and hands its keys to `onKey` first.
private final class QuickSearchPanel: NSPanel {
    var onKey: ((NSEvent) -> Bool)?

    init() {
        super.init(contentRect: NSRect(x: 0, y: 0, width: 640, height: 470), styleMask: [.borderless, .nonactivatingPanel],
                   backing: .buffered, defer: true)
        isFloatingPanel = true
        level = .floating
        hidesOnDeactivate = false
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

    override func resignKey() {
        super.resignKey()
        orderOut(nil)
    }
}

/// What the panel shows: the query, its results from the store, and which one is selected.
@MainActor
@Observable
final class QuickSearchModel {
    let store: LoftStore
    var query = "" { didSet { selection = 0 } }
    var selection = 0
    /// Bumped each time the panel opens, so the field takes the keyboard again.
    var opened = 0

    init(store: LoftStore) { self.store = store }

    var results: [QuickSearch.Result] {
        QuickSearch.results(query, agents: store.agents, tasks: store.tasks, deliveries: Array(store.deliverySummaries.values))
    }

    /// Why only pages can be found: Loft isn't connected, or hasn't loaded its agents and issues yet.
    var unavailable: String? {
        guard store.agents.isEmpty && store.tasks.isEmpty else { return nil }
        switch store.connection {
        case .connected, .polling: return nil
        case .connecting: return "Loading Loft…"
        case .notConfigured, .reconnecting: return "Not connected to Loft"
        }
    }
}

/// Owns the shortcut and the panel, and opens the picked result in the app or the browser.
@MainActor
final class QuickSearchController {
    let model: QuickSearchModel
    private let nav: MacNav
    /// Opens or raises the main window; set by the menu bar item, which is there for the app's whole life.
    var showWindow: (() -> Void)?
    private var panel: QuickSearchPanel?
    private var hotKey: GlobalHotKey!
    private var shortcut: MacShortcut?

    init(store: LoftStore, nav: MacNav) {
        model = QuickSearchModel(store: store)
        self.nav = nav
        hotKey = GlobalHotKey(id: GlobalHotKey.quickSearch) { [weak self] in self?.toggle() }
        applyShortcut()
        NotificationCenter.default.addObserver(forName: UserDefaults.didChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.applyShortcut() }
            // Again once every observer has run: the other shortcut may only now have let go of these keys.
            DispatchQueue.main.async { self?.applyShortcut() }
        }
        #if DEBUG
        // Needs no window: a directly launched app may not open one.
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-render-quick-search"), i + 2 < args.count {
            Task { await renderQuickSearch(self, query: args[i + 1], to: args[i + 2]) }
        }
        #endif
    }

    private func applyShortcut() {
        let s = MacShortcut(stored: UserDefaults.standard.string(forKey: Prefs.quickSearchShortcut))
        // Tries again when it failed, as the clash may be gone.
        guard s != shortcut || HotKeyFailures.shared.byID[GlobalHotKey.quickSearch] != nil else { return }
        shortcut = s
        hotKey.set(s)
    }

    func toggle() {
        if panel?.isVisible == true { close() } else { show() }
    }

    func show() {
        let panel = panel ?? makePanel()
        model.query = ""
        model.opened += 1
        // Centred, a third of the way down the screen with the mouse, like Spotlight.
        if let screen = NSScreen.screens.first(where: { $0.frame.contains(NSEvent.mouseLocation) }) ?? NSScreen.main {
            let f = screen.visibleFrame
            panel.setFrameTopLeftPoint(NSPoint(x: f.midX - panel.frame.width / 2, y: f.maxY - f.height / 5))
        }
        panel.makeKeyAndOrderFront(nil)
    }

    func close() { panel?.orderOut(nil) }

    private func makePanel() -> QuickSearchPanel {
        let p = QuickSearchPanel()
        p.contentView = NSHostingView(rootView: QuickSearchView(model: model) { [weak self] r, flags in self?.open(r, flags) })
        p.onKey = { [weak self] in self?.handle($0) ?? false }
        panel = p
        return p
    }

    /// ↑ ↓ move, ↵ opens (⌘ the agent's chat, ⌥ the other of app and browser), esc closes.
    private func handle(_ e: NSEvent) -> Bool {
        let results = model.results
        switch Int(e.keyCode) {
        case kVK_DownArrow: model.selection = min(model.selection + 1, max(results.count - 1, 0))
        case kVK_UpArrow: model.selection = max(model.selection - 1, 0)
        case kVK_Return, kVK_ANSI_KeypadEnter:
            if results.indices.contains(model.selection) { open(results[model.selection], e.modifierFlags) }
        case kVK_Escape: close()
        default: return false
        }
        return true
    }

    private func open(_ r: QuickSearch.Result, _ flags: NSEvent.ModifierFlags) {
        close()
        let chat = flags.contains(.command) && r.kind == .agent
        let browserDefault = UserDefaults.standard.string(forKey: Prefs.quickSearchOpensIn) == "browser"
        if !chat, r.webOnly || flags.contains(.option) != browserDefault, let base = model.store.api?.baseURL {
            NSWorkspace.shared.open(base.appendingPathComponent(r.webPath))
            return
        }
        go(r.kind, r.target, webPath: r.webPath)
    }

    /// A Loft item chosen in macOS Spotlight: opens it in the app.
    func open(spotlightItem identifier: String) {
        guard let (kind, id) = SpotlightIndex.target(of: identifier) else { return }
        go(kind, id, webPath: "")
    }

    /// Goes to the agent, issue or page in the main window, opening or raising it.
    private func go(_ kind: QuickSearch.Kind, _ target: String, webPath: String) {
        switch kind {
        case .agent: nav.openChat(target)
        case .spec, .ticket, .issue:
            nav.section = .issues
            nav.issuePath = [target]
        case .page:
            nav.section = Self.sections[webPath] ?? .overview
        }
        showWindow?()
        NSApp.activate(ignoringOtherApps: true)
    }

    private static let sections: [String: MacSection] = [
        "/repos": .overview, "/agents": .agents, "/issues": .issues, "/needs-you": .needsYou, "/settings": .settings,
    ]
}

struct QuickSearchView: View {
    @Bindable var model: QuickSearchModel
    let onPick: (QuickSearch.Result, NSEvent.ModifierFlags) -> Void
    @FocusState private var focused: Bool

    var body: some View {
        let results = model.results
        let typed = !model.query.trimmingCharacters(in: .whitespaces).isEmpty
        let note = !typed ? nil : model.unavailable.map { "\($0): only pages are searched" } ?? (results.isEmpty ? "Nothing matches" : nil)
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass").font(.system(size: 20)).foregroundStyle(WebTheme.fg3)
                    .accessibilityHidden(true)
                TextField("Search agents, issues, tickets and specs", text: $model.query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 20))
                    .focused($focused)
                    .accessibilityIdentifier("quickSearchField")
            }
            .padding(14)
            if let note {
                Divider()
                Text(note).foregroundStyle(WebTheme.fg3).padding(14).frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("quickSearchNote")
            }
            if !results.isEmpty {
                Divider()
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(spacing: 2) {
                            ForEach(Array(results.enumerated()), id: \.element.id) { i, r in
                                Button { onPick(r, NSEvent.modifierFlags) } label: { row(r, selected: i == model.selection) }
                                    .buttonStyle(.plain)
                                    .id(i)
                            }
                        }
                        .padding(6)
                    }
                    // Rows are about 33pt: as tall as they need, up to nine.
                    .frame(height: min(CGFloat(results.count) * 33 + 12, 309))
                    .onChange(of: model.selection) { proxy.scrollTo(model.selection) }
                }
                Divider()
                Text("↵ Open   ⌘↵ Chat   ⌥↵ \(UserDefaults.standard.string(forKey: Prefs.quickSearchOpensIn) == "browser" ? "Mac app" : "Browser")   esc Close")
                    .font(.caption.monospaced())
                    .foregroundStyle(WebTheme.fg3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
            }
        }
        .background(WebTheme.canvas, in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(WebTheme.fg3.opacity(0.25)))
        .shadow(color: .black.opacity(0.35), radius: 18, y: 8)
        .padding(20)
        .frame(width: 640, height: 470, alignment: .top)
        .onAppear { focused = true }
        .onChange(of: model.opened) { focused = true }
    }

    private func row(_ r: QuickSearch.Result, selected: Bool) -> some View {
        HStack(spacing: 10) {
            Image(systemName: Self.symbol(r.kind)).frame(width: 20).foregroundStyle(selected ? Theme.accent : WebTheme.fg3)
                .accessibilityHidden(true)
            Text(r.title).lineLimit(1)
            Spacer(minLength: 8)
            if let n = r.number { Text(n).font(.caption.monospaced()) }
            Text(r.kind.label).font(.caption.weight(.medium))
                .padding(.horizontal, 6).padding(.vertical, 2)
                .background(.quaternary, in: Capsule())
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(selected ? Theme.accent.opacity(0.18) : .clear, in: RoundedRectangle(cornerRadius: 8))
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private static func symbol(_ k: QuickSearch.Kind) -> String {
        switch k {
        case .agent: "person.crop.circle"
        case .spec: "doc.text"
        case .ticket: "ticket"
        case .issue: "smallcircle.filled.circle"
        case .page: "rectangle.on.rectangle"
        }
    }
}

/// Settings › Quick search: the shortcut, or none, and where Enter opens a result; and Spotlight on or off.
struct QuickSearchSettings: View {
    @AppStorage(Prefs.quickSearchShortcut) private var shortcut = MacShortcut.optionSpace.rawValue
    @AppStorage(Prefs.quickSearchOpensIn) private var opensIn = "app"
    @AppStorage(Prefs.spotlight) private var spotlight = true

    var body: some View {
        Section("Quick search") {
            Picker("Shortcut", selection: $shortcut) {
                ForEach(MacShortcut.allCases) { Text($0.label).tag($0.rawValue) }
            }
            .accessibilityIdentifier("quickSearchShortcutPicker")
            ShortcutFailure(id: GlobalHotKey.quickSearch)
            Picker("Open results in", selection: $opensIn) {
                Text("Mac app").tag("app")
                Text("Browser").tag("browser")
            }
            .pickerStyle(.segmented)
            Text("Finds agents, issues, tickets and specs from any app. ↵ opens, ⌘↵ opens an agent's chat, ⌥↵ opens in the other place. Pages only the web has always open in the browser. If the shortcut does nothing, another app may use it too: pick another.")
                .font(.caption).foregroundStyle(.secondary)
        }
        Section("Spotlight") {
            Toggle("Show Loft in Spotlight", isOn: $spotlight).accessibilityIdentifier("spotlightToggle")
            Text("⌘Space finds agents and open issues, tickets and specs, and opens them here. Off removes them from Spotlight.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// "⌥Space couldn't be set up…" under a shortcut picker, when it couldn't be registered.
struct ShortcutFailure: View {
    let id: UInt32

    var body: some View {
        if let failed = HotKeyFailures.shared.byID[id] {
            Text("\(failed.label) couldn't be set up: Loft's other shortcut may use it. Pick another.")
                .font(.caption).foregroundStyle(.red)
                .accessibilityIdentifier("shortcutFailure")
        }
    }
}

#if DEBUG
/// `-render-quick-search <query> <file.png>`: opens the panel with the query typed, draws it to a PNG, then quits. For
/// screenshots when a UI test can't drive the app.
@MainActor
func renderQuickSearch(_ controller: QuickSearchController, query: String, to file: String) async {
    for _ in 0..<80 where controller.model.store.agents.isEmpty { try? await Task.sleep(for: .milliseconds(100)) }
    controller.show()
    controller.model.query = query
    try? await Task.sleep(for: .seconds(1.5))
    if let view = NSApp.windows.first(where: { $0 is NSPanel && $0.isVisible })?.contentView,
       let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
        view.cacheDisplay(in: view.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: file))
    }
    NSApp.terminate(nil)
}
#endif
