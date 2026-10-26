import SwiftUI

@main
struct LoftApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var store: LoftStore
    @State private var router = Router()
    @State private var voice: VoiceSession
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage(Prefs.appearance) private var appearance = "dark"

    init() {
        if TestMode.noAnimations { UIView.setAnimationsEnabled(false) }
        Prefs.registerDefaults()
        let store = LoftStore(api: Self.makeAPI())
        _store = State(initialValue: store)
        _voice = State(initialValue: VoiceSession(store: store))
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-fixture-digest") {
            UserDefaults.standard.set(Date().addingTimeInterval(-2 * 3600).timeIntervalSince1970, forKey: Prefs.digestLastLooked)
        }
        #endif
        PushRegistrar.shared.store = store
    }

    private static func makeAPI() -> LoftAPI? {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-ui-onboarding") { return nil }
        if ProcessInfo.processInfo.arguments.contains("-ui-testing") {
            // Each UI test starts from nothing, not from the last test's cached state.
            SnapshotStore.standard.clear()
            return FixtureAPI.fromLaunchArguments()
        }
        #endif
        // `-serverURL <url>` overrides the saved address (handy for a development backend).
        let override = UserDefaults.standard.string(forKey: "serverURL-override")
        let raw = override ?? UserDefaults.standard.string(forKey: Prefs.serverURL)
        return raw.flatMap(LiveLoftAPI.normalize).map(LiveLoftAPI.init(baseURL:))
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                // UI tests: every SwiftUI change lands at once (scrolls, sheets, withAnimation).
                .transaction { if TestMode.noAnimations { $0.disablesAnimations = true; $0.animation = nil } }
                .environment(store)
                .environment(router)
                .environment(voice)
                .environment(voice.speaker)
                .preferredColorScheme(appearance == "light" ? .light : appearance == "dark" ? .dark : nil)
                .onAppear { AppDelegate.open = { router.open($0) } }
                // `loft://task/<id>`: from a message, a note or another app.
                .onOpenURL { url in if let id = TaskLink.id(from: url) { router.open(.task(taskId: id)) } }
                // Once the server says it can push, register this phone (and again when the server changes).
                .task(id: store.info?.supports(.push) == true) { await PushRegistrar.shared.start() }
        }
        .onChange(of: scenePhase, initial: true) { old, phase in
            switch phase {
            case .active: store.activate()
            // A voice session keeps running in the background (audio mode) and needs live updates for the reply.
            // Only on a real move to the background: the first call (old == phase) can report .background while the
            // app is still launching, and stamping "last looked" then emptied "While you were away" on every launch.
            case .background where !voice.isActive && old != phase:
                UserDefaults.standard.set(Date().timeIntervalSince1970, forKey: Prefs.digestLastLooked)
                store.deactivate()
            default: break
            }
        }
        #if DEBUG
        .onChange(of: store.agents.isEmpty, initial: true) { _, empty in
            let args = ProcessInfo.processInfo.arguments
            guard !empty, let i = args.firstIndex(of: "-fixture-voice"), i + 1 < args.count, voice.phase == .idle else { return }
            voice.applyFixture(args[i + 1], agentId: store.orchestrator?.id ?? store.agents[0].id)
        }
        #endif
    }
}

/// Cross-tab navigation: lets any screen open a chat, agent or task.
@MainActor
@Observable
final class Router {
    enum Tab: Hashable { case overview, agents, tasks, chat }

    var tab: Tab = .overview
    var overviewPath = NavigationPath()
    var agentsPath = NavigationPath()
    var tasksPath = NavigationPath()
    var chatPath = NavigationPath()
    var composer: Composer?

    enum Composer: Identifiable {
        case newTask(agentId: String?, parentId: String?)
        case editTask(HBTask)
        case pickRecipient

        var id: String {
            switch self {
            case .newTask(let a, let p): "new-\(a ?? "")-\(p ?? "")"
            case .editTask(let t): "edit-\(t.id)"
            case .pickRecipient: "recipient"
            }
        }
    }

    /// A tapped notification: straight to the item, on a fresh stack so Back goes to the tab's top.
    func open(_ d: PushDestination) {
        composer = nil
        switch d {
        case .chat(let agentId): openChat(agentId)
        case .task(let id):
            tab = .tasks
            tasksPath = NavigationPath()
            tasksPath.append(TaskRoute(id: id))
        case .needsYou:
            tab = .overview
            overviewPath = NavigationPath()
            overviewPath.append(NeedsYouRoute())
        case .url(let u):
            UIApplication.shared.open(u)
        }
    }

    func openChat(_ agentId: String, draft: String? = nil, dictate: Bool = false) {
        tab = .chat
        chatPath = NavigationPath()
        chatPath.append(ChatRoute(agentId: agentId, draft: draft, dictate: dictate))
    }
}

struct ChatRoute: Hashable {
    let agentId: String
    var draft: String?
    /// Start dictation as soon as the chat opens (the Overview's Talk button).
    var dictate = false
}

struct AgentRoute: Hashable { let id: String }
struct TaskRoute: Hashable { let id: String }
struct ActivityRoute: Hashable {}
struct SettingsRoute: Hashable {}

struct RootView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router

    var body: some View {
        @Bindable var router = router
        Group {
            if store.connection == .notConfigured {
                NavigationStack { ConnectView(isOnboarding: true) }
            } else {
                TabView(selection: $router.tab) {
                    NavigationStack(path: $router.overviewPath) {
                        OverviewView().withRoutes()
                    }
                    .tabItem { Label("Overview", systemImage: "square.grid.2x2") }
                    .badge(attentionCount)
                    .tag(Router.Tab.overview)

                    NavigationStack(path: $router.agentsPath) {
                        AgentsListView().withRoutes()
                    }
                    .tabItem { Label("Agents", systemImage: "person.3") }
                    .tag(Router.Tab.agents)

                    NavigationStack(path: $router.tasksPath) {
                        TaskListView().withRoutes()
                    }
                    .tabItem { Label("Tasks", systemImage: "checklist") }
                    .tag(Router.Tab.tasks)

                    NavigationStack(path: $router.chatPath) {
                        ConversationsView().withRoutes()
                    }
                    .tabItem { Label("Chat", systemImage: "bubble.left.and.bubble.right") }
                    .tag(Router.Tab.chat)
                }
                .readableBarEdges()
                .voicePresenter()
                .sheet(item: $router.composer) { c in
                    switch c {
                    case .newTask(let agentId, let parentId):
                        NavigationStack { TaskEditorView(agentId: agentId, parentId: parentId) }
                    case .editTask(let t):
                        NavigationStack { TaskEditorView(editing: t) }
                    case .pickRecipient:
                        NavigationStack { RecipientPicker() }
                    }
                }
            }
        }
    }

    private var attentionCount: Int {
        store.pendingApprovals().count
            + store.tasks.filter { $0.state == .failed || $0.state == .blocked }.count
            + store.agents.filter { store.state(of: $0) == .failed }.count
    }
}

extension View {
    /// Destinations every tab can push.
    func withRoutes() -> some View {
        navigationDestination(for: AgentRoute.self) { AgentDetailView(agentId: $0.id) }
            .navigationDestination(for: TaskRoute.self) { TaskDetailView(taskId: $0.id) }
            .navigationDestination(for: ChatRoute.self) { ChatView(agentId: $0.agentId, draft: $0.draft, dictate: $0.dictate) }
            .navigationDestination(for: ActivityRoute.self) { _ in ActivityListView() }
            .navigationDestination(for: SettingsRoute.self) { _ in SettingsView() }
            .navigationDestination(for: NeedsYouRoute.self) { _ in NeedsYouListView() }
    }

    /// The global create button, in every tab's navigation bar.
    func createMenu() -> some View {
        modifier(CreateMenu())
    }
}

private struct CreateMenu: ViewModifier {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router

    func body(content: Content) -> some View {
        content.toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button { router.composer = .newTask(agentId: nil, parentId: nil) } label: {
                        Label("New task", systemImage: "checklist")
                    }
                    .disabled(!store.supportsTasks)
                    if let o = store.orchestrator {
                        Button { router.openChat(o.id) } label: {
                            Label("Ask \(o.name)", systemImage: "sparkles")
                        }
                    }
                    Button { router.composer = .pickRecipient } label: {
                        Label("Message an agent", systemImage: "bubble.left")
                    }
                } label: {
                    Image(systemName: "plus.circle.fill")
                        .font(.title3)
                        .symbolRenderingMode(.hierarchical)
                }
                .accessibilityLabel("Create")
                .accessibilityIdentifier("createButton")
            }
        }
    }
}

/// Pick who to talk to.
struct RecipientPicker: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        List(store.sortedAgents) { a in
            Button {
                dismiss()
                router.openChat(a.id)
            } label: {
                AgentRow(agent: a).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .navigationTitle("Message an agent")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        .overlay {
            if store.agents.isEmpty {
                EmptyState(symbol: "person.3", title: "No agents", message: "Create agents in Loft on your Mac first.")
            }
        }
    }
}
