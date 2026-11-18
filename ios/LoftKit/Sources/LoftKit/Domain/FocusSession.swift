import Foundation

/// A focus session in the Mac app (#609): it runs for a chosen length, and while it runs every app off the allow list is hidden.
public struct FocusSession: Equatable, Sendable {
    public let startedAt: Date
    public let endsAt: Date

    public init(minutes: Int, from now: Date) {
        startedAt = now
        endsAt = now.addingTimeInterval(TimeInterval(FocusDuration.clamp(minutes) * 60))
    }

    /// The window's sections while a session runs: only Agents and Issues.
    public static let sections: [MacSection] = [.agents, .issues]

    /// Whole seconds left, never below 0.
    public func secondsLeft(at now: Date) -> Int {
        max(0, Int(endsAt.timeIntervalSince(now).rounded(.up)))
    }

    public func isOver(at now: Date) -> Bool { secondsLeft(at: now) == 0 }

    /// How far through the session is, 0 to 1, for the window's ring.
    public func progress(at now: Date) -> Double {
        let total = endsAt.timeIntervalSince(startedAt)
        guard total > 0 else { return 1 }
        return min(1, max(0, now.timeIntervalSince(startedAt) / total))
    }

    /// The window's countdown: "24:59", "1:29:59".
    public func countdown(at now: Date) -> String {
        let s = secondsLeft(at: now), h = s / 3600, m = (s % 3600) / 60, sec = s % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, sec) : String(format: "%d:%02d", m, sec)
    }

    /// The Dock icon's short label, in whole minutes up ("25m", "1m"), then seconds in the last minute ("42s").
    public func dockLabel(at now: Date) -> String {
        let s = secondsLeft(at: now)
        return s >= 60 ? "\((s + 59) / 60)m" : "\(s)s"
    }
}

/// The lengths a session can be.
public enum FocusDuration {
    public static let presets = [25, 50, 90]
    public static let defaultMinutes = 25
    /// A custom length: 1 minute to 8 hours.
    public static let range = 1...480

    public static func clamp(_ minutes: Int) -> Int { min(range.upperBound, max(range.lowerBound, minutes)) }

    /// The last chosen length, from the saved value; nothing saved yet (0) gives the default.
    public static func remembered(_ saved: Int) -> Int { saved == 0 ? defaultMinutes : clamp(saved) }
}

/// An app on the allow list.
public struct FocusApp: Codable, Hashable, Identifiable, Sendable {
    public let bundleId: String
    public let name: String
    public var id: String { bundleId }

    public init(bundleId: String, name: String) {
        self.bundleId = bundleId
        self.name = name
    }
}

/// The apps a session leaves on screen. The user edits it in the app's settings.
public struct FocusAllowList: Equatable, Sendable {
    public private(set) var apps: [FocusApp]

    public init(apps: [FocusApp]) { self.apps = apps }

    /// What a new install starts with: Loft and the coding tools.
    /// - loftBundleId: the Mac app's bundle id, which depends on the build's `LOFT_BUNDLE_ID`.
    public static func starting(loftBundleId: String) -> FocusAllowList {
        FocusAllowList(apps: [
            FocusApp(bundleId: loftBundleId, name: "Loft"),
            FocusApp(bundleId: "com.apple.dt.Xcode", name: "Xcode"),
            FocusApp(bundleId: "com.apple.Terminal", name: "Terminal"),
            FocusApp(bundleId: "com.googlecode.iterm2", name: "iTerm"),
            FocusApp(bundleId: "com.microsoft.VSCode", name: "Visual Studio Code"),
            FocusApp(bundleId: "com.todesktop.230313mzl4w4u92", name: "Cursor"),
            FocusApp(bundleId: "com.apple.iphonesimulator", name: "Simulator"),
        ])
    }

    public func allows(_ bundleId: String) -> Bool { apps.contains { $0.bundleId == bundleId } }

    /// Whether a running app is hidden while a session runs: a regular (Dock) app that isn't on the list. Loft itself
    /// always stays, even if it was taken off the list, and an app without a bundle id is left alone.
    public func hides(bundleId: String?, isRegularApp: Bool, loftBundleId: String) -> Bool {
        guard isRegularApp, let bundleId, bundleId != loftBundleId else { return false }
        return !allows(bundleId)
    }

    /// Adds an app at the end; one already on the list isn't added twice.
    public mutating func add(_ app: FocusApp) {
        if !allows(app.bundleId) { apps.append(app) }
    }

    public mutating func remove(_ bundleId: String) {
        apps.removeAll { $0.bundleId == bundleId }
    }

    /// Saved as JSON in the app's settings.
    public var saved: Data { (try? JSONEncoder().encode(apps)) ?? Data() }

    /// The saved list, or the starting one when nothing (or nothing readable) is saved.
    public static func load(_ data: Data?, loftBundleId: String) -> FocusAllowList {
        if let data, let apps = try? JSONDecoder().decode([FocusApp].self, from: data) { return FocusAllowList(apps: apps) }
        return starting(loftBundleId: loftBundleId)
    }
}
