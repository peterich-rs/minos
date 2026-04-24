import Foundation
import Security

/// Persists the Cloudflare Access Service Token pair (CF-Access-Client-Id +
/// CF-Access-Client-Secret) in the macOS login Keychain under the shared
/// service identifier `ai.minos.macos`. Mirrors the Rust-side
/// `KeychainTrustedDeviceStore` (which owns the `device-secret` account
/// under the same service) so all Minos secrets live in one Keychain entry
/// group; users can audit / wipe them via Keychain Access in one place.
///
/// Plan 05 Phase H.1.
enum KeychainRelayConfig {
    static let service = "ai.minos.macos"
    static let accountClientId = "cf-client-id"
    static let accountClientSecret = "cf-client-secret"

    struct Creds: Equatable {
        let clientId: String
        let clientSecret: String
    }

    /// Read both halves of the CF token pair. Returns `nil` if either is
    /// missing — partial state means we're not configured.
    static func read() -> Creds? {
        guard
            let id = readItem(account: accountClientId),
            let secret = readItem(account: accountClientSecret)
        else { return nil }
        return Creds(clientId: id, clientSecret: secret)
    }

    /// Persist both halves. Existing values are overwritten in place.
    static func write(_ creds: Creds) throws {
        try writeItem(account: accountClientId, value: creds.clientId)
        try writeItem(account: accountClientSecret, value: creds.clientSecret)
    }

    /// Remove both halves. Idempotent: missing entries are not an error.
    static func clear() throws {
        try deleteItem(account: accountClientId)
        try deleteItem(account: accountClientSecret)
    }

    // MARK: - Internal

    private static func readItem(account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard
            status == errSecSuccess,
            let data = item as? Data,
            let s = String(data: data, encoding: .utf8)
        else {
            return nil
        }
        return s
    }

    private static func writeItem(account: String, value: String) throws {
        let attrs: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let valueData: [String: Any] = [
            kSecValueData as String: Data(value.utf8),
        ]
        let combined = attrs.merging(valueData, uniquingKeysWith: { $1 })
        let status = SecItemAdd(combined as CFDictionary, nil)
        if status == errSecDuplicateItem {
            let updateStatus = SecItemUpdate(attrs as CFDictionary, valueData as CFDictionary)
            if updateStatus != errSecSuccess {
                throw KeychainError.unexpected(status: updateStatus)
            }
        } else if status != errSecSuccess {
            throw KeychainError.unexpected(status: status)
        }
    }

    private static func deleteItem(account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecSuccess && status != errSecItemNotFound {
            throw KeychainError.unexpected(status: status)
        }
    }

    enum KeychainError: Error {
        case unexpected(status: OSStatus)
    }
}
