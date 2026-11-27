import XCTest
@testable import LoftKit

final class RequestURLTests: XCTestCase {
    private let base = URL(string: "http://h:1")!

    func testEncodedActionPathIsSentUnchanged() {
        let url = LiveLoftAPI.url(base, "/needs-you/task%3At-old%3Ablocked/dismiss", encoded: true)
        XCTAssertEqual(url.absoluteString, "http://h:1/api/needs-you/task%3At-old%3Ablocked/dismiss")
    }

    func testPlainActionPathsStillWork() {
        XCTAssertEqual(LiveLoftAPI.url(base, "/permissions/p1", encoded: true).absoluteString, "http://h:1/api/permissions/p1")
        XCTAssertEqual(LiveLoftAPI.url(base, "/needs-you/dismiss-all", encoded: true).absoluteString, "http://h:1/api/needs-you/dismiss-all")
        XCTAssertEqual(LiveLoftAPI.url(base, "/pull-requests/pr1/merge", encoded: true).absoluteString, "http://h:1/api/pull-requests/pr1/merge")
    }

    func testOtherPathsAreStillEncodedByTheClient() {
        XCTAssertEqual(LiveLoftAPI.url(base, "/saved/a b").absoluteString, "http://h:1/api/saved/a%20b")
        XCTAssertEqual(LiveLoftAPI.url(base, "/agents", query: [URLQueryItem(name: "limit", value: "5")]).absoluteString, "http://h:1/api/agents?limit=5")
    }
}
