import XCTest
@testable import Loft
@testable import LoftKit

final class GitHubAvatarTests: XCTestCase {
    func testOwnerPictureURLFromLogin() {
        XCTAssertEqual(GitHubAvatar.url(login: "octo-dev", pixels: 48)?.absoluteString, "https://github.com/octo-dev.png?size=48")
        // A typed-in Owner: trimmed, without its @.
        XCTAssertEqual(GitHubAvatar.url(login: "  @octo-dev ", pixels: 66)?.absoluteString, "https://github.com/octo-dev.png?size=66")
        // Anything that isn't a login character can't reach another path.
        XCTAssertEqual(GitHubAvatar.url(login: "a/b?c", pixels: 32)?.absoluteString, "https://github.com/a%2Fb%3Fc.png?size=32")
    }

    func testNoLoginNoPicture() {
        XCTAssertNil(GitHubAvatar.url(login: "", pixels: 32))
        XCTAssertNil(GitHubAvatar.url(login: " @ ", pixels: 32))
        XCTAssertNil(GitHubAvatar.login("@"))
        XCTAssertEqual(GitHubAvatar.login("@octo"), "octo")
    }
}
