import Foundation

/// One git repo in the projects folder (`GET /api/repos`), as the web's home counts it.
public struct RepoSummary: Codable, Hashable, Identifiable {
    public var name: String
    public var path: String
    public var branch: String
    public var ahead: Int
    public var behind: Int
    public var staged: Int
    public var modified: Int
    public var untracked: Int
    public var totalCommits: Int
    /// Commits per day, oldest first, the last 26 weeks.
    public var daily: [Int]
    public var error: String?
    /// The repo detail's numbers: branches, files per language and commits per author, biggest first.
    public var branches: Int?
    public var languages: [Share]?
    public var contributors: [Share]?

    public var id: String { name }
    /// Has local changes (staged, modified or untracked).
    public var isDirty: Bool { staged + modified + untracked > 0 }
    public var commits30: Int { daily.suffix(30).reduce(0, +) }
    public var commitsPrior30: Int { daily.dropLast(30).suffix(30).reduce(0, +) }
}

/// A repo's open pull requests and issues on GitHub (`GET /api/repos/github`, by repo name; read `slug`, not the
/// key, since the decoder's snake-case conversion may rewrite keys).
public struct RepoGithub: Codable, Hashable {
    public struct Item: Codable, Hashable, Identifiable {
        public var number: Int
        public var title: String
        public var author: String
        public var url: String
        public var draft: Bool
        public var updatedAt: Date
        public var labels: [String]?
        public var id: String { url }
    }

    /// "owner/name"
    public var slug: String?
    public var pulls: [Item]
    public var issues: [Item]
    public var error: String?
    public var stars: Int?
    public var forks: Int?
    public var description: String?
}

/// `GET /api/agents/summary?totals=true` without the per-agent rows: runs and spend this period, the one before,
/// and all time.
public struct AgentsTotals: Codable, Hashable {
    public struct Spend: Codable, Hashable { public var runs: Int; public var costUsd: Double }
    public struct Period: Codable, Hashable {
        /// `usage_week` (since the plan's weekly reset) or `last_7_days`
        public var kind: String
        public var runs: Int
        public var costUsd: Double
        public var previous: Spend
    }

    public var period: Period
    public var allTime: Spend
}

/// A `[name, count]` pair, as `/api/repos` sends languages and contributors.
public struct Share: Codable, Hashable {
    public var name: String
    public var count: Int

    public init(_ name: String, _ count: Int) { self.name = name; self.count = count }

    public init(from decoder: Decoder) throws {
        var c = try decoder.unkeyedContainer()
        name = try c.decode(String.self)
        count = try c.decode(Int.self)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.unkeyedContainer()
        try c.encode(name)
        try c.encode(count)
    }
}

/// A repo's recent commit (`GET /api/repos/{name}/commits`); `timestamp` in seconds.
public struct RepoCommit: Codable, Hashable, Identifiable {
    public var hash: String
    public var subject: String
    public var author: String
    public var timestamp: Int64
    public var insertions: Int?
    public var deletions: Int?

    public var id: String { hash }
    public var date: Date { Date(timeIntervalSince1970: TimeInterval(timestamp)) }
}
