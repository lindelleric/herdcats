import Foundation
import Testing

@testable import Herdcats

@Suite("Codex timeline decoding and capabilities")
struct CodexTimelineDecodingTests {
    @Test func capabilitiesDefaultToFalseForOldHelpers() throws {
        let json = "{\"claude_timeline\": true}"
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let caps = try decoder.decode(AgentBridgeCapabilities.self, from: Data(json.utf8))
        #expect(caps.claudeTimeline == true)
        #expect(caps.codexTimeline == false)
        #expect(caps.recordContent == nil)
    }

    @Test func capabilitiesDecodeExplicitCodexTimeline() throws {
        let jsonEnabled = "{\"claude_timeline\": true, \"codex_timeline\": true, \"record_content\": true}"
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let enabled = try decoder.decode(AgentBridgeCapabilities.self, from: Data(jsonEnabled.utf8))
        #expect(enabled.claudeTimeline == true)
        #expect(enabled.codexTimeline == true)
        #expect(enabled.recordContent == true)

        let jsonDisabled = "{\"claude_timeline\": true, \"codex_timeline\": false, \"record_content\": true}"
        let disabled = try decoder.decode(AgentBridgeCapabilities.self, from: Data(jsonDisabled.utf8))
        #expect(disabled.claudeTimeline == true)
        #expect(disabled.codexTimeline == false)
    }

    @Test func capabilitiesDecodeRealHelper014Object() throws {
        // Real capabilities object emitted by helper 0.1.4
        let json014 = """
        {
            "claude_timeline": true,
            "codex_timeline": false,
            "record_content": true,
            "prompts": false,
            "approvals": false,
            "questions": false,
            "routed_hosts": false
        }
        """
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let caps = try decoder.decode(AgentBridgeCapabilities.self, from: Data(json014.utf8))
        #expect(caps.claudeTimeline == true)
        #expect(caps.codexTimeline == false)
        #expect(caps.recordContent == true)
    }

    @Test func capabilitiesDecodeCodexDaemonStopWithAndWithoutTheFlag() throws {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        // 0.2.2 handshake advertises both daemon capabilities.
        let json022 = "{\"claude_timeline\": true, \"codex_timeline\": true, \"codex_daemon_config\": true, \"codex_daemon_stop\": true, \"record_content\": true}"
        let caps022 = try decoder.decode(AgentBridgeCapabilities.self, from: Data(json022.utf8))
        #expect(caps022.codexDaemonStop == true)
        #expect(caps022.codexDaemonConfig == true)

        // 0.2.0/0.2.1 handshakes omit codex_daemon_stop; it defaults to false.
        let json021 = "{\"claude_timeline\": true, \"codex_timeline\": true, \"codex_daemon_config\": true, \"record_content\": true}"
        let caps021 = try decoder.decode(AgentBridgeCapabilities.self, from: Data(json021.utf8))
        #expect(caps021.codexDaemonStop == false)
        #expect(caps021.codexDaemonConfig == true)

        // Explicit false stays false; the memberwise default matches older helpers.
        let jsonFalse = "{\"claude_timeline\": true, \"codex_daemon_stop\": false}"
        #expect(try decoder.decode(AgentBridgeCapabilities.self, from: Data(jsonFalse.utf8)).codexDaemonStop == false)
        #expect(AgentBridgeCapabilities(claudeTimeline: true).codexDaemonStop == false)
    }

    @Test func helperVersionGatingForCodex() {
        #expect(!AgentBridgeClient.isVersionAtLeast("0.1.4", major: 0, minor: 2))
        #expect(!AgentBridgeClient.isVersionAtLeast("0.1.0", major: 0, minor: 2))
        #expect(!AgentBridgeClient.isVersionAtLeast(nil, major: 0, minor: 2))
        #expect(!AgentBridgeClient.isVersionAtLeast("invalid", major: 0, minor: 2))
        #expect(AgentBridgeClient.isVersionAtLeast("0.2.0", major: 0, minor: 2))
        #expect(AgentBridgeClient.isVersionAtLeast("0.2.1", major: 0, minor: 2))
        #expect(AgentBridgeClient.isVersionAtLeast("1.0.0", major: 0, minor: 2))
    }

    @Test func initializeFrameDecodingAndUnavailableResolution() throws {
        // Real initialize frame from 0.1.4: codex_timeline false, bridge_version "0.1.4" -> helper_update_required
        let json014 = """
        {
          "id": "init-1",
          "result": {
            "protocol": 1,
            "bridge_version": "0.1.4",
            "epoch": "epoch-014",
            "capabilities": {
              "claude_timeline": true,
              "codex_timeline": false,
              "record_content": true
            }
          }
        }
        """
        let frame014 = try AgentBridgeFrame.decode(Data(json014.utf8))
        guard let payload014 = frame014.result else {
            Issue.record("Expected result payload in 0.1.4 frame")
            return
        }
        #expect(payload014.bridgeVersion == "0.1.4")
        #expect(payload014.capabilities?.codexTimeline == false)
        #expect(AgentBridgeClient.unavailableCodeForCodex(handshake: payload014) == "helper_update_required")

        // Real initialize frame from 0.2.0 with codex_timeline false -> transcript_root_unavailable
        let json020 = """
        {
          "id": "init-2",
          "result": {
            "protocol": 1,
            "bridge_version": "0.2.0",
            "epoch": "epoch-020",
            "capabilities": {
              "claude_timeline": true,
              "codex_timeline": false,
              "record_content": true
            }
          }
        }
        """
        let frame020 = try AgentBridgeFrame.decode(Data(json020.utf8))
        guard let payload020 = frame020.result else {
            Issue.record("Expected result payload in 0.2.0 frame")
            return
        }
        #expect(payload020.bridgeVersion == "0.2.0")
        #expect(payload020.capabilities?.codexTimeline == false)
        #expect(AgentBridgeClient.unavailableCodeForCodex(handshake: payload020) == "transcript_root_unavailable")
    }
}

@Suite("Codex timeline agent availability and gating")
struct CodexTimelineGatingTests {
    @Test func supportsCodexAndClaude() {
        #expect(TimelineAgentAvailability.supports(agent: "claude"))
        #expect(TimelineAgentAvailability.supports(agent: "codex"))
        #expect(!TimelineAgentAvailability.supports(agent: "pi"))
        #expect(!TimelineAgentAvailability.supports(agent: "cursor"))
        #expect(!TimelineAgentAvailability.supports(agent: nil))
    }

    @Test func helperProbeGatingForCodexAndClaude() throws {
        func report(agents: [String], version: String = "0.2.0") -> String {
            let agentsJSON = agents.map { "\"\($0)\"" }.joined(separator: ",")
            return "HC_HELPER_MANAGED\n{\"name\":\"herdcats-bridge\",\"version\":\"\(version)\",\"protocol\":1,\"herdr_protocol\":22,\"agents\":[\(agentsJSON)]}"
        }

        // Old helper (0.1.4) only has claude and is older than pinned 0.2.0
        let oldHelperReport = report(agents: ["claude"], version: "0.1.4")
        let probeForClaudeOnOld = try HostHelperProbe(output: oldHelperReport, agent: "claude")
        #expect(probeForClaudeOnOld == .present(version: "0.1.4", managed: true, compatible: false))

        let probeForCodexOnOld = try HostHelperProbe(output: oldHelperReport, agent: "codex")
        #expect(probeForCodexOnOld == .present(version: "0.1.4", managed: true, compatible: false))
        #expect(HostHelperSetupPlan.decision(probe: probeForCodexOnOld, replace: false, checkOnly: false, agent: "codex") == .replacementRequired(probeForCodexOnOld))
        #expect(HostHelperSetupPlan.decision(probe: probeForCodexOnOld, replace: true, checkOnly: false, agent: "codex") == .install(replace: true))

        // New helper (0.2.0) has both claude and codex
        let newHelperReport = report(agents: ["claude", "codex"], version: "0.2.0")
        let probeForClaudeOnNew = try HostHelperProbe(output: newHelperReport, agent: "claude")
        #expect(probeForClaudeOnNew == .present(version: "0.2.0", managed: true, compatible: true))

        let probeForCodexOnNew = try HostHelperProbe(output: newHelperReport, agent: "codex")
        #expect(probeForCodexOnNew == .present(version: "0.2.0", managed: true, compatible: true))
    }
}

@Suite("Routed host Timeline pane copy and gating")
@MainActor struct RoutedHostTimelinePaneTests {
    @Test func routedHostCopyUsesGatewayHostAndMachineLabel() {
        let full = "This machine is reached through Herdr on studio-mac. Timeline can read sessions only on a computer Herdcats connects to with SSH. To use Timeline here, add Studio Mini as its own connection. You can still send messages below."
        #expect(AgentTimelineView.routedHostUnavailableTitle == "Timeline isn't available here yet")
        #expect(AgentTimelineView.routedHostExplanation(gatewayHost: "studio-mac", machineLabel: "Studio Mini") == full)
        #expect(AgentTimelineView.explanation("routed_host_unsupported", agent: "claude", gatewayHost: "studio-mac", machineLabel: "Studio Mini") == full)
        #expect(AgentTimelineView.explanation("routed_host_unsupported", agent: "codex", gatewayHost: "studio-mac", machineLabel: "Studio Mini") == full)
    }

    @Test func routedHostCopyFallsBackWithoutInventingNames() {
        #expect(AgentTimelineView.routedHostExplanation(gatewayHost: nil, machineLabel: nil)
            == "This machine is reached through Herdr on your SSH computer. Timeline can read sessions only on a computer Herdcats connects to with SSH. To use Timeline here, add this machine as its own connection. You can still send messages below.")
        // Missing gateway host keeps the machine label.
        let noHost = AgentTimelineView.routedHostExplanation(gatewayHost: nil, machineLabel: "Studio Mini")
        #expect(noHost.contains("through Herdr on your SSH computer."))
        #expect(noHost.contains("add Studio Mini as its own connection."))
        // Missing machine label keeps the gateway host.
        let noLabel = AgentTimelineView.routedHostExplanation(gatewayHost: "studio-mac", machineLabel: nil)
        #expect(noLabel.contains("through Herdr on studio-mac."))
        #expect(noLabel.contains("add this machine as its own connection."))
        // Blank or placeholder hosts never render as names.
        #expect(!AgentTimelineView.routedHostExplanation(gatewayHost: "  ", machineLabel: "  ").contains("  "))
        #expect(!AgentTimelineView.routedHostExplanation(gatewayHost: "unknown", machineLabel: nil).contains("on unknown"))
    }

    @Test func routedHostPaneShowsNoRetrySetupOrDaemonActions() {
        for agent in ["claude", "codex"] {
            #expect(AgentTimelineView.unavailableTitle(code: "routed_host_unsupported", agent: agent) == "Timeline isn't available here yet")
            #expect(!AgentTimelineView.showsPaneCheckAgain(code: "routed_host_unsupported", agent: agent))
            #expect(!AgentTimelineView.showsInstallOrUpgrade(code: "routed_host_unsupported", agent: agent))
            #expect(!AgentTimelineView.isNeedsUpgrade(code: "routed_host_unsupported", agent: agent))
            #expect(AgentTimelineView.codexNoSessionAction(agent: agent, code: "routed_host_unsupported", route: nil, capabilities: AgentBridgeCapabilities(claudeTimeline: true, codexTimeline: true, codexDaemonConfig: true, codexDaemonStop: true, recordContent: true), codexDaemon: "running") == .manualFallback)
        }
        // Other unavailable codes keep the pane Check again button.
        #expect(AgentTimelineView.showsPaneCheckAgain(code: "herdr_unavailable", agent: "claude"))
    }

    @Test func routedHostPaneReadsGatewayAndMachineFromRoute() {
        let machine = HerdrMachine(id: "studio-mini", label: "Studio Mini", target: "studio-mini.local", session: "studio-mini", enabled: true)
        let gatewayIdentity = ConnectionIdentity(host: "gateway.example.com", port: 2222, username: "demo")
        let routed = HerdrConnection(gateway: HerdrConnection(), machine: machine, generation: ConnectionGeneration(rawValue: 1), identity: gatewayIdentity)
        let view = AgentTimelineView(
            paneId: "p-routed",
            agent: "codex",
            connection: routed,
            timeline: AgentTimelineState(),
            topOverlayInset: 0,
            bottomOverlayInset: 0,
            onOutputInteraction: {},
            setupTarget: gatewayIdentity.scoped(to: machine),
            onUseTerminal: {},
            onSetupComplete: {}
        )
        #expect(view.title("routed_host_unsupported") == "Timeline isn't available here yet")
        #expect(view.explanation("routed_host_unsupported")
            == "This machine is reached through Herdr on gateway.example.com. Timeline can read sessions only on a computer Herdcats connects to with SSH. To use Timeline here, add Studio Mini as its own connection. You can still send messages below.")
    }
}

@Suite("Codex timeline states and state transitions")
@MainActor struct CodexTimelineStateTests {
    let owner = UUID()

    private func frame(
        _ event: String, agent: String = "codex", seq: Int = 1, generation: Int = 1,
        code: String? = nil, entries: [String] = ["hello from codex"]
    ) throws -> AgentBridgeFrame {
        let rows = entries.map {
            ["id": "message:\($0)", "kind": "message", "text": $0, "role": "assistant", "truncated": false] as [String: Any]
        }
        var data: [String: Any] = [
            "pane_id": "p-codex",
            "generation": generation,
        ]
        if let code {
            data["code"] = code
        } else {
            data["identity"] = [
                "pane_id": "p-codex",
                "terminal_id": "t1",
                "agent": agent,
                "native_session_id": "00000000-0000-0000-0000-000000000001",
            ]
            data["entries"] = rows
            data["has_more"] = false
        }
        return try AgentBridgeFrame.decode(
            JSONSerialization.data(withJSONObject: [
                "event": event,
                "epoch": "epoch-codex",
                "seq": seq,
                "data": data,
            ])
        )
    }

    @Test func snapshotAcceptsCodexIdentity() throws {
        let state = AgentTimelineState()
        state.begin(owner: owner, paneId: "p-codex")
        let snapshot = try frame("timeline.snapshot", agent: "codex")
        try state.consume(snapshot, owner: owner)
        #expect(state.phase == .ready)
        #expect(state.identity?.agent == "codex")
        #expect(state.entries.count == 1)
        #expect(state.entries.first?.text == "hello from codex")
    }

    @Test func snapshotRejectsUnsupportedAgent() throws {
        let state = AgentTimelineState()
        state.begin(owner: owner, paneId: "p-codex")
        let unsupported = try frame("timeline.snapshot", agent: "unknown_agent")
        #expect(throws: AgentBridgeError.self) {
            try state.consume(unsupported, owner: owner)
        }
    }

    @Test func unavailableEventKeepsCodexDaemonAndReemitUpdatesState() throws {
        func unavailableFrame(seq: Int, code: String, codexDaemon: String?) throws -> AgentBridgeFrame {
            var data: [String: Any] = ["pane_id": "p-codex", "generation": 1, "code": code]
            if let codexDaemon { data["codex_daemon"] = codexDaemon }
            return try AgentBridgeFrame.decode(
                JSONSerialization.data(withJSONObject: [
                    "event": "timeline.unavailable", "epoch": "epoch-codex", "seq": seq, "data": data,
                ]))
        }

        // The wire frame keeps codex_daemon for the pane to act on.
        let running = try unavailableFrame(seq: 1, code: "session_reference_unavailable", codexDaemon: "running")
        #expect(running.data?.codexDaemon == "running")

        let state = AgentTimelineState()
        state.begin(owner: owner, paneId: "p-codex")
        try state.consume(running, owner: owner)
        #expect(state.phase == .unavailable("session_reference_unavailable"))
        #expect(state.codexDaemon == "running")

        // Same code re-emitted with a different codex_daemon updates the state.
        try state.consume(
            try unavailableFrame(seq: 2, code: "session_reference_unavailable", codexDaemon: "not_running"),
            owner: owner)
        #expect(state.phase == .unavailable("session_reference_unavailable"))
        #expect(state.codexDaemon == "not_running")

        // A later re-emit back to running is visible too (same code, new value).
        try state.consume(
            try unavailableFrame(seq: 3, code: "session_reference_unavailable", codexDaemon: "unknown"),
            owner: owner)
        #expect(state.codexDaemon == "unknown")

        // Older bridges omit the field; missing and unrecognized values stay nil.
        try state.consume(
            try unavailableFrame(seq: 4, code: "session_reference_unavailable", codexDaemon: nil),
            owner: owner)
        #expect(state.codexDaemon == nil)
        try state.consume(
            try unavailableFrame(seq: 5, code: "session_reference_unavailable", codexDaemon: "something_new"),
            owner: owner)
        #expect(state.codexDaemon == nil)
        #expect(AgentTimelineState.normalizedCodexDaemon("running") == "running")
        #expect(AgentTimelineState.normalizedCodexDaemon("something_new") == nil)
        #expect(AgentTimelineState.normalizedCodexDaemon(nil) == nil)

        // Leaving the unavailable phase (a ready snapshot) clears it.
        try state.consume(try frame("timeline.snapshot", seq: 6), owner: owner)
        #expect(state.phase == .ready)
        #expect(state.codexDaemon == nil)
    }

    @Test func stopResultDoesNotLeakIntoALaterNoSessionEpisode() throws {
        func unavailable(_ seq: Int, daemon: String?) throws -> AgentBridgeFrame {
            var data: [String: Any] = ["pane_id": "p-codex", "generation": 1, "code": "session_reference_unavailable"]
            if let daemon { data["codex_daemon"] = daemon }
            return try AgentBridgeFrame.decode(
                JSONSerialization.data(withJSONObject: [
                    "event": "timeline.unavailable", "epoch": "epoch-codex", "seq": seq, "data": data,
                ]))
        }
        let state = AgentTimelineState()
        state.begin(owner: owner, paneId: "p-codex")

        // Stop offered and run while the server is running.
        try state.consume(try unavailable(1, daemon: "running"), owner: owner)
        // The successful stop's "restart Codex" result survives the bridge's
        // not_running re-emission within the same episode.
        try state.consume(try unavailable(2, daemon: "not_running"), owner: owner)
        #expect(!AgentTimelineView.codexDaemonResultIsStale(errorOccurred: false, showsStopCommand: false, codexDaemon: state.codexDaemon))

        // The user restarts Codex: the pane becomes ready, ending the episode,
        // so a later no-session state starts without the old result.
        try state.consume(try frame("timeline.snapshot", seq: 3), owner: owner)
        #expect(state.phase == .ready)
        #expect(!AgentTimelineView.codexDaemonResultIsCurrent(phase: state.phase))
        try state.consume(try unavailable(4, daemon: "running"), owner: owner)
        #expect(AgentTimelineView.codexDaemonResultIsCurrent(phase: state.phase))
        // A fresh running report contradicts the recorded "server is off" result.
        #expect(AgentTimelineView.codexDaemonResultIsStale(errorOccurred: false, showsStopCommand: false, codexDaemon: state.codexDaemon))
    }

    @Test func codexPaneDisplaysHelperTooOldState() {
        let view = AgentTimelineView(
            paneId: "p-codex",
            agent: "codex",
            connection: HerdrConnection(),
            timeline: AgentTimelineState(),
            topOverlayInset: 0,
            bottomOverlayInset: 0,
            onOutputInteraction: {},
            setupTarget: ConnectionIdentity(host: "localhost", port: 22, username: "test"),
            onUseTerminal: {},
            onSetupComplete: {}
        )
        // When Codex is supported by pinned installer (0.2.2 pinned release with HostHelperSetupPlan.agents == ["claude", "codex"])
        // Without an installedVersion recorded yet:
        #expect(view.explanation("helper_update_required") == "The Herdcats Bridge on this computer can't show Codex conversations. Upgrade to 0.2.2.")
        #expect(view.explanation("unsupported_agent") == "Timeline is not available for this agent yet. You can still send messages below or use Terminal.")
        #expect(view.explanation("helper_missing") == "Install Herdcats Bridge to see this session as native messages. Terminal is available now.")

        // Explicitly test both ways with injected supportedAgents:
        // 1. Without Codex in supportedAgents
        let unsupportedAgents: Set<String> = ["claude"]
        #expect(AgentTimelineView.explanation("helper_update_required", agent: "codex", supportedAgents: unsupportedAgents) == "This version of Herdcats can't install a version of Herdcats Bridge that supports Codex yet.")
        #expect(AgentTimelineView.explanation("unsupported_agent", agent: "codex", supportedAgents: unsupportedAgents) == "This version of Herdcats can't install a version of Herdcats Bridge that supports Codex yet.")
        #expect(AgentTimelineView.explanation("helper_missing", agent: "codex", supportedAgents: unsupportedAgents) == "This version of Herdcats can't install a version of Herdcats Bridge that supports Codex yet.")

        // 2. With Codex in supportedAgents
        let supportedWithCodex: Set<String> = ["claude", "codex"]
        #expect(AgentTimelineView.explanation("helper_update_required", agent: "codex", installedVersion: "0.1.4", supportedAgents: supportedWithCodex) == "Herdcats Bridge 0.1.4 on this computer can't show Codex conversations. Upgrade to 0.2.2.")
        #expect(AgentTimelineView.explanation("helper_update_required", agent: "codex", installedVersion: nil, supportedAgents: supportedWithCodex) == "The Herdcats Bridge on this computer can't show Codex conversations. Upgrade to 0.2.2.")
        #expect(AgentTimelineView.explanation("unsupported_agent", agent: "codex", installedVersion: "0.1.4", supportedAgents: supportedWithCodex) == "Herdcats Bridge 0.1.4 on this computer can't show Codex conversations. Upgrade to 0.2.2.")
        #expect(AgentTimelineView.explanation("unsupported_agent", agent: "codex", installedVersion: nil, supportedAgents: supportedWithCodex) == "Timeline is not available for this agent yet. You can still send messages below or use Terminal.")
        #expect(AgentTimelineView.explanation("helper_missing", agent: "codex", supportedAgents: supportedWithCodex) == "Install Herdcats Bridge to see this session as native messages. Terminal is available now.")

        // 3. Claude with and without installedVersion
        #expect(AgentTimelineView.explanation("helper_update_required", agent: "claude", installedVersion: "0.1.4", supportedAgents: unsupportedAgents) == "Herdcats Bridge 0.1.4 on this computer can't show session images. Upgrade to 0.2.2.")
        #expect(AgentTimelineView.explanation("helper_update_required", agent: "claude", installedVersion: nil, supportedAgents: unsupportedAgents) == "The Herdcats Bridge on this computer can't show session images. Upgrade to 0.2.2.")
        #expect(AgentTimelineView.explanation("unsupported_agent", agent: "claude", supportedAgents: unsupportedAgents) == "Timeline is not available for this agent yet. You can still send messages below or use Terminal.")
        #expect(AgentTimelineView.explanation("helper_missing", agent: "claude", supportedAgents: unsupportedAgents) == "Install Herdcats Bridge to see this session as native messages. Terminal is available now.")

        // 4. Grammar for fallback
        #expect(AgentTimelineView.explanation("unknown_code", agent: "codex") == "Check that the optional Herdcats Bridge is available on your host, then try again.")
    }

    @Test func canPinnedInstallerSupportVerification() {
        #expect(AgentTimelineView.canPinnedInstallerSupport(agent: "claude"))
        #expect(AgentTimelineView.canPinnedInstallerSupport(agent: "codex"))
        #expect(AgentTimelineView.canPinnedInstallerSupport(agent: nil))
        #expect(AgentTimelineView.canPinnedInstallerSupport(agent: "claude", supportedAgents: ["claude"]))
        #expect(!AgentTimelineView.canPinnedInstallerSupport(agent: "codex", supportedAgents: ["claude"]))
        #expect(AgentTimelineView.canPinnedInstallerSupport(agent: "codex", supportedAgents: ["claude", "codex"]))
        #expect(AgentTimelineView.canPinnedInstallerSupport(agent: nil, supportedAgents: ["claude"]))
    }

    @Test func upgradeRequiresKnownOlderBridge() {
        let cases: [(String?, Bool)] = [("0.1.4", true), ("0.2.0", false), (nil, false), ("invalid", false), ("0.2.1", false), ("0.2.2", false)]
        for (version, needsUpgrade) in cases {
            for code in ["unsupported_agent", "record_too_large"] {
                #expect(AgentTimelineView.isNeedsUpgrade(code: code, agent: "codex", bridgeVersion: version) == needsUpgrade)
                #expect(AgentTimelineView.showsInstallOrUpgrade(code: code, agent: "codex", bridgeVersion: version) == needsUpgrade)
                #expect(AgentTimelineView.showsPaneCheckAgain(code: code, agent: "codex", bridgeVersion: version) == !needsUpgrade)
                if !needsUpgrade {
                    #expect(!AgentTimelineView.explanation(code, agent: "codex", installedVersion: version).contains("Upgrade"))
                }
            }
            #expect(AgentTimelineView.isNeedsUpgrade(code: "helper_update_required", agent: "codex", bridgeVersion: version))
            #expect(!AgentTimelineView.isNeedsUpgrade(code: "helper_missing", agent: "codex", bridgeVersion: version))
            #expect(AgentTimelineView.showsInstallOrUpgrade(code: "helper_missing", agent: "codex", bridgeVersion: version))
        }
    }

    @Test func manualInstallCommandReplacesOnlyForUpgrade() {
        let install = "curl -fsSL https://github.com/zhiyao/herdcats/releases/download/herdcats-bridge-v0.2.2/install.sh | sh -s -- --version 0.2.2"
        #expect(HostHelperSetupPlan.manualInstallCommand(reason: .missing) == install)
        #expect(HostHelperSetupPlan.manualInstallCommand(reason: .updateRequired) == install + " --replace")
    }

    @Test func upgradePitchCopyVerification() {
        // Installed below minimum (e.g. 0.1.4 or nil)
        #expect(
            AgentTimelineView.upgradePitch(agent: "codex", installedVersion: "0.1.4")
            == "Version 0.2.2 adds Codex conversations. Your agent keeps running while you upgrade."
        )
        #expect(
            AgentTimelineView.upgradePitch(agent: "codex", installedVersion: nil)
            == "Version 0.2.2 adds Codex conversations. Your agent keeps running while you upgrade."
        )
        #expect(
            AgentTimelineView.upgradePitch(agent: "claude", installedVersion: "0.1.4")
            == "Version 0.2.2 adds session images. Your agent keeps running while you upgrade."
        )

        // Installed at or above minimum (e.g. 0.2.0)
        #expect(
            AgentTimelineView.upgradePitch(agent: "codex", installedVersion: "0.2.0")
            == "Version 0.2.2 includes fixes for Codex conversations. Your agent keeps running while you upgrade."
        )
        #expect(
            AgentTimelineView.upgradePitch(agent: "claude", installedVersion: "0.2.0")
            == "Version 0.2.2 includes fixes for session history. Your agent keeps running while you upgrade."
        )
    }

    @Test func turnInterruptedEntryModel() throws {
        let json = """
        {
            "id": "turn_aborted:test-turn",
            "kind": "message",
            "role": "assistant",
            "status": "failed",
            "text": "Turn interrupted",
            "truncated": false
        }
        """
        let entry = try JSONDecoder().decode(AgentTimelineEntry.self, from: Data(json.utf8))
        #expect(entry.id == "turn_aborted:test-turn")
        #expect(entry.role == "assistant")
        #expect(entry.status == "failed")
        #expect(entry.text == "Turn interrupted")
    }

    @Test func unknownBridgeDoesNotSuggestUpgrade() {
        #expect(!AgentTimelineView.isNeedsUpgrade(code: "unsupported_agent", agent: "codex"))
        #expect(!AgentTimelineView.isNeedsUpgrade(code: "record_too_large", agent: "codex"))
    }

    @Test func unavailableGatingAndPaneActions() {
        // Oversized records on a known older Bridge offer an upgrade.
        #expect(AgentTimelineView.isNeedsUpgrade(code: "record_too_large", agent: "codex", bridgeVersion: "0.1.4"))
        #expect(AgentTimelineView.isNeedsUpgrade(code: "record_too_large", agent: "claude", bridgeVersion: "0.1.4"))
        #expect(AgentTimelineView.showsInstallOrUpgrade(code: "record_too_large", agent: "codex", bridgeVersion: "0.1.4"))
        #expect(AgentTimelineView.showsInstallOrUpgrade(code: "record_too_large", agent: "claude", bridgeVersion: "0.1.4"))
        #expect(!AgentTimelineView.showsPaneCheckAgain(code: "record_too_large", agent: "codex", bridgeVersion: "0.1.4"))

        // helper_missing and helper_update_required show setup button and suppress pane Check again
        #expect(AgentTimelineView.showsInstallOrUpgrade(code: "helper_missing", agent: "codex"))
        #expect(!AgentTimelineView.showsPaneCheckAgain(code: "helper_missing", agent: "codex"))
        #expect(AgentTimelineView.showsInstallOrUpgrade(code: "helper_update_required", agent: "codex"))
        #expect(!AgentTimelineView.showsPaneCheckAgain(code: "helper_update_required", agent: "codex"))

        // protocol_mismatch stays as is (not needsUpgrade, shows pane Check again)
        #expect(!AgentTimelineView.isNeedsUpgrade(code: "protocol_mismatch", agent: "codex"))
        #expect(!AgentTimelineView.showsInstallOrUpgrade(code: "protocol_mismatch", agent: "codex"))
        #expect(AgentTimelineView.showsPaneCheckAgain(code: "protocol_mismatch", agent: "codex"))

        // other unavailable codes keep pane Check again
        #expect(AgentTimelineView.showsPaneCheckAgain(code: "session_reference_unavailable", agent: "codex"))
        #expect(AgentTimelineView.showsPaneCheckAgain(code: "session_cwd_mismatch", agent: "codex"))
        #expect(AgentTimelineView.showsPaneCheckAgain(code: "transcript_root_unavailable", agent: "codex"))
        #expect(AgentTimelineView.showsPaneCheckAgain(code: "source_changing", agent: "codex"))
        #expect(AgentTimelineView.showsPaneCheckAgain(code: "source_too_large", agent: "codex"))

        // unsupported agent refuses upgrade/install and keeps Check again
        let unsupportedAgents: Set<String> = ["claude"]
        #expect(!AgentTimelineView.isNeedsUpgrade(code: "record_too_large", agent: "codex", bridgeVersion: "0.1.4", supportedAgents: unsupportedAgents))
        #expect(!AgentTimelineView.showsInstallOrUpgrade(code: "record_too_large", agent: "codex", bridgeVersion: "0.1.4", supportedAgents: unsupportedAgents))
        #expect(AgentTimelineView.showsPaneCheckAgain(code: "record_too_large", agent: "codex", bridgeVersion: "0.1.4", supportedAgents: unsupportedAgents))
    }

    @Test func helperSetupViewLabelAndFeedbackLogic() {
        // update reason idle -> "Upgrade to 0.2.2"
        #expect(
            AgentTimelineView.installLabel(stage: .idle, reason: .updateRequired)
            == "Upgrade to 0.2.2"
        )
        #expect(
            AgentTimelineView.installLabel(stage: .idle, reason: .updateRequired, pinnedVersion: "0.2.2")
            == "Upgrade to 0.2.2"
        )

        // busy labels
        #expect(
            AgentTimelineView.installLabel(stage: .checking)
            == "Checking host…"
        )
        #expect(
            AgentTimelineView.installLabel(stage: .installing, reason: .updateRequired)
            == "Upgrading Herdcats Bridge…"
        )
        #expect(
            AgentTimelineView.installLabel(stage: .installing, reason: .missing)
            == "Installing Herdcats Bridge…"
        )
        #expect(
            AgentTimelineView.installLabel(stage: .verifying)
            == "Verifying installation…"
        )

        // missing -> "Install Herdcats Bridge"
        #expect(
            AgentTimelineView.installLabel(stage: .idle, reason: .missing)
            == "Install Herdcats Bridge"
        )
        #expect(
            AgentTimelineView.installLabel(stage: .idle)
            == "Install Herdcats Bridge"
        )

        // managed 0.1.4 replacementRequired -> "Replace Herdcats Bridge with 0.2.2" + found-version feedback
        let managed014 = HostHelperProbe.present(version: "0.1.4", managed: true, compatible: false)
        #expect(
            AgentTimelineView.installLabel(stage: .replacementRequired(managed014))
            == "Replace Herdcats Bridge with 0.2.2"
        )
        #expect(
            AgentTimelineView.replacementFeedback(probe: managed014)
            == "Found Herdcats Bridge 0.1.4 on this host. Tap Replace to upgrade it at the location shown."
        )

        // external helper -> "Install managed Herdcats Bridge"
        let externalHelper = HostHelperProbe.present(version: "0.1.4", managed: false, compatible: false)
        #expect(
            AgentTimelineView.installLabel(stage: .replacementRequired(externalHelper))
            == "Install managed Herdcats Bridge"
        )
        #expect(
            AgentTimelineView.replacementFeedback(probe: externalHelper)
            == "An existing Herdcats Bridge was found elsewhere. Install a managed copy at the location shown; the other copy will remain unchanged."
        )
    }

    @Test func codexPaneDisplaysNoSessionReferenceState() {
        let view = AgentTimelineView(
            paneId: "p-codex",
            agent: "codex",
            connection: HerdrConnection(),
            timeline: AgentTimelineState(),
            topOverlayInset: 0,
            bottomOverlayInset: 0,
            onOutputInteraction: {},
            setupTarget: ConnectionIdentity(host: "localhost", port: 22, username: "test"),
            onUseTerminal: {},
            onSetupComplete: {}
        )
        #expect(view.title("session_reference_unavailable") == "Waiting for Codex session")
        #expect(view.title("ambiguous_session") == "Waiting for Codex session")
        #expect(view.title("invalid_session_reference") == "Waiting for Codex session")
        let explanation = view.explanation("session_reference_unavailable")
        #expect(
            explanation
            == "Codex reports its session after the first message. Send a message below to start. If Timeline still doesn't appear after Codex replies, this Codex is using its shared daemon. Restart it with `codex --no-daemon`."
        )
        // Composer remains usable when session reference is unavailable (identity is nil before first message)
        #expect(TimelineComposePolicy.canCompose(identity: nil, paneId: "p-codex", status: nil, connected: true))
    }

    @Test func codexPaneDisplaysCwdMismatchState() {
        let view = AgentTimelineView(
            paneId: "p-codex",
            agent: "codex",
            connection: HerdrConnection(),
            timeline: AgentTimelineState(),
            topOverlayInset: 0,
            bottomOverlayInset: 0,
            onOutputInteraction: {},
            setupTarget: ConnectionIdentity(host: "localhost", port: 22, username: "test"),
            onUseTerminal: {},
            onSetupComplete: {}
        )
        let explanation = view.explanation("session_cwd_mismatch")
        #expect(explanation.contains("The session folder does not match the pane’s folder") || explanation.contains("The session folder does not match the pane's folder"))
        #expect(explanation.contains("so Herdcats will not guess"))
    }
}

@Suite("Codex timeline prompt submission and routing")
struct CodexTimelinePromptRoutingTests {
    @Test func timelinePromptRoutingDecision() {
        // codex + working -> type then enter
        #expect(HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "codex", agentStatus: "working"))

        // codex idle/done -> agent prompt
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "codex", agentStatus: "idle"))
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "codex", agentStatus: "done"))
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "codex", agentStatus: "blocked"))
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "codex", agentStatus: nil))

        // claude any -> agent prompt
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "claude", agentStatus: "working"))
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "claude", agentStatus: "idle"))
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "claude", agentStatus: "done"))
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "claude", agentStatus: nil))

        // other agents -> agent prompt
        #expect(!HerdrConnection.shouldTypeAndEnterForTimelinePrompt(agent: "cursor", agentStatus: "working"))
    }
}

@Suite("Codex timeline task key")
struct CodexTimelineTaskKeyTests {
    @Test func timelineTaskKeyRestartsOnAgentChange() {
        let conn = AgentTimelineConnectionState(
            phase: .connected(host: "host", username: "user"),
            generation: ConnectionGeneration(rawValue: 1)
        )
        let keyCodex = AgentTimelineView.TaskKey(paneId: "p1", agent: "codex", connection: conn, isActive: true, retry: 0)
        let keyClaude = AgentTimelineView.TaskKey(paneId: "p1", agent: "claude", connection: conn, isActive: true, retry: 0)
        let keyNil = AgentTimelineView.TaskKey(paneId: "p1", agent: nil, connection: conn, isActive: true, retry: 0)
        #expect(keyCodex != keyClaude)
        #expect(keyCodex != keyNil)
        #expect(keyClaude != keyNil)
        #expect(keyCodex == AgentTimelineView.TaskKey(paneId: "p1", agent: "codex", connection: conn, isActive: true, retry: 0))
    }
}

@Suite("Codex daemon gating, messaging and stop command")
struct CodexDaemonConfigTests {
    @Test func codexNoSessionDecisionTable() {
        let stopCaps = AgentBridgeCapabilities(claudeTimeline: true, codexTimeline: true, codexDaemonConfig: true, codexDaemonStop: true, recordContent: true)
        let configOnlyCaps = AgentBridgeCapabilities(claudeTimeline: true, codexTimeline: true, codexDaemonConfig: true, recordContent: true)
        let oldCaps = AgentBridgeCapabilities(claudeTimeline: true, codexTimeline: true, recordContent: true)
        let gateway = HerdrConnection()
        let machine = HerdrMachine(id: "child", label: "Child", target: "u@child", session: "default", enabled: true)
        let route = HerdrMachineRoute(gateway: gateway, machine: machine, generation: ConnectionGeneration(rawValue: 1))

        for code in ["session_reference_unavailable", "ambiguous_session", "invalid_session_reference"] {
            // 3A: bridge can stop the server; daemon running or unverifiable -> offer the stop.
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: stopCaps, codexDaemon: "running") == .turnOffServer)
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: stopCaps, codexDaemon: "unknown") == .turnOffServer)
            // 3A: daemon already off (or unreported) -> no stop button, wait for the first message.
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: stopCaps, codexDaemon: "not_running") == .waitForSession)
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: stopCaps, codexDaemon: nil) == .waitForSession)
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: stopCaps, codexDaemon: "something_new") == .waitForSession)
            // 3B: 0.2.0/0.2.1 bridge can only change config -> upgrade offer, no config-only stop button.
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: configOnlyCaps, codexDaemon: "running") == .upgradeBridge)
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: configOnlyCaps, codexDaemon: nil) == .upgradeBridge)
            // 3C: neither capability -> manual fallback only.
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: oldCaps, codexDaemon: "running") == .manualFallback)
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: nil, capabilities: nil, codexDaemon: "running") == .manualFallback)
            // Routed hosts never get any of these actions, whatever the bridge can do.
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: route, capabilities: stopCaps, codexDaemon: "running") == .manualFallback)
            #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: code, route: route, capabilities: configOnlyCaps, codexDaemon: "running") == .manualFallback)
        }
        // Not a no-session code, or not Codex -> none of the Codex actions apply.
        #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: "session_cwd_mismatch", route: nil, capabilities: stopCaps, codexDaemon: "running") == .manualFallback)
        #expect(AgentTimelineView.codexNoSessionAction(agent: "codex", code: "transcript_root_unavailable", route: nil, capabilities: stopCaps, codexDaemon: "running") == .manualFallback)
        #expect(AgentTimelineView.codexNoSessionAction(agent: "claude", code: "session_reference_unavailable", route: nil, capabilities: stopCaps, codexDaemon: "running") == .manualFallback)
        #expect(AgentTimelineView.codexNoSessionAction(agent: nil, code: "session_reference_unavailable", route: nil, capabilities: stopCaps, codexDaemon: "running") == .manualFallback)
    }

    @Test func codexNoSessionExplanationsMatchTheAction() {
        #expect(AgentTimelineView.codexNoSessionExplanation(.turnOffServer, code: "session_reference_unavailable", agent: "codex")
            == "Codex's shared background server is running, so Herdcats can't tell which session belongs to this pane.")
        #expect(AgentTimelineView.codexNoSessionExplanation(.waitForSession, code: "session_reference_unavailable", agent: "codex")
            == "Codex reports its session after the first message. Send a message below to start. If Timeline still doesn't appear, quit Codex in this pane and start it again.")
        #expect(AgentTimelineView.codexNoSessionExplanation(.upgradeBridge, code: "ambiguous_session", agent: "codex")
            == "Upgrade Herdcats Bridge to turn off Codex's shared background server from here.")
        // 3C keeps today's manual-fallback copy verbatim.
        let manual = AgentTimelineView.codexNoSessionExplanation(.manualFallback, code: "invalid_session_reference", agent: "codex")
        #expect(manual == AgentTimelineView.explanation("invalid_session_reference", agent: "codex"))
        #expect(manual.contains("codex --no-daemon"))
    }

    @Test func statusAndErrorMessageMapping() {
        #expect(AgentTimelineView.codexDaemonStopCommand == "codex app-server daemon stop")

        // Config errors keep their existing copy; nothing was stopped.
        let resInvalid = CodexDaemonConfigResult(status: nil, backup: nil, error: "config_invalid")
        #expect(CodexDaemonConfigResult.message(for: resInvalid).contains("invalid TOML"))
        #expect(!CodexDaemonConfigResult.showsStopCommand(for: resInvalid))
        let resSymlink = CodexDaemonConfigResult(status: nil, backup: nil, error: "config_symlink")
        #expect(CodexDaemonConfigResult.message(for: resSymlink).contains("symbolic link"))
        let resNotTable = CodexDaemonConfigResult(status: nil, backup: nil, error: "features_not_table")
        #expect(CodexDaemonConfigResult.message(for: resNotTable).contains("not a table"))
        let resWriteFailed = CodexDaemonConfigResult(status: nil, backup: nil, error: "write_failed")
        #expect(CodexDaemonConfigResult.message(for: resWriteFailed).contains("failed to write"))
        let resVerification = CodexDaemonConfigResult(status: nil, backup: nil, error: "verification_failed")
        #expect(CodexDaemonConfigResult.message(for: resVerification).contains("verification failed"))
        let resUnknownError = CodexDaemonConfigResult(status: nil, backup: nil, error: "custom_err")
        #expect(CodexDaemonConfigResult.message(for: resUnknownError).contains("custom_err"))
        #expect(!CodexDaemonConfigResult.showsStopCommand(for: resUnknownError))

        // stopped / not_running -> the server is off; no stop command to copy.
        let stopped = CodexDaemonConfigResult(status: "changed", backup: "config.toml.bak.herdcats-20261020T083528Z", error: nil, daemon: "stopped", updater: "stopped")
        #expect(CodexDaemonConfigResult.message(for: stopped)
            == "Codex's shared server is off. Quit Codex in this pane and start it again so Herdcats can identify the session.")
        #expect(!CodexDaemonConfigResult.showsStopCommand(for: stopped))
        let notRunning = CodexDaemonConfigResult(status: "already_disabled", backup: nil, error: nil, daemon: "not_running", updater: "not_running")
        #expect(CodexDaemonConfigResult.message(for: notRunning)
            == "Codex's shared server is off. Quit Codex in this pane and start it again so Herdcats can identify the session.")
        #expect(!CodexDaemonConfigResult.showsStopCommand(for: notRunning))

        // stop_failed on either process -> still running + copy stop command.
        let daemonStopFailed = CodexDaemonConfigResult(status: "changed", backup: nil, error: nil, daemon: "stop_failed", updater: "stopped")
        #expect(CodexDaemonConfigResult.message(for: daemonStopFailed)
            == "Codex's shared server is still running. Run `codex app-server daemon stop` on this computer, then restart Codex in this pane.")
        #expect(CodexDaemonConfigResult.showsStopCommand(for: daemonStopFailed))
        let updaterStopFailed = CodexDaemonConfigResult(status: "changed", backup: nil, error: nil, daemon: "stopped", updater: "stop_failed")
        #expect(CodexDaemonConfigResult.message(for: updaterStopFailed)
            == "Codex's shared server is still running. Run `codex app-server daemon stop` on this computer, then restart Codex in this pane.")
        #expect(CodexDaemonConfigResult.showsStopCommand(for: updaterStopFailed))

        // unknown on either process -> left running + copy stop command.
        let daemonUnknown = CodexDaemonConfigResult(status: "changed", backup: nil, error: nil, daemon: "unknown", updater: "stopped")
        #expect(CodexDaemonConfigResult.message(for: daemonUnknown)
            == "Herdcats couldn't confirm the running server belongs to Codex, so it left it running. Run `codex app-server daemon stop` on this computer, then restart Codex in this pane.")
        #expect(CodexDaemonConfigResult.showsStopCommand(for: daemonUnknown))
        let updaterUnknown = CodexDaemonConfigResult(status: "changed", backup: nil, error: nil, daemon: "stopped", updater: "unknown")
        #expect(CodexDaemonConfigResult.message(for: updaterUnknown)
            == "Herdcats couldn't confirm the running server belongs to Codex, so it left it running. Run `codex app-server daemon stop` on this computer, then restart Codex in this pane.")
        #expect(CodexDaemonConfigResult.showsStopCommand(for: updaterUnknown))

        // Codex not installed keeps its own copy.
        let notInstalled = CodexDaemonConfigResult(status: "codex_not_installed", backup: nil, error: nil)
        #expect(CodexDaemonConfigResult.message(for: notInstalled).contains("Codex directory was not found"))
        #expect(!CodexDaemonConfigResult.showsStopCommand(for: notInstalled))

        // Missing daemon fields (legacy reply shape) fall through to the stopped copy.
        let missingFields = CodexDaemonConfigResult(status: "changed", backup: nil, error: nil)
        #expect(CodexDaemonConfigResult.message(for: missingFields)
            == "Codex's shared server is off. Quit Codex in this pane and start it again so Herdcats can identify the session.")
        #expect(!CodexDaemonConfigResult.showsStopCommand(for: missingFields))
    }

    @Test func manualFallbackText() {
        let fallback = AgentTimelineView.codexManualFallbackText()
        #expect(fallback.contains("daemon_auto_start = false"))
        #expect(fallback.contains("[features]"))
        #expect(fallback.contains("~/.codex/config.toml"))
        #expect(fallback.contains("codex --no-daemon"))

        #expect(AgentTimelineView.codexManualConfigLine == "daemon_auto_start = false")
    }

    @Test func resultParsingFromSingleLineJSON() throws {
        let rawChanged = "{\"status\":\"changed\",\"backup\":\"config.toml.bak.herdcats-123\"}\n"
        let resChanged = try CodexDaemonConfigResult.parse(rawChanged)
        #expect(resChanged.status == "changed")
        #expect(resChanged.backup == "config.toml.bak.herdcats-123")
        #expect(resChanged.error == nil)
        #expect(resChanged.daemon == nil)
        #expect(resChanged.updater == nil)

        let rawError = "{\"error\":\"config_symlink\"}\n"
        let resError = try CodexDaemonConfigResult.parse(rawError)
        #expect(resError.error == "config_symlink")
        #expect(resError.status == nil)

        let rawStop = "{\"status\":\"changed\",\"backup\":\"config.toml.bak.herdcats-456\",\"daemon\":\"stopped\",\"updater\":\"not_running\"}\n"
        let resStop = try CodexDaemonConfigResult.parse(rawStop)
        #expect(resStop.status == "changed")
        #expect(resStop.daemon == "stopped")
        #expect(resStop.updater == "not_running")
        #expect(resStop.error == nil)
    }

    @Test func handshakeCapabilitiesPersistWhenWatchFailsWithSessionReferenceUnavailable() async throws {
        let wire = CodexTestWire()
        let connection = HerdrConnection()
        let generation = await connection.generation

        let client = AgentBridgeClient(
            write: { await wire.write($0) },
            close: { await wire.close() },
            onEvent: { _ in }
        )

        let task = Task {
            try await client.performHandshakeAndWatch(
                paneId: "p-codex",
                agent: "codex",
                onHandshake: { capabilities, version in
                    await connection.recordAgentBridgeHandshake(capabilities: capabilities, version: version, generation: generation)
                }
            )
        }

        for _ in 0..<500 {
            if await !wire.commands.isEmpty { break }
            try await Task.sleep(for: .milliseconds(2))
        }
        let initId = try await wire.requestId()
        let handshakeJSON = """
        {"id":"\(initId)","result":{"protocol":1,"epoch":"e1","bridge_version":"0.2.2","capabilities":{"claude_timeline":true,"codex_timeline":true,"record_content":true,"codex_daemon_config":true,"codex_daemon_stop":true}}}
        """
        await client.receive(Data((handshakeJSON + "\n").utf8))

        for _ in 0..<500 {
            if await wire.commands.count >= 2 { break }
            try await Task.sleep(for: .milliseconds(2))
        }
        let watchId = try await wire.requestId()
        let watchErrorJSON = """
        {"id":"\(watchId)","error":{"code":"session_reference_unavailable"}}
        """
        await client.receive(Data((watchErrorJSON + "\n").utf8))

        var caughtCode: String?
        do {
            _ = try await task.value
        } catch let AgentBridgeError.unavailable(code) {
            caughtCode = code
        }
        #expect(caughtCode == "session_reference_unavailable")

        let recordedCaps = await connection.bridgeCapabilities(for: generation)
        #expect(recordedCaps?.codexDaemonStop == true)
        let recordedVersion = await connection.bridgeVersion(for: generation)
        #expect(recordedVersion == "0.2.2")

        // A running shared server gets the in-app stop; an already-off one does not.
        #expect(AgentTimelineView.codexNoSessionAction(
            agent: "codex", code: caughtCode ?? "", route: nil, capabilities: recordedCaps, codexDaemon: "running"
        ) == .turnOffServer)
        #expect(AgentTimelineView.codexNoSessionAction(
            agent: "codex", code: caughtCode ?? "", route: nil, capabilities: recordedCaps, codexDaemon: "not_running"
        ) == .waitForSession)

        await connection.closeAllAgentBridges()
        let clearedCaps = await connection.bridgeCapabilities(for: generation)
        #expect(clearedCaps == nil)
    }

    @Test func stopRunningUsesItsOwnScriptAndTimeout() {
        let stopScript = HerdrConnection.codexDisableDaemonStopScript
        #expect(stopScript.contains("codex-config disable-daemon --stop-running"))
        // The stop path shares the helper lookup and missing-helper fallback.
        for path in [
            "$HOME/.local/bin/herdcats-bridge",
            "$HOME/.cargo/bin/herdcats-bridge",
            "/opt/homebrew/bin/herdcats-bridge",
            "/usr/local/bin/herdcats-bridge"
        ] {
            #expect(stopScript.contains(path))
        }
        #expect(stopScript.contains("command -v herdcats-bridge"))
        #expect(stopScript.contains("{\"error\":\"helper_missing\"}"))
        #expect(HerdrConnection.codexDisableDaemonStopCommand.starts(with: "exec /bin/sh -c "))

        // The stop pass can take ~60 s on the bridge side; the app bounds it at 90 s.
        #expect(HerdrConnection.codexDisableDaemonTimeout(stopRunning: true) == 90)
        #expect(HerdrConnection.codexDisableDaemonTimeout(stopRunning: false) == 15)

        // The config-only command never stops a running server.
        #expect(!HerdrConnection.codexDisableDaemonScript.contains("--stop-running"))
        #expect(HerdrConnection.codexDisableDaemonCommand.starts(with: "exec /bin/sh -c "))
    }

    @Test func bridgeServeScriptKeepsItsOwnShape() {
        let bridgeScript = HerdrConnection.agentBridgeScript
        #expect(bridgeScript.contains("exec \"$B\" serve --stdio --socket \"$S\""))
        #expect(bridgeScript.contains("IFS= read -r HB_REQUEST || exit 127"))
        #expect(bridgeScript.contains("{\"error\":{\"code\":\"helper_missing\"}}"))
        #expect(HerdrConnection.agentBridgeCommand.starts(with: "exec /bin/sh -c "))
    }

    @Test func stopResultIsRetiredAtEpisodeBoundariesAndContradictions() {
        // The result survives only within the unavailable episode it answered.
        #expect(AgentTimelineView.codexDaemonResultIsCurrent(phase: .unavailable("session_reference_unavailable")))
        #expect(AgentTimelineView.codexDaemonResultIsCurrent(phase: .unavailable("ambiguous_session")))
        #expect(!AgentTimelineView.codexDaemonResultIsCurrent(phase: .ready))
        #expect(!AgentTimelineView.codexDaemonResultIsCurrent(phase: .loading))
        #expect(!AgentTimelineView.codexDaemonResultIsCurrent(phase: .disconnected))

        // A successful stop stays on screen through the bridge's not_running
        // re-emission (and when no daemon value is reported)...
        #expect(!AgentTimelineView.codexDaemonResultIsStale(errorOccurred: false, showsStopCommand: false, codexDaemon: "not_running"))
        #expect(!AgentTimelineView.codexDaemonResultIsStale(errorOccurred: false, showsStopCommand: false, codexDaemon: nil))
        // ...but the server coming back (or being unverifiable) retires it, so
        // the Turn off button returns instead of stale "server is off" copy.
        #expect(AgentTimelineView.codexDaemonResultIsStale(errorOccurred: false, showsStopCommand: false, codexDaemon: "running"))
        #expect(AgentTimelineView.codexDaemonResultIsStale(errorOccurred: false, showsStopCommand: false, codexDaemon: "unknown"))

        // "Still running" results (stop_failed, unverified, or a config/
        // transport error) stay until a not_running report retires them.
        for recorded in [(errorOccurred: true, showsStopCommand: false), (errorOccurred: false, showsStopCommand: true)] {
            #expect(!AgentTimelineView.codexDaemonResultIsStale(errorOccurred: recorded.errorOccurred, showsStopCommand: recorded.showsStopCommand, codexDaemon: "running"))
            #expect(!AgentTimelineView.codexDaemonResultIsStale(errorOccurred: recorded.errorOccurred, showsStopCommand: recorded.showsStopCommand, codexDaemon: "unknown"))
            #expect(!AgentTimelineView.codexDaemonResultIsStale(errorOccurred: recorded.errorOccurred, showsStopCommand: recorded.showsStopCommand, codexDaemon: nil))
            #expect(AgentTimelineView.codexDaemonResultIsStale(errorOccurred: recorded.errorOccurred, showsStopCommand: recorded.showsStopCommand, codexDaemon: "not_running"))
        }
    }
}

private actor CodexTestWire {
    var commands: [Data] = []
    var closes = 0
    func write(_ bytes: Data) { commands.append(bytes) }
    func close() { closes += 1 }
    func requestId() throws -> String {
        let obj = try JSONSerialization.jsonObject(with: commands.last!) as! [String: Any]
        return obj["id"] as! String
    }
}

