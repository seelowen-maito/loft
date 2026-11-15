import CoreSpotlight
import ImageIO
import XCTest
@testable import LoftKit

/// Loft in macOS Spotlight: the items built from the store, how a chosen one is read back, and what a change re-indexes.
final class SpotlightIndexTests: XCTestCase {
    private static let sticker = ##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="#3a7"/></svg>"##

    private func agent(_ id: String, _ name: String, project: String? = "loft", role: String? = "dev", svg: String? = sticker) -> Agent {
        var a = Agent(id: id, name: name, status: "idle")
        a.project = project
        a.role = role
        a.avatar = svg.map(Avatar.init(svg:))
        return a
    }

    private func task(_ id: String, _ title: String, number: Int? = nil, labels: [String] = [], parent: String? = nil,
                      status: String = "ready") -> HBTask {
        var t = HBTask(id: id, title: title, description: "", kind: "work", status: status, priority: "normal", project: "loft", agentId: nil,
                       parentId: parent, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "", createdAt: Date(),
                       updatedAt: Date())
        t.labels = labels
        t.github = number.map { .init(repo: "o/loft", number: $0, url: "", state: "open") }
        return t
    }

    func testAgentsAndOpenIssuesTicketsAndSpecsAreIndexedNotClosedOnes() {
        let tasks = [task("s", "Mac app web look", number: 456, labels: ["spec"]), task("t", "Loft in macOS Spotlight", number: 460, parent: "s"),
                     task("i", "Fix the badge"), task("c", "Old work", status: "done")]
        let e = SpotlightIndex.entries(agents: [agent("dev", "Loft mobile dev")], tasks: tasks, deliveries: [])
        XCTAssertEqual(e.map(\.id), ["agent:dev", "spec:s", "ticket:t", "issue:i"])
    }

    func testATicketCarriesTitleKindNumberAndItsIdentifier() {
        let e = SpotlightIndex.entries(agents: [], tasks: [task("t", "Loft in macOS Spotlight", number: 460, parent: "s"),
                                                           task("s", "Spec", labels: ["spec"])], deliveries: [])[0]
        let item = SpotlightIndex.item(e)
        XCTAssertEqual(item.uniqueIdentifier, "ticket:t")
        XCTAssertEqual(item.domainIdentifier, "loft")
        XCTAssertEqual(item.attributeSet.title, "Loft in macOS Spotlight")
        XCTAssertEqual(item.attributeSet.contentDescription, "Ticket · loft#460")
        XCTAssertEqual(item.attributeSet.keywords, ["Ticket", "460", "#460"])
        XCTAssertNil(item.attributeSet.thumbnailData)
    }

    func testAnAgentReadsProjectAndRoleWithItsStickerAsTheImage() throws {
        let item = SpotlightIndex.item(SpotlightIndex.entries(agents: [agent("dev", "Loft mobile dev")], tasks: [], deliveries: [])[0])
        XCTAssertEqual(item.attributeSet.title, "Loft mobile dev")
        XCTAssertEqual(item.attributeSet.contentDescription, "loft · Dev")
        XCTAssertEqual(item.attributeSet.keywords, ["Agent"])
        let png = try XCTUnwrap(item.attributeSet.thumbnailData)
        let image = try XCTUnwrap(CGImageSourceCreateWithData(png as CFData, nil).flatMap { CGImageSourceCreateImageAtIndex($0, 0, nil) })
        XCTAssertEqual(image.width, 128)
        XCTAssertEqual(image.height, 128)
    }

    func testAnAgentWithoutAProjectStickerOrRole() {
        let item = SpotlightIndex.item(SpotlightIndex.entries(agents: [agent("o", "Orchestrator", project: nil, role: nil, svg: nil)],
                                                              tasks: [], deliveries: [])[0])
        XCTAssertEqual(item.attributeSet.contentDescription, "Custom")
        XCTAssertNil(item.attributeSet.thumbnailData)
    }

    func testAStickerIsDrawnWithItsBorderAndUnreadableOnesAreNot() throws {
        let png = try XCTUnwrap(Sticker.png(svg: Self.sticker, pixels: 64))
        let image = try XCTUnwrap(CGImageSourceCreateWithData(png as CFData, nil).flatMap { CGImageSourceCreateImageAtIndex($0, 0, nil) })
        // The centre is the art, a corner stays clear.
        XCTAssertGreaterThan(alpha(image, x: 32, y: 32), 250)
        XCTAssertEqual(alpha(image, x: 0, y: 0), 0)
        XCTAssertNotNil(Sticker.png(svg: Self.sticker, pixels: 64, lead: true))
        XCTAssertNil(Sticker.png(svg: "not svg", pixels: 64))
        XCTAssertNil(Sticker.png(svg: "<svg/>", pixels: 64))
    }

    private func alpha(_ image: CGImage, x: Int, y: Int) -> UInt8 {
        var px = [UInt8](repeating: 0, count: 4)
        let ctx = CGContext(data: &px, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        ctx.draw(image, in: CGRect(x: -x, y: -(image.height - 1 - y), width: image.width, height: image.height))
        return px[3]
    }

    func testAChosenItemNamesWhatToOpen() {
        XCTAssertEqual(SpotlightIndex.target(of: "agent:dev")?.kind, .agent)
        XCTAssertEqual(SpotlightIndex.target(of: "agent:dev")?.id, "dev")
        XCTAssertEqual(SpotlightIndex.target(of: "issue:a:b")?.id, "a:b")
        XCTAssertNil(SpotlightIndex.target(of: "page:/repos"))
        XCTAssertNil(SpotlightIndex.target(of: "agent:"))
        XCTAssertNil(SpotlightIndex.target(of: "nonsense"))
    }

    func testAChosenItemOpensOnTheWebOnlyWhenSettingsSaysBrowser() {
        let base = URL(string: "https://loft.example:8080")!
        XCTAssertEqual(SpotlightIndex.webURL(of: "agent:dev", opensIn: "browser", base: base)?.absoluteString,
                       "https://loft.example:8080/agents/dev")
        for kind in ["issue", "ticket", "spec"] {
            XCTAssertEqual(SpotlightIndex.webURL(of: "\(kind):42", opensIn: "browser", base: base)?.absoluteString,
                           "https://loft.example:8080/issues/42")
        }
        XCTAssertNil(SpotlightIndex.webURL(of: "agent:dev", opensIn: "app", base: base), "Mac app opens it here")
        XCTAssertNil(SpotlightIndex.webURL(of: "agent:dev", opensIn: nil, base: base), "the default is the Mac app")
        XCTAssertNil(SpotlightIndex.webURL(of: "agent:dev", opensIn: "browser", base: nil), "no web Loft to open")
        XCTAssertNil(SpotlightIndex.webURL(of: "nonsense", opensIn: "browser", base: base))
    }

    func testNewRenamedClosedAndRestyledItemsChangeTheIndex() {
        let dev = agent("dev", "Loft mobile dev")
        var moved = dev
        moved.role = "qa"
        let before = SpotlightIndex.entries(agents: [dev], tasks: [task("a", "One"), task("b", "Two")], deliveries: [])
        let after = SpotlightIndex.entries(agents: [moved], tasks: [task("a", "One, renamed"), task("b", "Two", status: "done"), task("c", "Three")],
                                           deliveries: [])
        let change = SpotlightIndex.changes(from: before, to: after)
        XCTAssertEqual(change.index.map(\.id), ["agent:dev", "issue:a", "issue:c"])
        XCTAssertEqual(change.remove, ["issue:b"])
        XCTAssertTrue(SpotlightIndex.changes(from: after, to: after).index.isEmpty)
    }
}
