import Foundation
import Security

/// The pairing token an earlier version kept in the Keychain. Pairing is gone, so it is deleted at launch.
enum OldPairing {
    static let service = "loft.client-token"

    static func clear() {
        SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: service] as CFDictionary)
    }
}
