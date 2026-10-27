import XCTest
@testable import Loft

final class PushTests: XCTestCase {
    func testAPushOpensItsItem() {
        XCTAssertEqual(PushDestination.from(["needs_you_id": "permission:p1", "target": ["screen": "chat", "agent_id": "be"]]), .chat(agentId: "be"))
        XCTAssertEqual(PushDestination.from(["target": ["screen": "task", "task_id": "t1"]]), .task(taskId: "t1"))
        XCTAssertEqual(PushDestination.from(["target": ["url": "https://github.com/o/r/pull/7"]]), .url(URL(string: "https://github.com/o/r/pull/7")!))
        XCTAssertEqual(PushDestination.from(["needs_you_id": "stuck:be:1"]), .needsYou, "no target: the Needs you list")
        XCTAssertEqual(PushDestination.from(["type": "summary", "target": ["screen": "needs_you"]]), .needsYou, "the rate-limit summary")
        XCTAssertNil(PushDestination.from(["type": "test"]), "a test push just opens the app")
        XCTAssertNil(PushDestination.from(["target": ["url": "javascript:alert(1)"]]), "only https links open")
        XCTAssertNil(PushDestination.from([:]))
    }

    @MainActor
    func testRoutingLandsOnTheItemWithoutExtraScreens() {
        let r = Router()
        r.tab = .agents
        r.open(.task(taskId: "t1"))
        XCTAssertEqual(r.tab, .tasks)
        XCTAssertEqual(r.tasksPath.count, 1, "one level: Back goes to the task list")
        r.open(.needsYou)
        XCTAssertEqual(r.tab, .overview)
        XCTAssertEqual(r.overviewPath.count, 1)
        r.open(.chat(agentId: "be"))
        XCTAssertEqual(r.tab, .chat)
        XCTAssertEqual(r.chatPath.count, 1)
    }

    func testAPushAndALocalNotificationForTheSameItemShowOnce() {
        let delivered: [(id: String, info: [AnyHashable: Any])] = [("apns-1", ["needs_you_id": "permission:p1"]), ("task:t1:blocked", [:])]
        XCTAssertTrue(Notifier.alreadyShown("permission:p1", in: delivered), "a push already showed it")
        XCTAssertTrue(Notifier.alreadyShown("task:t1:blocked", in: delivered), "a local one already showed it")
        XCTAssertFalse(Notifier.alreadyShown("task:t1:failed", in: delivered), "a changed item is a new one")
        XCTAssertEqual(NeedsYouKey.permission("p1"), "permission:p1", "the same ids as the server's Needs you list")
        XCTAssertEqual(NeedsYouKey.task("t1", "blocked"), "task:t1:blocked")
    }

    func testTheRegistrationCarriesKindsAndQuietHours() {
        let d = UserDefaults(suiteName: "push-test-\(UUID())")!
        d.set(false, forKey: PushKind.prReady.prefKey)
        d.set(true, forKey: Prefs.pushQuietHours)
        d.set("23:00", forKey: Prefs.pushQuietStart)
        let r = DeviceRegistration.current(token: "abc", defaults: d)
        XCTAssertEqual(r.token, "abc")
        XCTAssertFalse(r.kinds.contains("pr_ready"))
        XCTAssertTrue(r.kinds.contains("permission"), "on unless turned off")
        // The server's kinds, one to one with the Needs you types (it rejects unknown names).
        XCTAssertEqual(Set(r.kinds), ["permission", "task_approval", "task_blocked", "task_failed", "question", "grill_round", "stuck"])
        XCTAssertEqual(r.quietHours?.start, "23:00")
        XCTAssertEqual(r.quietHours?.end, "07:00")
        guard case .object(let o) = r.json else { return XCTFail() }
        XCTAssertEqual(o["environment"], .string("development"), "Debug builds use the APNs sandbox")
        guard case .object(let q)? = o["quiet_hours"] else { return XCTFail() }
        XCTAssertEqual(q["utc_offset_minutes"], .number(Double(TimeZone.current.secondsFromGMT() / 60)), "the server has no tz database")
        d.set(false, forKey: Prefs.pushQuietHours)
        guard case .object(let o2) = DeviceRegistration.current(token: "abc", defaults: d).json else { return XCTFail() }
        XCTAssertNil(o2["quiet_hours"], "off: left out of the registration (cleared with a PATCH)")
    }
}
