import XCTest
@testable import LoftKit

/// Replies read aloud for messages that came by voice, on fixture data and a fake reader (spec #382).
final class VoiceRepliesTests: XCTestCase {
    private var cacheURL: URL { FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString) }

    @MainActor
    private func connected() async throws -> LoftStore {
        let store = LoftStore(api: FixtureAPI(replies: true), cache: SnapshotStore(url: cacheURL))
        store.activate()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        try await store.loadConversation("qa")
        return store
    }

    @MainActor
    func testTheSettledReplyIsHandedToTheReader() async throws {
        let store = try await connected()
        defer { store.deactivate() }
        var heard: [String] = []
        await store.sendByVoice("Run the smoke tests", to: "qa", reads: true) { text, _ in heard.append(text) }
        XCTAssertEqual(heard, ["Done. I ran it and everything passed."])
    }

    @MainActor
    func testNothingIsReadWhenReadingIsOff() async throws {
        let store = try await connected()
        defer { store.deactivate() }
        var heard: [String] = []
        await store.sendByVoice("Run the smoke tests", to: "qa", reads: false) { text, _ in heard.append(text) }
        XCTAssertTrue(heard.isEmpty)
        XCTAssertTrue(store.conversation("qa").contains { $0.displayContent == "Run the smoke tests" }, "it was still sent")
    }

    @MainActor
    func testLeavingStopsTheWait() async throws {
        let store = try await connected()
        defer { store.deactivate() }
        var heard: [String] = []
        await store.sendByVoice("Run the smoke tests", to: "qa", reads: true, keepWaiting: { false }) { text, _ in heard.append(text) }
        XCTAssertTrue(heard.isEmpty)
    }

    /// A dictated reply on the Mac goes through here: the quote goes up with it (spec #572).
    @MainActor
    func testAMessageByVoiceCarriesTheQuote() async throws {
        let store = try await connected()
        defer { store.deactivate() }
        let meta = MessageQuote.meta(nil, replyingTo: MessageQuote(id: 30, author: nil, excerpt: ""))
        await store.sendByVoice("Run the smoke tests", to: "qa", meta: meta, reads: false) { _, _ in }
        XCTAssertTrue(store.pending(for: "qa").isEmpty, "sent, not failed")
        XCTAssertEqual(store.conversation("qa").last { $0.content == "Run the smoke tests" }?.replyQuote?.id, 30)
    }
}
