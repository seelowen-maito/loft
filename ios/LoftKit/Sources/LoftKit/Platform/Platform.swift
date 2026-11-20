#if os(iOS)
import UIKit
#else
import AppKit
#endif
import UserNotifications

/// The few system calls the shared code makes that differ between the iPhone and the Mac.
@MainActor
public enum Platform {
    /// VoiceOver is on: speak through it instead of over it.
    public static var isVoiceOverRunning: Bool {
        #if os(iOS)
        UIAccessibility.isVoiceOverRunning
        #else
        NSWorkspace.shared.isVoiceOverEnabled
        #endif
    }

    /// Has VoiceOver read `text` out.
    public static func announce(_ text: String) {
        #if os(iOS)
        UIAccessibility.post(notification: .announcement, argument: text)
        #else
        guard let app = NSApp else { return }
        NSAccessibility.post(element: app, notification: .announcementRequested,
                             userInfo: [.announcement: text, .priority: NSAccessibilityPriorityLevel.high.rawValue])
        #endif
    }

    /// Asks APNs for a device token; it arrives in the app delegate.
    public static func registerForRemoteNotifications() {
        #if os(iOS)
        UIApplication.shared.registerForRemoteNotifications()
        #else
        NSApplication.shared.registerForRemoteNotifications()
        #endif
    }

    /// Notifications are allowed (the iPhone also has app-clip "ephemeral" permission).
    public static func notificationsAllowed(_ status: UNAuthorizationStatus) -> Bool {
        #if os(iOS)
        [.authorized, .provisional, .ephemeral].contains(status)
        #else
        [.authorized, .provisional].contains(status)
        #endif
    }
}
