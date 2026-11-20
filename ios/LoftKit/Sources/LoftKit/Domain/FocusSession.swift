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

    /// VoiceOver's time left, in whole minutes up, then seconds in the last minute: "15 minutes left", "42 seconds left".
    public func spokenLeft(at now: Date) -> String {
        let s = secondsLeft(at: now)
        let (n, unit) = s >= 60 ? ((s + 59) / 60, "minute") : (s, "second")
        return "\(n) \(unit)\(n == 1 ? "" : "s") left"
    }

    /// What the Dock tile draws now. The tile redraws only when this changes: the label, or a whole percent of the ring.
    public func dockFace(at now: Date) -> FocusDockFace {
        FocusDockFace(label: dockLabel(at: now), percent: Int(progress(at: now) * 100))
    }

    /// Under the countdown: "No apps hidden", "1 app hidden", "5 apps hidden".
    public static func hiddenLabel(_ count: Int) -> String {
        switch count {
        case 0: "No apps hidden"
        case 1: "1 app hidden"
        default: "\(count) apps hidden"
        }
    }
}

/// The Dock tile during a session: the minutes capsule and the ring's whole percent.
public struct FocusDockFace: Equatable, Sendable {
    public let label: String
    public let percent: Int

    public init(label: String, percent: Int) {
        self.label = label
        self.percent = percent
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

extension FocusSession {
    /// The section the window shows: while a session runs, one it hides (quick search, Spotlight, a widget) shows Agents.
    public static func shown(_ section: MacSection, focusing: Bool) -> MacSection {
        focusing && !sections.contains(section) ? .agents : section
    }
}

/// A running app as a focus session sees it: `NSRunningApplication` on the Mac, a fake in the tests.
public protocol FocusRunningApp: AnyObject {
    var bundleIdentifier: String? { get }
    var isRegularApp: Bool { get }
    var isHidden: Bool { get }
    var isTerminated: Bool { get }
    func hide() -> Bool
    func unhide() -> Bool
}

/// The apps a session hid, to show again when it ends (never one the user had hidden before).
public struct FocusHider<App: FocusRunningApp> {
    public private(set) var hidden: [App] = []

    public init() {}

    /// The hidden apps' bundle ids, saved so a launch after a crash can show them again.
    public var bundleIds: [String] { hidden.compactMap(\.bundleIdentifier) }

    /// Hides the app when it's off the allow list and not hidden already.
    public mutating func hide(_ app: App, unless allowList: FocusAllowList, loftBundleId: String) {
        guard !app.isHidden,
              allowList.hides(bundleId: app.bundleIdentifier, isRegularApp: app.isRegularApp, loftBundleId: loftBundleId)
        else { return }
        // hide() returns false even when it works (seen from the sandboxed, signed app on macOS 27), so every app
        // asked is shown again at the end.
        _ = app.hide()
        if !hidden.contains(where: { $0 === app }) { hidden.append(app) }
    }

    /// A newly allowed app comes back at once.
    public mutating func unhide(bundleId: String) {
        for app in hidden where app.bundleIdentifier == bundleId && !app.isTerminated { _ = app.unhide() }
        hidden.removeAll { $0.bundleIdentifier == bundleId }
    }

    /// The session ended: every app it hid comes back.
    public mutating func unhideAll() {
        for app in hidden where !app.isTerminated { _ = app.unhide() }
        hidden = []
    }

    /// The first launch after a crash or force quit mid-session: the apps that session hid come back.
    public static func unhide(saved bundleIds: [String], in running: [App]) {
        for app in running where app.isHidden && app.bundleIdentifier.map(bundleIds.contains) == true { _ = app.unhide() }
    }
}
