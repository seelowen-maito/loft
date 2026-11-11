import AVFoundation
import Foundation

/// The one owner of the audio session: the only place that sets its category and activates or deactivates it.
/// The speaker and the microphone ask it for the audio; one of them holds it at a time. Asking while another
/// holds it hands the audio over: the one holding it is told to let go first. An interruption (a call,
/// another app) ends whoever holds it, and the next request starts afresh.
@MainActor
final class AudioSessionOwner {
    static let shared = AudioSessionOwner(SystemAudioSession())

    /// What the audio is for.
    enum Use: String {
        /// Reading aloud.
        case speaking
        /// The microphone: the composer's dictation and the Talk screen.
        case listening
        /// Reading aloud while listening for a spoken command (echo cancellation needs both on one engine).
        case readingWhileListening

        var needsMic: Bool { self != .speaking }
    }

    /// Why a holder lost the audio.
    enum Loss { case handedOver, interrupted }

    private struct Holder {
        let id: UUID
        let use: Use
        let name: String
        let onLost: @MainActor (Loss) -> Void
    }

    private let session: AudioSessionAPI
    private let attempts: Int
    private let retryWait: Duration
    /// How long the session stays active after the last holder lets go, so a quick hand-over (read, then
    /// listen again) doesn't deactivate and reactivate it, which renegotiates a Bluetooth route.
    private let linger: Duration
    private var holder: Holder?
    private var deactivation: Task<Void, Never>?
    private var interruption: NSObjectProtocol?

    init(_ session: AudioSessionAPI, attempts: Int = 5, retryWait: Duration = .milliseconds(200), linger: Duration = .seconds(1)) {
        self.session = session
        self.attempts = attempts
        self.retryWait = retryWait
        self.linger = linger
    }

    /// Who holds the audio now, for tests and the log.
    var heldFor: Use? { holder?.use }

    /// Takes the audio for `use` and returns the lease to `release`. Whoever holds it is told to let go first
    /// (`onLost(.handedOver)`). Retries for about a second while the audio is refused (the mic still letting go,
    /// another app holding it), then throws. `onLost` is called if the audio is taken away later.
    func acquire(_ use: Use, by name: String, onLost: @escaping @MainActor (Loss) -> Void) async throws -> UUID {
        observeInterruptions()
        deactivation?.cancel()
        deactivation = nil
        if let old = holder {
            VoiceLog.shared.add("audio: handing over from \(old.name) (\(old.use.rawValue)) to \(name) (\(use.rawValue))")
            holder = nil
            old.onLost(.handedOver)
        }
        let id = UUID()
        holder = Holder(id: id, use: use, name: name, onLost: onLost)
        var last = "unknown"
        for attempt in 0..<attempts {
            // Another request took over while this one waited.
            guard holder?.id == id else { throw VoiceError("The audio went to another request.") }
            do {
                try configure(for: use)
                try session.setActive(true)
                if use.needsMic && !session.isInputAvailable {
                    holder = nil
                    VoiceLog.shared.add("audio: refused \(name): no microphone on \(session.route)")
                    scheduleDeactivation()
                    throw VoiceError("No microphone is available.")
                }
                VoiceLog.shared.add("audio: \(name) holds it (\(use.rawValue), \(session.category.rawValue), \(session.route))"
                    + (attempt > 0 ? " after \(attempt) retries" : ""))
                return id
            } catch let e as VoiceError {
                throw e
            } catch {
                last = error.localizedDescription
                VoiceLog.shared.add("audio: refused for \(name) (\(last))"
                    + (session.isOtherAudioPlaying ? ", another app is playing" : "") + (attempt + 1 < attempts ? "; retrying" : ""))
                if attempt + 1 < attempts { try? await Task.sleep(for: retryWait) }
            }
        }
        if holder?.id == id { holder = nil }
        throw VoiceError("The audio wasn't available (\(last)).")
    }

    /// Lets go of the audio. Does nothing when `id` no longer holds it (it was handed over or interrupted).
    func release(_ id: UUID) {
        guard let h = holder, h.id == id else { return }
        holder = nil
        VoiceLog.shared.add("audio: \(h.name) let go")
        scheduleDeactivation()
    }

    /// The system's interruption (a call, Siri, another app): whoever holds the audio ends; nothing resumes by
    /// itself, and the next request activates the session again.
    func interrupted(began: Bool) {
        guard began else { return VoiceLog.shared.add("audio: interruption ended") }
        deactivation?.cancel()
        deactivation = nil
        guard let h = holder else { return VoiceLog.shared.add("audio: interrupted while nobody held it") }
        holder = nil
        VoiceLog.shared.add("audio: interrupted; \(h.name) (\(h.use.rawValue)) ends")
        h.onLost(.interrupted)
    }

    private func configure(for use: Use) throws {
        switch use {
        case .speaking:
            // Keep recording possible when it's already set up: a voice session listens again right after,
            // and switching category renegotiates a Bluetooth route.
            if session.category != .playAndRecord {
                try session.setCategory(.playback, mode: .spokenAudio, options: [.duckOthers])
            }
        case .listening:
            try session.setCategory(.playAndRecord, mode: .spokenAudio, options: [.defaultToSpeaker, .allowBluetoothHFP, .duckOthers])
        case .readingWhileListening:
            try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker, .allowBluetoothHFP, .duckOthers])
        }
    }

    private func scheduleDeactivation() {
        deactivation?.cancel()
        deactivation = Task { [weak self, linger] in
            try? await Task.sleep(for: linger)
            guard let self, !Task.isCancelled, self.holder == nil else { return }
            self.deactivation = nil
            do {
                try self.session.setActive(false)
                VoiceLog.shared.add("audio: session off")
            } catch {
                VoiceLog.shared.add("audio: couldn't turn the session off (\(error.localizedDescription))")
            }
        }
    }

    private func observeInterruptions() {
        guard interruption == nil, session is SystemAudioSession else { return }
        interruption = NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] n in
            let began = (n.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt).flatMap(AVAudioSession.InterruptionType.init) == .began
            MainActor.assumeIsolated { self?.interrupted(began: began) }
        }
    }
}

/// The parts of `AVAudioSession` the owner uses, so tests can swap in a fake.
@MainActor
protocol AudioSessionAPI: AnyObject {
    var category: AVAudioSession.Category { get }
    var isInputAvailable: Bool { get }
    var isOtherAudioPlaying: Bool { get }
    /// The current route, for the log.
    var route: String { get }
    func setCategory(_ category: AVAudioSession.Category, mode: AVAudioSession.Mode, options: AVAudioSession.CategoryOptions) throws
    func setActive(_ active: Bool) throws
}

@MainActor
final class SystemAudioSession: AudioSessionAPI {
    private var s: AVAudioSession { .sharedInstance() }
    var category: AVAudioSession.Category { s.category }
    var isInputAvailable: Bool { s.isInputAvailable }
    var isOtherAudioPlaying: Bool { s.isOtherAudioPlaying }
    var route: String {
        let r = s.currentRoute
        let ins = r.inputs.map(\.portType.rawValue).joined(separator: "+")
        let outs = r.outputs.map(\.portType.rawValue).joined(separator: "+")
        return "in \(ins.isEmpty ? "none" : ins), out \(outs.isEmpty ? "none" : outs)"
    }
    func setCategory(_ category: AVAudioSession.Category, mode: AVAudioSession.Mode, options: AVAudioSession.CategoryOptions) throws {
        try s.setCategory(category, mode: mode, options: options)
    }
    func setActive(_ active: Bool) throws {
        try s.setActive(active, options: .notifyOthersOnDeactivation)
    }
}
