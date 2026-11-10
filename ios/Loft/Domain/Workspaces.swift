import Foundation

/// `GET /api/workspaces`: each agent's workspace size, the Mac's free disk and what the last sweep freed.
struct WorkspaceSizes: Codable, Hashable {
    struct Row: Codable, Hashable, Identifiable {
        var id: String
        var name: String
        var bytes: Int64
    }

    var agents: [Row]
    /// Nil when the Mac couldn't say.
    var freeBytes: Int64?
    var lowDiskBytes: Int64

    /// Under the server's limit (20 GB), where Needs you says "Disk almost full".
    var isLow: Bool { freeBytes.map { $0 < lowDiskBytes } ?? false }
}

/// `POST /api/workspaces/clean`: what Clean now removed.
struct SweepReport: Codable, Hashable {
    var freedBytes: Int64
}
