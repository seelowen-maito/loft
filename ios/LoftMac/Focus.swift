import AppKit
import LoftKit
import Observation
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

/// Focus mode (#609): for a chosen length, every app that isn't on the allow list is hidden (and hidden again when it
/// comes forward), the sidebar shows only Agents and Issues, and the Dock icon and the sidebar show the time left. When
/// it ends, the hidden apps come back. Started from the Dock icon's menu, the menu bar item, ⇧⌘F or the sidebar.
@MainActor
@Observable
final class FocusController {
    static let minutesKey = "focusMinutes"
    static let allowListKey = "focusAllowList"

    private(set) var session: FocusSession?
    /// The last chosen length, remembered across launches.
    private(set) var minutes: Int
    var allowList: FocusAllowList {
        didSet { UserDefaults.standard.set(allowList.saved, forKey: Self.allowListKey) }
    }
    /// The custom length sheet is up.
    var picksLength = false

    let loftBundleId: String
    /// Off for screenshots: a session drawn for a PR mustn't hide the apps on this Mac.
    private let hidesApps: Bool
    /// The apps this session hid, to show again at the end (never one the user had hidden before).
    private var hidden: [NSRunningApplication] = []
    private var observers: [NSObjectProtocol] = []
    private var ticker: Task<Void, Never>?
    private let dockTile = FocusDockTile()

    init(hidesApps: Bool = true) {
        self.hidesApps = hidesApps
        loftBundleId = Bundle.main.bundleIdentifier ?? "loft"
        minutes = FocusDuration.remembered(UserDefaults.standard.integer(forKey: Self.minutesKey))
        allowList = FocusAllowList.load(UserDefaults.standard.data(forKey: Self.allowListKey), loftBundleId: loftBundleId)
    }

    func start(minutes: Int) {
        guard session == nil else { return }
        self.minutes = FocusDuration.clamp(minutes)
        UserDefaults.standard.set(self.minutes, forKey: Self.minutesKey)
        session = FocusSession(minutes: self.minutes, from: .now)
        if hidesApps {
            for app in NSWorkspace.shared.runningApplications { hideIfNeeded(app) }
            let center = NSWorkspace.shared.notificationCenter
            for name in [NSWorkspace.didActivateApplicationNotification, NSWorkspace.didLaunchApplicationNotification] {
                observers.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] note in
                    guard let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication else { return }
                    MainActor.assumeIsolated { self?.hideIfNeeded(app) }
                })
            }
        }
        if !TestMode.noAlerts { Task { _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) } }
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, let session = self.session else { return }
                if session.isOver(at: .now) { self.end(timeUp: true); return }
                self.dockTile.show(session.dockLabel(at: .now))
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    /// Ends the session, early (End, quitting) or when time is up, and brings the hidden apps back.
    func end(timeUp: Bool = false) {
        guard let session else { return }
        self.session = nil
        ticker?.cancel()
        ticker = nil
        observers.forEach(NSWorkspace.shared.notificationCenter.removeObserver)
        observers = []
        for app in hidden where !app.isTerminated { app.unhide() }
        hidden = []
        dockTile.show(nil)
        if !TestMode.noAlerts { notifyEnded(session, timeUp: timeUp) }
    }

    /// A newly allowed app comes back at once.
    func allow(_ app: FocusApp) {
        allowList.add(app)
        for running in hidden where running.bundleIdentifier == app.bundleId { running.unhide() }
        hidden.removeAll { $0.bundleIdentifier == app.bundleId }
    }

    private func hideIfNeeded(_ app: NSRunningApplication) {
        guard session != nil, !app.isHidden,
              allowList.hides(bundleId: app.bundleIdentifier, isRegularApp: app.activationPolicy == .regular, loftBundleId: loftBundleId)
        else { return }
        // hide() returns false even when it works (seen from the sandboxed, signed app on macOS 27), so every app
        // asked is shown again at the end.
        _ = app.hide()
        if !hidden.contains(app) { hidden.append(app) }
    }

    private func notifyEnded(_ session: FocusSession, timeUp: Bool) {
        let content = UNMutableNotificationContent()
        content.title = "Focus session ended"
        let length = Int(session.endsAt.timeIntervalSince(session.startedAt) / 60)
        content.body = timeUp ? "Your \(length)-minute session is over. Your apps are back." : "You ended the session early. Your apps are back."
        content.sound = .default
        Task { try? await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "focus-ended", content: content, trigger: nil)) }
    }

    /// The Dock icon's menu: the lengths, or End.
    /// - showWindow: brings the main window up, for Custom….
    func dockMenu(showWindow: @escaping () -> Void) -> NSMenu {
        let menu = NSMenu()
        if session != nil {
            menu.addItem(ClosureMenuItem("End Focus") { [weak self] in self?.end() })
        } else {
            for m in FocusDuration.presets {
                menu.addItem(ClosureMenuItem("Focus for \(m) min") { [weak self] in self?.start(minutes: m) })
            }
            menu.addItem(ClosureMenuItem("Focus for…") { [weak self] in
                showWindow()
                self?.picksLength = true
            })
        }
        return menu
    }
}

/// A menu item that runs a closure: the Dock menu is AppKit's.
private final class ClosureMenuItem: NSMenuItem {
    private let run: () -> Void

    init(_ title: String, run: @escaping () -> Void) {
        self.run = run
        super.init(title: title, action: #selector(fire), keyEquivalent: "")
        target = self
    }

    required init(coder: NSCoder) { fatalError("not used") }

    @objc private func fire() { run() }
}

/// The Dock icon: the app icon, with the time left under it while a session runs. The badge stays the Needs you count.
@MainActor
final class FocusDockTile {
    private var label: String?

    func show(_ label: String?) {
        guard label != self.label else { return }
        self.label = label
        let tile = NSApp.dockTile
        tile.contentView = label.map { text in
            let view = NSHostingView(rootView: FocusDockIcon(timeLeft: text))
            view.frame = NSRect(origin: .zero, size: tile.size)
            return view
        }
        tile.display()
    }
}

struct FocusDockIcon: View {
    let timeLeft: String

    var body: some View {
        ZStack(alignment: .bottom) {
            Image(nsImage: NSApp.applicationIconImage).resizable().scaledToFit()
            Text(timeLeft)
                .font(.system(size: 26, weight: .bold, design: .rounded).monospacedDigit())
                .foregroundStyle(.black)
                .padding(.horizontal, 12).padding(.vertical, 2)
                .background(Capsule().fill(WebTheme.good))
                .padding(.bottom, 6)
        }
    }
}

/// The Focus menu's items (the menu bar's Focus menu, with ⇧⌘F, and the menu bar item's popover).
struct FocusMenuItems: View {
    let focus: FocusController
    /// Brings the main window up, for Custom….
    let showWindow: () -> Void

    var body: some View {
        if focus.session != nil {
            Button("End Focus") { focus.end() }
                .keyboardShortcut("f", modifiers: [.command, .shift])
        } else {
            Button("Start Focus (\(focus.minutes) min)") { focus.start(minutes: focus.minutes) }
                .keyboardShortcut("f", modifiers: [.command, .shift])
            ForEach(FocusDuration.presets.filter { $0 != focus.minutes }, id: \.self) { m in
                Button("Focus for \(m) min") { focus.start(minutes: m) }
            }
            Button("Focus for…") {
                showWindow()
                focus.picksLength = true
            }
        }
    }
}

/// Under the sidebar: Start focus, or the time left and End.
struct FocusSidebarControl: View {
    @Environment(FocusController.self) private var focus

    var body: some View {
        if let session = focus.session {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                HStack(spacing: 8) {
                    Image(systemName: "timer").foregroundStyle(WebTheme.good).accessibilityHidden(true)
                    Text(session.countdown(at: context.date)).font(.body.monospacedDigit().weight(.semibold)).foregroundStyle(WebTheme.fg)
                        .accessibilityLabel("Focus, \(session.countdown(at: context.date)) left")
                        .accessibilityIdentifier("focusCountdown")
                    Spacer()
                    Button("End") { focus.end() }.buttonStyle(.plain).foregroundStyle(WebTheme.fg2)
                        .accessibilityIdentifier("endFocus")
                }
            }
        } else {
            Button { focus.picksLength = true } label: {
                Label("Start focus", systemImage: "timer").frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
            .foregroundStyle(WebTheme.fg2)
            .help("Hides every app that isn't on the allow list (Settings) for a while")
            .accessibilityIdentifier("startFocus")
        }
    }
}

/// Picks the length and starts: 25, 50 or 90 minutes, or a custom length.
struct FocusLengthSheet: View {
    @Environment(FocusController.self) private var focus
    @Environment(\.dismiss) private var dismiss
    @State private var minutes = FocusDuration.defaultMinutes

    var body: some View {
        VStack(spacing: 18) {
            Text("Focus").font(.title2.bold())
            Text("\(minutes) min").font(.system(size: 48, weight: .bold, design: .rounded).monospacedDigit())
                .accessibilityIdentifier("focusLength")
            HStack(spacing: 8) {
                ForEach(FocusDuration.presets, id: \.self) { m in
                    Button { minutes = m } label: {
                        Text("\(m) min").font(.callout.weight(.semibold)).padding(.horizontal, 14).padding(.vertical, 7)
                            .foregroundStyle(minutes == m ? Color.black : WebTheme.fg)
                            .background(Capsule().fill(minutes == m ? WebTheme.good : WebTheme.raised))
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(minutes == m ? .isSelected : [])
                    .accessibilityIdentifier("length-\(m)")
                }
            }
            Stepper("Custom length", value: $minutes, in: FocusDuration.range, step: 5).fixedSize()
                .accessibilityIdentifier("customLength")
            Text("Apps that aren't on your allow list (Settings) are hidden until the time is up. Only Agents and Issues stay in the sidebar.")
                .font(.callout).foregroundStyle(WebTheme.fg2).multilineTextAlignment(.center)
            HStack {
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Start focus") {
                    focus.start(minutes: minutes)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .accessibilityIdentifier("startFocusSession")
            }
        }
        .padding(24)
        .frame(width: 400)
        .onAppear { minutes = focus.minutes }
    }
}

/// Settings › Focus: the apps a session leaves on screen.
struct FocusSettings: View {
    @Environment(FocusController.self) private var focus

    private var runningApps: [FocusApp] {
        NSWorkspace.shared.runningApplications
            .filter { $0.activationPolicy == .regular }
            .compactMap { app in app.bundleIdentifier.map { FocusApp(bundleId: $0, name: app.localizedName ?? $0) } }
            .filter { !focus.allowList.allows($0.bundleId) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    var body: some View {
        Section("Focus") {
            ForEach(focus.allowList.apps) { app in
                HStack {
                    AppIcon(bundleId: app.bundleId).frame(width: 20, height: 20)
                    Text(app.name)
                    Spacer()
                    Button { focus.allowList.remove(app.bundleId) } label: { Image(systemName: "minus.circle") }
                        .buttonStyle(.borderless)
                        .help("Remove \(app.name)")
                        .accessibilityLabel("Remove \(app.name)")
                        .disabled(app.bundleId == focus.loftBundleId)
                }
                .accessibilityIdentifier("allowed-\(app.bundleId)")
            }
            Menu("Add App") {
                ForEach(runningApps) { app in Button(app.name) { focus.allow(app) } }
                if !runningApps.isEmpty { Divider() }
                Button("Choose…") { choose() }
            }
            .fixedSize()
            .accessibilityIdentifier("addAllowedApp")
            Text("These apps stay on screen during a focus session (⇧⌘F, the Dock icon's menu or the menu bar). Every other app is hidden until it ends.")
                .font(.footnote).foregroundStyle(.secondary)
        }
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.application]
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        guard panel.runModal() == .OK, let url = panel.url, let id = Bundle(url: url)?.bundleIdentifier else { return }
        let name = FileManager.default.displayName(atPath: url.path).replacingOccurrences(of: ".app", with: "")
        focus.allow(FocusApp(bundleId: id, name: name))
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
