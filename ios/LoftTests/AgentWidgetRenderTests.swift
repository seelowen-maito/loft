import SwiftUI
import XCTest
@testable import Loft
@testable import LoftKit

/// The One agent widget drawn on the iPhone (spec #462; its view model is tested in LoftKitTests/AgentWidgetTests).
/// `WIDGET_SHOTS=<folder>` also writes each drawing there as a PNG, for screenshots.
final class AgentWidgetRenderTests: XCTestCase {
    private func load(_ id: String) async -> AgentWidgetSnapshot {
        let api = FixtureAPI()
        return await AgentWidgetSnapshot.load(agentId: id, name: "", agents: api.agents, tasks: api.tasks,
                                              messages: { try await api.messages(agentId: id, limit: 50, before: nil) }, permissions: api.permissions)
    }

    @MainActor
    func testEveryStateDrawsAtBothSizesAndTheStatesLookDifferent() async throws {
        var longName = await load("orch")
        longName.name = "Dineout Marketplace Frontend Dev"
        let states: [(String, AgentWidgetSnapshot)] = [
            ("working", await load("orch")), ("idle", await load("qa")), ("approval", await load("ios")),
            ("gone", AgentWidgetSnapshot(phase: .gone, agentId: "qa", name: "QA Agent")), ("not-chosen", AgentWidgetSnapshot(phase: .notChosen)),
            ("long-name", longName), ("offline", AgentWidgetSnapshot(phase: .unreachable, agentId: "qa", name: "QA Agent")),
        ]
        var seen: [Data] = []
        for (name, snapshot) in states {
            for (size, points) in [(AgentWidgetView.Size.small, CGSize(width: 170, height: 170)), (.medium, CGSize(width: 364, height: 170))] {
                let view = AgentWidgetView(snapshot, size: size).padding(16).frame(width: points.width, height: points.height)
                    .background(Color(white: 0.11)).environment(\.colorScheme, .dark).environment(\.widgetLinksTap, false)
                let renderer = ImageRenderer(content: view)
                renderer.scale = 3
                let png = try XCTUnwrap(renderer.uiImage?.pngData(), "\(name) \(size)")
                seen.append(png)
                if let dir = ProcessInfo.processInfo.environment["WIDGET_SHOTS"] {
                    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                    try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("ios-agent-\(name)-\(size).png"))
                }
            }
        }
        XCTAssertEqual(Set(seen).count, seen.count, "no two states or sizes draw the same")
    }
}
