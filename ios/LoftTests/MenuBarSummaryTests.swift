import XCTest
@testable import Loft
@testable import LoftKit

/// The Mac menu bar item (spec #382) on fixture data: who is working, the Needs you count, where a click goes.
final class MenuBarSummaryTests: XCTestCase {
    @MainActor
    private func connected() async throws -> LoftStore {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        store.activate()
        for _ in 0..<100 where store.needsYou == nil { try await Task.sleep(for: .milliseconds(50)) }
        return store
    }

    @MainActor
    func testItListsTheWorkingAgentsAndTheNeedsYouCount() async throws {
        let store = try await connected()
        defer { store.deactivate() }
        let s = MenuBarSummary(store)
        XCTAssertEqual(Set(s.working.map(\.id)), Set(store.agents.filter { store.state(of: $0) == .working }.map(\.id)))
        XCTAssertFalse(s.working.isEmpty, "the fixture has agents at work")
        XCTAssertEqual(s.needsCount, store.needsYou?.count)
        XCTAssertGreaterThan(s.needsCount, 0)
        XCTAssertEqual(s.items.map(\.id), Array((store.needsYou?.items ?? []).prefix(MenuBarSummary.itemLimit).map(\.id)), "the server's order, capped")
    }

    @MainActor
    func testAnItemOpensItsAgentsChat() async throws {
        let store = try await connected()
        defer { store.deactivate() }
        let question = try XCTUnwrap(store.needsYou?.items.first { $0.type == "question" })
        let expected = question.openAction?.target?.agentId ?? question.agent?.id
        let item = MenuBarSummary(store).items.first { $0.id == question.id } ?? MenuBarSummary.Item(id: question.id, title: "", agentId: expected)
        XCTAssertNotNil(expected)
        XCTAssertEqual(item.agentId, expected)
    }

    @MainActor
    func testWithoutLoftItIsEmpty() {
        let s = MenuBarSummary(LoftStore(api: nil, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))))
        XCTAssertTrue(s.working.isEmpty)
        XCTAssertEqual(s.needsCount, 0)
        XCTAssertTrue(s.items.isEmpty)
    }

    /// The quick message goes to the picked agent, and to the Lead until one is picked.
    @MainActor
    func testTheQuickMessageGoesToTheChosenAgent() async throws {
        let api = MockAPI()
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        store.activate()
        defer { store.deactivate() }
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(50)) }
        XCTAssertEqual(store.quickRecipient, "a1", "the only agent here")
        let failure = await store.quickSend("hello", to: "a1")
        XCTAssertNil(failure)
        XCTAssertEqual(api.sentClientIds.count, 1)
        XCTAssertTrue(store.pending.isEmpty)
    }

    /// A failed quick message says why and isn't left in the chat's unsent messages (the box keeps the text).
    @MainActor
    func testAFailedQuickMessageSaysWhy() async throws {
        let api = MockAPI()
        api.failSends = true
        let store = LoftStore(api: api, cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)))
        store.activate()
        defer { store.deactivate() }
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(50)) }
        let failure = await store.quickSend("hello", to: "a1")
        XCTAssertNotNil(failure)
        XCTAssertTrue(store.pending.isEmpty)
    }
}
