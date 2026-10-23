import Observation
import SwiftUI

// MARK: - Main tab view

/// Keeps the chosen tab while replacing navigation and data on machine changes.
struct MainTabView: View {
    @Environment(AppModel.self) private var appModel
    @State private var selectedTab = ConnectedTabsView.MainTab.requestedInitialTab()

    var body: some View {
        ConnectedTabsView(selectedTab: $selectedTab)
            .environment(appModel.selectedMachineModel ?? appModel)
            .id(appModel.isShowingAllMachines ? "all" : appModel.selectedMachineModel?.connectionIdentity?.storageKey ?? "gateway")
            // The session view appears while still connecting; rediscover
            // once connected (and after reconnects, which reset machines).
            .task(id: appModel.phase) {
                guard case .connected = appModel.phase else { return }
                await appModel.discoverMachines()
            }
    }
}

/// Owns the selected machine's shared data and lifecycle-aware refresh loop.
struct ConnectedTabsView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var model = SpacesModel()
    @Binding var selectedTab: MainTab

    enum MainTab: String, Hashable {
        case spaces
        case agents
        case quota
        case settings

        /// Optional launch-argument driven initial tab (UI tests, screenshots):
        /// `-hc.tab agents` / `-hc.tab spaces` / `-hc.tab quota` / `-hc.tab settings`.
        static func requestedInitialTab() -> MainTab {
            let arguments = ProcessInfo.processInfo.arguments
            guard let index = arguments.firstIndex(of: "-hc.tab"),
                  index + 1 < arguments.count,
                  let tab = MainTab(rawValue: arguments[index + 1]) else {
                if AutoConnect.configIfRequested() != nil || LaunchPreference.autoConnectConfig() != nil {
                    return .agents
                }
                return .spaces
            }
            return tab
        }
    }

    var body: some View {
        TabView(selection: $selectedTab) {
            SpacesScreen(model: model)
                .tabItem {
                    Label("Spaces", systemImage: "square.grid.3x3")
                }
                .tag(MainTab.spaces)

            AgentsScreen(model: model)
                .tabItem {
                    Label("Agents", systemImage: "cpu")
                }
                .badge(model.blockedAgentCount)
                .tag(MainTab.agents)

            QuotaScreen(model: model)
                .tabItem {
                    Label("Quota", systemImage: "gauge.with.dots.needle.67percent")
                }
                .tag(MainTab.quota)

            SettingsScreen(model: model)
                .tabItem {
                    Label("Settings", systemImage: "gearshape")
                }
                .tag(MainTab.settings)
        }
        .environment(model)
        .task(id: appModel.machineCatalog.machines) {
            model.stopAutoRefresh()
            if appModel.isShowingAllMachines {
                await model.bindMachines(appModel)
            } else {
                model.bindConnectionScope(appModel.connectionIdentity)
            }
            guard !Task.isCancelled else { return }
            let active = scenePhase == .active
            model.isSceneActive = active
            appModel.isSceneActive = active
            for source in model.machineSources { source.app.isSceneActive = active }
            await model.refresh(appModel.connection)
            guard !Task.isCancelled else { return }
            model.startAutoRefresh(appModel.connection)
        }
        .onChange(of: appModel.phase) { oldPhase, newPhase in
            if case .connected = newPhase, case .offline = oldPhase {
                Task {
                    await model.refresh(appModel.connection)
                    await model.refreshUsage(appModel.connection, force: true)
                }
            }
        }
        .onChange(of: scenePhase) { _, phase in
            let active = phase == .active
            model.isSceneActive = active
            appModel.isSceneActive = active
            for source in model.machineSources { source.app.isSceneActive = active }
            if active {
                if appModel.isOffline {
                    Task { await appModel.reconnect(automatically: true) }
                } else if model.autoRefreshEnabled {
                    Task {
                        await model.refresh(appModel.connection)
                        await model.refreshUsage(appModel.connection, force: true)
                    }
                }
            }
        }
        .onChange(of: model.autoRefreshEnabled) { _, enabled in
            for source in model.machineSources { source.model.autoRefreshEnabled = enabled }
        }
        .onDisappear {
            model.stopAutoRefresh()
        }
    }
}

// MARK: - Agents screen

/// Every agent across every space. Ordered by herdr's attention priority
/// (flat queue) or grouped by space — both selectable from the ⋯ menu.
/// Tapping a row opens that agent's pane directly.
struct AgentsScreen: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let model: SpacesModel
    @State private var raisedCardID: String?

    @AppStorage(AgentsSortPreference.storageKey)
    private var sortRaw = AgentsSortPreference.default.rawValue

    private var sort: AgentsSortPreference {
        AgentsSortPreference(rawValue: sortRaw) ?? .default
    }

    private var sortSelection: Binding<AgentsSortPreference> {
        Binding(
            get: { AgentsSortPreference(rawValue: sortRaw) ?? .default },
            set: { sortRaw = $0.rawValue }
        )
    }

    /// The flat attention queue — herdr's `priority` agent panel sort:
    /// blocked, done, working, idle, unknown, latest state change first.
    private var items: [AgentListItem] {
        model.spaces
            .flatMap { space in space.agents.map { AgentListItem(space: space, agent: $0) } }
            .sorted(by: AgentListItem.attentionOrder)
    }

    /// Spaces with their agents — herdr's `spaces` agent panel sort.
    private var sections: [(space: Space, items: [AgentListItem])] {
        AgentListItem.grouped(in: model.spaces)
    }

    private var cardOrder: [ReorderingCard] {
        let displayedItems = sort == .priority ? items : sections.flatMap { $0.items }
        return displayedItems.map { item in
            ReorderingCard(
                id: item.id,
                priorityKey: "\(item.agent.agentStatus)|\(item.agent.stateChangeSeq.map { String($0) } ?? "")"
            )
        }
    }

    var body: some View {
        NavigationStack {
            Group {
                if appModel.isOffline && items.isEmpty {
                    OfflineHerdView()
                } else if items.isEmpty && model.showsAgentLoading {
                    loadingSkeleton
                } else if items.isEmpty && !model.hasLoadedAgentList && model.agentListError != nil {
                    ContentUnavailableView("Unable to Load Agents", systemImage: "exclamationmark.triangle",
                                           description: Text("Retry to load agents from your machines."))
                } else if items.isEmpty {
                    emptyState
                } else {
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            switch sort {
                            case .priority:
                                let currentItems = items
                                ForEach(currentItems) { item in
                                    agentRow(item)
                                }
                            case .spaces:
                                let currentSections = sections
                                ForEach(Array(currentSections.enumerated()), id: \.element.space.id) { index, section in
                                    if model.showsAllMachines,
                                       index == 0 || currentSections[index - 1].space.machine?.id != section.space.machine?.id {
                                        MachineSectionHeader(name: section.space.machine?.label ?? "Machine")
                                    }
                                    spaceHeader(section.space)
                                    ForEach(section.items) { item in
                                        agentRow(item)
                                    }
                                }
                            }
                        }
                        .animation(reduceMotion ? nil : .spring(duration: 0.5, bounce: 0.12), value: cardOrder.map(\.id))
                        .onChange(of: cardOrder) { old, new in
                            raisedCardID = ReorderingCard.movingID(from: old, to: new)
                        }
                        .padding(.horizontal, 20)
                        .padding(.top, 4)
                        .padding(.bottom, 28)
                    }
                }
            }
            .background(Theme.listBackground.ignoresSafeArea())
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    HerdrcatBrandMark(showsHost: true, title: "Agents")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Picker("Sort", selection: sortSelection) {
                            ForEach(AgentsSortPreference.allCases) { preference in
                                Label(preference.title, systemImage: preference.systemImage)
                                    .tag(preference)
                            }
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                            .font(.jost(16, weight: .medium))
                            .foregroundStyle(.primary)
                    }
                    .accessibilityLabel("Agent List Options")
                    .accessibilityIdentifier("agent-list-options-menu")
                }
            }
            .toolbarBackground(.hidden, for: .navigationBar)
            .navigationDestination(for: AgentListItem.self) { item in
                if let target = model.app(for: item.space, fallback: appModel) {
                    WorkspacePaneView(space: item.space, initialPaneID: item.agent.paneId)
                        .environment(target)
                        .environment(model.model(for: item.space))
                } else {
                    ContentUnavailableView("Machine Unavailable", systemImage: "desktopcomputer.trianglebadge.exclamationmark")
                }
            }
            .overlay(alignment: .bottom) {
                if let error = model.agentListError,
                   case .connected = appModel.phase {
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
                await model.refresh(appModel.connection)
            }
            .connectionStatusBanner(appModel: appModel)
        }
    }

    // MARK: Pieces

    private var loadingSkeleton: some View {
        ScrollView {
            VStack(spacing: 0) {
                ForEach(0..<6) { index in
                    HStack(alignment: .top, spacing: QuietListMetrics.columnSpacing) {
                        Circle()
                            .fill(Theme.subtleFill)
                            .frame(width: 9, height: 9)
                            .frame(width: QuietListMetrics.statusColumn, height: 14)
                            .padding(.top, 3)
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                skeletonBar(width: index.isMultiple(of: 2) ? 170 : 140, height: 14)
                                Spacer(minLength: 4)
                                skeletonBar(width: 48, height: 10)
                            }
                            skeletonBar(width: 120, height: 10)
                        }
                    }
                    .padding(.vertical, 13)
                    .overlay(alignment: .bottom) { QuietListDivider() }
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 4)
            .padding(.bottom, 28)
        }
        .allowsHitTesting(false)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Loading agents")
        .accessibilityIdentifier("agent-list-loading")
    }

    private func skeletonBar(width: CGFloat, height: CGFloat) -> some View {
        RoundedRectangle.continuous(DesignSystem.CornerRadius.sm)
            .fill(Theme.subtleFill)
            .frame(width: width, height: height)
    }

    private func agentRow(_ item: AgentListItem) -> some View {
        NavigationLink(value: item) {
            AgentListRow(
                item: item,
                lastUpdated: model.model(for: item.space).lastUpdatedByPaneID[item.agent.paneId],
                machineLabel: model.showsAllMachines && sort == .priority ? item.space.machine?.label : nil
            )
        }
        .buttonStyle(.plain)
        .zIndex(raisedCardID == item.id ? 1 : 0)
        .transition(.opacity)
    }

    private func spaceHeader(_ space: Space) -> some View {
        HStack(spacing: 8) {
            Text("#\(space.workspace.number) · \(space.label)")
                .font(.pixel(10, bold: true))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            StatusPill(status: space.status)
            Spacer(minLength: 0)
        }
        .padding(.top, 18)
        .padding(.bottom, 2)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Space \(space.label), \(space.status.label)")
    }

    private var emptyState: some View {
        VStack(spacing: 14) {
            Image(systemName: "cpu")
                .font(.jost(42, weight: .light))
                .foregroundStyle(.tertiary)
            Text("No agents running")
                .font(.jost(.title3, weight: .semibold))
            Text("Launch an agent inside a herdr space —\nit will show up here on the next refresh.")
                .font(.jost(.footnote))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(30)
    }
}

// MARK: - Agent list item

struct AgentListItem: Identifiable, Hashable {
    let space: Space
    let agent: AgentEntry

    var id: String {
        guard let machine = space.machine else { return agent.paneId }
        return machine.id + "|p=" + ConnectionIdentity.encodeComponent(agent.paneId)
    }

    var status: AgentStatus {
        AgentStatus(rawValue: agent.agentStatus) ?? .unknown
    }

    var title: String {
        agent.displayTitle
    }

    /// Herdr's attention queue: `AgentStatus.attentionPriority` descending,
    /// then the most recent state change first — the same order herdr's
    /// priority-sorted agent panel uses.
    static func attentionOrder(_ lhs: Self, _ rhs: Self) -> Bool {
        if lhs.status.attentionPriority != rhs.status.attentionPriority {
            return lhs.status.attentionPriority > rhs.status.attentionPriority
        }
        // Sequence numbers belong to one server; never compare them across machines.
        if lhs.space.machine?.id != rhs.space.machine?.id {
            return (lhs.space.machine?.order ?? 0, lhs.space.machine?.id ?? "")
                < (rhs.space.machine?.order ?? 0, rhs.space.machine?.id ?? "")
        }
        // Older servers may omit the sequence; keep those agents after known changes.
        switch (lhs.agent.stateChangeSeq, rhs.agent.stateChangeSeq) {
        case let (left?, right?) where left != right:
            return left > right
        case (_?, nil): return true
        case (nil, _?): return false
        default: break
        }
        return (lhs.space.workspace.number, lhs.title, lhs.id)
            < (rhs.space.workspace.number, rhs.title, rhs.id)
    }

    /// One section per space that actually hosts agents — spaces in workspace
    /// order, agents in the space's join order — mirroring herdr's `spaces`
    /// agent panel sort. Agent-less spaces are hidden instead of rendering an
    /// empty header.
    static func grouped(in spaces: [Space]) -> [(space: Space, items: [AgentListItem])] {
        spaces.compactMap { space in
            guard !space.agents.isEmpty else { return nil }
            return (space, space.agents.map { AgentListItem(space: space, agent: $0) })
        }
    }
}

// MARK: - Agent list row

struct AgentListRow: View {
    let item: AgentListItem
    var lastUpdated: Date?
    var machineLabel: String? = nil

    private var agent: AgentEntry { item.agent }

    var body: some View {
        HStack(alignment: .listTitleCenter, spacing: QuietListMetrics.columnSpacing) {
            StatusDot(status: item.status)
                .accessibilityLabel(item.status.label)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(item.title)
                        .font(.appHeadlineSm)
                        .lineLimit(1)
                    if agent.focused {
                        Image(systemName: "scope")
                            .font(.jost(11, weight: .semibold))
                            .foregroundStyle(Theme.accent)
                            .accessibilityLabel("Focused")
                    }
                    Spacer(minLength: 8)
                    RowTimestamp(date: lastUpdated)
                }
                .alignmentGuide(.listTitleCenter) { $0[VerticalAlignment.center] }
                HStack(spacing: 6) {
                    AgentKindIcon(kind: agent.agent, size: 13)
                        .accessibilityElement()
                        .accessibilityLabel(AgentKindVisual.displayName(for: agent.agent))
                    if let machineLabel {
                        Label(machineLabel, systemImage: "desktopcomputer")
                            .font(.jost(.caption))
                            .lineLimit(1)
                            .truncationMode(.middle)
                    } else if !agent.shortCwd.isEmpty {
                        Text(agent.shortCwd)
                            .font(.caption.monospaced())
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 11)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .overlay(alignment: .bottom) { QuietListDivider() }
        .accessibilityElement(children: .combine)
    }
}

// MARK: - Shared toolbar

struct HerdrcatBrandMark: View {
    @Environment(AppModel.self) private var appModel
    var showsHost = false
    var title: String? = nil

    private var host: String? {
        guard showsHost else { return nil }
        switch appModel.phase {
        case let .connected(host, _):
            return appModel.isShowingAllMachines ? "All Machines" : host
        case let .offline(host, _):
            return appModel.isShowingAllMachines ? "All Machines" : host
        default:
            return nil
        }
    }

    private var shortHost: String {
        if appModel.isShowingAllMachines { return "All Machines" }
        if let machine = appModel.selectedMachine { return machine.label }
        let fullHost = host ?? ""
        // Only abbreviate DNS names; keep numeric and IPv6 addresses intact.
        if fullHost.contains(":") || fullHost.allSatisfy({ $0.isNumber || $0 == "." }) {
            return fullHost
        }
        return String(fullHost.split(separator: ".").first ?? Substring(fullHost))
    }

    var body: some View {
        VStack(spacing: 1) {
            HStack(spacing: 8) {
                if title == nil {
                    Image("HerdrcatLogo")
                        .resizable()
                        .scaledToFit()
                        .frame(width: 28, height: 28)
                        .clipShape(RoundedRectangle.continuous(DesignSystem.CornerRadius.xs))
                }
                Text(title ?? "Herdcats")
                    .font(.jost(17, weight: .semibold))
                    .foregroundStyle(.primary)
            }
            if let host {
                Menu {
                    Text(host)
                    Divider()
                    Button {
                        appModel.machineCatalog.selectAllMachines()
                    } label: {
                        Label("All Machines", systemImage: appModel.machineCatalog.isShowingAllMachines ? "checkmark" : "desktopcomputer")
                    }
                    Divider()
                    Button {
                        Task { await appModel.machineCatalog.selectMachine(nil) }
                    } label: {
                        Label(appModel.machineCatalog.machineDisplayName,
                              systemImage: !appModel.machineCatalog.isShowingAllMachines && appModel.selectedMachine == nil ? "checkmark" : "desktopcomputer")
                    }
                    ForEach(appModel.machineCatalog.machines) { machine in
                        Button {
                            Task { await appModel.machineCatalog.selectMachine(machine) }
                        } label: {
                            Label(machine.label + (machine.enabled ? "" : " (disabled)"),
                                  systemImage: appModel.selectedMachine?.id == machine.id ? "checkmark" : "desktopcomputer")
                        }
                        .disabled(!machine.enabled)
                    }
                    Divider()
                    Button("Refresh Machines", systemImage: "arrow.clockwise") {
                        Task { await appModel.machineCatalog.discoverMachines() }
                    }
                    .disabled(appModel.machineCatalog.isDiscoveringMachines)
                    if let error = appModel.machineCatalog.machineDiscoveryError {
                        Text(error)
                    }
                } label: {
                    HStack(spacing: 4) {
                        Text(shortHost).lineLimit(1).truncationMode(.middle)
                        Image(systemName: "chevron.down")
                    }
                    .font(.jost(.caption2))
                    .foregroundStyle(Color(uiColor: .secondaryLabel))
                }
                .accessibilityLabel("Machine: \(host)")
                .accessibilityIdentifier("machine-picker")
            }
        }
        .accessibilityElement(children: .contain)
    }
}
