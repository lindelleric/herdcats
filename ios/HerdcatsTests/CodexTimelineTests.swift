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

        // Old helper (0.1.4) only has claude
        let oldHelperReport = report(agents: ["claude"], version: "0.1.4")
        let probeForClaudeOnOld = try HostHelperProbe(output: oldHelperReport, agent: "claude")
        #expect(probeForClaudeOnOld == .present(version: "0.1.4", managed: true, compatible: true))

        let probeForCodexOnOld = try HostHelperProbe(output: oldHelperReport, agent: "codex")
        #expect(probeForCodexOnOld == .present(version: "0.1.4", managed: true, compatible: false))

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
        // Verify explanation texts for Codex
        #expect(view.explanation("helper_update_required") == "Update the Herdcats helper to see Codex conversations.")
        #expect(view.explanation("unsupported_agent") == "Update the Herdcats helper to see Codex conversations.")
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
}
