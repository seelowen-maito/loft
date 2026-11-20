import Foundation
import UserNotifications

/// Remote notifications: asks for permission, registers with APNs, sends the device token to the Mac
/// (`POST /api/devices`) and keeps it current, and removes it when notifications are turned off or the
/// server is forgotten. Shown in Settings › Notifications.
@MainActor
@Observable
public final class PushRegistrar {
    public static let shared = PushRegistrar()

    public private(set) var authorization: UNAuthorizationStatus = .notDetermined
    /// The APNs token, hex.
    public private(set) var token: String?
    /// The token the Mac has, for this server.
    public private(set) var registered = false
    public private(set) var lastError: String?
    public private(set) var lastTestAt: Date?

    @ObservationIgnored public weak var store: LoftStore?

    public nonisolated static var environment: String {
        #if DEBUG
        "development"
        #else
        "production"
        #endif
    }

    public func refreshAuthorization() async {
        authorization = await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }

    /// After notifications are allowed: get a token from APNs (it arrives in `didReceive(token:)`).
    public func start() async {
        #if os(macOS)
        // Loft registers iPhones only (backend devices.rs takes `platform: "ios"`); the Mac posts its own local
        // notifications (LoftMac's MacAlerts) and never registers.
        return
        #else
        await refreshAuthorization()
        guard Platform.notificationsAllowed(authorization) else {
            if registered { await unregister() }
            return
        }
        Platform.registerForRemoteNotifications()
        #endif
    }

    public func didReceive(token data: Data) {
        let t = data.map { String(format: "%02x", $0) }.joined()
        let changed = t != token
        token = t
        if changed || !registered { Task { await register() } }
    }

    public func didFail(_ error: Error) {
        lastError = "APNs: \(error.localizedDescription)"
        VoiceLog.shared.add("push: APNs registration failed (\(error.localizedDescription))")
    }

    /// Sends the token and the chosen kinds and quiet hours to the Mac. Called again when those settings change.
    public func register() async {
        guard let token, let api = store?.api, store?.info?.supports(.push) == true else { return }
        do {
            let reg = DeviceRegistration.current(token: token)
            try await api.registerDevice(reg)
            if reg.quietHours == nil { try await api.updateDevice(token: token, ["quiet_hours": .null]) }
            registered = true
            lastError = nil
        } catch {
            registered = false
            lastError = APIError.from(error).title
        }
    }

    /// Notifications turned off, or the server forgotten: the Mac stops pushing to this phone.
    public func unregister() async {
        guard let token, let api = store?.api else { registered = false; return }
        try? await api.unregisterDevice(token: token)
        registered = false
    }

    public func sendTest() async {
        guard let token, let api = store?.api else { return }
        do {
            try await api.testPush(token: token)
            lastTestAt = Date()
            lastError = nil
        } catch {
            lastError = APIError.from(error).title
        }
    }
}

/// `POST /api/devices` body.
public struct DeviceRegistration: Codable, Equatable {
    /// The server has no time zone database: `utcOffsetMinutes` is what it uses; `tz` is informational.
    public struct QuietHours: Codable, Equatable { public var start: String; public var end: String; public var tz: String; public var utcOffsetMinutes: Int }
    public var token: String
    /// Always "ios": only the iPhone registers (see `PushRegistrar.start`).
    public var platform = "ios"
    public var environment: String
    public var bundleId: String
    public var kinds: [String]
    public var quietHours: QuietHours?

    public static func current(token: String, defaults: UserDefaults = .standard) -> DeviceRegistration {
        let kinds = PushKind.allCases.filter { $0.isOn(defaults) }.flatMap(\.serverKinds)
        var quiet: QuietHours?
        if defaults.bool(forKey: Prefs.pushQuietHours) {
            quiet = QuietHours(start: defaults.string(forKey: Prefs.pushQuietStart) ?? "22:00",
                               end: defaults.string(forKey: Prefs.pushQuietEnd) ?? "07:00", tz: TimeZone.current.identifier,
                               utcOffsetMinutes: TimeZone.current.secondsFromGMT() / 60)
        }
        return DeviceRegistration(token: token, environment: PushRegistrar.environment,
                                  bundleId: Bundle.main.bundleIdentifier ?? "", kinds: kinds, quietHours: quiet)
    }

    public var json: JSONValue {
        var o: [String: JSONValue] = ["token": .string(token), "platform": .string(platform), "environment": .string(environment),
                                      "bundle_id": .string(bundleId), "kinds": .array(kinds.map(JSONValue.string))]
        // Left out when off: a registration without it keeps what the server had, so turning it off is a PATCH (`clearQuietHours`).
        if let q = quietHours {
            o["quiet_hours"] = .object(["start": .string(q.start), "end": .string(q.end), "tz": .string(q.tz),
                                        "utc_offset_minutes": .number(Double(q.utcOffsetMinutes))])
        }
        return .object(o)
    }
}
