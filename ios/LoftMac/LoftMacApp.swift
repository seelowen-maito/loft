import CoreSpotlight
import LoftKit
import SwiftUI

/// Loft for Mac (spec #382): the same client as the iPhone app, in Mac windows with a sidebar of sections.
@main
struct LoftMacApp: App {
    @NSApplicationDelegateAdaptor(MacAppDelegate.self) private var delegate
    @State private var store: LoftStore
    @State private var nav: MacNav
    @State private var alerts: MacAlerts
    @State private var voice: VoiceSession
    @State private var quickSearch: QuickSearchController
    @State private var voicePanel: VoicePanelController
    @State private var spotlight: MacSpotlight?

    init() {
        Prefs.registerDefaults()
        let store = LoftStore(api: Self.makeAPI())
        // Live from launch: the menu bar item needs it with the window closed.
        store.activate()
        _store = State(initialValue: store)
        let nav = MacNav()
        _nav = State(initialValue: nav)
        _alerts = State(initialValue: MacAlerts(store: store, nav: nav))
        let voice = VoiceSession(store: store)
        _voice = State(initialValue: voice)
        _voicePanel = State(initialValue: VoicePanelController(store: store, nav: nav, voice: voice))
        let quickSearch = QuickSearchController(store: store, nav: nav)
        _quickSearch = State(initialValue: quickSearch)
        MacAppDelegate.openSpotlightItem = { quickSearch.open(spotlightItem: $0) }
        _spotlight = State(initialValue: Self.indexesSpotlight ? MacSpotlight(store: store) : nil)
    }

    private static func makeAPI() -> LoftAPI? {
        #if DEBUG
        let args = ProcessInfo.processInfo.arguments
        if args.contains("-ui-onboarding") {
            UserDefaults.standard.removeObject(forKey: Prefs.serverURL)
            return nil
        }
        if args.contains("-ui-testing") {
            SnapshotStore.standard.clear()
            return FixtureAPI.fromLaunchArguments()
        }
        #endif
        return UserDefaults.standard.string(forKey: Prefs.serverURL).flatMap(LiveLoftAPI.normalize).map(LiveLoftAPI.init(baseURL:))
    }

    /// Fixture runs (UI tests, screenshots) keep their sample agents and issues out of this Mac's Spotlight.
    private static var indexesSpotlight: Bool {
        #if DEBUG
        let args = ProcessInfo.processInfo.arguments
        return !args.contains("-ui-testing") && !args.contains("-ui-onboarding")
        #else
        return true
        #endif
    }

    var body: some Scene {
        Window("Loft", id: "main") {
            MacRootView()
                .tint(Theme.accent)
                .environment(store)
                .environment(nav)
                .environment(voice)
                .frame(minWidth: 1180, minHeight: 640)
                .onAppear { store.activate() }
                // A tap on a widget.
                .onOpenURL { url in if let link = DeepLink(url: url) { nav.open(link) } }
                // A Loft item chosen in Spotlight, when SwiftUI hands it to the window rather than the app delegate.
                .onContinueUserActivity(CSSearchableItemActionType) { activity in
                    if let id = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String { quickSearch.open(spotlightItem: id) }
                }
                #if DEBUG
                .task {
                    let args = ProcessInfo.processInfo.arguments
                    if let i = args.firstIndex(of: "-render-menubar"), i + 1 < args.count { await renderMenuBar(store: store, nav: nav, to: args[i + 1]) }
                    if let i = args.firstIndex(of: "-render-window"), i + 1 < args.count { await renderWindow(store: store, nav: nav, to: args[i + 1]) }
                }
                #endif
                .onReceive(NotificationCenter.default.publisher(for: NSApplication.willTerminateNotification)) { _ in store.saveNow() }
        }
        // Room for the sidebar, the agents, the chat and the agent's settings side by side.
        .defaultSize(width: 1400, height: 860)
        .commands { LoftCommands(nav: nav) }

        MenuBarExtra { MenuBarContent(voicePanel: voicePanel).tint(Theme.accent).environment(store).environment(nav) } label: { MenuBarLabel(alerts: alerts, quickSearch: quickSearch).environment(store) }
            .menuBarExtraStyle(.window)

        Settings {
            AddressForm(isOnboarding: false)
                .tint(Theme.accent)
                .environment(store)
                .frame(width: 520)
                .padding()
        }
    }
}

/// The window's sections, in sidebar order; ⌘1 to ⌘5 open them.
enum MacSection: Int, CaseIterable, Identifiable {
    case overview = 1, agents, issues, needsYou, settings

    var id: Int { rawValue }

    var title: String {
        switch self {
        case .overview: "Overview"
        case .agents: "Agents"
        case .issues: "Issues"
        case .needsYou: "Needs you"
        case .settings: "Settings"
        }
    }

    var symbol: String {
        switch self {
        case .overview: "square.grid.2x2"
        case .agents: "person.3"
        case .issues: "checklist"
        case .needsYou: "bell.badge"
        case .settings: "gearshape"
        }
    }
}

/// Where the window is: the section, the agent whose chat is open, the issue pages open, and the New message picker.
@MainActor
@Observable
final class MacNav {
    var section: MacSection = .agents
    var agentId: String?
    var picksRecipient = false
    /// The issue pages open in the Issues section, from the board's card on.
    var issuePath: [String] = []

    func openChat(_ agentId: String) {
        section = .agents
        self.agentId = agentId
    }

    func open(_ link: DeepLink) {
        switch link {
        case .needsYou: section = .needsYou
        case .agents: section = .agents
        case .agent(let id): openChat(id)
        }
    }
}

struct LoftCommands: Commands {
    let nav: MacNav
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New Message…") { show { nav.picksRecipient = true } }
                .keyboardShortcut("n")
        }
        CommandMenu("Go") {
            ForEach(MacSection.allCases) { s in
                Button(s.title) { show { nav.section = s } }
                    .keyboardShortcut(KeyEquivalent(Character(String(s.rawValue))))
            }
            Divider()
            Button("Back") { nav.issuePath.removeLast() }
                .keyboardShortcut("[")
                .disabled(nav.section != .issues || nav.issuePath.isEmpty)
        }
    }

    /// Goes there in the main window, opening or raising it: with only the menu bar item left, a shortcut still shows
    /// where it went.
    private func show(_ go: () -> Void) {
        go()
        openWindow(id: "main")
        NSApp.activate(ignoringOtherApps: true)
    }
}

/// First the address, then the window's sections. Loft unreachable with nothing loaded yet says so.
struct MacRootView: View {
    @Environment(LoftStore.self) private var store
    @AppStorage(Prefs.appearance) private var appearance = AppearanceChoice.dark.rawValue

    var body: some View {
        content
            .background(WebTheme.canvas)
            .onChange(of: appearance, initial: true) { Appearance.apply(appearance) }
    }

    @ViewBuilder private var content: some View {
        if store.connection == .notConfigured {
            AddressForm(isOnboarding: true)
                .frame(maxWidth: 520)
                .padding(32)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if store.lastSync == nil, case .reconnecting(_, let error) = store.connection {
            UnreachableScreen(error: error)
        } else {
            MainWindow()
        }
    }
}

/// Loft didn't answer at the saved address and nothing is cached: why, and what to try.
struct UnreachableScreen: View {
    @Environment(LoftStore.self) private var store
    let error: APIError

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "bolt.horizontal.circle").font(.system(size: 44)).foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text("Can't reach Loft").font(.title2.bold()).accessibilityIdentifier("unreachableTitle")
            Text("Nothing answered at \(store.api?.baseURL.absoluteString ?? "the saved address"). Check that Loft is running on that Mac, or change the address.")
                .multilineTextAlignment(.center).foregroundStyle(.secondary)
            Text(error.title).font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
            HStack {
                Button("Try again") { store.reconnect() }.keyboardShortcut(.defaultAction)
                Button("Change address…") { store.forgetServer() }
            }
        }
        .frame(maxWidth: 460)
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct MainWindow: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav

    var body: some View {
        @Bindable var nav = nav
        NavigationSplitView {
            List(selection: $nav.section) {
                ForEach(MacSection.allCases) { s in
                    Label(s.title, systemImage: s.symbol).accessibilityIdentifier("section-\(s.title)")
                        .badge(s == .needsYou ? store.needsYou?.count ?? 0 : 0)
                        .tag(s)
                }
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180)
            .scrollContentBackground(.hidden)
            .background(WebTheme.canvas)
            .safeAreaInset(edge: .bottom) {
                Label {
                    Text(store.connection.label).font(.system(size: 10.5, design: .monospaced)).foregroundStyle(WebTheme.fg3)
                } icon: {
                    Image(systemName: store.connection.isUsable ? "circle.fill" : "circle.dotted")
                        .foregroundStyle(store.connection.color)
                }
                    .font(.caption)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .accessibilityIdentifier("connectionLabel")
            }
        } detail: {
            switch nav.section {
            case .overview: OverviewSection()
            case .agents: AgentsSection()
            case .issues: IssuesSection()
            case .needsYou: NeedsYouScreen()
            case .settings: SettingsSection()
            }
        }
        // The web's page, cards and text colours behind every section.
        .webPage()
        .sheet(isPresented: $nav.picksRecipient) { RecipientPicker() }
    }
}

/// ⌘N: pick an agent and its chat opens.
struct RecipientPicker: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""

    private var agents: [Agent] {
        store.sortedAgents.filter { search.isEmpty || $0.name.localizedCaseInsensitiveContains(search) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("New message").font(.headline)
            TextField("Search agents", text: $search)
                .textFieldStyle(.roundedBorder)
                .accessibilityIdentifier("recipientSearch")
                .onSubmit { if let a = agents.first { pick(a) } }
            List(agents) { a in
                Button { pick(a) } label: { AgentRow(agent: a).contentShape(Rectangle()) }
                    .buttonStyle(.plain)
            }
            .frame(height: 280)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding()
        .frame(width: 360)
    }

    private func pick(_ a: Agent) {
        nav.openChat(a.id)
        dismiss()
    }
}

#if DEBUG
/// `-render-window <folder>`: draws the main window to PNGs (the address step, or a chat, Issues, Overview, Needs you and
/// Settings), then quits. For screenshots when a UI test can't drive the app (another copy of it is open, and XCUITest
/// picks that one). `-appearance light|dark|system` picks the look for this launch only, without saving it.
@MainActor
func renderWindow(store: LoftStore, nav: MacNav, to folder: String) async {
    for _ in 0..<80 where store.agents.isEmpty { try? await Task.sleep(for: .milliseconds(100)) }
    try? await Task.sleep(for: .seconds(1))
    func png(_ name: String) {
        guard let window = NSApp.windows.first(where: { $0.isVisible && $0.frame.width > 600 }),
              let view = window.contentView?.superview ?? window.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
    }
    if store.agents.isEmpty {
        png("connect")
    } else {
        if let id = store.agents.first(where: { $0.name == "Orchestrator" })?.id { nav.openChat(id) }
        try? await Task.sleep(for: .seconds(2))
        png("agents")
        for section in [MacSection.issues, .overview, .needsYou, .settings] {
            nav.section = section
            try? await Task.sleep(for: .seconds(2))
            png("\(section)")
        }
    }
    NSApp.terminate(nil)
}
#endif
