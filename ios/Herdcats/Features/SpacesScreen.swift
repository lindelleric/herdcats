import Observation
import SwiftUI

// MARK: - Spaces model

@MainActor
@Observable
final class SpacesModel {
    private(set) var spaces: [Space] = [] {
        didSet {
            spaceGroups = SpaceGroup.group(spaces)
        }
    }
    private(set) var spaceGroups: [SpaceGroup] = []
    private(set) var machineSources: [MachineSpacesSource] = []
    private(set) var showsAllMachines = false

    @MainActor
    struct MachineSpacesSource: Identifiable {
        let machine: SpaceMachine
        let app: AppModel
        let model: SpacesModel
        var id: String { machine.id }
    }

    func bindMachines(_ catalog: AppModel) async {
        showsAllMachines = true
        var sources: [MachineSpacesSource] = []
        var apps = [catalog]
        for machine in catalog.machines where machine.enabled {
            if let app = try? await catalog.model(for: machine) { apps.append(app) }
        }
        guard !Task.isCancelled else { return }
        for (order, app) in apps.enumerated() {
            guard let scope = app.connectionIdentity else { continue }
            let child = machineSources.first { $0.id == scope.storageKey }?.model ?? SpacesModel()
            child.bindConnectionScope(scope)
            child.autoRefreshEnabled = autoRefreshEnabled
            sources.append(MachineSpacesSource(
                machine: SpaceMachine(scope: scope, label: app.machineDisplayName, order: order),
                app: app, model: child
            ))
        }
        machineSources = sources
        rebuildMachineSpaces()
    }

    func app(for space: Space, fallback: AppModel) -> AppModel? {
        guard let machine = space.machine else { return fallback }
        return machineSources.first { $0.id == machine.id }?.app
    }

    func model(for space: Space) -> SpacesModel {
        machineSources.first { $0.id == space.machine?.id }?.model ?? self
    }

    /// Latest pane activity among the space's agents — the list row's timestamp.
    func lastActivity(in space: Space) -> Date? {
        let dates = model(for: space).lastUpdatedByPaneID
        return space.agents.compactMap { dates[$0.paneId] }.max()
    }

    func connection(for space: Space?, fallback: HerdrConnection) throws -> HerdrConnection {
        guard let machine = space?.machine else { return fallback }
        guard let source = machineSources.first(where: { $0.id == machine.id }) else {
            throw HerdrError.invalidConfiguration("This machine is no longer available. Refresh the machine list.")
        }
        return source.app.connection
    }

    private func rebuildMachineSpaces() {
        spaces = machineSources.flatMap { source in
            source.model.spaces.map { space in
                var scoped = space
                scoped.machine = source.machine
                return scoped
            }
        }
        unconfirmedSpaces = SpacePresentationOverlay.unconfirmed(unconfirmedSpaces, after: spaces)
        let errors = machineSources.compactMap { source in
            source.model.errorMessage.map { "\(source.machine.label): \($0)" }
        }
        errorMessage = errors.isEmpty ? nil : errors.joined(separator: "\n")
    }

    private func refreshMachines() async {
        let generation = refreshGate.begin()
        isLoading = true
        defer {
            if refreshGate.finishIfCurrent(generation) { isLoading = false }
        }
        let sources = machineSources
        await withTaskGroup(of: Void.self) { group in
            for source in sources {
                group.addTask { await source.model.refresh(source.app.connection) }
            }
            for await _ in group {
                guard refreshGate.isCurrent(generation), !Task.isCancelled else { continue }
                rebuildMachineSpaces()
            }
        }
        guard refreshGate.isCurrent(generation), !Task.isCancelled else { return }
        lastUpdated = Date()
    }
    private(set) var lastUpdated: Date?
    /// Pane-id → last activity time, shared with pane detail cards.
    private(set) var lastUpdatedByPaneID: [String: Date] = [:]
    // The view can render before its initial refresh task starts.
    private(set) var isLoading = true

    /// An empty aggregate is conclusive only after every machine has loaded.
    var hasLoadedAgentList: Bool {
        if showsAllMachines {
            return !machineSources.isEmpty && machineSources.allSatisfy { $0.model.hasLoadedAgentList }
        }
        return lastUpdated != nil
    }
    /// Provider usage cards for the Agents header (from remote `quota-axi`).
    private(set) var usageCards: [AgentUsageCard] = []
    private(set) var usageLastUpdated: Date?
    private(set) var usageUnavailableMessage: String?
    /// True when the Herdr machine has no `quota-axi` binary to query.
    private(set) var quotaAxiMissing = false
    /// quota-axi provider ids with a card retry in flight.
    private(set) var retryingUsageProviders: Set<String> = []

    var errorMessage: String?

    /// Initial failures must expose Retry instead of looking like an endless load.
    var agentListError: String? {
        guard let errorMessage,
              !HerdrConnection.isDisconnectionOrTransitionMessage(errorMessage) else { return nil }
        return errorMessage
    }

    var showsAgentLoading: Bool {
        isLoading || (!hasLoadedAgentList && agentListError == nil)
    }
    var autoRefreshEnabled = true
    /// When false, periodic polls sleep without issuing remote commands.
    /// Initial / pull-to-refresh loads still run.
    var isSceneActive = true
    private(set) var isCreatingSpace = false
    private(set) var pendingSpaceCreationTitle: String?
    private(set) var lastPresentedSpaceID: String?
    private(set) var unconfirmedSpaces: [Space] = []
    private(set) var pendingWorktreeAction: PendingWorktreeAction?
    private(set) var highlightedWorktreeID: String?

    var visibleSpaceGroups: [SpaceGroup] {
        guard !unconfirmedSpaces.isEmpty else { return spaceGroups }
        return SpaceGroup.group(SpacePresentationOverlay.visibleSpaces(spaces, unconfirmed: unconfirmedSpaces))
    }

    /// Attention badge for the Agents tab — hidden while zero.
    var blockedAgentCount: Int {
        spaces.flatMap(\.agents).filter { AgentStatus(rawValue: $0.agentStatus) == .blocked }.count
    }

    private var autoRefreshTask: Task<Void, Never>?
    private var usageRefreshTask: Task<Void, Never>?
    private(set) var isRefreshingUsage = false
    @ObservationIgnored var quotaRetryFetch: ((String) async throws -> QuotaReport)?
    @ObservationIgnored var quotaReportFetch: (() async throws -> QuotaReport)?
    @ObservationIgnored var agentListFetch: (() async throws -> ([Workspace], [AgentEntry]))?
    private var refreshGate = RefreshGenerationGate()
    private var usageGate = RefreshGenerationGate()
    private var connectionScope: ConnectionIdentity?
    private var spaceCreationGeneration = 0
    private var worktreeActionGeneration = 0
    /// Agent statuses from the last successful refresh; `nil` until the first
    /// load so connecting never replays existing done / blocked states.
    private var alertBaseline: [String: AgentStatus]?
    /// Test seam: plays the user's chosen sound and haptic for an agent alert.
    var playAlert: (AgentAlertEvent) -> Void = { event in
        AgentAlertSound.selected(for: event).play()
        HapticFeedback.play(for: event)
    }

    func bindConnectionScope(_ scope: ConnectionIdentity?) {
        if connectionScope != scope {
            spaceCreationGeneration += 1
            isCreatingSpace = false
            pendingSpaceCreationTitle = nil
            lastPresentedSpaceID = nil
            unconfirmedSpaces = []
            worktreeActionGeneration += 1
            isPerformingWorktreeAction = false
            pendingWorktreeAction = nil
            highlightedWorktreeID = nil
        }
        connectionScope = scope
        alertBaseline = nil
        if let scope {
            lastUpdatedByPaneID = PaneLastUpdatedStore.dates(scope: scope)
        } else {
            lastUpdatedByPaneID = [:]
        }
    }

    func refresh(_ connection: HerdrConnection) async {
        if showsAllMachines {
            await refreshMachines()
            return
        }
        let generation = refreshGate.begin()
        isLoading = true

        // Watchdog: invalidate this generation so a late completion cannot
        // overwrite a newer refresh, then unlock the UI.
        let watchdog = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(45))
            guard let self else { return }
            guard self.refreshGate.watchdogTimeout(for: generation) else { return }
            self.isLoading = false
            self.errorMessage = "Refreshing timed out — the connection may be unhealthy."
        }
        defer {
            watchdog.cancel()
            if refreshGate.finishIfCurrent(generation) {
                isLoading = false
            }
        }

        do {
            let workspaceList: [Workspace]
            let agentList: [AgentEntry]
            if let agentListFetch {
                (workspaceList, agentList) = try await agentListFetch()
            } else {
                async let workspaces = connection.workspaceList()
                async let agents = connection.agentList()
                (workspaceList, agentList) = try await (workspaces, agents)
            }
            guard refreshGate.isCurrent(generation) else { return }
            spaces = Space.join(workspaces: workspaceList, agents: agentList)
            unconfirmedSpaces = SpacePresentationOverlay.unconfirmed(
                unconfirmedSpaces,
                after: spaces
            )
            if let event = AgentStatusAlerts.event(previous: alertBaseline, current: agentList) {
                playAlert(event)
            }
            alertBaseline = AgentStatusAlerts.snapshot(agentList)
            let fallbackScope: ConnectionIdentity?
            if let connectionScope {
                fallbackScope = connectionScope
            } else {
                fallbackScope = await connection.identity
            }
            if let scope = fallbackScope {
                connectionScope = scope
                lastUpdatedByPaneID = PaneLastUpdatedStore.observe(agentList, scope: scope)
            }
            lastUpdated = Date()
            errorMessage = nil
        } catch {
            guard refreshGate.isCurrent(generation) else { return }
            reportError(error)
        }
    }

    private func reportError(_ error: Error) {
        let message = HerdrConnection.friendlyMessage(for: error)
        if !HerdrConnection.isDisconnectionOrTransitionMessage(message) {
            errorMessage = message
        }
    }

    /// Creates a new space on the Herdr machine. When `base` is set, the new
    /// space opens in that space's directory; `nil` lets herdr pick the
    /// default directory.
    func createSpace(basedOn base: Space?, _ connection: HerdrConnection) async {
        guard !isCreatingSpace else { return }
        spaceCreationGeneration += 1
        let generation = spaceCreationGeneration
        isCreatingSpace = true
        errorMessage = nil
        lastPresentedSpaceID = nil
        withAnimation(.snappy(duration: 0.28)) {
            pendingSpaceCreationTitle = base.map { "Creating space from \($0.label)…" } ?? "Creating space…"
        }
        defer {
            if spaceCreationGeneration == generation {
                isCreatingSpace = false
                withAnimation(.snappy(duration: 0.28)) {
                    pendingSpaceCreationTitle = nil
                }
            }
        }
        do {
            let target = try self.connection(for: base, fallback: connection)
            let cwd = try await Self.baseWorkingDirectory(for: base, connection: target)
            guard spaceCreationGeneration == generation, !Task.isCancelled else { return }
            let created = (try await target.workspaceCreate(cwd: cwd)).workspace
            guard spaceCreationGeneration == generation, !Task.isCancelled else { return }
            withAnimation(.snappy(duration: 0.28)) {
                if !spaces.contains(where: { $0.workspace.workspaceId == created.workspaceId }) {
                    unconfirmedSpaces.append(Space(workspace: created, agents: []))
                }
                pendingSpaceCreationTitle = nil
                lastPresentedSpaceID = created.workspaceId
            }
            await refresh(connection)
        } catch {
            guard spaceCreationGeneration == generation, !Task.isCancelled else { return }
            reportError(error)
        }
    }

    private(set) var isPerformingWorktreeAction = false

    /// Finds the default (root / primary non-linked) space for the repository of a given space,
    /// falling back to the space itself if none is found.
    func defaultSpace(for space: Space, in spaces: [Space]? = nil) -> Space {
        let pool = spaces ?? self.spaces
        guard let repoRoot = space.workspace.worktree?.repoRoot, !repoRoot.isEmpty else {
            return space
        }
        return pool.first { candidate in
            candidate.machine?.id == space.machine?.id &&
            candidate.workspace.worktree?.repoRoot == repoRoot &&
            candidate.workspace.worktree?.isLinkedWorktree == false
        } ?? space
    }

    /// Opens an existing Git worktree in the specified space.
    func openWorktree(
        in space: Space,
        path: String? = nil,
        branch: String? = nil,
        _ connection: HerdrConnection
    ) async {
        guard !isPerformingWorktreeAction else { return }
        worktreeActionGeneration += 1
        let generation = worktreeActionGeneration
        let targetSpace = defaultSpace(for: space)
        isPerformingWorktreeAction = true
        errorMessage = nil
        lastPresentedSpaceID = nil
        highlightedWorktreeID = nil
        withAnimation(.snappy(duration: 0.28)) {
            pendingWorktreeAction = PendingWorktreeAction(
                sourceSpaceID: targetSpace.id,
                title: PendingWorktreeAction.openTitle(path: path, branch: branch)
            )
        }
        defer { finishWorktreeAction(generation) }
        do {
            let conn = try self.connection(for: targetSpace, fallback: connection)
            let result: WorktreeOpenResult
            if targetSpace.workspace.worktree?.isLinkedWorktree == false {
                result = try await conn.worktreeOpen(
                    workspaceId: targetSpace.workspace.workspaceId,
                    path: path,
                    branch: branch
                )
            } else if let repoRoot = targetSpace.workspace.worktree?.repoRoot, !repoRoot.isEmpty {
                result = try await conn.worktreeOpen(
                    cwd: repoRoot,
                    path: path,
                    branch: branch
                )
            } else {
                result = try await conn.worktreeOpen(
                    workspaceId: targetSpace.workspace.workspaceId,
                    path: path,
                    branch: branch
                )
            }
            guard worktreeActionGeneration == generation, !Task.isCancelled else { return }
            presentWorktree(result.workspace, entry: result.worktree, source: targetSpace)
            await refresh(connection)
        } catch {
            guard worktreeActionGeneration == generation, !Task.isCancelled else { return }
            reportError(error)
        }
    }

    /// Creates and opens a new Git worktree based on the specified space.
    func createWorktree(
        in space: Space,
        name: String? = nil,
        label: String? = nil,
        _ connection: HerdrConnection
    ) async {
        guard !isPerformingWorktreeAction else { return }
        worktreeActionGeneration += 1
        let generation = worktreeActionGeneration
        let targetSpace = defaultSpace(for: space)
        isPerformingWorktreeAction = true
        errorMessage = nil
        lastPresentedSpaceID = nil
        highlightedWorktreeID = nil
        withAnimation(.snappy(duration: 0.28)) {
            pendingWorktreeAction = PendingWorktreeAction(
                sourceSpaceID: targetSpace.id,
                title: PendingWorktreeAction.createTitle(name: name, label: label)
            )
        }
        defer { finishWorktreeAction(generation) }
        do {
            let trimmedName = name?.trimmingCharacters(in: .whitespacesAndNewlines)
            let trimmedLabel = label?.trimmingCharacters(in: .whitespacesAndNewlines)
            let branch = trimmedName?.isEmpty == false ? trimmedName : nil
            let labelText = trimmedLabel?.isEmpty == false ? trimmedLabel : nil
            let conn = try self.connection(for: targetSpace, fallback: connection)

            let result: WorktreeCreateResult
            if targetSpace.workspace.worktree?.isLinkedWorktree == false {
                result = try await conn.worktreeCreate(
                    workspaceId: targetSpace.workspace.workspaceId,
                    branch: branch,
                    label: labelText
                )
            } else if let repoRoot = targetSpace.workspace.worktree?.repoRoot, !repoRoot.isEmpty {
                result = try await conn.worktreeCreate(
                    cwd: repoRoot,
                    branch: branch,
                    label: labelText
                )
            } else {
                result = try await conn.worktreeCreate(
                    workspaceId: targetSpace.workspace.workspaceId,
                    branch: branch,
                    label: labelText
                )
            }
            guard worktreeActionGeneration == generation, !Task.isCancelled else { return }
            presentWorktree(result.workspace, entry: result.worktree, source: targetSpace)
            await refresh(connection)
        } catch {
            guard worktreeActionGeneration == generation, !Task.isCancelled else { return }
            reportError(error)
        }
    }

    private func presentWorktree(_ workspace: Workspace, entry: WorktreeEntry?, source: Space) {
        let presented = WorktreePresentation.space(workspace, entry: entry, source: source)
        withAnimation(.snappy(duration: 0.28)) {
            if !spaces.contains(where: { $0.id == presented.id }) {
                unconfirmedSpaces.append(presented)
            }
            pendingWorktreeAction = nil
            lastPresentedSpaceID = presented.id
            highlightedWorktreeID = presented.id
        }
        let highlightedID = presented.id
        let generation = worktreeActionGeneration
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard self?.worktreeActionGeneration == generation,
                  self?.highlightedWorktreeID == highlightedID else { return }
            withAnimation(.easeOut(duration: 0.3)) {
                self?.highlightedWorktreeID = nil
            }
        }
    }

    private func finishWorktreeAction(_ generation: Int) {
        guard worktreeActionGeneration == generation else { return }
        isPerformingWorktreeAction = false
        withAnimation(.snappy(duration: 0.28)) {
            pendingWorktreeAction = nil
        }
    }

    private(set) var isPerformingSpaceAction = false

    /// Renames a space (Herdr workspace label).
    func renameSpace(_ space: Space, to name: String, _ connection: HerdrConnection) async {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !isPerformingSpaceAction else { return }
        isPerformingSpaceAction = true
        defer { isPerformingSpaceAction = false }
        do {
            try await self.connection(for: space, fallback: connection).workspaceRename(workspaceId: space.workspace.workspaceId, label: trimmed)
            await refresh(connection)
        } catch {
            reportError(error)
        }
    }

    /// Closes a space and removes it from the list after refresh.
    func closeSpace(_ space: Space, _ connection: HerdrConnection) async {
        guard !isPerformingSpaceAction else { return }
        isPerformingSpaceAction = true
        defer { isPerformingSpaceAction = false }
        do {
            try await self.connection(for: space, fallback: connection).workspaceClose(workspaceId: space.workspace.workspaceId)
            await refresh(connection)
        } catch {
            reportError(error)
        }
    }

    /// Directory a new space inherits from the space it is based on: the
    /// checkout path when the space has repo metadata, else the first live
    /// pane's working directory.
    private static func baseWorkingDirectory(
        for base: Space?,
        connection: HerdrConnection
    ) async throws -> String? {
        guard let base else { return nil }
        if let checkout = base.workspace.worktree?.checkoutPath, !checkout.isEmpty {
            return checkout
        }
        let panes = try await connection.paneList(workspaceId: base.workspace.workspaceId)
        return panes.first { $0.cwd?.isEmpty == false }?.cwd
    }
    /// Rate-limited unless `force` is set (pull-to-refresh / periodic poll).
    func refreshUsage(_ connection: HerdrConnection, force: Bool = false) async {
        guard !showsAllMachines else {
            withAnimation(.spring(response: 0.38, dampingFraction: 0.82)) {
                usageCards = []
                quotaAxiMissing = false
                usageUnavailableMessage = "Select a specific machine to view its quota."
            }
            return
        }
        guard connection.supportsHostServices else {
            withAnimation(.spring(response: 0.38, dampingFraction: 0.82)) {
                usageCards = []
                quotaAxiMissing = false
                usageUnavailableMessage = "Switch to your primary remote machine to view agent quota."
            }
            return
        }
        guard !isRefreshingUsage else { return }
        if !force, let last = usageLastUpdated, Date().timeIntervalSince(last) < 50 {
            return
        }
        let generation = usageGate.begin()
        isRefreshingUsage = true
        defer {
            if usageGate.finishIfCurrent(generation) {
                isRefreshingUsage = false
            }
        }

        do {
            let report: QuotaReport
            if let quotaReportFetch {
                report = try await quotaReportFetch()
            } else {
                report = try await connection.quotaReport()
            }
            guard usageGate.isCurrent(generation) else { return }
            withAnimation(.spring(response: 0.38, dampingFraction: 0.82)) {
                applyUsageReport(report)
            }
        } catch let error as HerdrError where error == .quotaAxiNotFound {
            guard usageGate.isCurrent(generation) else { return }
            withAnimation(.spring(response: 0.38, dampingFraction: 0.82)) {
                usageCards = []
                usageUnavailableMessage = nil
                quotaAxiMissing = true
            }
        } catch {
            guard usageGate.isCurrent(generation) else { return }
            // Keep the last good snapshot; only surface a soft note when empty.
            quotaAxiMissing = false
            if usageCards.isEmpty {
                let raw = HerdrConnection.friendlyMessage(for: error)
                if raw.localizedCaseInsensitiveContains("commandfailed")
                    || raw.localizedCaseInsensitiveContains("exitcode") {
                    usageUnavailableMessage = "Could not read quota-axi on this remote machine. Pull to refresh."
                } else {
                    usageUnavailableMessage = raw
                }
            }
        }
    }

    func applyUsageReport(_ report: QuotaReport) {
        usageCards = report.usageCards()
        usageLastUpdated = Date()
        usageUnavailableMessage = nil
        quotaAxiMissing = false
    }

    /// Re-reads one provider's quota for its unavailable card's retry button.
    func retryUsage(provider id: String, connection: HerdrConnection) async {
        if let quotaRetryFetch {
            await retryUsage(provider: id, fetch: quotaRetryFetch)
            return
        }
        await retryUsage(provider: id) { try await connection.quotaReport(provider: $0) }
    }

    /// Test seam for `retryUsage(provider:connection:)`. A newer full refresh
    /// wins; a failed retry keeps the card in its unavailable state.
    func retryUsage(provider id: String, fetch: (String) async throws -> QuotaReport) async {
        guard !isRefreshingUsage, !retryingUsageProviders.contains(id) else { return }
        retryingUsageProviders.insert(id)
        defer { retryingUsageProviders.remove(id) }
        let generation = usageGate.generation
        guard let report = try? await fetch(id) else { return }
        guard usageGate.isCurrent(generation), !isRefreshingUsage, !usageCards.isEmpty else { return }
        withAnimation(.spring(response: 0.38, dampingFraction: 0.82)) {
            usageCards = AgentUsageCard.merging(usageCards, quotaProvider: id, report: report)
        }
    }

    func startAutoRefresh(_ connection: HerdrConnection) {
        stopAutoRefresh()
        autoRefreshTask = Task { [weak self] in
            while !Task.isCancelled {
                do {
                    try await Task.sleep(for: .seconds(5))
                } catch {
                    return
                }
                guard let self else { return }
                guard self.autoRefreshEnabled, self.isSceneActive else { continue }
                await self.refresh(connection)
            }
        }
        // Quota changes slowly; poll independently so the 5s herdr loop stays light.
        usageRefreshTask = Task { [weak self] in
            // First fetch shortly after connect (spaces refresh runs in parallel).
            await self?.refreshUsage(connection, force: true)
            while !Task.isCancelled {
                do {
                    try await Task.sleep(for: .seconds(60))
                } catch {
                    return
                }
                guard let self else { return }
                guard self.autoRefreshEnabled, self.isSceneActive else { continue }
                await self.refreshUsage(connection, force: true)
            }
        }
    }

    func stopAutoRefresh() {
        autoRefreshTask?.cancel()
        autoRefreshTask = nil
        usageRefreshTask?.cancel()
        usageRefreshTask = nil
    }
}

struct PendingWorktreeAction: Equatable {
    static let viewID = "worktree-pending-row"

    let sourceSpaceID: String
    let title: String

    static func openTitle(path: String?, branch: String?) -> String {
        let subject = [branch, path.map { ($0 as NSString).lastPathComponent }]
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }
        guard let subject else { return "Opening worktree…" }
        return "Opening worktree “\(subject)”…"
    }

    static func createTitle(name: String?, label: String?) -> String {
        let subject = [label, name]
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }
        guard let subject else { return "Creating worktree…" }
        return "Creating worktree “\(subject)”…"
    }
}

enum WorktreePresentation {
    static func space(_ workspace: Workspace, entry: WorktreeEntry?, source: Space) -> Space {
        guard workspace.worktree == nil,
              let entry,
              let sourceWorktree = source.workspace.worktree else {
            return Space(workspace: workspace, agents: [], machine: source.machine)
        }
        let completed = Workspace(
            workspaceId: workspace.workspaceId,
            label: workspace.label,
            number: workspace.number,
            tabCount: workspace.tabCount,
            paneCount: workspace.paneCount,
            activeTabId: workspace.activeTabId,
            agentStatus: workspace.agentStatus,
            focused: workspace.focused,
            worktree: Worktree(
                checkoutPath: entry.path,
                repoName: sourceWorktree.repoName,
                repoRoot: sourceWorktree.repoRoot,
                isLinkedWorktree: entry.isLinkedWorktree ?? true
            )
        )
        return Space(workspace: completed, agents: [], machine: source.machine)
    }
}

/// Holds successful command responses on screen until a list refresh confirms them.
enum SpacePresentationOverlay {
    static func visibleSpaces(_ spaces: [Space], unconfirmed: [Space]) -> [Space] {
        let listedIDs = Set(spaces.map(\.id))
        return spaces + unconfirmed.filter { !listedIDs.contains($0.id) }
    }

    static func unconfirmed(_ pending: [Space], after spaces: [Space]) -> [Space] {
        let listedIDs = Set(spaces.map(\.id))
        return pending.filter { !listedIDs.contains($0.id) }
    }
}

// MARK: - Spaces screen

struct SpacesScreen: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let model: SpacesModel

    @AppStorage(SpaceSortPreference.storageKey)
    private var sortRaw = SpaceSortPreference.default.rawValue

    @State private var isShowingCreateSpace = false
    /// Set when the user picks an option in the drawer; executed in
    /// `onDismiss` so the async work is not cancelled by sheet teardown.
    @State private var pendingCreateSpace: CreateSpaceRequest?

    @State private var openWorktreeTarget: Space?
    @State private var pendingOpenWorktree: OpenWorktreeRequest?

    @State private var createWorktreeTarget: Space?
    @State private var pendingCreateWorktree: CreateWorktreeRequest?

    @State private var renameTarget: Space?
    @State private var renameText = ""
    @State private var isShowingRenameAlert = false
    @State private var closeTarget: Space?
    @State private var isShowingCloseConfirmation = false
    /// Space whose swipe "More" drawer is open.
    @State private var actionsTarget: Space?
    /// Chosen in the drawer; performed in `onDismiss` so the next sheet or
    /// alert is not presented while the drawer is still on screen.
    @State private var pendingSpaceAction: (action: SpaceAction, space: Space)?

    private var sort: SpaceSortPreference {
        SpaceSortPreference(rawValue: sortRaw) ?? .default
    }

    private var sortSelection: Binding<SpaceSortPreference> {
        Binding(
            get: { SpaceSortPreference(rawValue: sortRaw) ?? .default },
            set: { sortRaw = $0.rawValue }
        )
    }

    private var displayedGroups: [SpaceGroup] {
        switch sort {
        case .workspaceOrder:
            model.visibleSpaceGroups
        case .priority:
            model.visibleSpaceGroups.sorted(by: SpaceGroup.attentionOrder)
        }
    }

    private var cardOrder: [ReorderingCard] {
        displayedGroups.map { group in
            let latestChange = group.members.flatMap { $0.agents.compactMap(\.stateChangeSeq) }.max()
            return ReorderingCard(
                id: group.id,
                priorityKey: "\(group.status.rawValue)|\(latestChange.map { String($0) } ?? "")"
            )
        }
    }

    var body: some View {
        @Bindable var model = model

        return NavigationStack {
            Group {
                if model.spaces.isEmpty && model.unconfirmedSpaces.isEmpty
                    && model.pendingSpaceCreationTitle == nil {
                    if appModel.isOffline {
                        OfflineHerdView()
                    } else if model.isLoading {
                        LazyCatLoadingView(message: "Loading spaces…")
                    } else {
                        emptyState
                    }
                } else {
                    ScrollViewReader { proxy in
                        List {
                            if let title = model.pendingSpaceCreationTitle {
                                PendingSpaceRow(title: title)
                                    .quietListRow()
                                    .transition(.opacity.combined(with: .scale(scale: 0.95)))
                            }
                            let groups = displayedGroups
                            ForEach(Array(groups.enumerated()), id: \.element.id) { index, group in
                                if model.showsAllMachines, sort == .workspaceOrder,
                                   index == 0 || groups[index - 1].root.machine?.id != group.root.machine?.id {
                                    MachineSectionHeader(name: group.root.machine?.label ?? "Machine")
                                        .quietListRow()
                                }
                                spaceLink(group.root) {
                                    SpaceRow(
                                        group: group,
                                        machineLabel: model.showsAllMachines && sort == .priority ? group.root.machine?.label : nil,
                                        isHighlighted: model.highlightedWorktreeID == group.root.id,
                                        lastUpdated: model.lastActivity(in: group.root)
                                    )
                                }
                                .accessibilityIdentifier("space-row-\(group.root.workspace.number)")
                                .id(group.id)
                                ForEach(group.worktrees) { space in
                                    spaceLink(space) {
                                        WorktreeRow(
                                            space: space,
                                            isHighlighted: model.highlightedWorktreeID == space.id,
                                            lastUpdated: model.lastActivity(in: space)
                                        )
                                    }
                                }
                                if let pending = model.pendingWorktreeAction,
                                   pending.sourceSpaceID == group.root.id {
                                    PendingWorktreeRow(action: pending)
                                        .quietListRow()
                                }
                            }
                        }
                        .listStyle(.plain)
                        .scrollContentBackground(.hidden)
                        .environment(\.defaultMinListRowHeight, 0)
                        .contentMargins(.top, 4, for: .scrollContent)
                        .contentMargins(.bottom, 44, for: .scrollContent)
                        .animation(reduceMotion ? nil : .spring(duration: 0.5, bounce: 0.12), value: cardOrder.map(\.id))
                        .onChange(of: model.lastPresentedSpaceID) { _, spaceID in
                            guard let spaceID,
                                  let group = displayedGroups.first(where: {
                                      $0.members.contains { $0.id == spaceID }
                                  }) else { return }
                            withAnimation(.snappy(duration: 0.28)) {
                                proxy.scrollTo(group.id, anchor: .center)
                            }
                        }
                        .onChange(of: model.pendingWorktreeAction) { _, pending in
                            guard pending != nil else { return }
                            withAnimation(.snappy(duration: 0.28)) {
                                proxy.scrollTo(PendingWorktreeAction.viewID, anchor: .center)
                            }
                        }
                    }
                }
            }
            .background(Theme.listBackground.ignoresSafeArea())
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        Task { await appModel.disconnect() }
                    } label: {
                        Image(systemName: "chevron.left")
                            .font(.jost(16, weight: .semibold))
                            .foregroundStyle(.primary)
                    }
                    .accessibilityLabel("Back to Connections")
                    .accessibilityIdentifier("spaces-back-button")
                }
                ToolbarItem(placement: .principal) {
                    HerdrcatBrandMark(showsHost: true, title: "Spaces")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button {
                            isShowingCreateSpace = true
                        } label: {
                            Label("New Space", systemImage: "plus")
                        }
                        .disabled(model.isCreatingSpace || model.showsAllMachines)
                        .accessibilityIdentifier("space-create-button")

                        if model.showsAllMachines {
                            Text("Select a machine to create a space")
                        }
                        Divider()

                        Picker("Sort", selection: sortSelection) {
                            ForEach(SpaceSortPreference.allCases) { preference in
                                Label(preference.title, systemImage: preference.systemImage)
                                    .tag(preference)
                            }
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                            .font(.jost(16, weight: .medium))
                            .foregroundStyle(.primary)
                    }
                    .accessibilityLabel("Space List Options")
                    .accessibilityIdentifier("space-list-options-menu")
                }
            }
            .toolbarBackground(.hidden, for: .navigationBar)
            .navigationDestination(for: Space.self) { space in
                if let target = model.app(for: space, fallback: appModel) {
                    WorkspacePaneView(space: space)
                        .environment(target)
                        .environment(model.model(for: space))
                } else {
                    ContentUnavailableView("Machine Unavailable", systemImage: "desktopcomputer.trianglebadge.exclamationmark")
                }
            }
            .sheet(item: $actionsTarget, onDismiss: {
                guard let pending = pendingSpaceAction else { return }
                pendingSpaceAction = nil
                perform(pending.action, on: pending.space)
            }) { space in
                SpaceActionsDrawer(
                    space: space,
                    isWorktreeActionDisabled: model.isPerformingWorktreeAction,
                    onSelect: { action in
                        pendingSpaceAction = (action, space)
                        actionsTarget = nil
                    },
                    onCancel: {
                        pendingSpaceAction = nil
                        actionsTarget = nil
                    }
                )
                .presentationDetents([.medium])
                .presentationDragIndicator(.visible)
                .presentationCornerRadius(20)
            }
            .sheet(isPresented: $isShowingCreateSpace, onDismiss: {
                guard let request = pendingCreateSpace else { return }
                pendingCreateSpace = nil
                Task { await model.createSpace(basedOn: request.base, appModel.connection) }
            }) {
                CreateSpaceDrawer(
                    spaces: model.spaces,
                    onCreate: { base in
                        pendingCreateSpace = CreateSpaceRequest(base: base)
                        isShowingCreateSpace = false
                    },
                    onCancel: {
                        pendingCreateSpace = nil
                        isShowingCreateSpace = false
                    }
                )
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
                .presentationCornerRadius(20)
            }
            .sheet(item: $openWorktreeTarget, onDismiss: {
                guard let request = pendingOpenWorktree else { return }
                pendingOpenWorktree = nil
                Task {
                    await model.openWorktree(
                        in: request.space,
                        path: request.path,
                        branch: request.branch,
                        appModel.connection
                    )
                }
            }) { space in
                if let target = model.app(for: space, fallback: appModel) {
                    OpenWorktreeDrawer(
                        space: space,
                        connection: target.connection,
                        onOpen: { path, branch in
                            pendingOpenWorktree = OpenWorktreeRequest(space: space, path: path, branch: branch)
                            openWorktreeTarget = nil
                        },
                        onCancel: {
                            pendingOpenWorktree = nil
                            openWorktreeTarget = nil
                        }
                    )
                    .presentationDetents([.medium, .large])
                    .presentationDragIndicator(.visible)
                    .presentationCornerRadius(20)
                }
            }
            .sheet(item: $createWorktreeTarget, onDismiss: {
                guard let request = pendingCreateWorktree else { return }
                pendingCreateWorktree = nil
                Task {
                    await model.createWorktree(
                        in: request.space,
                        name: request.name,
                        label: request.label,
                        appModel.connection
                    )
                }
            }) { space in
                CreateWorktreeDrawer(
                    space: space,
                    onCreate: { name, label in
                        pendingCreateWorktree = CreateWorktreeRequest(
                            space: space,
                            name: name,
                            label: label
                        )
                        createWorktreeTarget = nil
                    },
                    onCancel: {
                        pendingCreateWorktree = nil
                        createWorktreeTarget = nil
                    }
                )
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
                .presentationCornerRadius(20)
            }
            .alert("Rename Space", isPresented: $isShowingRenameAlert) {
                TextField("Space Name", text: $renameText)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("space-rename-text-field")
                Button("Rename") {
                    let target = renameTarget
                    let name = renameText
                    renameTarget = nil
                    guard let target else { return }
                    Task {
                        await model.renameSpace(target, to: name, appModel.connection)
                    }
                }
                .accessibilityIdentifier("space-rename-confirm-button")
                Button("Cancel", role: .cancel) {
                    renameTarget = nil
                }
            } message: {
                Text("Enter a new name for this space.")
            }
            .confirmationDialog(
                "Close Space",
                isPresented: $isShowingCloseConfirmation,
                titleVisibility: .visible
            ) {
                Button("Close Space", role: .destructive) {
                    let target = closeTarget
                    closeTarget = nil
                    guard let target else { return }
                    Task {
                        await model.closeSpace(target, appModel.connection)
                    }
                }
                .accessibilityIdentifier("space-close-confirm-button")
                Button("Cancel", role: .cancel) {
                    closeTarget = nil
                }
            } message: {
                if let closeTarget {
                    Text("Are you sure you want to close “\(closeTarget.label)”? This will close its tabs and panes.")
                } else {
                    Text("Are you sure you want to close this space? This will close its tabs and panes.")
                }
            }
            .overlay(alignment: .bottom) {
                if let error = model.errorMessage,
                   case .connected = appModel.phase,
                   !HerdrConnection.isDisconnectionOrTransitionMessage(error) {
                    VStack {
                        Spacer()
                        ErrorBanner(message: error) {
                            Task { await model.refresh(appModel.connection) }
                        }
                        .padding(.horizontal, 16)
                        .padding(.bottom, 8)
                    }
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .animation(.easeInOut(duration: 0.25), value: model.errorMessage == nil)
            .refreshable {
                async let spaces: Void = model.refresh(appModel.connection)
                async let usage: Void = model.refreshUsage(appModel.connection, force: true)
                _ = await (spaces, usage)
            }
            .connectionStatusBanner(appModel: appModel)
        }
    }

    // MARK: Pieces

    /// Makes a space row open the space and adds the trailing swipe
    /// actions: More (the actions drawer) and Close.
    private func spaceLink<Row: View>(_ space: Space, @ViewBuilder row: () -> Row) -> some View {
        // A direct NavigationLink keeps the tap on UIKit's cell selection.
        // Any SwiftUI gesture on the row content (tap, Button, drag) can win
        // over the List's swipe recognizer on device and open the space.
        NavigationLink(value: space) {
            row()
        }
        .hidesNavigationIndicator()
        .quietListRow()
        // WhatsApp-style: drag right to left; Close sits at the screen edge.
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            swipeCloseButton(space)
            swipeMoreButton(space)
        }
    }

    private func swipeMoreButton(_ space: Space) -> some View {
        Button {
            actionsTarget = space
        } label: {
            Label("More", systemImage: "ellipsis")
                .labelStyle(.iconOnly)
        }
        .tint(Theme.subtleFill)
        .buttonStyle(.automatic)
        .accessibilityIdentifier("space-swipe-more-\(space.id)")
    }

    private func swipeCloseButton(_ space: Space) -> some View {
        Button {
            perform(.close, on: space)
        } label: {
            Label("Close", systemImage: "xmark")
                .labelStyle(.iconOnly)
        }
        .tint(Theme.destructive)
        .buttonStyle(.automatic)
        .accessibilityIdentifier("space-swipe-close-\(space.id)")
    }

    private func perform(_ action: SpaceAction, on space: Space) {
        switch action {
        case .openWorktree:
            openWorktreeTarget = model.defaultSpace(for: space)
        case .createWorktree:
            createWorktreeTarget = model.defaultSpace(for: space)
        case .rename:
            renameTarget = space
            renameText = space.label
            isShowingRenameAlert = true
        case .close:
            closeTarget = space
            isShowingCloseConfirmation = true
        }
    }

    private var emptyState: some View {
        VStack(spacing: 14) {
            Image(systemName: "square.grid.3x3")
                .font(.jost(42, weight: .light))
                .foregroundStyle(.tertiary)
            Text("No Spaces Yet")
                .font(.jost(.title3, weight: .semibold))
            Text("Start herdr on your remote machine and create a workspace,\nor tap ⋯ to create one from here.")
                .font(.jost(.footnote))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(30)
    }
}

// MARK: - Create space drawer

/// Selection captured from the create-space drawer.
struct CreateSpaceRequest {
    /// The space to base the new space on; `nil` creates a blank space in
    /// herdr's default directory.
    var base: Space?
}

/// Drawer behind the Spaces screen's options menu: pick an existing space to
/// base the new space on — it opens in the same directory — or start a
/// blank one. Tapping a row creates the space and dismisses the drawer.
private struct CreateSpaceDrawer: View {
    let spaces: [Space]
    let onCreate: (Space?) -> Void
    let onCancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("New Space")
                .font(.jost(.title3, weight: .semibold))
            Text("Choose a space to base the new space on — it opens in the same directory. A blank space starts in the default one.")
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    blankRow

                    if !spaces.isEmpty {
                        Text("Base on an Existing Space")
                            .font(.jost(.caption, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .padding(.top, 8)
                    }

                    ForEach(spaces) { space in
                        baseRow(space)
                    }
                }
                .padding(.bottom, 4)
            }

            Button("Cancel", action: onCancel)
                .buttonStyle(.herdrGhost())
        }
        .padding(.horizontal, 20)
        .padding(.top, 24)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Theme.backgroundGradient.ignoresSafeArea())
    }

    private var blankRow: some View {
        Button {
            onCreate(nil)
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "plus.square.dashed")
                    .font(.jost(13, weight: .semibold))
                    .foregroundStyle(Theme.accent)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Blank Space")
                        .font(.jost(15, weight: .semibold))
                    Text("Starts in the default directory")
                        .font(.jost(.caption))
                        .foregroundStyle(.tertiary)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .herdrField()
        }
        .buttonStyle(.herdrCard)
        .accessibilityLabel("Create Blank Space")
        .accessibilityIdentifier("space-create-blank-button")
    }

    private func baseRow(_ space: Space) -> some View {
        Button {
            onCreate(space)
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "square.grid.3x3")
                    .font(.jost(12, weight: .semibold))
                    .foregroundStyle(.tertiary)
                VStack(alignment: .leading, spacing: 2) {
                    Text("#\(space.workspace.number) · \(space.label)")
                        .font(.jost(15, weight: .semibold))
                        .lineLimit(1)
                        .minimumScaleFactor(0.6)
                    Text(directoryHint(for: space))
                        .font(.caption.monospaced())
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 6)
                StatusPill(status: space.status)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .herdrField()
        }
        .buttonStyle(.herdrCard)
        .accessibilityLabel("Create space based on \(space.label)")
        .accessibilityIdentifier("space-create-base-\(space.id)")
    }

    /// Directory the new space would open in: the checkout when the space
    /// has repo metadata, else the first agent's cwd, else pane/tab counts.
    private func directoryHint(for space: Space) -> String {
        if let worktree = space.workspace.worktree {
            return worktree.checkoutPath
        }
        if let cwd = space.agents.compactMap(\.cwd).first, !cwd.isEmpty {
            return cwd
        }
        let panes = space.workspace.paneCount
        let tabs = space.workspace.tabCount
        return "\(panes) pane\(panes == 1 ? "" : "s") · \(tabs) tab\(tabs == 1 ? "" : "s")"
    }
}

// MARK: - Open worktree drawer

/// Selection captured from the open-worktree drawer.
struct OpenWorktreeRequest {
    var space: Space
    var path: String?
    var branch: String?
}

private struct OpenWorktreeDrawer: View {
    let space: Space
    let connection: HerdrConnection
    let onOpen: (_ path: String?, _ branch: String?) -> Void
    let onCancel: () -> Void

    @State private var worktrees: [WorktreeEntry] = []
    @State private var isLoading = true
    @State private var errorMessage: String?
    @State private var customBranchOrPath = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header

            if isLoading {
                CircularArcSpinner("Loading worktrees…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let errorMessage {
                VStack(spacing: 14) {
                    ErrorBanner(message: errorMessage) {
                        Task { await loadWorktrees() }
                    }
                    customInputSection
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        if !worktrees.isEmpty {
                            Text("Existing worktrees")
                                .font(.jost(.caption, weight: .semibold))
                                .foregroundStyle(.secondary)
                                .padding(.top, 4)

                            ForEach(worktrees) { entry in
                                worktreeRow(entry)
                            }
                        } else {
                            ContentUnavailableView(
                                "No worktrees found",
                                systemImage: "arrow.triangle.branch",
                                description: Text("No additional Git worktrees were found for this repository.")
                            )
                            .padding(.vertical, 8)
                        }

                        customInputSection
                    }
                    .padding(.bottom, 4)
                }
            }

            Button("Cancel", action: onCancel)
                .buttonStyle(.herdrGhost())
                .accessibilityIdentifier("open-worktree-cancel-button")
        }
        .padding(.horizontal, 20)
        .padding(.top, 24)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Theme.backgroundGradient.ignoresSafeArea())
        .task {
            await loadWorktrees()
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Open Worktree")
                .font(.jost(.title3, weight: .semibold))
            Text("Open an existing Git worktree in #\(space.workspace.number) · \(space.label) as a Herdr workspace.")
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let worktree = space.workspace.worktree {
                RepoPill(worktree: worktree)
                    .padding(.top, 2)
            }
        }
    }

    private func worktreeRow(_ entry: WorktreeEntry) -> some View {
        Button {
            onOpen(entry.path, entry.branch)
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "arrow.triangle.branch")
                    .font(.jost(13, weight: .semibold))
                    .foregroundStyle(Theme.accent)

                VStack(alignment: .leading, spacing: 2) {
                    Text(entry.displayTitle)
                        .font(.jost(15, weight: .semibold))
                        .lineLimit(1)
                    Text(entry.path)
                        .font(.caption.monospaced())
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 6)

                if let openId = entry.openWorkspaceId {
                    Text("Open (#\(openId))")
                        .font(.jost(.caption2, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Theme.subtleFill, in: Capsule())
                } else if entry.isLinkedWorktree == false {
                    Text("Root")
                        .font(.jost(.caption2, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Theme.subtleFill, in: Capsule())
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .herdrField()
        }
        .buttonStyle(.herdrCard)
        .accessibilityLabel("Open worktree \(entry.displayTitle)")
        .accessibilityIdentifier("worktree-open-row-\(entry.displayTitle)")
    }

    private var customInputSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Or open by branch or path")
                .font(.jost(.caption, weight: .semibold))
                .foregroundStyle(.secondary)
                .padding(.top, 8)

            HStack(spacing: 8) {
                TextField("Branch or path", text: $customBranchOrPath)
                    .textFieldStyle(.plain)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .herdrField()
                    .accessibilityIdentifier("open-worktree-custom-input")

                Button("Open") {
                    let trimmed = customBranchOrPath.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !trimmed.isEmpty else { return }
                    if trimmed.hasPrefix("/") || trimmed.hasPrefix("~") {
                        onOpen(trimmed, nil)
                    } else {
                        onOpen(nil, trimmed)
                    }
                }
                .buttonStyle(.herdrPrimary(fullWidth: false))
                .controlSize(.small)
                .disabled(customBranchOrPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityIdentifier("open-worktree-custom-button")
            }
        }
    }

    private func loadWorktrees() async {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }
        do {
            let result = try await connection.worktreeList(workspaceId: space.workspace.workspaceId)
            worktrees = result.worktrees
        } catch {
            let message = HerdrConnection.friendlyMessage(for: error)
            if !HerdrConnection.isDisconnectionOrTransitionMessage(message) {
                errorMessage = message
            }
        }
    }
}

// MARK: - Create worktree drawer

/// Selection captured from the create-worktree drawer.
struct CreateWorktreeRequest {
    var space: Space
    var name: String?
    var label: String?
}

private struct CreateWorktreeDrawer: View {
    let space: Space
    let onCreate: (_ name: String?, _ label: String?) -> Void
    let onCancel: () -> Void

    @State private var name = ""
    @State private var label = ""

    private var validationError: String? {
        WorktreeNameValidator.validate(name)
    }

    private var isValid: Bool {
        validationError == nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header

            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Worktree name")
                            .font(.jost(.caption, weight: .semibold))
                            .foregroundStyle(.secondary)
                        TextField("e.g. my-feature (optional)", text: $name)
                            .textFieldStyle(.plain)
                            .autocorrectionDisabled()
                            .textInputAutocapitalization(.never)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 10)
                            .herdrField()
                            .overlay {
                                if validationError != nil {
                                    RoundedRectangle(cornerRadius: DesignSystem.CornerRadius.md)
                                        .stroke(Theme.warning, lineWidth: 1)
                                }
                            }
                            .accessibilityIdentifier("create-worktree-name-input")
                        if let error = validationError {
                            Text(error)
                                .font(.jost(.caption2))
                                .foregroundStyle(Theme.warning)
                                .accessibilityIdentifier("create-worktree-validation-error")
                        } else {
                            Text("Herdr will generate a worktree name if left blank.")
                                .font(.jost(.caption2))
                                .foregroundStyle(.tertiary)
                        }
                    }

                    VStack(alignment: .leading, spacing: 6) {
                        Text("Workspace label")
                            .font(.jost(.caption, weight: .semibold))
                            .foregroundStyle(.secondary)
                        TextField("e.g. My Feature (optional)", text: $label)
                            .textFieldStyle(.plain)
                            .autocorrectionDisabled()
                            .textInputAutocapitalization(.never)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 10)
                            .herdrField()
                            .accessibilityIdentifier("create-worktree-label-input")
                        Text("Display title for the new workspace. Defaults to the worktree name.")
                            .font(.jost(.caption2))
                            .foregroundStyle(.tertiary)
                    }
                }
                .padding(.top, 4)
                .padding(.bottom, 8)
            }

            VStack(spacing: 8) {
                Button("Create Worktree") {
                    guard isValid else { return }
                    let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
                    let trimmedLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
                    onCreate(
                        trimmedName.isEmpty ? nil : trimmedName,
                        trimmedLabel.isEmpty ? nil : trimmedLabel
                    )
                }
                .buttonStyle(.herdrPrimary())
                .disabled(!isValid)
                .accessibilityIdentifier("create-worktree-submit-button")

                Button("Cancel", action: onCancel)
                    .buttonStyle(.herdrGhost())
                    .accessibilityIdentifier("create-worktree-cancel-button")
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 24)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Theme.backgroundGradient.ignoresSafeArea())
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("New Worktree")
                .font(.jost(.title3, weight: .semibold))
            Text("Create a new Git worktree and open it in a workspace for #\(space.workspace.number) · \(space.label).")
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let worktree = space.workspace.worktree {
                RepoPill(worktree: worktree)
                    .padding(.top, 2)
            }
        }
    }
}

// MARK: - Space row

struct MachineSectionHeader: View {
    let name: String

    var body: some View {
        Label(name, systemImage: "desktopcomputer")
            .font(.pixel(10, bold: true))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 12)
            .padding(.bottom, 2)
            .accessibilityAddTraits(.isHeader)
    }
}

private struct PendingSpaceRow: View {
    let title: String

    var body: some View {
        HStack(alignment: .center, spacing: QuietListMetrics.columnSpacing) {
            CircularArcSpinner(size: QuietListMetrics.statusColumn)
            Text(title)
                .font(.appHeadlineSm)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 0)
        }
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) { QuietListDivider() }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityIdentifier("space-pending-card")
    }
}

/// A space group's root as a chat-style row: status column, title with the
/// last-activity time pinned top-right, and a line of tab/pane/agent counts.
struct SpaceRow: View {
    let group: SpaceGroup
    var machineLabel: String? = nil
    var isHighlighted = false
    var lastUpdated: Date? = nil

    private var workspace: Workspace { group.root.workspace }

    var body: some View {
        HStack(alignment: .listTitleCenter, spacing: QuietListMetrics.columnSpacing) {
            StatusDot(status: group.root.status)
                .accessibilityLabel(group.root.status.label)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(group.root.label)
                        .font(.appHeadlineSm)
                        .lineLimit(1)
                    if workspace.focused {
                        FocusedMark()
                    }
                    Spacer(minLength: 8)
                    RowTimestamp(date: lastUpdated)
                }
                .alignmentGuide(.listTitleCenter) { $0[VerticalAlignment.center] }
                HStack(spacing: 12) {
                    StatChip(icon: "rectangle.split.3x1", value: "\(workspace.tabCount)", caption: "tabs")
                    StatChip(icon: "rectangle.split.2x2", value: "\(workspace.paneCount)", caption: "panes")
                    StatChip(icon: "sparkles", value: "\(group.root.agents.count)", caption: "agents")
                    if let machineLabel {
                        Label(machineLabel, systemImage: "desktopcomputer")
                            .font(.jost(.caption))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
            }
        }
        .padding(.vertical, 11)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .background { RowHighlight(isActive: isHighlighted) }
        .overlay(alignment: .bottom) { QuietListDivider() }
        .accessibilityElement(children: .combine)
    }
}

/// A linked worktree, indented under its space's row.
struct WorktreeRow: View {
    let space: Space
    var isHighlighted = false
    var lastUpdated: Date? = nil

    var body: some View {
        HStack(alignment: .center, spacing: QuietListMetrics.columnSpacing) {
            StatusDot(status: space.status)
                .accessibilityLabel(space.status.label)
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: "arrow.triangle.branch")
                    .font(.jost(11, weight: .semibold))
                    .foregroundStyle(.secondary)
                Text(space.label)
                    .font(.jost(15, weight: .medium))
                    .lineLimit(1)
                if space.workspace.focused {
                    FocusedMark()
                }
                Spacer(minLength: 8)
                RowTimestamp(date: lastUpdated)
            }
        }
        .padding(.leading, QuietListMetrics.titleInset)
        .padding(.vertical, 9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .background { RowHighlight(isActive: isHighlighted) }
        .overlay(alignment: .bottom) { QuietListDivider(leadingInset: QuietListMetrics.titleInset * 2) }
        .accessibilityElement(children: .combine)
    }
}

private struct PendingWorktreeRow: View {
    let action: PendingWorktreeAction

    var body: some View {
        HStack(spacing: QuietListMetrics.columnSpacing) {
            CircularArcSpinner(size: 11)
                .frame(width: QuietListMetrics.statusColumn)
            Text(action.title)
                .font(.jost(14, weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 0)
        }
        .padding(.leading, QuietListMetrics.titleInset)
        .padding(.vertical, 9)
        .overlay(alignment: .bottom) { QuietListDivider(leadingInset: QuietListMetrics.titleInset * 2) }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(action.title)
        .accessibilityIdentifier("worktree-pending-row")
        .id(PendingWorktreeAction.viewID)
        .transition(.opacity.combined(with: .scale(scale: 0.95)))
    }
}

private struct FocusedMark: View {
    var body: some View {
        Image(systemName: "scope")
            .font(.jost(11, weight: .semibold))
            .foregroundStyle(Theme.accent)
            .accessibilityLabel("Focused")
    }
}

private struct RowHighlight: View {
    let isActive: Bool

    var body: some View {
        if isActive {
            RoundedRectangle.continuous(DesignSystem.CornerRadius.md)
                .fill(Theme.selectedFill)
                .padding(.horizontal, -8)
        }
    }
}

private extension View {
    /// Hides the List row chevron where SwiftUI supports it (iOS 26+).
    @ViewBuilder
    func hidesNavigationIndicator() -> some View {
        if #available(iOS 26.0, *) {
            navigationLinkIndicatorVisibility(.hidden)
        } else {
            self
        }
    }

    /// Strips List chrome so a row sits on the canvas like the Agents list.
    func quietListRow() -> some View {
        listRowInsets(EdgeInsets(top: 0, leading: 20, bottom: 0, trailing: 20))
            .listRowSeparator(.hidden)
            .listRowBackground(Color.clear)
    }
}

enum SpaceAction: Identifiable {
    case openWorktree, createWorktree, rename, close

    var id: Self { self }
}

/// The swipe's "More" drawer: every action for one space.
private struct SpaceActionsDrawer: View {
    let space: Space
    let isWorktreeActionDisabled: Bool
    let onSelect: (SpaceAction) -> Void
    let onCancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(space.label)
                .font(.jost(.title3, weight: .semibold))
                .lineLimit(1)

            VStack(spacing: 8) {
                row(.openWorktree, title: "Open Worktree", icon: "arrow.triangle.branch",
                    identifier: "space-open-worktree-button-\(space.id)")
                    .disabled(isWorktreeActionDisabled)
                row(.createWorktree, title: "Create Worktree", icon: "plus",
                    identifier: "space-create-worktree-button-\(space.id)")
                    .disabled(isWorktreeActionDisabled)
                row(.rename, title: "Rename Space", icon: "pencil",
                    identifier: "space-rename-button-\(space.id)")
                row(.close, title: "Close Space", icon: "xmark.circle",
                    identifier: "space-close-button-\(space.id)", tint: Theme.destructive)
            }

            Spacer(minLength: 0)

            Button("Cancel", action: onCancel)
                .buttonStyle(.herdrGhost())
        }
        .padding(.horizontal, 20)
        .padding(.top, 24)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Theme.backgroundGradient.ignoresSafeArea())
    }

    private func row(
        _ action: SpaceAction,
        title: String,
        icon: String,
        identifier: String,
        tint: Color = Theme.accent
    ) -> some View {
        Button {
            onSelect(action)
        } label: {
            HStack(spacing: 12) {
                Image(systemName: icon)
                    .font(.jost(14, weight: .semibold))
                    .foregroundStyle(tint)
                    .frame(width: 20)
                Text(title)
                    .font(.jost(15, weight: .semibold))
                    .foregroundStyle(action == .close ? tint : Theme.text)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .herdrField()
        }
        .buttonStyle(.herdrCard)
        .accessibilityIdentifier(identifier)
    }
}

// Resolves the workspace's initial pane without adding an overview to the back stack.
struct WorkspacePaneView: View {
    let space: Space
    var initialPaneID: String? = nil

    @Environment(AppModel.self) private var appModel
    @State private var panes: [PaneEntry] = []
    @State private var isLoading = true
    @State private var errorMessage: String?

    private var initialPane: PaneEntry? {
        if let initialPaneID {
            return panes.first { $0.id == initialPaneID }
        }
        return panes.first(where: \.focused)
            ?? panes.first { $0.tabId == space.workspace.activeTabId }
            ?? panes.first
    }

    var body: some View {
        Group {
            if let pane = initialPane {
                PaneDetailView(pane: pane, siblings: panes)
            } else {
                VStack(spacing: 16) {
                    if isLoading {
                        CircularArcSpinner("Loading panes…")
                    } else if let errorMessage,
                              !HerdrConnection.isDisconnectionOrTransitionMessage(errorMessage) {
                        ErrorBanner(message: errorMessage) {
                            Task { await loadPanes() }
                        }
                    } else {
                        ContentUnavailableView(
                            initialPaneID == nil ? "No panes available" : "Pane unavailable",
                            systemImage: "rectangle.split.2x2",
                            description: Text(initialPaneID == nil
                                ? "Open a pane in this workspace, then retry."
                                : "This agent's pane may have closed. Go back to refresh the agents list, or retry.")
                        )
                        Button("Retry") { Task { await loadPanes() } }
                    }
                }
                .padding()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .navigationTitle(space.label)
                .navigationBarTitleDisplayMode(.inline)
            }
        }
        .background(Theme.backgroundGradient.ignoresSafeArea())
        .toolbarBackground(.hidden, for: .navigationBar)
        .task(id: space.id) { await loadPanes() }
        .connectionStatusBanner(appModel: appModel)
    }

    private func loadPanes() async {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }
        do {
            let fetched = try await appModel.connection.paneList(workspaceId: space.workspace.workspaceId)
            try Task.checkCancellation()
            panes = fetched.filter { $0.workspaceId == space.workspace.workspaceId }
        } catch is CancellationError {
            return
        } catch {
            let message = HerdrConnection.friendlyMessage(for: error)
            if !HerdrConnection.isDisconnectionOrTransitionMessage(message) {
                errorMessage = message
            }
        }
    }
}
