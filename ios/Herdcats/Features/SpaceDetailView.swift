import AVFoundation
import PhotosUI
import Foundation
import Observation
import SwiftUI
import UIKit

// MARK: - Pane detail (live output + input)

/// Live view of one terminal pane: scrollable terminal output on top, a text
/// field underneath that types directly into the pane (Return sends text +
/// Enter, exactly as if typed at the keyboard).
struct PaneDetailView: View {
    let pane: PaneEntry

    @Environment(AppModel.self) private var appModel
    @Environment(SpacesModel.self) private var spacesModel: SpacesModel?
    @Environment(\.dismiss) private var dismiss
    @State private var siblings: [PaneEntry]
    @State private var tabs: [TabEntry] = []
    @State private var selectedPaneID: String
    @State private var timelineModes: [String: Bool] = [:]
    @State private var bridgeUpgrade = TimelineBridgeUpgrade()
    @State private var lastUpdatedByPaneID: [String: Date] = [:]
    /// Pane switcher shows chips instead of cards to give the output more room.
    @State private var isSwitcherCompact = false
    @ScaledMetric(relativeTo: .subheadline) private var paneCardHeight = 100
    @ScaledMetric(relativeTo: .caption) private var paneChipHeight = 30
    @State private var showLiveDiagnostics = false
    @AppStorage(PaneComposePreference.storageKey)
    private var showComposeByDefault = PaneComposePreference.default

    @State private var isShowingRenameAlert = false
    @State private var renameText = ""
    @State private var isShowingKillConfirmation = false
    /// Pane targeted by rename/kill from the toolbar or a card context menu.
    @State private var actionTargetPane: PaneEntry?
    @State private var isShowingAddPane = false
    @State private var pendingAddPane: AddPaneRequest?
    /// Placeholder shown in the switcher while a new pane or tab is being created.
    @State private var creatingPane: PendingPanePlaceholder?
    @State private var actionErrorMessage: String?
    @State private var isPerformingAction = false
    /// Workspace owning `pane`, refreshed on appear so workspace-level actions
    /// (rename / close / delete worktree checkout) target live data.
    @State private var workspace: Workspace?
    @State private var pendingWorkspaceAction: WorkspaceAction?

    init(pane: PaneEntry, siblings: [PaneEntry] = []) {
        self.pane = pane
        let peers = siblings.filter {
            $0.workspaceId == pane.workspaceId
        }
        _siblings = State(initialValue: peers.contains(where: { $0.id == pane.id }) ? peers : [pane] + peers)
        _selectedPaneID = State(initialValue: pane.id)
    }

    private var selectedPane: PaneEntry {
        siblings.first { $0.id == selectedPaneID } ?? pane
    }

    private var paneSwitcherHeight: CGFloat {
        isSwitcherCompact ? paneChipHeight + 12 : paneCardHeight + 20
    }

    /// Identity for `PaneSessionView`: the selected pane within the current
    /// connection scope. Changing either tears the session view down and
    /// rebuilds it with fresh state and per-pane/per-scope persistence keys,
    /// so Voice/Compose drafts, attachments, dictation, and Live sessions
    /// can never cross panes or connection identities. `storageKey` is
    /// percent-encoded and UserDefaults-safe; `#` cannot appear in it.
    private var sessionIdentity: String {
        "\(connectionScope.storageKey)#\(selectedPaneID)"
    }

    private var connectionScope: ConnectionIdentity {
        appModel.connectionIdentity
            ?? ConnectionIdentity(host: "unknown", port: 0, username: "unknown")
    }

    private var timelineMode: Binding<Bool> {
        let key = sessionIdentity
        return Binding(
            get: {
                guard TimelineAgentAvailability.supports(agent: selectedPane.agent) else { return false }
                return timelineModes[key] ?? !UserDefaults.standard.bool(forKey: TimelineSetupPreference.key(scope: connectionScope))
            },
            set: { timelineModes[key] = TimelineAgentAvailability.supports(agent: selectedPane.agent) && $0 }
        )
    }

    private var selectedPaneSession: some View {
        PaneSessionView(
            pane: selectedPane,
            connectionScope: connectionScope,
            showsTimeline: timelineMode,
            bridgeUpgrade: bridgeUpgrade,
            topOutputInset: paneSwitcherHeight,
            showLiveDiagnostics: $showLiveDiagnostics,
            onMarkedSeen: {
                Task {
                    await refreshPeersAndTabs()
                    if let spacesModel {
                        await spacesModel.refresh(appModel.connection)
                    }
                }
            },
            onOutputInteraction: {
                guard !isSwitcherCompact else { return }
                withAnimation(.snappy(duration: 0.28)) {
                    isSwitcherCompact = true
                }
            }
        )
        .id(sessionIdentity)
    }

    var body: some View {
        selectedPaneSession
        .overlay(alignment: .top) {
            PaneSwitcher(
                panes: siblings,
                tabs: tabs,
                selection: $selectedPaneID,
                isCompact: $isSwitcherCompact,
                lastUpdatedByPaneID: lastUpdatedByPaneID,
                pendingPane: creatingPane,
                isAddDisabled: isPerformingAction,
                onAdd: {
                    isShowingAddPane = true
                },
                onRename: { pane in
                    beginRename(pane)
                },
                onKill: { pane in
                    beginKill(pane)
                }
            )
        }
        .background(Theme.backgroundGradient.ignoresSafeArea())
        .navigationTitle(selectedPane.displayTitle)
        .navigationBarTitleDisplayMode(.inline)
        // Hide the system back control to disable interactive swipe-back.
        // Leaving a pane requires an explicit tap.
        .navigationBarBackButtonHidden(true)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button {
                    dismiss()
                } label: {
                    Label("Back", systemImage: "chevron.backward")
                }
                .tint(Color.primary)
                .accessibilityIdentifier("pane-back-button")
            }
            ToolbarItem(placement: .principal) {
                StatusNavigationTitle(
                    title: selectedPane.displayTitle,
                    status: selectedPane.status,
                    subtitle: selectedPane.shortCwd.isEmpty ? nil : selectedPane.shortCwd
                )
            }
            if TimelineAgentAvailability.supports(agent: selectedPane.agent) {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        timelineMode.wrappedValue.toggle()
                    } label: {
                        Image(systemName: timelineMode.wrappedValue ? "terminal" : "text.bubble")
                            .font(.jost(16, weight: .medium))
                            .foregroundStyle(.primary)
                    }
                    .accessibilityLabel(timelineMode.wrappedValue ? "Terminal" : "Timeline")
                    .accessibilityIdentifier(timelineMode.wrappedValue ? "timeline-open-terminal" : "terminal-open-timeline")
                }
            }
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    if timelineMode.wrappedValue, let version = bridgeUpgrade.bridgeVersion, HostHelperProbe.isOlderThanPinned(version) {
                        Button {
                            bridgeUpgrade.requestUpgrade()
                        } label: {
                            Label("Upgrade Herdcats Bridge", systemImage: "arrow.up.circle")
                        }
                        .accessibilityIdentifier("timeline-upgrade-menu-button")
                        Divider()
                    }
                    Toggle("Show Compose by Default", isOn: $showComposeByDefault)
                        .accessibilityIdentifier("pane-show-compose-toggle")
                    Divider()

                    Button {
                        if let workspace {
                            pendingWorkspaceAction = .rename(workspace)
                        }
                    } label: {
                        Label("Rename Workspace", systemImage: "square.and.pencil")
                    }
                    .disabled(workspace == nil)
                    .accessibilityIdentifier("workspace-rename-button")

                    Button(role: .destructive) {
                        if let workspace {
                            pendingWorkspaceAction = .close(workspace)
                        }
                    } label: {
                        Label("Close Workspace", systemImage: "xmark.circle")
                    }
                    .disabled(workspace == nil)
                    .accessibilityIdentifier("workspace-close-button")

                    Button(role: .destructive) {
                        if let workspace {
                            pendingWorkspaceAction = .deleteWorktree(workspace)
                        }
                    } label: {
                        Label("Delete Worktree Checkout", systemImage: "trash")
                    }
                    .disabled(workspace?.worktree == nil)
                    .accessibilityIdentifier("workspace-delete-worktree-button")

                    #if DEBUG
                    Divider()

                    Button {
                        showLiveDiagnostics.toggle()
                        PaneLiveInputMetrics.shared.setEnabled(showLiveDiagnostics)
                    } label: {
                        Label(
                            showLiveDiagnostics ? "Hide Diagnostics" : "Show Diagnostics",
                            systemImage: "exclamationmark.triangle"
                        )
                    }
                    .accessibilityLabel(showLiveDiagnostics ? "Hide diagnostics" : "Show diagnostics")
                    .accessibilityIdentifier("pane-diagnostics-button")
                    #endif
                } label: {
                    Image(systemName: "ellipsis")
                        .font(.jost(16, weight: .medium))
                        .foregroundStyle(.primary)
                }
                .disabled(isPerformingAction)
                .accessibilityLabel("Pane Options")
                .accessibilityIdentifier("pane-options-menu")
            }
        }
        .toolbarBackground(.hidden, for: .navigationBar)
        .onChange(of: selectedPaneID) { _, _ in
            bridgeUpgrade.bridgeVersion = nil
        }
        .toolbar(.hidden, for: .tabBar)
        .onDisappear {
            #if DEBUG
            showLiveDiagnostics = false
            PaneLiveInputMetrics.shared.setEnabled(false)
            #endif
        }
        .sheet(item: $pendingWorkspaceAction) { action in
            WorkspaceActionDrawer(
                action: action,
                onConfirm: { outcome in
                    pendingWorkspaceAction = nil
                    Task { await performWorkspaceAction(action, outcome: outcome) }
                },
                onCancel: {
                    pendingWorkspaceAction = nil
                }
            )
            .presentationDetents([.height(WorkspaceActionDrawer.estimatedHeight(for: action))])
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(20)
        }
        .alert("Rename Pane", isPresented: $isShowingRenameAlert) {
            TextField("Pane Name", text: $renameText)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityIdentifier("pane-rename-text-field")
            Button("Rename") {
                let target = actionTargetPane ?? selectedPane
                let name = renameText
                actionTargetPane = nil
                Task {
                    await renamePane(target, to: name)
                }
            }
            .accessibilityIdentifier("pane-rename-confirm-button")
            Button("Cancel", role: .cancel) {
                actionTargetPane = nil
            }
        } message: {
            Text("Enter a new name for this pane.")
        }
        .confirmationDialog(
            "Kill Pane",
            isPresented: $isShowingKillConfirmation,
            titleVisibility: .visible
        ) {
            Button("Kill Pane", role: .destructive) {
                let target = actionTargetPane ?? selectedPane
                actionTargetPane = nil
                Task {
                    await killPane(target)
                }
            }
            .accessibilityIdentifier("pane-kill-confirm-button")
            Button("Cancel", role: .cancel) {
                actionTargetPane = nil
            }
        } message: {
            let title = (actionTargetPane ?? selectedPane).displayTitle
            Text("Are you sure you want to kill “\(title)”? This will close the terminal pane.")
        }
        .alert(
            "Action Failed",
            isPresented: Binding(
                get: { actionErrorMessage != nil },
                set: { if !$0 { actionErrorMessage = nil } }
            )
        ) {
            Button("OK", role: .cancel) {
                actionErrorMessage = nil
            }
        } message: {
            if let actionErrorMessage {
                Text(actionErrorMessage)
            }
        }
        .sheet(isPresented: $isShowingAddPane, onDismiss: {
            guard let request = pendingAddPane else { return }
            pendingAddPane = nil
            Task { await addPane(request) }
        }) {
            AddPaneDrawer(
                onAdd: { request in
                    pendingAddPane = request
                    isShowingAddPane = false
                },
                onCancel: {
                    pendingAddPane = nil
                    isShowingAddPane = false
                }
            )
            .presentationDetents([.height(340)])
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(20)
        }
        .task(id: pane.workspaceId) {
            await refreshWorkspace()
            await refreshPeersAndTabs()
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(15)) } catch { return }
                let auto = spacesModel?.autoRefreshEnabled ?? true
                let active = appModel.isSceneActive
                guard PaneRefreshPolicy.shouldAutoPoll(
                    autoRefreshEnabled: auto,
                    isSceneActive: active
                ) else { continue }
                await refreshPeersAndTabs()
            }
        }
    }

    private func refreshPeersAndTabs() async {
        if let latest = try? await appModel.connection.paneList(workspaceId: pane.workspaceId),
           !Task.isCancelled {
            switch PanePeerRefresh.outcome(peers: latest, selectedPaneID: selectedPaneID) {
            case .dismiss:
                dismiss()
            case let .apply(peers, nextSelected):
                siblings = peers
                lastUpdatedByPaneID = PaneLastUpdatedStore.observe(peers, scope: connectionScope)
                selectedPaneID = nextSelected
            }
        }
        if let latestTabs = try? await appModel.connection.tabList(workspaceId: pane.workspaceId),
           !Task.isCancelled {
            tabs = PanePeerRefresh.sortedTabs(latestTabs)
        }
    }

    private func beginRename(_ pane: PaneEntry) {
        actionTargetPane = pane
        let initial = (pane.label != nil && pane.label != "command")
            ? pane.label!
            : pane.displayTitle
        renameText = initial
        isShowingRenameAlert = true
    }

    private func beginKill(_ pane: PaneEntry) {
        actionTargetPane = pane
        isShowingKillConfirmation = true
    }

    /// Loads the workspace owning this pane so workspace-level actions act on
    /// live label / worktree data.
    private func refreshWorkspace() async {
        guard
            let latest = try? await appModel.connection.workspaceList(),
            let match = latest.first(where: { $0.workspaceId == pane.workspaceId })
        else { return }
        workspace = match
    }

    /// Executes a drawer-confirmed workspace action. Close and worktree
    /// removal tear down every pane in the workspace, so the detail view
    /// dismisses itself on success.
    private func performWorkspaceAction(_ action: WorkspaceAction, outcome: WorkspaceActionOutcome) async {
        isPerformingAction = true
        defer { isPerformingAction = false }
        do {
            switch (action, outcome) {
            case let (.rename(target), .rename(name)):
                let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !trimmed.isEmpty else { return }
                workspace = try await appModel.connection.workspaceRename(
                    workspaceId: target.workspaceId,
                    label: trimmed
                )
            case let (.close(target), .close):
                try await appModel.connection.workspaceClose(workspaceId: target.workspaceId)
                dismiss()
            case let (.deleteWorktree(target), .deleteWorktree(force, trustRepository)):
                try await appModel.connection.worktreeRemove(
                    workspaceId: target.workspaceId,
                    force: force,
                    trustRepository: trustRepository
                )
                dismiss()
            default:
                return
            }
        } catch {
            actionErrorMessage = HerdrConnection.friendlyMessage(for: error)
        }
    }

    private func renamePane(_ paneToRename: PaneEntry, to newName: String) async {
        isPerformingAction = true
        defer { isPerformingAction = false }
        do {
            let updatedPane = try await appModel.connection.paneRename(paneId: paneToRename.paneId, name: newName)
            if let idx = siblings.firstIndex(where: { $0.id == updatedPane.id }) {
                siblings[idx] = updatedPane
            }
            if let latest = try? await appModel.connection.paneList(workspaceId: paneToRename.workspaceId) {
                siblings = latest
                lastUpdatedByPaneID = PaneLastUpdatedStore.observe(latest, scope: connectionScope)
            }
        } catch {
            actionErrorMessage = HerdrConnection.friendlyMessage(for: error)
        }
    }

    private func killPane(_ paneToKill: PaneEntry) async {
        isPerformingAction = true
        defer { isPerformingAction = false }
        do {
            try await appModel.connection.paneClose(paneId: paneToKill.paneId)
            let remaining = siblings.filter { $0.id != paneToKill.id }
            siblings = remaining
            if remaining.isEmpty {
                dismiss()
            } else {
                if selectedPaneID == paneToKill.id {
                    selectedPaneID = remaining[0].id
                }
            }
            if let latest = try? await appModel.connection.paneList(workspaceId: paneToKill.workspaceId) {
                siblings = latest
                lastUpdatedByPaneID = PaneLastUpdatedStore.observe(latest, scope: connectionScope)
                if latest.isEmpty {
                    dismiss()
                } else if !latest.contains(where: { $0.id == selectedPaneID }) {
                    selectedPaneID = latest[0].id
                }
            }
        } catch {
            actionErrorMessage = HerdrConnection.friendlyMessage(for: error)
        }
    }

    private func addPane(_ request: AddPaneRequest) async {
        isPerformingAction = true
        let source = selectedPane
        withAnimation(.snappy(duration: 0.28)) {
            creatingPane = PendingPanePlaceholder(
                title: PendingPanePlaceholder.title(name: request.name, kind: request.kind),
                afterPaneID: request.kind == .pane ? source.id : nil,
                kind: request.kind
            )
        }
        defer {
            isPerformingAction = false
            withAnimation(.snappy(duration: 0.28)) {
                creatingPane = nil
            }
        }
        do {
            let createdPaneID: String
            var createdPane: PaneEntry?
            switch request.kind {
            case .pane:
                let created = try await appModel.connection.paneSplit(
                    paneId: source.paneId,
                    direction: "right",
                    cwd: source.cwd,
                    focus: false
                )
                createdPaneID = created.paneId
                createdPane = created
                let trimmedName = request.name.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmedName.isEmpty {
                    createdPane = try await appModel.connection.paneRename(paneId: createdPaneID, name: trimmedName)
                }
            case .tab:
                createdPaneID = try await appModel.connection.tabCreate(
                    workspaceId: source.workspaceId,
                    cwd: source.cwd,
                    label: request.name
                )
            }
            if let latest = try? await appModel.connection.paneList(workspaceId: source.workspaceId) {
                siblings = latest
                lastUpdatedByPaneID = PaneLastUpdatedStore.observe(latest, scope: connectionScope)
                if latest.contains(where: { $0.id == createdPaneID }) {
                    selectedPaneID = createdPaneID
                }
            } else if let createdPane {
                siblings.append(createdPane)
                lastUpdatedByPaneID = PaneLastUpdatedStore.observe(siblings, scope: connectionScope)
                selectedPaneID = createdPaneID
            }
            if let latestTabs = try? await appModel.connection.tabList(workspaceId: source.workspaceId) {
                tabs = PanePeerRefresh.sortedTabs(latestTabs)
            }
        } catch {
            actionErrorMessage = HerdrConnection.friendlyMessage(for: error)
            if let latest = try? await appModel.connection.paneList(workspaceId: source.workspaceId) {
                siblings = latest
                lastUpdatedByPaneID = PaneLastUpdatedStore.observe(latest, scope: connectionScope)
            }
        }
    }
}

/// A pane or tab still being created, shown in the switcher until Herdr returns it.
struct PendingPanePlaceholder: Equatable {
    static let viewID = "pane-pending"

    let title: String
    /// Pane the new one is split from; nil places a new tab at the end.
    let afterPaneID: String?
    let kind: AddPaneKind

    static func title(name: String, kind: AddPaneKind = .pane) -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return trimmed }
        return kind == .pane ? "Opening pane…" : "Opening tab…"
    }
}

/// Snap rules for the pane switcher's vertical drag between cards and chips.
enum PaneSwitcherCollapse {
    /// Drag distance past which releasing the strip toggles its state.
    static let snapDistance: CGFloat = 24

    /// Whether the switcher is compact after a vertical drag on it ends.
    /// Dragging down expands; dragging up collapses.
    static func isCompact(afterDragFrom startCompact: Bool, translation: CGFloat) -> Bool {
        startCompact ? translation <= snapDistance : translation < -snapDistance
    }

    /// Strip height while it follows the finger, clamped between both states.
    static func height(
        isCompact: Bool,
        translation: CGFloat?,
        compactHeight: CGFloat,
        expandedHeight: CGFloat
    ) -> CGFloat {
        let base = isCompact ? compactHeight : expandedHeight
        guard let translation else { return base }
        return min(expandedHeight, max(compactHeight, base + translation))
    }
}

private struct PaneBottomDockHeightPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

/// Each selectable pane gets its own solid card in the expanded and compact rows:
/// `--surface` with a `--border` outline, or `--raised` with an accent outline
/// when selected, like the website's active rows. Moonlit uses no glass or blur.
private struct PaneSwitcherGlassItem<S: InsettableShape>: ViewModifier {
    let shape: S
    var isSelected = false

    func body(content: Content) -> some View {
        content
            .background(isSelected ? Theme.selectedFill : Theme.cardBackground, in: shape)
            .overlay(
                shape.strokeBorder(isSelected ? Theme.accent : Theme.hairline, lineWidth: isSelected ? 1.5 : 1)
                    .allowsHitTesting(false)
            )
    }
}

/// The same pane identity and status shown in space detail, presented as
/// separate translucent cards. Drag up to collapse to a row of glass chips.
private struct PaneSwitcher: View {
    let panes: [PaneEntry]
    let tabs: [TabEntry]
    @Binding var selection: String
    @Binding var isCompact: Bool
    var lastUpdatedByPaneID: [String: Date] = [:]
    var pendingPane: PendingPanePlaceholder?
    var isAddDisabled = false
    var onAdd: () -> Void
    var onRename: (PaneEntry) -> Void = { _ in }
    var onKill: (PaneEntry) -> Void = { _ in }
    @ScaledMetric(relativeTo: .subheadline) private var cardHeight = 100
    @ScaledMetric(relativeTo: .caption) private var chipHeight = 30
    /// Vertical translation while the strip follows a drag; nil otherwise.
    @State private var dragTranslation: CGFloat?
    /// Set once a drag commits to horizontal so it is left to the scroll view.
    @State private var isHorizontalDrag = false

    private var expandedHeight: CGFloat { cardHeight + 20 }
    private var compactHeight: CGFloat { chipHeight + 12 }

    private var orderedPanes: [PaneEntry] {
        let knownTabIDs = Set(tabs.map(\.id))
        return tabs.flatMap { tab in panes.filter { $0.tabId == tab.id } }
            + panes.filter { !knownTabIDs.contains($0.tabId) }
    }

    private func paneAccessibilityLabel(for pane: PaneEntry) -> String {
        var parts = [pane.displayTitle]
        if let agent = pane.agent {
            parts.append(AgentKindVisual.displayName(for: agent))
        } else {
            parts.append("Command line")
        }
        if let updated = lastUpdatedByPaneID[pane.paneId] {
            parts.append(PaneUpdatedFormat.label(for: updated))
        }
        parts.append(pane.status.label)
        if pane.focused {
            parts.append("Focused in Herdr")
        }
        return parts.joined(separator: ", ")
    }

    var body: some View {
        let height = PaneSwitcherCollapse.height(
            isCompact: isCompact,
            translation: dragTranslation,
            compactHeight: compactHeight,
            expandedHeight: expandedHeight
        )
        let showsCards = height > (compactHeight + expandedHeight) / 2

        ZStack(alignment: .top) {
            cardsRow
                .opacity(showsCards ? 1 : 0)
                .allowsHitTesting(showsCards)
                .accessibilityHidden(!showsCards)
            chipsRow
                .opacity(showsCards ? 0 : 1)
                .allowsHitTesting(!showsCards)
                .accessibilityHidden(showsCards)
        }
        .frame(height: height, alignment: .top)
        .clipped()
        .contentShape(Rectangle())
        .simultaneousGesture(collapseDrag)
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: isCompact ? "Show Pane Cards" : "Show Pane Chips") {
            withAnimation(.snappy(duration: 0.28)) {
                isCompact.toggle()
            }
        }
    }

    private var collapseDrag: some Gesture {
        DragGesture(minimumDistance: 10)
            .onChanged { value in
                guard !isHorizontalDrag else { return }
                if dragTranslation == nil {
                    guard abs(value.translation.height) > abs(value.translation.width) else {
                        isHorizontalDrag = true
                        return
                    }
                }
                dragTranslation = value.translation.height
            }
            .onEnded { value in
                defer { isHorizontalDrag = false }
                guard dragTranslation != nil else { return }
                let next = PaneSwitcherCollapse.isCompact(
                    afterDragFrom: isCompact,
                    translation: value.translation.height
                )
                if next != isCompact {
                    HapticFeedback.impact(.light)
                }
                withAnimation(.snappy(duration: 0.28)) {
                    isCompact = next
                    dragTranslation = nil
                }
            }
    }

    private var cardsRow: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                Group {
                    HStack(spacing: 8) {
                        let ordered = orderedPanes
                        ForEach(Array(ordered.enumerated()), id: \.element.id) { index, pane in
                            if index > 0, ordered[index - 1].tabId != pane.tabId {
                                tabDivider(height: cardHeight * 0.62)
                            }

                            Button {
                                selection = pane.id
                            } label: {
                                VStack(alignment: .leading, spacing: 8) {
                                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                                        StatusDot(status: pane.status)
                                            .alignmentGuide(.firstTextBaseline) { dimensions in
                                                dimensions.height * 0.5 + 4.5
                                            }
                                        Text(pane.displayTitle)
                                            .font(.jost(.subheadline, weight: .medium))
                                            .lineLimit(2, reservesSpace: true)
                                            .frame(maxWidth: .infinity, alignment: .leading)
                                    }
                                    HStack(spacing: 6) {
                                        if let agent = pane.agent {
                                            AgentBadge(kind: agent)
                                                .opacity(0.7)
                                        } else {
                                            CommandLineBadge()
                                                .opacity(0.7)
                                        }
                                        Spacer(minLength: 0)
                                        if let updated = lastUpdatedByPaneID[pane.paneId] {
                                            Text(PaneUpdatedFormat.label(for: updated))
                                                .font(.jost(.caption2))
                                                .foregroundStyle(.secondary)
                                                .lineLimit(1)
                                        }
                                    }
                                }
                                .padding(12)
                                .frame(width: 260, height: cardHeight, alignment: .leading)
                                .contentShape(RoundedRectangle.continuous(DesignSystem.CornerRadius.lg))
                                .modifier(PaneSwitcherGlassItem(
                                    shape: RoundedRectangle.continuous(DesignSystem.CornerRadius.lg),
                                    isSelected: selection == pane.id
                                ))
                                .herdrFocusLine(pane.focused, in: RoundedRectangle.continuous(DesignSystem.CornerRadius.lg))
                            }
                            .buttonStyle(.herdrCard)
                            .contextMenu { paneMenu(for: pane) }
                            .accessibilityLabel(paneAccessibilityLabel(for: pane))
                            .accessibilityAddTraits(selection == pane.id ? .isSelected : [])
                            .id(pane.id)

                            if pendingPane?.afterPaneID == pane.id {
                                pendingCard
                            }
                        }
                        if let pendingPane, !ordered.contains(where: { $0.id == pendingPane.afterPaneID }) {
                            if pendingPane.kind == .tab, !ordered.isEmpty {
                                tabDivider(height: cardHeight * 0.62)
                            }
                            pendingCard
                        }

                        Button(action: onAdd) {
                            Image(systemName: "plus")
                                .font(.jost(.title2, weight: .semibold))
                                .foregroundStyle(Theme.accent)
                                .frame(width: 56, height: cardHeight)
                                .contentShape(RoundedRectangle.continuous(DesignSystem.CornerRadius.lg))
                                .modifier(PaneSwitcherGlassItem(
                                    shape: RoundedRectangle.continuous(DesignSystem.CornerRadius.lg)
                                ))
                        }
                        .buttonStyle(.herdrCard)
                        .disabled(isAddDisabled)
                        .opacity(isAddDisabled ? 0.45 : 1)
                        .accessibilityLabel("Add Pane or Tab")
                        .accessibilityIdentifier("pane-add-button")
                        .id("pane-add")
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                }
            }
            .onAppear { proxy.scrollTo(selection, anchor: .center) }
            .onChange(of: selection) { _, id in
                withAnimation { proxy.scrollTo(id, anchor: .center) }
            }
            .onChange(of: isCompact) { _, compact in
                if !compact { proxy.scrollTo(selection, anchor: .center) }
            }
            .onChange(of: pendingPane) { _, pending in
                if pending != nil {
                    withAnimation { proxy.scrollTo(PendingPanePlaceholder.viewID, anchor: .center) }
                }
            }
        }
    }

    private var chipsRow: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                Group {
                    HStack(spacing: 6) {
                        let ordered = orderedPanes
                        ForEach(Array(ordered.enumerated()), id: \.element.id) { index, pane in
                            if index > 0, ordered[index - 1].tabId != pane.tabId {
                                tabDivider(height: 20)
                            }
                            chip(for: pane)
                            if pendingPane?.afterPaneID == pane.id {
                                pendingChip
                            }
                        }
                        if let pendingPane, !ordered.contains(where: { $0.id == pendingPane.afterPaneID }) {
                            if pendingPane.kind == .tab, !ordered.isEmpty {
                                tabDivider(height: 20)
                            }
                            pendingChip
                        }

                        Button(action: onAdd) {
                            Image(systemName: "plus")
                                .font(.jost(.caption, weight: .semibold))
                                .foregroundStyle(Theme.accent)
                                .frame(width: chipHeight, height: chipHeight)
                                .contentShape(Capsule())
                                .modifier(PaneSwitcherGlassItem(shape: Capsule()))
                        }
                        .buttonStyle(.herdrCard)
                        .disabled(isAddDisabled)
                        .opacity(isAddDisabled ? 0.45 : 1)
                        .accessibilityLabel("Add Pane or Tab")
                        .accessibilityIdentifier("pane-add-button")
                        .id("pane-add")
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                }
            }
            .onAppear { proxy.scrollTo(selection, anchor: .center) }
            .onChange(of: selection) { _, id in
                withAnimation { proxy.scrollTo(id, anchor: .center) }
            }
            .onChange(of: isCompact) { _, compact in
                if compact { proxy.scrollTo(selection, anchor: .center) }
            }
            .onChange(of: pendingPane) { _, pending in
                if pending != nil {
                    withAnimation { proxy.scrollTo(PendingPanePlaceholder.viewID, anchor: .center) }
                }
            }
        }
    }

    private func chip(for pane: PaneEntry) -> some View {
        let isSelected = selection == pane.id
        return Button {
            selection = pane.id
        } label: {
            HStack(spacing: 6) {
                StatusDot(status: pane.status)
                Text(pane.displayTitle)
                    .font(.jost(.caption, weight: .medium))
                    .lineLimit(1)
            }
            .padding(.horizontal, 10)
            .frame(maxWidth: 160)
            .frame(height: chipHeight)
            .contentShape(Capsule())
            .modifier(PaneSwitcherGlassItem(shape: Capsule(), isSelected: isSelected))
            .herdrFocusLine(
                pane.focused,
                in: Capsule(),
                thickness: 2
            )
        }
        .buttonStyle(.herdrCard)
        .contextMenu { paneMenu(for: pane) }
        .accessibilityLabel(paneAccessibilityLabel(for: pane))
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .id(pane.id)
    }

    /// Non-interactive stand-in for a pane that Herdr is still creating.
    private var pendingCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                CircularArcSpinner(size: 14)
                    .alignmentGuide(.firstTextBaseline) { dimensions in
                        dimensions.height * 0.5 + 4.5
                    }
                Text(pendingPane?.title ?? "")
                    .font(.jost(.subheadline, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(2, reservesSpace: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .frame(width: 260, height: cardHeight, alignment: .leading)
        .modifier(PaneSwitcherGlassItem(
            shape: RoundedRectangle.continuous(DesignSystem.CornerRadius.lg)
        ))
        .transition(.opacity.combined(with: .scale(scale: 0.95)))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(pendingAccessibilityLabel)
        .accessibilityIdentifier("pane-pending-card")
        .id(PendingPanePlaceholder.viewID)
    }

    private var pendingChip: some View {
        HStack(spacing: 6) {
            CircularArcSpinner(size: 11)
            Text(pendingPane?.title ?? "")
                .font(.jost(.caption, weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .padding(.horizontal, 10)
        .frame(maxWidth: 160)
        .frame(height: chipHeight)
        .modifier(PaneSwitcherGlassItem(shape: Capsule()))
        .transition(.opacity.combined(with: .scale(scale: 0.9)))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(pendingAccessibilityLabel)
        .accessibilityIdentifier("pane-pending-chip")
        .id(PendingPanePlaceholder.viewID)
    }

    private var pendingAccessibilityLabel: String {
        let kind = pendingPane?.kind == .tab ? "tab" : "pane"
        return "\(pendingPane?.title ?? "New \(kind)"), creating \(kind)"
    }

    private func tabDivider(height: CGFloat) -> some View {
        Rectangle()
            .fill(Color.primary.opacity(0.45))
            .frame(width: 1, height: height)
            .accessibilityHidden(true)
    }

    @ViewBuilder
    private func paneMenu(for pane: PaneEntry) -> some View {
        Button {
            onRename(pane)
        } label: {
            Label("Rename", systemImage: "pencil")
        }
        .disabled(isAddDisabled)
        .accessibilityIdentifier("pane-card-rename-\(pane.id)")

        Button(role: .destructive) {
            onKill(pane)
        } label: {
            Label("Kill", systemImage: "xmark.circle")
        }
        .disabled(isAddDisabled)
        .accessibilityIdentifier("pane-card-kill-\(pane.id)")
    }
}

struct PaneSessionView: View {
    let pane: PaneEntry
    let connectionScope: ConnectionIdentity
    var topOutputInset: CGFloat = 0
    var showLiveDiagnostics: Binding<Bool>? = nil
    /// Called after Herdr successfully marks a Done agent as seen.
    var onMarkedSeen: (() -> Void)?
    /// Called when the user scrolls the output or focuses the composer.
    var onOutputInteraction: (() -> Void)?

    @Environment(AppModel.self) private var appModel
    @Environment(SpacesModel.self) private var spacesModel: SpacesModel?
    @Environment(\.colorScheme) private var colorScheme
    @State private var session: PaneSessionState
    /// Pane text is held in view state and synchronously persisted through the
    /// privacy policy so a cleared session cannot restore stale values.
    @State private var draft: String
    @State private var persistence = PaneContentPersistence.shared
    @State private var persistenceRevision: UInt64
    private let draftKey: String
    private let historyKey: String
    @Binding private var showsTimeline: Bool
    let bridgeUpgrade: TimelineBridgeUpgrade
    @State private var timeline = AgentTimelineState()
    @State private var isTimelineSending = false
    @State private var shouldPinToBottom = true
    @State private var bottomDockHeight: CGFloat = 58
    @State private var draftSaveTask: Task<Void, Never>?
    /// True while the Compose bubble's text field holds keyboard focus.
    @FocusState private var composeFieldFocused: Bool
    @Environment(\.scenePhase) private var scenePhase
    @State private var dictation = PaneDictation()
    @State private var dictationError: String?
    @State private var photoItem: PhotosPickerItem?
    @State private var isPhotoPickerPresented = false
    @State private var isLoadingPhoto = false
    @State private var attachment: PendingPaneAttachment?
    @State private var attachmentUploadTask: Task<Void, Never>?
    @State private var sessionError: String?
    @State private var isMarkingSeen = false
    /// Optimistic hide of the mark-as-read control until the next peer refresh.
    @State private var locallyClearedDone = false
    /// Pane ID for which implicit Done→Idle already ran this visit.
    @State private var autoMarkedPaneID: String?

    /// Which input surface is shown: Live toolbar (default), Voice
    /// recording bar, or Compose bubble. Owned here; `PaneVoiceComposeBar`
    /// renders the visuals for the current surface.
    @State private var surfaceModel = PaneInputSurfaceModel()
    /// Cursor-owned modifier/chord model: Ctrl arms for the next key and
    /// emits `ctrl+<token>` through the ordered Live queue; ⌘ fails closed.
    @State private var liveModifierComposer = PaneLiveModifierComposer()
    /// Visual armed-modifier state for `PaneVoiceComposeBar`, driven strictly
    /// by `liveModifierComposer` outcomes so the bar never synthesizes keys.
    @State private var armedLiveModifiers: Set<PaneLiveModifier> = []
    /// Live delivery queue and its generation-scoped session token, created
    /// lazily when Live input first becomes active for this pane.
    @State private var liveQueue: PaneLiveInputQueue?
    @State private var liveSession: PaneLiveInputSession?
    /// Monotonic seconds (`systemUptime`) of the last Live acknowledgement
    /// and last rapid read, driving `PaneLiveRefreshPolicy`.
    @State private var lastLiveAckAt: TimeInterval?
    @State private var lastLiveReadAt: TimeInterval?
    /// True while the Live keyboard capture holds focus (software keyboard
    /// up, or a hardware keyboard is actively typing).
    @State private var liveInputFocused = false
    /// Identifies the in-flight dictation setup so a checkmark tapped during
    /// async authorization/microphone setup cannot leave a revived engine
    /// recording silently after the surface has moved on.
    @State private var voiceSession: UUID?
    /// Aggregate local diagnostics. Recording no-ops unless a deliberate
    /// test run enables it from the DEBUG-only navigation button.
    private let liveMetrics = PaneLiveInputMetrics.shared

    @AppStorage(PaneTextSizePreference.storageKey)
    private var paneTextSizeRaw = PaneTextSizePreference.default.rawValue
    @AppStorage(PaneScrollBehaviorPreference.storageKey)
    private var paneScrollBehaviorRaw = PaneScrollBehaviorPreference.default.rawValue
    @AppStorage(PaneComposePreference.storageKey)
    private var showComposeByDefault = PaneComposePreference.default
    @AppStorage(DoneClearPreference.storageKey)
    private var doneClearRaw = DoneClearPreference.default.rawValue

    private var output: String { session.output }
    private var isLoading: Bool { session.isLoading }
    private var errorMessage: String? { session.errorMessage }
    private var isSending: Bool { session.isSending || isTimelineSending }

    // Observable queue projections so SwiftUI tracks Live delivery state.
    private var liveQueueState: PaneLiveInputQueueState? { liveQueue?.state }
    private var livePendingCount: Int { liveQueue?.pendingCount ?? 0 }
    private var isLiveDraining: Bool { liveQueue?.isDraining ?? false }
    /// Monotonic count of delivered Live events; each increment is one
    /// per-event acknowledgement that drives rapid-refresh pacing and metrics.
    private var liveAcknowledgedCount: UInt64 { liveQueue?.acknowledgedCount ?? 0 }

    private var outputFont: Font {
        (PaneTextSizePreference(rawValue: paneTextSizeRaw) ?? .default).outputFont
    }

    private var paneScrollBehavior: PaneScrollBehaviorPreference {
        PaneScrollBehaviorPreference(rawValue: paneScrollBehaviorRaw) ?? .default
    }

    private var doneClearPreference: DoneClearPreference {
        DoneClearPreference(rawValue: doneClearRaw) ?? .default
    }

    private var showsMarkAsReadButton: Bool {
        doneClearPreference == .explicit
            && pane.status == .done
            && !locallyClearedDone
    }

    init(
        pane: PaneEntry,
        connectionScope: ConnectionIdentity,
        showsTimeline: Binding<Bool>,
        bridgeUpgrade: TimelineBridgeUpgrade = TimelineBridgeUpgrade(),
        topOutputInset: CGFloat = 0,
        showLiveDiagnostics: Binding<Bool>? = nil,
        onMarkedSeen: (() -> Void)? = nil,
        onOutputInteraction: (() -> Void)? = nil
    ) {
        self.pane = pane
        self.connectionScope = connectionScope
        self._showsTimeline = showsTimeline
        self.bridgeUpgrade = bridgeUpgrade
        self.topOutputInset = topOutputInset
        self.showLiveDiagnostics = showLiveDiagnostics
        self.onMarkedSeen = onMarkedSeen
        self.onOutputInteraction = onOutputInteraction
        let state = PaneSessionState(pane: pane, connectionScope: connectionScope)
        _session = State(initialValue: state)
        let policy = PaneContentPersistence.shared
        let draftKey = state.draftKey
        _draft = State(initialValue: policy.loadDraft(key: draftKey))
#if DEBUG && targetEnvironment(simulator)
        if ScreenshotFixtures.enabled {
            let surface = PaneInputSurfaceModel()
            if ["compose", "timeline-helper-missing", "timeline-unavailable", "timeline-pending", "timeline-disconnected", "optional-upgrade-menu", "turn-interrupted", "timeline-codex-no-session"].contains(ScreenshotFixtures.screen) {
                surface.requestComposeDraft()
                _draft = State(initialValue: "Looks good. Add a test for reconnecting, then open a pull request.")
            } else if ScreenshotFixtures.screen == "voice" {
                surface.requestVoiceRecording()
                let recording = PaneDictation()
                recording.showScreenshotRecording()
                _dictation = State(initialValue: recording)
            }
            _surfaceModel = State(initialValue: surface)
        }
#endif
        _persistence = State(initialValue: policy)
        _persistenceRevision = State(initialValue: policy.revision)
        self.draftKey = draftKey
        self.historyKey = state.historyKey
    }

    private var terminalPalette: TerminalPalette {
        let dark = true
        let preferences = AppThemePreferences.shared
        return TerminalPalette(palette: dark ? preferences.dark : preferences.light, dark: dark)
    }

    private func updateSessionParsedOutput() {
        session.updateParsedOutput(palette: terminalPalette)
    }

    /// External send gate — the Compose bubble only *hides* Send for a blank
    /// draft; actual enablement still honors upload, dictation, and
    /// in-flight-send guards owned here. Per the accepted rule, non-whitespace
    /// text is required before either the Send button or Return can send — an
    /// attached image alone is never sent (Return would otherwise leak an
    /// image-only message past the hidden button).
    private var canSend: Bool {
        !trimmedDraft.isEmpty
            && !isSending && !isTimelineSending
            && (!showsTimeline || TimelineComposePolicy.canCompose(identity:timeline.identity,paneId:pane.paneId,status:timeline.agentStatus,connected:AgentTimelineConnectionState(phase:appModel.phase,generation:appModel.ownership.transportGeneration).isReady))
            && !dictation.isBusy
            && !(attachment?.isUploading ?? false)
    }

    private func handleSend() {
        Task { await send() }
    }

    private func handlePaneChange(_ newPane: PaneEntry) {
        session.updatePane(newPane)
        updateSessionParsedOutput()
        if newPane.status != .done {
            locallyClearedDone = false
        }
        Task { await considerImplicitMarkSeen() }
    }

    @ViewBuilder
    private var outputScrollView: some View {
        PaneOutputScrollView(
            output: output,
            outputLines: session.outputLines,
            outputRevision: session.outputRevision,
            outputFont: outputFont,
            paneScrollBehavior: paneScrollBehavior,
            // Combined keyboard-focus signal: the Compose bubble's field or
            // the hidden Live capture field — either keyboard shrinking the
            // viewport must repin the newest output above it.
            keyboardFocused: composeFieldFocused || liveInputFocused,
            isOffline: appModel.isOffline,
            isLoading: isLoading,
            reservesMarkAsReadSpace: showsMarkAsReadButton,
            topOverlayInset: topOutputInset,
            bottomOverlayInset: bottomDockHeight,
            paneId: pane.paneId,
            shouldPinToBottom: $shouldPinToBottom,
            onUserScroll: {
                if liveInputFocused {
                    // The hidden Live capture field is a UIKit `UITextField`
                    // outside the scroll view, so `.interactively` and drag
                    // gestures cannot reliably reach it. Any deliberate
                    // output scroll while the Live keyboard is up drops it:
                    // the user is shifting attention to the TUI, and Keyboard
                    // re-opens with one tap. Belt-and-braces: clear the focus
                    // binding and force UIKit first-responder resignation.
                    // Compose keeps its own dismissal paths untouched.
                    liveInputFocused = false
                    UIApplication.shared.sendAction(
                        #selector(UIResponder.resignFirstResponder),
                        to: nil,
                        from: nil,
                        for: nil
                    )
                }
                onOutputInteraction?()
            },
            onTapOutput: {
                guard surfaceModel.surface != .voiceRecording, liveInputEnabled else { return }
                // Output is the direct terminal input surface. Preserve the
                // pane-scoped Compose draft while moving keyboard focus.
                let wasComposing = surfaceModel.isComposeSurface
                surfaceModel.collapseToLiveToolbar()
                composeFieldFocused = false
                if wasComposing {
                    // Let Compose relinquish first responder after its field
                    // leaves the hierarchy, then focus the Live capture.
                    DispatchQueue.main.async {
                        guard surfaceModel.isLiveKeyboardSurface, liveInputEnabled else { return }
                        liveInputFocused = true
                    }
                } else {
                    liveInputFocused = true
                }
            },
            onDismissKeyboardDrag: dismissLiveKeyboard
        )
        .equatable()
        // Keep the mark-as-read control above the floating input dock.
        .overlay(alignment: .bottomTrailing) {
            if showsMarkAsReadButton {
                markAsReadButton
                    .padding(.trailing, 20)
                    .padding(.bottom, bottomDockHeight + 10)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.easeInOut(duration: 0.2), value: showsMarkAsReadButton)
    }

    /// A small, slow indicator at the bottom right while Live input is queued
    /// or recovering. Diagnostics appear only when opened from the DEBUG
    /// navigation button.
    @ViewBuilder
    private var liveStatusRow: some View {
        if surfaceModel.isLiveKeyboardSurface {
            VStack(spacing: 4) {
                if !appModel.isOffline && (!liveInputEnabled || livePendingCount > 0 || isLiveDraining) {
                    HStack {
                        Spacer(minLength: 0)
                        CircularArcSpinner(size: 15)
                            .accessibilityLabel("Live input working")
                            .accessibilityIdentifier("pane-live-activity-spinner")
                    }
                    .padding(.trailing, 18)
                }
                #if DEBUG
                if showLiveDiagnostics?.wrappedValue == true {
                    liveDiagnosticsFooter
                }
                #endif
            }
            .padding(.top, 6)
        }
    }

    #if DEBUG
    /// Local diagnostics are enabled by the DEBUG navigation button; nothing
    /// is persisted or uploaded, and hiding this bar clears its samples.
    private var liveDiagnosticsFooter: some View {
        let metrics = PaneLiveInputMetrics.shared
        return VStack(spacing: 4) {
            Text(PaneLiveDiagnostics.summaryLine(for: metrics.summary))
                .font(.caption2.monospaced())
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("pane-live-metrics-summary")
            if let error = sessionError ?? errorMessage {
                Text(error)
                    .font(.jost(.caption2))
                    .foregroundStyle(Theme.warning)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.horizontal, 14)
    }
    #endif

    /// The Live toolbar with Keys drawer / recording bar / Compose bubble.
    /// Visuals are `PaneVoiceComposeBar` (Agy-owned); every transition and
    /// remote effect is owned by this view through `PaneInputSurfaceModel`,
    /// `PaneLiveModifierComposer`, and the ordered Live queue.
    private var voiceComposeBar: some View {
        var bar = PaneVoiceComposeBar(
            mode: Binding(
                get: {
                    showsTimeline
                        ? (surfaceModel.isLiveKeyboardSurface ? .compose : surfaceModel.surface.barMode)
                        : surfaceModel.surface.barMode
                },
                set: { _ in }
            ),
            draft: $draft,
            isLiveKeyboardFocused: $liveInputFocused,
            isComposeFieldFocused: $composeFieldFocused,
            isLiveEnabled: liveInputEnabled,
            armedModifiers: $armedLiveModifiers,
            onLiveKey: { token in
                applyLiveModifierOutcome(liveModifierComposer.applyBaseKey(token))
            },
            onLiveModifier: { modifier in
                applyLiveModifierOutcome(
                    liveModifierComposer.toggle(
                        {
                            switch modifier {
                            case .ctrl: return .control
                            case .alt: return .alt
                            case .shift: return .shift
                            }
                        }())
                )
            },
            onLiveEvent: { event in
                applyLiveModifierOutcome(liveModifierComposer.apply(event))
            },
            dictation: dictation,
            isVoiceDraining: surfaceModel.isDrainingDictation,
            dictationError: $dictationError,
            onStartVoice: startVoice,
            onFinishVoice: finishVoice,
            onOpenCompose: openCompose,
            photoItem: $photoItem,
            isPhotoPickerPresented: $isPhotoPickerPresented,
            attachment: attachment.map(PaneAttachmentInfo.init),
            onRemoveAttachment: { clearAttachment() },
            supportsHostServices: appModel.supportsHostServices,
            placeholder: composerPlaceholder,
            canSend: canSend,
            isSending: isSending,
            onSend: handleSend
        )
        bar.composeFieldIdentifier = showsTimeline ? "timeline-compose-field" : "pane-compose-field"
        bar.composeSendIdentifier = showsTimeline ? "timeline-compose-send" : "pane-compose-send-button"
        return bar
    }

    private var liveInputEnabled: Bool {
        liveSession != nil && liveQueueState != .needsResync && !appModel.isOffline
    }

    /// Hard dismissal for the hidden Live capture field: clear the focus
    /// binding (so the bar's Keyboard icon unlights) and force UIKit-level
    /// first-responder resignation in one action.
    private func dismissLiveKeyboard() {
        guard liveInputFocused else { return }
        liveInputFocused = false
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )
    }

    private var outputWithInputDock: some View {
        VStack(spacing: 0) {
            Group {
                if showsTimeline {
                    AgentTimelineView(
                        paneId: pane.paneId, agent: pane.agent, connection: appModel.connection, timeline: timeline,
                        bridgeUpgrade: bridgeUpgrade,
                        topOverlayInset: topOutputInset, bottomOverlayInset: bottomDockHeight,
                        onOutputInteraction: { onOutputInteraction?() }, setupTarget: connectionScope,
                        onUseTerminal: {
                            UserDefaults.standard.set(true, forKey: TimelineSetupPreference.key(scope: connectionScope))
                            showsTimeline = false
                        },
                        onSetupComplete: {
                            let key = TimelineSetupPreference.key(scope: connectionScope)
                            if UserDefaults.standard.bool(forKey: key) { UserDefaults.standard.removeObject(forKey: key) }
                        },
                        onFocusComposer: focusApprovalComposer
                    )
                } else {
                    outputScrollView
                        .environment(\.colorScheme, .dark)
                        .background(
                            Color(uiColor: UIColor(hex: terminalPalette.background)),
                            ignoresSafeAreaEdges: .bottom
                        )
                }
            }
            // Extend only the output surface behind the keyboard and home
            // indicator; the navigation bar keeps the app theme.
            .background {
                (showsTimeline ? Theme.background : Color(uiColor: UIColor(hex: terminalPalette.background)))
                    .ignoresSafeArea(edges: .bottom)
            }
        }
        .overlay(alignment: .bottom) {
            VStack(spacing: 0) {
                if showsTimeline {
                    if let sessionError {
                        Text(sessionError).font(.jost(.caption)).foregroundStyle(Theme.destructive)
                            .padding(.horizontal, 16)
                    }
                } else {
                    liveStatusRow
                }
                voiceComposeBar
                    .environment(\.colorScheme, showsTimeline ? colorScheme : .dark)
            }
            .background {
                GeometryReader { geometry in
                    Color.clear.preference(
                        key: PaneBottomDockHeightPreferenceKey.self, value: geometry.size.height)
                }
            }
        }
        .onChange(of: showsTimeline) { _, _ in
            dismissLiveKeyboard(); composeFieldFocused = false; dictation.cancel()
            surfaceModel.resetToDefaultSurface()
            updateSessionParsedOutput()
        }
        .onPreferenceChange(PaneBottomDockHeightPreferenceKey.self) { height in
            if height > 0 { bottomDockHeight = height }
        }
    }

    private var outputWithLiveTasks: some View {
        outputWithInputDock
            .task(id: pane.id) {
                locallyClearedDone = false
                autoMarkedPaneID = nil
                await reload()
                updateSessionParsedOutput()
                await considerImplicitMarkSeen()
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .seconds(5)) } catch { return }
                    let auto = spacesModel?.autoRefreshEnabled ?? true
                    guard
                        PaneRefreshPolicy.shouldAutoPoll(
                            autoRefreshEnabled: auto,
                            isSceneActive: appModel.isSceneActive
                        )
                    else { continue }
                    await reload()
                    updateSessionParsedOutput()
                }
            }
        .task(id: liveSessionTaskKey) {
            var retryDelay = 2.0
            while !Task.isCancelled && !appModel.isOffline && appModel.isSceneActive {
                if liveSession == nil || liveQueueState == .needsResync {
                    await resumeLiveInput()
                    retryDelay = liveInputEnabled ? 2 : min(retryDelay * 2, 10)
                }
                do {
                    try await Task.sleep(for: .seconds(liveInputEnabled ? 1 : retryDelay))
                } catch { return }
            }
        }
        .task(id: liveRefreshTaskKey) {
            await runLiveForegroundRefresh()
        }
        .onChange(of: appModel.isOffline) { _, offline in
            // A disconnect invalidates the Live session generation; queued
            // input is discarded rather than replayed after reconnect, and a
            // half-armed modifier must not leak across SSH generations.
            guard offline else { return }
            abandonLiveInput()
            resetLiveModifiers()
        }
        .onChange(of: liveAcknowledgedCount) { oldCount, newCount in
            // Per-event acknowledgement: refresh pacing restarts and metrics
            // consume one FIFO enqueue timestamp per delivered event — even
            // while later events are still in flight. SwiftUI may coalesce
            // several increments into one observation, so apply the old→new
            // delta once per newly acknowledged event; a replaced queue
            // (count resets to 0) yields a negative delta and is skipped —
            // its timestamps were already abandoned.
            let delta = PaneLiveInputMetrics.acknowledgedDelta(from: oldCount, to: newCount)
            guard delta > 0 else { return }
            for _ in 0..<delta {
                liveMetrics.noteAcknowledged()
            }
            lastLiveAckAt = ProcessInfo.processInfo.systemUptime
        }
        .onChange(of: isLiveDraining) { _, draining in
            // An ambiguous send is discarded. The session task acquires a
            // fresh token for future input without replaying uncertain keys.
            guard !draining, liveQueue != nil, liveQueueState == .needsResync else { return }
            liveMetrics.noteDeliveryFailure()
            lastLiveAckAt = nil
            PaneLiveSessionTeardown.retire(&liveSession, connection: appModel.connection)
            resetLiveModifiers()
        }
    }

    private var outputWithLifecycle: some View {
        outputWithLiveTasks
        .task(id: showComposeByDefault) {
            surfaceModel.setComposeVisibleByDefault(showComposeByDefault)
            guard showComposeByDefault else { return }
            // Let SwiftUI mount the Compose field before requesting focus.
            await Task.yield()
            guard !Task.isCancelled, surfaceModel.isComposeSurface else { return }
            dismissLiveKeyboard()
            composeFieldFocused = true
        }
        .onChange(of: showComposeByDefault) { _, visible in
            surfaceModel.setComposeVisibleByDefault(visible)
            if !visible, trimmedDraft.isEmpty, attachment == nil {
                surfaceModel.collapseToLiveToolbar()
            }
        }
        .onDisappear {
            flushDraftSave()
            dictation.cancel()
            attachmentUploadTask?.cancel()
            abandonLiveInput()
            surfaceModel.resetToDefaultSurface()
            resetLiveModifiers()
        }
        .onChange(of: pane) { _, newPane in
            handlePaneChange(newPane)
        }
        .onChange(of: session.output) { _, _ in
            updateSessionParsedOutput()
        }
        .onChange(of: terminalPalette) { _, _ in
            updateSessionParsedOutput()
        }
        .onChange(of: doneClearRaw) { _, _ in
            Task { await considerImplicitMarkSeen() }
        }
        .onChange(of: draft) { _, newDraft in
            scheduleDraftSave(newDraft)
        }
        .onChange(of: persistence.revision) { _, revision in
            draftSaveTask?.cancel()
            draftSaveTask = nil
            persistenceRevision = revision
            draft = ""
        }
        .onChange(of: pane.id) { _, _ in
            // A different pane must never receive queued Live events, a
            // stale session token, an armed modifier, or another pane's
            // input surface.
            abandonLiveInput()
            surfaceModel.resetToDefaultSurface()
            resetLiveModifiers()
        }
        .onChange(of: scenePhase) { _, phase in
            appModel.isSceneActive = phase == .active
            if phase == .background {
                abandonLiveInput()
                flushDraftSave()
                dictation.cancel()
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: AVAudioSession.interruptionNotification)) { _ in
            dictation.cancel()
        }
        .onChange(of: photoItem) { _, item in
            guard let item else { return }
            isLoadingPhoto = true
            surfaceModel.notePhotoSelected()
            Task { await attachPhoto(item) }
        }
        .onChange(of: isPhotoPickerPresented) { _, isPresented in
            if !isPresented && attachment != nil {
                composeFieldFocused = true
            }
        }
        .alert("Dictation unavailable", isPresented: Binding<Bool>(
            get: { dictationError != nil },
            set: { if !$0 { dictationError = nil } }
        )) {
            Button("OK", role: .cancel) { dictationError = nil }
        } message: {
            Text(dictationError ?? "")
        }
    }

    var body: some View {
        outputWithLifecycle
        .animation(.easeInOut(duration: 0.2), value: surfaceModel.surface)
        .onChange(of: composeFieldFocused) { _, focused in
            if focused {
                onOutputInteraction?()
            } else if surfaceModel.isComposeSurface,
                      !showComposeByDefault,
                      trimmedDraft.isEmpty,
                      !isPhotoPickerPresented,
                      !isLoadingPhoto,
                      attachment == nil {
                // Blank-bubble exit: the same gesture that dismisses the
                // keyboard (swipe down on the bubble, interactive output
                // drag) collapses an empty Compose bubble back to the Live
                // toolbar. A draft with text or an attachment never collapses.
                surfaceModel.collapseToLiveToolbar()
            }
        }
        .onChange(of: liveInputFocused) { _, focused in
            if focused {
                onOutputInteraction?()
            }
        }
        .onChange(of: dictation.isBusy) { _, busy in
            guard !busy else { return }
            completeDictationSurface()
        }
    }

    // MARK: Live input

    private func focusApprovalComposer() {
        if surfaceModel.surface == .voiceRecording {
            finishVoice()
            return
        }
        surfaceModel.requestComposeDraft()
        dismissLiveKeyboard()
        composeFieldFocused = true
    }

    private func openCompose() {
        guard surfaceModel.requestComposeDraft() else { return }
        dismissLiveKeyboard()
        composeFieldFocused = true
    }

    /// Restarted per pane (and on reconnect), acquiring a session token
    /// after a successful pane read. The Voice/Compose detour keeps the
    /// session warm — it is the same pane and SSH generation.
    private var liveSessionTaskKey: String {
        guard !appModel.isOffline, appModel.isSceneActive else { return "off" }
        return "live-session:\(pane.paneId)"
    }

    /// Restarted when the scene is foregrounded; cancelled in the
    /// background so rapid reads never run there.
    private var liveRefreshTaskKey: String {
        guard appModel.isSceneActive else { return "off" }
        return "live-refresh:\(pane.paneId)"
    }

    /// Acquires a fresh Live session token and installs it only after a
    /// successful fresh pane read. `beginPaneLiveInput` runs FIRST so its
    /// input-lane barrier waits out any in-flight Compose mutation before
    /// Live can own the keyboard; the read then confirms the pane state
    /// before input is enabled. Used on Live entry and automatic recovery
    /// after an ambiguous delivery failure; discarded events are never
    /// replayed. Re-checks mode, pane identity, connection, and task
    /// lifetime after every await so a stale token can never be installed
    /// for a pane or mode the user has left.
    private func resumeLiveInput() async {
        let targetPaneId = pane.paneId
        guard !appModel.isOffline else { return }
        PaneLiveSessionTeardown.retire(&liveSession, connection: appModel.connection)
        resetLiveModifiers()
        do {
            let token = try await appModel.connection.beginPaneLiveInput(paneId: targetPaneId)
            var installed = false
            defer {
                if !installed {
                    Task { await appModel.connection.endPaneLiveInput(token) }
                }
            }
            guard !Task.isCancelled,
                  pane.paneId == targetPaneId,
                  !appModel.isOffline else { return }
            await reload()
            updateSessionParsedOutput()
            guard !Task.isCancelled,
                  pane.paneId == targetPaneId,
                  !appModel.isOffline else { return }
            guard session.errorMessage == nil else {
                if session.errorMessage?.contains("Timed out") == true {
                    liveMetrics.noteTimeout()
                }
                return
            }
            let queue = liveQueue ?? PaneLiveInputQueue(sender: appModel.connection)
            if queue.state == .needsResync {
                queue.noteResynced()
            }
            liveQueue = queue
            liveSession = token
            installed = true
            liveModifierComposer.noteSession(token)
        } catch {
            // Keep later input disabled until the recovery loop acquires a
            // fresh token; never retry an uncertain key.
            PaneLiveSessionTeardown.retire(&liveSession, connection: appModel.connection)
        }
    }

    /// Routes every Live pane mutation through the ordered queue. Controls
    /// are disabled without a session token, and the token's pane is
    /// re-checked so a stale view can never type into another pane. Only the
    /// Live toolbar surface accepts live keys — the recording bar and
    /// Compose bubble never enqueue terminal input.
    private func sendLiveEvent(_ event: PaneLiveInputEvent) {
        guard surfaceModel.isLiveKeyboardSurface,
              let queue = liveQueue,
              let session = liveSession,
              session.paneId == pane.paneId,
              !appModel.isOffline else { return }
        guard queue.enqueue(event, session: session) else {
            // Rejection means the queue is full or stopped. Discard the
            // rejected key and backlog; the recovery loop reacquires a
            // fresh session for later input. Nothing is replayed.
            liveMetrics.noteDiscardedEvents(1)
            liveMetrics.noteAbandoned()
            queue.abandonQueuedInput()
            PaneLiveSessionTeardown.retire(&liveSession, connection: appModel.connection)
            lastLiveAckAt = nil
            return
        }
        liveMetrics.noteEnqueued(depth: queue.pendingCount)
    }

    /// Drops queued Live events and the session token on pane switch,
    /// disconnect, or leaving the pane. Accepted-but-undelivered events are
    /// discarded — never replayed into another pane or SSH generation.
    private func abandonLiveInput() {
        if let liveQueue {
            liveMetrics.noteAbandoned()
            liveQueue.abandonQueuedInput()
        }
        PaneLiveSessionTeardown.retire(&liveSession, connection: appModel.connection)
        lastLiveAckAt = nil
        lastLiveReadAt = nil
        resetLiveModifiers()
    }

    /// Toolbar / Live keyboard events flow through Cursor's modifier
    /// composer first; only its `.enqueue` outcomes reach the ordered queue.
    /// The visual armed state always mirrors the composer, so a one-shot
    /// modifier (or a multi-char paste that disarms without chord-wrapping)
    /// clears the highlight immediately.
    private func applyLiveModifierOutcome(_ outcome: PaneLiveModifierComposer.Outcome) {
        switch outcome {
        case .state:
            break
        case let .enqueue(event):
            sendLiveEvent(event)
        case .unsupportedCommand:
            // ⌘ has no verified agent-TUI mapping; fail closed and say so.
            sessionError = "Command (⌘) chords aren’t supported with this agent yet. Nothing was sent."
        }
        armedLiveModifiers = Set(liveModifierComposer.armed.compactMap { modifier in
            switch modifier {
            case .control: return PaneLiveModifier.ctrl
            case .alt: return .alt
            case .shift: return .shift
            case .command: return nil
            }
        })
    }

    /// Clears the armed modifier and its visual state on pane switch,
    /// disconnect, or any session invalidation.
    private func resetLiveModifiers() {
        liveModifierComposer.noteSession(nil)
        armedLiveModifiers = []
    }

    /// Voice entry from the Live toolbar or from inside the Compose bubble.
    /// Both live capture and compose focus yield to the microphone; the
    /// existing pane-scoped draft seeds the transcript so the new speech
    /// merges into (never replaces) unsent text.
    private func startVoice() {
        surfaceModel.requestVoiceRecording()
        liveInputFocused = false
        composeFieldFocused = false
        let id = UUID()
        voiceSession = id
        let dictationRevision = persistenceRevision
        Task {
            await dictation.start { transcript in
                guard persistence.revision == dictationRevision else { return }
                draft = transcript
            } onError: { message in
                dictationError = message
                // Authorization/microphone failures cycle isBusy true→false
                // fast enough that the onChange transition can coalesce away;
                // the error callback opens the preserved Compose draft itself
                // so the recording surface can never get stuck.
                completeDictationSurface()
            } initialText: { draft }
            guard !Task.isCancelled else { return }
            if voiceSession != id {
                // The user tapped the checkmark (or left) while setup was
                // still running. `PaneDictation.cancel()` already ran, but
                // the setup tail can revive the engine after it — force it
                // down and open the preserved draft.
                dictation.cancel()
                completeDictationSurface()
            } else if surfaceModel.surface == .voiceRecording, !dictation.isBusy {
                // Fast-fail that returned without an error callback.
                completeDictationSurface()
            }
        }
    }

    /// The recording bar's single action: stop capture. `finish()` returns
    /// at once (cancelling outright when no recognition request exists yet,
    /// e.g. mid-authorization) while final recognition drains — isBusy stays
    /// true up to ~15s. `completeDictationSurface()` — from the isBusy
    /// transition, the error callback, or the reconciliation above — is the
    /// only path that opens the Compose bubble, so a partial transcript is
    /// never shown or sent.
    private func finishVoice() {
        guard surfaceModel.requestFinishDictation() else { return }
        voiceSession = nil
        dictation.finish()
    }

    /// Single idempotent funnel off the recording surface onto the Compose
    /// bubble; repeat calls no-op inside the surface state machine. The
    /// final `onText` (full merged transcript, prior draft included) always
    /// precedes `isBusy` clearing on the main actor, so the bubble opens
    /// with a complete transcript — on success, error, timeout, or
    /// interruption alike, preserving the captured draft.
    private func completeDictationSurface() {
        let wasRecording = surfaceModel.surface == .voiceRecording
        surfaceModel.noteDictationIdle()
        if wasRecording, surfaceModel.isComposeSurface {
            composeFieldFocused = true
        }
    }

    /// Foreground-only rapid pane reads after acknowledged Live input.
    /// Coalesces reads per `PaneLiveRefreshPolicy`, then idles once typing
    /// stops; the five-second lifecycle-aware poll remains the fallback.
    private func runLiveForegroundRefresh() async {
        while !Task.isCancelled {
            do {
                try await Task.sleep(for: .seconds(PaneLiveRefreshPolicy.tickInterval))
            } catch { return }
            guard appModel.isSceneActive, !session.isLoading else { continue }
            let now = ProcessInfo.processInfo.systemUptime
            guard PaneLiveRefreshPolicy.shouldRead(
                now: now,
                lastReadAt: lastLiveReadAt,
                lastAcknowledgedAt: lastLiveAckAt
            ) else { continue }
            lastLiveReadAt = now
            let before = session.output
            await reload()
            updateSessionParsedOutput()
            if session.output != before {
                // Changed read is only a proxy for visible echo.
                liveMetrics.noteOutputChangedRead()
            }
        }
    }

    // MARK: Input bar

    private var markAsReadButton: some View {
        Button {
            Task { await markSeen(triggeredByUser: true) }
        } label: {
            Group {
                if isMarkingSeen {
                    CircularArcSpinner(size: 18)
                } else {
                    Image(systemName: "checkmark")
                        .font(.jost(17, weight: .semibold))
                        .foregroundStyle(Theme.onPrimary)
                }
            }
            .frame(width: 44, height: 44)
            .background(AgentStatus.done.color, in: Circle())
        }
        .buttonStyle(.plain)
        .disabled(isMarkingSeen)
        .accessibilityLabel("Mark as read")
        .accessibilityHint("Clears Done and marks this agent Idle")
        .accessibilityIdentifier("pane-mark-as-read-button")
    }

    private func scheduleDraftSave(_ text: String) {
        draftSaveTask?.cancel()
        let key = draftKey
        let revision = persistenceRevision
        draftSaveTask = Task {
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            persistence.saveDraft(text, key: key, revision: revision)
        }
    }

    private func flushDraftSave() {
        draftSaveTask?.cancel()
        draftSaveTask = nil
        persistence.saveDraft(draft, key: draftKey, revision: persistenceRevision)
    }

    private var trimmedDraft: String {
        draft.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var composerPlaceholder: String {
        if let agent = pane.agent { return "Message \(agent)…" }
        return "Type to send to this pane…"
    }

    // MARK: Data

    private func reload() async {
        await session.reload(appModel.connection)
    }

    /// Focuses the agent/tab on the Mac so Herdr clears Done → Idle.
    private func markSeen(triggeredByUser: Bool) async {
        guard pane.status == .done, !isMarkingSeen else { return }
        isMarkingSeen = true
        defer { isMarkingSeen = false }
        do {
            try await appModel.connection.markAgentSeen(
                paneId: pane.paneId,
                tabId: pane.tabId,
                hasAgent: pane.agent != nil
            )
            locallyClearedDone = true
            if triggeredByUser {
                HapticFeedback.selection()
            }
            onMarkedSeen?()
        } catch {
            if triggeredByUser {
                setSessionError(error)
            }
        }
    }

    private func setSessionError(_ error: Error) {
        let message = HerdrConnection.friendlyMessage(for: error)
        if !HerdrConnection.isDisconnectionOrTransitionMessage(message) {
            sessionError = message
        }
    }

    private func considerImplicitMarkSeen() async {
        guard doneClearPreference == .implicit else { return }
        guard pane.status == .done else { return }
        guard autoMarkedPaneID != pane.paneId else { return }
        autoMarkedPaneID = pane.paneId
        await markSeen(triggeredByUser: false)
    }

    private func send() async {
        guard canSend else { return }
        draftSaveTask?.cancel()
        draftSaveTask = nil
        HapticFeedback.impact(.medium)
        let attachedPath = attachment?.remotePath
        // Bind to the pane that owns this session view so a mid-flight switch
        // cannot misroute the upload prompt.
        guard attachedPath == nil || attachment?.paneId == pane.paneId else {
            sessionError = "Image belongs to a different pane — remove it and reattach."
            return
        }
        let text = attachedPath.map {
            HerdrConnection.imagePrompt(remotePath: $0, userMessage: trimmedDraft)
        } ?? trimmedDraft
        guard !text.isEmpty else { return }

        let sendRevision = persistenceRevision
        let sentDraft = trimmedDraft
        let originalDraft = draft
        do {
            if showsTimeline, let identity = timeline.identity {
                guard let owned=appModel.ownership.transportGeneration else {throw HerdrError.notConnected}
                isTimelineSending=true
                defer {isTimelineSending=false}
                try await appModel.connection.submitTimelinePrompt(text:text,identity:identity,owned:owned)
            } else {
                try await session.send(text:text,connection:appModel.connection)
            }
            guard persistence.revision == sendRevision else { return }
            // Remember what the user typed (not image-path wrappers) for reuse.
            persistence.record(sentDraft, key: historyKey, revision: sendRevision)
            if TimelineComposePolicy.shouldClearDraft(current:draft,sent:originalDraft) { draft = "" }
            clearAttachment()
            shouldPinToBottom = true
            // Return to the preferred idle surface only after a successful send,
            // dismissing the keyboard while keeping Compose visible if enabled.
            surfaceModel.noteSendSucceeded()
            composeFieldFocused = false
            liveInputFocused = false
            // Give the remote terminal a beat to echo the input before re-reading.
            try? await Task.sleep(for: .milliseconds(800))
            await reload()
            updateSessionParsedOutput()
        } catch {
            // Keep draft + attachment so the user can retry; the Compose
            // bubble stays up and the existing error banner reports why.
            setSessionError(error)
        }
    }

    // MARK: Attachment

    private func attachPhoto(_ item: PhotosPickerItem) async {
        attachmentUploadTask?.cancel()
        photoItem = nil
        defer { isLoadingPhoto = false }
        do {
            guard let data = try await item.loadTransferable(type: Data.self),
                  let image = UIImage(data: data) else {
                sessionError = "Could not load that image."
                return
            }
            let usePNG = item.supportedContentTypes.contains { $0.identifier == "public.png" }
            guard let encoded = PaneImageEncoder.encode(image, prefersPNG: usePNG) else {
                sessionError = "Could not encode that image."
                return
            }
            let pending = PendingPaneAttachment(
                id: UUID(),
                paneId: pane.paneId,
                preview: encoded.preview,
                imageData: encoded.data,
                fileExtension: encoded.fileExtension,
                remotePath: nil,
                uploadError: nil,
                isUploading: true
            )
            attachment = pending
            surfaceModel.notePhotoSelected()
            if !isPhotoPickerPresented {
                composeFieldFocused = true
            }
            attachmentUploadTask = Task {
                await uploadAttachment(pending)
            }
        } catch {
            setSessionError(error)
        }
    }

    private func uploadAttachment(_ pending: PendingPaneAttachment) async {
        do {
            let path = try await appModel.connection.uploadAttachment(
                data: pending.imageData,
                fileExtension: pending.fileExtension
            )
            guard !Task.isCancelled else { return }
            guard attachment?.id == pending.id else { return }
            attachment = pending.with(remotePath: path, uploadError: nil, isUploading: false)
        } catch {
            guard !Task.isCancelled else { return }
            guard attachment?.id == pending.id else { return }
            attachment = pending.with(
                remotePath: nil,
                uploadError: HerdrConnection.friendlyMessage(for: error),
                isUploading: false
            )
        }
    }

    private func clearAttachment() {
        attachmentUploadTask?.cancel()
        attachmentUploadTask = nil
        attachment = nil
        photoItem = nil
    }
}

// MARK: - Pane input surface state machine

/// Which input surface a pane shows now that the Compose/Live segmented
/// control is gone: the Live toolbar (default), the Voice recording
/// bar, or the editable Compose bubble reached by Keyboard, Voice, or a photo.
/// Pure transition logic owned by `PaneSessionView` wiring — view code maps
/// these transitions to `PaneDictation`, keyboard focus, and the existing
/// Compose send path.
@MainActor
@Observable
final class PaneInputSurfaceModel {
    enum Surface: Equatable {
        case liveToolbar
        case voiceRecording
        case composeDraft
    }

    private(set) var surface: Surface = .liveToolbar
    private var composeVisibleByDefault = false

    /// Applies the saved default without interrupting recording or discarding a draft.
    func setComposeVisibleByDefault(_ visible: Bool) {
        composeVisibleByDefault = visible
        if visible, surface == .liveToolbar {
            surface = .composeDraft
        }
    }
    /// True after the recording bar's checkmark stops capture and while final
    /// recognition drains — `PaneDictation.finish()` returns immediately but
    /// `isBusy` stays true up to ~15s until the final transcript lands. The
    /// recording surface must stay mounted and inert during the drain so an
    /// incomplete transcript is never shown or sent.
    private(set) var isDrainingDictation = false

    var isLiveKeyboardSurface: Bool { surface == .liveToolbar }
    var isComposeSurface: Bool { surface == .composeDraft }

    /// Direct entry from the Live toolbar keeps any existing pane draft.
    @discardableResult
    func requestComposeDraft() -> Bool {
        guard surface == .liveToolbar else { return false }
        surface = .composeDraft
        return true
    }

    /// The photo picker can resign Compose focus and return to Live before its
    /// image finishes loading. A selected photo always restores Compose.
    func notePhotoSelected() {
        guard surface != .voiceRecording else { return }
        surface = .composeDraft
    }

    /// Voice starts from the toolbar, or again from inside the Compose bubble,
    /// merging the new transcript into the existing pane-scoped draft (the
    /// merge itself is `PaneDictation`'s `initialText` behavior).
    func requestVoiceRecording() {
        guard surface != .voiceRecording else { return }
        surface = .voiceRecording
        isDrainingDictation = false
    }

    /// The recording bar's single action: stop capture. Returns false (and
    /// does nothing) for repeat taps or taps outside the recording surface.
    /// The surface stays `.voiceRecording` (draining) until dictation reports
    /// idle; `PaneDictation` delivers its final `onText` (full merged
    /// transcript, prior draft included) on the main actor *before* `isBusy`
    /// clears, so opening the Compose bubble on idle can never surface a
    /// partial transcript.
    @discardableResult
    func requestFinishDictation() -> Bool {
        guard surface == .voiceRecording, !isDrainingDictation else { return false }
        isDrainingDictation = true
        return true
    }

    /// `isBusy` became false — successful drain, error, timeout, or
    /// interruption alike. Opening the bubble on error and timeout too is
    /// deliberate: the captured draft stays visible, editable, and unsent,
    /// and the existing dictation error alert reports what went wrong.
    func noteDictationIdle() {
        guard surface == .voiceRecording else { return }
        isDrainingDictation = false
        surface = .composeDraft
    }

    /// Send success returns to the preferred idle surface. Failures never call this,
    /// so the bubble (with its retained draft and attachment) stays up for a
    /// retry alongside the existing error banner.
    func noteSendSucceeded() {
        guard surface == .composeDraft else { return }
        surface = composeVisibleByDefault ? .composeDraft : .liveToolbar
    }

    /// Explicit collapse from the Compose bubble back to the Live toolbar
    /// without sending. The pane-scoped draft is preserved untouched.
    func collapseToLiveToolbar() {
        guard surface == .composeDraft else { return }
        surface = .liveToolbar
    }

    /// Pane switches and view teardown reset to the preferred default surface.
    func resetToDefaultSurface() {
        surface = composeVisibleByDefault ? .composeDraft : .liveToolbar
        isDrainingDictation = false
    }
}

extension PaneInputSurfaceModel.Surface {
    /// Visual mode of `PaneVoiceComposeBar` for this surface.
    var barMode: PaneVoiceComposeBarMode {
        switch self {
        case .liveToolbar: .live
        case .voiceRecording: .recording
        case .composeDraft: .compose
        }
    }
}

extension PaneAttachmentInfo {
    fileprivate init(_ pending: PendingPaneAttachment) {
        self.init(
            id: pending.id,
            preview: pending.preview,
            remotePath: pending.remotePath,
            uploadError: pending.uploadError,
            isUploading: pending.isUploading
        )
    }
}

/// DEBUG-only aggregate Live diagnostics formatting, kept by the pane wiring
/// so `PaneVoiceComposeBar` stays a pure visual component. Durations,
/// depths, and counts only — no text, output, host, pane, machine, or
/// credential data. `outΔ` is the acknowledgement → next changed read
/// duration, a proxy for echo.
enum PaneLiveDiagnostics {
    static func summaryLine(for summary: PaneLiveInputMetricsSummary) -> String {
        func fmt(_ percentiles: PaneLiveInputMetricPercentiles) -> String {
            guard let p50 = percentiles.p50Millis, let p95 = percentiles.p95Millis else {
                return "n/a"
            }
            return "p50\(Int(p50.rounded()))ms p95\(Int(p95.rounded()))ms n\(percentiles.sampleCount)"
        }
        return "ack \(fmt(summary.enqueueToAck)) · outΔ \(fmt(summary.ackToOutputChange)) · "
            + "depth \(summary.maxQueueDepth) · "
            + "timeouts \(summary.timeoutCount) · fails \(summary.failureCount) · "
            + "dropped \(summary.discardedEventCount)"
    }
}

private struct PaneOutputScrollView: View, Equatable {
    let output: String
    let outputLines: [ANSIText.Line]
    let outputRevision: UInt64
    let outputFont: Font
    let paneScrollBehavior: PaneScrollBehaviorPreference
    /// True while either keyboard surface (Compose field or hidden Live
    /// capture) holds focus, driving the focus-change and keyboard-frame
    /// repin guards.
    let keyboardFocused: Bool
    let isOffline: Bool
    let isLoading: Bool
    /// True while the floating mark-as-read control is shown; only then
    /// does the output reserve room below its newest line.
    let reservesMarkAsReadSpace: Bool
    let topOverlayInset: CGFloat
    let bottomOverlayInset: CGFloat
    let paneId: String
    @Binding var shouldPinToBottom: Bool
    let onUserScroll: () -> Void
    let onTapOutput: () -> Void
    /// Deliberate downward-drag dismissal for the Live keyboard capture
    /// field — a UIKit `UITextField` outside this scroll view that neither
    /// SwiftUI's `.scrollDismissesKeyboard` nor scroll-phase callbacks can
    /// reliably reach. Fired from a content-level simultaneous drag.
    var onDismissKeyboardDrag: (() -> Void)?

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.outputRevision == rhs.outputRevision
            && lhs.isLoading == rhs.isLoading
            && lhs.isOffline == rhs.isOffline
            && lhs.reservesMarkAsReadSpace == rhs.reservesMarkAsReadSpace
            && lhs.topOverlayInset == rhs.topOverlayInset
            && lhs.bottomOverlayInset == rhs.bottomOverlayInset
            && lhs.outputFont == rhs.outputFont
            && lhs.paneScrollBehavior == rhs.paneScrollBehavior
            && lhs.keyboardFocused == rhs.keyboardFocused
            && lhs.shouldPinToBottom == rhs.shouldPinToBottom
            && lhs.paneId == rhs.paneId
    }

    private struct RenderedLine: Identifiable {
        let id: Int
        let attributed: AttributedString
        let tint: PaneOutputPresentation.DiffTint?
    }

    private var renderedLines: [RenderedLine] {
        var oldHunkLines = 0
        var newHunkLines = 0
        return outputLines.enumerated().map { index, line in
            let tint = PaneOutputPresentation.diffTint(
                for: line.plain,
                oldHunkLines: &oldHunkLines,
                newHunkLines: &newHunkLines
            )
            let attributed = line.attributed.characters.isEmpty
                ? AttributedString(" ")
                : line.attributed
            return RenderedLine(id: index, attributed: attributed, tint: tint)
        }
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if output.isEmpty && !isLoading && !isOffline {
                        Text("No output yet")
                            .font(outputFont)
                            .foregroundStyle(Color.secondary)
                            .padding(.bottom, 4)
                    }
                    ForEach(renderedLines) { line in
                        Text(line.attributed)
                            .font(outputFont)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, line.tint == nil ? 0 : 6)
                            .padding(.vertical, line.tint == nil ? 0 : 1)
                            .background(diffBackground(line.tint))
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                }
                .padding(14)
                .padding(.top, topOverlayInset)
                #if DEBUG
                .paneScrollContentProbe()
                #endif
                // The scroll view spans behind the floating dock. Its bottom
                // marker stays above the dock at every keyboard and bar height.
                Color.clear
                    .frame(height: bottomOverlayInset + (reservesMarkAsReadSpace ? 68 : 0))
                    .id("pane-output-bottom")
            }
            // Content-level simultaneous drag: unlike gestures attached to
            // the ScrollView itself (never delivered beside the native pan
            // in the failing runs), a drag on the content VStack fires
            // reliably during scrolling, including over selectable text.
            .contentShape(Rectangle())
            .simultaneousGesture(
                TapGesture().onEnded { onTapOutput() }
            )
            .simultaneousGesture(
                DragGesture(minimumDistance: 20)
                    .onEnded { value in
                        guard value.translation.height > 20,
                              value.translation.height > abs(value.translation.width) else { return }
                        onDismissKeyboardDrag?()
                    }
            )
            #if DEBUG
            .paneScrollDiagnostics(tag: paneId)
            #endif
            // Dragging down through the output pulls the composer keyboard
            // away with the finger, as in Messages.
            .scrollDismissesKeyboard(.interactively)
            .accessibilityIdentifier("pane-output-scroll-view")
            // UIKit bridge: SwiftUI's `.scrollDismissesKeyboard` manages only
            // SwiftUI-focus text fields on this OS; force the underlying
            // UIScrollView into native interactive dismissal, which tracks
            // the finger and resigns ANY first responder — including the
            // Live capture UITextField living outside the scroll view.
            .background(ScrollViewKeyboardDismissBridge())
            .modifier(UserScrollObserver(action: onUserScroll))
            // Pin to the newest output when content first arrives and
            // again after the user sends input. During periodic background
            // refreshes the reader is only moved when they opted into
            // `.followOutput`; otherwise their position is held.
            .onChange(of: output) { _, newOutput in
                guard !newOutput.isEmpty else { return }
                guard shouldPinToBottom || paneScrollBehavior == .followOutput else { return }
                withAnimation(.easeOut(duration: 0.25)) {
                    proxy.scrollTo("pane-output-bottom", anchor: .bottom)
                }
                shouldPinToBottom = false
            }
            // Either keyboard surface (Compose bubble or the hidden Live
            // capture) shrinks the scroll viewport when it gains focus;
            // re-pin so the newest output stays visible above the keyboard
            // instead of hidden behind it.
            .onChange(of: keyboardFocused) { _, focused in
                guard focused, !output.isEmpty else { return }
                withAnimation(.easeOut(duration: 0.25)) {
                    proxy.scrollTo("pane-output-bottom", anchor: .bottom)
                }
            }
            // A growing Compose field moves the dock upward. Keep the newest
            // line in the same place above its new edge.
            .onChange(of: bottomOverlayInset) { _, _ in
                guard !output.isEmpty else { return }
                withAnimation(.easeOut(duration: 0.25)) {
                    proxy.scrollTo("pane-output-bottom", anchor: .bottom)
                }
            }
            // Re-pin once the keyboard settles at its final frame while
            // typing (QuickType bar height changes, hardware keyboard
            // attach). Skips hide events so a reader who scrolled up with
            // the keyboard open keeps their position when it dismisses.
            .onReceive(
                NotificationCenter.default.publisher(
                    for: UIResponder.keyboardWillChangeFrameNotification
                )
            ) { note in
                guard keyboardFocused, !output.isEmpty,
                      let endFrame = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect,
                      endFrame.minY < UIScreen.main.bounds.height
                else { return }
                withAnimation(.easeOut(duration: 0.25)) {
                    proxy.scrollTo("pane-output-bottom", anchor: .bottom)
                }
            }
        }
        .overlay {
            if output.isEmpty {
                if isOffline {
                    OfflineHerdView()
                        .padding(.bottom, 68)
                        .allowsHitTesting(false)
                } else if isLoading {
                    CircularArcSpinner("Reading pane…")
                }
            }
        }
    }

    private func diffBackground(_ tint: PaneOutputPresentation.DiffTint?) -> Color {
        switch tint {
        case .addition: Theme.diffAdditionBackground
        case .deletion: Theme.diffDeletionBackground
        case nil: Color.clear
        }
    }
}

/// UIKit bridge that forces the terminal output's underlying `UIScrollView`
/// into native interactive keyboard dismissal. SwiftUI's
/// `.scrollDismissesKeyboard` manages only SwiftUI-focus text fields on this
/// OS; UIKit's `keyboardDismissMode` tracks the finger and resigns ANY first
/// responder — including the Live capture `UITextField` living outside the
/// scroll view. Re-asserted on every SwiftUI update in case SwiftUI resets
/// the property it also manages.
private struct ScrollViewKeyboardDismissBridge: UIViewRepresentable {
    final class Coordinator {
        weak var scrollView: UIScrollView?

        func apply(to view: UIView) {
            if let scrollView, scrollView.window != nil {
                scrollView.keyboardDismissMode = .interactive
                return
            }
            var current: UIView? = view.superview
            while let candidate = current {
                if let scrollView = candidate as? UIScrollView {
                    self.scrollView = scrollView
                    scrollView.keyboardDismissMode = .interactive
                    return
                }
                current = candidate.superview
            }
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> UIView {
        let view = UIView(frame: .zero)
        view.backgroundColor = .clear
        view.isUserInteractionEnabled = false
        view.isHidden = true
        return view
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        context.coordinator.apply(to: uiView)
    }
}

/// Reports finger-driven scrolling, ignoring programmatic pins to the bottom.
private struct UserScrollObserver: ViewModifier {
    let action: () -> Void

    func body(content: Content) -> some View {
        if #available(iOS 18.0, *) {
            content.onScrollPhaseChange { _, phase in
                if phase == .interacting { action() }
            }
        } else {
            // iOS 17 has no scroll-phase API; a simultaneous drag observes
            // the finger without taking the scroll away from the view.
            content.simultaneousGesture(
                DragGesture(minimumDistance: 20).onChanged { _ in action() }
            )
        }
    }
}

enum AddPaneKind: String, CaseIterable {
    case pane
    case tab
}

private struct AddPaneRequest {
    var kind: AddPaneKind
    var name: String
}

// MARK: - Workspace actions

/// A workspace-level action awaiting confirmation in a drawer.
enum WorkspaceAction: Identifiable {
    case rename(Workspace)
    case close(Workspace)
    case deleteWorktree(Workspace)

    var id: String {
        switch self {
        case .rename: "rename"
        case .close: "close"
        case .deleteWorktree: "deleteWorktree"
        }
    }

    var workspace: Workspace {
        switch self {
        case let .rename(workspace), let .close(workspace), let .deleteWorktree(workspace):
            workspace
        }
    }
}

/// Values collected by the confirmation drawer for a workspace action.
enum WorkspaceActionOutcome {
    case rename(String)
    case close
    case deleteWorktree(force: Bool, trustRepository: Bool)
}

/// Bottom drawer that confirms workspace-level actions (rename, close,
/// delete worktree checkout) before they run on the remote machine.
private struct WorkspaceActionDrawer: View {
    let action: WorkspaceAction
    let onConfirm: (WorkspaceActionOutcome) -> Void
    let onCancel: () -> Void

    @State private var name: String
    @State private var force = false
    @State private var trustRepository = false

    init(
        action: WorkspaceAction,
        onConfirm: @escaping (WorkspaceActionOutcome) -> Void,
        onCancel: @escaping () -> Void
    ) {
        self.action = action
        self.onConfirm = onConfirm
        self.onCancel = onCancel
        _name = State(initialValue: action.workspace.label)
    }

    private var trimmedName: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    static func estimatedHeight(for action: WorkspaceAction) -> CGFloat {
        switch action {
        case .rename: 320
        case .close: 300
        case .deleteWorktree: 470
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            switch action {
            case .rename:
                Text("Rename Workspace")
                    .font(.jost(.title3, weight: .semibold))
                Text("Enter a new label for this workspace. Pane names are unchanged.")
                    .font(.jost(.subheadline))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 8) {
                    Text("Workspace name")
                        .font(.jost(.caption, weight: .semibold))
                        .foregroundStyle(.secondary)
                    TextField("Workspace Name", text: $name)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                        .background(
                            Theme.fieldBackground,
                            in: RoundedRectangle.continuous(DesignSystem.CornerRadius.md)
                        )
                        .accessibilityIdentifier("workspace-rename-text-field")
                }
                Spacer(minLength: 8)
                VStack(spacing: 10) {
                    Button("Rename Workspace") {
                        onConfirm(.rename(name))
                    }
                    .buttonStyle(.herdrPrimary())
                    .disabled(trimmedName.isEmpty)
                    .accessibilityIdentifier("workspace-rename-confirm-button")

                    Button("Cancel", action: onCancel)
                        .buttonStyle(.herdrGhost())
                }

            case .close:
                Text("Close Workspace")
                    .font(.jost(.title3, weight: .semibold))
                Text(
                    "This closes “\(action.workspace.label)” and stops all of its tabs and panes on the remote machine."
                )
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
                VStack(spacing: 10) {
                    Button("Close Workspace", role: .destructive) {
                        onConfirm(.close)
                    }
                    .buttonStyle(.herdrPrimary(tint: Theme.destructive))
                    .accessibilityIdentifier("workspace-close-confirm-button")

                    Button("Cancel", action: onCancel)
                        .buttonStyle(.herdrGhost())
                }

            case .deleteWorktree:
                Text("Delete Worktree Checkout")
                    .font(.jost(.title3, weight: .semibold))
                VStack(alignment: .leading, spacing: 8) {
                    Text("Checkout")
                        .font(.jost(.caption, weight: .semibold))
                        .foregroundStyle(.secondary)
                    Text(action.workspace.worktree?.checkoutPath ?? "")
                        .font(.system(size: 13, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                        .background(
                            Theme.fieldBackground,
                            in: RoundedRectangle.continuous(DesignSystem.CornerRadius.md)
                        )
                }
                Text(
                    "This deletes the checkout folder on the remote machine and closes this workspace. The worktree branch itself is not deleted."
                )
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                Toggle(isOn: $force) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Force delete")
                            .font(.jost(.subheadline, weight: .medium))
                        Text("Remove even when the checkout has uncommitted changes.")
                            .font(.jost(.caption))
                            .foregroundStyle(.secondary)
                    }
                }
                .toggleStyle(.switch)
                .accessibilityIdentifier("workspace-delete-force-toggle")
                Toggle(isOn: $trustRepository) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Trust repository")
                            .font(.jost(.subheadline, weight: .medium))
                        Text("Required when this checkout is the repository’s main checkout.")
                            .font(.jost(.caption))
                            .foregroundStyle(.secondary)
                    }
                }
                .toggleStyle(.switch)
                .accessibilityIdentifier("workspace-delete-trust-toggle")
                Spacer(minLength: 8)
                VStack(spacing: 10) {
                    Button("Delete Checkout", role: .destructive) {
                        onConfirm(.deleteWorktree(force: force, trustRepository: trustRepository))
                    }
                    .buttonStyle(.herdrPrimary(tint: Theme.destructive))
                    .accessibilityIdentifier("workspace-delete-confirm-button")

                    Button("Cancel", action: onCancel)
                        .buttonStyle(.herdrGhost())
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 24)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Theme.backgroundGradient.ignoresSafeArea())
    }
}

private struct AddPaneDrawer: View {
    let onAdd: (AddPaneRequest) -> Void
    let onCancel: () -> Void

    @State private var kind: AddPaneKind = .pane
    @State private var name = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Add to Space")
                .font(.jost(.title3, weight: .semibold))
            Picker("Add", selection: $kind) {
                Text("Pane").tag(AddPaneKind.pane)
                Text("Tab").tag(AddPaneKind.tab)
            }
            .pickerStyle(.segmented)
            .accessibilityIdentifier("pane-add-kind-picker")

            Text(kind == .pane
                 ? "Add a pane beside this one. Name is optional."
                 : "Add a tab with its own pane. Name is optional.")
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            VStack(alignment: .leading, spacing: 8) {
                Text(kind == .pane ? "Pane name" : "Tab name")
                    .font(.jost(.caption, weight: .semibold))
                    .foregroundStyle(.secondary)
                TextField("Optional", text: $name)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .background(Theme.fieldBackground, in: RoundedRectangle.continuous(DesignSystem.CornerRadius.md))
                    .accessibilityIdentifier("pane-add-name-field")
            }

            Spacer(minLength: 8)

            VStack(spacing: 10) {
                Button(kind == .pane ? "Add Pane" : "Add Tab") {
                    onAdd(AddPaneRequest(kind: kind, name: name))
                }
                .buttonStyle(.herdrPrimary())
                .accessibilityIdentifier("pane-add-confirm-button")

                Button("Cancel", action: onCancel)
                    .buttonStyle(.herdrGhost())
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 24)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Theme.backgroundGradient.ignoresSafeArea())
    }
}

/// One staged image bound to a specific pane identity.
private struct PendingPaneAttachment: Identifiable {
    let id: UUID
    let paneId: String
    let preview: UIImage
    let imageData: Data
    let fileExtension: String
    var remotePath: String?
    var uploadError: String?
    var isUploading: Bool

    func with(remotePath: String?, uploadError: String?, isUploading: Bool) -> PendingPaneAttachment {
        PendingPaneAttachment(
            id: id,
            paneId: paneId,
            preview: preview,
            imageData: imageData,
            fileExtension: fileExtension,
            remotePath: remotePath,
            uploadError: uploadError,
            isUploading: isUploading
        )
    }
}
