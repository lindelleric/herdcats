import SwiftUI
import UIKit
import ImageIO
import UniformTypeIdentifiers
import QuickLook

/// Native session content; the owning pane supplies its composition dock.
struct AgentTimelineView: View {
    let paneId: String
    var agent: String? = nil
    let connection: HerdrConnection
    let timeline: AgentTimelineState
    var bridgeUpgrade: TimelineBridgeUpgrade = TimelineBridgeUpgrade()
    let topOverlayInset: CGFloat
    let bottomOverlayInset: CGFloat
    let onOutputInteraction: () -> Void
    let setupTarget: ConnectionIdentity
    let onUseTerminal: () -> Void
    let onSetupComplete: () -> Void
    var onFocusComposer: () -> Void = {}
    @Environment(AppModel.self) private var appModel
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var waitingKey: WaitingTaskKey?
    @State private var phoneAnswersSelection: PhoneAnswersSelection?
    @State private var approvalResponse = ApprovalResponseState()
    @State private var waitingRowHeight: CGFloat = 0
    @State private var sheetState = TimelineSheetState()
    @State private var approvalViewVisible = false
    @State private var activeClient: AgentBridgeClient?
    @State private var activeOwner: UUID?
    @State private var showsSetup = false
    @State private var setupReason: HostHelperSetupReason = .missing
    @State private var setupShouldUseTerminal = true
    @State private var isCheckingSetup = false
    @State private var setupFeedback: String?
    @State private var helperStage: HostHelperSetupStage = .idle
    @State private var helperInstallTask: Task<Void, Never>?
    @State private var helperInstallToken: UUID?
    @State private var fixtureHelperInstalled = false
    @State private var helperMayHaveInstalled = false
    @State private var retry = 0
    @State private var contentFiles = TimelineContentFiles()
    @State private var historyLoader = TimelineHistoryLoader()
    @State private var isPaging = false
    @State private var pageError: String?
    @State private var following = true
    @State private var atBottom = true
    @State private var entryFrames: [String: CGRect] = [:]
    @State private var viewportHeight: CGFloat = 0
    @State private var historyScrollAnchor: TimelineHistoryScrollAnchor?
    @State private var bridgeCapabilities: AgentBridgeCapabilities?
    @State private var bridgeVersion: String?
    @State private var showsCodexDaemonConfirm = false
    @State private var isDisablingCodexDaemon = false
    @State private var codexDaemonStopOutcome: CodexDaemonStopOutcome?
    /// Episode token for the recorded outcome: bumped on every clear, so a
    /// stop command still in flight can detect that its episode ended.
    @State private var codexDaemonEpisode = 0
    /// Sequence of `codex_daemon` report changes; lets a completing stop tell
    /// an unchanged pre-stop report from one that changed while it ran.
    @State private var codexDaemonReportSeq = 0
    @State private var copiedCodexConfig = false
    @State private var copiedCodexStop = false
    @State private var copiedCodexUpdaterStop = false

    private var approvalInset: CGFloat { showsWaitingRow ? waitingRowHeight + 8 : 0 }
    private var hooksKey: PhoneAnswersStore.Key? {
        guard let generation = connectionState.generation,
            let agent = timeline.identity?.agent, ApprovalAgent(rawValue: agent) != nil else { return nil }
        return .init(target: setupTarget, generation: generation, agent: agent)
    }
    struct WaitingTaskKey: Equatable {
        let hooks: PhoneAnswersStore.Key?
        let owner: UUID?
        let identity: AgentTimelineIdentity?
        let generation: UInt64?
        let eligible: Bool
    }
    private var waitingTaskKey: WaitingTaskKey {
        WaitingTaskKey(hooks: hooksKey, owner: activeOwner, identity: timeline.identity, generation: timeline.generation,
            eligible: PhoneAnswersPresentation.eligible(approvals: bridgeCapabilities?.approvals,
                ready: timeline.phase == .ready && connectionState.isReady && appModel.supportsHostServices,
                blocked: timeline.agentStatus == "blocked", pending: timeline.pendingApproval != nil,
                active: scenePhase == .active && appModel.isSceneActive))
    }
    private var showsWaitingRow: Bool { waitingTaskKey.eligible && waitingKey == waitingTaskKey }
    private func hooksContextIsCurrent(_ key: PhoneAnswersStore.Key) -> Bool {
        scenePhase == .active && appModel.isSceneActive && connectionState.isReady && appModel.supportsHostServices
            && setupTarget == key.target && connectionState.generation == key.generation
            && bridgeCapabilities?.approvals == true
    }
    private var showsApproval: Bool {
        ApprovalResponseState.isVisible(pending: timeline.pendingApproval, approvals: bridgeCapabilities?.approvals)
    }

    private var conversationItems: [TimelineConversationItem] {
        TimelineConversationItem.group(timeline.entries, showsRecords: agent == "codex")
    }

    private var connectionState: AgentTimelineConnectionState {
        AgentTimelineConnectionState(
            phase: appModel.phase, generation: appModel.ownership.transportGeneration)
    }
    struct TaskKey: Equatable {
        let paneId: String
        let agent: String?
        let connection: AgentTimelineConnectionState
        let isActive: Bool
        let retry: Int
    }
    private var taskKey: TaskKey {
        TaskKey(paneId: paneId, agent: agent, connection: connectionState, isActive: scenePhase == .active, retry: retry)
    }
    private var canPinnedInstallerSupportAgent: Bool {
        Self.canPinnedInstallerSupport(agent: agent)
    }
    static func canPinnedInstallerSupport(agent: String?, supportedAgents: Set<String> = Set(HostHelperSetupPlan.agents)) -> Bool {
        agent.map { supportedAgents.contains($0) } ?? true
    }
    static func installLabel(
        stage: HostHelperSetupStage,
        reason: HostHelperSetupReason = .missing,
        pinnedVersion: String = HostHelperSetupPlan.version
    ) -> String {
        HostHelperSetupPlan.installLabel(stage: stage, reason: reason, pinnedVersion: pinnedVersion)
    }
    static func replacementFeedback(probe: HostHelperProbe) -> String? {
        HostHelperSetupPlan.replacementFeedback(probe: probe)
    }
    var body: some View {
        ScrollViewReader { proxy in
            GeometryReader { viewport in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        if timeline.phase == .ready {
                            if timeline.hasMore {
                                Text(isPaging ? "Loading earlier messages…" : (pageError ?? "Pull down to load earlier messages"))
                                    .font(.jost(.caption))
                                    .foregroundStyle(Theme.textMuted)
                                    .frame(maxWidth: .infinity)
                                    .accessibilityIdentifier("timeline-history-hint")
                                    .accessibilityAction(named: "Load earlier messages") {
                                        Task { await refreshHistory() }
                                    }
                            }
                            if conversationItems.isEmpty {
                                ContentUnavailableView("No messages yet", systemImage: "text.bubble")
                            }
                            ForEach(conversationItems) { item in
                                let scope = contentScope
                                Group {
                                    if item.isActivity {
                                        TimelineActivityView(entries: item.entries,
                                            loadContent: { reference in try await loadContent(reference, scope: scope) })
                                    } else if let entry = item.entries.first {
                                        AgentTimelineEntryView(entry: entry, files: contentFiles, scope: scope,
                                            loadContent: { reference in try await loadContent(reference, scope: scope) })
                                    }
                                }
                                .id("\(scope):\(item.id)")
                                .background {
                                    GeometryReader { row in
                                        Color.clear.preference(
                                            key: TimelineEntryFramesPreferenceKey.self,
                                            value: [item.id: row.frame(in: .named("timeline-viewport"))])
                                    }
                                }
                            }
                            if timeline.delayed {
                                Text("Source update delayed; showing verified history.")
                                    .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                            }
                            if showsApproval, let pending = timeline.pendingApproval {
                                ApprovalCard(pending: pending, response: approvalResponse) { option in
                                    respondToApproval(pending, option: option)
                                }
                                .id("\(contentScope):approval:\(pending.requestId)")
                                .disabled(approvalResponse.isDisabled)
                            }
                        } else {
                            availability
                        }
                        // Clear scroll space lets the final entry sit above the
                        // floating Compose bubble without an opaque dock panel.
                        Color.clear
                            .frame(height: bottomOverlayInset + approvalInset)
                            .padding(.bottom, DesignSystem.Spacing.base)
                            .id("timeline-bottom")
                    }
                    .scrollTargetLayout()
                    .padding([.horizontal, .top], DesignSystem.Spacing.base)
                    .padding(.top, topOverlayInset)
                    .frame(minHeight: viewport.size.height + 1, alignment: .top)
                    .background {
                        GeometryReader { content in
                            Color.clear.preference(key: TimelineAtBottomPreferenceKey.self,
                                value: content.frame(in: .named("timeline-viewport")).maxY <= viewport.size.height + 8)
                        }
                    }
                }
                .coordinateSpace(name: "timeline-viewport")
                .onChange(of: viewport.size.height, initial: true) { _, height in
                    viewportHeight = height
                    if following && showsApproval { proxy.scrollTo("timeline-bottom", anchor: .bottom) }
                }
                .onChange(of: timeline.pendingApproval?.requestId, initial: true) { _, requestId in
                    guard requestId != nil, showsApproval else { return }
                    following = true
                    UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
                    withAnimation(reduceMotion ? nil : .smooth(duration: 0.4)) {
                        proxy.scrollTo("timeline-bottom", anchor: .bottom)
                    }
                }
                .onPreferenceChange(TimelineEntryFramesPreferenceKey.self) { entryFrames = $0 }
                .onChange(of: historyScrollAnchor) { _, anchor in
                    guard let anchor else { return }
                    following = false
                    var transaction = Transaction(animation: nil)
                    transaction.disablesAnimations = true
                    withTransaction(transaction) {
                        proxy.scrollTo("\(contentScope):\(anchor.id)", anchor: UnitPoint(x: 0, y: anchor.unitY))
                    }
                }
                .onPreferenceChange(TimelineAtBottomPreferenceKey.self) { bottom in
                    atBottom = bottom
                    if bottom { following = true }
                }
                .accessibilityIdentifier("agent-timeline-scroll")
                .scrollDismissesKeyboard(.interactively)
                .simultaneousGesture(
                    DragGesture()
                        .onChanged { value in
                            guard abs(value.translation.height) > abs(value.translation.width) else { return }
                            following = false
                            onOutputInteraction()
                        }
                        .onEnded { value in
                            guard abs(value.translation.height) > abs(value.translation.width) else { return }
                            let firstFrame = conversationItems.first.flatMap { entryFrames[$0.id] }
                            let atHistoryTop = conversationItems.isEmpty || (firstFrame.map { $0.minY >= topOverlayInset - 8 } ?? false)
                            let loadHistory = atHistoryTop && value.translation.height > 60
                            if loadHistory && timeline.hasMore {
                                Task { await refreshHistory() }
                            } else if atBottom { following = true }
                        }
                )
                .onChange(of: timeline.revision) { _, _ in
                    guard following, (!conversationItems.isEmpty || showsApproval) else { return }
                    proxy.scrollTo("timeline-bottom", anchor: .bottom)
                }
                .onChange(of: timeline.identity) { _, _ in
                    following = true
                    #if DEBUG && targetEnvironment(simulator)
                        if ScreenshotFixtures.enabled && ScreenshotFixtures.screen == "timeline-history-position" {
                            following = false
                        }
                    #endif
                }
                .onChange(of: bottomOverlayInset) { _, _ in
                    if following && (!conversationItems.isEmpty || showsApproval) {
                        proxy.scrollTo("timeline-bottom", anchor: .bottom)
                    }
                }
                .onChange(of: approvalInset) { _, _ in
                    if following && (!conversationItems.isEmpty || showsApproval) {
                        proxy.scrollTo("timeline-bottom", anchor: .bottom)
                    }
                }
                .overlay(alignment: .bottomTrailing) {
                    Group {
                    if !atBottom && timeline.phase == .ready {
                        Button {
                            following = true
                            withAnimation(reduceMotion ? nil : .smooth(duration: 0.4)) {
                                proxy.scrollTo("timeline-bottom", anchor: .bottom)
                            }
                        } label: {
                            latestIcon
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Jump to latest messages")
                        .accessibilityIdentifier("timeline-latest")
                        .padding(.trailing, 16)
                        .padding(.bottom, bottomOverlayInset + approvalInset + 8)
                        .transition(.opacity)
                    }
                    }
                    .animation(reduceMotion ? nil : .easeOut(duration: 0.2), value: !atBottom && timeline.phase == .ready)
                }
            }
        }
        // Let messages scroll behind the pane cards, but keep them below
        // the navigation bar and status area.
        .overlay(alignment: .bottom) {
            Group {
                if showsWaitingRow, let key = hooksKey, let agent = ApprovalAgent(rawValue: key.agent) {
                    PhoneAnswersWaitingRow(agent: agent, result: appModel.phoneAnswers.result(for: key)) { mode in
                        phoneAnswersSelection = PhoneAnswersSelection(key: key, mode: mode)
                    }
                }
            }
            .background {
                GeometryReader { geometry in
                    Color.clear.onChange(of: geometry.size.height, initial: true) { _, height in waitingRowHeight = height }
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, bottomOverlayInset + 8)
        }
        .clipped()
        .background(Theme.background)
        .onChange(of: timeline.pendingApproval, initial: true) { _, pending in approvalResponse.sync(pending) }
        .task(id: taskKey) { await run() }
        .task(id: waitingTaskKey) { await prepareWaitingRow() }
        .onAppear { approvalViewVisible = true; updateTimelineSheet() }
        .onChange(of: bridgeCapabilities?.approvals) { _, _ in updateTimelineSheet() }
        .onChange(of: phoneAnswersSelection?.id) { _, _ in updateTimelineSheet() }
        .onChange(of: showsSetup) { _, _ in updateTimelineSheet() }
        .onChange(of: showsCodexDaemonConfirm) { _, _ in updateTimelineSheet() }
        .onChange(of: timeline.revision, initial: true) { _, _ in
            switch timeline.phase {
            case .ready:
                onSetupComplete()
                isCheckingSetup = false
                if showsSetup {
                    setupShouldUseTerminal = false
                    showsSetup = false
                }
            case .unavailable(let code):
                if isCheckingSetup {
                    isCheckingSetup = false
                    setupFeedback = code == "helper_missing"
                        ? "Herdcats Bridge was not found yet. Run the command on your SSH host, then check again."
                        : explanation(code)
                }
                let isNeedsUpgrade = Self.isNeedsUpgrade(code: code, agent: agent, bridgeVersion: bridgeVersion)
                let isMissing = code == "helper_missing"
                if (isMissing || isNeedsUpgrade) && !showsSetup && canPinnedInstallerSupportAgent {
                    #if DEBUG && targetEnvironment(simulator)
                    if ScreenshotFixtures.enabled && ProcessInfo.processInfo.arguments.contains("-hc.timeline-suppress-drawer") {
                        // Suppress drawer auto-presentation for pane screenshots
                    } else {
                        beginSetup(reason: isNeedsUpgrade ? .updateRequired : .missing)
                    }
                    #else
                    beginSetup(reason: isNeedsUpgrade ? .updateRequired : .missing)
                    #endif
                }
            case .disconnected:
                if isCheckingSetup {
                    isCheckingSetup = false
                    setupFeedback = "Reconnect to your SSH host, then check again."
                }
            case .loading: break
            }
        }
        .onChange(of: connectionState) { _, _ in cancelHelperInstallation() }
        .onChange(of: scenePhase) { _, phase in if phase != .active { cancelHelperInstallation() } }
        .onChange(of: timeline.phase) { _, newPhase in
            // A stop result only describes the no-session episode it answered;
            // a later episode (after the pane was ready again) starts fresh.
            if !Self.codexDaemonResultIsCurrent(phase: newPhase) { clearCodexDaemonResult() }
            dismissStaleCodexDaemonConfirm()
        }
        .onChange(of: timeline.codexDaemon) { _, codexDaemon in
            codexDaemonReportSeq += 1
            // A fresh daemon report that contradicts the recorded result (the
            // server came back after "off", or was stopped despite "still
            // running") hands the pane back to the live state.
            if Self.codexDaemonResultIsStale(outcome: codexDaemonStopOutcome, codexDaemon: codexDaemon) {
                clearCodexDaemonResult()
            }
            dismissStaleCodexDaemonConfirm()
        }
        .onChange(of: bridgeUpgrade.requested) { _, requested in
            if requested {
                bridgeUpgrade.requested = false
                // The menu item is offered while Timeline is working; dismissing
                // the optional upgrade must not dump a working Timeline.
                beginSetup(reason: .updateRequired, shouldUseTerminalOnDismiss: timeline.phase != .ready)
            }
        }
        .onChange(of: contentScope) { _, _ in
            historyLoader.cancel()
            entryFrames = [:]
            historyScrollAnchor = nil
            contentFiles.reset()
        }
        .onChange(of: taskKey) { _, _ in historyLoader.cancel(); updateTimelineSheet() }
        .onDisappear {
            approvalViewVisible = false
            approvalResponse.reset()
            updateTimelineSheet()
            historyLoader.cancel(); cancelHelperInstallation(); contentFiles.reset()
        }
        .sheet(item: timelineSheetBinding, onDismiss: timelineSheetDismissed) { sheet in timelineSheet(sheet) }
    }
    private var timelineSheetBinding: Binding<TimelineSheetState.Sheet?> {
        Binding(get: { sheetState.presented }, set: { value in
            guard value == nil else { return }
            switch sheetState.presented {
            case .phoneAnswers: phoneAnswersSelection = nil
            case .helper: showsSetup = false
            case .codexDaemon: showsCodexDaemonConfirm = false
            default: break
            }
            sheetState.close()
        })
    }
    @MainActor private func timelineSheetDismissed() {
        let dismissed = sheetState.dismissing
        sheetState.didDismiss()
        if dismissed == .helper {
            cancelHelperInstallation()
            if setupShouldUseTerminal { onUseTerminal() }
        }
        updateTimelineSheet()
    }
    @ViewBuilder private func timelineSheet(_ sheet: TimelineSheetState.Sheet) -> some View {
        switch sheet {
        case .phoneAnswers:
            if let selection = phoneAnswersSelection {
                PhoneAnswersSheet(selection: selection, connection: connection, store: appModel.phoneAnswers,
                    isCurrent: { hooksContextIsCurrent(selection.key) })
            }
        case .helper:
            AgentBridgeSetupView(
                reason: setupReason, agent: agent, installedVersion: bridgeVersion,
                isChecking: isCheckingSetup, feedback: setupFeedback,
                stage: helperStage, target: setupTarget,
                canInstall: connectionState.isReady && appModel.supportsHostServices && appModel.connectionIdentity == setupTarget && canPinnedInstallerSupportAgent,
                onInstall: startHelperInstallation,
                onCheckAgain: { setupFeedback = nil; isCheckingSetup = true; retry += 1 },
                onUseTerminal: { cancelHelperInstallation(); showsSetup = false }
            )
            .presentationDetents([.large])
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(28)
        case .codexDaemon:
            CodexDaemonConfirmSheet(
                onConfirm: { showsCodexDaemonConfirm = false; disableCodexDaemon() },
                onCancel: { showsCodexDaemonConfirm = false }
            )
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(28)
        }
    }
    @MainActor private func updateTimelineSheet() {
        sheetState.sync(visible: approvalViewVisible && scenePhase == .active,
            phone: phoneAnswersSelection != nil, helper: showsSetup, codex: showsCodexDaemonConfirm)
    }
    private var contentScope: String {
        "\(activeOwner?.uuidString ?? "none"):\(timeline.generation ?? 0):\(timeline.identity?.nativeSessionId ?? "")"
    }
    @MainActor private func loadContent(_ reference: String, scope: String) async throws -> Data {
        guard scope == contentScope, timeline.phase == .ready else { throw AgentBridgeError.closed }
        let result = try await contentFiles.data(key: "\(scope):\(reference)") {
            #if DEBUG && targetEnvironment(simulator)
            if ScreenshotFixtures.enabled { return try AgentTimelineScreenshotFixture.content(reference: reference) }
            #endif
            guard let client = activeClient, let identity = timeline.identity, let generation = timeline.generation else { throw AgentBridgeError.closed }
            return try await client.content(reference: reference, generation: generation, identity: identity)
        }
        try Task.checkCancellation()
        guard scope == contentScope, timeline.phase == .ready else { throw AgentBridgeError.closed }
        return result
    }

    @ViewBuilder private var latestIcon: some View {
        let icon = Image(systemName: "arrow.down")
            .font(.system(size: 18, weight: .semibold))
            .foregroundStyle(Theme.text)
            .frame(width: 44, height: 44)
            .contentShape(Circle())
        if reduceTransparency {
            icon.background(Theme.cardBackground, in: Circle())
        } else if #available(iOS 26.0, *) {
            icon.glassEffect(.regular.interactive(), in: Circle())
        } else {
            icon.background(.ultraThinMaterial, in: Circle())
        }
    }

    private func beginSetup(reason: HostHelperSetupReason? = nil, shouldUseTerminalOnDismiss: Bool = true) {
        let isNeedsUpgrade: Bool
        if case .unavailable(let code) = timeline.phase {
            isNeedsUpgrade = Self.isNeedsUpgrade(code: code, agent: agent, bridgeVersion: bridgeVersion)
        } else {
            isNeedsUpgrade = false
        }
        let resolvedReason = reason ?? (isNeedsUpgrade ? .updateRequired : .missing)
        setupReason = resolvedReason
        setupShouldUseTerminal = shouldUseTerminalOnDismiss
        setupFeedback = nil
        showsSetup = true
    }
    @MainActor private func startHelperInstallation() {
        guard helperInstallTask == nil, !helperStage.isBusy, !isCheckingSetup else { return }
        if helperStage == .installed { isCheckingSetup = true; retry += 1; return }
        let checkOnly = helperStage == .uncertain
        let replacing: Bool
        if case .replacementRequired = helperStage { replacing = true } else { replacing = false }
        let token = UUID()
        helperInstallToken = token
        let ownedConnection = connectionState
        setupFeedback = nil
        helperStage = .checking
        helperMayHaveInstalled = checkOnly
        helperInstallTask = Task { @MainActor in
            do {
                let result: HostHelperSetupDecision
                #if DEBUG && targetEnvironment(simulator)
                if ScreenshotFixtures.enabled {
                    try await Task.sleep(for: .milliseconds(150))
                    let args = ProcessInfo.processInfo.arguments
                    let scenario = args.firstIndex(of: "-hc.timeline-install-result").flatMap { index in
                        index + 1 < args.count ? args[index + 1] : nil
                    } ?? "failure"
                    if scenario == "replacement" && !replacing {
                        if setupReason == .updateRequired {
                            helperStage = .installing
                            helperMayHaveInstalled = true
                            try await Task.sleep(for: .milliseconds(150))
                            fixtureHelperInstalled = true
                            result = .ready
                        } else {
                            result = .replacementRequired(.present(version: "0.1.0", managed: true, compatible: false))
                        }
                    } else if scenario == "external_replacement" && !replacing {
                        result = .replacementRequired(.present(version: "0.1.0", managed: false, compatible: false))
                    } else if scenario == "uncertain" && !checkOnly {
                        helperStage = .installing
                        helperMayHaveInstalled = true
                        throw AgentBridgeError.closed
                    } else if scenario == "probe_failure" {
                        throw AgentBridgeError.closed
                    } else if scenario == "failure" {
                        throw HostHelperSetupError.failed("download_failed")
                    } else {
                        if !checkOnly { helperStage = .installing; helperMayHaveInstalled = true }
                        try await Task.sleep(for: .milliseconds(150))
                        fixtureHelperInstalled = true
                        result = .ready
                    }
                } else {
                    guard let owned = ownedConnection.generation else { throw AgentBridgeError.closed }
                    result = try await connection.setUpHostHelper(
                        replace: replacing, checkOnly: checkOnly, target: setupTarget, owned: owned, agent: agent,
                        reason: setupReason,
                        onStage: { stage in await commitHelperStage(stage, token: token, connection: ownedConnection) })
                }
                #else
                guard let owned = ownedConnection.generation else { throw AgentBridgeError.closed }
                result = try await connection.setUpHostHelper(
                    replace: replacing, checkOnly: checkOnly, target: setupTarget, owned: owned, agent: agent,
                    reason: setupReason,
                    onStage: { stage in await commitHelperStage(stage, token: token, connection: ownedConnection) })
                #endif
                try Task.checkCancellation()
                guard helperInstallToken == token, ownedConnection == connectionState, showsSetup else { return }
                helperInstallTask = nil
                switch result {
                case .ready:
                    helperStage = .installed
                    isCheckingSetup = true
                    retry += 1
                case .missing:
                    helperStage = .idle
                    setupReason = .missing
                    setupFeedback = "No Herdcats Bridge was found. Tap Install Herdcats Bridge to try installation, or use manual setup."
                case .replacementRequired(let probe):
                    helperStage = .replacementRequired(probe)
                    setupFeedback = HostHelperSetupPlan.replacementFeedback(probe: probe)
                case .unsupportedByPinnedInstaller(let agent):
                    helperStage = .idle
                    setupFeedback = "This version of Herdcats can't install a version of Herdcats Bridge that supports \(agent.capitalized) yet."
                case .install: throw HostHelperSetupError.invalidResponse
                }
            } catch {
                guard helperInstallToken == token, ownedConnection == connectionState, showsSetup else { return }
                helperInstallTask = nil
                if let setupError = error as? HostHelperSetupError {
                    if helperMayHaveInstalled && (setupError == .invalidResponse || setupError == .failed("install_failed")) {
                        helperStage = .uncertain
                        setupFeedback = "Installation could not be verified. Check installation before trying again, or use manual setup."
                    } else {
                        helperStage = .idle
                        helperMayHaveInstalled = false
                        setupFeedback = setupError.localizedDescription
                    }
                } else if helperMayHaveInstalled {
                    helperStage = .uncertain
                    setupFeedback = "Installation could not be confirmed. It may have finished on the host. Check installation before trying again, or use manual setup."
                } else {
                    helperStage = .idle
                    setupFeedback = "The host could not be checked. Reconnect and try again, or use manual setup. No installation was started."
                }
            }
        }
    }
    @MainActor private func commitHelperStage(_ stage: HostHelperSetupStage, token: UUID, connection: AgentTimelineConnectionState) {
        guard helperInstallToken == token, connection == connectionState, showsSetup else { return }
        helperStage = stage
        if stage == .installing { helperMayHaveInstalled = true }
    }
    @MainActor private func cancelHelperInstallation() {
        guard let task = helperInstallTask else { return }
        helperInstallToken = nil
        helperInstallTask = nil
        task.cancel()
        helperStage = helperMayHaveInstalled ? .uncertain : .idle
        setupFeedback = helperMayHaveInstalled
            ? "Installation may have finished on the host. Check installation before trying again."
            : "The host check stopped. Reconnect and try again. No installation was started."
    }
    @ViewBuilder private var availability: some View {
        switch timeline.phase {
        case .loading:
            HStack {
                ProgressView()
                Text("Connecting to timeline…")
            }.frame(maxWidth: .infinity).padding(.vertical, 32)
        case .disconnected:
            ContentUnavailableView {
                Label("Timeline disconnected", systemImage: "wifi.slash")
            } description: {
                if !connectionState.isReady { Text("Reconnect to load current session messages.") }
            } actions: {
                Button("Check again") { retry += 1 }
                    .buttonStyle(.herdrSecondary)
                    .disabled(!connectionState.isReady)
                    .accessibilityIdentifier("timeline-helper-retry")
            }
        case .unavailable("transcript_unavailable"):
            // Claude writes its transcript only after the first prompt; the
            // helper keeps polling and swaps in the timeline once it exists.
            ContentUnavailableView("Send a message to start", systemImage: "text.bubble")
        case .unavailable(let code):
            let isCodex = agent == "codex"
            let isSupportedAgent = canPinnedInstallerSupportAgent
            let isNeedsUpgrade = Self.isNeedsUpgrade(code: code, agent: agent, bridgeVersion: bridgeVersion)
            let isMissing = code == "helper_missing"
            let showsInstallOrUpgrade = Self.showsInstallOrUpgrade(code: code, agent: agent, bridgeVersion: bridgeVersion)
            let showsProtocolMismatch = code == "protocol_mismatch" && isSupportedAgent
            let isNoSession = ["session_reference_unavailable", "ambiguous_session", "invalid_session_reference"].contains(code)
            let isCwdMismatch = code == "session_cwd_mismatch"
            let codexNoSession: CodexNoSessionAction? = isCodex && isNoSession
                ? Self.codexNoSessionAction(
                    agent: agent,
                    code: code,
                    route: connection.route,
                    capabilities: bridgeCapabilities,
                    codexDaemon: timeline.codexDaemon)
                : nil

            ContentUnavailableView {
                Label(codexNoSession.map(Self.codexNoSessionTitle) ?? title(code), systemImage: "text.bubble")
            } description: {
                if codexNoSession != nil, let outcome = codexDaemonStopOutcome {
                    Text(outcome.message)
                } else if let codexNoSession {
                    if let explanation = Self.codexNoSessionExplanation(codexNoSession, code: code, agent: agent) {
                        Text(explanation)
                    }
                } else {
                    Text(explanation(code))
                }
            } actions: {
                if showsInstallOrUpgrade || showsProtocolMismatch {
                    Button(isNeedsUpgrade ? "Upgrade Herdcats Bridge" : (isMissing ? "Install Herdcats Bridge" : "Set up Timeline")) {
                        beginSetup(reason: isNeedsUpgrade ? .updateRequired : .missing)
                    }
                    .buttonStyle(.herdrPrimary)
                    .accessibilityIdentifier("timeline-helper-setup")
                }
                if let codexNoSession {
                    ForEach(
                        Self.codexNoSessionRecovery(outcome: codexDaemonStopOutcome, action: codexNoSession),
                        id: \.self
                    ) { recovery in
                        switch recovery {
                        case .turnOffServer:
                            Button {
                                showsCodexDaemonConfirm = true
                            } label: {
                                HStack {
                                    if isDisablingCodexDaemon { ProgressView() }
                                    Text(isDisablingCodexDaemon ? "Turning off…" : "Turn off Codex's shared background server")
                                }.frame(maxWidth: .infinity)
                            }
                            .buttonStyle(.herdrPrimary)
                            .accessibilityIdentifier("timeline-codex-disable-daemon")
                            .disabled(isDisablingCodexDaemon)
                        case .upgradeBridge:
                            Button("Upgrade Herdcats Bridge") {
                                beginSetup(reason: .updateRequired)
                            }
                            .buttonStyle(.herdrPrimary)
                            .accessibilityIdentifier("timeline-codex-upgrade-bridge")
                        case .manualFallback:
                            codexManualFallback
                        case .stopCommand:
                            codexStopCommandFallback
                        case .updaterStopCommand:
                            codexUpdaterStopFallback
                        }
                    }
                }
                // The bridge re-resolves every poll, so a session that starts
                // replaces this state on its own; retrying adds nothing.
                if codexNoSession != .waitForSession,
                   Self.showsPaneCheckAgain(code: code, agent: agent, bridgeVersion: bridgeVersion) {
                    Button("Check again") { retry += 1 }.buttonStyle(.herdrSecondary)
                        .accessibilityIdentifier("timeline-helper-retry")
                }
            }
        case .ready: EmptyView()
        }
    }
    func explanation(_ code: String) -> String {
        Self.explanation(
            code,
            agent: agent,
            installedVersion: bridgeVersion,
            pinnedVersion: HostHelperSetupPlan.version,
            supportedAgents: Set(HostHelperSetupPlan.agents),
            gatewayHost: setupTarget.host,
            machineLabel: connection.route?.machine.label
        )
    }

    func title(_ code: String) -> String {
        Self.unavailableTitle(
            code: code,
            agent: agent,
            bridgeVersion: bridgeVersion,
            supportedAgents: Set(HostHelperSetupPlan.agents)
        )
    }

    static func isNeedsUpgrade(
        code: String,
        agent: String?,
        bridgeVersion: String? = nil,
        supportedAgents: Set<String> = Set(HostHelperSetupPlan.agents)
    ) -> Bool {
        let isSupported = canPinnedInstallerSupport(agent: agent, supportedAgents: supportedAgents)
        guard isSupported else { return false }
        let isCodex = agent == "codex"
        return code == "helper_update_required" || (HostHelperProbe.isOlderThanMinimum(bridgeVersion) && (code == "record_too_large" || (isCodex && code == "unsupported_agent")))
    }

    static func upgradePitch(
        agent: String?,
        installedVersion: String?,
        pinnedVersion: String = HostHelperSetupPlan.version
    ) -> String {
        let isCodex = agent == "codex"
        let installedBelowMinimum = !HostHelperProbe.isAtLeastMinimum(installedVersion)
        if installedBelowMinimum {
            let feature = isCodex ? "Codex conversations" : "session images"
            return "Version \(pinnedVersion) adds \(feature). Your agent keeps running while you upgrade."
        } else {
            let feature = isCodex ? "fixes for Codex conversations" : "fixes for session history"
            return "Version \(pinnedVersion) includes \(feature). Your agent keeps running while you upgrade."
        }
    }

    static func showsInstallOrUpgrade(
        code: String,
        agent: String?,
        bridgeVersion: String? = nil,
        supportedAgents: Set<String> = Set(HostHelperSetupPlan.agents)
    ) -> Bool {
        let isSupported = canPinnedInstallerSupport(agent: agent, supportedAgents: supportedAgents)
        guard isSupported else { return false }
        return code == "helper_missing" || isNeedsUpgrade(code: code, agent: agent, bridgeVersion: bridgeVersion, supportedAgents: supportedAgents)
    }

    static func showsPaneCheckAgain(
        code: String,
        agent: String?,
        bridgeVersion: String? = nil,
        supportedAgents: Set<String> = Set(HostHelperSetupPlan.agents)
    ) -> Bool {
        // Retrying can never help a routed host: the pane is reached through
        // Herdr on the SSH-connected gateway, not a computer Herdcats dialed.
        guard code != "routed_host_unsupported" else { return false }
        return !showsInstallOrUpgrade(code: code, agent: agent, bridgeVersion: bridgeVersion, supportedAgents: supportedAgents)
    }

    /// What the pane can do about a Codex pane with no identified session,
    /// decided from what the installed bridge can actually do.
    enum CodexNoSessionAction: Equatable, CaseIterable {
        /// The bridge (0.2.2+) can stop the server and it is running or unverifiable.
        case turnOffServer
        /// The server is off and there is no session reference yet.
        case waitForSession
        /// The server is off but the existing reference cannot identify a session.
        case identifySession
        /// The bridge (0.2.0/0.2.1) can only change the config; upgrading adds
        /// the in-app stop.
        case upgradeBridge
        /// Older bridge, routed host, or nothing to do: keep the manual fallback.
        case manualFallback
    }

    static func codexNoSessionAction(
        agent: String?,
        code: String,
        route: HerdrMachineRoute?,
        capabilities: AgentBridgeCapabilities?,
        codexDaemon: String?
    ) -> CodexNoSessionAction {
        guard agent == "codex",
            ["session_reference_unavailable", "ambiguous_session", "invalid_session_reference"].contains(code),
            route == nil
        else { return .manualFallback }
        if capabilities?.codexDaemonStop == true {
            if codexDaemon == "running" || codexDaemon == "unknown" { return .turnOffServer }
            return code == "session_reference_unavailable" ? .waitForSession : .identifySession
        }
        if capabilities?.codexDaemonConfig == true {
            return .upgradeBridge
        }
        return .manualFallback
    }

    static let codexDaemonRunningExplanation =
        "Codex's shared background server is running, so Herdcats can't tell which session belongs to this pane."

    /// The title says what the user has to do: "Waiting" only fits when the
    /// session will arrive on its own, which it never does while the shared
    /// server is running.
    static func codexNoSessionTitle(_ action: CodexNoSessionAction) -> String {
        switch action {
        case .turnOffServer, .upgradeBridge, .identifySession: return "Codex session not identified"
        case .waitForSession: return "Send a message to start"
        case .manualFallback: return "Waiting for Codex session"
        }
    }
    static let codexDaemonUpgradeExplanation =
        "Upgrade Herdcats Bridge to turn off Codex's shared background server from here."

    /// Only a missing session reference needs no explanation beyond the title.
    static func codexNoSessionExplanation(_ action: CodexNoSessionAction, code: String, agent: String?) -> String? {
        switch action {
        case .turnOffServer: return codexDaemonRunningExplanation
        case .waitForSession: return nil
        case .identifySession: return "Restart Codex in this pane, then check again."
        case .upgradeBridge: return codexDaemonUpgradeExplanation
        case .manualFallback: return explanation(code, agent: agent)
        }
    }

    /// One recovery control offered under a no-session Codex state.
    enum CodexRecoveryAction: Hashable {
        case turnOffServer, upgradeBridge, manualFallback, stopCommand, updaterStopCommand
    }

    /// Which recovery actions the pane offers. A recorded stop outcome drives
    /// its own recovery regardless of the live daemon action — after an
    /// updater-only failure the not_running re-emit moves the pane to waiting
    /// while the updater may still restart the server, so the command the
    /// message points at must stay reachable — and an outcome error replaces,
    /// never duplicates, the action's own fallbacks. Without an outcome the
    /// live action decides.
    static func codexNoSessionRecovery(
        outcome: CodexDaemonStopOutcome?, action: CodexNoSessionAction
    ) -> [CodexRecoveryAction] {
        if let outcome {
            if outcome.errorOccurred { return [.manualFallback] }
            if outcome.showsStopCommand { return [.stopCommand] }
            if outcome.showsUpdaterStopCommand { return [.updaterStopCommand] }
            return []
        }
        switch action {
        case .turnOffServer: return [.turnOffServer]
        case .upgradeBridge: return [.upgradeBridge, .manualFallback]
        case .manualFallback: return [.manualFallback]
        case .waitForSession, .identifySession: return []
        }
    }

    /// The stop is destructive, so it may run only while the live pane still
    /// asks for it: a no-session Codex pane whose bridge can stop the server
    /// and currently reports it as running or unverifiable. Anything else —
    /// the session was identified, the pane reconnected, the server is
    /// already off, or an older bridge/routed host — must not stop anything.
    static func canRunCodexDaemonStop(
        phase: AgentTimelineState.Phase,
        agent: String?,
        route: HerdrMachineRoute?,
        capabilities: AgentBridgeCapabilities?,
        codexDaemon: String?
    ) -> Bool {
        guard case .unavailable(let code) = phase else { return false }
        return codexNoSessionAction(
            agent: agent, code: code, route: route, capabilities: capabilities, codexDaemon: codexDaemon
        ) == .turnOffServer
    }

    /// The recorded stop result for the current no-session episode. Keeps the
    /// per-process outcome: `codex_daemon` events report only the daemon
    /// process, so an updater-only failure is not contradicted by them.
    struct CodexDaemonStopOutcome: Equatable {
        let message: String
        let errorOccurred: Bool
        let showsStopCommand: Bool
        /// The daemon stopped but its updater survived or is unverified;
        /// recovery needs the updater-aware bridge command, not Codex's own
        /// `daemon stop`.
        let showsUpdaterStopCommand: Bool
        /// The result reported the daemon process itself as still running
        /// (stop_failed) or alive-but-unverifiable (unknown).
        let daemonStillRunning: Bool

        init(result: CodexDaemonConfigResult) {
            self.message = CodexDaemonConfigResult.message(for: result)
            self.errorOccurred = result.error != nil
            self.showsStopCommand = CodexDaemonConfigResult.showsStopCommand(for: result)
            self.showsUpdaterStopCommand = CodexDaemonConfigResult.showsUpdaterStopCommand(for: result)
            self.daemonStillRunning = result.daemon == "stop_failed" || result.daemon == "unknown"
        }

        /// The stop command failed before any result came back.
        init(errorDescription: String) {
            self.message = errorDescription
            self.errorOccurred = true
            self.showsStopCommand = false
            self.showsUpdaterStopCommand = false
            self.daemonStillRunning = false
        }
    }

    /// A stop result survives only within the unavailable episode it answered;
    /// once the pane is ready (or reconnecting) again, a later no-session
    /// episode must not inherit its feedback.
    static func codexDaemonResultIsCurrent(phase: AgentTimelineState.Phase) -> Bool {
        if case .unavailable = phase { return true }
        return false
    }

    /// Whether a fresh `codex_daemon` report retires a recorded stop result.
    /// The event reports only the daemon process:
    /// - Both processes failed: no daemon-only report can contradict the
    ///   recorded updater failure, so only the episode end retires it.
    /// - Only the updater failed (the daemon was stopped): the daemon being
    ///   alive again (running or unverifiable) contradicts it.
    /// - Only the daemon failed, or nothing was stopped (an error): a
    ///   not_running report resolves it.
    /// - A successful stop: the daemon being alive again retires it.
    static func codexDaemonResultIsStale(outcome: CodexDaemonStopOutcome?, codexDaemon: String?) -> Bool {
        guard let outcome, let codexDaemon else { return false }
        if outcome.showsUpdaterStopCommand {
            if outcome.daemonStillRunning { return false }
            return codexDaemon == "running" || codexDaemon == "unknown"
        }
        if outcome.errorOccurred || outcome.daemonStillRunning {
            return codexDaemon == "not_running"
        }
        return codexDaemon == "running" || codexDaemon == "unknown"
    }

    /// A stop command can run for up to 90 s, and its result belongs to the
    /// no-session episode that initiated it. The episode token is captured
    /// before awaiting and bumped by every clear, so a completion that lands
    /// after the pane moved on (ready, a re-run, a later episode) is
    /// discarded instead of replacing the live guidance. A daemon report
    /// that never changed during the flight carries no new information —
    /// the completion's own result is the freshest evidence — so the
    /// staleness comparison applies only to a report that changed after the
    /// stop started; an updater failure recorded then is not retired by a
    /// daemon-only value the bridge never re-emitted.
    static func shouldRecordStopOutcome(
        initiatedEpisode: Int,
        currentEpisode: Int,
        initiatedReportSeq: Int,
        currentReportSeq: Int,
        outcome: CodexDaemonStopOutcome,
        latestCodexDaemon: String?
    ) -> Bool {
        guard initiatedEpisode == currentEpisode else { return false }
        guard initiatedReportSeq != currentReportSeq else { return true }
        return !codexDaemonResultIsStale(outcome: outcome, codexDaemon: latestCodexDaemon)
    }

    static let codexManualConfigLine = "daemon_auto_start = false"
    static let codexDaemonStopCommand = "codex app-server daemon stop"
    /// Recovery for a surviving or unverified updater: Codex's own
    /// `daemon stop` cannot address it, and only the bridge stops the
    /// updater before the daemon. The lookup mirrors the app's helper
    /// discovery exactly — the managed ~/.local/bin path first, then PATH,
    /// then the remaining known locations — so it runs the same bridge
    /// binary that advertised codex_daemon_stop instead of an older copy
    /// left on PATH.
    static let codexUpdaterStopCommand = "B=\"$HOME/.local/bin/herdcats-bridge\"; [ -x \"$B\" ] || B=$(command -v herdcats-bridge 2>/dev/null); [ -n \"$B\" ] || for P in \"$HOME/.cargo/bin/herdcats-bridge\" /opt/homebrew/bin/herdcats-bridge /usr/local/bin/herdcats-bridge; do [ -x \"$P\" ] && { B=\"$P\"; break; }; done; \"$B\" codex-config disable-daemon --stop-running"

    /// The pane options menu's optional upgrade item reads `bridgeUpgrade`,
    /// which outlives this view's runs. Every refresh of the handshake-derived
    /// Bridge version must reach it, but only while `owner` still owns the run
    /// — a superseded run must not clobber a newer handshake's version.
    @MainActor
    static func publishBridgeVersion(
        _ version: String?, to upgrade: TimelineBridgeUpgrade, owner: UUID, activeOwner: UUID?
    ) {
        guard activeOwner == owner else { return }
        upgrade.bridgeVersion = version
    }

    static func codexManualFallbackText() -> String {
        "Add `daemon_auto_start = false` under `[features]` in `~/.codex/config.toml`, or start Codex with `codex --no-daemon`."
    }

    static func unavailableTitle(
        code: String,
        agent: String?,
        bridgeVersion: String? = nil,
        supportedAgents: Set<String> = Set(HostHelperSetupPlan.agents)
    ) -> String {
        let isNeedsUpgrade = isNeedsUpgrade(code: code, agent: agent, bridgeVersion: bridgeVersion, supportedAgents: supportedAgents)
        let isCodex = agent == "codex"
        let isNoSession = ["session_reference_unavailable", "ambiguous_session", "invalid_session_reference"].contains(code)
        let isCwdMismatch = code == "session_cwd_mismatch"
        let isMissing = code == "helper_missing"
        if isNeedsUpgrade { return "Upgrade Herdcats Bridge" }
        if isCodex && isNoSession { return "Waiting for Codex session" }
        if isCodex && isCwdMismatch { return "This pane’s Codex session could not be confirmed" }
        if isMissing { return "Set up Timeline" }
        if code == "routed_host_unsupported" { return routedHostUnavailableTitle }
        return "Timeline unavailable"
    }

    /// A saved machine is reached through the Herdr running on the SSH-connected
    /// gateway computer; the user is connected, so the copy never says otherwise.
    static let routedHostUnavailableTitle = "Timeline isn't available here yet"

    /// `gatewayHost` is the host Herdcats itself is SSH'd into (the gateway the
    /// routed machine is reached through); `machineLabel` is the routed
    /// HerdrMachine's label. Missing values fall back to wording that never
    /// invents a name.
    static func routedHostExplanation(gatewayHost: String?, machineLabel: String?) -> String {
        let host = gatewayHost?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let gatewayName = !host.isEmpty && host != "unknown" ? host : "your SSH computer"
        let label = machineLabel?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let machineName = !label.isEmpty ? label : "this machine"
        return "This machine is reached through Herdr on \(gatewayName). Timeline can read sessions only on a computer Herdcats connects to with SSH. To use Timeline here, add \(machineName) as its own connection. You can still send messages below."
    }

    static func explanation(
        _ code: String,
        agent: String?,
        installedVersion: String? = nil,
        pinnedVersion: String = HostHelperSetupPlan.version,
        supportedAgents: Set<String> = Set(HostHelperSetupPlan.agents),
        gatewayHost: String? = nil,
        machineLabel: String? = nil
    ) -> String {
        let isCodex = agent == "codex"
        let canSupportCodex = supportedAgents.contains("codex")
        switch code {
        case "helper_missing":
            if isCodex {
                return canSupportCodex
                    ? "Install Herdcats Bridge to see this session as native messages."
                    : "This version of Herdcats can't install a version of Herdcats Bridge that supports Codex yet."
            }
            return "Install Herdcats Bridge to see this session as native messages."
        case "helper_update_required":
            if let agent, !supportedAgents.contains(agent) {
                return "This version of Herdcats can't install a version of Herdcats Bridge that supports \(agent.capitalized) yet."
            }
            let feature = isCodex ? "Codex conversations" : "session images"
            if let installed = installedVersion, !installed.isEmpty {
                return "Herdcats Bridge \(installed) on this computer can't show \(feature). Upgrade to \(pinnedVersion)."
            } else {
                return "The Herdcats Bridge on this computer can't show \(feature). Upgrade to \(pinnedVersion)."
            }
        case "unsupported_agent":
            if isCodex {
                if isNeedsUpgrade(code: code, agent: agent, bridgeVersion: installedVersion, supportedAgents: supportedAgents),
                   let installedVersion {
                    return "Herdcats Bridge \(installedVersion) on this computer can't show Codex conversations. Upgrade to \(pinnedVersion)."
                }
                if !canSupportCodex {
                    return "This version of Herdcats can't install a version of Herdcats Bridge that supports Codex yet."
                }
            }
            return "Timeline isn't available for this agent yet."
        case "routed_host_unsupported":
            return routedHostExplanation(gatewayHost: gatewayHost, machineLabel: machineLabel)
        case "session_reference_unavailable", "ambiguous_session", "invalid_session_reference":
            if isCodex {
                return "If this stays after Codex replies, restart Codex with `codex --no-daemon`."
            }
            return "This pane has no verified session history."
        case "session_cwd_mismatch":
            if isCodex {
                return "The session folder does not match the pane’s folder, so Herdcats will not guess."
            }
            return "This pane's session folder does not match the verified session."
        case "transcript_root_unavailable":
            return "Session history is unavailable on this host."
        case "source_changing": return "Session history is being updated. Try again in a moment."
        case "record_too_large":
            if isNeedsUpgrade(code: code, agent: agent, bridgeVersion: installedVersion, supportedAgents: supportedAgents) {
                return "Herdcats Bridge cannot read an oversized session record. Upgrade Herdcats Bridge to reload history."
            }
            return "Herdcats Bridge cannot read an oversized session record."
        case "source_too_large": return "This session exceeds the current history size limit."
        case "unsafe_transcript_path", "invalid_transcript", "session_mismatch":
            return "Session history could not be read safely."
        case "herdr_unavailable": return "Herdcats Bridge cannot reach Herdr on this host. Check the host session, then try again."
        case "unsupported_herdr_protocol", "protocol_mismatch":
            return "Herdcats Bridge and the Herdr version are not compatible."
        default:
            return "Check that the optional Herdcats Bridge is available on your host, then try again."
        }
    }
    @MainActor private func prepareWaitingRow() async {
        let expected = waitingTaskKey
        waitingKey = nil
        guard expected.eligible, let key = expected.hooks, let agent = ApprovalAgent(rawValue: key.agent) else { return }
        var grace = PhoneAnswersGrace()
        _ = grace.update(eligible: true, now: ProcessInfo.processInfo.systemUptime)
        do { try await Task.sleep(for: .seconds(PhoneAnswersGrace.delay)) } catch { return }
        guard !Task.isCancelled, waitingTaskKey == expected,
            grace.update(eligible: true, now: ProcessInfo.processInfo.systemUptime) else { return }
        waitingKey = expected
        await appModel.phoneAnswers.check(key) {
            guard !Task.isCancelled, waitingTaskKey == expected, hooksContextIsCurrent(key) else { throw HerdrError.notConnected }
            return try await connection.hooksStatus(agent: agent, target: key.target, owned: key.generation)
        }
    }

    @MainActor private func respondToApproval(_ pending: AgentPendingApproval, option: AgentApprovalOption) {
        guard showsApproval, let owner = activeOwner, let client = activeClient,
            connectionState.isReady, scenePhase == .active,
            let current = timeline.pendingApproval,
            ApprovalResponseState.Scope(current) == ApprovalResponseState.Scope(pending),
            current.options.contains(option) else { return }
        approvalResponse.sync(current)
        guard let token = approvalResponse.begin(current) else { return }
        let ownedConnection = connectionState
        Task { @MainActor in
            do {
                let result = try await client.respond(requestId: current.requestId, option: option.number, labelLines: option.labelLines)
                guard approvalViewVisible, activeOwner == owner, connectionState == ownedConnection, scenePhase == .active,
                    timeline.phase == .ready, timeline.identity == current.identity,
                    timeline.generation == current.generation else { return }
                approvalResponse.sync(timeline.pendingApproval)
                let effects = approvalResponse.succeed(token: token, confirmed: result.confirmed,
                    focusComposer: ApprovalResponseState.focusesComposer(agent: current.agent, option: option))
                if effects.haptic { HapticFeedback.impact(.light) }
                if effects.focusComposer { onFocusComposer() }
            } catch {
                guard activeOwner == owner, connectionState == ownedConnection, scenePhase == .active else { return }
                approvalResponse.sync(timeline.pendingApproval)
                approvalResponse.fail(token: token, error: (error as? AgentBridgeError) ?? .closed)
            }
        }
    }

    @MainActor private func run() async {
        historyLoader.cancel()
        approvalResponse.reset()
        let owner = UUID()
        activeOwner = owner
        timeline.begin(owner: owner, paneId: paneId)
        pageError = nil
        isPaging = false
        bridgeCapabilities = await connection.bridgeCapabilities(for: connectionState.generation)
        bridgeVersion = await connection.bridgeVersion(for: connectionState.generation)
        Self.publishBridgeVersion(bridgeVersion, to: bridgeUpgrade, owner: owner, activeOwner: activeOwner)
        clearCodexDaemonResult()
        guard scenePhase == .active, connectionState.isReady else {
            timeline.end(owner: owner, error: .closed)
            return
        }
        #if DEBUG && targetEnvironment(simulator)
            if ScreenshotFixtures.enabled {
                if ScreenshotFixtures.screen == "timeline-history-position" { following = false }
                if ScreenshotFixtures.screen.contains("helper-old") || ScreenshotFixtures.screen.contains("needs-upgrade") {
                    bridgeVersion = "0.1.4"
                } else if ScreenshotFixtures.screen == "optional-upgrade-menu" {
                    bridgeVersion = "0.2.0"
                }
                if ScreenshotFixtures.screen == "timeline-codex-no-session" {
                    bridgeCapabilities = AgentBridgeCapabilities(claudeTimeline: true, codexTimeline: true, codexDaemonConfig: true, codexDaemonStop: true, recordContent: true)
                }
                if AgentTimelineScreenshotFixture.showsApproval {
                    bridgeCapabilities = AgentBridgeCapabilities(claudeTimeline: true, codexTimeline: true, recordContent: true, approvals: true)
                }
                Self.publishBridgeVersion(bridgeVersion, to: bridgeUpgrade, owner: owner, activeOwner: activeOwner)
                do {
                    try timeline.consume(AgentTimelineScreenshotFixture.frame(
                        paneId: paneId,
                        helperInstalled: fixtureHelperInstalled || (retry > 0 && ProcessInfo.processInfo.arguments.contains("-hc.timeline-helper-installed"))
                    ), owner: owner)
                    if AgentTimelineScreenshotFixture.showsApproval {
                        try timeline.consume(AgentTimelineScreenshotFixture.approvalFrame(paneId: paneId, agent: ScreenshotFixtures.paneAgent), owner: owner)
                        approvalResponse.sync(timeline.pendingApproval)
                        updateTimelineSheet()
                    }
                } catch { timeline.end(owner: owner, error: (error as? AgentBridgeError) ?? .invalidFrame) }
                return
            }
        #endif
        let ownedConnection = connectionState
        var opened: AgentBridgeClient?
        do {
            let client = try await connection.openAgentTimeline(paneId: paneId, agent: agent) { frame in
                try await commit(frame, owner: owner, connectionState: ownedConnection)
            }
            opened = client
            try Task.checkCancellation()
            if activeOwner == owner {
                activeClient = client
                bridgeCapabilities = await connection.bridgeCapabilities(for: ownedConnection.generation)
                bridgeVersion = await connection.bridgeVersion(for: ownedConnection.generation)
                Self.publishBridgeVersion(bridgeVersion, to: bridgeUpgrade, owner: owner, activeOwner: activeOwner)
            }
            try await client.waitUntilClosed()
        } catch {
            if activeOwner == owner {
                bridgeCapabilities = await connection.bridgeCapabilities(for: ownedConnection.generation)
                bridgeVersion = await connection.bridgeVersion(for: ownedConnection.generation)
                Self.publishBridgeVersion(bridgeVersion, to: bridgeUpgrade, owner: owner, activeOwner: activeOwner)
            }
            timeline.end(owner: owner, error: (error as? AgentBridgeError) ?? .closed)
        }
        if let opened { await connection.endAgentTimeline(opened) }
        if activeOwner == owner {
            activeClient = nil
            activeOwner = nil
            isPaging = false
        }
    }

    @ViewBuilder private var codexManualFallback: some View {
        VStack(spacing: 8) {
            Text(Self.codexManualFallbackText())
                .font(.jost(.subheadline))
                .foregroundStyle(Theme.textMuted)
                .multilineTextAlignment(.center)
            Button {
                UIPasteboard.general.string = Self.codexManualConfigLine
                copiedCodexConfig = true
            } label: {
                Label(copiedCodexConfig ? "Setting copied" : "Copy setting", systemImage: copiedCodexConfig ? "checkmark" : "doc.on.doc")
            }
            .buttonStyle(.herdrSecondary)
            .accessibilityIdentifier("timeline-codex-copy-setting")
        }
    }

    @ViewBuilder private var codexStopCommandFallback: some View {
        Button {
            UIPasteboard.general.string = Self.codexDaemonStopCommand
            copiedCodexStop = true
        } label: {
            Label(copiedCodexStop ? "Command copied" : "Copy stop command", systemImage: copiedCodexStop ? "checkmark" : "doc.on.doc")
        }
        .buttonStyle(.herdrPrimary)
        .accessibilityIdentifier("timeline-codex-copy-stop")
    }

    @ViewBuilder private var codexUpdaterStopFallback: some View {
        Button {
            UIPasteboard.general.string = Self.codexUpdaterStopCommand
            copiedCodexUpdaterStop = true
        } label: {
            Label(copiedCodexUpdaterStop ? "Command copied" : "Copy turn-off command", systemImage: copiedCodexUpdaterStop ? "checkmark" : "doc.on.doc")
        }
        .buttonStyle(.herdrPrimary)
        .accessibilityIdentifier("timeline-codex-copy-updater-stop")
    }

    private func clearCodexDaemonResult() {
        codexDaemonStopOutcome = nil
        codexDaemonEpisode += 1
    }

    /// The confirmation sheet can outlive the state that offered it; if the
    /// pane no longer asks for the stop (session identified, server already
    /// off, stream re-ran), close it instead of stopping a working server.
    private func dismissStaleCodexDaemonConfirm() {
        guard showsCodexDaemonConfirm else { return }
        if !Self.canRunCodexDaemonStop(
            phase: timeline.phase,
            agent: agent,
            route: connection.route,
            capabilities: bridgeCapabilities,
            codexDaemon: timeline.codexDaemon
        ) {
            showsCodexDaemonConfirm = false
        }
    }

    private func disableCodexDaemon() {
        guard !isDisablingCodexDaemon else { return }
        // The sheet may have stayed open after the pane moved on; revalidate
        // the live state so confirming a stale sheet never stops a server the
        // pane no longer reports as running.
        guard Self.canRunCodexDaemonStop(
            phase: timeline.phase,
            agent: agent,
            route: connection.route,
            capabilities: bridgeCapabilities,
            codexDaemon: timeline.codexDaemon
        ) else { return }
        guard let generation = connectionState.generation else { return }
        // Bind the up-to-90 s stop to the episode that started it. Any clear
        // while it runs — the pane left the unavailable state, the stream
        // re-ran, or a fresh daemon report retired the previous result — ends
        // that episode, and the late answer must not replace newer guidance.
        let initiatedEpisode = codexDaemonEpisode
        let initiatedReportSeq = codexDaemonReportSeq
        isDisablingCodexDaemon = true
        Task {
            let outcome: CodexDaemonStopOutcome
            do {
                let result = try await connection.disableCodexDaemon(
                    target: setupTarget,
                    owned: generation,
                    stopRunning: true
                )
                // No retry: the bridge re-emits timeline.unavailable when
                // codex_daemon changes, and a re-run would clear this feedback,
                // which tells the user to restart this pane's Codex.
                outcome = CodexDaemonStopOutcome(result: result)
            } catch {
                outcome = CodexDaemonStopOutcome(
                    errorDescription: "Could not turn off Codex's shared background server: \(error.localizedDescription)")
            }
            if Self.shouldRecordStopOutcome(
                initiatedEpisode: initiatedEpisode,
                currentEpisode: codexDaemonEpisode,
                initiatedReportSeq: initiatedReportSeq,
                currentReportSeq: codexDaemonReportSeq,
                outcome: outcome,
                latestCodexDaemon: timeline.codexDaemon
            ) {
                codexDaemonStopOutcome = outcome
            }
            isDisablingCodexDaemon = false
        }
    }
    @MainActor private func commit(
        _ frame: AgentBridgeFrame, owner: UUID, connectionState owned: AgentTimelineConnectionState
    ) async throws {
        // The handshake is recorded before watch events arrive. Load its version
        // before publishing unavailable, so an older host can open the upgrade drawer.
        let version = await connection.bridgeVersion(for: owned.generation)
        // Readiness/generation and the store mutation share one MainActor turn;
        // a callback suspended before the UI hop cannot commit after reconnect.
        try owned.requireCurrent(connectionState)
        guard activeOwner == owner else { return }
        bridgeVersion = version
        Self.publishBridgeVersion(version, to: bridgeUpgrade, owner: owner, activeOwner: activeOwner)
        try timeline.consume(frame, owner: owner)
        approvalResponse.sync(timeline.pendingApproval)
    }
    @MainActor private func mergeHistoryPreservingPosition(
        _ page: AgentBridgePayload, owner: UUID, generation: UInt64
    ) throws {
        // Capture at response time so scrolling while a page is in flight is respected.
        let visible = entryFrames
            .filter { $0.value.maxY > topOverlayInset && $0.value.minY < viewportHeight - bottomOverlayInset - approvalInset }
            .min { $0.value.minY < $1.value.minY }
        try timeline.mergeHistory(page, owner: owner, generation: generation)
        guard activeOwner == owner, timeline.generation == generation, let visible,
              let item = conversationItems.first(where: { $0.entries.contains { $0.id == visible.key } })
        else { return }
        historyScrollAnchor = TimelineHistoryScrollAnchor(
            id: item.id, top: visible.value.minY, height: visible.value.height, viewportHeight: viewportHeight)
    }

    @MainActor private func refreshHistory() async {
        await historyLoader.load { await loadOlder() }
    }

    @MainActor private func loadOlder() async {
        guard timeline.phase == .ready, timeline.hasMore, !isPaging,
            let owner = activeOwner, let generation = timeline.generation, let before = timeline.before
        else {
            TimelineHistoryDiagnostics.record("history.skipped ready=\(timeline.phase == .ready) has_more=\(timeline.hasMore) paging=\(isPaging) owner_present=\(activeOwner != nil) generation_present=\(timeline.generation != nil) cursor_present=\(timeline.before != nil)")
            return
        }
        let attempt = UUID().uuidString
        let started = ContinuousClock.now
        var stage = "request"
        TimelineHistoryDiagnostics.record("history.start attempt=\(attempt) client=\(activeClient?.id.uuidString ?? "none") generation=\(generation) loaded_entries=\(timeline.entries.count) cursor_in_loaded_entries=\(timeline.entries.contains { $0.id == before }) task_cancelled=\(Task.isCancelled)")
        following = false
        isPaging = true
        pageError = nil
        defer { if activeOwner == owner { isPaging = false } }
        let ownedConnection = connectionState
        do {
            #if DEBUG && targetEnvironment(simulator)
                if ScreenshotFixtures.enabled {
                    if ScreenshotFixtures.screen == "timeline-history-position" {
                        try await Task.sleep(for: .seconds(5))
                    } else if ProcessInfo.processInfo.arguments.contains("-hc.timeline-history-delay") {
                        try await Task.sleep(for: .seconds(2))
                    }
                    let page = try AgentTimelineScreenshotFixture.historyPage(paneId: paneId, generation: generation)
                    try mergeHistoryPreservingPosition(page, owner: owner, generation: generation)
                    TimelineHistoryDiagnostics.record("history.fixture_merged attempt=\(attempt) loaded_entries=\(timeline.entries.count)")
                    return
                }
            #endif
            guard let client = activeClient else { throw AgentBridgeError.closed }
            let page = try await client.request(
                .history, params: AgentBridgeParams(generation: generation, before: before), timeout: 15)
            try Task.checkCancellation()
            stage = "connection_validation"
            TimelineHistoryDiagnostics.record("history.response attempt=\(attempt) elapsed=\(started.duration(to: .now)) response_generation=\(page.generation.map(String.init) ?? "missing") entries=\(page.entries?.count ?? -1) has_more=\(page.hasMore.map(String.init) ?? "missing") identity_present=\(page.identity != nil) pane_match=\(page.identity?.paneId == timeline.identity?.paneId) terminal_match=\(page.identity?.terminalId == timeline.identity?.terminalId) agent_match=\(page.identity?.agent == timeline.identity?.agent) session_match=\(page.identity?.nativeSessionId == timeline.identity?.nativeSessionId)")
            try ownedConnection.requireCurrent(connectionState)
            stage = "merge"
            try mergeHistoryPreservingPosition(page, owner: owner, generation: generation)
            TimelineHistoryDiagnostics.record("history.merged attempt=\(attempt) elapsed=\(started.duration(to: .now)) loaded_entries=\(timeline.entries.count) has_more=\(timeline.hasMore) owner_current=\(activeOwner == owner) generation_current=\(timeline.generation == generation)")
        } catch {
            TimelineHistoryDiagnostics.record("history.failed attempt=\(attempt) stage=\(stage) reason=\(TimelineHistoryDiagnostics.reason(error)) elapsed=\(started.duration(to: .now)) task_cancelled=\(Task.isCancelled) owner_current=\(activeOwner == owner) generation_current=\(timeline.generation == generation) connection_current=\(ownedConnection == connectionState)", failure: true)
            if !Task.isCancelled, activeOwner == owner, timeline.generation == generation {
                pageError = "Earlier messages could not load. Pull down to try again."
            }
        }
    }
}

private struct TimelineActivityView: View {
    let entries: [AgentTimelineEntry]
    let loadContent: @MainActor (String) async throws -> Data
    private struct PreviewKey: Equatable { let entries: [AgentTimelineEntry]; let retry: Int }
    @State private var previewRetry = 0
    @State private var previewFailed = false
    @State private var previewLoading = false
    @State private var showsDetails = false
    @State private var fileChanges: [TimelineFileChange] = []
    private var failures: Int { entries.filter { $0.status == "failed" }.count }

    var body: some View {
        VStack(spacing: 12) {
            Button { showsDetails = true } label: {
                HStack(spacing: 12) {
                    if entries.contains(where: TimelineActivitySummary.isRunning) {
                        ProgressView()
                    } else {
                        Image(systemName: failures > 0 ? "exclamationmark.circle" : "wrench.and.screwdriver")
                            .foregroundStyle(failures > 0 ? Theme.destructive : Theme.textMuted)
                    }
                    VStack(alignment: .leading, spacing: 4) {
                        Text(TimelineActivitySummary.title(entries))
                            .font(.jost(.subheadline, weight: .medium))
                        if failures > 0 {
                            Text("\(failures) failed or interrupted")
                                .font(.jost(.caption)).foregroundStyle(Theme.destructive)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Image(systemName: "chevron.right").font(.jost(.caption, weight: .medium))
                        .foregroundStyle(Theme.textMuted)
                }
                .foregroundStyle(Theme.text).padding(.vertical, 16)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("timeline-tool-\(entries[0].id)")
            .sheet(isPresented: $showsDetails) {
                TimelineActivityDrawer(entries: entries)
                    .presentationDetents([.medium, .large])
                    .presentationDragIndicator(.visible)
                    .presentationCornerRadius(20)
            }
            ForEach(fileChanges) { change in
                TimelineDiffCard(change: change)
            }
            if previewLoading && fileChanges.isEmpty {
                Text("Loading code preview…").font(.jost(.caption)).foregroundStyle(Theme.textMuted)
            }
            if previewFailed {
                HStack {
                    Text("Some code previews couldn’t load.").foregroundStyle(Theme.textMuted)
                    Button("Retry") { previewRetry += 1 }.tint(Theme.accent)
                }.font(.jost(.caption))
            }
        }
        .task(id: PreviewKey(entries: entries, retry: previewRetry)) {
            let snapshot = entries
            var changes: [TimelineFileChange] = []
            var failed = false
            previewLoading = true
            defer { if !Task.isCancelled { previewLoading = false } }
            for entry in snapshot where entry.status == "completed" && ["edit", "multiedit", "write"].contains(entry.toolName?.lowercased() ?? "") {
                let preview = await Task.detached(priority: .userInitiated) { TimelineFileChange.from(entry) }.value
                if !preview.isEmpty { changes += preview; continue }
                guard entry.truncated else { continue }
                guard let reference = entry.contentRef, let length = TimelineContentReference.length(reference), length <= 512 * 1024 else {
                    failed = true
                    continue
                }
                do {
                    let input = try await loadContent(reference)
                    try Task.checkCancellation()
                    let complete = await Task.detached(priority: .userInitiated) { TimelineFileChange.from(entry, inputData: input) }.value
                    changes += complete
                    if complete.isEmpty { failed = true }
                } catch {
                    guard !Task.isCancelled else { return }
                    failed = true
                    TimelineHistoryDiagnostics.record("code.preview_failed reason=\(TimelineHistoryDiagnostics.reason(error))", failure: true)
                }
            }
            guard !Task.isCancelled else { return }
            fileChanges = changes
            previewFailed = failed
        }
    }
}

private struct TimelineDiffCard: View {
    let change: TimelineFileChange
    @State private var showsDiff = false
    var body: some View {
        Button { showsDiff = true } label: {
            VStack(alignment: .leading, spacing: 10) {
                TimelineDiffHeading(change: change)
                TimelineDiffCode(lines: Array(change.previewLines.prefix(12)))
                    .frame(maxHeight: 260)
            }
            .padding(14).foregroundStyle(Theme.text)
            .background(Theme.cardBackground)
            .overlay {
                Rectangle().strokeBorder(Theme.hairline, lineWidth: 1)
            }
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("timeline-diff-\(change.id)")
        .sheet(isPresented: $showsDiff) {
            NavigationStack {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        TimelineDiffHeading(change: change)
                        TimelineDiffCode(lines: change.previewLines)
                    }.padding(16)
                }
                .background(Theme.background)
                .navigationTitle(change.writtenContent ? "Written content" : "Code changes")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button { showsDiff = false } label: { Image(systemName: "xmark") }
                            .accessibilityLabel("Close").accessibilityIdentifier("timeline-diff-close")
                    }
                }
            }
            .presentationDetents([.large])
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(20)
        }
    }
}

private struct TimelineDiffHeading: View {
    let change: TimelineFileChange
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(change.displayPath).font(.jost(.subheadline, weight: .semibold)).lineLimit(2)
                Spacer()
                if !change.writtenContent {
                    Text("+\(change.additions)").foregroundStyle(Theme.successGreen)
                    Text("−\(change.deletions)").foregroundStyle(Theme.destructive)
                }
            }.font(.system(.caption, design: .monospaced, weight: .medium))
            if change.previewShortened {
                Text("Showing the first \(change.lines.count) lines").font(.jost(.caption2)).foregroundStyle(Theme.textMuted)
            }
        }
    }
}

private struct TimelineDiffCode: View {
    let lines: [TimelineDiffLine]
    var body: some View {
        ScrollView(.horizontal) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(lines) { line in
                    HStack(alignment: .top, spacing: 8) {
                        Text(line.kind == .added ? "+" : line.kind == .removed ? "−" : " ")
                            .frame(width: 14).foregroundStyle(tint(line))
                        Text(highlight(line.text, omitted: line.kind == .omitted))
                            .fixedSize(horizontal: true, vertical: true)
                        Spacer(minLength: 8)
                    }
                    .font(.system(.caption, design: .monospaced))
                    .padding(.horizontal, 8).padding(.vertical, 3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(tint(line).opacity(line.kind == .context ? 0 : 0.13))
                }
            }
        }
        .background(Theme.subtleFill)
        .clipped()
    }
    private func tint(_ line: TimelineDiffLine) -> Color {
        switch line.kind {
        case .added: Theme.successGreen
        case .removed: Theme.destructive
        default: Theme.textMuted
        }
    }
    private func highlight(_ text: String, omitted: Bool) -> AttributedString {
        let result = NSMutableAttributedString(string: text, attributes: [.foregroundColor: UIColor(omitted ? Theme.textMuted : Theme.text)])
        guard !omitted else { return AttributedString(result) }
        let rules: [(String, UIColor)] = [
            (#"\b(let|var|const|func|function|return|if|else|for|while|class|struct|enum|import|from|true|false|null|nil|async|await|try|throw|public|private|static)\b"#, .systemPurple),
            (#"\b[0-9]+(?:\.[0-9]+)?\b"#, .systemOrange),
            (#""(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'"#, .systemGreen),
            (#"//.*$"#, .secondaryLabel)
        ]
        for (pattern, color) in rules {
            guard let expression = try? NSRegularExpression(pattern: pattern) else { continue }
            for match in expression.matches(in: text, range: NSRange(text.startIndex..., in: text)) {
                result.addAttribute(.foregroundColor, value: color, range: match.range)
            }
        }
        return AttributedString(result)
    }
}

private struct TimelineActivityDrawer: View {
    let entries: [AgentTimelineEntry]
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    ForEach(entries) { entry in
                        HStack(alignment: .top, spacing: 12) {
                            Image(systemName: "wrench.and.screwdriver")
                                .font(.jost(.body))
                                .foregroundStyle(entry.status == "failed" ? Theme.destructive : Theme.textMuted)
                                .accessibilityLabel(entry.status == "failed" ? "Failed or interrupted" : "Tool activity")
                                .accessibilityIdentifier("timeline-activity-icon-\(entry.id)")
                            Text(TimelineActivitySummary.purpose(entry))
                                .font(.jost(.body, weight: .medium))
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .foregroundStyle(Theme.text)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(16)
                        .background(Theme.cardBackground, in: .rect(cornerRadius: DesignSystem.CornerRadius.lg))
                    }
                }
                .padding(16)
            }
            .background(Theme.background)
            .navigationTitle("Activity")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button { dismiss() } label: {
                        Image(systemName: "xmark")
                    }
                    .accessibilityLabel("Close")
                    .accessibilityIdentifier("timeline-activity-close")
                }
            }
        }
    }
}

private struct AgentTimelineEntryView: View {
    let entry: AgentTimelineEntry
    let files: TimelineContentFiles
    let scope: String
    let loadContent: @MainActor (String) async throws -> Data
    @State private var thumbnail: UIImage?
    @State private var originalURL: URL?
    @State private var previewURL: URL?
    @State private var imageLink: URL?
    @State private var imageError: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if entry.kind == "image" {
                imageContent
            } else if entry.kind == "record" {
                Label("Unsupported content: \(entry.text)", systemImage: "questionmark.square.dashed")
                    .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                if let details = entry.details {
                    Text(details).font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                        .textSelection(.enabled).lineLimit(8)
                        .accessibilityIdentifier("timeline-record-\(entry.id)")
                }
            } else if entry.isTurnInterrupted {
                Label(entry.text, systemImage: "stop.circle")
                    .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                    .accessibilityIdentifier("timeline-interrupted-\(entry.id)")
            } else {
                Text(markdown).font(.jost(.body)).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if entry.truncated {
                Text("Preview shortened.")
                    .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
            }
        }
        .foregroundStyle(Theme.text).frame(maxWidth: .infinity, alignment: .leading)
        .padding(entry.role == "user" ? 16 : 0)
        .padding(.vertical, entry.role == "user" ? 0 : (entry.isTurnInterrupted ? 4 : 8))
        .background {
            if entry.role == "user" {
                RoundedRectangle(cornerRadius: DesignSystem.CornerRadius.lg)
                    .fill(Theme.subtleFill)
            }
        }
        .padding(.leading, entry.role == "user" ? 32 : 0)
        .task(id: entry.contentRef) { if entry.kind == "image" { await loadImage() } }
        .onDisappear { thumbnail = nil }
        .quickLookPreview($previewURL)
    }
    @ViewBuilder private var imageContent: some View {
        if let thumbnail {
            Button { previewURL = originalURL } label: {
                Image(uiImage: thumbnail).resizable().scaledToFit().frame(maxHeight: 320)
                    .accessibilityLabel("Session image")
                    .accessibilityIdentifier("timeline-image-\(entry.id)")
            }.buttonStyle(.plain)
            Text("Tap to open original").font(.jost(.caption)).foregroundStyle(Theme.textMuted)
        } else if let imageLink {
            Label("Image link", systemImage: "photo")
            Text(imageLink.absoluteString).font(.jost(.caption)).textSelection(.enabled)
            Link("Open image", destination: imageLink)
        } else if let imageError {
            Label("Image", systemImage: "photo")
            Text(imageError).font(.jost(.caption)).foregroundStyle(Theme.textMuted)
            Button("Retry") { Task { await loadImage() } }
        } else {
            ProgressView("Loading image…")
        }
    }
    @MainActor private func loadImage() async {
        guard let reference = entry.contentRef else { imageError = "Image reference is unavailable."; return }
        imageError = nil
        do {
            let data = try await loadContent(reference)
            let decoded = try await Task.detached(priority: .userInitiated) { try TimelineImagePayload.decode(data) }.value
            try Task.checkCancellation()
            switch decoded {
            case .image(let preview, let original, let suffix):
                originalURL = try files.file(data: original, key: "\(scope):\(reference):image", name: "Session image.\(suffix)")
                thumbnail = UIImage(data: preview)
                if thumbnail == nil { throw AgentBridgeError.invalidFrame }
            case .link(let url): imageLink = url
            }
        } catch {
            if !Task.isCancelled { imageError = "This image could not be displayed. Try loading it again." }
        }
    }
    private var markdown: AttributedString {
        (try? AttributedString(
            markdown: entry.text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(entry.text)
    }
}

/// Explicit opt-in installation on the displayed SSH endpoint; manual setup stays available.
private struct AgentBridgeSetupView: View {
    var reason: HostHelperSetupReason = .missing
    var agent: String? = nil
    var installedVersion: String? = nil
    var pinnedVersion: String = HostHelperSetupPlan.version
    let isChecking: Bool
    let feedback: String?
    let stage: HostHelperSetupStage
    let target: ConnectionIdentity
    let canInstall: Bool
    let onInstall: () -> Void
    let onCheckAgain: () -> Void
    let onUseTerminal: () -> Void
    @State private var copied = false
    private var command: String {
        HostHelperSetupPlan.manualInstallCommand(reason: reason, pinnedVersion: pinnedVersion)
    }
    private var installLabel: String {
        HostHelperSetupPlan.installLabel(stage: stage, reason: reason, pinnedVersion: pinnedVersion)
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    if reason == .updateRequired {
                        Image(systemName: "arrow.up.circle")
                            .font(.system(size: 36)).foregroundStyle(Theme.text)
                        Text("A newer Herdcats Bridge is available")
                            .font(.jost(.title, weight: .semibold))
                            .accessibilityAddTraits(.isHeader)
                        Text(AgentTimelineView.upgradePitch(agent: agent, installedVersion: installedVersion, pinnedVersion: pinnedVersion))
                            .font(.jost(.body)).foregroundStyle(Theme.textMuted)
                        VStack(alignment: .leading, spacing: 6) {
                            Text("\(target.username)@\(target.host)")
                                .font(.jost(.subheadline, weight: .semibold))
                            Text("~/.local/bin/herdcats-bridge")
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(Theme.textMuted)
                            Text("Port \(target.port)")
                                .font(.jost(.caption))
                                .foregroundStyle(Theme.textMuted)
                            let versionRow = (installedVersion?.isEmpty == false)
                                ? "\(installedVersion!) → \(pinnedVersion)"
                                : "→ \(pinnedVersion)"
                            Text(versionRow)
                                .font(.jost(.caption))
                                .foregroundStyle(Theme.textMuted)
                        }
                        .padding(18).frame(maxWidth: .infinity, alignment: .leading)
                        .background(Theme.cardBackground, in: RoundedRectangle(cornerRadius: 18))
                        Text("Tapping Upgrade downloads \(pinnedVersion), verifies its checksum, and replaces the copy at the location shown. No sudo, background service, or agent hooks are required.")
                            .font(.jost(.subheadline)).foregroundStyle(Theme.textMuted)
                    } else {
                        Image(systemName: "text.bubble.fill")
                            .font(.system(size: 36)).foregroundStyle(Theme.text)
                        Text("A clearer view of your agent")
                            .font(.jost(.title, weight: .semibold))
                            .accessibilityAddTraits(.isHeader)
                        Text("Timeline turns your agent session into readable messages and tool activity, with colors that match your app.")
                            .font(.jost(.body)).foregroundStyle(Theme.textMuted)
                        VStack(alignment: .leading, spacing: 14) {
                            benefit("Read comfortably in light or dark mode", icon: "circle.lefthalf.filled")
                            benefit("Follow messages and review activity summaries", icon: "text.alignleft")
                            benefit("Send prompts with Compose, voice, and photos", icon: "square.and.pencil")
                        }
                        .padding(18).frame(maxWidth: .infinity, alignment: .leading)
                        .background(Theme.cardBackground, in: RoundedRectangle(cornerRadius: 18))
                        VStack(alignment: .leading, spacing: 12) {
                            Text("How Herdcats Bridge works")
                                .font(.jost(.headline)).accessibilityAddTraits(.isHeader)
                            Text("Herdcats Bridge is a small program on your SSH computer that reads your existing agent session and sends its history to Herdcats over your SSH connection. Your agent keeps running in the same pane.")
                                .font(.jost(.subheadline)).foregroundStyle(Theme.textMuted)
                            Text("It runs only while Timeline is open. There’s no background service or agent hook to configure.")
                                .font(.jost(.subheadline)).foregroundStyle(Theme.textMuted)
                        }
                        VStack(alignment: .leading, spacing: 6) {
                            Text("\(target.username)@\(target.host)")
                                .font(.jost(.subheadline, weight: .semibold))
                            Text("~/.local/bin/herdcats-bridge")
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(Theme.textMuted)
                            Text("Port \(target.port) · Herdcats Bridge \(pinnedVersion)")
                                .font(.jost(.caption))
                                .foregroundStyle(Theme.textMuted)
                        }
                        .padding(18).frame(maxWidth: .infinity, alignment: .leading)
                        .background(Theme.cardBackground, in: RoundedRectangle(cornerRadius: 18))
                        Text("Tapping Install downloads the pinned Herdcats Bridge release and verifies its checksum before running the installer. No sudo, background service, or agent hooks are required.")
                            .font(.jost(.subheadline)).foregroundStyle(Theme.textMuted)
                    }

                    DisclosureGroup("Manual setup") {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("Set it up once on your SSH host")
                                .font(.jost(.headline)).accessibilityAddTraits(.isHeader)
                            Button {
                                UIPasteboard.general.string = command
                                copied = true
                            } label: {
                                Label(copied ? "Command copied" : "Copy install command", systemImage: copied ? "checkmark" : "doc.on.doc")
                                    .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(.herdrSecondary)
                            .accessibilityIdentifier("timeline-helper-copy-command")
                            Text("1. Copy the command below.\n2. Run it on the computer where Herdr and your agent are running.\n3. Come back here and tap Check again.")
                                .font(.jost(.subheadline)).foregroundStyle(Theme.textMuted)
                            Button(action: onCheckAgain) {
                                HStack {
                                    if isChecking { ProgressView() }
                                    Text(isChecking ? "Checking…" : "Check again")
                                }.frame(maxWidth: .infinity)
                            }
                            .disabled(isChecking || stage.isBusy).buttonStyle(.herdrSecondary)
                            .accessibilityIdentifier("timeline-helper-check-again")
                            Text(command)
                                .font(.system(.footnote, design: .monospaced)).textSelection(.enabled)
                                .padding(16).frame(maxWidth: .infinity, alignment: .leading)
                                .background(Theme.fieldBackground, in: RoundedRectangle(cornerRadius: 16))
                            Text(reason == .updateRequired
                                ? "The installer checks the download and replaces Herdcats Bridge in your user account. The command includes `--replace`."
                                : "The installer checks the download and installs in your user account.")
                                .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                            Link("View downloads and setup details", destination: URL(string: "https://github.com/zhiyao/herdcats/releases")!)
                                .font(.jost(.subheadline))
                        }
                        .padding(.top, 12)
                    }
                }
                .padding(24)
            }
            .accessibilityIdentifier("timeline-setup-scroll")
            .safeAreaInset(edge: .bottom) {
                VStack(spacing: 12) {
                    if let feedback {
                        Text(feedback).font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                            .accessibilityIdentifier("timeline-setup-feedback")
                    }
                    Button(action: onInstall) {
                        HStack {
                            if stage.isBusy { ProgressView() }
                            Text(installLabel)
                        }.frame(maxWidth: .infinity)
                    }
                    .disabled(stage.isBusy || isChecking || !canInstall)
                    .buttonStyle(.herdrPrimary)
                    .accessibilityIdentifier("timeline-helper-install")
                    if !canInstall {
                        Button(action: onCheckAgain) {
                            HStack {
                                if isChecking { ProgressView() }
                                Text(isChecking ? "Checking…" : "Check again")
                            }.frame(maxWidth: .infinity)
                        }
                        .disabled(isChecking || stage.isBusy).buttonStyle(.herdrSecondary)
                        .accessibilityIdentifier("timeline-helper-check-again")
                    }
                    Button("Use Terminal for now", action: onUseTerminal)
                        .buttonStyle(.herdrSecondary)
                        .accessibilityIdentifier("timeline-helper-use-terminal")
                }
                .padding(20).background(Theme.background)
            }
            .background(Theme.background).foregroundStyle(Theme.text)
            .navigationTitle(reason == .updateRequired ? "Upgrade Herdcats Bridge" : "Set up Timeline")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(action: onUseTerminal) { Image(systemName: "xmark") }
                        .accessibilityLabel("Use Terminal for now")
                        .accessibilityIdentifier("timeline-helper-dismiss")
                }
            }
        }
    }
    private func benefit(_ title: String, icon: String) -> some View {
        Label(title, systemImage: icon).font(.jost(.subheadline))
    }
}

private struct TimelineHistoryScrollAnchor: Equatable {
    let token = UUID()
    let id: String
    let top: CGFloat
    let height: CGFloat
    let viewportHeight: CGFloat
    var unitY: CGFloat {
        let available = viewportHeight - height
        return abs(available) > 1 ? top / available : 0
    }
}

private struct TimelineEntryFramesPreferenceKey: PreferenceKey {
    static var defaultValue: [String: CGRect] = [:]
    static func reduce(value: inout [String: CGRect], nextValue: () -> [String: CGRect]) {
        value.merge(nextValue(), uniquingKeysWith: { _, next in next })
    }
}

private struct TimelineAtBottomPreferenceKey: PreferenceKey {
    static var defaultValue: Bool { true }
    static func reduce(value: inout Bool, nextValue: () -> Bool) { value = value && nextValue() }
}

@MainActor private final class TimelineContentFiles {
    private let cache = NSCache<NSString, NSData>()
    private let directory = FileManager.default.temporaryDirectory.appendingPathComponent("timeline-" + UUID().uuidString)
    private var files: [String: URL] = [:]
    init() { cache.totalCostLimit = 32 * 1024 * 1024 }
    func data(key: String, load: @MainActor () async throws -> Data) async throws -> Data {
        if let bytes = cache.object(forKey: key as NSString) { return bytes as Data }
        let bytes = try await load()
        try Task.checkCancellation()
        cache.setObject(bytes as NSData, forKey: key as NSString, cost: bytes.count)
        return bytes
    }
    func file(data: Data, key: String, name: String) throws -> URL {
        if let file = files[key], FileManager.default.fileExists(atPath: file.path) { return file }
        let folder = directory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let url = folder.appendingPathComponent(name)
        try data.write(to: url, options: [.atomic, .completeFileProtection])
        files[key] = url
        return url
    }
    func reset() { cache.removeAllObjects(); files.removeAll(); try? FileManager.default.removeItem(at: directory) }
    deinit { try? FileManager.default.removeItem(at: directory) }
}

enum TimelineImagePayload: Sendable {
    case image(preview: Data, original: Data, suffix: String)
    case link(URL)
    nonisolated static func decode(_ data: Data) throws -> Self {
        guard let block = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let source = block["source"] as? [String: Any] else { throw AgentBridgeError.invalidFrame }
        if let raw = source["url"] as? String, let url = URL(string: raw),
           ["https", "http"].contains(url.scheme?.lowercased() ?? ""), url.host != nil { return .link(url) }
        guard let encoded = source["data"] as? String, let original = Data(base64Encoded: encoded),
              let image = CGImageSourceCreateWithData(original as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              let properties = CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
              let height = properties[kCGImagePropertyPixelHeight] as? NSNumber,
              width.doubleValue > 0, height.doubleValue > 0,
              width.doubleValue * height.doubleValue <= 200_000_000,
              let thumbnail = CGImageSourceCreateThumbnailAtIndex(image, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: 1024,
                kCGImageSourceShouldCacheImmediately: true] as CFDictionary) else { throw AgentBridgeError.invalidFrame }
        let preview = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(preview, UTType.png.identifier as CFString, 1, nil) else { throw AgentBridgeError.invalidFrame }
        CGImageDestinationAddImage(destination, thumbnail, nil)
        guard CGImageDestinationFinalize(destination) else { throw AgentBridgeError.invalidFrame }
        let identifier = CGImageSourceGetType(image).map { $0 as String } ?? UTType.data.identifier
        let suffix = UTType(identifier)?.preferredFilenameExtension ?? "image"
        return .image(preview: preview as Data, original: original, suffix: suffix)
    }
}

struct CodexDaemonConfirmSheet: View {
    let onConfirm: () -> Void
    let onCancel: () -> Void

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Text("Codex's shared background server only remembers the terminal that started it, so Herdcats can't tell which pane this conversation belongs to. Turning it off lets Herdcats identify this pane's session.")
                        .font(.jost(.body))
                        .foregroundStyle(Theme.text)

                    VStack(alignment: .leading, spacing: 8) {
                        Text("What happens")
                            .font(.jost(.headline))
                        Text("Sets `daemon_auto_start = false` in ~/.codex/config.toml (a backup is made first), then stops Codex's shared background server and its updater on this computer.")
                            .font(.jost(.subheadline))
                            .foregroundStyle(Theme.textMuted)
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text("Cost")
                            .font(.jost(.headline))
                        Text("Every Codex session using the shared server disconnects. Restart those sessions; new ones start without it.")
                            .font(.jost(.subheadline))
                            .foregroundStyle(Theme.textMuted)
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text("Undo")
                            .font(.jost(.headline))
                        Text("codex features enable daemon_auto_start")
                            .font(.system(.footnote, design: .monospaced))
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Theme.fieldBackground, in: RoundedRectangle(cornerRadius: 10))
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text("Safety")
                            .font(.jost(.headline))
                        Text("Herdcats only stops processes it can confirm belong to Codex; anything else is left running.")
                            .font(.jost(.subheadline))
                            .foregroundStyle(Theme.textMuted)
                    }

                    Link("Learn more", destination: URL(string: "https://herdcats.dev/support/#using-app-codex-shared-background-server")!)
                        .font(.jost(.subheadline))
                }
                .padding(20)
            }
            .navigationTitle("Turn off shared server")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { onCancel() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Turn off", role: .destructive) { onConfirm() }
                        .fontWeight(.semibold)
                }
            }
        }
    }
}
