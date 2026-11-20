import SwiftUI
import XCTest
@testable import Loft
@testable import LoftKit

/// The widgets (spec #389) on fixture data: what they show, where a tap goes, and that each state draws at both sizes.
/// `WIDGET_SHOTS=<folder>` also writes each drawing there as a PNG, for screenshots.
final class WidgetSnapshotTests: XCTestCase {
    private func fixture(spend: Double? = 12.4) async throws -> WidgetSnapshot {
        let api = FixtureAPI()
        return WidgetSnapshot(needsYou: try await api.needsYou(), agents: try await api.agents(), weekSpend: spend)
    }

    func testItShowsTheNeedsYouCountItsFirstItemsAndTheAgentsAtWork() async throws {
        let list = try await FixtureAPI().needsYou()
        let agents = try await FixtureAPI().agents()
        let s = try await fixture()
        XCTAssertEqual(s.phase, .ready)
        XCTAssertEqual(s.needsCount, list.count)
        XCTAssertGreaterThan(s.needsCount, 0)
        XCTAssertEqual(s.items.map(\.id), Array(list.items.prefix(WidgetSnapshot.itemLimit).map(\.id)), "the server's order, capped")
        XCTAssertEqual(Set(s.working), Set(agents.filter { AgentState(agent: $0, pendingApprovals: 0) == .working }.map(\.name)))
        XCTAssertFalse(s.working.isEmpty, "the fixture has agents at work")
        XCTAssertEqual(s.spendText, "$12.40")
    }

    func testAnItemOpensItsAgentsChatAndTheCountOpensNeedsYou() async throws {
        let s = try await fixture()
        let first = try XCTUnwrap(s.items.first)
        let item = try await FixtureAPI().needsYou().items[0]
        XCTAssertEqual(first.link, .agent(try XCTUnwrap(item.openAction?.target?.agentId ?? item.agent?.id)))
        XCTAssertEqual(DeepLink(url: DeepLink.needsYou.url), .needsYou)
        XCTAssertEqual(DeepLink(url: DeepLink.agents.url), .agents)
        XCTAssertEqual(DeepLink(url: DeepLink.agent("a1").url), .agent("a1"))
        XCTAssertNil(DeepLink(url: URL(string: "loft://agent")!))
        XCTAssertNil(DeepLink(url: URL(string: "https://example.com/needs-you")!))
    }

    /// The Talk widget and control with no one to talk to yet: the Overview's top, with Talk asking who.
    @MainActor
    func testTalkWithNoAgentOpensTheOverview() {
        XCTAssertEqual(DeepLink(url: DeepLink.talk.url), .talk)
        let r = Router()
        r.openChat("be")
        r.open(.needsYou)
        r.openLink(.talk)
        XCTAssertEqual(r.tab, .overview)
        XCTAssertTrue(r.overviewPath.isEmpty)
        XCTAssertTrue(r.picksTalk, "Talk asks who, instead of a plain Overview")
    }

    /// Like the menu bar: an agent waiting on a permission needs the user, so it isn't listed as working.
    func testAnAgentWaitingOnAPermissionIsNotWorking() async throws {
        let api = FixtureAPI()
        let agents = try await api.agents()
        let busy = try XCTUnwrap(agents.first { AgentState(agent: $0, pendingApprovals: 0) == .working })
        let ask = PermissionRequest(id: "w1", agentId: busy.id, source: "prompt", toolName: "Bash", input: nil, reason: nil, rule: nil,
                                    status: "pending", createdAt: Date())
        let needs = try await api.needsYou()
        XCTAssertFalse(WidgetSnapshot(needsYou: needs, agents: agents, permissions: [ask], weekSpend: nil).working.contains(busy.name))
        var answered = ask
        answered.status = "allowed_once"
        XCTAssertTrue(WidgetSnapshot(needsYou: needs, agents: agents, permissions: [answered], weekSpend: nil).working.contains(busy.name))
    }

    /// Without permissions (the default), every busy agent counts as working.
    func testNoPermissionsLeavesEveryBusyAgentWorking() async throws {
        let api = FixtureAPI()
        let agents = try await api.agents()
        let busy = agents.filter { AgentState(agent: $0, pendingApprovals: 0) == .working }.map(\.name)
        let needs = try await api.needsYou()
        XCTAssertEqual(Set(WidgetSnapshot(needsYou: needs, agents: agents, weekSpend: nil).working), Set(busy))
    }

    /// A failed permissions or spend read leaves them out; only Needs you or the agents failing means unreachable.
    func testOnlyNeedsYouOrTheAgentsFailingMakesLoftUnreachable() async throws {
        let api = FixtureAPI()
        let down: () async throws -> [PermissionRequest] = { throw APIError.server(status: 404, message: "Not Found") }
        let noSpend: () async throws -> Double = { throw APIError.unreachable("timeout") }
        let s = await WidgetSnapshot.load(needsYou: api.needsYou, agents: api.agents, permissions: down, weekSpend: noSpend)
        XCTAssertEqual(s.phase, .ready)
        let count = try await api.needsYou().count
        XCTAssertEqual(s.needsCount, count)
        XCTAssertFalse(s.working.isEmpty, "no permissions: the busy agents still show as working")
        XCTAssertNil(s.weekSpend)
        let noAgents: () async throws -> [Agent] = { throw APIError.unreachable("timeout") }
        let gone = await WidgetSnapshot.load(needsYou: api.needsYou, agents: noAgents, permissions: api.permissions, weekSpend: { 12.4 })
        XCTAssertEqual(gone.phase, WidgetSnapshot.Phase.unreachable)
    }

    func testUnknownSpendIsLeftOut() async throws {
        let s = try await fixture(spend: nil)
        XCTAssertNil(s.spendText)
    }

    @MainActor
    func testEveryStateDrawsAtBothSizesAndTheStatesLookDifferent() async throws {
        var quiet = try await fixture()
        quiet.needsCount = 0
        quiet.items = []
        quiet.working = []
        let states: [(String, WidgetSnapshot)] = [("ready", try await fixture()), ("quiet", quiet), ("not-connected", .notConnected), ("unreachable", .unreachable)]
        var seen: [Data] = []
        for (name, snapshot) in states {
            for (size, points) in [(LoftWidgetView.Size.small, CGSize(width: 158, height: 158)), (.medium, CGSize(width: 338, height: 158))] {
                let view = LoftWidgetView(snapshot, size: size).padding(16).frame(width: points.width, height: points.height)
                    .background(Color(white: 0.11)).environment(\.colorScheme, .dark).environment(\.widgetLinksTap, false)
                let renderer = ImageRenderer(content: view)
                renderer.scale = 2
                let png = try XCTUnwrap(renderer.uiImage?.pngData(), "\(name) \(size)")
                seen.append(png)
                if let dir = ProcessInfo.processInfo.environment["WIDGET_SHOTS"] {
                    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                    try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("ios-\(name)-\(size).png"))
                }
            }
        }
        XCTAssertEqual(Set(seen).count, seen.count, "no two states or sizes draw the same")
    }
}
