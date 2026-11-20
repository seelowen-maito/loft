import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

/// The one way features use Apple's on-device model (Foundation Models, iOS 26+ with Apple Intelligence on).
/// It reports whether the model can be used, runs a job with a time limit, and falls back to today's behaviour
/// whenever the model is missing, slow or wrong. Features never call `LanguageModelSession` directly.
public enum OnDeviceModel {
    public enum Status: Equatable {
        case available
        case appleIntelligenceOff
        case deviceNotEligible
        case modelNotReady
        /// The model doesn't speak the iPhone's language; the name of that language.
        case languageNotSupported(String)
        /// A reason this app doesn't know by name (a newer iOS); the system's own text for it.
        case unavailable(String)
        /// Older than iOS 26.
        case needsNewerSystem

        public var isAvailable: Bool { self == .available }

        /// For Settings › Apple Intelligence.
        public var text: String {
            switch self {
            case .available: "Available"
            case .appleIntelligenceOff: "Turned off in \(Platform.settingsApp) › Apple Intelligence & Siri"
            case .deviceNotEligible: "This \(Platform.deviceName) doesn't support it"
            case .modelNotReady: "Getting ready (downloading)"
            case .languageNotSupported(let language): "Not available in \(language)"
            case .unavailable(let reason): "Unavailable (\(reason))"
            case .needsNewerSystem: "Needs \(Platform.systemName) 26 or later"
            }
        }

        /// Where to look when it stays like this; nil when the status says it already.
        public var hint: String? {
            switch self {
            case .modelNotReady:
                "\(Platform.systemName) downloads the model itself. Check \(Platform.settingsApp) › Apple Intelligence & Siri; this turns on by itself once it's ready."
            case .languageNotSupported:
                "Check the language in \(Platform.settingsApp) › Apple Intelligence & Siri."
            default: nil
            }
        }
    }

    private enum Platform {
        #if os(macOS)
        static let settingsApp = "System Settings", deviceName = "Mac", systemName = "macOS"
        #else
        static let settingsApp = "Settings", deviceName = "iPhone", systemName = "iOS"
        #endif
    }

    /// Read fresh on every call, so a feature turns on as soon as the model is ready, without a relaunch.
    public static var status: Status {
        #if DEBUG
        // `-fixture-ai-status off|ineligible|notReady|old|language|unknown|available`: show a state the Simulator can't produce.
        switch FixtureAPI.launchText("fixture-ai-status") {
        case "off": return .appleIntelligenceOff
        case "ineligible": return .deviceNotEligible
        case "notReady": return .modelNotReady
        case "old": return .needsNewerSystem
        case "language": return .languageNotSupported("Icelandic")
        case "unknown": return .unavailable("someNewReason")
        case "available": return .available
        default: break
        }
        #endif
        if TestMode.noAI { return .appleIntelligenceOff }
        guard let s = system else { return .needsNewerSystem }
        logAvailability(s.raw)
        return status(unavailableReason: s.reason, languageSupported: s.languageSupported)
    }

    /// The system's own answer, unmapped: "unavailable(modelNotReady), en_US supported: true". For the log and
    /// Settings' help text, so a status that looks wrong can be checked on the device.
    public static var rawAvailability: String {
        #if DEBUG
        if let f = FixtureAPI.launchText("fixture-ai-status") { return "fixture \(f)" }
        #endif
        if TestMode.noAI { return "-no-ai" }
        return system?.raw ?? "no FoundationModels"
    }

    private static var system: (reason: String?, languageSupported: Bool, raw: String)? {
        #if canImport(FoundationModels)
        if #available(iOS 26.0, macOS 26.0, *) {
            let model = SystemLanguageModel.default
            var reason: String?
            if case .unavailable(let r) = model.availability { reason = "\(r)" }
            let languageSupported = model.supportsLocale()
            return (reason, languageSupported,
                    "\(reason.map { "unavailable(\($0))" } ?? "available"), \(Locale.current.identifier) supported: \(languageSupported)")
        }
        #endif
        return nil
    }

    /// What Settings shows for the system's unavailable reason (nil when the model is available). A reason
    /// without a name here is shown as the system wrote it, never as a download.
    public static func status(unavailableReason reason: String?, languageSupported: Bool, language: String = currentLanguage) -> Status {
        switch reason {
        case nil: return .available
        case "appleIntelligenceNotEnabled": return .appleIntelligenceOff
        case "deviceNotEligible": return .deviceNotEligible
        // Checked before "not ready": the model never downloads for a language it doesn't speak.
        case _ where !languageSupported: return .languageNotSupported(language)
        case "modelNotReady": return .modelNotReady
        case let other?: return .unavailable(other)
        }
    }

    public static var currentLanguage: String {
        Locale.current.localizedString(forLanguageCode: Locale.current.language.languageCode?.identifier ?? "") ?? Locale.current.identifier
    }

    @MainActor private static var lastAvailability: String?

    /// Logs the raw availability when it differs from the last check, so the log shows each change without
    /// a line for every voice command.
    private static func logAvailability(_ line: String) {
        Task { @MainActor in
            guard line != lastAvailability else { return }
            lastAvailability = line
            VoiceLog.shared.add("ai: availability \(line)")
        }
    }

    /// What a job produced, and where it came from (for the log and for tests).
    public enum Outcome<T> {
        case model(T, seconds: Double)
        case fallback(T, reason: String)

        public var value: T {
            switch self {
            case .model(let v, _), .fallback(let v, _): v
            }
        }
        public var usedModel: Bool { if case .model = self { true } else { false } }
    }

    public struct TimedOut: Error {}

    /// Runs `job` on the model within `timeout`, or returns `fallback()` when the model isn't available, takes
    /// too long or throws. The model call is cancelled on timeout. Every run is logged with how it went.
    public static func run<T: Sendable>(_ name: String, timeout: Duration = .seconds(2), available: Bool = status.isAvailable,
                                 fallback: () -> T, job: @escaping @Sendable () async throws -> T) async -> Outcome<T> {
        guard available else { return log(name, .fallback(fallback(), reason: "model \(status.text.lowercased())")) }
        let start = ContinuousClock.now
        do {
            let value = try await withThrowingTaskGroup(of: T.self) { group in
                group.addTask { try await job() }
                group.addTask {
                    try await Task.sleep(for: timeout)
                    throw TimedOut()
                }
                defer { group.cancelAll() }
                guard let first = try await group.next() else { throw TimedOut() }
                return first
            }
            let d = (ContinuousClock.now - start).components
            return log(name, .model(value, seconds: Double(d.seconds) + Double(d.attoseconds) / 1e18))
        } catch is TimedOut {
            return log(name, .fallback(fallback(), reason: "took longer than \(timeout)"))
        } catch {
            return log(name, .fallback(fallback(), reason: error.localizedDescription))
        }
    }

    @discardableResult
    private static func log<T>(_ name: String, _ o: Outcome<T>) -> Outcome<T> {
        let line: String
        switch o {
        case .model(_, let s): line = "ai: \(name) by the on-device model in \(String(format: "%.1f", s)) s"
        case .fallback(_, let why): line = "ai: \(name) fell back (\(why))"
        }
        Task { @MainActor in VoiceLog.shared.add(line) }
        return o
    }
}
