import AppKit
import LoftKit
import SwiftUI
import UniformTypeIdentifiers

/// What Loft Focus's window shows: the timer, the agents and the tickets. Everything else stays in the main Mac app.
enum FocusPane: String, CaseIterable, Identifiable {
    case focus, agents, issues

    var id: String { rawValue }
    var title: String {
        switch self {
        case .focus: "Focus"
        case .agents: "Agents"
        case .issues: "Issues"
        }
    }
    var symbol: String {
        switch self {
        case .focus: "timer"
        case .agents: "person.3"
        case .issues: "checklist"
        }
    }
}

/// First the address, like the main app's, then the window.
struct FocusRootView: View {
    @Environment(LoftStore.self) private var store
    @AppStorage(Prefs.appearance) private var appearance = AppearanceChoice.dark.rawValue

    var body: some View {
        Group {
            if store.connection == .notConfigured {
                AddressForm(isOnboarding: true)
                    .frame(maxWidth: 520)
                    .padding(32)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if store.lastSync == nil, case .reconnecting(_, let error) = store.connection {
                UnreachableScreen(error: error)
            } else {
                FocusWindow()
            }
        }
        .background(WebTheme.canvas)
        .onChange(of: appearance, initial: true) { Appearance.apply(appearance) }
    }
}

struct FocusWindow: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    @Environment(FocusController.self) private var focus
    @State private var pane = FocusPane.focus

    var body: some View {
        NavigationSplitView {
            List(selection: $pane) {
                ForEach(FocusPane.allCases) { p in
                    Label(p.title, systemImage: p.symbol).tag(p).accessibilityIdentifier("pane-\(p.title)")
                }
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180)
            .scrollContentBackground(.hidden)
            .background(WebTheme.canvas)
            .safeAreaInset(edge: .bottom) { if focus.session != nil { SidebarCountdown() } }
        } detail: {
            switch pane {
            case .focus: FocusTimerView()
            case .agents: AgentsSection()
            case .issues: IssuesSection()
            }
        }
        .webPage()
        // A notification or a link opens a chat or an issue: show where it went.
        // (Needs you, which this window doesn't have, shows as Agents.)
        .onChange(of: nav.section) { pane = nav.section == .issues ? .issues : .agents }
        .onChange(of: nav.selection.composerRequests) { pane = .agents }
        .onChange(of: nav.issuePath) { if !nav.issuePath.isEmpty { pane = .issues } }
    }
}

/// Under the sidebar while a session runs, so the countdown shows from Agents and Issues too.
struct SidebarCountdown: View {
    @Environment(FocusController.self) private var focus

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            if let session = focus.session {
                HStack {
                    Image(systemName: "timer").foregroundStyle(WebTheme.good).accessibilityHidden(true)
                    Text(session.countdown(at: context.date)).font(.body.monospacedDigit().weight(.semibold)).foregroundStyle(WebTheme.fg)
                    Spacer()
                    Button("End") { focus.end() }.buttonStyle(.plain).foregroundStyle(WebTheme.fg2)
                }
                .padding(12)
                .accessibilityElement(children: .combine)
                .accessibilityLabel("Focus, \(session.countdown(at: context.date)) left")
                .accessibilityIdentifier("sidebarCountdown")
            }
        }
    }
}

/// The Focus pane: pick a length and start, or the countdown and End.
struct FocusTimerView: View {
    @Environment(FocusController.self) private var focus
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        VStack(spacing: 28) {
            Spacer()
            if let session = focus.session { running(session) } else { idle }
            Spacer()
            allowed
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .navigationTitle("Focus")
        .toolbarBackground(WebTheme.canvas, for: .windowToolbar)
    }

    private var idle: some View {
        @Bindable var focus = focus
        let isCustom = !FocusDuration.presets.contains(focus.minutes)
        return VStack(spacing: 22) {
            Text("\(focus.minutes) min").font(.system(size: 64, weight: .bold, design: .rounded).monospacedDigit())
                .accessibilityIdentifier("focusLength")
            HStack(spacing: 8) {
                ForEach(FocusDuration.presets, id: \.self) { m in
                    lengthButton("\(m) min", selected: focus.minutes == m) { focus.minutes = m }
                        .accessibilityIdentifier("length-\(m)")
                }
                lengthButton("Custom", selected: isCustom) { if !isCustom { focus.minutes = 30 } }
                    .accessibilityIdentifier("length-custom")
            }
            if isCustom {
                Stepper(value: $focus.minutes, in: FocusDuration.range, step: 5) {
                    Text("\(focus.minutes) minutes")
                }
                .fixedSize()
                .accessibilityIdentifier("customLength")
            }
            Button { focus.start() } label: {
                Label("Start focus", systemImage: "play.fill").font(.title3.weight(.semibold)).padding(.horizontal, 18).padding(.vertical, 6)
            }
            .buttonStyle(.borderedProminent)
            .keyboardShortcut(.defaultAction)
            .accessibilityIdentifier("startFocus")
            Text("Apps that aren't on your allow list are hidden until the time is up.")
                .font(.callout).foregroundStyle(WebTheme.fg2)
        }
    }

    private func running(_ session: FocusSession) -> some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            VStack(spacing: 22) {
                ZStack {
                    Circle().stroke(WebTheme.line, lineWidth: 14)
                    Circle().trim(from: 0, to: 1 - session.progress(at: context.date))
                        .stroke(WebTheme.good, style: StrokeStyle(lineWidth: 14, lineCap: .round))
                        .rotationEffect(.degrees(-90))
                    VStack(spacing: 4) {
                        Text(session.countdown(at: context.date))
                            .font(.system(size: 56, weight: .bold, design: .rounded).monospacedDigit())
                            .accessibilityIdentifier("focusCountdown")
                        Text("until \(session.endsAt.formatted(date: .omitted, time: .shortened))").foregroundStyle(WebTheme.fg2)
                    }
                }
                .frame(width: 260, height: 260)
                .accessibilityElement(children: .combine)
                .accessibilityLabel("\(session.countdown(at: context.date)) left")
                Button("End session") { focus.end() }
                    .controlSize(.large)
                    .accessibilityIdentifier("endFocus")
            }
        }
    }

    private func lengthButton(_ title: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).font(.callout.weight(.semibold)).padding(.horizontal, 14).padding(.vertical, 7)
                .foregroundStyle(selected ? Color.black : WebTheme.fg)
                .background(Capsule().fill(selected ? WebTheme.good : WebTheme.raised))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    /// Which apps stay on screen, and the way to change that.
    private var allowed: some View {
        HStack(spacing: 6) {
            Text("Stays on screen:").foregroundStyle(WebTheme.fg3)
            ForEach(focus.allowList.apps.prefix(8)) { app in
                AppIcon(bundleId: app.bundleId).frame(width: 20, height: 20).help(app.name)
            }
            Button("Edit…") { openSettings() }.buttonStyle(.link).accessibilityIdentifier("editAllowList")
        }
        .font(.callout)
    }
}

/// An app's icon from its bundle id; a generic one when it isn't installed.
struct AppIcon: View {
    let bundleId: String

    var body: some View {
        let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId)
        Image(nsImage: url.map { NSWorkspace.shared.icon(forFile: $0.path) } ?? NSWorkspace.shared.icon(for: .applicationBundle))
            .resizable().scaledToFit()
            .accessibilityHidden(true)
    }
}

/// Settings › Allow list: the apps a session leaves on screen.
struct AllowListSettings: View {
    @Environment(FocusController.self) private var focus

    private var runningApps: [FocusApp] {
        NSWorkspace.shared.runningApplications
            .filter { $0.activationPolicy == .regular }
            .compactMap { app in app.bundleIdentifier.map { FocusApp(bundleId: $0, name: app.localizedName ?? $0) } }
            .filter { !focus.allowList.allows($0.bundleId) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("These apps stay on screen during a focus session. Every other app is hidden until it ends.")
                .foregroundStyle(.secondary)
            List {
                ForEach(focus.allowList.apps) { app in
                    HStack {
                        AppIcon(bundleId: app.bundleId).frame(width: 22, height: 22)
                        Text(app.name)
                        Spacer()
                        Button { focus.allowList.remove(app.bundleId) } label: { Image(systemName: "minus.circle") }
                            .buttonStyle(.borderless)
                            .help("Remove \(app.name)")
                            .accessibilityLabel("Remove \(app.name)")
                            .disabled(app.bundleId == focus.focusBundleId)
                    }
                    .accessibilityIdentifier("allowed-\(app.bundleId)")
                }
            }
            .frame(minHeight: 260)
            Menu("Add App") {
                ForEach(runningApps) { app in Button(app.name) { focus.allow(app) } }
                if !runningApps.isEmpty { Divider() }
                Button("Choose…") { choose() }
            }
            .fixedSize()
            .accessibilityIdentifier("addAllowedApp")
        }
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.application]
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        guard panel.runModal() == .OK, let url = panel.url, let bundle = Bundle(url: url), let id = bundle.bundleIdentifier else { return }
        let name = FileManager.default.displayName(atPath: url.path).replacingOccurrences(of: ".app", with: "")
        focus.allow(FocusApp(bundleId: id, name: name))
    }
}
