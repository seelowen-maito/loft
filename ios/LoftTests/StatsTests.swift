import XCTest
@testable import Loft

/// Payloads captured from a running Loft (`/api/usage`, `/api/agents/summary`).
final class StatsTests: XCTestCase {
    let usageJSON = ###"{"error":null,"windows":[{"key":"five_hour","label":"session","percent":18.0,"resets_at":"2026-10-21T18:49:59.676909+00:00"},{"key":"seven_day","label":"week","percent":9.0,"resets_at":"2026-10-28T03:59:59.676930+00:00"}]}"###
    let summaryJSON = ###"{"agent": {"allowed_tools": ["Read", "Bash(git *)"], "avatar": {"svg": "<svg/>"}, "context_tokens": 138285, "created_at": "2026-10-11T10:44:58.501865+00:00", "disallowed_tools": [], "effort": "", "id": "agent-1", "interval_minutes": 180, "last_run_at": "2026-10-21T12:57:29.121396+00:00", "max_subagents": null, "mcp_servers": {}, "model": "opus", "name": "Backend Agent", "next_run_at": "2026-10-21T16:00:00+00:00", "permission_mode": "auto", "personality": "", "project": "loft", "prompt": "x", "quiet_hours": [{"days": [0, 1, 2, 3, 4], "end": "16:00", "start": "09:00"}], "running": false, "session_id": null, "status": "idle", "status_text": "", "subagent_model": "", "title": "", "total_cost_usd": 43760.16651370003, "workspace": "/tmp/agent-1"}, "cost_usd": 43760.16651370002, "daily_auto": [0, 0, 0, 11, 16, 24, 41, 17, 17, 13, 8, 6, 6, 3], "daily_user": [0, 0, 0, 24, 24, 38, 51, 8, 8, 9, 16, 0, 0, 1], "days": 14, "last_error": null, "last_reply": {"at": "2026-10-21T12:57:41.293660+00:00", "text": "Done."}, "memories": 12, "pending_permissions": 0, "prs": {"merged": 86, "open": 1, "total": 88}, "tool_calls": 2245, "top_tools": [["Bash", 1878], ["Read", 180], ["mcp__loft__message_agent", 64], ["Write", 39]], "user_messages": 116}"###

    func testPlanUsage() throws {
        let u = try JSONCoding.decoder.decode(PlanUsage.self, from: Data(usageJSON.utf8))
        XCTAssertNil(u.error)
        XCTAssertNotNil(u.session)
        XCTAssertNotNil(u.week)
        XCTAssertNotNil(u.session?.resetsAt)
        XCTAssert((0...100).contains(u.session!.percent))
    }

    func testSystemReading() throws {
        let r = try JSONCoding.decoder.decode(SystemReading.self, from: Data(#"{"cpu_temp_c":64.3,"level":"nominal","sampled_at":"2026-10-21T17:25:50.420977+00:00"}"#.utf8))
        XCTAssertEqual(r.cpuTempC, 64.3)
        XCTAssertEqual(r.level, "nominal")
        XCTAssertNotNil(r.sampledAt)
        let none = try JSONCoding.decoder.decode(SystemReading.self, from: Data(#"{"cpu_temp_c":null,"level":null,"sampled_at":null}"#.utf8))
        XCTAssertNil(none.cpuTempC)
        guard case .system(let e) = try ServerEvent.decode(Data(#"{"type":"system","system":{"cpu_temp_c":88.0,"level":"warm","sampled_at":null}}"#.utf8)) else { return XCTFail() }
        XCTAssertEqual(e.level, "warm")
    }

    func testContextWarnsFrom80k() {
        XCTAssertEqual(ContextWindow.warnAt, 80_000)
        XCTAssertFalse(ContextWindow.warns(0))
        XCTAssertFalse(ContextWindow.warns(79_999), "below 80k is neutral")
        XCTAssertTrue(ContextWindow.warns(80_000), "80k itself warns")
        XCTAssertTrue(ContextWindow.warns(250_000))
    }

    func testUsageError() throws {
        let u = try JSONCoding.decoder.decode(PlanUsage.self, from: Data(#"{"error":"not logged in","windows":[]}"#.utf8))
        XCTAssertEqual(u.error, "not logged in")
        XCTAssertNil(u.session)
    }

    func testAgentSummary() throws {
        let s = try JSONCoding.decoder.decode(AgentSummary.self, from: Data(summaryJSON.utf8))
        XCTAssertEqual(s.days, 14)
        XCTAssertEqual(s.dailyAuto.count, 14)
        XCTAssertEqual(s.dailyUser.count, 14)
        XCTAssertEqual(s.runs, s.dailyAuto.reduce(0, +) + s.dailyUser.reduce(0, +))
        XCTAssertFalse(s.topToolCounts.isEmpty)
        XCTAssertGreaterThanOrEqual(s.prs.total, s.prs.merged + s.prs.open)
        XCTAssertNotNil(s.agent.contextTokens)
    }

    @MainActor
    func testStoreLoadsStatsAndTotals() async throws {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("s-\(UUID()).json")))
        try await store.refresh()
        await store.refreshStats()
        XCTAssertEqual(store.usage?.session?.percent, 42)
        XCTAssertEqual(store.system?.cpuTempC, 64.3)
        XCTAssertEqual(store.summaries.count, store.agents.count)
        let t = store.totals
        XCTAssertEqual(t.daily.count, 14)
        XCTAssertEqual(t.runs, t.daily.reduce(0) { $0 + $1.auto + $1.user })
    }
}
