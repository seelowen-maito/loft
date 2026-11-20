import Foundation

/// `GET /api/backups`: the Mac's backups folder, its newest backup and the last attempt's error (a good backup
/// clears it). `icloud` is iCloud Drive → Loft backups, nil when iCloud Drive is off on the Mac.
public struct BackupStatus: Codable, Hashable {
    public struct Last: Codable, Hashable {
        public var at: Date
        /// The database and settings together.
        public var bytes: Int64
        public var path: String
    }

    public struct Failure: Codable, Hashable {
        public var message: String
        public var at: Date
    }

    public var dir: String
    public var last: Last?
    public var error: Failure?
    public var icloud: String?

    /// Where Settings' `backup_dir` points: "" is the Mac's default folder.
    public enum Folder: Hashable {
        case local, icloud, other(String)
    }

    /// The choice the saved `backup_dir` stands for.
    public func folder(_ backupDir: String) -> Folder {
        let d = backupDir.trimmingCharacters(in: .whitespaces)
        if d.isEmpty { return .local }
        if d == icloud { return .icloud }
        return .other(d)
    }
}
