import Observation
import SwiftUI

@MainActor
@Observable
final class AppModel {
    enum Phase: Equatable, Sendable {
        case disconnected
        case connecting
        case connected(host: String, username: String)
        case offline(host: String, username: String)
    }

    var phase: Phase = .disconnected
    var hasActiveSession: Bool
    var activeConfig: ConnectionConfig?
    var connectionBanner: ConnectionBanner?
    var lastError: String?
    var connectionProgressMessage = "Connecting…"
    var herdrMissingOnLastConnect = false
    var herdrSessionUnavailableOnLastConnect = false
    var herdrVersion: String?
    var connectionIdentity: ConnectionIdentity?
    var hostKeyChallenge: SSHHostKeyChallenge?
    var hostKeyVerificationBlocked = false
    var isSceneActive = true

    /// Production ownership primitive — unit tests exercise this same type.
    var ownership = ConnectionSessionOwnership()

    let phoneAnswers = PhoneAnswersStore()
    let connection: HerdrConnection
    weak var gatewayModel: AppModel?
    var selectedMachine: HerdrMachine?
    var selectedMachineModel: AppModel?
    var isShowingAllMachines = false
    var machines: [HerdrMachine] = []
    var machineDiscoveryError: String?
    var isDiscoveringMachines = false
    var machineSelectionRevision = 0
    /// Reuse each machine's input coordinator when switching away and back.
    var machineModels: [ConnectionIdentity: AppModel] = [:]
    var dismissBannerTask: Task<Void, Never>?
    var autoReconnectTask: Task<Void, Never>?
    var isDialing = false

    var machineCatalog: AppModel { gatewayModel ?? self }
    var supportsHostServices: Bool { selectedMachine == nil }
    var machineDisplayName: String {
        if let selectedMachine { return selectedMachine.label }
        if case let .connected(host, _) = phase { return host }
        if case let .offline(host, _) = phase { return host }
        return "Remote Machine"
    }

    init(
        connection: HerdrConnection = HerdrConnection(),
        autoConnectOnLaunch: Bool = true
    ) {
        self.connection = connection
        if autoConnectOnLaunch,
           let autoConfig = AutoConnect.configIfRequested() ?? LaunchPreference.autoConnectConfig() {
            self.hasActiveSession = true
            self.activeConfig = autoConfig
            self.phase = .disconnected
            self.connectionBanner = .connecting(message: "Connecting…")
        } else {
            self.hasActiveSession = false
            self.phase = .disconnected
        }
    }

    func discoverMachines() async {
        guard gatewayModel == nil, case .connected = phase, !isDiscoveringMachines else { return }
        let attempt = ownership.attempt
        isDiscoveringMachines = true
        defer { if ownership.isCurrent(attempt) { isDiscoveringMachines = false } }
        do {
            let result = try await connection.machineList()
            guard ownership.isCurrent(attempt), case .connected = phase else { return }
            machines = result
            machineDiscoveryError = nil
        } catch {
            guard ownership.isCurrent(attempt), case .connected = phase else { return }
            machineDiscoveryError = "Could not load saved machines. Check that Herdr on the remote machine "
                + "supports machine list. \(HerdrConnection.friendlyMessage(for: error))"
        }
    }

    func selectMachine(_ machine: HerdrMachine?) async {
        guard gatewayModel == nil, case .connected = phase else { return }
        machineSelectionRevision += 1
        let revision = machineSelectionRevision
        let attempt = ownership.attempt
        isShowingAllMachines = false
        guard let machine else {
            selectedMachineModel = nil
            return
        }
        guard machine.enabled else { return }
        do {
            let model = try await model(for: machine)
            guard ownership.isCurrent(attempt), machineSelectionRevision == revision else { return }
            selectedMachineModel = model
        } catch {
            guard ownership.isCurrent(attempt), machineSelectionRevision == revision else { return }
            machineDiscoveryError = HerdrConnection.friendlyMessage(for: error)
        }
    }

    func selectAllMachines() {
        guard gatewayModel == nil, case .connected = phase else { return }
        machineSelectionRevision += 1
        isShowingAllMachines = true
        selectedMachineModel = nil
    }

    /// Obtain a fixed target without changing the user's machine filter.
    func model(for machine: HerdrMachine) async throws -> AppModel {
        guard machine.enabled, let scope = connectionIdentity?.scoped(to: machine) else {
            throw HerdrError.notConnected
        }
        if let cached = machineModels[scope] {
            cached.selectedMachine = machine
            if case let .connected(_, username) = phase {
                cached.phase = .connected(host: machine.label, username: username)
            } else if case let .offline(_, username) = phase {
                cached.phase = .offline(host: machine.label, username: username)
            }
            return cached
        }
        let attempt = ownership.attempt
        let routedConnection = try await connection.connection(for: machine)
        guard ownership.isCurrent(attempt), case .connected = phase else { throw HerdrError.notConnected }
        // Another caller may have created this scope while the actor call waited.
        if let cached = machineModels[scope] { return cached }
        let model = AppModel(connection: routedConnection, autoConnectOnLaunch: false)
        model.gatewayModel = self
        model.selectedMachine = machine
        model.connectionIdentity = scope
        if case let .connected(_, username) = phase {
            model.phase = .connected(host: machine.label, username: username)
        } else if case let .offline(_, username) = phase {
            model.phase = .offline(host: machine.label, username: username)
        }
        machineModels[scope] = model
        return model
    }

    func resetMachines() {
        machineSelectionRevision += 1
        selectedMachineModel = nil
        isShowingAllMachines = false
        machineModels = [:]
        machines = []
        machineDiscoveryError = nil
        isDiscoveringMachines = false
    }

    /// Test seam: overrides the 30s connect watchdog sleep.
    var connectWatchdogNanoseconds: UInt64 = 30_000_000_000

    var isConnecting: Bool {
        if case .connecting = phase { return true }
        return false
    }

    var isOffline: Bool {
        if case .offline = phase { return true }
        return false
    }

    func showBackOnline(message: String = "Back online") {
        dismissBannerTask?.cancel()
        withAnimation(.easeInOut(duration: 0.25)) {
            connectionBanner = .backOnline(message: message)
        }
        dismissBannerTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(2.5))
            guard !Task.isCancelled else { return }
            withAnimation(.easeInOut(duration: 0.3)) {
                self?.connectionBanner = nil
            }
        }
    }

    func scheduleAutoReconnect(delaySeconds: Double = 2.5) {
        guard !hostKeyVerificationBlocked else { return }
        autoReconnectTask?.cancel()
        guard hasActiveSession, activeConfig != nil else { return }
        autoReconnectTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(delaySeconds))
            guard !Task.isCancelled, let self else { return }
            guard self.isOffline, self.hasActiveSession else { return }
            // The reconnect task is handing control to reconnect(). Clear its
            // slot first so reconnect() does not cancel the task that is now
            // performing the SSH attempt.
            self.autoReconnectTask = nil
            await self.reconnect(automatically: true)
        }
    }

    func cancelReconnect() {
        autoReconnectTask?.cancel()
        autoReconnectTask = nil
    }

    func reconnect(automatically: Bool = false) async {
        guard !automatically || !hostKeyVerificationBlocked else { return }
        guard let config = activeConfig else { return }
        guard !isDialing else { return }
        cancelReconnect()
        #if DEBUG
        print("[HC] reconnecting to \(config.host):\(config.port)")
        #endif
        connectionBanner = .connecting(message: "Reconnecting…")
        await connect(config: config)
    }
}
