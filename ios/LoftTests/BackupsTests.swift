import XCTest
@testable import Loft
@testable import LoftKit

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

    /// #302: the iPhone reads and saves the idle timeout Web Settings has, sending only that key.
    @MainActor
    func testSavesOnlyTheIdleTimeout() async throws {
        let api = FixtureAPI()
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("it-\(UUID()).json")))
        store.activate()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        let before = try await store.idleTimeoutHours()
        XCTAssertEqual(before, 4)
        try await store.saveIdleTimeout(hours: 12)
        XCTAssertEqual(api.settingsPut.last, ["idle_timeout_hours": .number(12)])
        let after = try await store.idleTimeoutHours()
        XCTAssertEqual(after, 12)
        let now = try await api.serverSettings()
        XCTAssertEqual(now["auto_update"], true, "the rest stays as saved")
        store.deactivate()
    }

    /// `GET /api/workspaces`, the clean preview and the clean log as the server answers them.
    func testDecodesWorkspaceSizesTheCleanPreviewAndItsLog() throws {
        let json = #"{"agents":[{"id":"a","name":"Dev","bytes":5000,"busy":false},{"id":"b","name":"Lead","bytes":1,"busy":true}],"free_bytes":14300000000,"low_disk_bytes":20000000000}"#
        let s = try JSONCoding.decoder.decode(WorkspaceSizes.self, from: Data(json.utf8))
        XCTAssertEqual(s.agents.first?.bytes, 5000)
        XCTAssertEqual(s.agents.map(\.busy), [false, true])
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
        XCTAssertEqual(items.map(\.shownCommand), ["", "rm -rf /s/a/old", ""], "no command on what can't be removed")
        // The table's Status and Type columns.
        XCTAssertEqual(items.map(\.status.text), ["Kept: main clone of loft", "Deletable", "Working"])
        XCTAssertEqual(items.map(\.status.locked), [true, false, true])
        XCTAssertEqual(items.map(\.typeText), ["git clone", "git clone", "folder"])
        // Sorted by size, by agent name, and by last change.
        let name: (String) -> String = { $0 == "a" ? "Zed" : "Amy" }
        XCTAssertEqual(sortedClean(items, by: .size, agentName: name).map(\.rel), ["loft", "old", "x"])
        XCTAssertEqual(sortedClean(items, by: .agent, agentName: name).map(\.rel), ["x", "loft", "old"])
        XCTAssertEqual(sortedClean(items, by: .changed, agentName: name).map(\.rel), ["loft", "old", "x"])

        let log = try JSONCoding.decoder.decode(CleanLog.self, from: Data(#"""
        {"at":"2026-11-10T13:21:00.5+00:00","what":"Clean all","running":false,
         "lines":[{"agent":"a","path":"/s/a/loft/target","rel":"loft/target","bytes":700,"error":null,"command":"rm -rf /s/a/loft/target"},{"agent":"a","path":"/s/a/x/node_modules","rel":"x/node_modules","bytes":0,"error":"Permission denied","command":"rm -rf /s/a/x/node_modules"}],
         "skipped":[{"agent":"a","rel":"loft/ios/build","reason":"unticked"}],"freed_bytes":700,"removed":1,"failed":1}
        """#.utf8))
        XCTAssertEqual(log.freedBytes, 700)
        XCTAssertEqual(log.lines.last?.error, "Permission denied")
        XCTAssertEqual(log.lines.map(\.text), ["Removed /s/a/loft/target (\(bytesText(700)))", "failed /s/a/x/node_modules: Permission denied"])
        XCTAssertEqual([log.removed, log.failed], [1, 1])
        XCTAssertEqual(groupedByAgent(log.lines, \.agent).map(\.items.count), [2])
        XCTAssertNil(log.title, "an older Mac's log has no title")
    }

    /// Settings › Disk lists what an idle agent left running, and counts it; an agent is busy only in a run.
    func testDecodesWhatIdleAgentsLeftRunning() throws {
        let sizes = try JSONCoding.decoder.decode(WorkspaceSizes.self, from: Data(#"{"agents":[{"id":"a","name":"Dev","bytes":5,"busy":false,"processes":2}],"free_bytes":9,"low_disk_bytes":20}"#.utf8))
        XCTAssertEqual(sizes.agents.first?.processes, 2)
        let left = try JSONCoding.decoder.decode([String: [LeftProcess]].self, from: Data(#"{"a-1":[{"pid":41,"command":"node vite","age_secs":21600},{"pid":42,"command":"tail -f x","age_secs":90}]}"#.utf8))
        XCTAssertEqual(left["a-1"]?.map(\.pid), [41, 42])
        XCTAssertEqual(left["a-1"]?.map(\.ageText), ["6 h", "1 min"])
    }

    /// Settings › Disk lists agents' Simulator devices; the clean log says which device was deleted or shut down.
    func testDecodesAgentDevicesAndTheirLogLines() throws {
        let json = #"{"devices":[{"udid":"U1","name":"Loft Dev (Loft)","state":"Booted","bytes":2878722048,"agent":{"id":"a","name":"Loft Dev"}},{"udid":"U2","name":"Loft Lead (Loft)","state":"Shutdown","bytes":5,"agent":{"id":"b","name":"Loft Lead"}}]}"#
        let d = try JSONCoding.decoder.decode(AgentDevices.self, from: Data(json.utf8)).devices
        XCTAssertEqual(d.map(\.title), ["Loft Dev", "Loft Lead"])
        XCTAssertEqual(d.map(\.stateText), ["Booted", "Shut down"])
        XCTAssertEqual(d.first?.bytes, 2_878_722_048)

        let log = try JSONCoding.decoder.decode(CleanLog.self, from: Data(#"""
        {"at":"2026-11-10T13:21:00+00:00","what":"Simulator cleanup","title":"Simulator cleanup","running":false,
         "lines":[{"agent":"","path":"/sim/U3","rel":"Loft Dev 93543007","bytes":700,"error":null,"command":"xcrun simctl delete U3","done":"Deleted Simulator device"},
                  {"agent":"a","path":"/sim/U1","rel":"Loft Dev (Loft)","bytes":0,"error":null,"command":"xcrun simctl shutdown U1","done":"Shut down Simulator device"},
                  {"agent":"","path":"/sim/U4","rel":"Gone (Loft)","bytes":0,"error":"busy","command":"xcrun simctl delete U4","done":"Deleted Simulator device"},
                  {"agent":"a","path":"pid:4242","rel":"node vite --port 20070","bytes":0,"error":null,"command":"kill 4242","done":"Stopped process"}],
         "skipped":[],"freed_bytes":700,"removed":2,"failed":1}
        """#.utf8))
        XCTAssertEqual(log.lines.map(\.text), ["Deleted Simulator device Loft Dev 93543007 (\(bytesText(700)))", "Shut down Simulator device Loft Dev (Loft)", "failed: Deleted Simulator device Gone (Loft): busy", "Stopped process node vite --port 20070"])
    }

    /// Sizes read "0 bytes", never "Zero KB".
    func testSizesReadZeroBytes() {
        XCTAssertEqual(bytesText(0), "0 bytes")
        XCTAssertEqual(bytesText(2_200_000_000), 2_200_000_000.formatted(.byteCount(style: .file)))
    }

    /// Clean's Check as the Mac answers it, read with its sign; only clones and worktrees get the button.
    func testGitCheckReadsWithItsSign() throws {
        let c = try JSONCoding.decoder.decode(GitCheck.self, from: Data(#"{"state":"unsafe","lines":["2 commits not pushed (main)","Uncommitted changes (1 file)"]}"#.utf8))
        XCTAssertEqual(c.text, "⚠️ 2 commits not pushed (main) · Uncommitted changes (1 file)")
        XCTAssertEqual(GitCheck(state: "safe", lines: ["Clean, all pushed: safe to delete"]).text, "✅ Clean, all pushed: safe to delete")
        XCTAssertEqual(GitCheck(state: "unknown", lines: ["Can't check: unusual git config"]).text, "❔ Can't check: unusual git config")
    }

    /// "Copy commands": each agent's `rm -rf` lines under its name.
    func testCopyCommandsGroupsTheCommandsByAgent() {
        let items = [(agent: "a", command: "rm -rf /s/a/x"), (agent: "a", command: "rm -rf /s/a/y"), (agent: "b", command: "rm -rf /s/b/z")]
        XCTAssertEqual(commandsText(items) { $0 == "a" ? "Loft Dev" : "Lead" }, "# Loft Dev\nrm -rf /s/a/x\nrm -rf /s/a/y\n\n# Lead\nrm -rf /s/b/z\n")
        // A name with a newline (a deleted agent's folder) stays on its `#` line.
        XCTAssertEqual(commandsText([(agent: "\nrm -r ~", command: "rm -rf /s/x")]) { "Deleted agent \($0.prefix(8))" }, "# Deleted agent rm -r ~\nrm -rf /s/x\n")
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
