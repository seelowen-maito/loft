import Foundation
import UserNotifications

/// Local notifications for events the app sees on its live connection.
/// They only fire while the app is running (including the short time iOS keeps it
/// alive in the background). Push while the app is closed needs APNs — see ios/README.md.
final class Notifier {
    enum Kind {
        case approval, taskOutcome, failure

        var prefKey: String {
            switch self {
            case .approval: Prefs.notifyApprovals
            case .taskOutcome: Prefs.notifyTaskOutcomes
            case .failure: Prefs.notifyFailures
            }
        }
    }

    func requestAuthorization() async -> Bool {
        (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])) ?? false
    }

    func authorizationStatus() async -> UNAuthorizationStatus {
        await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }

    /// Posts only when the app isn't in front (the UI already shows it there) and the user wants this kind.
    /// Bodies carry names and titles only, never tool input or message content.
    /// `itemId`: the item's "Needs you" id, so a push about the same item and this one aren't both shown;
    /// `target` is where a tap leads (same shape as a push's).
    func post(_ kind: Kind, title: String, body: String, whenActive active: Bool, itemId: String? = nil, target: [String: String]? = nil) {
        guard !active, UserDefaults.standard.bool(forKey: kind.prefKey) else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = String(body.prefix(120))
        content.sound = .default
        var info: [String: Any] = [:]
        if let itemId { info["needs_you_id"] = itemId }
        if let target { info["target"] = target }
        content.userInfo = info
        Task {
            let center = UNUserNotificationCenter.current()
            if let itemId, Self.alreadyShown(itemId, in: await center.deliveredNotifications().map { ($0.request.identifier, $0.request.content.userInfo) }) {
                return
            }
            try? await center.add(UNNotificationRequest(identifier: itemId ?? UUID().uuidString, content: content, trigger: nil))
        }
    }

    /// Whether a delivered notification (push or local) is already about `itemId`.
    static func alreadyShown(_ itemId: String, in delivered: [(id: String, info: [AnyHashable: Any])]) -> Bool {
        delivered.contains { $0.id == itemId || PushDestination.itemId($0.info) == itemId }
    }
}
