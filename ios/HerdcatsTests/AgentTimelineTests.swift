import Foundation
import Testing

@testable import Herdcats

@Suite("Agent bridge byte framing")
struct AgentBridgeFramingTests {
    @Test func splitUnicodeAndMultipleFramesDecodeExactlyOnce() throws {
        let bytes = Data("{\"id\":\"猫🐈\"}\n{\"id\":\"next\"}\n".utf8)
        for split in 0..<bytes.count {
            var parser = AgentBridgeFramer()
            let frames =
                try parser.append(Data(bytes.prefix(split))) + parser.append(Data(bytes.dropFirst(split)))
            #expect(frames.map(\.id) == ["猫🐈", "next"])
            try parser.finish()
        }
    }
    @Test func oversizedInvalidAndIncompleteFramesFailClosed() throws {
        var parser = AgentBridgeFramer(limit: 32)
        #expect(throws: AgentBridgeError.self) { try parser.append(Data(repeating: 65, count: 33)) }
        parser = AgentBridgeFramer()
        #expect(throws: AgentBridgeError.self) { try parser.append(Data("{bad}\n".utf8)) }
        parser = AgentBridgeFramer()
        _ = try parser.append(Data("{\"id\":\"partial\"}".utf8))
        #expect(throws: AgentBridgeError.self) { try parser.finish() }
    }
}

@Suite("Agent timeline identity and history")
@MainActor struct AgentTimelineTests {
    let owner = UUID()
    func frame(
        _ event: String, seq: Int = 1, generation: Int = 1, session: String = "session-a",
        entries: [String] = ["a"], hasMore: Bool = false
    ) throws -> AgentBridgeFrame {
        let rows = entries.map {
            ["id": "message:\($0)", "kind": "message", "text": $0, "role": "assistant", "truncated": false]
                as [String: Any]
        }
        return try AgentBridgeFrame.decode(
            JSONSerialization.data(withJSONObject: [
                "event": event, "epoch": "epoch-a", "seq": seq,
                "data": [
                    "identity": [
                        "pane_id": "w1:p1", "terminal_id": "t1", "agent": "claude",
                        "native_session_id": session,
                    ], "pane_id": "w1:p1", "generation": generation, "entries": rows, "has_more": hasMore,
                    "before": rows.first?["id"] as Any, "code": "source_changing",
                ],
            ]))
    }
    func store() -> AgentTimelineState {
        let s = AgentTimelineState()
        s.begin(owner: owner, paneId: "w1:p1")
        return s
    }
    @Test func snapshotUpdatesAndDuplicateUpsertsKeepStableOrder() throws {
        let s = store()
        try s.consume(frame("timeline.snapshot"), owner: owner)
        try s.consume(frame("timeline.updated", seq: 2, entries: ["a", "b"]), owner: owner)
        #expect(s.entries.map(\.text) == ["a", "b"])
        #expect(s.phase == .ready)
    }
    @Test func occupantResetAndUnavailableClearOldContent() throws {
        let s = store()
        try s.consume(frame("timeline.snapshot"), owner: owner)
        try s.consume(
            frame("timeline.snapshot", seq: 2, generation: 2, session: "session-b", entries: ["b"]),
            owner: owner)
        #expect(s.entries.map(\.text) == ["b"])
        try s.consume(frame("timeline.unavailable", seq: 3, generation: 3), owner: owner)
        #expect(s.entries.isEmpty)
        #expect(s.identity == nil)
    }
    @Test func wrongPaneSequenceGapAndChangedIdentityFailClosed() throws {
        let s = store()
        try s.consume(frame("timeline.snapshot"), owner: owner)
        #expect(throws: AgentBridgeError.self) {
            try s.consume(frame("timeline.updated", seq: 3), owner: owner)
        }
        let t = store()
        try t.consume(frame("timeline.snapshot"), owner: owner)
        #expect(throws: AgentBridgeError.self) {
            try t.consume(frame("timeline.updated", seq: 2, session: "other"), owner: owner)
        }
        let wrong = AgentTimelineState()
        wrong.begin(owner: owner, paneId: "different")
        #expect(throws: AgentBridgeError.self) { try wrong.consume(frame("timeline.snapshot"), owner: owner) }
    }
    @Test func evictionsPreserveLoadedHistoryAndDoNotResurrectFromDelayedPage() throws {
        let s = store()
        try s.consume(frame("timeline.snapshot", entries: ["a", "b", "c"], hasMore: true), owner: owner)
        let delayed = try frame("timeline.snapshot", entries: ["a", "b"]).data!
        let update = try AgentBridgeFrame.decode(Data("{\"event\":\"timeline.updated\",\"epoch\":\"epoch-a\",\"seq\":2,\"data\":{\"identity\":{\"pane_id\":\"w1:p1\",\"terminal_id\":\"t1\",\"agent\":\"claude\",\"native_session_id\":\"session-a\"},\"generation\":1,\"entries\":[],\"evicted_ids\":[\"message:a\"],\"unknown_records\":2}}".utf8))
        try s.consume(update, owner: owner)
        #expect(s.entries.map(\.text) == ["b", "c"])
        #expect(s.generation == 1)
        #expect(s.unknownRecords == 2)
        try s.mergeHistory(delayed, owner: owner, generation: 1)
        #expect(s.entries.map(\.text) == ["b", "c"])
    }
    @Test func delayedEventsAndFailuresCannotReplaceNewOwner() throws {
        let s = store()
        let next = UUID()
        s.begin(owner: next, paneId: "w1:p2")
        try s.consume(frame("timeline.snapshot"), owner: owner)
        s.end(owner: owner, error: .closed)
        #expect(s.entries.isEmpty)
        #expect(s.phase == .loading)
    }
    @Test func historyIsGenerationBoundAndOverlapsDoNotOverwriteNewerOutput() throws {
        let s = store()
        try s.consume(frame("timeline.snapshot", entries: ["b"], hasMore: true), owner: owner)
        let old = try frame("timeline.snapshot", entries: ["a", "b"]).data!
        try s.mergeHistory(old, owner: owner, generation: 1)
        #expect(s.entries.map(\.text) == ["a", "b"])
        try s.consume(frame("timeline.snapshot", seq: 2, generation: 2, entries: ["new"]), owner: owner)
        try s.mergeHistory(old, owner: owner, generation: 1)
        #expect(s.entries.map(\.text) == ["new"])
    }
}
