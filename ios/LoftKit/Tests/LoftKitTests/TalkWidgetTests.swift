import AppKit
import SwiftUI
import XCTest
@testable import LoftKit

/// The Talk widget and control (spec #462): the link they open, who Talk starts with, and the widget drawn on the Mac.
/// `WIDGET_SHOTS=<folder>` also writes each drawing there as a PNG, for screenshots.
final class TalkWidgetTests: XCTestCase {
    func testTheTalkLinkRoundTrips() {
        XCTAssertEqual(DeepLink.talk.url.absoluteString, "loft://talk")
        XCTAssertEqual(DeepLink(url: DeepLink.talk.url), .talk)
        XCTAssertEqual(DeepLink(url: URL(string: "loft://talk/")!), .talk)
        XCTAssertNil(DeepLink(url: URL(string: "https://example.com/talk")!))
    }

    /// Any app or web page can open `loft://talk`: only the widget's and the control's signed link starts the mic.
    func testOnlyALinkSignedWithTheKeyStartsListening() {
        let key = "0f1e2d"
        let signed = TalkKey.url(key: key)
        XCTAssertEqual(DeepLink(url: signed), .talk, "a signed link is still the Talk link")
        XCTAssertTrue(TalkKey.signs(signed, key: key))
        XCTAssertFalse(TalkKey.signs(DeepLink.talk.url, key: key), "a bare link from another app")
        XCTAssertFalse(TalkKey.signs(URL(string: "loft://talk?key=guess")!, key: key))
        XCTAssertFalse(TalkKey.signs(URL(string: "loft://talk?key=")!, key: key))
        XCTAssertFalse(TalkKey.signs(URL(string: "loft://agents?key=\(key)")!, key: key), "only Talk links")
        XCTAssertFalse(TalkKey.signs(signed, key: nil), "no key (no shared Keychain): nothing starts the mic")
        XCTAssertFalse(TalkKey.signs(URL(string: "loft://talk?key=")!, key: ""))
        XCTAssertEqual(TalkKey.url(key: nil), DeepLink.talk.url)
    }

    func testTalkStartsWithTheLastAgentElseTheOrchestratorElseAsks() async throws {
        let agents = try await FixtureAPI().agents()
        XCTAssertEqual(TalkRecipients.starter(lastAgentId: "qa", orchestratorId: "orch", agents: agents), "qa")
        XCTAssertEqual(TalkRecipients.starter(lastAgentId: nil, orchestratorId: "orch", agents: agents), "orch")
        XCTAssertEqual(TalkRecipients.starter(lastAgentId: "deleted", orchestratorId: "orch", agents: agents), "orch", "a deleted last agent")
        XCTAssertNil(TalkRecipients.starter(lastAgentId: "deleted", orchestratorId: nil, agents: agents))
        XCTAssertNil(TalkRecipients.starter(lastAgentId: "qa", orchestratorId: "orch", agents: []), "no agents loaded yet")
    }

    /// What the app does with a link: the signed one starts Talk or asks who; an unsigned one only opens.
    func testTheLinkRoutesToStartPickOrOpen() async throws {
        let agents = try await FixtureAPI().agents()
        let key = "0f1e2d"
        let signed = TalkKey.url(key: key)
        func route(_ url: URL, last: String? = "qa", orch: String? = "orch") -> TalkLink.Route? {
            TalkLink.route(url, key: key, lastAgentId: last, orchestratorId: orch, agents: agents)
        }
        XCTAssertEqual(route(signed), .start("qa"))
        XCTAssertEqual(route(signed, last: nil), .start("orch"))
        XCTAssertEqual(route(signed, last: "deleted", orch: nil), .pick, "no one to start with")
        XCTAssertEqual(route(DeepLink.talk.url), .open(.talk), "unsigned: mic off")
        XCTAssertEqual(route(URL(string: "loft://talk?key=guess")!), .open(.talk))
        XCTAssertEqual(TalkLink.route(signed, key: nil, lastAgentId: "qa", orchestratorId: nil, agents: agents), .open(.talk), "no shared Keychain")
        XCTAssertEqual(route(DeepLink.agents.url), .open(.agents))
        XCTAssertNil(route(URL(string: "https://example.com/talk")!))
    }

    @MainActor
    func testTheWidgetDrawsSmallAndRound() throws {
        var seen: [Data] = []
        for (name, round, side) in [("small", false, 158.0), ("round", true, 72.0)] {
            let view = TalkWidgetView(round: round).padding(round ? 0 : 16).frame(width: side, height: side)
                .background(Color(white: 0.11)).environment(\.colorScheme, .dark)
            let renderer = ImageRenderer(content: view)
            renderer.scale = 2
            let tiff = try XCTUnwrap(renderer.nsImage?.tiffRepresentation, name)
            let png = try XCTUnwrap(NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]))
            seen.append(png)
            if let dir = ProcessInfo.processInfo.environment["WIDGET_SHOTS"] {
                try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("mac-talk-\(name).png"))
            }
        }
        XCTAssertEqual(Set(seen).count, seen.count)
    }
}
