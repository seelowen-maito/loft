import SwiftUI
import XCTest
@testable import Loft
@testable import LoftKit

/// The Needs you widget (spec #462): its items, Approve's link and Deny against a fake API, and its states drawn.
/// `WIDGET_SHOTS=<folder>` also writes each drawing there as a PNG, for screenshots.
final class NeedsYouWidgetTests: XCTestCase {
    private func load(_ api: FixtureAPI) async -> WidgetSnapshot {
        await WidgetSnapshot.load(needsYou: api.needsYou, agents: api.agents, permissions: api.permissions, weekSpend: { 12.4 })
    }

    func testItListsTheItemsInTheServersOrderWithTheAgentAndASummary() async throws {
        let api = FixtureAPI()
        let list = try await api.needsYou()
        let s = await load(api)
        XCTAssertEqual(s.items.map(\.id), Array(list.items.prefix(WidgetSnapshot.itemLimit).map(\.id)))
        let permission = try XCTUnwrap(s.items.first { $0.id == "permission:p1" })
        XCTAssertEqual(permission.agentName, "Backend Agent")
        XCTAssertEqual(permission.summary, "$ git push https://x:[redacted]@github.com/o/r",
                       "a permission's summary is the command it runs, not the agent's description of it")
        let question = try XCTUnwrap(s.items.first { list.items.first(where: { $0.type == "question" })?.id == $0.id })
        XCTAssertEqual(question.summary, "Should I use B2 or S3?")
    }

    func testOnlyPermissionsGetApproveAndDenyAndAQuestionOpensItsChat() async throws {
        let api = FixtureAPI()
        let list = try await api.needsYou()
        let s = await load(api)
        for item in s.items {
            let type = try XCTUnwrap(list.items.first { $0.id == item.id }).type
            XCTAssertEqual(item.approve, type == "permission" ? .needsYouItem(item.id) : nil, item.id)
            XCTAssertEqual(item.deny != nil, type == "permission", item.id)
        }
        let p = try XCTUnwrap(s.items.first { $0.deny != nil })
        XCTAssertEqual(p.deny?.body?["decision"], .string("deny"))
        let question = try XCTUnwrap(list.items.first { $0.type == "question" })
        XCTAssertEqual(s.items.first { $0.id == question.id }?.link, .agent(try XCTUnwrap(question.openAction?.target?.agentId)))
    }

    func testDenySendsTheItemsOwnAction() async throws {
        let loaded = await load(FixtureAPI())
        let p = try XCTUnwrap(loaded.items.first { $0.deny != nil })
        var sent: [(String, String, JSONValue?)] = []
        try await NeedsYouDecisionIntent(try XCTUnwrap(p.deny)).send { sent.append(($0, $1, $2)) }
        XCTAssertEqual(sent.map(\.0), ["POST"])
        XCTAssertEqual(sent.map(\.1), ["/api/permissions/p1"])
        XCTAssertEqual(sent.map(\.2), [p.deny?.body])
    }

    func testADeniedItemIsGoneAfterTheReload() async throws {
        let api = FixtureAPI()
        let before = await load(api)
        let p = try XCTUnwrap(before.items.first { $0.deny != nil })
        try await NeedsYouDecisionIntent(try XCTUnwrap(p.deny)).send(api.needsYouRequest)
        let after = await load(api)
        XCTAssertFalse(after.items.contains { $0.id == p.id })
        let count = try await api.needsYou().count
        XCTAssertEqual(after.needsCount, count)
    }

    /// Approve lands on the Needs you list at that request's card, where the user approves with one more tap.
    @MainActor
    func testApproveOpensNeedsYouAtTheRequest() {
        let r = Router()
        r.tab = .agents
        r.openLink(.needsYouItem("permission:p1"))
        XCTAssertEqual(r.tab, .overview)
        XCTAssertEqual(r.overviewPath, { var p = NavigationPath(); p.append(NeedsYouRoute(focus: "permission:p1")); return p }())
    }

    func testTheWidgetsReloadWhetherTheRequestWorksOrFails() async throws {
        let loaded = await load(FixtureAPI())
        let p = try XCTUnwrap(loaded.items.first { $0.deny != nil })
        let intent = NeedsYouDecisionIntent(try XCTUnwrap(p.deny))
        var reloads = 0
        try await intent.send(FixtureAPI().needsYouRequest) { reloads += 1 }
        XCTAssertEqual(reloads, 1)
        struct AnsweredElsewhere: Error {}
        do {
            try await intent.send({ _, _, _ in throw AnsweredElsewhere() }) { reloads += 1 }
            XCTFail("a failed request still throws")
        } catch {}
        XCTAssertEqual(reloads, 2, "a failed request reloads too, so a stale item drops off")
    }

    @MainActor
    func testTheAppReloadsTheWidgetsWhenNeedsYouChanges() async throws {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        var reloads = 0
        store.reloadWidgets = { reloads += 1 }
        store.activate()
        for _ in 0..<100 where store.connection != .connected || store.needsYou == nil { try await Task.sleep(for: .milliseconds(50)) }
        defer { store.deactivate() }
        let first = reloads
        XCTAssertGreaterThan(first, 0, "the first list reaches the widgets")
        await store.refreshNeedsYou()
        XCTAssertEqual(reloads, first, "an unchanged list doesn't reload them")
        let restart = try XCTUnwrap(store.needsYou?.items.first { $0.type == "stuck" }?.actions.first { $0.kind == "restart" })
        let done = try await store.perform(restart)
        XCTAssertTrue(done)
        XCTAssertEqual(reloads, first + 1, "an item answered in the app leaves the widget")
    }

    func testAFailedRequestThrowsSoTheWidgetKeepsTheItem() async throws {
        let loaded = await load(FixtureAPI())
        let p = try XCTUnwrap(loaded.items.first { $0.deny != nil })
        let offline = FixtureAPI(mode: .offline)
        do {
            try await NeedsYouDecisionIntent(try XCTUnwrap(p.deny)).send(offline.needsYouRequest)
            XCTFail("an offline Loft fails the intent")
        } catch {}
        let s = await load(offline)
        XCTAssertEqual(s.phase, .unreachable)
    }

    @MainActor
    func testEveryStateDrawsAndTheStatesLookDifferent() async throws {
        let ready = await load(FixtureAPI())
        let empty = WidgetSnapshot(needsYou: NeedsYouList(version: 1, count: 0, urgent: 0, items: []), agents: [], weekSpend: nil)
        let states: [(String, WidgetSnapshot)] = [("ready", ready), ("empty", empty), ("not-connected", .notConnected), ("unreachable", .unreachable)]
        var seen: [Data] = []
        for (name, snapshot) in states {
            for (size, points) in [(NeedsYouWidgetView.Size.medium, CGSize(width: 338, height: 158)), (.large, CGSize(width: 338, height: 354))] {
                let view = NeedsYouWidgetView(snapshot, size: size).padding(16).frame(width: points.width, height: points.height)
                    .background(Color(white: 0.11)).environment(\.colorScheme, .dark).environment(\.widgetLinksTap, false)
                let renderer = ImageRenderer(content: view)
                renderer.scale = 2
                let png = try XCTUnwrap(renderer.uiImage?.pngData(), "\(name) \(size)")
                seen.append(png)
                if let dir = ProcessInfo.processInfo.environment["WIDGET_SHOTS"] {
                    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                    try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("ios-needs-you-\(name)-\(size).png"))
                }
            }
        }
        XCTAssertEqual(Set(seen).count, seen.count, "no two states or sizes draw the same")
    }
}
