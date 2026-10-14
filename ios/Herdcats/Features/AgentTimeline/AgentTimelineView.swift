import SwiftUI
import UIKit

/// Native session content; the owning pane supplies its composition dock.
struct AgentTimelineView: View {
    let paneId: String
    let connection: HerdrConnection
    let timeline: AgentTimelineState
    @Environment(AppModel.self) private var appModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var activeClient: AgentBridgeClient?
    @State private var activeOwner: UUID?
    @State private var showsSetup = false
    @State private var retry = 0
    @State private var isPaging = false
    @State private var pageError: String?
    @State private var following = true
    @State private var scrollPosition: String?

    private var connectionState: AgentTimelineConnectionState {
        AgentTimelineConnectionState(
            phase: appModel.phase, generation: appModel.ownership.transportGeneration)
    }
    private struct TaskKey: Equatable {
        let paneId: String
        let connection: AgentTimelineConnectionState
        let isActive: Bool
        let retry: Int
    }
    private var taskKey: TaskKey {
        TaskKey(paneId: paneId, connection: connectionState, isActive: scenePhase == .active, retry: retry)
    }
    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 16) {
                    if timeline.phase == .ready {
                        HStack {
                            Label("Conversation", systemImage: "text.bubble")
                            Spacer()
                            if timeline.delayed {
                                Text("Delayed")
                            } else if let status = timeline.agentStatus {
                                Text(status.capitalized)
                            }
                        }
                        .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                        if timeline.hasMore {
                            Button {
                                following = false
                                Task { await loadOlder() }
                            } label: {
                                HStack {
                                    if isPaging { ProgressView() }
                                    Text("Load earlier messages")
                                }
                                .frame(maxWidth: .infinity)
                            }
                            .disabled(isPaging).buttonStyle(.herdrSecondary)
                        }
                        if let pageError {
                            Text(pageError).font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                        }
                        if timeline.entries.isEmpty {
                            ContentUnavailableView(
                                "No messages yet", systemImage: "text.bubble",
                                description: Text("Messages from this Claude session will appear here."))
                        }
                        ForEach(timeline.entries) { entry in AgentTimelineEntryView(entry: entry).id(entry.id)
                        }
                        if timeline.delayed {
                            Text("Source update delayed; showing verified history.")
                                .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                        }
                        if timeline.unknownRecords > 0 {
                            Text("Some session records are not displayed.")
                                .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                        }
                        if timeline.droppedEntries > 0 {
                            Text("Showing recent session history.")
                                .font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                        }
                    } else {
                        availability
                    }
                }
                .scrollTargetLayout()
                .padding(DesignSystem.Spacing.base)
            }
            .accessibilityIdentifier("agent-timeline-scroll")
            .scrollPosition(id: $scrollPosition, anchor: .top)
            .scrollDismissesKeyboard(.interactively)
            .simultaneousGesture(DragGesture().onChanged { _ in following = false })
            .onChange(of: timeline.revision) { _, _ in
                guard following, let last = timeline.entries.last else { return }
                proxy.scrollTo(last.id, anchor: .bottom)
            }
            .onChange(of: timeline.identity) { _, _ in
                scrollPosition = nil
                following = true
            }
            .safeAreaInset(edge: .bottom) {
                if !following && timeline.phase == .ready {
                    Button("Latest", systemImage: "arrow.down") {
                        following = true
                        if let last = timeline.entries.last { proxy.scrollTo(last.id, anchor: .bottom) }
                    }
                    .accessibilityIdentifier("timeline-latest")
                    .font(.jost(.subheadline)).padding(8)
                    .frame(maxWidth: .infinity).background(Theme.cardBackground)
                }
            }
        }
        .background(Theme.background)
        .task(id: taskKey) { await run() }
        .sheet(isPresented: $showsSetup) {
            AgentBridgeSetupView {
                showsSetup = false
                retry += 1
            }
        }
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
        case .unavailable(let code):
            ContentUnavailableView {
                Label(code == "helper_missing" ? "Set up Timeline" : "Timeline unavailable", systemImage: "text.bubble")
            } description: {
                Text(explanation(code))
            } actions: {
                if ["helper_missing", "protocol_mismatch", "record_too_large"].contains(code) {
                    Button("Set up Timeline") { showsSetup = true }
                        .buttonStyle(.herdrPrimary)
                        .accessibilityIdentifier("timeline-helper-setup")
                }
                Button("Check again") { retry += 1 }.buttonStyle(.herdrSecondary)
                    .accessibilityIdentifier("timeline-helper-retry")
            }
        case .ready: EmptyView()
        }
    }
    private func explanation(_ code: String) -> String {
        switch code {
        case "helper_missing":
            return "Install the small host helper to see this session as native messages. Terminal is available now."
        case "unsupported_agent":
            return "Native history currently supports Claude. You can still send messages below or use Terminal."
        case "routed_host_unsupported": return "Native history is not available through saved machines yet. Messages can still be sent below."
        case "session_reference_unavailable", "ambiguous_session", "invalid_session_reference":
            return "This pane has no verified session history. Compose and Terminal remain available."
        case "transcript_root_unavailable", "transcript_unavailable":
            return "Claude session history is unavailable on this host."
        case "source_changing": return "Session history is being updated. Try again in a moment."
        case "record_too_large": return "This helper cannot read an oversized session record. Update the helper to reload history."
        case "source_too_large": return "This session exceeds the current history size limit. Compose and Terminal remain available."
        case "unsafe_transcript_path", "invalid_transcript", "session_mismatch":
            return "Session history could not be read safely. Compose and Terminal remain available."
        case "herdr_unavailable": return "The helper cannot reach Herdr on this host. Check the host session, then try again."
        case "unsupported_herdr_protocol", "protocol_mismatch":
            return "The host helper and Herdr version are not compatible."
        default:
            return "Check that the optional herdcats-bridge helper is available on your host, then try again."
        }
    }
    @MainActor private func run() async {
        let owner = UUID()
        activeOwner = owner
        timeline.begin(owner: owner, paneId: paneId)
        pageError = nil
        isPaging = false
        guard scenePhase == .active, connectionState.isReady else {
            timeline.end(owner: owner, error: .closed)
            return
        }
        #if DEBUG && targetEnvironment(simulator)
            if ScreenshotFixtures.enabled {
                do {
                    try timeline.consume(AgentTimelineScreenshotFixture.frame(paneId: paneId), owner: owner)
                } catch { timeline.end(owner: owner, error: (error as? AgentBridgeError) ?? .invalidFrame) }
                return
            }
        #endif
        let ownedConnection = connectionState
        var opened: AgentBridgeClient?
        do {
            let client = try await connection.openAgentTimeline(paneId: paneId) { frame in
                try await commit(frame, owner: owner, connectionState: ownedConnection)
            }
            opened = client
            try Task.checkCancellation()
            if activeOwner == owner { activeClient = client }
            try await client.waitUntilClosed()
        } catch {
            timeline.end(owner: owner, error: (error as? AgentBridgeError) ?? .closed)
        }
        if let opened { await connection.endAgentTimeline(opened) }
        if activeOwner == owner {
            activeClient = nil
            activeOwner = nil
            isPaging = false
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
    @MainActor private func loadOlder() async {
        guard !isPaging, let client = activeClient, let owner = activeOwner,
            let generation = timeline.generation, let before = timeline.before
        else { return }
        isPaging = true
        pageError = nil
        let ownedConnection = connectionState
        do {
            let page = try await client.request(
                .history, params: AgentBridgeParams(generation: generation, before: before))
            try ownedConnection.requireCurrent(connectionState)
            try timeline.mergeHistory(page, owner: owner, generation: generation)
        } catch {
            if activeOwner == owner { pageError = "History changed or could not load. Try again." }
        }
        if activeOwner == owner { isPaging = false }
    }
}

private struct AgentTimelineEntryView: View {
    let entry: AgentTimelineEntry
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if entry.kind == "tool" {
                DisclosureGroup {
                    if let input = entry.details, !input.isEmpty {
                        Text("Input").font(.jost(.caption, weight: .semibold))
                        Text(input).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                    }
                    if !entry.text.isEmpty {
                        Text("Output").font(.jost(.caption, weight: .semibold))
                        Text(entry.text).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                    }
                } label: {
                    VStack(alignment: .leading) {
                        Label(entry.toolName ?? "Tool", systemImage: "wrench.and.screwdriver")
                            .font(.jost(.subheadline, weight: .semibold))
                        Text(toolStatus).font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                    }
                }
                .tint(Theme.accent)
                .accessibilityIdentifier("timeline-tool-\(entry.id)")
            } else {
                Text(entry.role == "user" ? "You" : "Claude")
                    .font(.jost(.caption, weight: .semibold)).foregroundStyle(Theme.textMuted)
                Text(markdown).font(.jost(.body)).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if entry.truncated {
                Text("Entry shortened").font(.jost(.caption)).foregroundStyle(Theme.textMuted)
            }
        }
        .foregroundStyle(Theme.text).frame(maxWidth: .infinity, alignment: .leading)
        .padding(16).background(entry.role == "user" ? Theme.subtleFill : Theme.cardBackground)
        .clipShape(.rect(cornerRadius: DesignSystem.CornerRadius.lg))
        .overlay {
            RoundedRectangle(cornerRadius: DesignSystem.CornerRadius.lg).strokeBorder(
                Theme.hairline, lineWidth: 1)
        }
    }
    private var toolStatus: String {
        switch entry.status {
        case "completed": "Completed"
        case "failed": "Failed or interrupted"
        default: "No result recorded yet"
        }
    }
    private var markdown: AttributedString {
        (try? AttributedString(
            markdown: entry.text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(entry.text)
    }
}

/// Instructions only: installation always runs explicitly on the selected host.
private struct AgentBridgeSetupView: View {
    let onCheckAgain: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var copied = false
    private let command = "curl -fsSL https://github.com/zhiyao/herdcats/releases/download/herdcats-bridge-v0.1.3/install.sh | sh -s -- --version 0.1.3"

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    Label("Connect your session", systemImage: "text.bubble")
                        .font(.jost(.title2, weight: .semibold))
                    Text("Run this command on the computer you connected to over SSH, where Herdr and your agent are running.")
                        .font(.jost(.body))
                    Text(command)
                        .font(.system(.footnote, design: .monospaced))
                        .textSelection(.enabled)
                        .padding(16)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Theme.fieldBackground, in: RoundedRectangle(cornerRadius: 16))
                    Button {
                        UIPasteboard.general.string = command
                        copied = true
                    } label: {
                        Label(copied ? "Command copied" : "Copy install command", systemImage: copied ? "checkmark" : "doc.on.doc")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.herdrSecondary)
                    .accessibilityIdentifier("timeline-helper-copy-command")
                    Text("The installer downloads the helper for your computer and checks its checksum. It installs in your user account and keeps your running agent in its existing pane.")
                        .font(.jost(.subheadline)).foregroundStyle(Theme.textMuted)
                    Text("Already installed? Add --replace to the command to update it.")
                        .font(.jost(.subheadline)).foregroundStyle(Theme.textMuted)
                    Button("Check again", action: onCheckAgain)
                        .buttonStyle(.herdrPrimary)
                        .accessibilityIdentifier("timeline-helper-check-again")
                    Link("View downloads and setup details", destination: URL(string: "https://github.com/zhiyao/herdcats/releases")!)
                        .font(.jost(.subheadline))
                }
                .padding(24)
            }
            .background(Theme.background)
            .foregroundStyle(Theme.text)
            .navigationTitle("Set up native Timeline")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) { Button("Done") { dismiss() } }
            }
        }
    }
}
