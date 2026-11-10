import XCTest
@testable import Loft

final class BackupsTests: XCTestCase {
    /// `GET /api/workspaces` and `POST /api/workspaces/clean` as the server answers them.
    func testDecodesWorkspaceSizesAndWhatCleanNowFreed() throws {
        let json = #"{"agents":[{"id":"a","name":"Dev","bytes":5000}],"free_bytes":14300000000,"low_disk_bytes":20000000000,"last_sweep":{"at":null,"removed":[],"freed_bytes":0}}"#
        let s = try JSONCoding.decoder.decode(WorkspaceSizes.self, from: Data(json.utf8))
        XCTAssertEqual(s.agents.first?.bytes, 5000)
        XCTAssertTrue(s.isLow)
        let ok = try JSONCoding.decoder.decode(WorkspaceSizes.self, from: Data(json.replacingOccurrences(of: "14300000000", with: "50000000000").utf8))
        XCTAssertFalse(ok.isLow)
        let r = try JSONCoding.decoder.decode(SweepReport.self, from: Data(#"{"at":"2026-11-10T12:00:00+01:00","removed":["/x"],"freed_bytes":777}"#.utf8))
        XCTAssertEqual(r.freedBytes, 777)
    }

    /// `GET /api/backups` as the server answers it (chrono's local times, with an offset).
    func testDecodesTheServersStatusAndNamesTheFolderChoice() throws {
        let json = #"""
        {"dir":"/Users/me/Library/Application Support/Loft/backups",
         "last":{"at":"2026-11-10T11:12:52.123456+01:00","bytes":176850,"path":"/Users/me/Library/Application Support/Loft/backups/2026-11-10_111252"},
         "error":{"message":"can't create /Volumes/Gone","at":"2026-11-10T12:00:00+01:00"},
         "icloud":"/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Loft backups"}
        """#
        let s = try JSONCoding.decoder.decode(BackupStatus.self, from: Data(json.utf8))
        XCTAssertEqual(s.last?.bytes, 176_850)
        XCTAssertEqual(s.error?.message, "can't create /Volumes/Gone")
        XCTAssertEqual(s.folder(""), .local)
        XCTAssertEqual(s.folder("/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Loft backups"), .icloud)
        XCTAssertEqual(s.folder("/Volumes/Disk/Loft backups"), .other("/Volumes/Disk/Loft backups"))

        // iCloud Drive off on the Mac, and no backup yet.
        let off = try JSONCoding.decoder.decode(BackupStatus.self, from: Data(#"{"dir":"/x","last":null,"error":null,"icloud":null}"#.utf8))
        XCTAssertNil(off.icloud)
        XCTAssertNil(off.last)
        XCTAssertEqual(off.folder("/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Loft backups"), .other("/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Loft backups"))
    }
}
