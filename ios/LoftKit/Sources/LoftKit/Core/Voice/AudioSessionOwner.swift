import Foundation

/// The one owner of the audio session: the only place that sets its category and activates or deactivates it.
/// The speaker and the microphone ask it for the audio; one of them holds it at a time. Asking while another
/// holds it hands the audio over: the one holding it is told to let go first. An interruption (a call,
/// another app) ends whoever holds it, and the next request starts afresh. A route change (a headset, the car's
/// Bluetooth switching to call quality when the mic starts) is acted on once, after the route has settled:
/// the microphone restarts on the new route, or ends with a message when the route has no microphone.
@MainActor
public final class AudioSessionOwner {
    public static let shared = AudioSessionOwner(SystemAudioSession())

    /// What the audio is for.
    public enum Use: String {
        /// Reading aloud.
        case speaking
        /// The microphone: the composer's dictation and the Talk screen.
        case listening
        /// Reading aloud while listening for a spoken command (echo cancellation needs both on one engine).
        case readingWhileListening

        public var needsMic: Bool { self != .speaking }
    }

    /// Why a holder lost the audio.
    public enum Loss { case handedOver, interrupted, noMicrophone }

    private struct Holder {
        public let id: UUID
        public let use: Use
        public let name: String
        public let onLost: @MainActor (Loss) -> Void
        public let onRouteChange: (@MainActor () -> Void)?
        /// The route the holder last started on.
        public var route: String
    }

    private let session: AudioSessionAPI
    private let attempts: Int
    private let retryWait: Duration
    /// How long the session stays active after the last holder lets go, so a quick hand-over (read, then
    /// listen again) doesn't deactivate and reactivate it, which renegotiates a Bluetooth route.
    private let linger: Duration
    /// How long the route must stay unchanged before it's acted on. A Bluetooth route reports several changes
    /// in a row while it switches; acting on each would turn the mic off and on again and again.
    private let settle: Duration
    private var holder: Holder?
    private var deactivation: Task<Void, Never>?
    private var routeSettling: Task<Void, Never>?
    private var observers: [NSObjectProtocol] = []

    public init(_ session: AudioSessionAPI, attempts: Int = 5, retryWait: Duration = .milliseconds(200), linger: Duration = .seconds(1),
         settle: Duration = .milliseconds(500)) {
        self.session = session
        self.attempts = attempts
        self.retryWait = retryWait
        self.linger = linger
        self.settle = settle
    }

    /// Who holds the audio now, for tests and the log.
    public var heldFor: Use? { holder?.use }

    /// Takes the audio for `use` and returns the lease to `release`. Whoever holds it is told to let go first
    /// (`onLost(.handedOver)`). Retries for about a second while the audio is refused (the mic still letting go,
    /// another app holding it), then throws. `onLost` is called if the audio is taken away later, and
    /// `onRouteChange` once the route has settled on a different one while the holder listens: restart the
    /// microphone there with `Microphone.start`.
    public func acquire(_ use: Use, by name: String, onRouteChange: (@MainActor () -> Void)? = nil,
                 onLost: @escaping @MainActor (Loss) -> Void) async throws -> UUID {
        observeSystem()
        deactivation?.cancel()
        deactivation = nil
        if let old = holder {
            log("handing over from \(old.name) (\(old.use.rawValue)) to \(name) (\(use.rawValue))")
            holder = nil
            old.onLost(.handedOver)
        }
        let id = UUID()
        holder = Holder(id: id, use: use, name: name, onLost: onLost, onRouteChange: onRouteChange, route: session.route)
        var last = "unknown"
        for attempt in 0..<attempts {
            // Another request took over while this one waited.
            guard holder?.id == id else { throw VoiceError("The audio went to another request.") }
            do {
                try configure(for: use)
                try session.setActive(true)
                if use.needsMic && !session.isInputAvailable {
                    holder = nil
                    log("refused \(name): no microphone on this route")
                    scheduleDeactivation()
                    throw VoiceError("No microphone is available.")
                }
                holder?.route = session.route
                log("\(name) holds it (\(use.rawValue))" + (attempt > 0 ? " after \(attempt) retries" : ""))
                return id
            } catch let e as VoiceError {
                throw e
            } catch {
                last = error.localizedDescription
                log("refused for \(name) (\(last))"
                    + (session.isOtherAudioPlaying ? ", another app is playing" : "") + (attempt + 1 < attempts ? "; retrying" : ""))
                if attempt + 1 < attempts { try? await Task.sleep(for: retryWait) }
            }
        }
        if holder?.id == id { holder = nil }
        throw VoiceError("The audio wasn't available (\(last)).")
    }

    /// Lets go of the audio. Does nothing when `id` no longer holds it (it was handed over or interrupted).
    public func release(_ id: UUID) {
        guard let h = holder, h.id == id else { return }
        holder = nil
        log("\(h.name) let go")
        scheduleDeactivation()
    }

    /// The system's interruption (a call, Siri, another app): whoever holds the audio ends; nothing resumes by
    /// itself, and the next request activates the session again.
    public func interrupted(began: Bool) {
        guard began else { return log("interruption ended") }
        deactivation?.cancel()
        deactivation = nil
        routeSettling?.cancel()
        routeSettling = nil
        guard let h = holder else { return log("interrupted while nobody held it") }
        holder = nil
        log("interrupted; \(h.name) (\(h.use.rawValue)) ends")
        h.onLost(.interrupted)
    }

    /// The system says the route changed. Waits for it to settle, then acts once: a listener on a route
    /// without a microphone ends (`.noMicrophone`); a listener whose route is now different restarts its mic;
    /// a route that came back to where it was changes nothing.
    public func routeChanged(reason: String) {
        log("route changed (\(reason))")
        routeSettling?.cancel()
        routeSettling = Task { [weak self, settle] in
            try? await Task.sleep(for: settle)
            guard let self, !Task.isCancelled else { return }
            self.routeSettling = nil
            self.routeSettled()
        }
    }

    private func routeSettled() {
        guard let h = holder, h.use.needsMic else { return }
        let route = session.route
        if !session.isInputAvailable {
            holder = nil
            log("no microphone on \(route); \(h.name) ends")
            scheduleDeactivation()
            h.onLost(.noMicrophone)
        } else if route == h.route {
            log("route settled back on \(route); \(h.name) carries on")
        } else {
            holder?.route = route
            log("route settled on \(route); \(h.name) restarts the mic")
            h.onRouteChange?()
        }
    }

    /// Every line says the route, the session category and who holds the audio, so a voice bug can be read
    /// back from the log.
    private func log(_ text: String) {
        let category = session.category.rawValue.replacingOccurrences(of: "AVAudioSessionCategory", with: "")
        let held = holder.map { "\($0.name) (\($0.use.rawValue))" } ?? "nobody"
        VoiceLog.shared.add("audio: \(text) [route \(session.route), category \(category), held by \(held)]")
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
                self.log("session off")
            } catch {
                self.log("couldn't turn the session off (\(error.localizedDescription))")
            }
        }
    }

    private func observeSystem() {
        guard observers.isEmpty, session is SystemAudioSession else { return }
        observers = SystemAudioSession.observe(
            interrupted: { [weak self] began in self?.interrupted(began: began) },
            routeChanged: { [weak self] reason in self?.routeChanged(reason: reason) })
    }
}
