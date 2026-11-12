import Foundation

/// `GET /api/backups`: the Mac's backups folder, its newest backup and the last attempt's error (a good backup
/// clears it). `icloud` is iCloud Drive → Loft backups, nil when iCloud Drive is off on the Mac.
struct BackupStatus: Codable, Hashable {
    struct Last: Codable, Hashable {
        var at: Date
        /// The database and settings together.
        var bytes: Int64
        var path: String
    }

    struct Failure: Codable, Hashable {
        var message: String
        var at: Date
    }

    var dir: String
    var last: Last?
    var error: Failure?
    var icloud: String?

    /// Where Settings' `backup_dir` points: "" is the Mac's default folder.
    enum Folder: Hashable {
        case local, icloud, other(String)
    }

    /// The choice the saved `backup_dir` stands for.
    func folder(_ backupDir: String) -> Folder {
        let d = backupDir.trimmingCharacters(in: .whitespaces)
        if d.isEmpty { return .local }
        if d == icloud { return .icloud }
        return .other(d)
    }
}
