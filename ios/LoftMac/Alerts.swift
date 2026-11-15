import AppKit
import LoftKit
import Observation
import UserNotifications

/// Mac notifications and the Dock badge (spec #382). Watches the store's Needs you list for as long as the app runs,
/// window open or not: new questions, permissions and ready PRs become native notifications, and the Dock badge
/// follows the Needs you count. Nothing goes through the relay. A refused permission only loses the notifications.
@MainActor
final class MacAlerts: NSObject, UNUserNotificationCenterDelegate {
    private let store: LoftStore
    private let nav: MacNav
    private var alerts = NeedsYouAlerts()
    private var asked = false
    /// Brings the main window up; set by the menu bar label, which lives as long as the app.
    var showWindow: (() -> Void)?

    init(store: LoftStore, nav: MacNav) {
        self.store = store
        self.nav = nav
        super.init()
        UNUserNotificationCenter.current().delegate = self
        watch()
    }

    private func watch() {
        let list = withObservationTracking { store.needsYou } onChange: { [weak self] in
            Task { @MainActor in self?.watch() }
        }
        NSApp?.dockTile.badgeLabel = NeedsYouAlerts.badge(for: list)
        guard let list else { return }
        let fresh = alerts.fresh(in: list)
        if !fresh.isEmpty, !NSApp.isActive { Task { await post(fresh) } }
    }

    private func post(_ items: [NeedsYouItem]) async {
        let center = UNUserNotificationCenter.current()
        if !asked {
            asked = true
            _ = try? await center.requestAuthorization(options: [.alert, .sound])
        }
        for item in items {
            let content = UNMutableNotificationContent()
            content.title = item.agent.map { "\($0.name) needs you" } ?? "Needs you"
            // Titles only, never message content (like the iPhone's).
            content.body = String(item.title.prefix(120))
            content.sound = .default
            var info: [String: Any] = ["needs_you_id": item.id]
            if let target = NeedsYouAlerts.target(of: item) { info["target"] = target }
            content.userInfo = info
            try? await center.add(UNNotificationRequest(identifier: item.id, content: content, trigger: nil))
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        await MainActor.run {
            switch PushDestination.from(info, loftHost: store.api?.baseURL.host()) {
            case .chat(let id): nav.openChat(id); show()
            case .url(let url): NSWorkspace.shared.open(url)
            default: nav.section = .needsYou; show()
            }
        }
    }

    private func show() {
        showWindow?()
        NSApp.activate(ignoringOtherApps: true)
    }
}
