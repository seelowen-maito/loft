import Foundation

/// `GET /api/workspaces`: each agent's workspace size and the Mac's free disk.
public struct WorkspaceSizes: Codable, Hashable {
    public struct Row: Codable, Hashable, Identifiable {
        public var id: String
        public var name: String
        public var bytes: Int64
        /// In a run, so Clean skips it. Nil from a Mac without per-agent Clean.
        public var busy: Bool?
        /// How many processes an idle agent left running. Nil from a Mac that doesn't say.
        public var processes: Int?
    }

    public var agents: [Row]
    /// Nil when the Mac couldn't say.
    public var freeBytes: Int64?
    public var lowDiskBytes: Int64

    /// Under the server's limit (20 GB), where Needs you says "Disk almost full".
    public var isLow: Bool { freeBytes.map { $0 < lowDiskBytes } ?? false }
}

/// `GET /api/workspaces/processes`: a process an idle agent left running, and how long ago it started.
public struct LeftProcess: Codable, Hashable, Identifiable {
    public var pid: Int
    public var command: String
    public var ageSecs: Int
    public var id: Int { pid }
    public var ageText: String {
        ageSecs < 3600 ? "\(max(1, ageSecs / 60)) min" : ageSecs < 86400 ? "\(ageSecs / 3600) h" : "\(ageSecs / 86400) d"
    }
}

/// What a clean leaves, and why. `rel` is inside the agent's workspace, empty for all of it.
public struct CleanSkip: Codable, Hashable {
    public var agent: String
    public var rel: String
    public var reason: String
}

/// One thing Clean lists: a workspace's top-level item, or a folder inside a main clone
/// (`GET /api/workspaces/clean/preview` and `.../folders`).
public struct CleanItem: Codable, Hashable, Identifiable {
    public var agent: String
    public var path: String
    /// Inside the agent's workspace, like `loft` or `loft/backend/target`.
    public var rel: String
    public var name: String
    /// "clone", "worktree", "build" (build output), "folder", "file" or "symlink".
    public var kind: String
    public var bytes: Int64
    /// Its newest file's change.
    public var changed: Date
    /// The equivalent `rm -rf`, to read or copy; the Mac removes by path. Empty for `.git`, which never goes.
    public var command: String
    /// Why it starts unticked, like "main clone of loft".
    public var label: String
    /// Said under it, like what keeping a main clone costs.
    public var note: String
    /// What removing it risks, like "may contain unpushed work".
    public var warning: String
    public var ticked: Bool
    /// Why it can't be ticked ("Working", "symlink", "git history: never deleted"); empty when it can.
    public var blocked: String
    /// It opens up to its own folders.
    public var opens: Bool
    public var id: String { path }
    /// A main clone itself can't be ticked; the folders inside it can.
    public var canTick: Bool { blocked.isEmpty && !(opens && !label.isEmpty) }
    /// The `rm -rf` shown under it; none when it can't be removed.
    public var shownCommand: String { canTick ? command : "" }
    /// The Type column: "git clone", "build output", or the kind as it is.
    public var typeText: String { kind == "clone" ? "git clone" : kind == "build" ? "build output" : kind }
    /// The Status column: "Working", "Locked: symlink", "Kept: main clone of loft" or "Deletable", and whether it is locked.
    public var status: (text: String, locked: Bool) {
        if !blocked.isEmpty { return (blocked == "Working" ? "Working" : "Locked: \(blocked)", true) }
        if !canTick { return ("Kept: \(label)", true) }
        return ("Deletable", false)
    }
    /// It gets a Check button: a clone or worktree (only top-level items are).
    public var canCheck: Bool { kind == "clone" || kind == "worktree" }
}

/// `POST /api/workspaces/clean/check`: whether a clone or worktree is safe to delete. `state` is "safe" (clean and
/// pushed), "unsafe" (unpushed commits or uncommitted changes) or "unknown" (the Mac wouldn't run git there).
public struct GitCheck: Codable, Hashable {
    public var state: String
    public var lines: [String]

    public init(state: String, lines: [String]) {
        self.state = state
        self.lines = lines
    }
    /// "✅ Clean, all pushed: safe to delete" or "⚠️ 2 commits not pushed (main) · …".
    public var text: String { "\(state == "safe" ? "✅" : state == "unsafe" ? "⚠️" : "❔") \(lines.joined(separator: " · "))" }
}

public struct CleanListing: Codable, Hashable {
    public var items: [CleanItem]
}

/// `POST /api/workspaces/clean` and `GET /api/workspaces/log`: a clean's log, live while `running`.
public struct CleanLog: Codable, Hashable {
    public struct Line: Codable, Hashable, Identifiable {
        public var agent: String
        public var path: String
        public var rel: String
        public var bytes: Int64
        public var error: String?
        public var command: String
        /// What was done when it isn't a removal, with its noun: "Shut down Simulator device" or "Stopped process" (`rel` names it); nil for a removal.
        public var done: String?
        public var id: String { path }
        /// Past tense like the log's header: "Removed /path (2.2 GB)" or "failed /path: why".
        public var text: String {
            if let done, !done.isEmpty {
                let what = "\(done) \(rel)"
                return error.map { "failed: \(what): \($0)" } ?? (bytes > 0 ? "\(what) (\(bytesText(bytes)))" : what)
            }
            return error.map { "failed \(path): \($0)" } ?? "Removed \(path) (\(bytesText(bytes)))"
        }
    }

    public var at: Date?
    /// "Clean", "Clean all", "Daily sweep" or "Sweep before update".
    public var what: String
    /// "Cleaning Loft Tester…" while running, then "Cleaned Loft Tester"; nil from an older Mac.
    public var title: String?
    public var running: Bool
    public var lines: [Line]
    public var skipped: [CleanSkip]
    public var freedBytes: Int64
    public var removed: Int
    public var failed: Int

    public init(at: Date?, what: String, title: String?, running: Bool, lines: [Line], skipped: [CleanSkip], freedBytes: Int64,
                removed: Int, failed: Int) {
        self.at = at
        self.what = what
        self.title = title
        self.running = running
        self.lines = lines
        self.skipped = skipped
        self.freedBytes = freedBytes
        self.removed = removed
        self.failed = failed
    }
}

/// `GET /api/simulator/devices`: an agent's Simulator device, its size on disk and whether it is booted.
public struct AgentDevice: Codable, Hashable, Identifiable {
    public struct Owner: Codable, Hashable {
        public var id: String
        public var name: String
    }

    public var udid: String
    public var name: String
    /// "Booted" or "Shutdown", as simctl says.
    public var state: String
    public var bytes: Int64
    /// Nil for an orphan: a device of a deleted agent, which the tidy deletes.
    public var agent: Owner?
    public var id: String { udid }
    /// What Settings › Disk calls it.
    public var title: String { agent.map(\.name) ?? "Orphaned · \(name)" }
    public var stateText: String { state == "Booted" ? "Booted" : "Shut down" }
}

public struct AgentDevices: Codable, Hashable {
    public var devices: [AgentDevice]
}

/// A size as Settings › Disk says it: "0 bytes", never "Zero KB".
public func bytesText(_ n: Int64) -> String {
    n == 0 ? "0 bytes" : n.formatted(.byteCount(style: .file))
}

/// The commands for "Copy commands": each agent's under a `# <name>` line, kept to that line whatever the name.
public func commandsText(_ items: [(agent: String, command: String)], name: (String) -> String) -> String {
    groupedByAgent(items, \.agent)
        .map { (["# " + String(String.UnicodeScalarView(name($0.agent).unicodeScalars.filter { !CharacterSet.controlCharacters.contains($0) }))] + $0.items.map(\.command)).joined(separator: "\n") }
        .joined(separator: "\n\n") + "\n"
}

/// Groups by agent, in the order they come.
public func groupedByAgent<T>(_ items: [T], _ agent: (T) -> String) -> [(agent: String, items: [T])] {
    var out: [(agent: String, items: [T])] = []
    for i in items {
        if out.last?.agent == agent(i) { out[out.count - 1].items.append(i) } else { out.append((agent(i), [i])) }
    }
    return out
}

/// How Clean's table is ordered.
public enum CleanSort: String, CaseIterable, Identifiable {
    case size = "Size", agent = "Agent", changed = "Last change"
    public var id: String { rawValue }
}

/// Clean's table order: by size (biggest first), agent name (A–Z) or last change (newest first); ties go biggest
/// first, then by name.
public func sortedClean(_ items: [CleanItem], by sort: CleanSort, agentName: (String) -> String) -> [CleanItem] {
    items.sorted { a, b in
        switch sort {
        case .size where a.bytes != b.bytes: return a.bytes > b.bytes
        case .changed where a.changed != b.changed: return a.changed > b.changed
        case .agent:
            let (x, y) = (agentName(a.agent).lowercased(), agentName(b.agent).lowercased())
            if x != y { return x < y }
        default: break
        }
        return a.bytes != b.bytes ? a.bytes > b.bytes : a.name < b.name
    }
}
