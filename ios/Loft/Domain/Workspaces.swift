import Foundation

/// `GET /api/workspaces`: each agent's workspace size and the Mac's free disk.
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

/// What a clean leaves, and why. `rel` is inside the agent's workspace, empty for all of it.
struct CleanSkip: Codable, Hashable {
    var agent: String
    var rel: String
    var reason: String
}

/// `GET /api/workspaces/clean/preview`: the build folders Clean (or Clean all) would remove, and what it skips.
struct CleanPlan: Codable, Hashable {
    struct Folder: Codable, Hashable, Identifiable {
        var agent: String
        var path: String
        var rel: String
        var bytes: Int64
        /// Its clone's newest change.
        var changed: Date
        var id: String { path }
    }

    var folders: [Folder]
    var skipped: [CleanSkip]
    var totalBytes: Int64
}

/// `POST /api/workspaces/clean` and `GET /api/workspaces/log`: a clean's log, live while `running`.
struct CleanLog: Codable, Hashable {
    struct Line: Codable, Hashable, Identifiable {
        var agent: String
        var path: String
        var rel: String
        var bytes: Int64
        var error: String?
        var id: String { path }
    }

    var at: Date?
    /// "Clean", "Clean all", "Daily sweep" or "Sweep before update".
    var what: String
    var running: Bool
    var lines: [Line]
    var skipped: [CleanSkip]
    var freedBytes: Int64
    var removed: Int
    var failed: Int
}

/// Groups by agent, in the order they come.
func groupedByAgent<T>(_ items: [T], _ agent: (T) -> String) -> [(agent: String, items: [T])] {
    var out: [(agent: String, items: [T])] = []
    for i in items {
        if out.last?.agent == agent(i) { out[out.count - 1].items.append(i) } else { out.append((agent(i), [i])) }
    }
    return out
}
