import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

/// The one way features use Apple's on-device model (Foundation Models, iOS 26+ with Apple Intelligence on).
/// It reports whether the model can be used, runs a job with a time limit, and falls back to today's behaviour
/// whenever the model is missing, slow or wrong. Features never call `LanguageModelSession` directly.
enum OnDeviceModel {
    enum Status: Equatable {
        case available
        case appleIntelligenceOff
        case deviceNotEligible
        case modelNotReady
        /// Older than iOS 26.
        case needsNewerSystem

        var isAvailable: Bool { self == .available }

        /// For Settings › Apple Intelligence.
        var text: String {
            switch self {
            case .available: "Available"
            case .appleIntelligenceOff: "Turned off in Settings › Apple Intelligence & Siri"
            case .deviceNotEligible: "This iPhone doesn't support it"
            case .modelNotReady: "Getting ready (downloading)"
            case .needsNewerSystem: "Needs iOS 26 or later"
            }
        }
    }

    static var status: Status {
        #if DEBUG
        // `-fixture-ai-status off|ineligible|notReady|old|available`: show a state the Simulator can't produce.
        switch FixtureAPI.launchText("fixture-ai-status") {
        case "off": return .appleIntelligenceOff
        case "ineligible": return .deviceNotEligible
        case "notReady": return .modelNotReady
        case "old": return .needsNewerSystem
        case "available": return .available
        default: break
        }
        #endif
        if TestMode.noAI { return .appleIntelligenceOff }
        #if canImport(FoundationModels)
        if #available(iOS 26.0, macOS 26.0, *) { return status(from: SystemLanguageModel.default.availability) }
        #endif
        return .needsNewerSystem
    }

    #if canImport(FoundationModels)
    @available(iOS 26.0, macOS 26.0, *)
    static func status(from a: SystemLanguageModel.Availability) -> Status {
        switch a {
        case .available: .available
        case .unavailable(.appleIntelligenceNotEnabled): .appleIntelligenceOff
        case .unavailable(.deviceNotEligible): .deviceNotEligible
        case .unavailable(.modelNotReady): .modelNotReady
        case .unavailable: .modelNotReady
        }
    }
    #endif

    /// What a job produced, and where it came from (for the log and for tests).
    enum Outcome<T> {
        case model(T, seconds: Double)
        case fallback(T, reason: String)

        var value: T {
            switch self {
            case .model(let v, _), .fallback(let v, _): v
            }
        }
        var usedModel: Bool { if case .model = self { true } else { false } }
    }

    struct TimedOut: Error {}

    /// Runs `job` on the model within `timeout`, or returns `fallback()` when the model isn't available, takes
    /// too long or throws. The model call is cancelled on timeout. Every run is logged with how it went.
    static func run<T: Sendable>(_ name: String, timeout: Duration = .seconds(2), available: Bool = status.isAvailable,
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
