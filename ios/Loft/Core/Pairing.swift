import Foundation
import Security

/// Pairing with Loft (spec #164, ADR 0004): only a paired client may change things through the API. The app
/// shows a code, polls with a secret, and keeps the token it gets in the Keychain.
enum Pairing {
    /// Posted when Loft refuses a request because this app isn't paired (a 401): the pairing screen comes back.
    static let unpairedNotification = Notification.Name("loft.unpaired")

    static func reportUnpaired() {
        NotificationCenter.default.post(name: unpairedNotification, object: nil)
    }

    static var isSimulator: Bool {
        #if targetEnvironment(simulator)
        true
        #else
        false
        #endif
    }

    /// Why this app may not pair, or nil when it may. Agents drive the Simulator and can read its files, so it
    /// never holds a token; neither does the demo. `fixture`: UI tests may walk the pairing screen on fixture data
    /// (`-fixture-pairing`), where nothing is ever stored.
    static func refusal(isDemo: Bool, fixture: Bool) -> String? {
        if isDemo {
            return "The demo runs on sample data and never pairs with a Mac. Leave the demo to pair this iPhone."
        }
        if isSimulator && !fixture {
            return "Loft never pairs the iOS Simulator: agents drive it and can read its files. Pair from a real iPhone, or open the demo's sample data here."
        }
        return nil
    }
}

/// `POST /api/pair`: the code to show, and the secret to poll with (never shown).
struct PairingStart: Decodable, Equatable {
    var secret: String
    var code: String
    var expiresIn: Int?
}

/// A browser or iPhone paired with Loft (`GET /api/paired-clients`); `this` is the one asking.
struct PairedClient: Decodable, Identifiable, Equatable {
    var id: String
    var name: String
    var pairedAt: Date
    var lastUsedAt: Date
    var this: Bool
}

/// `POST /api/pair/poll`: `pending`, `denied`, `gone` (expired, cancelled or already collected), or `allowed`
/// with the token, once.
struct PairingPoll: Decodable, Equatable {
    var status: String
    var token: String?
}

/// This app's token for one Loft server, in the Keychain only (`AfterFirstUnlockThisDeviceOnly`: not in backups,
/// not synced). In the Simulator nothing is ever read or written.
enum ClientToken {
    private static let service = "loft.client-token"

    static func load(for server: URL) -> String? {
        #if targetEnvironment(simulator)
        return nil
        #else
        var q = query(server)
        q[kSecReturnData] = true
        q[kSecMatchLimit] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
        #endif
    }

    @discardableResult
    static func save(_ token: String, for server: URL) -> Bool {
        #if targetEnvironment(simulator)
        return false
        #else
        SecItemDelete(query(server) as CFDictionary)
        var q = query(server)
        q[kSecValueData] = Data(token.utf8)
        q[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(q as CFDictionary, nil) == errSecSuccess
        #endif
    }

    static func delete(for server: URL) {
        #if !targetEnvironment(simulator)
        SecItemDelete(query(server) as CFDictionary)
        #endif
    }

    private static func query(_ server: URL) -> [CFString: Any] {
        [kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: server.absoluteString]
    }
}
