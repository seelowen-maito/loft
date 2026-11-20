import SwiftUI
import XCTest
@testable import Loft
@testable import LoftKit

/// The Delivery progress widget drawn on the iPhone (spec #462; its view model is tested in LoftKitTests/DeliveryWidgetTests).
/// `WIDGET_SHOTS=<folder>` also writes each drawing there as a PNG, for screenshots.
final class DeliveryWidgetRenderTests: XCTestCase {
    @MainActor
    func testEveryStateDrawsAndTheStatesLookDifferent() throws {
        func t(_ n: Int, _ state: String, agent: String? = nil, verdicts: [String: String]? = nil) -> DeliverySummary.Ticket {
            .init(number: n, title: "Ticket \(n)", state: state, agentName: agent, verdicts: verdicts)
        }
        func ready(_ state: String, _ tickets: [DeliverySummary.Ticket]) -> DeliveryWidgetSnapshot {
            let d = DeliverySummary(state: state, agentName: nil, repo: "o/loft", spec: 462, specId: nil, local: false, pr: 463, current: nil, tickets: tickets)
            return DeliveryWidgetSnapshot(key: "o/loft#462", title: "#462 Widgets", deliveries: [d], tasks: [])
        }
        let states: [(String, DeliveryWidgetSnapshot)] = [
            ("running", ready("running", [t(464, "closed"), t(465, "review", agent: "Loft Dev", verdicts: ["qa": "pass", "security": "pending"]),
                                          t(466, "started", agent: "Loft Dev"), t(467, "open")])),
            ("changes", ready("running", [t(464, "closed"), t(465, "changes", agent: "Loft Dev", verdicts: [:])])),
            ("stalled", ready("stalled", [t(464, "closed"), t(465, "open")])),
            ("needs-you", { var s = ready("running", [t(464, "needs_human"), t(465, "started", agent: "Loft Dev")]); s.waitingOnYou = "#464 Ticket 464"; s.status = .needsYou; return s }()),
            ("done", ready("ready", [t(464, "closed"), t(465, "closed")])),
            ("not-chosen", DeliveryWidgetSnapshot(phase: .notChosen)),
        ]
        var seen: [Data] = []
        for (name, snapshot) in states {
            let view = DeliveryWidgetView(snapshot).padding(16).frame(width: 364, height: 170)
                .background(Color(white: 0.11)).environment(\.colorScheme, .dark)
            let renderer = ImageRenderer(content: view)
            renderer.scale = 3
            let png = try XCTUnwrap(renderer.uiImage?.pngData(), name)
            seen.append(png)
            if let dir = ProcessInfo.processInfo.environment["WIDGET_SHOTS"] {
                try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("ios-delivery-\(name).png"))
            }
        }
        XCTAssertEqual(Set(seen).count, seen.count, "no two states draw the same")
    }
}
