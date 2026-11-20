import AppKit
import SwiftUI
import XCTest
@testable import LoftKit

/// The One agent widget (spec #462): the agents it offers, what it shows for one, and its states drawn on the Mac.
/// `WIDGET_SHOTS=<folder>` also writes each drawing there as a PNG, for screenshots.
final class AgentWidgetTests: XCTestCase {
    private func load(_ id: String, name: String = "", _ api: FixtureAPI = FixtureAPI()) async -> AgentWidgetSnapshot {
        await AgentWidgetSnapshot.load(agentId: id, name: name, agents: api.agents, tasks: api.tasks,
                                       messages: { try await api.messages(agentId: id, limit: 50, before: nil) }, permissions: api.permissions)
    }

    func testThePickerOffersEveryAgentByName() async throws {
        let agents = try await FixtureAPI().agents()
        let offered = AgentEntityQuery.entities(agents)
        XCTAssertEqual(Set(offered.map(\.id)), Set(agents.map(\.id)))
        XCTAssertEqual(offered.map(\.name), ["Backend Agent", "Docs Agent", "iOS Agent", "Orchestrator", "QA Agent"])
    }

    func testAWorkingAgentShowsItsTicketItsOwnLastWordsAndALiveTimer() async throws {
        let api = FixtureAPI()
        let s = await load("orch", api)
        let agents = try await api.agents()
        let agent = try XCTUnwrap(agents.first { $0.id == "orch" })
        XCTAssertEqual(s.phase, .ready)
        XCTAssertEqual(s.name, "Orchestrator")
        XCTAssertEqual(s.state, .working)
        XCTAssertEqual(s.ticket, "#42 Implement settings functionality")
        XCTAssertEqual(s.lastMessage, "Delegated: Backend Agent has the endpoints, iOS Agent the screen, and QA Agent tests once both land.",
                       "its own reply, not QA's mention reply copied into its chat, and without the bold marks")
        XCTAssertEqual(s.runStart, agent.lastRunAt)
        XCTAssertNil(s.lastRun)
        XCTAssertNotNil(s.avatarSVG)
    }

    func testAnIdleAgentShowsHowLongItsLastRunTook() async throws {
        let s = await load("qa")
        XCTAssertEqual(s.state, .idle)
        XCTAssertNil(s.runStart)
        XCTAssertNil(s.ticket)
        XCTAssertEqual(s.lastMessage, "All 14 smoke tests passed on staging.")
        XCTAssertTrue(s.lastRun?.hasPrefix("Done in 2m 23s · ") == true, s.lastRun ?? "nil")
    }

    func testAPendingPermissionReadsAsNeedsApproval() async {
        let s = await load("ios")
        XCTAssertEqual(s.state, .waitingApproval)
    }

    func testAMirroredTicketShowsItsNumberAndTheLatestOneWins() async throws {
        var agent = try await FixtureAPI().agents()[0]
        agent = Agent(id: "dev", name: "Dev", project: nil, model: nil, effort: nil, permissionMode: nil, intervalMinutes: nil, status: "running",
                      statusText: nil, running: true, lastRunAt: nil, nextRunAt: nil, createdAt: nil, totalCostUsd: nil, contextTokens: nil,
                      prompt: nil, avatar: agent.avatar)
        func task(_ id: String, _ status: String, number: Int?, updated: TimeInterval) -> HBTask {
            var t = HBTask(id: id, title: "Ticket \(id)", description: "", kind: "work", status: status, priority: "medium", project: "loft",
                           agentId: "dev", parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "",
                           createdAt: .now, updatedAt: .now.addingTimeInterval(updated), startedAt: nil, completedAt: nil)
            t.github = number.map { .init(repo: "o/loft", number: $0, url: "", state: "open") }
            return t
        }
        let tasks = [task("a", "in_progress", number: 464, updated: -60), task("b", "in_progress", number: 465, updated: 0),
                     task("c", "done", number: 466, updated: 60)]
        let s = AgentWidgetSnapshot(agentId: "dev", name: "", agents: [agent], tasks: tasks, messages: [], permissions: [])
        XCTAssertEqual(s.ticket, "#465 Ticket b")
        XCTAssertNil(s.lastMessage)
    }

    func testALongMessageIsCutToOneLine() {
        let line = AgentWidgetSnapshot.oneLine("First line\n\n" + String(repeating: "word ", count: 60))
        XCTAssertFalse(line.contains("\n"))
        XCTAssertEqual(line.count, AgentWidgetSnapshot.messageLimit)
        XCTAssertTrue(line.hasPrefix("First line word"))
        XCTAssertTrue(line.hasSuffix("…"))
    }

    func testADeletedAgentIsGoneAndKeepsTheNameItHad() async throws {
        let api = FixtureAPI()
        try await api.deleteAgent(id: "qa", purgeFiles: false)
        let s = await load("qa", name: "QA Agent", api)
        XCTAssertEqual(s.phase, .gone)
        XCTAssertEqual(s.name, "QA Agent")
        XCTAssertEqual(s.link, .agents)
    }

    func testADeletedAgentKeepsTheNameTheWidgetLastShowed() async throws {
        let names = try XCTUnwrap(UserDefaults(suiteName: "AgentWidgetTests"))
        names.removePersistentDomain(forName: "AgentWidgetTests")
        let api = FixtureAPI()
        func load() async -> AgentWidgetSnapshot {
            await AgentWidgetSnapshot.load(agentId: "qa", name: "", agents: api.agents, tasks: api.tasks,
                                           messages: { try await api.messages(agentId: "qa", limit: 50, before: nil) },
                                           permissions: api.permissions, names: names)
        }
        _ = await load()
        try await api.deleteAgent(id: "qa", purgeFiles: false)
        let s = await load()
        XCTAssertEqual(s.phase, .gone)
        XCTAssertEqual(s.name, "QA Agent", "the picker gives a deleted agent no name; the widget remembers it")
    }

    func testOfflineIsUnreachableAndTheExtrasFailingLeaveTheirLinesOut() async {
        let offline = await load("orch", FixtureAPI(mode: .offline))
        XCTAssertEqual(offline.phase, .unreachable)
        let api = FixtureAPI()
        struct Down: Error {}
        let s = await AgentWidgetSnapshot.load(agentId: "orch", name: "", agents: api.agents, tasks: { throw Down() },
                                               messages: { throw Down() }, permissions: { throw Down() })
        XCTAssertEqual(s.phase, .ready)
        XCTAssertNil(s.ticket)
        XCTAssertNil(s.lastMessage)
    }

    func testATapOpensTheAgentsChat() async {
        let s = await load("orch")
        XCTAssertEqual(s.link, .agent("orch"))
        XCTAssertEqual(DeepLink(url: DeepLink.agent("orch").url), .agent("orch"))
    }

    @MainActor
    func testEveryStateDrawsAtBothSizesAndTheStatesLookDifferent() async throws {
        var noSticker = await load("qa")
        noSticker.avatarSVG = nil
        var longName = await load("orch")
        longName.name = "Dineout Marketplace Frontend Dev"
        let states: [(String, AgentWidgetSnapshot)] = [
            ("working", await load("orch")), ("idle", noSticker), ("approval", await load("ios")),
            ("gone", AgentWidgetSnapshot(phase: .gone, agentId: "qa", name: "QA Agent")), ("not-chosen", AgentWidgetSnapshot(phase: .notChosen)),
            ("unreachable", AgentWidgetSnapshot(phase: .unreachable)), ("long-name", longName), ("offline", AgentWidgetSnapshot(phase: .unreachable, agentId: "qa", name: "QA Agent")),
        ]
        var seen: [Data] = []
        for (name, snapshot) in states {
            for (size, points) in [(AgentWidgetView.Size.small, CGSize(width: 158, height: 158)), (.medium, CGSize(width: 338, height: 158))] {
                let view = AgentWidgetView(snapshot, size: size).padding(16).frame(width: points.width, height: points.height)
                    .background(Color(white: 0.11)).environment(\.colorScheme, .dark).environment(\.widgetLinksTap, false)
                let renderer = ImageRenderer(content: view)
                renderer.scale = 2
                let tiff = try XCTUnwrap(renderer.nsImage?.tiffRepresentation, "\(name) \(size)")
                let png = try XCTUnwrap(NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]))
                seen.append(png)
                if let dir = ProcessInfo.processInfo.environment["WIDGET_SHOTS"] {
                    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                    try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("mac-agent-\(name)-\(size).png"))
                }
            }
        }
        XCTAssertEqual(Set(seen).count, seen.count, "no two states or sizes draw the same")
    }
}
