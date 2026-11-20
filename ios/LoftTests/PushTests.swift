import XCTest
@testable import Loft
@testable import LoftKit

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

    /// Needs you's Open and the Overview's rows open a server link only when it's https on github.com or the Loft's host.
    func testAServerLinkOpensOnlyWhenTrusted() {
        let host = "mac.tail1.ts.net"
        XCTAssertEqual(PushDestination.link("https://github.com/o/r/pull/7", loftHost: host), URL(string: "https://github.com/o/r/pull/7"))
        XCTAssertEqual(PushDestination.link("https://MAC.tail1.ts.net/tasks/t1", loftHost: host), URL(string: "https://MAC.tail1.ts.net/tasks/t1"))
        XCTAssertNil(PushDestination.link("http://github.com/o/r/pull/7", loftHost: host), "not https")
        XCTAssertNil(PushDestination.link("javascript:alert(1)", loftHost: host))
        XCTAssertNil(PushDestination.link("file:///etc/passwd", loftHost: host))
        XCTAssertNil(PushDestination.link("ssh://github.com/o/r", loftHost: host))
        XCTAssertNil(PushDestination.link("https://evil.example/x", loftHost: host))
        XCTAssertNil(PushDestination.link("https://github.com.evil.example/x", loftHost: host))
        XCTAssertNil(PushDestination.link("https://mac.tail1.ts.net/x", loftHost: nil), "no connected Loft: only github.com")
        XCTAssertNil(PushDestination.link(nil, loftHost: host))
    }

    /// Needs you's Open also takes the server's Settings (`http`) and Screen Sharing (`vnc`) links on the Loft's own
    /// hosts, and still refuses everything else.
    func testANeedsYouLinkOpensTheLoftsOwnSettingsAndScreenSharing() {
        func link(_ s: String, loft: String? = "127.0.0.1") -> URL? {
            PushDestination.needsYouLink(s, loftHost: loft, tailnetHost: "mac.tail1.ts.net")
        }
        XCTAssertNotNil(link("http://mac.tail1.ts.net:7420/settings#mac-access"), "Open Mac access")
        XCTAssertNotNil(link("http://mac.tail1.ts.net:7420/settings#backups"), "Open Settings")
        XCTAssertNotNil(link("vnc://mac.tail1.ts.net"), "Screen Sharing")
        XCTAssertNotNil(link("vnc://MAC.tail1.ts.net"))
        XCTAssertNotNil(link("http://127.0.0.1:7420/settings#push"), "the connected Loft's own host")
        XCTAssertNotNil(link("https://github.com/o/r/pull/7"), "what any server link may open")
        XCTAssertNil(link("http://mac.tail1.ts.net.evil.example/settings"), "look-alike host")
        XCTAssertNil(link("vnc://mac.tail1.ts.net.evil.example"), "look-alike host")
        XCTAssertNil(link("http://evil.example/settings"), "foreign host")
        XCTAssertNil(link("vnc://evil.example"), "foreign host")
        XCTAssertNil(link("http://github.com/o/r"), "http only on the Loft's hosts")
        XCTAssertNil(link("ssh://mac.tail1.ts.net"), "other schemes")
        XCTAssertNil(link("file:///etc/passwd"))
        XCTAssertNil(link("javascript:alert(1)"))
        XCTAssertNil(PushDestination.needsYouLink("vnc://mac.tail1.ts.net", loftHost: nil, tailnetHost: nil), "no known hosts")
    }

    /// A Mac notification's userInfo (MacAlerts: `needs_you_id` plus `NeedsYouAlerts.target`) routes back to its item.
    func testAMacNotificationOpensItsItem() {
        func agent(_ id: String) -> NeedsYouItem.AgentRef { .init(id: id, name: "Agent") }
        func item(_ type: String, agent a: String?, open: NeedsYouAction.Target? = nil) -> NeedsYouItem {
            NeedsYouItem(id: "\(type):1", type: type, urgency: 2, agent: a.map(agent), taskId: nil, title: "t", reason: "", text: "",
                         createdAt: nil, ageSeconds: nil, links: nil,
                         actions: open.map { [NeedsYouAction(kind: "open", label: "Open", target: $0)] } ?? [], project: nil, pr: nil)
        }
        func info(_ i: NeedsYouItem) -> [AnyHashable: Any] {
            var info: [String: Any] = ["needs_you_id": i.id]
            if let t = NeedsYouAlerts.target(of: i) { info["target"] = t }
            return info
        }
        let host = "mac.tail1.ts.net"
        XCTAssertEqual(PushDestination.from(info(item("question", agent: "be")), loftHost: host), .chat(agentId: "be"))
        XCTAssertEqual(PushDestination.from(info(item("permission", agent: nil, open: .init(screen: "chat", agentId: "ios"))), loftHost: host),
                       .chat(agentId: "ios"))
        let pr = "https://github.com/o/r/pull/7"
        XCTAssertEqual(PushDestination.from(info(item("pr_ready", agent: nil, open: .init(url: pr))), loftHost: host), .url(URL(string: pr)!))
        XCTAssertNil(PushDestination.from(info(item("pr_ready", agent: nil, open: .init(url: "https://evil.example/x"))), loftHost: host),
                     "an untrusted link only opens the app")
        XCTAssertEqual(PushDestination.from(info(item("pr_ready", agent: nil)), loftHost: host), .needsYou)
    }

    /// Sign in on the Mac's Settings › Push opens the relay's portal only over https on the relay's own host.
    func testThePortalLinkOpensOnlyOnTheRelay() {
        let relay = "https://relay.example.com"
        XCTAssertEqual(PushDestination.portal("https://relay.example.com/connect?code=1", relay: relay), URL(string: "https://relay.example.com/connect?code=1"))
        XCTAssertEqual(PushDestination.portal("https://Relay.Example.com/c", relay: relay), URL(string: "https://Relay.Example.com/c"), "hosts have no case")
        for link in ["http://relay.example.com/connect", "https://evil.example/connect", "https://relay.example.com.evil.example/c",
                     "javascript:alert(1)", "file:///etc/passwd", "relay.example.com/connect", ""] {
            XCTAssertNil(PushDestination.portal(link, relay: relay), link)
        }
        XCTAssertNil(PushDestination.portal(nil, relay: relay))
        XCTAssertNil(PushDestination.portal("https://relay.example.com/c", relay: ""), "no relay set up: nothing opens")
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
