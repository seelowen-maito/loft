import Foundation

/// `GET /api/workspaces`: each agent's workspace size, the Mac's free disk and what the last sweep freed.
struct WorkspaceSizes: Codable, Hashable {
    struct Row: Codable, Hashable, Identifiable {
        var id: String
        var name: String
        var bytes: Int64
        /// Running or with a live dev server, so Clean skips it. Nil from a Mac without per-agent Clean.
        var busy: Bool?
    }

    var agents: [Row]
    /// Nil when the Mac couldn't say.
    var freeBytes: Int64?
    var lowDiskBytes: Int64

    /// Under the server's limit (20 GB), where Needs you says "Disk almost full".
    var isLow: Bool { freeBytes.map { $0 < lowDiskBytes } ?? false }
    /// How many agents Clean all cleans: those not busy.
    var idleCount: Int { agents.filter { $0.busy != true }.count }
}

/// `POST /api/workspaces/clean`: what Clean or Clean all removed.
struct SweepReport: Codable, Hashable {
    var freedBytes: Int64
}
