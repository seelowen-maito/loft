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
    /// The bundle ids a running session hid, so the next launch shows them again after a crash or force quit.
    static let hiddenKey = "focusHidden"

    private(set) var session: FocusSession?
    /// The last chosen length, remembered across launches.
    private(set) var minutes: Int
    var allowList: FocusAllowList {
        didSet { UserDefaults.standard.set(allowList.saved, forKey: Self.allowListKey) }
    }
    /// The length popover is up, under the sidebar's "Start focus".
    var picksLength = false
    /// How many apps this session hid, under the countdown.
    private(set) var hiddenCount = 0

    let loftBundleId: String
    /// Off for screenshots: a session drawn for a PR mustn't hide the apps on this Mac.
    private let hidesApps: Bool
    /// The apps this session hid, to show again at the end.
    @ObservationIgnored private var hider = FocusHider<NSRunningApplication>() {
        didSet {
            UserDefaults.standard.set(hider.bundleIds, forKey: Self.hiddenKey)
            if hiddenCount != hider.hidden.count { hiddenCount = hider.hidden.count }
        }
    }
    private var observers: [NSObjectProtocol] = []
    private var ticker: Task<Void, Never>?
    private let dockTile = FocusDockTile()

    init(hidesApps: Bool = true) {
        self.hidesApps = hidesApps
        loftBundleId = Bundle.main.bundleIdentifier ?? "loft"
        minutes = FocusDuration.remembered(UserDefaults.standard.integer(forKey: Self.minutesKey))
        allowList = FocusAllowList.load(UserDefaults.standard.data(forKey: Self.allowListKey), loftBundleId: loftBundleId)
        // `willTerminate` doesn't run after a crash or force quit: what that session hid comes back now.
        if hidesApps, let saved = UserDefaults.standard.stringArray(forKey: Self.hiddenKey), !saved.isEmpty {
            FocusHider.unhide(saved: saved, in: NSWorkspace.shared.runningApplications)
            UserDefaults.standard.removeObject(forKey: Self.hiddenKey)
        }
    }

    func start(minutes: Int) {
        guard session == nil else { return }
        // A "Focus for…" that hasn't opened its popover yet mustn't open it after this session.
        picksLength = false
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
                self.dockTile.show(session.dockFace(at: .now))
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    /// Ends the session, early (End, quitting) or when time is up, and brings the hidden apps back.
    func end(timeUp: Bool = false) {
        guard let session else { return }
        self.session = nil
        picksLength = false
        ticker?.cancel()
        ticker = nil
        observers.forEach(NSWorkspace.shared.notificationCenter.removeObserver)
        observers = []
        hider.unhideAll()
        dockTile.show(nil)
        if !TestMode.noAlerts { notifyEnded(session, timeUp: timeUp) }
    }

    /// A newly allowed app comes back at once.
    func allow(_ app: FocusApp) {
        allowList.add(app)
        hider.unhide(bundleId: app.bundleId)
    }

    /// An app taken off the list mid-session is hidden at once.
    func disallow(_ bundleId: String) {
        allowList.remove(bundleId)
        guard hidesApps else { return }
        for app in NSRunningApplication.runningApplications(withBundleIdentifier: bundleId) { hideIfNeeded(app) }
    }

    private func hideIfNeeded(_ app: NSRunningApplication) {
        guard session != nil else { return }
        hider.hide(app, unless: allowList, loftBundleId: loftBundleId)
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

extension NSRunningApplication: @retroactive FocusRunningApp {
    public var isRegularApp: Bool { activationPolicy == .regular }
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

/// The Dock icon: the app icon in a ring that fills over the session, with the time left under it. The badge stays the
/// Needs you count. It redraws only when the label or the ring's whole percent changes.
@MainActor
final class FocusDockTile {
    private var face: FocusDockFace?

    func show(_ face: FocusDockFace?) {
        guard face != self.face else { return }
        self.face = face
        let tile = NSApp.dockTile
        tile.contentView = face.map { face in
            let view = NSHostingView(rootView: FocusDockIcon(timeLeft: face.label, progress: Double(face.percent) / 100))
            view.frame = NSRect(origin: .zero, size: tile.size)
            return view
        }
        tile.display()
    }
}

struct FocusDockIcon: View {
    let timeLeft: String
    let progress: Double

    var body: some View {
        ZStack(alignment: .bottom) {
            Image(nsImage: NSApp.applicationIconImage).resizable().scaledToFit().padding(10)
                .overlay { FocusRing(progress: progress, lineWidth: 4, track: WebTheme.good.opacity(0.25)).padding(4) }
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
            .keyboardShortcut("f", modifiers: [.command, .shift, .option])
        }
    }
}

/// Under the sidebar: Start focus (with the length popover), or the ring, the time left, the hidden count and ×.
struct FocusSidebarControl: View {
    @Environment(FocusController.self) private var focus
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The popover is up. The menus ask through `focus.picksLength`, which can come before this control is on screen
    /// (the window just opened, or the sidebar was collapsed), so it opens a moment later.
    @State private var picks = false

    var body: some View {
        if let session = focus.session {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                HStack(spacing: 10) {
                    HStack(spacing: 10) {
                        FocusRing(progress: session.progress(at: context.date), lineWidth: 3.5)
                            .frame(width: 34, height: 34)
                            .animation(reduceMotion ? nil : .linear(duration: 1), value: session.progress(at: context.date))
                        VStack(alignment: .leading, spacing: 1) {
                            Text(session.countdown(at: context.date)).font(.body.monospacedDigit().weight(.semibold)).foregroundStyle(WebTheme.fg)
                            Text(FocusSession.hiddenLabel(focus.hiddenCount)).font(.caption).foregroundStyle(WebTheme.fg3)
                        }
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Focus, \(session.spokenLeft(at: context.date)), \(FocusSession.hiddenLabel(focus.hiddenCount).lowercased())")
                    .accessibilityIdentifier("focusCountdown")
                    Spacer(minLength: 0)
                    Button { focus.end() } label: {
                        Image(systemName: "xmark").font(.system(size: 9, weight: .bold)).foregroundStyle(WebTheme.fg2)
                            .frame(width: 20, height: 20).background(Circle().fill(WebTheme.raised))
                    }
                    .buttonStyle(.plain)
                    .help("End focus")
                    .accessibilityLabel("End focus")
                    .accessibilityIdentifier("endFocus")
                }
            }
        } else {
            Button { picks = true } label: {
                Label("Start focus", systemImage: "timer").frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
            .foregroundStyle(WebTheme.fg2)
            .help("Hides every app that isn't on the allow list (Settings) for a while")
            .accessibilityIdentifier("startFocus")
            .popover(isPresented: $picks, arrowEdge: .trailing) { FocusLengthPopover() }
            .onChange(of: focus.picksLength, initial: true) {
                guard focus.picksLength else { return }
                Task {
                    try? await Task.sleep(for: .milliseconds(400))
                    // Not if a session started or ended meanwhile (they clear `picksLength`).
                    if focus.picksLength && focus.session == nil { picks = true }
                }
            }
            .onChange(of: picks) { if !picks { focus.picksLength = false } }
        }
    }
}

/// The ring that fills over a session, from 12 o'clock: under the sidebar, on the Dock tile and in the menu bar item.
/// VoiceOver skips it: the countdown next to it says the same.
struct FocusRing: View {
    let progress: Double
    let lineWidth: CGFloat
    var track: Color = WebTheme.lineStrong

    var body: some View {
        ZStack {
            Circle().stroke(track, lineWidth: lineWidth)
            Circle().trim(from: 0, to: progress)
                .stroke(WebTheme.good, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .padding(lineWidth / 2)
        .accessibilityHidden(true)
    }
}

/// Picks the length: a click on 25, 50 or 90 starts at once; Return starts the highlighted row (the last length at
/// first), ↑/↓ move, Esc closes. Custom takes 1 to 480 minutes.
struct FocusLengthPopover: View {
    @Environment(FocusController.self) private var focus
    @Environment(\.dismiss) private var dismiss
    /// The keyboard's highlight, which Return starts. The mouse only tints the row under it (`hovered`).
    @State private var choice = FocusLengthChoice(lastMinutes: FocusDuration.defaultMinutes)
    @State private var hovered: Int?
    @State private var custom = ""
    @FocusState private var focused: Field?

    private enum Field { case list, custom }
    private var customRow: Int { FocusDuration.presets.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(FocusDuration.presets.enumerated()), id: \.offset) { i, m in
                Button { start(m) } label: { line(i, isLast: m == focus.minutes) { Text("\(m) min") } }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(choice.row == i ? .isSelected : [])
                    .accessibilityIdentifier("length-\(m)")
            }
            Divider().padding(.vertical, 4)
            line(customRow, isLast: !FocusDuration.presets.contains(focus.minutes)) {
                HStack(spacing: 6) {
                    Text("Custom")
                    TextField("", text: $custom, prompt: Text("1–480"))
                        .textFieldStyle(.roundedBorder).frame(width: 56).multilineTextAlignment(.trailing)
                        .focused($focused, equals: .custom)
                        .onSubmit(startHighlighted)
                        // The field takes ↑/↓ before the popover does: they move the highlight from here too.
                        .onKeyPress(.upArrow) { move(-1) }
                        .onKeyPress(.downArrow) { move(1) }
                        .accessibilityLabel("Custom length in minutes")
                        .accessibilityIdentifier("customLength")
                    Text("min")
                    Spacer(minLength: 0)
                    Button("Start", action: startHighlighted).controlSize(.small).disabled(choice.isCustom && choice.minutes(custom: custom) == nil)
                        .accessibilityIdentifier("startFocusSession")
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityAddTraits(choice.isCustom ? .isSelected : [])
            .onTapGesture { select(customRow) }
            .onChange(of: focused) { if focused == .custom && !choice.isCustom { choice.select(customRow) } }
            Text("Hides apps not on your allow list.").font(.caption).foregroundStyle(WebTheme.fg3).padding([.horizontal, .top], 8)
        }
        .padding(8)
        .frame(width: 310)
        .focusable()
        .focusEffectDisabled()
        .focused($focused, equals: .list)
        .onKeyPress(.upArrow) { move(-1) }
        .onKeyPress(.downArrow) { move(1) }
        .onKeyPress(.return) {
            startHighlighted()
            return .handled
        }
        .onKeyPress(.escape) { dismiss(); return .handled }
        .onAppear {
            choice = FocusLengthChoice(lastMinutes: focus.minutes)
            custom = choice.customText
            focused = choice.isCustom ? .custom : .list
        }
    }

    /// A row: the highlighted one is tinted `good`, with "last used ↩" (or "↩") as the hint for Return; the one under
    /// the pointer gets a faint tint.
    private func line<Content: View>(_ i: Int, isLast: Bool, @ViewBuilder content: () -> Content) -> some View {
        let selected = choice.row == i
        return HStack {
            content().font(.body.monospacedDigit()).foregroundStyle(WebTheme.fg)
            Spacer(minLength: 8)
            if selected || isLast {
                Text(isLast ? (selected ? "last used ↩" : "last used") : "↩").font(.caption).foregroundStyle(selected ? WebTheme.good : WebTheme.fg3)
                    .accessibilityHidden(true)
            }
        }
        .padding(.horizontal, 8).frame(height: 30)
        .contentShape(Rectangle())
        .background(RoundedRectangle(cornerRadius: 6).fill(selected ? WebTheme.good.opacity(0.16) : hovered == i ? WebTheme.raised : .clear))
        .onHover { hovered = $0 ? i : (hovered == i ? nil : hovered) }
    }

    private func move(_ by: Int) -> KeyPress.Result {
        select(choice.row + by)
        return .handled
    }

    private func select(_ row: Int) {
        choice.select(row)
        focused = choice.isCustom ? .custom : .list
    }

    private func startHighlighted() {
        guard let m = choice.minutes(custom: custom) else { return }
        start(m)
    }

    private func start(_ minutes: Int) {
        focus.start(minutes: minutes)
        dismiss()
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
                    Button { focus.disallow(app.bundleId) } label: { Image(systemName: "minus.circle") }
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
