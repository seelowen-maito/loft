import XCTest
@testable import LoftKit

/// The address the widgets read (spec #382): what the app saves comes back, and delete clears it.
final class SharedServerTests: XCTestCase {
    func testSavedAddressComesBackAndDeleteClearsIt() throws {
        let before = SharedServer.address
        defer { if let before { SharedServer.save(before) } else { SharedServer.delete() } }
        let url = try XCTUnwrap(URL(string: "http://mac-mini.local:8080"))
        XCTAssertEqual(SharedServer.save(url), errSecSuccess)
        XCTAssertEqual(SharedServer.address, url)
        SharedServer.delete()
        XCTAssertNil(SharedServer.address)
    }
}
