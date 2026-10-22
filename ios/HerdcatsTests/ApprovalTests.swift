import Foundation
import Testing

@testable import Herdcats

@Suite("Approval protocol")
@MainActor struct ApprovalTests {
    func event(_ name: String, seq: Int, agent: String = "claude", generation: Int = 1, requestId: String = "request", extra: [String: Any] = [:]) throws -> AgentBridgeFrame {
        var data: [String: Any] = [
            "identity": ["pane_id": "w1:p1", "terminal_id": "t1", "agent": agent, "native_session_id": "session-a"],
            "generation": generation, "request_id": requestId, "kind": "approval", "agent": agent,
            "tool_name": "Bash", "command": "echo hello", "title": "Allow?",
            "description": agent == "claude" ? "A description" as Any : NSNull(),
            "justification": agent == "codex" ? "A justification" as Any : NSNull(),
            "options": [["number": 1, "label_lines": ["Yes", "once"], "key": agent == "claude" ? "1" : "y"]],
        ]
        data.merge(extra) { _, new in new }
        return try AgentBridgeFrame.decode(JSONSerialization.data(withJSONObject: ["event": name, "epoch": "epoch-a", "seq": seq, "data": data]))
    }
    @Test func pendingAndResolvedPreserveRowsAndScope() throws {
        for agent in ["claude", "codex"] {
            let owner = UUID(), state = AgentTimelineState()
            state.begin(owner: owner, paneId: "w1:p1")
            try state.consume(event("timeline.snapshot", seq: 1, agent: agent, extra: ["entries": []]), owner: owner)
            try state.consume(event("interaction.pending", seq: 2, agent: agent), owner: owner)
            #expect(state.pendingApproval?.options.first?.labelLines == ["Yes", "once"])
            #expect(state.pendingApproval?.agent == agent)
            try state.consume(event("interaction.resolved", seq: 3, agent: agent, requestId: "other"), owner: owner)
            #expect(state.pendingApproval != nil)
            try state.consume(event("interaction.resolved", seq: 4, agent: agent), owner: owner)
            #expect(state.pendingApproval == nil)
            try state.consume(event("interaction.pending", seq: 5, agent: agent), owner: owner)
            try state.consume(event("timeline.snapshot", seq: 6, agent: agent, extra: ["entries": []]), owner: owner)
            #expect(state.pendingApproval == nil)
        }
    }
    @Test func rejectsMismatchedScopeAndInvalidBounds() throws {
        let invalid: [[String: Any]] = [
            ["generation": 2], ["agent": "codex"],
            ["identity": ["pane_id": "w1:p1", "terminal_id": "other", "agent": "claude", "native_session_id": "session-a"]],
            ["request_id": ""], ["request_id": String(repeating: "猫", count: 89)],
            ["command": String(repeating: "x", count: 65537)], ["selected": 1],
            ["options": []],
            ["options": (1...17).map { ["number": $0, "label_lines": ["Yes"], "key": "y"] }],
            ["options": [["number": 1, "label_lines": Array(repeating: "Yes", count: 9), "key": "y"]]],
            ["options": [["number": 1, "label_lines": [String(repeating: "x", count: 1025)], "key": "y"]]],
            ["options": [["number": 1, "label_lines": ["Yes"], "key": "enter"]]],
        ]
        for extra in invalid {
            let owner = UUID(), state = AgentTimelineState()
            state.begin(owner: owner, paneId: "w1:p1")
            try state.consume(event("timeline.snapshot", seq: 1, extra: ["entries": []]), owner: owner)
            #expect(throws: AgentBridgeError.self) { try state.consume(event("interaction.pending", seq: 2, extra: extra), owner: owner) }
            #expect(state.pendingApproval == nil)
        }
    }
    @Test func unavailableAndEndClearApprovalAndSequenceGapsFail() throws {
        for end in [true, false] {
            let owner = UUID(), state = AgentTimelineState()
            state.begin(owner: owner, paneId: "w1:p1")
            try state.consume(event("timeline.snapshot", seq: 1, extra: ["entries": []]), owner: owner)
            try state.consume(event("interaction.pending", seq: 2), owner: owner)
            #expect(throws: AgentBridgeError.sequenceGap) { try state.consume(event("interaction.resolved", seq: 4), owner: owner) }
            #expect(state.pendingApproval != nil)
            if end { state.end(owner: owner, error: .closed) }
            else { try state.consume(event("timeline.unavailable", seq: 3, extra: ["pane_id": "w1:p1"]), owner: owner) }
            #expect(state.pendingApproval == nil)
        }
    }
    @Test func oldScopeResolutionAfterTimelineResetIsIgnored() throws {
        for reset in ["timeline.snapshot", "timeline.unavailable"] {
            let owner = UUID(), state = AgentTimelineState()
            state.begin(owner: owner, paneId: "w1:p1")
            try state.consume(event("timeline.snapshot", seq: 1, extra: ["entries": []]), owner: owner)
            try state.consume(event("interaction.pending", seq: 2), owner: owner)
            try state.consume(event(reset, seq: 3, generation: 2, extra: ["entries": [], "pane_id": "w1:p1"]), owner: owner)
            try state.consume(event("interaction.resolved", seq: 4), owner: owner)
            #expect(state.generation == 2)
            #expect(state.phase == (reset == "timeline.snapshot" ? .ready : .unavailable("timeline_unavailable")))
            #expect(state.pendingApproval == nil)
            if reset == "timeline.snapshot" {
                try state.consume(event("interaction.pending", seq: 5, generation: 2), owner: owner)
                #expect(state.pendingApproval?.generation == 2)
            }
        }
    }
    @Test func resolutionRequiresMatchingPendingIdentityAndGeneration() throws {
        let otherIdentity = ["pane_id": "w1:p1", "terminal_id": "other", "agent": "claude", "native_session_id": "session-a"]
        for extra: [String: Any] in [["identity": otherIdentity], ["generation": 2]] {
            let owner = UUID(), state = AgentTimelineState()
            state.begin(owner: owner, paneId: "w1:p1")
            try state.consume(event("timeline.snapshot", seq: 1, extra: ["entries": []]), owner: owner)
            try state.consume(event("interaction.pending", seq: 2), owner: owner)
            let pending = state.pendingApproval
            try state.consume(event("interaction.resolved", seq: 3, extra: extra), owner: owner)
            #expect(state.pendingApproval == pending)
            try state.consume(event("interaction.resolved", seq: 4), owner: owner)
            #expect(state.pendingApproval == nil)
        }
    }
    @Test func resolutionValidatesRequestIdEvenWithoutReadyTimeline() throws {
        for requestId in ["", String(repeating: "x", count: 265)] {
            let owner = UUID(), state = AgentTimelineState()
            state.begin(owner: owner, paneId: "w1:p1")
            #expect(throws: AgentBridgeError.invalidFrame) {
                try state.consume(event("interaction.resolved", seq: 1, requestId: requestId), owner: owner)
            }
        }
    }
    @Test func hooksParsingAndScripts() throws {
        for status in ["installed", "missing", "stale", "needs_trust", "disabled"] {
            let result = try AgentHooksResult.parse("{\"status\":\"\(status)\",\"spool_confirmed\":true,\"backup\":\"copy\"}")
            #expect(result.status?.rawValue == status)
            #expect(result.spoolConfirmed == true)
            #expect(result.backup == "copy")
        }
        #expect(try AgentHooksResult.parse("{\"status\":\"future\"}").status == nil)
        #expect(try AgentHooksResult.parse("{\"error\":\"helper_missing\"}").error == "helper_missing")
        for agent in ApprovalAgent.allCases {
            for operation in AgentHooksOperation.allCases {
                #expect(HerdrConnection.hooksScript(agent: agent, operation: operation).contains("exec \"$B\" hooks \(operation.rawValue) --agent \(agent.rawValue)"))
            }
        }
    }
    @Test func responseParametersAndCapabilities() throws {
        let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
        let params = AgentBridgeParams(requestId: "r", option: 2, labelLines: ["Yes", "session"])
        let json = try JSONSerialization.jsonObject(with: encoder.encode(params)) as? NSDictionary
        #expect(json == ["request_id": "r", "option": 2, "label_lines": ["Yes", "session"]] as NSDictionary)
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        #expect(try decoder.decode(AgentBridgeCapabilities.self, from: Data("{\"claude_timeline\":true}".utf8)).approvals == nil)
        #expect(try decoder.decode(AgentBridgeCapabilities.self, from: Data("{\"claude_timeline\":true,\"approvals\":true}".utf8)).approvals == true)
        for code in ["stale", "mismatch", "unsupported_capability", "not_watching", "timeline_unavailable"] {
            #expect(AgentBridgeError.unavailable(code).code == code)
        }
    }
}
