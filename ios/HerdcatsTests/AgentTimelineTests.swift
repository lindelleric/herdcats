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
    @Test func switchingPanesDoesNotShowPreviousPaneEntries() throws {
        // Pane 1 receives a snapshot and shows its entries
        let pane1 = store()
        try pane1.consume(frame("timeline.snapshot", entries: ["pane1-first", "pane1-second"]), owner: owner)
        #expect(pane1.entries.map(\.text) == ["pane1-first", "pane1-second"])
        #expect(pane1.phase == .ready)

        // Switching to Pane 2 uses a distinct pane session with its own fresh AgentTimelineState
        // and its own owner. The previous pane's entries are not retained or visible in the new pane.
        let pane2Owner = UUID()
        let pane2 = AgentTimelineState()
        pane2.begin(owner: pane2Owner, paneId: "w1:p2")
        #expect(pane2.entries.isEmpty)
        #expect(pane2.phase == .loading)
        #expect(pane2.identity == nil)

        // TimelineBridgeUpgrade also resets bridgeVersion when switching panes
        let bridgeUpgrade = TimelineBridgeUpgrade(bridgeVersion: "0.2.0")
        #expect(bridgeUpgrade.bridgeVersion == "0.2.0")
        bridgeUpgrade.bridgeVersion = nil
        #expect(bridgeUpgrade.bridgeVersion == nil)
        #expect(!bridgeUpgrade.requested)
        bridgeUpgrade.requestUpgrade()
        #expect(bridgeUpgrade.requested)
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
    @Test func delayedSignalPreservesHistoryAndClearsOnRecovery() throws {
        let s = store()
        try s.consume(frame("timeline.snapshot", entries: ["msg1", "msg2"]), owner: owner)
        #expect(!s.delayed)
        #expect(s.entries.map(\.text) == ["msg1", "msg2"])

        let delayedUpdate = try AgentBridgeFrame.decode(Data("{\"event\":\"timeline.updated\",\"epoch\":\"epoch-a\",\"seq\":2,\"data\":{\"identity\":{\"pane_id\":\"w1:p1\",\"terminal_id\":\"t1\",\"agent\":\"claude\",\"native_session_id\":\"session-a\"},\"generation\":1,\"entries\":[],\"delayed\":true}}".utf8))
        try s.consume(delayedUpdate, owner: owner)
        #expect(s.delayed)
        #expect(s.entries.map(\.text) == ["msg1", "msg2"])

        let recoveredUpdate = try AgentBridgeFrame.decode(Data("{\"event\":\"timeline.updated\",\"epoch\":\"epoch-a\",\"seq\":3,\"data\":{\"identity\":{\"pane_id\":\"w1:p1\",\"terminal_id\":\"t1\",\"agent\":\"claude\",\"native_session_id\":\"session-a\"},\"generation\":1,\"entries\":[],\"delayed\":false}}".utf8))
        try s.consume(recoveredUpdate, owner: owner)
        #expect(!s.delayed)
        #expect(s.entries.map(\.text) == ["msg1", "msg2"])
    }
}

@Suite("Timeline agent availability")
struct TimelineAgentAvailabilityTests {
    @Test func onlyImplementedAdaptersExposeTimeline() {
        #expect(TimelineAgentAvailability.supports(agent: "claude"))
        #expect(TimelineAgentAvailability.supports(agent: "codex"))
        for agent in [nil, "", "chatgpt", "cursor", "agy", "unknown"] as [String?] {
            #expect(!TimelineAgentAvailability.supports(agent: agent))
        }
    }
}

@Suite("Timeline preserved content")
@MainActor struct TimelinePreservedContentTests {
    @Test func sourceReferencesSurviveDecodingAndImageAndRecordBubblesAreAccepted() throws {
        for kind in ["image", "record"] {
            let row: [String:Any] = ["id":"content:0:0", "kind":kind,"text":"Image or unfamiliar record","truncated":false,
                                     "content_ref":"source:0:10","record_refs":["source:0:20"]]
            let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
            let entry = try decoder.decode(AgentTimelineEntry.self,from:JSONSerialization.data(withJSONObject:row))
            let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase
            let roundTrip = try JSONSerialization.jsonObject(with:encoder.encode(entry)) as! [String:Any]
            #expect(roundTrip["content_ref"] as? String == "source:0:10")
            let owner = UUID(); let state = AgentTimelineState(); state.begin(owner:owner,paneId:"p1")
            let frame = try AgentBridgeFrame.decode(JSONSerialization.data(withJSONObject:["event":"timeline.snapshot","epoch":"e","seq":1,
                "data":["identity":["pane_id":"p1","terminal_id":"t1","agent":"claude","native_session_id":"s"],"generation":1,"entries":[row]]]))
            try state.consume(frame,owner:owner)
            #expect(state.phase == .ready)
            #expect(state.entries.count == 1)
        }
    }
}

@Suite("Lossless Timeline source paging")
struct TimelineSourcePagingTests {
    @Test func pagesPreserveEveryUnicodeByteWithoutSplittingScalars() {
        let original = Data(String(repeating:"猫🐈é\n",count:15000).utf8)
        let pages = TimelineSourcePages.ranges(original)
        #expect(pages.count > 1)
        var decoded = ""
        for range in pages {
            #expect(range.count <= 32768)
            decoded += String(decoding:original[range],as:UTF8.self)
        }
        #expect(Data(decoded.utf8) == original)
        #expect(TimelineContentReference.length("source:0:10") == 10)
        for invalid in ["/etc/passwd","source:-1:3","source:0:0","source:67108864:1","source:0:4:5"] {
            #expect(TimelineContentReference.length(invalid) == nil)
        }
    }
}

@Suite("Conversation activity summaries")
struct TimelineActivityTests {
    private func entry(_ id: String, kind: String = "tool", name: String = "Bash",
                       status: String = "completed", details: String? = nil) -> AgentTimelineEntry {
        AgentTimelineEntry(id: id, kind: kind, text: "", role: nil, toolName: name,
                           status: status, details: details, truncated: false)
    }
    @Test func groupsAdjacentToolsWithoutCrossingMessages() {
        let items = TimelineConversationItem.group([
            entry("a"), entry("b", name: "Read"), entry("hidden", kind: "record"),
            entry("message", kind: "message"), entry("c"), entry("image", kind: "image"), entry("d")
        ])
        #expect(items.map(\.id) == ["a", "message", "c", "image", "d"])
        #expect(items[0].entries.map(\.id) == ["a", "b"])
        #expect(items[0].isActivity)
        #expect(!items[1].isActivity)
    }
    @Test func codexTimelinesKeepFallbackRecords() {
        let entries = [entry("a"), entry("rec", kind: "record"), entry("b", kind: "message")]
        #expect(TimelineConversationItem.group(entries, showsRecords: true).map(\.id) == ["a", "rec", "b"])
        #expect(TimelineConversationItem.group(entries).map(\.id) == ["a", "b"])
    }
    @Test func countsCommandsSeparatelyAndReflectsRunningStatus() {
        #expect(TimelineActivitySummary.title([entry("a"), entry("b", name: "Read"), entry("c", name: "Edit")])
                == "Ran a command · Used 2 tools")
        #expect(TimelineActivitySummary.title([entry("a", status: "running")]) == "Running a command")
        #expect(TimelineActivitySummary.title([entry("a", name: "Read")]) == "Used a tool")
        #expect(TimelineActivitySummary.status(entry("a", status: "failed")) == "Failed or interrupted")
    }
    @Test func summariesDescribeIntentRatherThanInventingResults() {
        let push = entry("a", status: "failed", details: #"{"description":"Push changes to pull request #11","command":"git push"}"#)
        #expect(TimelineActivitySummary.purpose(push) == "Push changes to pull request #11")
        #expect(TimelineActivitySummary.purpose(entry("b", name: "Read", details: #"{"file_path":"/project/README.md"}"#)) == "Read README.md")
        #expect(TimelineActivitySummary.purpose(entry("c", details: "incomplete JSON")) == "Run a shell command")
    }
}

@Suite("Recorded code changes")
struct TimelineFileChangeTests {
    private func entry(name: String = "Edit", status: String = "completed", input: [String: Any]) throws -> AgentTimelineEntry {
        let details = String(decoding: try JSONSerialization.data(withJSONObject: input), as: UTF8.self)
        return AgentTimelineEntry(id: "tool:edit", kind: "tool", text: "", role: nil, toolName: name,
                                  status: status, details: details, truncated: false)
    }
    @Test func replacementPreservesContextAndCountsOnlyChangedLines() throws {
        let e = try entry(input: ["file_path": "/project/src/triage.ts", "old_string": "start\nold\nend\n", "new_string": "start\nnew\nextra\nend\n"])
        let change = try #require(TimelineFileChange.from(e).first)
        #expect(change.additions == 2)
        #expect(change.deletions == 1)
        #expect(change.lines.map(\.text) == ["start", "old", "new", "extra", "end"])
        #expect(change.displayPath == "src/triage.ts")
    }
    @Test func repeatedUnicodeAndBlankLinesDiffCorrectly() throws {
        let e = try entry(input: ["file_path": "a.swift", "old_string": "猫\n\n猫\n", "new_string": "猫\n🐈\n猫\n"])
        let change = try #require(TimelineFileChange.from(e).first)
        #expect(change.additions == 1)
        #expect(change.deletions == 1)
        #expect(change.lines.filter { $0.kind == .removed }.map(\.text) == [""])
        #expect(change.lines.filter { $0.kind == .added }.map(\.text) == ["🐈"])
    }
    @Test func multiEditsStaySeparateAndFailedEditsDoNotClaimChanges() throws {
        let input: [String: Any] = ["file_path": "a.swift", "edits": [
            ["old_string": "one", "new_string": "two"], ["old_string": "three", "new_string": "four", "replace_all": true]
        ]]
        let changes = TimelineFileChange.from(try entry(name: "MultiEdit", input: input))
        #expect(changes.count == 2)
        #expect(changes[1].replacesAll)
        #expect(TimelineFileChange.from(try entry(name: "MultiEdit", status: "failed", input: input)).isEmpty)
        #expect(TimelineFileChange.from(try entry(name: "MultiEdit", status: "running", input: input)).isEmpty)
    }
    @Test func fileWritesAreContentPreviewsWithoutInventingThePreviousVersion() throws {
        let change = try #require(TimelineFileChange.from(try entry(name: "Write", input: ["file_path": "config.json", "content": "{\"enabled\":true}\n"])).first)
        #expect(change.writtenContent)
        #expect(change.additions == 0 && change.deletions == 0)
    }
    @Test func longUnchangedSectionsCollapseAndInvalidInputsAreIgnored() throws {
        let context = (0..<20).map { "line \($0)" }.joined(separator: "\n")
        let change = try #require(TimelineFileChange.from(try entry(input: ["file_path": "a", "old_string": context + "\nold", "new_string": context + "\nnew"])).first)
        #expect(change.previewLines.contains { $0.kind == .omitted && $0.text == "14 unchanged lines" })
        var invalid = try entry(input: ["file_path": "a", "old_string": "old", "new_string": "new"])
        invalid = AgentTimelineEntry(id: invalid.id, kind: "tool", text: "", role: nil, toolName: "Edit", status: "completed", details: "{incomplete", truncated: true)
        #expect(TimelineFileChange.from(invalid).isEmpty)
    }
    @Test func fullRecordedInputRecoversATruncatedPreview() throws {
        let input: [String: Any] = ["file_path": "/project/PaywallView.swift", "content": String(repeating: "let value = 1;\n", count: 1000)]
        let data = try JSONSerialization.data(withJSONObject: input)
        let row = AgentTimelineEntry(id: "tool:large", kind: "tool", text: "", role: nil, toolName: "Write", status: "completed",
                                    details: String(decoding: data.prefix(8192), as: UTF8.self), truncated: true, contentRef: "source:0:\(data.count)")
        #expect(TimelineFileChange.from(row).isEmpty)
        let change = try #require(TimelineFileChange.from(row, inputData: data).first)
        #expect(change.lines.count == 1000)
        #expect(change.previewLines.count == 1000)
        #expect(change.writtenContent)
    }
    @Test func backgroundNotificationsUseTheirSummary() {
        let entry = AgentTimelineEntry(id: "task:one", kind: "tool", text: "Background review finished", role: nil,
                                      toolName: "Background task", status: "completed", details: nil, truncated: false)
        #expect(TimelineActivitySummary.purpose(entry) == "Background review finished")
        #expect(TimelineActivitySummary.title([entry]) == "Background task update")
    }
}

@Suite("Timeline entry interruption rendering")
struct TimelineEntryInterruptionTests {
    @Test func codexErrorRecordsRenderAsMessagesAndOnlyAbortsMarkInterruption() {
        func message(id: String, status: String?) -> AgentTimelineEntry {
            AgentTimelineEntry(id: id, kind: "message", text: "Stream disconnected", role: "assistant",
                               toolName: nil, status: status, details: nil, truncated: false)
        }
        // Codex error records (CodexRecordAction::Error) are assistant/failed
        // with message:/error: ids and must keep normal message rendering.
        #expect(!message(id: "message:msg_0123", status: "failed").isTurnInterrupted)
        #expect(!message(id: "error:3", status: "failed").isTurnInterrupted)
        // Only turn-abort markers render as the interrupted status line.
        #expect(message(id: "turn_aborted:42", status: "failed").isTurnInterrupted)
        #expect(!message(id: "turn_aborted:42", status: "completed").isTurnInterrupted)
        #expect(!message(id: "message:msg_0123", status: nil).isTurnInterrupted)
    }
}

@Suite("Optional upgrade menu version publishing")
@MainActor struct BridgeUpgradePublishingTests {
    @Test func handshakeVersionReachesMenuOnlyWhileRunIsOwned() {
        let upgrade = TimelineBridgeUpgrade()
        let owner = UUID()
        // The handshake lands after the run started: the shared menu version updates.
        AgentTimelineView.publishBridgeVersion("0.2.0", to: upgrade, owner: owner, activeOwner: owner)
        #expect(upgrade.bridgeVersion == "0.2.0")
        #expect(HostHelperProbe.isOlderThanPinned("0.2.0"), "A 0.2.0 host must see the optional upgrade item")
        // A superseded run (or a finished run that cleared its owner) cannot
        // clobber the menu version with a stale handshake value.
        AgentTimelineView.publishBridgeVersion("0.1.4", to: upgrade, owner: owner, activeOwner: UUID())
        #expect(upgrade.bridgeVersion == "0.2.0")
        AgentTimelineView.publishBridgeVersion("0.2.2", to: upgrade, owner: owner, activeOwner: nil)
        #expect(upgrade.bridgeVersion == "0.2.0")
        // With 0.3.0 pinned, 0.2.0/0.2.1/0.2.2 hosts see the optional upgrade item;
        // the pinned version and anything newer never do.
        #expect(HostHelperProbe.isOlderThanPinned("0.2.1"))
        #expect(HostHelperProbe.isOlderThanPinned("0.2.2"))
        #expect(!HostHelperProbe.isOlderThanPinned("0.3.0"))
    }
}

@Suite("Timeline markdown link sanitization")
struct TimelineMarkdownTests {
    private func links(_ text: String) -> [URL] {
        TimelineMarkdown.sanitized(text).runs.compactMap(\.link)
    }

    @Test func httpAndHttpsLinksWithPlainLabelsArePreserved() {
        #expect(links("[docs](https://example.com/docs)") == [URL(string: "https://example.com/docs")!])
        #expect(links("[docs](http://example.com/docs)") == [URL(string: "http://example.com/docs")!])
        #expect(links("[HTTPS](HTTPS://Example.com/x)").count == 1)
    }

    @Test func linksWhoseLabelHostMatchesArePreserved() {
        #expect(links("[https://example.com](https://example.com/a)").count == 1)
        #expect(links("[https://EXAMPLE.com/page](https://example.com/other)").count == 1)
        #expect(links("<https://example.com/a>").count == 1)
    }

    @Test func nonWebSchemesAreStripped() {
        for target in ["shortcuts://run-shortcut?name=x", "tel:+15555550100", "sms:+15555550100",
                       "javascript:alert(1)", "myapp://deep/link", "file:///etc/passwd", "mailto:a@b.c"] {
            let result = TimelineMarkdown.sanitized("[tap me](\(target))")
            #expect(result.runs.compactMap(\.link).isEmpty, "\(target) kept its link")
            #expect(String(result.characters) == "tap me")
        }
    }

    @Test func schemeRelativeAndHostlessLinksAreStripped() {
        #expect(links("[x](/relative/path)").isEmpty)
        #expect(links("[x](https:///nohost)").isEmpty)
    }

    @Test func deceptiveLabelHostsAreStripped() {
        let result = TimelineMarkdown.sanitized("[https://apple.com/login](https://evil.example/login)")
        #expect(result.runs.compactMap(\.link).isEmpty)
        #expect(String(result.characters) == "https://apple.com/login")
        #expect(links("[http://bank.com](https://bank.com.evil.example)").isEmpty)
        #expect(links("[https://good.com](https://good.com@evil.example/)").isEmpty)
    }

    @Test func styledFragmentsOfOneLinkAreValidatedAsAWholeLabel() {
        // Styling splits the label into several runs; each must not be judged alone.
        #expect(links("[https://**bank.com**](https://evil.example)").isEmpty)
        #expect(links("[**https://bank.com**](https://evil.example)").isEmpty)
        #expect(links("[https://_bank_.com](https://evil.example)").isEmpty)
        #expect(links("[https://`bank.com`](https://evil.example)").isEmpty)
        let result = TimelineMarkdown.sanitized("[https://**bank.com**](https://evil.example)")
        #expect(String(result.characters) == "https://bank.com")
        // A matching host stays linked across all of its styled fragments.
        let kept = TimelineMarkdown.sanitized("[https://**bank.com**/login](https://bank.com/login)")
        #expect(kept.runs.compactMap(\.link).count == kept.runs.count)
        #expect(!kept.runs.isEmpty)
    }

    @Test func invisibleFormatCharactersCannotHideADeceptiveLabel() {
        for invisible in ["\u{200C}", "\u{200B}", "\u{200D}", "\u{2060}", "\u{FEFF}", "\u{200E}", "\u{202E}", "\u{00AD}"] {
            #expect(links("[\(invisible)https://bank.com](https://evil.example)").isEmpty, "prefix U+\(invisible.unicodeScalars.first!.value)")
            #expect(links("[https:\(invisible)//bank.com](https://evil.example)").isEmpty, "infix U+\(invisible.unicodeScalars.first!.value)")
            #expect(links("[  \(invisible) HTTPS://bank.com](https://evil.example)").isEmpty)
        }
        // A matching host stays linked even when the label contains invisible characters.
        #expect(links("[\u{200C}https://bank.com](https://bank.com/x)").count == 1)
        // Ordinary labels with invisible characters are unaffected.
        #expect(links("[read\u{200B}me](https://example.com)").count == 1)
        // Invisible characters are also removed from the displayed text of a kept link.
        let cleaned = TimelineMarkdown.sanitized("[\u{200C}https://bank.com](https://bank.com/x)")
        #expect(String(cleaned.characters) == "https://bank.com")
    }

    @Test func bidiControlsCannotDisguiseAValidatedLabel() {
        // U+202E reverses the glyphs that follow it: a label validated as
        // https://moc.live could render as something else entirely. The control
        // is removed from the displayed text, not just from the validation copy.
        let disguised = TimelineMarkdown.sanitized("[https://\u{202E}moc.live](https://moc.live)")
        #expect(disguised.runs.compactMap(\.link) == [URL(string: "https://moc.live")!])
        #expect(String(disguised.characters) == "https://moc.live")
        // Other bidi controls (pop, isolates) are cleaned the same way.
        #expect(links("[https://\u{202D}moc\u{202C}.live](https://moc.live)").count == 1)
        let isolated = TimelineMarkdown.sanitized("[\u{2066}https://moc.live\u{2069}](https://moc.live/x)")
        #expect(String(isolated.characters) == "https://moc.live")
        #expect(isolated.runs.compactMap(\.link).count == 1)

        // A mismatching target is still stripped after normalization.
        #expect(links("[https://\u{202E}moc.live](https://evil.example)").isEmpty)

        // Styling splits the label into runs; every fragment is cleaned.
        let styled = TimelineMarkdown.sanitized("[https://**\u{202E}moc.live**](https://moc.live)")
        #expect(String(styled.characters) == "https://moc.live")
        #expect(styled.runs.compactMap(\.link).count == styled.runs.count)

        // Bidi controls outside links are removed too.
        let mixed = TimelineMarkdown.sanitized("plain \u{202E}text and [x](https://example.com)")
        #expect(String(mixed.characters) == "plain text and x")
    }

    @Test func surroundingBidiControlsCannotDisguiseLinkLabels() {
        for (open, close) in [("\u{202E}", "\u{202C}"), ("\u{202D}", "\u{202C}"),
                              ("\u{2066}", "\u{2069}"), ("\u{2067}", "\u{2069}"),
                              ("\u{2068}", "\u{2069}")] {
            let result = TimelineMarkdown.sanitized("\(open)[moc.knab//:sptth](https://evil.example)\(close)")
            #expect(String(result.characters) == "moc.knab//:sptth")
            let styled = TimelineMarkdown.sanitized("\(open)[https://**bank.com**](https://bank.com)\(close)")
            #expect(String(styled.characters) == "https://bank.com")
            #expect(!styled.runs.compactMap(\.link).isEmpty)
        }
        // Ordinary unlinked whitespace and joiners retain their text semantics.
        let plain = TimelineMarkdown.sanitized("a\n b\u{200D}c")
        #expect(String(plain.characters) == "a\n b\u{200D}c")
    }

    @Test func brailleBlanksCannotHideDeceptiveLabels() {
        #expect(links("[\u{2800}https://bank.com](https://evil.example)").isEmpty)
        #expect(links("[ \u{2800}**https://bank.com**](https://evil.example)").isEmpty)
        #expect(links("[https:\u{2800}//bank.com](https://evil.example)").isEmpty)
        let matching = TimelineMarkdown.sanitized("[\u{2800}https://bank.com](https://bank.com)")
        #expect(String(matching.characters) == "https://bank.com")
        #expect(matching.runs.compactMap(\.link).count == 1)
    }

    @Test func hangulFillersCannotHideDeceptiveLabels() {
        for filler in ["\u{115F}", "\u{1160}", "\u{3164}", "\u{FFA0}"] {
            #expect(links("[\(filler)https://bank.com](https://evil.example)").isEmpty)
            #expect(links("[ \(filler)**https://bank.com**](https://evil.example)").isEmpty)
            #expect(links("[https:\(filler)//bank.com](https://evil.example)").isEmpty)
            let matching = TimelineMarkdown.sanitized("[\(filler)https://bank.com](https://bank.com)")
            #expect(String(matching.characters) == "https://bank.com")
            #expect(matching.runs.compactMap(\.link).count == 1)
            let plain = TimelineMarkdown.sanitized("plain \(filler) text")
            #expect(String(plain.characters) == "plain \(filler) text")
        }
    }

    @Test func defaultIgnorableNonFormatScalarsCannotHideADeceptiveLabel() {
        // Variation selectors and the combining grapheme joiner are default-ignorable
        // without being format/control characters; they must be stripped too.
        for invisible in ["\u{FE0F}", "\u{FE00}", "\u{034F}"] {
            #expect(links("[\(invisible)https://bank.com](https://evil.example)").isEmpty, "prefix U+\(invisible.unicodeScalars.first!.value)")
            #expect(links("[https:\(invisible)//bank.com](https://evil.example)").isEmpty, "infix U+\(invisible.unicodeScalars.first!.value)")
        }
        // A matching host stays linked, and ordinary labels keep working.
        #expect(links("[\u{FE0F}https://bank.com](https://bank.com/x)").count == 1)
        #expect(links("[read\u{034F}me](https://example.com)").count == 1)
    }

    @Test func onlyUnsafeLinksAreStrippedFromMixedText() {
        let result = TimelineMarkdown.sanitized(
            "ok [a](https://example.com) bad [b](tel:123) also [c](https://example.org)")
        #expect(result.runs.compactMap(\.link) == [URL(string: "https://example.com")!, URL(string: "https://example.org")!])
        #expect(String(result.characters) == "ok a bad b also c")
    }
}

@Suite("Model and effort control")
@MainActor struct AgentModelControlTests {
    let identity = AgentTimelineIdentity(paneId: "w1:p1", terminalId: "t", agent: "claude", nativeSessionId: "s")
    func payload(_ fields: [String: Any]) throws -> AgentBridgePayload {
        var result = fields
        result["identity"] = ["pane_id": "w1:p1", "terminal_id": "t", "agent": "claude", "native_session_id": "s"]
        result["generation"] = 1
        return try #require(AgentBridgeFrame.decode(JSONSerialization.data(withJSONObject: ["result": result])).result)
    }
    func state() -> AgentModelControlState {
        let state = AgentModelControlState()
        state.bind(identity: identity, generation: 1)
        state.connect(nil, supported: true)
        return state
    }
    @Test func capabilityDefaultsToFalseAndExpectationKeepsNullKeys() throws {
        let frame = try AgentBridgeFrame.decode(Data(#"{"result":{"capabilities":{"claude_timeline":true}}}"#.utf8))
        #expect(frame.result?.capabilities?.modelControl == false)
        let enabled = try AgentBridgeFrame.decode(Data(#"{"result":{"capabilities":{"claude_timeline":true,"model_control":true}}}"#.utf8))
        #expect(enabled.result?.capabilities?.modelControl == true)
        let data = try JSONEncoder().encode(AgentModelExpectation(model: nil, effort: nil))
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(object["model"] is NSNull)
        #expect(object["effort"] is NSNull)
    }
    @Test func statusUsesSelectedModelsEffortsAndFormatsLabels() throws {
        let state = state()
        try state.update(payload([
            "model": ["id": "claude-opus-5-5", "label": "Opus 5.5"], "effort": "HIGH",
            "models": [
                ["id": "opus", "label": "Opus", "current": true, "efforts": [["id": "high", "label": "High"]]],
                ["id": "sonnet", "label": "Sonnet", "efforts": []]
            ],
            "efforts": [["id": "low", "label": "Low"]]
        ]), options: true)
        #expect(state.label == "Opus 5.5 high")
        #expect(state.accessibilityLabel == "Model Opus 5.5, effort high")
        #expect(state.efforts.map(\.id) == ["high"])
        #expect(state.selectedModelID == "opus")
        #expect(state.isVisible)
        try state.apply(payload(["applied": ["model": ["id": "sonnet", "label": "Sonnet 5.5"], "effort": NSNull()], "confirmed": true]))
        #expect(state.efforts.isEmpty)
        #expect(state.effortLabel == "Default")
        #expect(state.label == "Sonnet 5.5")
        state.reset()
        #expect(!state.isVisible)
        #expect(state.models.isEmpty)
    }
    @Test func statusEffortsApplyWhenNoModelIsKnown() throws {
        let state = state()
        try state.update(payload([
            "model": NSNull(), "effort": NSNull(),
            "models": [["id": "opus", "label": "Opus 5.5", "efforts": [["id": "high", "label": "High"]]]],
            "efforts": [["id": "low", "label": "Low"], ["id": "high", "label": "High"]]
        ]), options: true)
        #expect(state.selectedModelID == nil)
        #expect(state.efforts.map(\.id) == ["low", "high"])
        state.reset()
        #expect(state.efforts.isEmpty)
    }
    @Test func streamedOlderVersionNeverChecksTheAliasRow() throws {
        let state = state()
        try state.update(payload([
            "model": ["id": "claude-haiku-5-5", "label": "Haiku 5.5"], "effort": "low",
            "models": [["id": "haiku", "label": "Haiku 5.5", "current": true, "efforts": [["id": "low", "label": "Low"]]]],
            "efforts": [["id": "low", "label": "Low"]]
        ]), options: true)
        #expect(state.selectedModelID == "haiku")
        try state.update(payload(["model": ["id": "claude-haiku-4-5", "label": "Haiku 4.5"], "effort": "low"]))
        #expect(state.selectedModelID == nil)
        #expect(state.modelName == "Haiku 4.5")
    }
    @Test func partialResultsPreserveAppliedPairAndShowSpecificErrors() throws {
        let state = state()
        for (code, message) in [
            ("agent_busy", "The agent is working — try again when it's idle."),
            ("composer_not_empty", "Clear the agent's input box at the desk first."),
            ("stale", "Something changed at the desk. Check the current model and try again."),
            ("expect_mismatch", "Couldn't change the model. Try again.")
        ] {
            try state.apply(payload(["applied": ["model": ["id": "sonnet", "label": "Sonnet 5.5"], "effort": "medium"],
                "confirmed": false, "error": code]))
            #expect(state.pair.model?.id == "sonnet")
            #expect(state.pair.effort == "medium")
            #expect(state.errorMessage == message)
            #expect(AgentModelControlState.message(for: code) == message)
        }
    }
    @Test func updatesDecodeStringModelsAndRejectOldGenerations() throws {
        let state = state()
        let update = try payload(["model": "claude-opus-5-5", "effort": "high"])
        try state.update(update)
        #expect(state.label == "Opus 5.5 high")
        #expect(AgentModelOption.label(for: "claude-opus-5-5-20261001") == "Opus 5.5")
        var stale = update
        stale.generation = 2
        #expect(throws: AgentBridgeError.identityMismatch) { try state.update(stale) }
        #expect(state.pair.effort == "high")
        state.connect(nil, supported: false)
        #expect(!state.isVisible)
    }
    @Test func modelEventsUseTimelineSequenceAndResetWithOccupant() throws {
        let timeline = AgentTimelineState()
        let owner = UUID()
        timeline.begin(owner: owner, paneId: "w1:p1")
        let data = try payload(["entries": []])
        try timeline.consume(AgentBridgeFrame(event: "timeline.snapshot", epoch: "e", seq: 1, data: data), owner: owner)
        try timeline.consume(AgentBridgeFrame(event: "model.updated", epoch: "e", seq: 2,
            data: payload(["model": "claude-opus-5-5", "effort": "high"])), owner: owner)
        #expect(timeline.modelControl.label == "Opus 5.5 high")
        timeline.end(owner: owner, error: .closed)
        #expect(timeline.modelControl.pair.model == nil)
        #expect(!timeline.modelControl.isVisible)
    }
}
