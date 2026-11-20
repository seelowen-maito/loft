import XCTest
@testable import LoftKit

final class DeadlineTests: XCTestCase {
    func testAWorkThatNeverFinishesReturnsNilOnTime() async {
        let start = Date()
        // Sleeps in a loop that ignores cancellation, like a hung request.
        let result = await within(seconds: 0.3) { () -> Bool in
            while true { try? await Task.sleep(for: .seconds(1)); if Date().timeIntervalSince(start) > 30 { return true } }
        }
        XCTAssertNil(result)
        XCTAssertLessThan(Date().timeIntervalSince(start), 1.5)
    }

    func testAFastWorkReturnsItsResult() async {
        let result = await within(seconds: 5) { 42 }
        XCTAssertEqual(result, 42)
    }
}
