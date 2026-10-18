import Testing

@testable import Herdcats

@Suite("Timeline composition policy")
struct TimelineComposePolicyTests {
    @Test func paneCompositionRemainsAvailableWhenHistoryHasNoIdentity() {
        #expect(TimelineComposePolicy.canCompose(identity: nil, paneId: "p", status: nil, connected: true))
        #expect(TimelineComposePolicy.canCompose(identity: nil, paneId: "p", status: "blocked", connected: true))
        #expect(!TimelineComposePolicy.canCompose(identity: nil, paneId: "p", status: nil, connected: false))
        let stale = AgentTimelineIdentity(paneId: "other", terminalId: "t", agent: "claude", nativeSessionId: "s")
        #expect(!TimelineComposePolicy.canCompose(identity: stale, paneId: "p", status: "idle", connected: true))
    }
    @Test func onlyMatchingUnblockedClaudeIdentityCanCompose() {
        let identity = AgentTimelineIdentity(
            paneId: "p", terminalId: "t", agent: "claude", nativeSessionId: "s")
        #expect(
            TimelineComposePolicy.canSend(identity: identity, paneId: "p", status: "idle", connected: true))
        #expect(!TimelineComposePolicy.canSend(identity: nil, paneId: "p", status: "idle", connected: true))
        #expect(
            !TimelineComposePolicy.canSend(
                identity: identity, paneId: "other", status: "idle", connected: true))
        #expect(
            !TimelineComposePolicy.canSend(
                identity: identity, paneId: "p", status: "blocked", connected: true))
        #expect(
            !TimelineComposePolicy.canSend(identity: identity, paneId: "p", status: "idle", connected: false))
    }
    @Test func matchingUnblockedCodexIdentityCanCompose() {
        let identity = AgentTimelineIdentity(
            paneId: "p", terminalId: "t", agent: "codex", nativeSessionId: "s")
        #expect(TimelineComposePolicy.canSend(identity: identity, paneId: "p", status: "idle", connected: true))
        #expect(TimelineComposePolicy.canSend(identity: identity, paneId: "p", status: "working", connected: true))
        #expect(TimelineComposePolicy.canSend(identity: identity, paneId: "p", status: "done", connected: true))
        #expect(!TimelineComposePolicy.canSend(identity: identity, paneId: "p", status: "blocked", connected: true))
        #expect(!TimelineComposePolicy.canSend(identity: identity, paneId: "other", status: "idle", connected: true))
        #expect(!TimelineComposePolicy.canSend(identity: identity, paneId: "p", status: "idle", connected: false))
    }
}

@Suite("Timeline prompt dispatch ownership")
struct TimelinePromptDispatchTests {
    @Test func replacedClientIsNeverSelectedForOldPrompt() throws {
        var slot = ConnectionClientSlot<String>()
        let old = slot.beginReplace().attempt
        _ = slot.install("old host", expected: old)
        let next = slot.beginReplace().attempt
        _ = slot.install("new host", expected: next)
        #expect(throws: HerdrError.self) { try slot.ownedClient(old) }
        #expect(try slot.ownedClient(next) == "new host")
    }
    @Test func laterDraftIsNotClearedByEarlierAcknowledgement() {
        #expect(!TimelineComposePolicy.shouldClearDraft(current: "next message", sent: "first message"))
        #expect(TimelineComposePolicy.shouldClearDraft(current: "first message", sent: "first message"))
    }
}
