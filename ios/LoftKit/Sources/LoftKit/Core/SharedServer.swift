import Foundation
import Security

/// The Loft address the app connected to, for the widgets: they run in their own process and can't read the app's
/// settings. It sits in the Keychain group the apps and the widgets share (`<team>.<bundle id>.shared`, the first
/// group in their entitlements). Only the address: Loft takes no token.
public enum SharedServer {
    private static let service = "loft.server"

    public static var address: URL? {
        var q = query
        q[kSecReturnData] = true
        q[kSecMatchLimit] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8).flatMap(URL.init(string:))
    }

    /// Saves the address; a failure (no shared Keychain group, as in an unsigned build) is logged, since the widgets
    /// then stay on "not connected".
    @discardableResult
    public static func save(_ server: URL) -> OSStatus {
        SecItemDelete(query as CFDictionary)
        var q = query
        q[kSecValueData] = Data(server.absoluteString.utf8)
        q[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlock
        let status = SecItemAdd(q as CFDictionary, nil)
        if status != errSecSuccess { Log.sync.error("widgets: saving the Loft address to the Keychain failed (\(status))") }
        return status
    }

    public static func delete() {
        SecItemDelete(query as CFDictionary)
    }

    private static var query: [CFString: Any] {
        var q: [CFString: Any] = [kSecClass: kSecClassGenericPassword, kSecAttrService: service]
        #if os(macOS)
        // Access groups only exist in the data protection Keychain.
        q[kSecUseDataProtectionKeychain] = true
        #endif
        return q
    }
}
