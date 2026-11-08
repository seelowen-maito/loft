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

    func testAPushLinkOpensOnlyOnGitHubOrTheConnectedLoft() {
        let open = { (url: String, host: String?) in PushDestination.from(["type": "delivery_done", "target": ["url": url]], loftHost: host) }
        for url in ["https://github.com/o/r/pull/1", "https://github.com/o/r/issues/40", "https://GitHub.com/o/r/pull/1"] {
            XCTAssertEqual(open(url, "mac.tail1.ts.net"), .url(URL(string: url)!), url)
            XCTAssertEqual(open(url, nil), .url(URL(string: url)!), "\(url), with no Loft connected")
        }
        XCTAssertEqual(open("https://mac.tail1.ts.net:7420/tasks/t1", "mac.tail1.ts.net"), .url(URL(string: "https://mac.tail1.ts.net:7420/tasks/t1")!), "the connected Loft")
        XCTAssertEqual(open("https://Mac.Tail1.ts.net/x", "mac.tail1.ts.net"), .url(URL(string: "https://Mac.Tail1.ts.net/x")!), "hosts have no case")
        for url in [
            "https://example.com/phish", "https://github.com.evil.example/o/r", "https://evil.example/github.com",
            "https://github.com@evil.example/o/r/pull/1", "https://gist.github.com/x", "https://other-mac.ts.net/tasks/t1",
            "http://github.com/o/r/pull/1", "http://mac.tail1.ts.net:7420/tasks/t1", "ftp://github.com/x", "javascript:alert(1)", "github.com/o/r",
        ] {
            XCTAssertNil(open(url, "mac.tail1.ts.net"), "\(url) just opens the app")
        }
        XCTAssertNil(PushDestination.from(["needs_you_id": "pr_ready:1", "target": ["url": "https://example.com/x", "screen": "chat", "agent_id": "be"]], loftHost: nil),
                     "an untrusted link isn't swapped for another destination: the tap only opens the app")
        // Targets without a link route as before, whatever the host.
        XCTAssertEqual(PushDestination.from(["target": ["screen": "chat", "agent_id": "be"]], loftHost: "mac.tail1.ts.net"), .chat(agentId: "be"))
        XCTAssertEqual(PushDestination.from(["target": ["screen": "task", "task_id": "t1"]], loftHost: "mac.tail1.ts.net"), .task(taskId: "t1"))
        XCTAssertEqual(PushDestination.from(["target": ["screen": "needs_you"]], loftHost: "mac.tail1.ts.net"), .needsYou)
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
        XCTAssertEqual(Set(r.kinds), ["permission", "task_approval", "task_blocked", "task_failed", "question", "grill_round", "stuck", "delivery_stalled", "delivery_done"])
        d.set(false, forKey: PushKind.deliveryStalled.prefKey)
        XCTAssertFalse(DeviceRegistration.current(token: "abc", defaults: d).kinds.contains("delivery_stalled"), "its switch turns the Delivery stalled push off")
        d.removeObject(forKey: PushKind.deliveryStalled.prefKey)
        d.set(false, forKey: PushKind.deliveryDone.prefKey)
        XCTAssertFalse(DeviceRegistration.current(token: "abc", defaults: d).kinds.contains("delivery_done"), "its switch turns the Delivery done push off")
        d.removeObject(forKey: PushKind.deliveryDone.prefKey)
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
