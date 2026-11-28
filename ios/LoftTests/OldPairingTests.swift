import Security
import XCTest
@testable import Loft
import LoftKit

final class OldPairingTests: XCTestCase {
    private func count() -> Int {
        let q: [CFString: Any] = [kSecClass: kSecClassGenericPassword, kSecAttrService: OldPairing.service, kSecMatchLimit: kSecMatchLimitAll, kSecReturnAttributes: true]
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess else { return 0 }
        return (out as? [[String: Any]])?.count ?? 0
    }

    /// An earlier version kept a pairing token per server in the Keychain; launching deletes them all.
    func testClearDeletesTheOldTokenOfEveryServer() {
        for server in ["http://a:7420", "http://b:7420"] {
            let add: [CFString: Any] = [kSecClass: kSecClassGenericPassword, kSecAttrService: OldPairing.service, kSecAttrAccount: server, kSecValueData: Data("t".utf8)]
            SecItemDelete(add as CFDictionary)
            XCTAssertEqual(SecItemAdd(add as CFDictionary, nil), errSecSuccess)
        }
        XCTAssertEqual(count(), 2)
        OldPairing.clear()
        XCTAssertEqual(count(), 0)
        OldPairing.clear()
    }
}
