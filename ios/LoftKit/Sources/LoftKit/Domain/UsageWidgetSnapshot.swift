import Foundation

/// What the Usage widget shows (#489): the Claude plan's session and week, and how the Loft Mac is doing (memory and
/// CPU temperature), from `GET /api/usage` and `GET /api/system`. A value Loft doesn't have stays nil and shows a dash.
public struct UsageWidgetSnapshot: Codable, Equatable {
    public enum Phase: String, Codable {
        /// The app hasn't connected to a Loft yet.
        case notConnected
        /// Loft didn't answer: the widget shows the last reading (if any) with its time.
        case unreachable
        case ready
    }

    /// One plan window: percent used and when it resets.
    public struct Meter: Codable, Equatable {
        public var percent: Double
        public var resetsAt: Date?
    }

    public var phase: Phase
    public var session: Meter?
    public var week: Meter?
    public var memUsedBytes: UInt64?
    public var memTotalBytes: UInt64?
    /// nominal | high | critical
    public var memLevel: String?
    public var cpuTempC: Double?
    /// nominal | warm | hot
    public var tempLevel: String?
    /// What the agents' runs cost this Usage week, in US dollars.
    public var weekSpend: Double?
    /// When the reading was taken; nil without one.
    public var takenAt: Date?

    public init(phase: Phase) { self.phase = phase }

    public init(usage: PlanUsage?, system: SystemReading?, weekSpend: Double?, at now: Date) {
        phase = .ready
        session = usage?.session.map { Meter(percent: $0.percent, resetsAt: $0.resetsAt) }
        week = usage?.week.map { Meter(percent: $0.percent, resetsAt: $0.resetsAt) }
        memUsedBytes = system?.memUsedBytes
        memTotalBytes = system?.memTotalBytes
        memLevel = system?.memLevel
        cpuTempC = system?.cpuTempC
        tempLevel = system?.level
        self.weekSpend = weekSpend
        takenAt = now
    }

    /// "42%", or a dash when Loft has no reading (not logged in, an older Loft).
    public static func percentText(_ m: Meter?) -> String { m.map { "\(Int($0.percent.rounded()))%" } ?? "—" }

    /// "Resets in 2h 14m", "Resets in 3d 23h", like the app's usage card; "Reset time unknown" without one.
    public static func resetText(_ at: Date?, now: Date) -> String {
        guard let at else { return "Reset time unknown" }
        let s = Int(at.timeIntervalSince(now))
        guard s > 59 else { return "Resets in under a minute" }
        let d = s / 86_400, h = (s % 86_400) / 3600, m = (s % 3600) / 60
        return "Resets in " + (d > 0 ? "\(d)d \(h)h" : h > 0 ? "\(h)h \(m)m" : "\(m)m")
    }

    /// "21 of 32 GB", or a dash.
    public var memoryText: String {
        guard let used = memUsedBytes, let total = memTotalBytes, total > 0 else { return "—" }
        let gb = { (b: UInt64) in Double(b) / Double(1 << 30) }
        return "\(Int(gb(used).rounded())) of \(Int(gb(total).rounded())) GB"
    }

    /// "64°", or a dash where the Mac has no sensor Loft can read.
    public var tempText: String { cpuTempC.map { "\(Int($0.rounded()))°" } ?? "—" }

    /// "$12.40", or nil when Loft didn't say.
    public var spendText: String? { weekSpend.map { $0.formatted(.currency(code: "USD").locale(Locale(identifier: "en_US"))) } }

    /// Reads Loft at the address the app connected to.
    public static func load() async -> UsageWidgetSnapshot {
        guard let server = SharedServer.address else { return UsageWidgetSnapshot(phase: .notConnected) }
        let api = LiveLoftAPI(baseURL: server)
        return await load(usage: api.usage, system: api.system, weekSpend: api.weekSpend, last: .standard)
    }

    static let lastKey = "usageWidget.last"

    /// Usage and the Mac's reading are each optional (no login, an older Loft without `/system`): Loft is unreachable
    /// only when both fail. Then the widget shows `last`'s saved reading, with its time.
    static func load(usage: () async throws -> PlanUsage, system: () async throws -> SystemReading, weekSpend: () async throws -> Double,
                     last: UserDefaults? = nil, now: Date = .now) async -> UsageWidgetSnapshot {
        async let u = try? usage()
        async let sys = try? system()
        async let spend = try? weekSpend()
        let (plan, mac) = (await u, await sys)
        guard plan != nil || mac != nil else {
            var s = last?.data(forKey: lastKey).flatMap { try? JSONDecoder().decode(Self.self, from: $0) } ?? UsageWidgetSnapshot(phase: .ready)
            s.phase = .unreachable
            return s
        }
        // A plan Loft can't read (its `error`, like "not logged in") is a dash, not 0%.
        let readable = plan?.error == nil ? plan : nil
        let s = UsageWidgetSnapshot(usage: readable, system: mac, weekSpend: await spend, at: now)
        if let data = try? JSONEncoder().encode(s) { last?.set(data, forKey: lastKey) }
        return s
    }
}
