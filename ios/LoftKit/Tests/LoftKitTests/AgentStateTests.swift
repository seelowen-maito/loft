import XCTest
@testable import LoftKit

/// Working comes from the live run, not the saved status alone (#595).
final class AgentStateTests: XCTestCase {
    private func state(_ status: String, running: Bool? = nil, pending: Int = 0) -> AgentState {
        var a = Agent(id: "a", name: "A", status: status)
        a.running = running
        return AgentState(agent: a, pendingApprovals: pending)
    }

    func testASavedRunningWithoutALiveRunIsIdle() {
        XCTAssertEqual(state("running"), .idle)
        XCTAssertEqual(state("running", running: false), .idle)
    }

    func testALiveRunIsWorking() {
        XCTAssertEqual(state("running", running: true), .working)
        XCTAssertEqual(state("idle", running: true), .working)
        XCTAssertEqual(state("error", running: true), .working)
    }

    func testTheOtherStates() {
        XCTAssertEqual(state("waiting"), .waitingApproval)
        XCTAssertEqual(state("idle", running: true, pending: 1), .waitingApproval)
        XCTAssertEqual(state("error"), .failed)
        XCTAssertEqual(state("idle"), .idle)
        XCTAssertEqual(state("weird"), .unknown)
    }
}
