import XCTest
@testable import Loft

/// The checks around the on-device spoken version (no model needed): what goes in, and when the app falls back.
final class SpokenVersionTests: XCTestCase {
    func testTheInputLeavesOutWhatCantBeSaid() throws {
        guard #available(iOS 26.0, *) else { throw XCTSkip("iOS 26") }
        let s = OnDeviceJobs.spokenInput("**Merged** [PR #214](https://x/y/pull/214). Run `make test`:\n```\nok\n```\nSee https://a.b/c")
        XCTAssertFalse(s.contains("#214"))
        XCTAssertFalse(s.contains("http"))
        XCTAssertFalse(s.contains("make test"))
        XCTAssertFalse(s.contains("*"))
        XCTAssertTrue(s.contains("Merged"))
        XCTAssertTrue(s.contains("PR"))
    }

    func testUnusableVersionsFallBack() throws {
        guard #available(iOS 26.0, *) else { throw XCTSkip("iOS 26") }
        let reply = "a long reply"
        XCTAssertNil(OnDeviceJobs.problem(withSpoken: "The docs site is live. Nothing needs you.", for: reply))
        XCTAssertNotNil(OnDeviceJobs.problem(withSpoken: "", for: reply))
        XCTAssertNotNil(OnDeviceJobs.problem(withSpoken: "Merged #63 and #65.", for: reply), "PR numbers aren't spoken")
        XCTAssertNotNil(OnDeviceJobs.problem(withSpoken: "See https://example.com.", for: reply))
        XCTAssertNotNil(OnDeviceJobs.problem(withSpoken: "Run `make`.", for: reply))
        XCTAssertNotNil(OnDeviceJobs.problem(withSpoken: "One. Two. Three. Four. Five.", for: reply), "a few sentences at most")
        XCTAssertNotNil(OnDeviceJobs.problem(withSpoken: String(repeating: "word ", count: 90), for: reply))
    }
}
