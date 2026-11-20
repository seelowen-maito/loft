import AVFoundation
import Foundation

// The audio session `AudioSessionOwner` drives. The iPhone has one (`AVAudioSession`); the Mac has none, so
// there the category is only remembered and activating always succeeds.

#if os(iOS)
public typealias AudioCategory = AVAudioSession.Category
public typealias AudioMode = AVAudioSession.Mode
public typealias AudioCategoryOptions = AVAudioSession.CategoryOptions
#else
public struct AudioCategory: RawRepresentable, Hashable, Sendable {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }
    public static let soloAmbient = AudioCategory(rawValue: "AVAudioSessionCategorySoloAmbient")
    public static let playback = AudioCategory(rawValue: "AVAudioSessionCategoryPlayback")
    public static let playAndRecord = AudioCategory(rawValue: "AVAudioSessionCategoryPlayAndRecord")
}

public struct AudioMode: RawRepresentable, Hashable, Sendable {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }
    public static let `default` = AudioMode(rawValue: "AVAudioSessionModeDefault")
    public static let spokenAudio = AudioMode(rawValue: "AVAudioSessionModeSpokenAudio")
}

public struct AudioCategoryOptions: OptionSet, Sendable {
    public let rawValue: UInt
    public init(rawValue: UInt) { self.rawValue = rawValue }
    public static let duckOthers = AudioCategoryOptions(rawValue: 1 << 1)
    public static let defaultToSpeaker = AudioCategoryOptions(rawValue: 1 << 3)
    public static let allowBluetoothHFP = AudioCategoryOptions(rawValue: 1 << 2)
}
#endif

/// The parts of `AVAudioSession` the owner uses, so tests can swap in a fake.
@MainActor
public protocol AudioSessionAPI: AnyObject {
    var category: AudioCategory { get }
    var isInputAvailable: Bool { get }
    var isOtherAudioPlaying: Bool { get }
    /// The current route, for the log.
    var route: String { get }
    func setCategory(_ category: AudioCategory, mode: AudioMode, options: AudioCategoryOptions) throws
    func setActive(_ active: Bool) throws
}

#if os(iOS)
@MainActor
public final class SystemAudioSession: AudioSessionAPI {
    public init() {}
    private var s: AVAudioSession { .sharedInstance() }
    public var category: AudioCategory { s.category }
    public var isInputAvailable: Bool { s.isInputAvailable }
    public var isOtherAudioPlaying: Bool { s.isOtherAudioPlaying }
    public var route: String {
        let r = s.currentRoute
        let ins = r.inputs.map(\.portType.rawValue).joined(separator: "+")
        let outs = r.outputs.map(\.portType.rawValue).joined(separator: "+")
        return "in \(ins.isEmpty ? "none" : ins), out \(outs.isEmpty ? "none" : outs)"
    }
    public func setCategory(_ category: AudioCategory, mode: AudioMode, options: AudioCategoryOptions) throws {
        try s.setCategory(category, mode: mode, options: options)
    }
    public func setActive(_ active: Bool) throws {
        try s.setActive(active, options: .notifyOthersOnDeactivation)
    }

    /// Tells the owner about the system's interruptions and route changes.
    static func observe(interrupted: @escaping @MainActor (Bool) -> Void,
                        routeChanged: @escaping @MainActor (String) -> Void) -> [NSObjectProtocol] {
        let center = NotificationCenter.default
        return [
            center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { n in
                let began = (n.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt).flatMap(AVAudioSession.InterruptionType.init) == .began
                MainActor.assumeIsolated { interrupted(began) }
            },
            center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { n in
                let raw = n.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt ?? 0
                let reason = AVAudioSession.RouteChangeReason(rawValue: raw).map(Self.name) ?? "reason \(raw)"
                MainActor.assumeIsolated { routeChanged(reason) }
            },
        ]
    }

    nonisolated private static func name(_ reason: AVAudioSession.RouteChangeReason) -> String {
        switch reason {
        case .newDeviceAvailable: "new device"
        case .oldDeviceUnavailable: "device gone"
        case .categoryChange: "category change"
        case .override: "override"
        case .wakeFromSleep: "wake from sleep"
        case .noSuitableRouteForCategory: "no suitable route"
        case .routeConfigurationChange: "configuration change"
        default: "reason \(reason.rawValue)"
        }
    }
}
#else
@MainActor
public final class SystemAudioSession: AudioSessionAPI {
    public init() {}
    public private(set) var category: AudioCategory = .soloAmbient
    public var isInputAvailable: Bool { AVCaptureDevice.default(for: .audio) != nil }
    public var isOtherAudioPlaying: Bool { false }
    public var route: String {
        "in \(AVCaptureDevice.default(for: .audio)?.localizedName ?? "none"), out system"
    }
    public func setCategory(_ category: AudioCategory, mode: AudioMode, options: AudioCategoryOptions) throws {
        self.category = category
    }
    public func setActive(_ active: Bool) throws {}

    /// The Mac reports no interruptions; route changes come with the Mac voice ticket.
    static func observe(interrupted: @escaping @MainActor (Bool) -> Void,
                        routeChanged: @escaping @MainActor (String) -> Void) -> [NSObjectProtocol] { [] }
}
#endif
