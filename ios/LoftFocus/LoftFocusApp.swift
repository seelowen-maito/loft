import LoftKit
import SwiftUI

/// Loft Focus (#609): a second Mac app on the main one's code, in the Dock, with only the agents, the tickets and a
/// focus timer. The main app's views come from the LoftMac folder, which this target compiles too (`LOFT_FOCUS`
/// leaves out its `@main`).
@main
struct LoftFocusApp: App {
    @State private var store: LoftStore
    @State private var nav: MacNav
    @State private var alerts: MacAlerts
    @State private var voice: VoiceSession
    @State private var focus: FocusController

    init() {
        Prefs.registerDefaults()
        let store = LoftStore(api: Self.makeAPI())
        store.activate()
        _store = State(initialValue: store)
        let nav = MacNav()
        _nav = State(initialValue: nav)
        // Needs you and done notifications keep coming through during a session.
        _alerts = State(initialValue: MacAlerts(store: store, nav: nav))
        _voice = State(initialValue: VoiceSession(store: store))
        let focus = FocusController(hidesApps: !Self.rendersScreenshots)
        _focus = State(initialValue: focus)
        #if DEBUG
        // Draws its own windows: a directly launched app may not open one.
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-render-focus"), i + 1 < args.count {
            let voice = _voice.wrappedValue
            Task { await renderFocus(store: store, nav: nav, voice: voice, focus: focus, to: args[i + 1]) }
        }
        #endif
    }

    /// The main app's address, from this app's own setting, else the one the main app shared with the widgets.
    private static func makeAPI() -> LoftAPI? {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-ui-testing") {
            SnapshotStore.standard.clear()
            return FixtureAPI.fromLaunchArguments()
        }
        #endif
        let saved = UserDefaults.standard.string(forKey: Prefs.serverURL).flatMap(LiveLoftAPI.normalize) ?? SharedServer.address
        return saved.map(LiveLoftAPI.init(baseURL:))
    }

    private static var rendersScreenshots: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-render-focus")
        #else
        false
        #endif
    }

    var body: some Scene {
        Window("Loft Focus", id: "focus") {
            FocusRootView()
                .tint(Theme.accent)
                .environment(store)
                .environment(nav)
                .environment(voice)
                .environment(focus)
                .frame(minWidth: 980, minHeight: 600)
                .onAppear { store.activate() }
                .modifier(ShowsWindowFromAlerts(alerts: alerts))
                // Quitting mid-session brings the hidden apps back.
                .onReceive(NotificationCenter.default.publisher(for: NSApplication.willTerminateNotification)) { _ in
                    focus.end()
                    store.saveNow()
                }
        }
        .defaultSize(width: 1200, height: 760)

        Settings {
            TabView {
                AllowListSettings()
                    .padding()
                    .tabItem { Label("Allow List", systemImage: "checklist") }
                AddressForm(isOnboarding: false)
                    .padding()
                    .tabItem { Label("Loft", systemImage: "network") }
            }
            .tint(Theme.accent)
            .environment(store)
            .environment(focus)
            .frame(width: 520)
        }
    }
}

/// A click on a notification brings the window back, even if it was closed.
private struct ShowsWindowFromAlerts: ViewModifier {
    let alerts: MacAlerts
    @Environment(\.openWindow) private var openWindow

    func body(content: Content) -> some View {
        content.onAppear { alerts.showWindow = { openWindow(id: "focus") } }
    }
}

#if DEBUG
/// `-render-focus <folder>`: draws the window idle and during a session (which hides nothing here), the allow list
/// settings and the Dock icon to PNGs, then quits. `-appearance light|dark` picks the look.
@MainActor
func renderFocus(store: LoftStore, nav: MacNav, voice: VoiceSession, focus: FocusController, to folder: String) async {
    for _ in 0..<80 where store.agents.isEmpty { try? await Task.sleep(for: .milliseconds(100)) }
    Appearance.apply()
    let dir = URL(fileURLWithPath: folder)
    func shoot<V: View>(_ name: String, _ content: V, size: NSSize, wait: Double) async {
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled, .fullSizeContentView], backing: .buffered, defer: false)
        window.title = "Loft Focus"
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: content.tint(Theme.accent).environment(store).environment(nav)
            .environment(voice).environment(focus))
        window.orderFront(nil)
        try? await Task.sleep(for: .seconds(wait))
        if let view = window.contentView?.superview ?? window.contentView, let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
            view.cacheDisplay(in: view.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: dir.appendingPathComponent("\(name).png"))
        }
        window.close()
    }
    let size = NSSize(width: 1200, height: 760)
    await shoot("idle", FocusRootView(), size: size, wait: 2)
    focus.minutes = 50
    focus.start()
    await shoot("session", FocusRootView(), size: size, wait: 4)
    // The other two panes, with the countdown under the sidebar.
    if let id = store.agents.first(where: { $0.name == "Orchestrator" })?.id ?? store.agents.first?.id {
        await shoot("agents", FocusRootView().task { nav.openChat(id) }, size: size, wait: 3)
    }
    await shoot("issues", FocusRootView().task { nav.section = .issues }, size: size, wait: 3)
    await shoot("settings", AllowListSettings().padding().frame(width: 520), size: NSSize(width: 520, height: 420), wait: 1)
    let icon = ImageRenderer(content: FocusDockIcon(timeLeft: "50m").frame(width: 128, height: 128))
    icon.scale = 2
    if let tiff = icon.nsImage?.tiffRepresentation, let data = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) {
        try? data.write(to: dir.appendingPathComponent("dock.png"))
    }
    focus.end()
    NSApp.terminate(nil)
}
#endif
