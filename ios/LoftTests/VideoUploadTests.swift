import XCTest
@testable import Loft

/// Spec #203: videos from the iPhone chat, checked, uploaded with progress and sent with the message.
@MainActor
final class VideoUploadTests: XCTestCase {
    private var files: [URL] = []

    override func tearDown() { files.forEach { try? FileManager.default.removeItem(at: $0) } }

    /// A file of `bytes` (sparse, so 200 MB costs nothing).
    private func file(bytes: Int) throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("video-\(UUID().uuidString).mp4")
        FileManager.default.createFile(atPath: url.path, contents: Data("\0\0\0\u{20}ftypisom".utf8))
        let h = try FileHandle(forWritingTo: url)
        try h.truncate(atOffset: UInt64(bytes))
        try h.close()
        files.append(url)
        return url
    }

    private func connectedStore(_ api: MockAPI) async throws -> LoftStore {
        let cache = FileManager.default.temporaryDirectory.appendingPathComponent("snap-\(UUID().uuidString).json")
        files.append(cache)
        let store = LoftStore(api: api, cache: SnapshotStore(url: cache))
        store.activate()
        for _ in 0..<100 where store.connection != .connected { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertEqual(store.connection, .connected)
        return store
    }

    private func settle(_ draft: VideoDraft) async throws {
        for _ in 0..<200 where draft.busy { try await Task.sleep(for: .milliseconds(10)) }
    }

    func testTheLimitIs200MB() {
        XCTAssertNil(VideoAttachment.refusal(bytes: 200_000_000))
        XCTAssertEqual(VideoAttachment.refusal(bytes: 200_000_001), "This video is over 200 MB. Send a shorter one.")
    }

    func testLengthsAndWhichUploadsAreVideos() {
        XCTAssertEqual(VideoAttachment.length(6.4), "0:06")
        XCTAssertEqual(VideoAttachment.length(83), "1:23")
        XCTAssertEqual(VideoAttachment.length(3725), "1:02:05")
        let m = ChatMessage(id: 1, agentId: "a1", runId: nil, role: "user", kind: "text", content: "",
                            meta: ["images": .array([.string("a.png"), .string("b.MOV"), .string("c.mp4"), .string("d.webm")])], createdAt: .now)
        XCTAssertEqual(m.videos, ["b.MOV", "c.mp4", "d.webm"])
    }

    func testAVideoUploadsWithProgressThenIsReady() async throws {
        let api = MockAPI()
        api.uploadSteps = [0.25, 0.5, 1]
        let store = try await connectedStore(api)
        defer { store.deactivate() }
        let video = try file(bytes: 1_000)
        let draft = VideoDraft()
        var seen: [VideoDraft.State] = []
        draft.start(load: { video }) { f, p in try await store.upload(f, to: "a1", progress: p) }
        for _ in 0..<200 where draft.busy {
            if seen.last != draft.state { seen.append(draft.state) }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertEqual(draft.state, .ready("v1.mp4"))
        XCTAssertEqual(draft.uploaded, "v1.mp4")
        XCTAssertTrue(seen.contains(.uploading(0.5)), "progress shows on the way: \(seen)")
        XCTAssertEqual(api.uploadsMade, 1)
    }

    func testAVideoOver200MBIsRefusedWithoutUploading() async throws {
        let api = MockAPI()
        let store = try await connectedStore(api)
        defer { store.deactivate() }
        let big = try file(bytes: 200_000_001)
        let draft = VideoDraft()
        draft.start(load: { big }) { f, p in try await store.upload(f, to: "a1", progress: p) }
        try await settle(draft)
        XCTAssertEqual(draft.state, .failed("This video is over 200 MB. Send a shorter one."))
        XCTAssertNil(draft.uploaded)
        XCTAssertEqual(api.uploadsMade, 0)
    }

    func testAFailedUploadSaysWhy() async throws {
        let api = MockAPI()
        api.uploadError = .server(status: 413, message: "video is over 200 MB")
        let store = try await connectedStore(api)
        defer { store.deactivate() }
        let video = try file(bytes: 1_000)
        let draft = VideoDraft()
        draft.start(load: { video }) { f, p in try await store.upload(f, to: "a1", progress: p) }
        try await settle(draft)
        guard case .failed = draft.state else { return XCTFail("\(draft.state)") }
        XCTAssertNil(draft.uploaded)
    }

    func testAVideoGoesWithItsMessageEvenWithoutText() async throws {
        let api = MockAPI()
        let store = try await connectedStore(api)
        defer { store.deactivate() }
        await store.send("", to: "a1")
        XCTAssertTrue(api.sentImages.isEmpty, "an empty message without a video isn't sent")
        await store.send("", to: "a1", images: ["v1.mp4"])
        await store.send("Look", to: "a1")
        XCTAssertEqual(api.sentImages, [["v1.mp4"], []])
        XCTAssertTrue(store.pending.isEmpty)
    }
}
