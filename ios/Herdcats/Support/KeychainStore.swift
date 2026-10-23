import Foundation
import Security

/// Minimal Keychain wrapper for remembering connection secrets.
protocol KeychainOperations {
    func add(_ query: CFDictionary) -> OSStatus
    func update(_ query: CFDictionary, _ attributes: CFDictionary) -> OSStatus
    func copyMatching(_ query: CFDictionary, _ result: UnsafeMutablePointer<CFTypeRef?>) -> OSStatus
    func delete(_ query: CFDictionary) -> OSStatus
}

struct SystemKeychainOperations: KeychainOperations {
    func add(_ query: CFDictionary) -> OSStatus { SecItemAdd(query, nil) }
    func update(_ query: CFDictionary, _ attributes: CFDictionary) -> OSStatus { SecItemUpdate(query, attributes) }
    func copyMatching(_ query: CFDictionary, _ result: UnsafeMutablePointer<CFTypeRef?>) -> OSStatus { SecItemCopyMatching(query, result) }
    func delete(_ query: CFDictionary) -> OSStatus { SecItemDelete(query) }
}

enum KeychainStore {
    private static let service = "com.enchantinglabs.herdrcat"
    static let operations: KeychainOperations = SystemKeychainOperations()

    struct StoreError: LocalizedError {
        let status: OSStatus
        var errorDescription: String? { "Could not access saved connection credentials (Keychain error \(status))." }
    }

    static func save(_ secret: String, account: String, using operations: KeychainOperations = KeychainStore.operations) throws {
        try save(Data(secret.utf8), account: account, using: operations)
    }

    static func save(_ data: Data, account: String, using operations: KeychainOperations = KeychainStore.operations) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]

        var addQuery = query
        addQuery[kSecValueData as String] = data
        addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        switch operations.add(addQuery as CFDictionary) {
        case errSecSuccess: return
        case errSecDuplicateItem:
            let status = operations.update(query as CFDictionary, [
                kSecValueData as String: data,
                kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            ] as CFDictionary)
            if status == errSecSuccess { return }
            guard status == errSecItemNotFound else { throw StoreError(status: status) }
            // The item was deleted after duplicate detection. Retry the add once,
            // repairing an intervening insert with one final update.
            switch operations.add(addQuery as CFDictionary) {
            case errSecSuccess: return
            case errSecDuplicateItem:
                let retryStatus = operations.update(query as CFDictionary, [
                    kSecValueData as String: data,
                    kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
                ] as CFDictionary)
                guard retryStatus == errSecSuccess else { throw StoreError(status: retryStatus) }
            case let retryStatus: throw StoreError(status: retryStatus)
            }
        case let status: throw StoreError(status: status)
        }
    }

    static func load(account: String, using operations: KeychainOperations = KeychainStore.operations) throws -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: CFTypeRef?
        let status = operations.copyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw StoreError(status: status) }
        guard let data = result as? Data, let secret = String(data: data, encoding: .utf8) else {
            throw StoreError(status: errSecDecode)
        }
        // Upgrade older saved credentials before exposing their value. The query
        // is scoped to this service and account, so unrelated Keychain items are
        // never touched. If the item disappeared or the policy change fails,
        // fail closed rather than returning data from storage we could not harden.
        let updateStatus = operations.update(
            [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecAttrAccount as String: account,
            ] as CFDictionary,
            [kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly] as CFDictionary
        )
        guard updateStatus == errSecSuccess else { throw StoreError(status: updateStatus) }
        return secret
    }

    static func delete(account: String, using operations: KeychainOperations = KeychainStore.operations) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let status = operations.delete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw StoreError(status: status) }
    }

    // MARK: - Accounts

    enum Account {
        static let password = "connection.password"
        static let privateKey = "connection.privateKey"
    }
}

/// Non-secret connection settings. Credentials are isolated by profile in Keychain.
struct RecentConnection: Codable, Identifiable, Equatable {
    var id = UUID()
    var host: String
    var port: Int
    var username: String
    var authMode: String
    var remember: Bool

    var secretAccount: String { "connection.\(id.uuidString).secret" }
}

enum RecentConnectionStore {
    static let storageKey = "hc.recentConnections"
    private static let pendingMigrationKey = "hc.pendingCredentialMigration"

    private struct PendingMigration: Codable {
        let sourceAccount: String
        let entry: RecentConnection
        var destinationAccount: String { entry.secretAccount }
    }

    static func load(defaults: UserDefaults = .standard, using keychain: KeychainOperations = KeychainStore.operations) throws -> [RecentConnection] {
        try finishPendingMigration(defaults: defaults, using: keychain)
        if let data = defaults.data(forKey: storageKey) {
            return (try? JSONDecoder().decode([RecentConnection].self, from: data)) ?? []
        }

        // Preserve the single connection saved by earlier versions of the app.
        let host = (defaults.string(forKey: "hc.host") ?? "").trimmingCharacters(in: .whitespaces)
        let username = (defaults.string(forKey: "hc.username") ?? "").trimmingCharacters(in: .whitespaces)
        guard !host.isEmpty, !username.isEmpty else { return [] }
        let port = Int(defaults.string(forKey: "hc.port") ?? "22") ?? 22
        guard (1...65_535).contains(port) else { return [] }
        let entry = RecentConnection(
            host: host, port: port, username: username,
            authMode: defaults.string(forKey: "hc.authMode") ?? "password",
            remember: defaults.object(forKey: "hc.remember") as? Bool ?? true
        )
        if entry.remember {
            let account = entry.authMode == "privateKey"
                ? KeychainStore.Account.privateKey : KeychainStore.Account.password
            if let secret = try KeychainStore.load(account: account, using: keychain) {
                // A pending marker makes cleanup retryable if the app stops between
                // recording the profile and deleting the verified legacy item.
                let pending = PendingMigration(sourceAccount: account, entry: entry)
                let pendingData = try JSONEncoder().encode(pending)
                defaults.set(pendingData, forKey: pendingMigrationKey)
                guard defaults.data(forKey: pendingMigrationKey) == pendingData else { throw StoreError.metadataWriteFailed }
                try KeychainStore.save(secret, account: entry.secretAccount, using: keychain)
                guard try KeychainStore.load(account: entry.secretAccount, using: keychain) == secret else {
                    throw KeychainStore.StoreError(status: errSecDecode)
                }
                _ = try record(entry, in: [], defaults: defaults)
                try finishPendingMigration(defaults: defaults, using: keychain)
                return [entry]
            }
        }
        return try record(entry, in: [], defaults: defaults)
    }

    static func entry(
        host: String, port: Int, username: String, authMode: String,
        remember: Bool, in entries: [RecentConnection]
    ) -> RecentConnection {
        let host = host.trimmingCharacters(in: .whitespaces)
        let username = username.trimmingCharacters(in: .whitespaces)
        var entry = entries.first {
            $0.host.lowercased() == host.lowercased() && $0.port == port && $0.username == username
        } ?? RecentConnection(host: host, port: port, username: username, authMode: authMode, remember: remember)
        entry.authMode = authMode
        entry.remember = remember
        return entry
    }

    @discardableResult
    static func record(
        _ entry: RecentConnection, in entries: [RecentConnection], defaults: UserDefaults = .standard
    ) throws -> [RecentConnection] {
        let updated = [entry] + entries.filter { $0.id != entry.id }
        try persist(updated, defaults: defaults)
        return updated
    }

    /// Records that a saved connection must no longer remember credentials, in place.
    /// Never inserts an unsaved connection and never reorders the list.
    @discardableResult
    static func disableRemembering(
        _ entry: RecentConnection, defaults: UserDefaults = .standard,
        using keychain: KeychainOperations = KeychainStore.operations
    ) throws -> [RecentConnection] {
        var entries = try load(defaults: defaults, using: keychain)
        guard let index = entries.firstIndex(where: { $0.id == entry.id }) else { return entries }
        var updated = entry
        updated.remember = false
        guard entries[index] != updated else { return entries }
        entries[index] = updated
        try persist(entries, defaults: defaults)
        return entries
    }

    /// Forgets a saved connection: deletes its Keychain secret and passphrase first
    /// (so a Keychain failure leaves the entry visible and retryable), then removes
    /// it from the saved list.
    @discardableResult
    static func remove(
        _ entry: RecentConnection, defaults: UserDefaults = .standard,
        using keychain: KeychainOperations = KeychainStore.operations
    ) throws -> [RecentConnection] {
        try KeychainStore.delete(account: entry.secretAccount, using: keychain)
        try KeychainStore.delete(account: entry.secretAccount + ".passphrase", using: keychain)
        let updated = try load(defaults: defaults, using: keychain).filter { $0.id != entry.id }
        try persist(updated, defaults: defaults)
        return updated
    }

    private static func persist(_ entries: [RecentConnection], defaults: UserDefaults) throws {
        let data = try JSONEncoder().encode(entries)
        defaults.set(data, forKey: storageKey)
        guard defaults.data(forKey: storageKey) == data else {
            throw StoreError.metadataWriteFailed
        }
    }

    private static func finishPendingMigration(defaults: UserDefaults, using keychain: KeychainOperations) throws {
        guard let data = defaults.data(forKey: pendingMigrationKey) else { return }
        let pending = try JSONDecoder().decode(PendingMigration.self, from: data)
        let entries: [RecentConnection]
        if let current = defaults.data(forKey: storageKey) {
            guard let decoded = try? JSONDecoder().decode([RecentConnection].self, from: current) else {
                throw StoreError.metadataWriteFailed
            }
            entries = decoded
        } else {
            entries = []
        }
        let committedEntry = entries.first { $0.id == pending.entry.id }
        let hasCommittedProfile = committedEntry == pending.entry && pending.entry.remember
        if defaults.data(forKey: storageKey) != nil && committedEntry == nil {
            // The profile was removed or replaced after the migration began.
            // Do not recreate it or clean up the legacy credential.
            defaults.removeObject(forKey: pendingMigrationKey)
            return
        }
        if committedEntry != nil && !hasCommittedProfile {
            // Settings changed after the interrupted migration; never replay its
            // pending copy or cleanup against the edited profile.
            defaults.removeObject(forKey: pendingMigrationKey)
            return
        }
        guard let source = try KeychainStore.load(account: pending.sourceAccount, using: keychain) else {
            guard hasCommittedProfile else { throw KeychainStore.StoreError(status: errSecItemNotFound) }
            // The committed profile proves migration already finished; this is a
            // crash after source deletion but before clearing the pending marker.
            defaults.removeObject(forKey: pendingMigrationKey)
            return
        }
        var destination = try KeychainStore.load(account: pending.destinationAccount, using: keychain)
        if destination == nil && hasCommittedProfile {
            // The profile was already committed and its credential was later removed.
            // Leave the legacy item intact; replay must not silently restore it.
            defaults.removeObject(forKey: pendingMigrationKey)
            return
        }
        if destination == nil {
            try KeychainStore.save(source, account: pending.destinationAccount, using: keychain)
            destination = try KeychainStore.load(account: pending.destinationAccount, using: keychain)
        }
        guard destination == source else {
            guard hasCommittedProfile else { throw KeychainStore.StoreError(status: errSecDuplicateItem) }
            // The destination may have been edited since migration; do not overwrite it
            // or delete the legacy source based on stale migration intent.
            defaults.removeObject(forKey: pendingMigrationKey)
            return
        }
        if !hasCommittedProfile {
            let entries = try defaults.data(forKey: storageKey).map { try JSONDecoder().decode([RecentConnection].self, from: $0) } ?? []
            _ = try record(pending.entry, in: entries, defaults: defaults)
        }
        try KeychainStore.delete(account: pending.sourceAccount, using: keychain)
        defaults.removeObject(forKey: pendingMigrationKey)
    }

    struct StoreError: LocalizedError {
        static let metadataWriteFailed = StoreError()
        var errorDescription: String? { "Could not save connection settings. Please try again." }
    }
}

/// Host public keys are trust anchors, kept separately from login credentials.
/// A failed Keychain read must never turn a known host into a first-use prompt.
actor SSHHostKeyStore {
    static let shared = SSHHostKeyStore()
    private let service: String

    init(service: String = "com.enchantinglabs.herdrcat.ssh-host-keys") {
        self.service = service
    }

    static func account(host: String, port: Int) -> String {
        let host = host.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return "[\(host)]:\(port)"
    }

    private func query(host: String, port: Int) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: Self.account(host: host, port: port)]
    }

    func load(host: String, port: Int) throws -> String? {
        var query = query(host: host, port: port)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw StoreError(status: status) }
        guard let data = result as? Data, let key = String(data: data, encoding: .utf8), !key.isEmpty else {
            throw StoreError(status: errSecDecode)
        }
        // Opportunistically upgrade pins stored under a weaker accessibility class.
        // Best effort: the pin is already read, and a failed upgrade must never turn
        // a known host into an unknown one.
        _ = SecItemUpdate(
            self.query(host: host, port: port) as CFDictionary,
            [kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly] as CFDictionary
        )
        return key
    }

    /// First-use approval is insert-only: it cannot replace an existing pin.
    func trust(_ key: String, host: String, port: Int) throws {
        var query = query(host: host, port: port)
        query[kSecValueData as String] = Data(key.utf8)
        query[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        let status = SecItemAdd(query as CFDictionary, nil)
        if status == errSecDuplicateItem {
            guard try load(host: host, port: port) == key else {
                throw StoreError(status: errSecDuplicateItem)
            }
        } else if status != errSecSuccess {
            throw StoreError(status: status)
        }
    }

    /// Replace only the pin reviewed by the user. A stale review must fail closed.
    func replace(_ key: String, replacing previousKey: String, host: String, port: Int) throws {
        guard try load(host: host, port: port) == previousKey else {
            throw StoreError(status: errSecDuplicateItem)
        }
        let match = query(host: host, port: port)
        let status = SecItemUpdate(match as CFDictionary,
                                  [kSecValueData as String: Data(key.utf8),
                                   kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly] as CFDictionary)
        guard status == errSecSuccess else { throw StoreError(status: status) }
    }

    struct StoreError: LocalizedError {
        let status: OSStatus
        var errorDescription: String? {
            "Could not access saved SSH host keys (Keychain error \(status)). Connection stopped."
        }
    }
}
