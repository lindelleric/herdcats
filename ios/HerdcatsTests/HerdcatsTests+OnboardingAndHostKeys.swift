import Foundation
import Security
import NIOSSH
import SwiftUI
import Testing
@testable import Herdcats

@Suite("All machines grouping and routing")
struct AllMachinesTests {
    func machine(_ id: String, order: Int) -> SpaceMachine {
        let gateway = ConnectionIdentity(host: "gateway", port: 22, username: "owner")
        return SpaceMachine(scope: gateway.scoped(to: HerdrMachine(
            id: id, label: id, target: id, session: "default", enabled: true
        )), label: id, order: order)
    }

    func spaces(on machine: SpaceMachine) throws -> [Space] {
        let workspaces = try HerdrConnection.decode(workspaceListWithWorktreesJSON, as: WorkspaceListResult.self)
            .workspaces
        return Space.join(workspaces: workspaces, agents: []).map {
            var space = $0
            space.machine = machine
            return space
        }
    }

    func agent(status: String, sequence: Int) throws -> AgentEntry {
        let json = """
        {"agent":"codex","agent_status":"\(status)","pane_id":"w4:p1","tab_id":"w4:t1","workspace_id":"w4",
        "focused":false,"state_change_seq":\(sequence)}
        """
        return try JSONDecoder().decode(AgentEntry.self, from: Data(json.utf8))
    }

    @Test func identicalIDsAndRepoPathsStaySeparateAcrossMachines() throws {
        let first = try spaces(on: machine("Mac", order: 0))
        let second = try spaces(on: machine("Mini", order: 1))
        let groups = SpaceGroup.group(second + first)
        #expect(groups.count == 8)
        #expect(Set((first + second).map(\.id)).count == first.count + second.count)
        #expect(groups.prefix(4).allSatisfy { $0.root.machine?.label == "Mac" })
        #expect(groups.suffix(4).allSatisfy { $0.root.machine?.label == "Mini" })
        for group in groups {
            #expect(group.worktrees.allSatisfy { $0.machine == group.root.machine })
        }
        let roots = groups.filter { $0.root.workspace.workspaceId == "w4" }
        #expect(roots.count == 2)
        #expect(roots.allSatisfy { $0.worktrees.count == 2 })
    }

    @Test func agentNavigationIDsIncludeMachineWhileCLIIDsStayRaw() throws {
        let first = try #require(spaces(on: machine("Mac", order: 0)).first)
        let second = try #require(spaces(on: machine("Mini", order: 1)).first)
        let entry = try agent(status: "working", sequence: 1)
        let firstItem = AgentListItem(space: first, agent: entry)
        let secondItem = AgentListItem(space: second, agent: entry)
        #expect(firstItem.id != secondItem.id)
        #expect(firstItem.agent.paneId == secondItem.agent.paneId)
        #expect(first.workspace.workspaceId == second.workspace.workspaceId)
    }

    @Test func prioritySortDoesNotCompareSequenceCountersAcrossMachines() throws {
        var first = try #require(spaces(on: machine("Mac", order: 0)).first)
        var second = try #require(spaces(on: machine("Mini", order: 1)).first)
        first = Space(
            workspace: first.workspace, agents: [try agent(status: "working", sequence: 1)], machine: first.machine)
        second = Space(
            workspace: second.workspace, agents: [try agent(status: "working", sequence: 999)], machine: second.machine)
        let firstItem = AgentListItem(space: first, agent: first.agents[0])
        let secondItem = AgentListItem(space: second, agent: second.agents[0])
        #expect(
            [secondItem, firstItem].sorted(by: AgentListItem.attentionOrder).map(\.id)
                == [firstItem.id, secondItem.id]
        )
        let firstGroup = SpaceGroup(root: first, worktrees: [])
        let secondGroup = SpaceGroup(root: second, worktrees: [])
        #expect(
            [secondGroup, firstGroup].sorted(by: SpaceGroup.attentionOrder).map(\.id)
                == [firstGroup.id, secondGroup.id]
        )
        let blocked = AgentListItem(space: second, agent: try agent(status: "blocked", sequence: 0))
        #expect([firstItem, blocked].sorted(by: AgentListItem.attentionOrder).first == blocked)
        let blockedSpace = Space(workspace: second.workspace, agents: [blocked.agent], machine: second.machine)
        let blockedGroup = SpaceGroup(root: blockedSpace, worktrees: [])
        #expect([firstGroup, blockedGroup].sorted(by: SpaceGroup.attentionOrder).first?.id == blockedGroup.id)
        // Within the same machine, the existing newest-change ordering remains.
        let newer = AgentListItem(space: first, agent: try agent(status: "working", sequence: 2))
        #expect(AgentListItem.attentionOrder(newer, firstItem))
    }

    @MainActor
    @Test func missingMachineCannotFallBackToGateway() throws {
        let model = SpacesModel()
        let gateway = AppModel()
        let scoped = try #require(spaces(on: machine("removed", order: 1)).first)
        #expect(model.app(for: scoped, fallback: gateway) == nil)
        #expect(throws: HerdrError.self) {
            try model.connection(for: scoped, fallback: gateway.connection)
        }
        let local = Space(workspace: scoped.workspace, agents: [])
        #expect(model.app(for: local, fallback: gateway) === gateway)
        #expect(try model.connection(for: local, fallback: gateway.connection) === gateway.connection)
    }
}

@Suite("Onboarding entry")
struct OnboardingEntryTests {
    @Test func firstUseAndReturnRoutes() {
        #expect(OnboardingRoute.initial(hasSeenIntro: false, hasSavedMachines: false) == .intro)
        #expect(OnboardingRoute.initial(hasSeenIntro: true, hasSavedMachines: false) == .connectionSelection)
        #expect(OnboardingRoute.initial(hasSeenIntro: false, hasSavedMachines: true) == .connectionSelection)
        #expect(OnboardingRoute.initial(hasSeenIntro: true, hasSavedMachines: true) == .connectionSelection)
        #expect(OnboardingRoute.initial(hasSeenIntro: true, hasSavedMachines: true, replay: true) == .intro)
    }
}

@Suite("Auto-connect launch and offline status bar")
struct AutoConnectAndOfflineStatusBarTests {
    @MainActor
    @Test func transientReadinessTimeoutOnOfflineReconnectKeepsRetrying() async {
        for timeout in [HerdrError.timeout("herdr --version"), HerdrError.timeout("workspace list")] {
            let config = ConnectionConfig(
                host: "offline.example", port: 22, username: "tester", auth: .password("fixture")
            )
            let model = AppModel(autoConnectOnLaunch: false)
            model.hasActiveSession = true
            model.phase = .offline(host: config.host, username: config.username)
            guard let state = await model.prepareConnectAttempt(config: config) else {
                Issue.record("Could not prepare offline reconnect")
                return
            }
            state.watchdog.cancel()
            #expect(state.wasOffline)
            model.handleConnectFailure(
                timeout, config: config, attempt: state.attempt,
                wasOffline: state.wasOffline, isCheckingHerdr: true
            )
            #expect(model.hasActiveSession)
            #expect(model.phase == .offline(host: config.host, username: config.username))
            #expect(model.connectionBanner?.canRetry == true)
            #expect(model.autoReconnectTask != nil)
            model.cancelReconnect()
        }
    }

    @MainActor
    @Test func unrecoverableReadinessErrorsRouteToRecovery() async {
        for error in [HerdrError.herdrNotFound, .sessionUnavailable] {
            let config = ConnectionConfig(
                host: "offline.example", port: 22, username: "tester", auth: .password("fixture")
            )
            let model = AppModel(autoConnectOnLaunch: false)
            model.hasActiveSession = true
            model.phase = .offline(host: config.host, username: config.username)
            guard let state = await model.prepareConnectAttempt(config: config) else {
                Issue.record("Could not prepare offline reconnect")
                return
            }
            state.watchdog.cancel()
            model.handleConnectFailure(
                error, config: config, attempt: state.attempt,
                wasOffline: state.wasOffline, isCheckingHerdr: true
            )
            #expect(!model.hasActiveSession)
            #expect(model.phase == .disconnected)
            #expect(model.connectionBanner == nil)
            #expect(model.autoReconnectTask == nil)
        }
        #expect(AppModel.connectFailureDisposition(
            error: HerdrError.timeout("workspace list"),
            hadActiveSession: false,
            wasOffline: false,
            isCheckingHerdr: true
        ) == .readinessRecovery)
    }

    @MainActor
    @Test func sessionReadinessOnlyClassifiesMissingDefaultSession() {
        let serverError = HerdrError.api(code: "session_unavailable", message: "No default session")
        #expect(AppModel.sessionReadinessError(serverError) as? HerdrError == .sessionUnavailable)
        let missingSession = HerdrError.api(code: "other", message: "Default session not found")
        #expect(AppModel.sessionReadinessError(missingSession) as? HerdrError == .sessionUnavailable)

        let unsupported = HerdrError.api(code: "unsupported_command", message: "workspace list is unsupported")
        #expect(AppModel.sessionReadinessError(unsupported) as? HerdrError == unsupported)
        #expect(AppModel.sessionReadinessError(HerdrError.herdrExit(1)) as? HerdrError == .herdrExit(1))

        let decoding = HerdrError.unexpectedResponse("invalid JSON")
        #expect(AppModel.sessionReadinessError(decoding) as? HerdrError == decoding)
        let emptyOutput = HerdrError.unexpectedResponse("empty output")
        let friendlyMessage = HerdrConnection.friendlyMessage(for: AppModel.sessionReadinessError(emptyOutput))
        #expect(friendlyMessage == "Unexpected response from herdr — empty output")

        let quotaEmptyOutput = HerdrError.unexpectedResponse("quota-axi returned empty output")
        #expect(
            HerdrConnection.friendlyMessage(for: quotaEmptyOutput)
                == "quota-axi returned no data. Pull to refresh."
        )

        let timeout = HerdrError.timeout("herdr command")
        #expect(AppModel.sessionReadinessError(timeout) as? HerdrError == timeout)
        #expect(AppModel.sessionReadinessError(HerdrError.herdrNotFound) as? HerdrError == .herdrNotFound)
    }

    @MainActor
    @Test func appModelInitWithoutAutoConnectStartsDisconnected() {
        let model = AppModel(autoConnectOnLaunch: false)
        #expect(!model.hasActiveSession)
        #expect(model.phase == .disconnected)
        #expect(model.connectionBanner == nil)
        #expect(!model.isOffline)
        #expect(!model.isConnecting)
    }

    @Test func connectionBannerProperties() {
        let offline = ConnectionBanner.offline(message: "Offline · Connection lost", canRetry: true)
        #expect(offline.message == "Offline · Connection lost")
        #expect(offline.canRetry == true)
        #expect(!offline.showsProgress)
        #expect(offline.systemImage == "wifi.slash")

        let connecting = ConnectionBanner.connecting(message: "Connecting…")
        #expect(connecting.message == "Connecting…")
        #expect(!connecting.canRetry)
        #expect(connecting.showsProgress == true)
        #expect(connecting.systemImage == "arrow.clockwise")

        let backOnline = ConnectionBanner.backOnline(message: "Back online")
        #expect(backOnline.message == "Back online")
        #expect(!backOnline.canRetry)
        #expect(!backOnline.showsProgress)
        #expect(backOnline.systemImage == "checkmark.circle.fill")
    }

    @MainActor
    @Test func showBackOnlineDismissesBanner() async throws {
        let model = AppModel(autoConnectOnLaunch: false)
        model.showBackOnline(message: "Back online")
        #expect(model.connectionBanner == .backOnline(message: "Back online"))

        try await Task.sleep(for: .seconds(2.7))
        #expect(model.connectionBanner == nil)
    }

    @MainActor
    @Test func explicitDisconnectClearsSessionAndBanner() async {
        let model = AppModel(autoConnectOnLaunch: false)
        model.showBackOnline(message: "Connected")
        #expect(model.connectionBanner != nil)

        await model.disconnect()
        #expect(!model.hasActiveSession)
        #expect(model.phase == .disconnected)
        #expect(model.connectionBanner == nil)
    }
}

@Suite("SSH host key verification")
struct SSHHostKeyVerificationTests {
    private let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAABAgMEBQYHCAkKCwwNDg8QERITFBUWFxgZGhscHR4f"

    @Test func fingerprintMatchesOpenSSHFormat() throws {
        let parsed = try NIOSSHPublicKey(openSSHPublicKey: key)
        let canonical = String(openSSHPublicKey: parsed)
        #expect(SSHHostKeyValidatorDelegate.fingerprint(canonical)
                == "SHA256:ZkAslGjFiUHdGf/WUL8rQvkib4PTvQatUV0OUQSncCA")
    }

    @Test func unknownHostFailsWithExactApprovalCandidate() {
        let validator = SSHHostKeyValidatorDelegate(host: "server", port: 2222, trustedKey: nil)
        #expect(throws: SSHHostKeyError.unknown(SSHHostKeyChallenge(host: "server", port: 2222, publicKey: key))) {
            try validator.validate(key)
        }
    }

    @Test func matchingKeySucceedsAndChangedKeyFails() throws {
        let validator = SSHHostKeyValidatorDelegate(host: "server", port: 22, trustedKey: key)
        try validator.validate(key)
        let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        #expect(throws: SSHHostKeyError.changed(SSHHostKeyChallenge(
            host: "server", port: 22, publicKey: other, previousPublicKey: key))) {
            try validator.validate(other)
        }
        // Corrupt stored trust data must fail closed, never offer first-use approval.
        #expect(throws: SSHHostKeyError.self) {
            try SSHHostKeyValidatorDelegate(host: "server", port: 22, trustedKey: "corrupt").validate(key)
        }
    }

    @Test func endpointIdentityNormalizesHostAndSeparatesPorts() {
        #expect(
            SSHHostKeyStore.account(host: " SERVER ", port: 22) == SSHHostKeyStore.account(host: "server", port: 22))
        #expect(
            SSHHostKeyStore.account(host: "server", port: 22) != SSHHostKeyStore.account(host: "server", port: 2222))
        #expect(SSHHostKeyStore.account(host: "::1", port: 22) == "[::1]:22")
    }

    @Test func pinPersistsAndApprovalCannotOverwriteIt() async throws {
        let service = "com.enchantinglabs.herdrcat.tests.hostkeys.\(UUID().uuidString)"
        defer {
            let status = SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: service] as CFDictionary)
            #expect(status == errSecSuccess || status == errSecItemNotFound)
        }
        let store = SSHHostKeyStore(service: service)
        #expect(try await store.load(host: "server", port: 22) == nil)
        try await store.trust(key, host: "server", port: 22)
        try await store.trust(key, host: "SERVER", port: 22)
        let reopened = SSHHostKeyStore(service: service)
        #expect(try await reopened.load(host: "server", port: 22) == key)
        #expect(try await reopened.load(host: "server", port: 2222) == nil)
        #expect(try await reopened.load(host: "other", port: 22) == nil)
        await #expect(throws: SSHHostKeyStore.StoreError.self) {
            try await store.trust("different key", host: "server", port: 22)
        }
        #expect(try await store.load(host: "server", port: 22) == key)
    }

    @Test @MainActor func changedKeyFailureOffersReviewAndStopsAutomaticReconnect() async throws {
        let config = ConnectionConfig(host: "server", port: 22, username: "tester", auth: .password("fixture"))
        let model = AppModel(autoConnectOnLaunch: false)
        model.hasActiveSession = true
        model.phase = .offline(host: config.host, username: config.username)
        let state = try #require(await model.prepareConnectAttempt(config: config))
        state.watchdog.cancel()
        let challenge = SSHHostKeyChallenge(host: "server", port: 22,
            publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            previousPublicKey: key)
        model.handleConnectFailure(SSHHostKeyError.changed(challenge), config: config,
            attempt: state.attempt, wasOffline: state.wasOffline, isCheckingHerdr: false)
        #expect(model.hostKeyChallenge == challenge)
        #expect(model.hostKeyVerificationBlocked)
        #expect(model.autoReconnectTask == nil)
        model.scheduleAutoReconnect()
        #expect(model.autoReconnectTask == nil)
        await model.reconnect(automatically: true)
        #expect(model.hostKeyChallenge == challenge)
        #expect(model.hostKeyVerificationBlocked)
    }

    @Test func replacementAndLoadEnforceThisDeviceOnlyAccessibility() async throws {
        let service = "com.enchantinglabs.herdrcat.tests.hostkeys.\(UUID().uuidString)"
        defer { SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: service] as CFDictionary) }
        let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        let account = SSHHostKeyStore.account(host: "server", port: 22)
        func accessibility() throws -> String? {
            var result: AnyObject?
            let status = SecItemCopyMatching([
                kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account,
                kSecReturnAttributes: true, kSecMatchLimit: kSecMatchLimitOne,
            ] as CFDictionary, &result)
            try #require(status == errSecSuccess)
            return (result as? [String: Any])?[kSecAttrAccessible as String] as? String
        }
        let weaker = kSecAttrAccessibleAfterFirstUnlock as String
        let strict = kSecAttrAccessibleWhenUnlockedThisDeviceOnly as String

        // A pin saved by an older build under a weaker class is upgraded on load.
        let legacyStatus = SecItemAdd([
            kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account,
            kSecValueData: Data(key.utf8), kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlock,
        ] as CFDictionary, nil)
        try #require(legacyStatus == errSecSuccess)
        #expect(try accessibility() == weaker)
        let store = SSHHostKeyStore(service: service)
        #expect(try await store.load(host: "server", port: 22) == key)
        #expect(try accessibility() == strict)

        // Replacement keeps (and re-applies) the strict class.
        let reset = SecItemUpdate(
            [kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account] as CFDictionary,
            [kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlock] as CFDictionary)
        try #require(reset == errSecSuccess)
        try await store.replace(other, replacing: key, host: "server", port: 22)
        #expect(try accessibility() == strict)
    }

    @Test func replacementPersistsAndRejectsStaleOrMissingPins() async throws {
        let service = "com.enchantinglabs.herdrcat.tests.hostkeys.\(UUID().uuidString)"
        defer { SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: service] as CFDictionary) }
        let store = SSHHostKeyStore(service: service)
        let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        try await store.trust(key, host: "server", port: 22)
        try await store.trust(key, host: "server", port: 2222)
        try await store.trust(key, host: "other", port: 22)
        try await store.replace(other, replacing: key, host: "SERVER", port: 22)
        #expect(try await SSHHostKeyStore(service: service).load(host: "server", port: 22) == other)
        #expect(try await store.load(host: "server", port: 2222) == key)
        #expect(try await store.load(host: "other", port: 22) == key)
        await #expect(throws: SSHHostKeyStore.StoreError.self) {
            try await store.replace(key, replacing: key, host: "server", port: 22)
        }
        await #expect(throws: SSHHostKeyStore.StoreError.self) {
            try await store.replace(other, replacing: key, host: "missing", port: 22)
        }
        #expect(try await store.load(host: "missing", port: 22) == nil)
        #expect(try await store.load(host: "server", port: 22) == other)
        try SSHHostKeyValidatorDelegate(host: "server", port: 22, trustedKey: other).validate(other)
        #expect(throws: SSHHostKeyError.self) {
            try SSHHostKeyValidatorDelegate(host: "server", port: 22, trustedKey: other).validate(key)
        }
    }

    @Test @MainActor func cancellationAndStaleChallengesCannotReplacePin() async throws {
        let service = "com.enchantinglabs.herdrcat.tests.hostkeys.\(UUID().uuidString)"
        defer { SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: service] as CFDictionary) }
        let store = SSHHostKeyStore(service: service)
        try await store.trust(key, host: "server", port: 22)
        let challenge = SSHHostKeyChallenge(host: "server", port: 22,
            publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            previousPublicKey: key)
        let model = AppModel(autoConnectOnLaunch: false)
        model.hostKeyChallenge = challenge
        model.hostKeyVerificationBlocked = true
        model.cancelHostKeyApproval()
        #expect(model.hostKeyVerificationBlocked)
        #expect(await model.approveHostKey(challenge, store: store) == false)
        #expect(try await store.load(host: "server", port: 22) == key)
        model.hostKeyChallenge = challenge
        var stale = challenge
        stale.previousPublicKey = "stale pin"
        #expect(await model.approveHostKey(stale, store: store) == false)
        #expect(try await store.load(host: "server", port: 22) == key)
        #expect(await model.approveHostKey(challenge, store: store))
        #expect(model.hostKeyChallenge == nil)
        #expect(!model.hostKeyVerificationBlocked)
        #expect(try await store.load(host: "server", port: 22) == challenge.publicKey)
    }

    @Test func malformedKeychainItemIsNotAnUnknownHost() async throws {
        let service = "com.enchantinglabs.herdrcat.tests.hostkeys.\(UUID().uuidString)"
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: SSHHostKeyStore.account(host: "server", port: 22)
        ]
        defer {
            let status = SecItemDelete(query as CFDictionary)
            #expect(status == errSecSuccess || status == errSecItemNotFound)
        }
        var add = query
        add[kSecValueData as String] = Data([0xff])
        add[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        #expect(SecItemAdd(add as CFDictionary, nil) == errSecSuccess)
        let store = SSHHostKeyStore(service: service)
        await #expect(throws: SSHHostKeyStore.StoreError.self) {
            _ = try await store.load(host: "server", port: 22)
        }
    }
}
