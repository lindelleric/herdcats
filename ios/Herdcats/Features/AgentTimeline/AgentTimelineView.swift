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
    let topOverlayInset: CGFloat
    let bottomOverlayInset: CGFloat
    let onOutputInteraction: () -> Void
    let setupTarget: ConnectionIdentity
    let onUseTerminal: () -> Void
    let onSetupComplete: () -> Void
    @Environment(AppModel.self) private var appModel
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
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
    @State private var codexDaemonFeedback: String?
    @State private var codexDaemonErrorOccurred = false
    @State private var copiedCodexConfig = false
    @State private var copiedCodexStop = false

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
                                ContentUnavailableView(
                                    "No messages yet", systemImage: "text.bubble",
                                    description: Text("Messages from this session will appear here."))
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
                        } else {
                            availability
                        }
                        // Clear scroll space lets the final entry sit above the
                        // floating Compose bubble without an opaque dock panel.
                        Color.clear
                            .frame(height: bottomOverlayInset)
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
                .onChange(of: viewport.size.height, initial: true) { _, height in viewportHeight = height }
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
                        .onChanged { _ in
                            following = false
                            onOutputInteraction()
                        }
                        .onEnded { value in
                            let firstFrame = conversationItems.first.flatMap { entryFrames[$0.id] }
                            let atHistoryTop = conversationItems.isEmpty || (firstFrame.map { $0.minY >= topOverlayInset - 8 } ?? false)
                            let loadHistory = atHistoryTop && value.translation.height > 60
                            if loadHistory && timeline.hasMore {
                                Task { await refreshHistory() }
                            } else if atBottom { following = true }
                        }
                )
                .onChange(of: timeline.revision) { _, _ in
                    guard following, !conversationItems.isEmpty else { return }
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
                    if following && !conversationItems.isEmpty {
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
                        .padding(.bottom, bottomOverlayInset + 8)
                        .transition(.opacity)
                    }
                    }
                    .animation(reduceMotion ? nil : .easeOut(duration: 0.2), value: !atBottom && timeline.phase == .ready)
                }
            }
        }
        // Let messages scroll behind the pane cards, but keep them below
        // the navigation bar and status area.
        .clipped()
        .background(Theme.background)
        .task(id: taskKey) { await run() }
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
                let isNeedsUpgrade = canPinnedInstallerSupportAgent && (code == "helper_update_required" || (agent == "codex" && code == "unsupported_agent"))
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
        .onChange(of: contentScope) { _, _ in
            historyLoader.cancel()
            entryFrames = [:]
            historyScrollAnchor = nil
            contentFiles.reset()
        }
        .onChange(of: taskKey) { _, _ in historyLoader.cancel() }
        .onDisappear { historyLoader.cancel(); cancelHelperInstallation(); contentFiles.reset() }
        .sheet(isPresented: $showsSetup, onDismiss: {
            cancelHelperInstallation()
            if setupShouldUseTerminal { onUseTerminal() }
        }) {
            AgentBridgeSetupView(
                reason: setupReason,
                agent: agent,
                installedVersion: bridgeVersion,
                isChecking: isCheckingSetup, feedback: setupFeedback,
                stage: helperStage, target: setupTarget,
                canInstall: connectionState.isReady && appModel.supportsHostServices && appModel.connectionIdentity == setupTarget && canPinnedInstallerSupportAgent,
                onInstall: startHelperInstallation,
                onCheckAgain: {
                    setupFeedback = nil
                    isCheckingSetup = true
                    retry += 1
                },
                onUseTerminal: { cancelHelperInstallation(); showsSetup = false }
            )
            .presentationDetents([.large])
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(28)
        }
        .sheet(isPresented: $showsCodexDaemonConfirm) {
            CodexDaemonConfirmSheet(
                onConfirm: {
                    showsCodexDaemonConfirm = false
                    disableCodexDaemon()
                },
                onCancel: {
                    showsCodexDaemonConfirm = false
                }
            )
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(28)
        }
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

    private func beginSetup(reason: HostHelperSetupReason? = nil) {
        let isCodex = agent == "codex"
        let isNeedsUpgrade = canPinnedInstallerSupportAgent && (timeline.phase == .unavailable("helper_update_required") || (isCodex && timeline.phase == .unavailable("unsupported_agent")))
        let resolvedReason = reason ?? (isNeedsUpgrade ? .updateRequired : .missing)
        setupReason = resolvedReason
        setupShouldUseTerminal = true
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
                Text(connectionState.isReady ? "The history connection ended. Check again to reload messages." : "Reconnect to load current session messages.")
            } actions: {
                Button("Check again") { retry += 1 }
                    .buttonStyle(.herdrSecondary)
                    .disabled(!connectionState.isReady)
                    .accessibilityIdentifier("timeline-helper-retry")
            }
        case .unavailable("transcript_unavailable"):
            // Claude writes its transcript only after the first prompt; the
            // helper keeps polling and swaps in the timeline once it exists.
            ContentUnavailableView(
                "No messages yet", systemImage: "text.bubble",
                description: Text("Send a message below to start this session."))
        case .unavailable(let code):
            let isCodex = agent == "codex"
            let isSupportedAgent = canPinnedInstallerSupportAgent
            let isNeedsUpgrade = isSupportedAgent && (code == "helper_update_required" || (isCodex && code == "unsupported_agent"))
            let isMissing = code == "helper_missing"
            let isNoSession = ["session_reference_unavailable", "ambiguous_session", "invalid_session_reference"].contains(code)
            let isCwdMismatch = code == "session_cwd_mismatch"
            let canTurnOffCodexDaemon = Self.canConfigureCodexDaemon(
                agent: agent,
                code: code,
                route: connection.route,
                capabilities: bridgeCapabilities
            )

            ContentUnavailableView {
                Label(
                    isNeedsUpgrade ? "Upgrade Herdcats Bridge" :
                    isCodex && isNoSession ? "Codex session not identified" :
                    isCodex && isCwdMismatch ? "This pane’s Codex session could not be confirmed" :
                    (isMissing ? "Set up Timeline" : "Timeline unavailable"),
                    systemImage: "text.bubble"
                )
            } description: {
                if isCodex && isNoSession, let feedback = codexDaemonFeedback {
                    Text(feedback)
                } else {
                    Text(explanation(code))
                }
            } actions: {
                if (isMissing || isNeedsUpgrade || ["protocol_mismatch", "record_too_large"].contains(code)) && isSupportedAgent {
                    Button(isNeedsUpgrade ? "Upgrade Herdcats Bridge" : (isMissing ? "Install Herdcats Bridge" : "Set up Timeline")) {
                        beginSetup(reason: isNeedsUpgrade ? .updateRequired : .missing)
                    }
                        .buttonStyle(.herdrPrimary)
                        .accessibilityIdentifier("timeline-helper-setup")
                }
                if isCodex && isNoSession {
                    if canTurnOffCodexDaemon && !codexDaemonErrorOccurred && codexDaemonFeedback == nil {
                        Button("Turn off Codex's shared daemon") {
                            showsCodexDaemonConfirm = true
                        }
                        .buttonStyle(.herdrPrimary)
                        .accessibilityIdentifier("timeline-codex-disable-daemon")
                        .disabled(isDisablingCodexDaemon)
                    } else {
                        VStack(spacing: 8) {
                            if !codexDaemonErrorOccurred && codexDaemonFeedback != nil {
                                Button {
                                    UIPasteboard.general.string = Self.codexDaemonStopCommand
                                    copiedCodexStop = true
                                } label: {
                                    Label(copiedCodexStop ? "Command copied" : "Copy stop command", systemImage: copiedCodexStop ? "checkmark" : "doc.on.doc")
                                }
                                .buttonStyle(.herdrPrimary)
                                .accessibilityIdentifier("timeline-codex-copy-stop")
                            } else {
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
                    }
                }
                Button("Check again") { retry += 1 }.buttonStyle(.herdrSecondary)
                    .accessibilityIdentifier("timeline-helper-retry")
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
            supportedAgents: Set(HostHelperSetupPlan.agents)
        )
    }

    static func canConfigureCodexDaemon(
        agent: String?,
        code: String,
        route: HerdrMachineRoute?,
        capabilities: AgentBridgeCapabilities?
    ) -> Bool {
        guard agent == "codex" else { return false }
        let isNoSession = ["session_reference_unavailable", "ambiguous_session", "invalid_session_reference"].contains(code)
        guard isNoSession else { return false }
        guard route == nil else { return false }
        return capabilities?.codexDaemonConfig == true
    }

    static let codexManualConfigLine = "daemon_auto_start = false"
    static let codexDaemonStopCommand = "codex app-server daemon stop"

    static func codexManualFallbackText() -> String {
        "Add `daemon_auto_start = false` under `[features]` in `~/.codex/config.toml`, or start Codex with `codex --no-daemon`."
    }

    static func explanation(
        _ code: String,
        agent: String?,
        installedVersion: String? = nil,
        pinnedVersion: String = HostHelperSetupPlan.version,
        supportedAgents: Set<String> = Set(HostHelperSetupPlan.agents)
    ) -> String {
        let isCodex = agent == "codex"
        let canSupportCodex = supportedAgents.contains("codex")
        switch code {
        case "helper_missing":
            if isCodex {
                return canSupportCodex
                    ? "Install Herdcats Bridge to see this session as native messages. Terminal is available now."
                    : "This version of Herdcats can't install a version of Herdcats Bridge that supports Codex yet."
            }
            return "Install Herdcats Bridge to see this session as native messages. Terminal is available now."
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
                if canSupportCodex {
                    if let installed = installedVersion, !installed.isEmpty {
                        return "Herdcats Bridge \(installed) on this computer can't show Codex conversations. Upgrade to \(pinnedVersion)."
                    } else {
                        return "The Herdcats Bridge on this computer can't show Codex conversations. Upgrade to \(pinnedVersion)."
                    }
                }
                return "This version of Herdcats can't install a version of Herdcats Bridge that supports Codex yet."
            }
            return "Timeline is not available for this agent yet. You can still send messages below or use Terminal."
        case "routed_host_unsupported": return "Native history is not available through saved machines yet. Messages can still be sent below."
        case "session_reference_unavailable", "ambiguous_session", "invalid_session_reference":
            if isCodex {
                return "Codex's shared daemon hides which terminal a session belongs to. Fix with `codex --no-daemon` or turn off the shared daemon."
            }
            return "This pane has no verified session history. Compose and Terminal remain available."
        case "session_cwd_mismatch":
            if isCodex {
                return "The session folder does not match the pane’s folder, so Herdcats will not guess."
            }
            return "This pane's session folder does not match the verified session. Compose and Terminal remain available."
        case "transcript_root_unavailable":
            return "Session history is unavailable on this host."
        case "source_changing": return "Session history is being updated. Try again in a moment."
        case "record_too_large": return "Herdcats Bridge cannot read an oversized session record. Upgrade Herdcats Bridge to reload history."
        case "source_too_large": return "This session exceeds the current history size limit. Compose and Terminal remain available."
        case "unsafe_transcript_path", "invalid_transcript", "session_mismatch":
            return "Session history could not be read safely. Compose and Terminal remain available."
        case "herdr_unavailable": return "Herdcats Bridge cannot reach Herdr on this host. Check the host session, then try again."
        case "unsupported_herdr_protocol", "protocol_mismatch":
            return "Herdcats Bridge and the Herdr version are not compatible."
        default:
            return "Check that the optional Herdcats Bridge is available on your host, then try again."
        }
    }
    @MainActor private func run() async {
        historyLoader.cancel()
        let owner = UUID()
        activeOwner = owner
        timeline.begin(owner: owner, paneId: paneId)
        pageError = nil
        isPaging = false
        bridgeCapabilities = await connection.bridgeCapabilities(for: connectionState.generation)
        bridgeVersion = await connection.bridgeVersion(for: connectionState.generation)
        codexDaemonFeedback = nil
        codexDaemonErrorOccurred = false
        guard scenePhase == .active, connectionState.isReady else {
            timeline.end(owner: owner, error: .closed)
            return
        }
        #if DEBUG && targetEnvironment(simulator)
            if ScreenshotFixtures.enabled {
                if ScreenshotFixtures.screen == "timeline-history-position" { following = false }
                if ScreenshotFixtures.screen.contains("helper-old") || ScreenshotFixtures.screen.contains("needs-upgrade") {
                    bridgeVersion = "0.1.4"
                }
                do {
                    try timeline.consume(AgentTimelineScreenshotFixture.frame(
                        paneId: paneId,
                        helperInstalled: fixtureHelperInstalled || (retry > 0 && ProcessInfo.processInfo.arguments.contains("-hc.timeline-helper-installed"))
                    ), owner: owner)
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
            }
            try await client.waitUntilClosed()
        } catch {
            if activeOwner == owner {
                bridgeCapabilities = await connection.bridgeCapabilities(for: ownedConnection.generation)
                bridgeVersion = await connection.bridgeVersion(for: ownedConnection.generation)
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

    private func disableCodexDaemon() {
        guard !isDisablingCodexDaemon else { return }
        guard let generation = connectionState.generation else { return }
        isDisablingCodexDaemon = true
        Task {
            do {
                let result = try await connection.disableCodexDaemon(
                    target: setupTarget,
                    owned: generation
                )
                codexDaemonFeedback = CodexDaemonConfigResult.message(for: result)
                codexDaemonErrorOccurred = (result.error != nil)
            } catch {
                codexDaemonFeedback = "Could not turn off Codex daemon: \(error.localizedDescription)"
                codexDaemonErrorOccurred = true
            }
            isDisablingCodexDaemon = false
        }
    }
    @MainActor private func commit(
        _ frame: AgentBridgeFrame, owner: UUID, connectionState owned: AgentTimelineConnectionState
    ) throws {
        // Readiness/generation and the store mutation share one MainActor turn;
        // a callback suspended before the UI hop cannot commit after reconnect.
        try owned.requireCurrent(connectionState)
        try timeline.consume(frame, owner: owner)
    }
    @MainActor private func mergeHistoryPreservingPosition(
        _ page: AgentBridgePayload, owner: UUID, generation: UInt64
    ) throws {
        // Capture at response time so scrolling while a page is in flight is respected.
        let visible = entryFrames
            .filter { $0.value.maxY > topOverlayInset && $0.value.minY < viewportHeight - bottomOverlayInset }
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
        .padding(.vertical, entry.role == "user" ? 0 : 8)
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
        "curl -fsSL https://github.com/zhiyao/herdcats/releases/download/herdcats-bridge-v\(pinnedVersion)/install.sh | sh -s -- --version \(pinnedVersion)"
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
                        let feature = agent == "codex" ? "Codex conversations" : "session images"
                        Text("Version \(pinnedVersion) adds \(feature). Your agent keeps running while you upgrade.")
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
                            Text("The installer checks the download and installs in your user account. Already installed? Add --replace to upgrade it.")
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
                    if reason == .missing {
                        VStack(spacing: 3) {
                            Text("\(target.username)@\(target.host)").font(.jost(.subheadline, weight: .semibold))
                            Text("~/.local/bin/herdcats-bridge").font(.system(.caption, design: .monospaced))
                            Text("Port \(target.port) · Herdcats Bridge \(pinnedVersion)").font(.jost(.caption))
                        }.foregroundStyle(Theme.textMuted)
                    }
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
                    Text("Turn off Codex's shared daemon so Herdcats can identify this pane's session.")
                        .font(.jost(.body))
                        .foregroundStyle(Theme.text)

                    VStack(alignment: .leading, spacing: 8) {
                        Text("File to modify")
                            .font(.jost(.headline))
                        Text("~/.codex/config.toml (or $CODEX_HOME/config.toml)")
                            .font(.system(.footnote, design: .monospaced))
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Theme.fieldBackground, in: RoundedRectangle(cornerRadius: 10))
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text("Setting")
                            .font(.jost(.headline))
                        Text("[features]\ndaemon_auto_start = false")
                            .font(.system(.footnote, design: .monospaced))
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Theme.fieldBackground, in: RoundedRectangle(cornerRadius: 10))
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text("Cost")
                            .font(.jost(.headline))
                        Text("Codex background sessions stop working on this computer.")
                            .font(.jost(.subheadline))
                            .foregroundStyle(Theme.textMuted)
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text("Backup & Undo")
                            .font(.jost(.headline))
                        Text("A timestamped backup of your config file is made before changes. To undo, delete the line from config.toml or restore the backup.")
                            .font(.jost(.subheadline))
                            .foregroundStyle(Theme.textMuted)
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        Text("Restart required")
                            .font(.jost(.headline))
                        Text(CodexDaemonConfigResult.restartGuidance)
                            .font(.jost(.subheadline))
                            .foregroundStyle(Theme.textMuted)
                    }
                }
                .padding(20)
            }
            .navigationTitle("Turn off Codex daemon")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { onCancel() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Turn off") { onConfirm() }
                        .fontWeight(.semibold)
                }
            }
        }
    }
}
