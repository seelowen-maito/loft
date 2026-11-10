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

/// One thing Clean lists: a workspace's top-level item, or a folder inside a main clone
/// (`GET /api/workspaces/clean/preview` and `.../folders`).
struct CleanItem: Codable, Hashable, Identifiable {
    var agent: String
    var path: String
    /// Inside the agent's workspace, like `loft` or `loft/backend/target`.
    var rel: String
    var name: String
    /// "clone", "worktree", "folder", "file" or "symlink".
    var kind: String
    var bytes: Int64
    /// Its newest file's change.
    var changed: Date
    /// The equivalent `rm -rf`, to read or copy; the Mac removes by path. Empty for `.git`, which never goes.
    var command: String
    /// Why it starts unticked, like "main clone of loft".
    var label: String
    /// Said under it, like what keeping a main clone costs.
    var note: String
    /// What removing it risks, like "may contain unpushed work".
    var warning: String
    var ticked: Bool
    /// Why it can't be ticked ("Working", "symlink", "git history: never deleted"); empty when it can.
    var blocked: String
    /// It opens up to its own folders.
    var opens: Bool
    var id: String { path }
    /// A main clone itself can't be ticked; the folders inside it can.
    var canTick: Bool { blocked.isEmpty && !(opens && !label.isEmpty) }
}

struct CleanListing: Codable, Hashable {
    var items: [CleanItem]
}

/// `POST /api/workspaces/clean` and `GET /api/workspaces/log`: a clean's log, live while `running`.
struct CleanLog: Codable, Hashable {
    struct Line: Codable, Hashable, Identifiable {
        var agent: String
        var path: String
        var rel: String
        var bytes: Int64
        var error: String?
        var command: String
        var id: String { path }
    }

    var at: Date?
    /// "Clean", "Clean all", "Daily sweep" or "Sweep before update".
    var what: String
    /// "Cleaning Loft Tester…" while running, then "Cleaned Loft Tester"; nil from an older Mac.
    var title: String?
    var running: Bool
    var lines: [Line]
    var skipped: [CleanSkip]
    var freedBytes: Int64
    var removed: Int
    var failed: Int
}

/// A size as Settings › Disk says it: "0 bytes", never "Zero KB".
func bytesText(_ n: Int64) -> String {
    n == 0 ? "0 bytes" : n.formatted(.byteCount(style: .file))
}

/// The commands for "Copy commands": each agent's under a `# <name>` line.
func commandsText(_ items: [(agent: String, command: String)], name: (String) -> String) -> String {
    groupedByAgent(items, \.agent)
        .map { (["# \(name($0.agent))"] + $0.items.map(\.command)).joined(separator: "\n") }
        .joined(separator: "\n\n") + "\n"
}

/// Groups by agent, in the order they come.
func groupedByAgent<T>(_ items: [T], _ agent: (T) -> String) -> [(agent: String, items: [T])] {
    var out: [(agent: String, items: [T])] = []
    for i in items {
        if out.last?.agent == agent(i) { out[out.count - 1].items.append(i) } else { out.append((agent(i), [i])) }
    }
    return out
}
