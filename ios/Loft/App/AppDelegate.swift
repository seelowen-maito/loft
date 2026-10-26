import UIKit
import UserNotifications

/// APNs token callbacks and taps on notifications (push or local), which open the item they're about.
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    /// Set by the app once its router exists; a tap that arrives before that is kept until then.
    @MainActor static var open: ((PushDestination) -> Void)? {
        didSet { if let d = pendingOpen, let open { pendingOpen = nil; open(d) } }
    }
    @MainActor private static var pendingOpen: PushDestination?

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { @MainActor in PushRegistrar.shared.didReceive(token: deviceToken) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        Task { @MainActor in PushRegistrar.shared.didFail(error) }
    }

    /// In the foreground the app already shows "Needs you": no banner, just the list and badge. A push about an
    /// item a local notification already showed isn't shown again.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        let info = notification.request.content.userInfo
        if let id = PushDestination.itemId(info) {
            let delivered = await center.deliveredNotifications()
            if delivered.contains(where: { $0.request.identifier == id }) { return [] }
        }
        return [.list, .badge]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        guard let d = PushDestination.from(info) else { return }
        await MainActor.run {
            if let open = Self.open { open(d) } else { Self.pendingOpen = d }
        }
    }
}
