import Foundation
import Security

/// Keeps the node's secret keys in the Keychain (`KeyStore` from the core).
///
/// Items are generic passwords under `service`, readable after the first unlock since boot (so
/// background refresh works) and never synced or restored to another device
/// (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`). The Keychain survives uninstalling the
/// app; the core drops keys it finds next to a brand-new database, so a reinstall starts clean.
public final class KeychainKeyStore: KeyStore {
    public let service: String

    public init(service: String = "sh.hoi.keys") {
        self.service = service
    }

    public func get(key: String) -> Data? {
        var query = base(key)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess else { return nil }
        return out as? Data
    }

    public func set(key: String, value: Data?) {
        let query = base(key)
        guard let value else {
            SecItemDelete(query as CFDictionary)
            return
        }
        let attributes: [String: Any] = [
            kSecValueData as String: value,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            SecItemAdd(query.merging(attributes) { $1 } as CFDictionary, nil)
        }
    }

    private func base(_ key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
            kSecUseDataProtectionKeychain as String: true,
        ]
    }
}
