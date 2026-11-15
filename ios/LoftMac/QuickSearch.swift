import AppKit
import Carbon.HIToolbox
import LoftKit
import SwiftUI

/// A floating panel that takes typing without bringing Loft to the front, like Spotlight. It closes when it loses
/// the keyboard (a click outside), and hands its keys to `onKey` first.
private final class QuickSearchPanel: NSPanel {
    var onKey: ((NSEvent) -> Bool)?

    init() {
        super.init(contentRect: NSRect(x: 0, y: 0, width: QuickSearchView.size.width, height: QuickSearchView.size.height), styleMask: [.borderless, .nonactivatingPanel],
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
    /// Herdr UI is set up on this Mac, so agents can open in it (checked each time the panel opens).
    var herdrReady = false
    /// The check that sets `herdrReady`: Enter waits for it, so the first Enter already knows.
    var herdrCheck: Task<Void, Never>?
    /// Enter opens agents in Herdr UI, ⌘Enter in the Mac app (Settings › Herdr UI).
    var herdrFirst: Bool { herdrReady && UserDefaults.standard.string(forKey: Prefs.agentsOpenIn) == "herdr" }
    var browserDefault: Bool { UserDefaults.standard.string(forKey: Prefs.quickSearchOpensIn) == "browser" }

    func checkHerdr() {
        guard !store.isDemo else { return }
        herdrCheck = Task { herdrReady = (try? await store.herdrStatus())?["linked"]?["root"]?.stringValue != nil }
    }

    /// Waits for the Herdr check, for a second at most; slower than that counts as not linked.
    func waitForHerdr() async {
        guard let check = herdrCheck else { return }
        if await within(seconds: 1, { await check.value; return true }) == nil { herdrReady = false }
    }

    /// Where an agent opens for these keys; the footer and Enter both ask this.
    func opening(command: Bool = false, option: Bool = false) -> AgentOpening {
        AgentOpening.choose(herdrReady: herdrReady, herdrFirst: herdrFirst, browserDefault: browserDefault, command: command, option: option)
    }

    init(store: LoftStore) { self.store = store }

    var results: [QuickSearch.Result] {
        QuickSearch.results(query, agents: store.agents, tasks: store.tasks, deliveries: Array(store.deliverySummaries.values),
                            pullRequests: store.pullRequests)
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
    private var binding: ShortcutBinding!

    init(store: LoftStore, nav: MacNav) {
        model = QuickSearchModel(store: store)
        self.nav = nav
        binding = ShortcutBinding(.quickSearch) { [weak self] in self?.toggle() }
        #if DEBUG
        // Needs no window: a directly launched app may not open one.
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-render-chat-note"), i + 1 < args.count { Task { renderChatNote(to: args[i + 1]) } }
        if let i = args.firstIndex(of: "-render-quick-search"), i + 2 < args.count {
            Task { await renderQuickSearch(self, query: args[i + 1], to: args[i + 2]) }
        }
        #endif
    }

    func toggle() {
        if panel?.isVisible == true { close() } else { show() }
    }

    func show() {
        let panel = panel ?? makePanel()
        model.query = ""
        model.opened += 1
        model.checkHerdr()
        // Centred, a third of the way down the screen with the mouse, like Spotlight.
        if let screen = NSScreen.screens.first(where: { $0.frame.contains(NSEvent.mouseLocation) }) ?? NSScreen.main {
            let f = screen.visibleFrame
            panel.setFrameTopLeftPoint(NSPoint(x: f.midX - panel.frame.width / 2, y: f.maxY - f.height / 5))
        }
        panel.makeKeyAndOrderFront(nil)
    }

    func close() { panel?.orderOut(nil) }

    private func makePanel() -> QuickSearchPanel {
        // The panel can open before the main window, whose root view otherwise applies Settings › Appearance.
        Appearance.apply()
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
        // The demo has no web Loft to open, and its pull requests aren't real: everything else opens here.
        if model.store.isDemo {
            if r.kind != .pullRequest { go(r.kind, r.target, webPath: r.webPath) }
            return
        }
        if r.kind == .pullRequest {
            if let url = r.link(loftHost: model.store.api?.baseURL.host()) { NSWorkspace.shared.open(url) }
            return
        }
        if r.kind == .agent {
            Task {
                await model.waitForHerdr()
                switch model.opening(command: flags.contains(.command), option: flags.contains(.option)) {
                case .herdr: openInHerdr(r.target)
                case .browser: openBrowser(r)
                case .chat: go(.agent, r.target, webPath: r.webPath)
                }
            }
            return
        }
        if r.webOnly || flags.contains(.option) != model.browserDefault {
            openBrowser(r)
            return
        }
        go(r.kind, r.target, webPath: r.webPath)
    }

    private func openBrowser(_ r: QuickSearch.Result) {
        if let base = model.store.api?.baseURL { NSWorkspace.shared.open(base.appendingPathComponent(r.webPath)) } else { go(r.kind, r.target, webPath: r.webPath) }
    }

    /// A Loft item chosen in macOS Spotlight: an agent in Herdr when Settings › Herdr UI says so, else where
    /// Settings › Quick search says, the app or the browser.
    func open(spotlightItem identifier: String) {
        if !model.store.isDemo, UserDefaults.standard.string(forKey: Prefs.agentsOpenIn) == "herdr",
           let (kind, id) = SpotlightIndex.target(of: identifier), kind == .agent {
            // Only when Herdr is linked, as in quick search: else it opens as Settings › Quick search says.
            Task {
                model.checkHerdr()
                await model.waitForHerdr()
                if model.herdrReady { openInHerdr(id) } else { openSpotlight(identifier) }
            }
            return
        }
        openSpotlight(identifier)
    }

    private func openSpotlight(_ identifier: String) {
        let opensIn = model.store.isDemo ? nil : UserDefaults.standard.string(forKey: Prefs.quickSearchOpensIn)
        if let url = SpotlightIndex.webURL(of: identifier, opensIn: opensIn, base: model.store.api?.baseURL) {
            NSWorkspace.shared.open(url)
            return
        }
        guard let (kind, id) = SpotlightIndex.target(of: identifier) else { return }
        go(kind, id, webPath: "")
    }

    /// Shows the agent's tab in Herdr UI; when Herdr can't, opens its chat here with a line saying why.
    private func openInHerdr(_ agentId: String) {
        Task {
            do {
                try await model.store.herdrFocus(agentId: agentId)
            } catch {
                go(.agent, agentId, webPath: "")
                nav.chatNote = (agentId, "Opened here: \(APIError.from(error).title)")
            }
        }
    }

    /// Goes to the agent, issue or page in the main window, opening or raising it.
    private func go(_ kind: QuickSearch.Kind, _ target: String, webPath: String) {
        switch kind {
        case .pullRequest:
            return // opened by `open(_:_:)`; Spotlight has no pull requests.
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
    /// The panel's window, the card plus room for its shadow.
    static let size = CGSize(width: 800, height: 660)
    /// A row's height, as the web's ⌘K search: a 40pt sticker and two lines.
    private static let rowHeight: CGFloat = 54

    @Bindable var model: QuickSearchModel
    let onPick: (QuickSearch.Result, NSEvent.ModifierFlags) -> Void
    @FocusState private var focused: Bool

    var body: some View {
        let results = model.results
        let typed = !model.query.trimmingCharacters(in: .whitespaces).isEmpty
        let note = !typed ? nil : model.unavailable.map { "\($0): only pages are searched" } ?? (results.isEmpty ? "Nothing matches" : nil)
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass").font(.system(size: 20)).foregroundStyle(WebTheme.fg3)
                    .accessibilityHidden(true)
                TextField("Search agents, specs, issues and pull requests…", text: $model.query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 22))
                    .focused($focused)
                    .accessibilityIdentifier("quickSearchField")
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 16)
            if let note {
                Divider()
                Text(note).font(.system(size: 13.5)).foregroundStyle(WebTheme.fg3).padding(18)
                    .frame(maxWidth: .infinity, alignment: .leading)
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
                    // As tall as the rows need, up to eight and a half, so a ninth shows there's more.
                    .frame(height: min(CGFloat(results.count) * (Self.rowHeight + 2) + 12, Self.rowHeight * 8.5 + 12))
                    .onChange(of: model.selection) { proxy.scrollTo(model.selection) }
                }
                Divider()
                Text("↵ Open   ⌘↵ \(model.opening(command: true) == .herdr ? "Herdr" : "Chat")   ⌥↵ \(model.opening(option: true) == .browser ? "Browser" : "Mac app")   esc Close")
                    .font(.system(size: 11.5, design: .monospaced))
                    .foregroundStyle(WebTheme.fg3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 10)
            }
        }
        .foregroundStyle(WebTheme.fg)
        .background(WebTheme.surface, in: RoundedRectangle(cornerRadius: WebTheme.panelRadius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: WebTheme.panelRadius, style: .continuous).strokeBorder(WebTheme.lineStrong))
        .shadow(color: .black.opacity(0.35), radius: 18, y: 8)
        .padding(20)
        .frame(width: Self.size.width, height: Self.size.height, alignment: .top)
        .onAppear { focused = true }
        .onChange(of: model.opened) { focused = true }
    }

    /// The web's search row: an agent's sticker, name, "project · role" and status; or an issue's, spec's or pull
    /// request's icon, title, "#480 · open · loft" and kind.
    private func row(_ r: QuickSearch.Result, selected: Bool) -> some View {
        let agent = r.kind == .agent ? model.store.agent(r.target) : nil
        let status = agent.map(QuickSearch.AgentStatus.init)
        let closed = r.closed
        return HStack(spacing: 12) {
            Group {
                if let agent {
                    AgentSticker(agent: agent, size: 40)
                        .overlay(alignment: .bottomTrailing) {
                            Circle().fill(status?.dot ?? WebTheme.fg3).frame(width: 11, height: 11)
                                .overlay(Circle().strokeBorder(WebTheme.surface, lineWidth: 2))
                        }
                } else {
                    Image(systemName: Self.symbol(r.kind)).font(.system(size: 17)).foregroundStyle(closed ? WebTheme.fg3 : WebTheme.fg2)
                }
            }
            .frame(width: 40, height: 40)
            .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(r.title).font(.system(size: 15)).foregroundStyle(closed ? WebTheme.fg2 : WebTheme.fg).lineLimit(1)
                if let d = r.detail, !d.isEmpty {
                    Text(d).font(.system(size: 11.5, design: .monospaced)).foregroundStyle(WebTheme.fg3).lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            Group {
                if let status {
                    Text(status.label).foregroundStyle(status.text)
                } else {
                    Text(r.kind.label).foregroundStyle(WebTheme.fg3)
                }
            }
            .font(.system(size: 11.5, design: .monospaced)).textCase(.uppercase).tracking(1.2)
            if r.webOnly {
                Image(systemName: "arrow.up.right.square").font(.system(size: 12)).foregroundStyle(WebTheme.fg3)
                    .accessibilityLabel("Opens in the browser")
            }
        }
        .padding(.horizontal, 10)
        .frame(height: Self.rowHeight)
        .background(selected ? WebTheme.fg.opacity(0.06) : .clear, in: RoundedRectangle(cornerRadius: WebTheme.rowRadius, style: .continuous))
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
        case .pullRequest: "arrow.triangle.pull"
        case .page: "rectangle.on.rectangle"
        }
    }
}

/// Settings › Quick search: the shortcut, or none, and where Enter opens a result; and Spotlight on or off.
struct QuickSearchSettings: View {
    @AppStorage(Prefs.quickSearchOpensIn) private var opensIn = "app"
    @AppStorage(Prefs.spotlight) private var spotlight = true

    var body: some View {
        Section("Quick search") {
            ShortcutRow(.quickSearch)
            Picker("Open results in", selection: $opensIn) {
                Text("Mac app").tag("app")
                Text("Browser").tag("browser")
            }
            .pickerStyle(.segmented)
            Text("Finds agents, issues, tickets and specs from any app. ↵ opens, ⌘↵ opens an agent's chat, ⌥↵ opens in the other place. Pages only the web has always open in the browser. Spotlight opens Loft items in the same place. If the shortcut does nothing, another app may use it too: record another.")
                .font(.caption).foregroundStyle(.secondary)
        }
        Section("Spotlight") {
            Toggle("Show Loft in Spotlight", isOn: $spotlight).accessibilityIdentifier("spotlightToggle")
            Text("⌘Space finds agents and open issues, tickets and specs, and opens them where quick search does: in the Mac app or the browser. Off removes them from Spotlight.")
                .font(.caption).foregroundStyle(.secondary)
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
    // `-render-herdr-ready`: as if Herdr UI were linked (the demo has no Herdr to ask); pair it with `-agentsOpenIn herdr|loft`.
    if ProcessInfo.processInfo.arguments.contains("-render-herdr-ready") { controller.model.herdrReady = true }
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

extension QuickSearch.AgentStatus {
    /// The status dot's colour, as the web's `statusOf`.
    var dot: Color {
        switch self {
        case .needsYou: WebTheme.warn
        case .waiting: WebTheme.info
        case .working: Theme.accent
        case .error: WebTheme.bad
        case .sleeping: WebTheme.info.opacity(0.7)
        case .idle: WebTheme.fg3
        }
    }

    /// The status label's colour, as the web's `statusOf`.
    var text: Color { self == .sleeping ? WebTheme.fg2 : dot }
}
