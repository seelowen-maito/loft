import XCTest

/// Waits that check at once and then every tenth of a second. XCTest's waitForExistence checks about once a
/// second, so each wait cost about a second even when the element was already on screen (13 of them made
/// testTalkButton take 22 s).
extension XCUIElement {
    @discardableResult
    func appears(timeout: TimeInterval) -> Bool { poll(timeout) { self.exists } }

    @discardableResult
    func disappears(timeout: TimeInterval) -> Bool { poll(timeout) { !self.exists } }

    private func poll(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let end = Date().addingTimeInterval(timeout)
        while true {
            if done() { return true }
            if Date() >= end { return false }
            Thread.sleep(forTimeInterval: 0.1)
        }
    }
}
