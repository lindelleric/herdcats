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
        // When Codex is supported by pinned installer (0.2.0 pinned release with HostHelperSetupPlan.agents == ["claude", "codex"])
        #expect(view.explanation("helper_update_required") == "Update the Herdcats helper to see Codex conversations.")
        #expect(view.explanation("unsupported_agent") == "Update the Herdcats helper to see Codex conversations.")
        #expect(view.explanation("helper_missing") == "Install the small host helper to see this session as native messages. Terminal is available now.")

        // Explicitly test both ways with injected supportedAgents:
        // 1. Without Codex in supportedAgents
        let unsupportedAgents: Set<String> = ["claude"]
        #expect(AgentTimelineView.explanation("helper_update_required", agent: "codex", supportedAgents: unsupportedAgents) == "This version of Herdcats can't install a Codex-capable helper yet.")
        #expect(AgentTimelineView.explanation("unsupported_agent", agent: "codex", supportedAgents: unsupportedAgents) == "This version of Herdcats can't install a Codex-capable helper yet.")
        #expect(AgentTimelineView.explanation("helper_missing", agent: "codex", supportedAgents: unsupportedAgents) == "This version of Herdcats can't install a Codex-capable helper yet.")

        // 2. With Codex in supportedAgents
        let supportedWithCodex: Set<String> = ["claude", "codex"]
        #expect(AgentTimelineView.explanation("helper_update_required", agent: "codex", supportedAgents: supportedWithCodex) == "Update the Herdcats helper to see Codex conversations.")
        #expect(AgentTimelineView.explanation("unsupported_agent", agent: "codex", supportedAgents: supportedWithCodex) == "Update the Herdcats helper to see Codex conversations.")
        #expect(AgentTimelineView.explanation("helper_missing", agent: "codex", supportedAgents: supportedWithCodex) == "Install the small host helper to see this session as native messages. Terminal is available now.")

        // 3. Claude is unaffected
        #expect(AgentTimelineView.explanation("helper_update_required", agent: "claude", supportedAgents: unsupportedAgents) == "Update the host helper to display session images.")
        #expect(AgentTimelineView.explanation("unsupported_agent", agent: "claude", supportedAgents: unsupportedAgents) == "Timeline is not available for this agent yet. You can still send messages below or use Terminal.")
        #expect(AgentTimelineView.explanation("helper_missing", agent: "claude", supportedAgents: unsupportedAgents) == "Install the small host helper to see this session as native messages. Terminal is available now.")
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
        let explanation = view.explanation("session_reference_unavailable")
        #expect(explanation.contains("Codex’s shared daemon") || explanation.contains("Codex's shared daemon"))
        #expect(explanation.contains("codex --no-daemon"))
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

@Suite("Codex daemon configuration gating and messaging")
struct CodexDaemonConfigTests {
    @Test func buttonGatingCapability() {
        let capsEnabled = AgentBridgeCapabilities(
            claudeTimeline: true, codexTimeline: true, codexDaemonConfig: true
        )
        let capsDisabled = AgentBridgeCapabilities(
            claudeTimeline: true, codexTimeline: true, codexDaemonConfig: false
        )
        let capsNil = AgentBridgeCapabilities(
            claudeTimeline: true, codexTimeline: true, codexDaemonConfig: nil
        )

        // Enabled capability + direct host (route == nil) + codex + no-session code -> true
        #expect(
            AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "session_reference_unavailable",
                route: nil,
                capabilities: capsEnabled
            )
        )
        #expect(
            AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "ambiguous_session",
                route: nil,
                capabilities: capsEnabled
            )
        )
        #expect(
            AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "invalid_session_reference",
                route: nil,
                capabilities: capsEnabled
            )
        )

        // Disabled capability -> false
        #expect(
            !AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "session_reference_unavailable",
                route: nil,
                capabilities: capsDisabled
            )
        )

        // Nil capability (old helper without codex_daemon_config) -> false
        #expect(
            !AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "session_reference_unavailable",
                route: nil,
                capabilities: capsNil
            )
        )
        #expect(
            !AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "session_reference_unavailable",
                route: nil,
                capabilities: nil
            )
        )
    }

    @Test func buttonGatingRoutedHost() {
        let capsEnabled = AgentBridgeCapabilities(
            claudeTimeline: true, codexTimeline: true, codexDaemonConfig: true
        )
        let gateway = HerdrConnection()
        let machine = HerdrMachine(id: "child", label: "Child", target: "u@child", session: "default", enabled: true)
        let route = HerdrMachineRoute(gateway: gateway, machine: machine, generation: ConnectionGeneration(rawValue: 1))

        // Routed host (route != nil) -> false even with capability enabled
        #expect(
            !AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "session_reference_unavailable",
                route: route,
                capabilities: capsEnabled
            )
        )
    }

    @Test func buttonGatingAgentAndCode() {
        let capsEnabled = AgentBridgeCapabilities(
            claudeTimeline: true, codexTimeline: true, codexDaemonConfig: true
        )

        // Claude agent -> false
        #expect(
            !AgentTimelineView.canConfigureCodexDaemon(
                agent: "claude",
                code: "session_reference_unavailable",
                route: nil,
                capabilities: capsEnabled
            )
        )

        // Unrelated unavailable code -> false
        #expect(
            !AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "session_cwd_mismatch",
                route: nil,
                capabilities: capsEnabled
            )
        )
        #expect(
            !AgentTimelineView.canConfigureCodexDaemon(
                agent: "codex",
                code: "transcript_root_unavailable",
                route: nil,
                capabilities: capsEnabled
            )
        )
    }

    @Test func statusAndErrorMessageMapping() {
        let expectedGuidance = "Quit Codex in every pane, run `codex app-server daemon stop` on this computer (this ends all Codex sessions that use the shared daemon), then start Codex again in this pane. Or start Codex with `codex --no-daemon` now."
        #expect(CodexDaemonConfigResult.restartGuidance == expectedGuidance)
        #expect(AgentTimelineView.codexDaemonStopCommand == "codex app-server daemon stop")

        // Changed with backup
        let resChangedBackup = CodexDaemonConfigResult(
            status: "changed", backup: "config.toml.bak.herdcats-20261017T083528Z", error: nil
        )
        let msgChangedBackup = CodexDaemonConfigResult.message(for: resChangedBackup)
        #expect(msgChangedBackup == expectedGuidance)

        // Changed without backup
        let resChangedNoBackup = CodexDaemonConfigResult(
            status: "changed", backup: nil, error: nil
        )
        let msgChangedNoBackup = CodexDaemonConfigResult.message(for: resChangedNoBackup)
        #expect(msgChangedNoBackup == expectedGuidance)

        // Already disabled
        let resAlready = CodexDaemonConfigResult(
            status: "already_disabled", backup: nil, error: nil
        )
        let msgAlready = CodexDaemonConfigResult.message(for: resAlready)
        #expect(msgAlready == expectedGuidance)

        // Codex not installed
        let resNotInstalled = CodexDaemonConfigResult(
            status: "codex_not_installed", backup: nil, error: nil
        )
        let msgNotInstalled = CodexDaemonConfigResult.message(for: resNotInstalled)
        #expect(msgNotInstalled.contains("Codex directory was not found"))

        // Errors
        let resInvalid = CodexDaemonConfigResult(status: nil, backup: nil, error: "config_invalid")
        #expect(CodexDaemonConfigResult.message(for: resInvalid).contains("invalid TOML"))

        let resNotTable = CodexDaemonConfigResult(status: nil, backup: nil, error: "features_not_table")
        #expect(CodexDaemonConfigResult.message(for: resNotTable).contains("not a table"))

        let resSymlink = CodexDaemonConfigResult(status: nil, backup: nil, error: "config_symlink")
        #expect(CodexDaemonConfigResult.message(for: resSymlink).contains("symbolic link"))

        let resWriteFailed = CodexDaemonConfigResult(status: nil, backup: nil, error: "write_failed")
        #expect(CodexDaemonConfigResult.message(for: resWriteFailed).contains("failed to write"))

        let resVerification = CodexDaemonConfigResult(status: nil, backup: nil, error: "verification_failed")
        #expect(CodexDaemonConfigResult.message(for: resVerification).contains("verification failed"))

        let resUnknown = CodexDaemonConfigResult(status: nil, backup: nil, error: "custom_err")
        #expect(CodexDaemonConfigResult.message(for: resUnknown).contains("custom_err"))
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

        let rawError = "{\"error\":\"config_symlink\"}\n"
        let resError = try CodexDaemonConfigResult.parse(rawError)
        #expect(resError.error == "config_symlink")
        #expect(resError.status == nil)
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
                onHandshake: { capabilities in
                    await connection.recordAgentBridgeCapabilities(capabilities, generation: generation)
                }
            )
        }

        for _ in 0..<500 {
            if await !wire.commands.isEmpty { break }
            try await Task.sleep(for: .milliseconds(2))
        }
        let initId = try await wire.requestId()
        let handshakeJSON = """
        {"id":"\(initId)","result":{"protocol":1,"epoch":"e1","bridge_version":"0.2.0","capabilities":{"claude_timeline":true,"codex_timeline":true,"record_content":true,"codex_daemon_config":true}}}
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
        #expect(recordedCaps?.codexDaemonConfig == true)

        let canTurnOff = AgentTimelineView.canConfigureCodexDaemon(
            agent: "codex",
            code: caughtCode ?? "",
            route: nil,
            capabilities: recordedCaps
        )
        #expect(canTurnOff == true)

        await connection.closeAllAgentBridges()
        let clearedCaps = await connection.bridgeCapabilities(for: generation)
        #expect(clearedCaps == nil)
    }

    @Test func sharedHelperLookupCommand() {
        let bridgeScript = HerdrConnection.agentBridgeScript
        let disableDaemonScript = HerdrConnection.codexDisableDaemonScript

        // Both commands share the exact binary search paths
        for path in [
            "$HOME/.local/bin/herdcats-bridge",
            "$HOME/.cargo/bin/herdcats-bridge",
            "/opt/homebrew/bin/herdcats-bridge",
            "/usr/local/bin/herdcats-bridge"
        ] {
            #expect(bridgeScript.contains(path))
            #expect(disableDaemonScript.contains(path))
        }

        #expect(bridgeScript.contains("command -v herdcats-bridge"))
        #expect(disableDaemonScript.contains("command -v herdcats-bridge"))

        // Bridge command serves stdio over herdr socket and drains stdin on missing
        #expect(bridgeScript.contains("exec \"$B\" serve --stdio --socket \"$S\""))
        #expect(bridgeScript.contains("IFS= read -r HB_REQUEST || exit 127"))
        #expect(bridgeScript.contains("{\"error\":{\"code\":\"helper_missing\"}}"))

        // Disable daemon command executes codex-config disable-daemon with JSON fallback
        #expect(disableDaemonScript.contains("exec \"$B\" codex-config disable-daemon"))
        #expect(disableDaemonScript.contains("{\"error\":\"helper_missing\"}"))

        // Both commands are wrapped safely for /bin/sh
        #expect(HerdrConnection.agentBridgeCommand.starts(with: "exec /bin/sh -c "))
        #expect(HerdrConnection.codexDisableDaemonCommand.starts(with: "exec /bin/sh -c "))
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

