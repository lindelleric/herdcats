import Foundation
import Security
import NIOSSH
import SwiftUI
import Testing
@testable import Herdcats

@Suite("Pane last updated")
struct PaneLastUpdatedTests {
    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        calendar.locale = Locale(identifier: "en_US_POSIX")
        return calendar
    }

    private var locale: Locale { Locale(identifier: "en_US_POSIX") }

    func date(_ year: Int, _ month: Int, _ day: Int, _ hour: Int = 15, _ minute: Int = 30) -> Date {
        calendar.date(from: DateComponents(
            year: year, month: month, day: day, hour: hour, minute: minute
        ))!
    }

    @Test func formatsTodayAsTime() {
        let now = date(2026, 9, 20, 18, 0)
        let label = PaneUpdatedFormat.label(
            for: date(2026, 9, 20, 9, 5),
            now: now,
            calendar: calendar,
            locale: locale
        )
        #expect(label.contains("9"))
        #expect(label.contains("05"))
    }

    @Test func formatsYesterday() {
        let now = date(2026, 9, 20)
        let label = PaneUpdatedFormat.label(
            for: date(2026, 9, 19, 22, 0),
            now: now,
            calendar: calendar,
            locale: locale
        )
        #expect(label == "Yesterday")
    }

    @Test func formatsWeekdayWithinWeek() {
        let now = date(2026, 9, 20) // Sunday
        let label = PaneUpdatedFormat.label(
            for: date(2026, 9, 17, 11, 0), // Thursday
            now: now,
            calendar: calendar,
            locale: locale
        )
        #expect(label == "Thursday")
    }

    @Test func formatsDateBeyondWeek() {
        let now = date(2026, 9, 20)
        let label = PaneUpdatedFormat.label(
            for: date(2026, 9, 1, 8, 0),
            now: now,
            calendar: calendar,
            locale: locale
        )
        #expect(label.contains("Sep"))
        #expect(label.contains("1"))
    }

    @Test func observeRecordsFirstSeenAndBumpsOnChange() throws {
        let suiteName = "PaneLastUpdatedTests.\(UUID())"
        let defaults = try #require(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let scope = ConnectionIdentity(host: "test.local", port: 22, username: "tester")

        let firstJSON = """
        {"id":"cli:pane:list","result":{"panes":[
        {"agent_status":"idle","focused":true,"pane_id":"w1:p1","revision":1,"tab_id":"w1:t1","workspace_id":"w1"}
        ]}}
        """
        let first = try HerdrConnection.decode(firstJSON, as: PaneListResult.self).panes
        let firstObservedAt = date(2026, 9, 20, 10, 0)
        let dates1 = PaneLastUpdatedStore.observe(first, scope: scope, now: firstObservedAt, defaults: defaults)
        #expect(dates1["w1:p1"] == firstObservedAt)

        let unchanged = PaneLastUpdatedStore.observe(
            first, scope: scope, now: date(2026, 9, 20, 11, 0), defaults: defaults)
        #expect(unchanged["w1:p1"] == firstObservedAt)

        let secondJSON = """
        {"id":"cli:pane:list","result":{"panes":[
        {"agent_status":"working","focused":true,"pane_id":"w1:p1","revision":2,"tab_id":"w1:t1","workspace_id":"w1"}
        ]}}
        """
        let second = try HerdrConnection.decode(secondJSON, as: PaneListResult.self).panes
        let secondObservedAt = date(2026, 9, 20, 12, 0)
        let dates2 = PaneLastUpdatedStore.observe(second, scope: scope, now: secondObservedAt, defaults: defaults)
        #expect(dates2["w1:p1"] == secondObservedAt)
    }

    @Test func observeAgentsSharesStoreWithPanes() throws {
        let suiteName = "PaneLastUpdatedTests.agents.\(UUID())"
        let defaults = try #require(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let scope = ConnectionIdentity(host: "test.local", port: 22, username: "tester")

        let agentsJSON = """
        {"id":"cli:agent:list","result":{"agents":[
        {"agent":"cursor","agent_status":"idle","focused":true,"pane_id":"w1:p1","revision":1,
        "tab_id":"w1:t1","workspace_id":"w1"}
        ]}}
        """
        let agents = try HerdrConnection.decode(agentsJSON, as: AgentListResult.self).agents
        let firstObservedAt = date(2026, 9, 20, 10, 0)
        let dates1 = PaneLastUpdatedStore.observe(agents, scope: scope, now: firstObservedAt, defaults: defaults)
        #expect(dates1["w1:p1"] == firstObservedAt)

        let panesJSON = """
        {"id":"cli:pane:list","result":{"panes":[
        {"agent":"cursor","agent_status":"idle","focused":true,"label":"command","pane_id":"w1:p1",
        "revision":1,"tab_id":"w1:t1","workspace_id":"w1"}
        ]}}
        """
        let panes = try HerdrConnection.decode(panesJSON, as: PaneListResult.self).panes
        let unchanged = PaneLastUpdatedStore.observe(
            panes, scope: scope, now: date(2026, 9, 20, 11, 0), defaults: defaults)
        #expect(unchanged["w1:p1"] == firstObservedAt)

        let workingAgentsJSON = """
        {"id":"cli:agent:list","result":{"agents":[
        {"agent":"cursor","agent_status":"working","focused":true,"pane_id":"w1:p1","revision":2,
        "tab_id":"w1:t1","workspace_id":"w1"}
        ]}}
        """
        let working = try HerdrConnection.decode(workingAgentsJSON, as: AgentListResult.self).agents
        let secondObservedAt = date(2026, 9, 20, 12, 0)
        let dates2 = PaneLastUpdatedStore.observe(working, scope: scope, now: secondObservedAt, defaults: defaults)
        #expect(dates2["w1:p1"] == secondObservedAt)
    }
}

@Suite("Agents attention ordering")
struct AgentAttentionOrderingTests {
    func item(_ id: String, _ status: String, _ sequence: UInt64?) throws -> AgentListItem {
        let workspace = try HerdrConnection.decode(workspaceListJSON, as: WorkspaceListResult.self).workspaces[0]
        let agent = AgentEntry(
            agent: "codex", agentStatus: status, stateChangeSeq: sequence,
            cwd: nil, paneId: id, tabId: "tab", workspaceId: workspace.workspaceId,
            terminalTitle: nil, terminalTitleStripped: "Same title", focused: false,
            revision: nil
        )
        return AgentListItem(space: Space(workspace: workspace, agents: [agent]), agent: agent)
    }

    @Test
    func blockedThenDoneThenWorkingThenIdleThenUnknownPerHerdrPriority() throws {
        let items = try [
            item("working", "working", 100),
            item("done", "done", 30),
            item("idle", "idle", 40),
            item("blocked", "blocked", 10),
            item("unknown", "unknown", 200)
        ]
        // Mirrors herdr's tab_attention_priority: blocked > done > working >
        // idle > unknown, regardless of state-change recency.
        #expect(items.sorted(by: AgentListItem.attentionOrder).map(\.id)
                == ["blocked", "done", "working", "idle", "unknown"])
    }

    @Test
    func missingSequenceAndTiesHaveStableFallbacks() throws {
        let items = try [item("z", "idle", nil), item("b", "done", 5),
                         item("a", "idle", 5), item("y", "done", nil)]
        #expect(items.sorted(by: AgentListItem.attentionOrder).map(\.id) == ["b", "y", "a", "z"])
    }

    @Test
    func groupedSectionsFollowSpaceOrderAndKeepJoinOrder() throws {
        let workspaces = try HerdrConnection.decode(workspaceListJSON, as: WorkspaceListResult.self).workspaces
        let agents = try HerdrConnection.decode(agentListJSON, as: AgentListResult.self).agents
        let spaces = Space.join(workspaces: workspaces, agents: agents)

        let sections = AgentListItem.grouped(in: spaces)
        // w32 has no agents: its section is hidden, not rendered empty.
        #expect(sections.map(\.space.id) == ["w4", "w4H"])
        #expect(sections.allSatisfy { !$0.items.isEmpty })

        // The two-agent space keeps its join (title) order in grouped mode —
        // grouping must not silently re-sort by attention.
        let samplenotes = try #require(sections.first { $0.space.id == "w4" })
        #expect(samplenotes.items.map(\.id) == ["w4:p2V", "w3W:p4"])
        #expect(samplenotes.items.map(\.id) == samplenotes.space.agents.map(\.paneId))
    }

    @Test
    func decodesSequenceAndAcceptsOlderResponses() throws {
        let agents = try HerdrConnection.decode(agentListJSON, as: AgentListResult.self).agents
        #expect(agents.first?.stateChangeSeq == 52)
        let olderJSON = agentListJSON.replacingOccurrences(of: "\"state_change_seq\":52,", with: "")
        let olderAgents = try HerdrConnection.decode(olderJSON, as: AgentListResult.self).agents
        #expect(olderAgents.first?.stateChangeSeq == nil)
    }
}

@Suite("Card reorder motion")
struct CardReorderMotionTests {
    @Test func raisesTheCardWhosePriorityChanged() {
        let old = [
            ReorderingCard(id: "a", priorityKey: "working"),
            ReorderingCard(id: "b", priorityKey: "working")
        ]
        let new = [
            ReorderingCard(id: "b", priorityKey: "working"),
            ReorderingCard(id: "a", priorityKey: "idle")
        ]
        #expect(ReorderingCard.movingID(from: old, to: new) == "a")
    }

    @Test func ignoresInitialLoadAndUnchangedOrder() {
        let cards = [ReorderingCard(id: "a", priorityKey: "working")]
        #expect(ReorderingCard.movingID(from: [], to: cards) == nil)
        #expect(ReorderingCard.movingID(from: cards, to: cards) == nil)
        #expect(
            ReorderingCard.movingID(from: cards, to: [ReorderingCard(id: "b", priorityKey: "blocked")] + cards) == nil)
    }
}

@Suite("Recent connections")
struct RecentConnectionTests {
    @Test func reconnectMovesExistingProfileToFrontAndPreservesCredentialAccount() throws {
        let defaults = try #require(UserDefaults(suiteName: "RecentConnectionTests.\(UUID())"))
        defer { defaults.removeObject(forKey: RecentConnectionStore.storageKey) }
        let first = RecentConnection(
            host: "mac.local", port: 22, username: "alice", authMode: "password", remember: true)
        let second = RecentConnection(
            host: "server.local", port: 2222, username: "bob", authMode: "privateKey", remember: false)
        let updated = RecentConnectionStore.entry(
            host: " MAC.local ", port: 22, username: " alice ", authMode: "privateKey",
            remember: false, in: [second, first]
        )
        #expect(updated.id == first.id)
        #expect(updated.secretAccount == first.secretAccount)
        #expect(updated.authMode == "privateKey")
        #expect(!updated.remember)
        let entries = try RecentConnectionStore.record(updated, in: [second, first], defaults: defaults)
        #expect(entries.map(\.id) == [first.id, second.id])
        #expect(try RecentConnectionStore.load(defaults: defaults) == entries)
    }

    @Test func differentPortsAndUsersHaveSeparateCredentialAccounts() {
        let first = RecentConnection(
            host: "mac.local", port: 22, username: "alice", authMode: "password", remember: true)
        let port = RecentConnectionStore.entry(
            host: first.host, port: 2222, username: first.username, authMode: "password", remember: true, in: [first])
        let user = RecentConnectionStore.entry(
            host: first.host, port: 22, username: "bob", authMode: "password", remember: true, in: [first])
        #expect(Set([first.secretAccount, port.secretAccount, user.secretAccount]).count == 3)
    }

    @Test func removeDeletesMetadataSecretAndPassphraseOfOnlyThatConnection() throws {
        let defaults = try #require(UserDefaults(suiteName: "RecentConnectionTests.\(UUID())"))
        defer { defaults.removeObject(forKey: RecentConnectionStore.storageKey) }
        let keychain = FakeKeychainOperations()
        let first = RecentConnection(
            host: "mac.local", port: 22, username: "alice", authMode: "privateKey", remember: true)
        let second = RecentConnection(
            host: "server.local", port: 22, username: "bob", authMode: "password", remember: true)
        _ = try RecentConnectionStore.record(first, in: [second], defaults: defaults)
        for entry in [first, second] {
            try KeychainStore.save("secret", account: entry.secretAccount, using: keychain)
            try KeychainStore.save("phrase", account: entry.secretAccount + ".passphrase", using: keychain)
        }

        let remaining = try RecentConnectionStore.remove(first, defaults: defaults, using: keychain)

        #expect(remaining == [second])
        #expect(try RecentConnectionStore.load(defaults: defaults, using: keychain) == [second])
        #expect(keychain.items[first.secretAccount] == nil)
        #expect(keychain.items[first.secretAccount + ".passphrase"] == nil)
        #expect(keychain.items[second.secretAccount] != nil)
        #expect(keychain.items[second.secretAccount + ".passphrase"] != nil)
    }

    @Test func removeOfUnknownConnectionLeavesOthersAndToleratesMissingSecrets() throws {
        let defaults = try #require(UserDefaults(suiteName: "RecentConnectionTests.\(UUID())"))
        defer { defaults.removeObject(forKey: RecentConnectionStore.storageKey) }
        let keychain = FakeKeychainOperations()
        let saved = RecentConnection(
            host: "mac.local", port: 22, username: "alice", authMode: "password", remember: false)
        let stranger = RecentConnection(
            host: "other.local", port: 22, username: "eve", authMode: "password", remember: true)
        _ = try RecentConnectionStore.record(saved, in: [], defaults: defaults)

        #expect(try RecentConnectionStore.remove(stranger, defaults: defaults, using: keychain) == [saved])
    }

    @Test func removeKeepsEntryWhenKeychainDeletionFails() throws {
        let defaults = try #require(UserDefaults(suiteName: "RecentConnectionTests.\(UUID())"))
        defer { defaults.removeObject(forKey: RecentConnectionStore.storageKey) }
        let keychain = FakeKeychainOperations()
        let saved = RecentConnection(
            host: "mac.local", port: 22, username: "alice", authMode: "password", remember: true)
        _ = try RecentConnectionStore.record(saved, in: [], defaults: defaults)
        try KeychainStore.save("secret", account: saved.secretAccount, using: keychain)
        keychain.nextDeleteStatus = errSecInteractionNotAllowed

        #expect(throws: KeychainStore.StoreError.self) {
            try RecentConnectionStore.remove(saved, defaults: defaults, using: keychain)
        }
        #expect(try RecentConnectionStore.load(defaults: defaults, using: keychain) == [saved])
    }

    @Test func disableRememberingUpdatesSavedEntryInPlaceWithoutReordering() throws {
        let defaults = try #require(UserDefaults(suiteName: "RecentConnectionTests.\(UUID())"))
        defer { defaults.removeObject(forKey: RecentConnectionStore.storageKey) }
        let keychain = FakeKeychainOperations()
        let first = RecentConnection(
            host: "mac.local", port: 22, username: "alice", authMode: "password", remember: true)
        let second = RecentConnection(
            host: "server.local", port: 22, username: "bob", authMode: "password", remember: true)
        _ = try RecentConnectionStore.record(second, in: [first], defaults: defaults)

        let entries = try RecentConnectionStore.disableRemembering(first, defaults: defaults, using: keychain)

        #expect(entries.map(\.id) == [second.id, first.id])
        #expect(entries.first { $0.id == first.id }?.remember == false)
        #expect(entries.first { $0.id == second.id }?.remember == true)
        #expect(try RecentConnectionStore.load(defaults: defaults, using: keychain) == entries)
    }

    @Test func disableRememberingNeverInsertsAnUnsavedConnection() throws {
        let defaults = try #require(UserDefaults(suiteName: "RecentConnectionTests.\(UUID())"))
        defer { defaults.removeObject(forKey: RecentConnectionStore.storageKey) }
        let keychain = FakeKeychainOperations()
        let unsaved = RecentConnection(
            host: "new.local", port: 22, username: "alice", authMode: "password", remember: false)

        #expect(try RecentConnectionStore.disableRemembering(unsaved, defaults: defaults, using: keychain).isEmpty)
        #expect(defaults.data(forKey: RecentConnectionStore.storageKey) == nil)
    }

    @Test func emptyOrCorruptHistoryIsSafe() throws {
        let defaults = try #require(UserDefaults(suiteName: "RecentConnectionTests.\(UUID())"))
        defer { defaults.removeObject(forKey: RecentConnectionStore.storageKey) }
        #expect(try RecentConnectionStore.load(defaults: defaults).isEmpty)
        defaults.set(Data("invalid".utf8), forKey: RecentConnectionStore.storageKey)
        #expect(try RecentConnectionStore.load(defaults: defaults).isEmpty)
    }
}

final class FakeKeychainOperations: KeychainOperations {
    var items: [String: Data] = [:]
    var nextAddStatus: OSStatus?
    var nextUpdateStatus: OSStatus?
    var nextDeleteStatus: OSStatus?
    var failingReadAccount: String?
    var mismatchReadAccount: String?
    var onAdd: ((String) -> Void)?
    var onDelete: ((String) -> Void)?
    var addedAccounts: [String] = []
    var deletedAccounts: [String] = []
    var lastAddQuery: NSDictionary?
    var protectionByAccount: [String: String] = [:]
    var updatedAccounts: [String] = []
    var copiedAccounts: [String] = []

    func account(_ query: CFDictionary) -> String {
        guard let account = (query as NSDictionary)[kSecAttrAccount as String] as? String else {
            preconditionFailure("Synthetic keychain query is missing its account")
        }
        return account
    }

    func add(_ query: CFDictionary) -> OSStatus {
        let account = account(query)
        addedAccounts.append(account)
        lastAddQuery = query as NSDictionary
        if let status = nextAddStatus { nextAddStatus = nil; return status }
        guard items[account] == nil else { return errSecDuplicateItem }
        items[account] = (query as NSDictionary)[kSecValueData as String] as? Data
        protectionByAccount[account] = (query as NSDictionary)[kSecAttrAccessible as String] as? String
        onAdd?(account)
        return errSecSuccess
    }

    func update(_ query: CFDictionary, _ attributes: CFDictionary) -> OSStatus {
        let account = account(query)
        updatedAccounts.append(account)
        if let status = nextUpdateStatus {
            nextUpdateStatus = nil
            if status == errSecItemNotFound { items[account] = nil }
            return status
        }
        guard items[account] != nil else { return errSecItemNotFound }
        if let data = (attributes as NSDictionary)[kSecValueData as String] as? Data { items[account] = data }
        if let protection = (attributes as NSDictionary)[kSecAttrAccessible as String] as? String {
            protectionByAccount[account] = protection
        }
        return errSecSuccess
    }

    func copyMatching(_ query: CFDictionary, _ result: UnsafeMutablePointer<CFTypeRef?>) -> OSStatus {
        let account = account(query)
        copiedAccounts.append(account)
        if failingReadAccount == account {
            failingReadAccount = nil
            return errSecInteractionNotAllowed
        }
        guard let data = items[account] else { return errSecItemNotFound }
        if mismatchReadAccount == account {
            mismatchReadAccount = nil
            result.pointee = Data("wrong-value".utf8) as CFData
        } else {
            result.pointee = data as CFData
        }
        return errSecSuccess
    }

    func delete(_ query: CFDictionary) -> OSStatus {
        let account = account(query)
        deletedAccounts.append(account)
        onDelete?(account)
        if let status = nextDeleteStatus { nextDeleteStatus = nil; return status }
        guard items.removeValue(forKey: account) != nil else { return errSecItemNotFound }
        return errSecSuccess
    }
}

final class MigrationTestDefaults: UserDefaults, @unchecked Sendable {
    var rejectedKeys: Set<String> = []

    override func set(_ value: Any?, forKey defaultName: String) {
        if !rejectedKeys.contains(defaultName) { super.set(value, forKey: defaultName) }
    }
}
