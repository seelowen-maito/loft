import AppKit
import LoftKit
import Observation
import SwiftUI
import UserNotifications

/// Runs Loft Focus sessions (#609): hides every app that isn't on the allow list, keeps hiding the ones that come
/// forward, shows the time left on the Dock icon, and shows the hidden apps again when the session ends.
@MainActor
@Observable
final class FocusController {
    static let minutesKey = "focusMinutes"
    static let allowListKey = "focusAllowList"

    private(set) var session: FocusSession?
    /// The chosen length, remembered for the next launch.
    var minutes: Int {
        didSet { UserDefaults.standard.set(minutes, forKey: Self.minutesKey) }
    }
    var allowList: FocusAllowList {
        didSet { UserDefaults.standard.set(allowList.saved, forKey: Self.allowListKey) }
    }

    let focusBundleId: String
    /// Off for screenshots: a session drawn for the PR mustn't hide the apps on this Mac.
    private let hidesApps: Bool
    /// The apps this session hid, to show again at the end (never one the user had hidden before).
    private var hidden: [NSRunningApplication] = []
    private var observers: [NSObjectProtocol] = []
    private var ticker: Task<Void, Never>?
    private let dockTile = FocusDockTile()

    init(hidesApps: Bool = true) {
        self.hidesApps = hidesApps
        focusBundleId = Bundle.main.bundleIdentifier ?? "loft.focus"
        // Loft Focus is `<Loft's bundle id>.focus`.
        let loftBundleId = focusBundleId.hasSuffix(".focus") ? String(focusBundleId.dropLast(".focus".count)) : focusBundleId
        minutes = FocusDuration.remembered(UserDefaults.standard.integer(forKey: Self.minutesKey))
        allowList = FocusAllowList.load(UserDefaults.standard.data(forKey: Self.allowListKey),
                                        loftBundleId: loftBundleId, focusBundleId: focusBundleId)
    }

    func start() {
        guard session == nil else { return }
        let session = FocusSession(minutes: minutes, from: .now)
        self.session = session
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
              allowList.hides(bundleId: app.bundleIdentifier, isRegularApp: app.activationPolicy == .regular, focusBundleId: focusBundleId)
        else { return }
        if app.hide(), !hidden.contains(app) { hidden.append(app) }
    }

    private func notifyEnded(_ session: FocusSession, timeUp: Bool) {
        let content = UNMutableNotificationContent()
        content.title = "Focus session ended"
        let length = Int(session.endsAt.timeIntervalSince(session.startedAt) / 60)
        content.body = timeUp ? "Your \(length)-minute session is over. Your apps are back." : "You ended the session early. Your apps are back."
        content.sound = .default
        Task { try? await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "focus-ended", content: content, trigger: nil)) }
    }
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
