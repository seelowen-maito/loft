import XCTest
@testable import Loft

final class BackupsTests: XCTestCase {
    /// Choosing a folder sends only `backup_dir`, so a setting changed on the web since the last read isn't overwritten.
    @MainActor
    func testChoosingAFolderSendsOnlyTheFolder() async throws {
        let api = FixtureAPI()
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("bk-\(UUID()).json")))
        store.activate()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        try await store.saveBackupDir("/Volumes/Disk/Loft backups")
        XCTAssertEqual(api.settingsPut.last, ["backup_dir": "/Volumes/Disk/Loft backups"])
        let now = try await api.serverSettings()
        XCTAssertEqual(now["backup_dir"], "/Volumes/Disk/Loft backups")
        XCTAssertEqual(now["auto_update"], true, "the rest stays as saved")
        store.deactivate()
    }

    /// `GET /api/workspaces`, the clean preview and the clean log as the server answers them.
    func testDecodesWorkspaceSizesTheCleanPreviewAndItsLog() throws {
        let json = #"{"agents":[{"id":"a","name":"Dev","bytes":5000,"busy":false},{"id":"b","name":"Lead","bytes":1,"busy":true}],"free_bytes":14300000000,"low_disk_bytes":20000000000}"#
        let s = try JSONCoding.decoder.decode(WorkspaceSizes.self, from: Data(json.utf8))
        XCTAssertEqual(s.agents.first?.bytes, 5000)
        XCTAssertEqual(s.agents.map(\.busy), [false, true])
        XCTAssertEqual(s.idleCount, 1, "Clean all skips the busy agent")
        XCTAssertTrue(s.isLow)
        // A Mac without per-agent Clean: no `busy`, so no Clean button on the row.
        let old = try JSONCoding.decoder.decode(WorkspaceSizes.self, from: Data(#"{"agents":[{"id":"a","name":"Dev","bytes":5}],"free_bytes":null,"low_disk_bytes":1}"#.utf8))
        XCTAssertNil(old.agents.first?.busy)
        let ok = try JSONCoding.decoder.decode(WorkspaceSizes.self, from: Data(json.replacingOccurrences(of: "14300000000", with: "50000000000").utf8))
        XCTAssertFalse(ok.isLow)

        let listing = try JSONCoding.decoder.decode(CleanListing.self, from: Data(#"""
        {"items":[{"agent":"a","path":"/s/a/loft","rel":"loft","name":"loft","kind":"clone","bytes":700,"changed":"2026-11-10T13:20:01.123456789+00:00","command":"rm -rf /s/a/loft",
                   "label":"main clone of loft","note":"Kept: this is the agent's working copy.","warning":"","ticked":false,"blocked":"","opens":true},
                  {"agent":"a","path":"/s/a/old","rel":"old","name":"old","kind":"clone","bytes":5,"changed":"2026-11-01T12:00:00+00:00","command":"rm -rf /s/a/old",
                   "label":"","note":"","warning":"may contain unpushed work","ticked":true,"blocked":"","opens":false},
                  {"agent":"b","path":"/s/b/x","rel":"x","name":"x","kind":"folder","bytes":1,"changed":"2026-11-01T12:00:00+00:00","command":"rm -rf /s/b/x",
                   "label":"","note":"","warning":"","ticked":false,"blocked":"Working","opens":false}]}
        """#.utf8))
        let items = listing.items
        XCTAssertEqual(items.map(\.rel), ["loft", "old", "x"])
        XCTAssertEqual(items.map(\.canTick), [false, true, false], "a main clone and a working agent's items can't be ticked")
        XCTAssertEqual(items.map(\.ticked), [false, true, false])
        XCTAssertEqual(items.first?.command, "rm -rf /s/a/loft")

        let log = try JSONCoding.decoder.decode(CleanLog.self, from: Data(#"""
        {"at":"2026-11-10T13:21:00.5+00:00","what":"Clean all","running":false,
         "lines":[{"agent":"a","path":"/s/a/loft/target","rel":"loft/target","bytes":700,"error":null,"command":"rm -rf /s/a/loft/target"},{"agent":"a","path":"/s/a/x/node_modules","rel":"x/node_modules","bytes":0,"error":"Permission denied","command":"rm -rf /s/a/x/node_modules"}],
         "skipped":[{"agent":"a","rel":"loft/ios/build","reason":"unticked"}],"freed_bytes":700,"removed":1,"failed":1}
        """#.utf8))
        XCTAssertEqual(log.freedBytes, 700)
        XCTAssertEqual(log.lines.last?.error, "Permission denied")
        XCTAssertEqual([log.removed, log.failed], [1, 1])
        XCTAssertEqual(groupedByAgent(log.lines, \.agent).map(\.items.count), [2])
        XCTAssertNil(log.title, "an older Mac's log has no title")
    }

    /// Sizes read "0 bytes", never "Zero KB".
    func testSizesReadZeroBytes() {
        XCTAssertEqual(bytesText(0), "0 bytes")
        XCTAssertEqual(bytesText(2_200_000_000), 2_200_000_000.formatted(.byteCount(style: .file)))
    }

    /// "Copy commands": each agent's `rm -rf` lines under its name.
    func testCopyCommandsGroupsTheCommandsByAgent() {
        let items = [(agent: "a", command: "rm -rf /s/a/x"), (agent: "a", command: "rm -rf /s/a/y"), (agent: "b", command: "rm -rf /s/b/z")]
        XCTAssertEqual(commandsText(items) { $0 == "a" ? "Loft Dev" : "Lead" }, "# Loft Dev\nrm -rf /s/a/x\nrm -rf /s/a/y\n\n# Lead\nrm -rf /s/b/z\n")
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
