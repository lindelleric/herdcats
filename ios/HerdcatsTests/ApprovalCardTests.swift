import Foundation
import Testing

@testable import Herdcats

@Suite("Approval card interactions")
@MainActor struct ApprovalCardTests {
    func approval(_ id: String = "request", agent: String = "claude") throws -> AgentPendingApproval {
        try AgentPendingApproval.decode(ApprovalTests().event("interaction.pending", seq: 1, agent: agent, requestId: id).data!)
    }
    @Test func onlyMiddlePositionsRequireSwipe() {
        #expect(ApprovalResponseState.requiresSwipe(index: 0, count: 1) == false)
        #expect(ApprovalResponseState.requiresSwipe(index: 0, count: 2) == false)
        #expect(ApprovalResponseState.requiresSwipe(index: 1, count: 2) == false)
        #expect(ApprovalResponseState.requiresSwipe(index: 0, count: 4) == false)
        #expect(ApprovalResponseState.requiresSwipe(index: 1, count: 4) == true)
        #expect(ApprovalResponseState.requiresSwipe(index: 2, count: 4) == true)
        #expect(ApprovalResponseState.requiresSwipe(index: 3, count: 4) == false)
    }
    @Test func confirmationWaitsForResolvedWithoutAllowingAnotherSend() throws {
        let state = ApprovalResponseState(), pending = try approval()
        state.sync(pending)
        let token = try #require(state.begin(pending))
        #expect(state.phase == .sending)
        #expect(state.begin(pending) == nil)
        let effects = state.succeed(token: token, confirmed: true, focusComposer: false)
        #expect(effects == .init(haptic: true, focusComposer: false))
        #expect(state.phase == .confirmed)
        #expect(state.begin(pending) == nil)
        state.sync(nil)
        #expect(state.scope == nil)
        #expect(state.phase == .idle)
    }
    @Test func unconfirmedAndRefusedResponsesHaveDistinctMessages() throws {
        let state = ApprovalResponseState(), pending = try approval()
        state.sync(pending)
        let sent = try #require(state.begin(pending))
        #expect(state.succeed(token: sent, confirmed: false, focusComposer: false).haptic == false)
        #expect(state.phase == .sent)
        #expect(state.message == "Sent. Waiting for the terminal…")
        #expect(state.begin(pending) == nil)
        for code in ["stale", "mismatch", "unsupported_capability"] {
            state.sync(nil); state.sync(pending)
            let token = try #require(state.begin(pending))
            state.fail(token: token, error: .unavailable(code))
            #expect(state.message == (code == "unsupported_capability" ? "Couldn't send your answer. Answer in Terminal, or try again." : "The dialog changed on the host."))
            #expect(state.isDisabled == false)
        }
    }
    @Test func failedAnswersUseActionableMessageAndAllowRetry() throws {
        for error: AgentBridgeError in [.closed, .timeout, .invalidFrame, .unavailable("unsupported_capability"), .unavailable("not_watching")] {
            let state = ApprovalResponseState(), pending = try approval()
            state.sync(pending)
            let token = try #require(state.begin(pending))
            state.fail(token: token, error: error)
            #expect(state.message == "Couldn't send your answer. Answer in Terminal, or try again.")
            #expect(state.isError)
            #expect(!state.isDisabled)
            #expect(state.begin(pending) != nil)
        }
    }
    @Test func escFocusesComposerForBothSuccessfulOutcomesEvenIfAlreadyResolved() throws {
        let pending = try approval(agent: "codex")
        let escape = AgentApprovalOption(number: 3, labelLines: ["No"], key: "esc")
        #expect(ApprovalResponseState.focusesComposer(agent: "codex", option: escape))
        #expect(!ApprovalResponseState.focusesComposer(agent: "claude", option: escape))
        #expect(!ApprovalResponseState.focusesComposer(agent: "codex", option: pending.options[0]))
        for confirmed in [true, false] {
            let state = ApprovalResponseState()
            state.sync(pending)
            let token = try #require(state.begin(pending))
            state.sync(nil)
            #expect(state.succeed(token: token, confirmed: confirmed, focusComposer: true) == .init(haptic: confirmed, focusComposer: true))
            #expect(state.phase == .idle)
        }
    }
    @Test func replacementIgnoresOldResponseAndRefusal() throws {
        let state = ApprovalResponseState(), first = try approval(), next = try approval("next")
        state.sync(first)
        let old = try #require(state.begin(first))
        state.sync(next)
        #expect(state.succeed(token: old, confirmed: true, focusComposer: true) == .init(haptic: false, focusComposer: false))
        state.fail(token: old, error: .unavailable("stale"))
        #expect(state.phase == .idle)
        #expect(state.scope == ApprovalResponseState.Scope(next))
    }
    @Test func cardRequiresExplicitCapabilityAndPending() throws {
        let pending = try approval()
        #expect(!ApprovalResponseState.isVisible(pending: pending, approvals: nil))
        #expect(!ApprovalResponseState.isVisible(pending: pending, approvals: false))
        #expect(!ApprovalResponseState.isVisible(pending: nil, approvals: true))
        #expect(ApprovalResponseState.isVisible(pending: pending, approvals: true))
    }
}

@Suite("Inline approval focus")
@MainActor struct InlineApprovalFocusTests {
    @Test func currentEscResponseFocusesImmediately() throws {
        let state = ApprovalResponseState(), pending = try ApprovalCardTests().approval(agent: "codex")
        state.sync(pending)
        let token = try #require(state.begin(pending))
        #expect(state.succeed(token: token, confirmed: true, focusComposer: true).focusComposer)
    }
    @Test func replacementDiscardsOldFocusRequest() throws {
        let state = ApprovalResponseState(), pending = try ApprovalCardTests().approval(agent: "codex")
        state.sync(pending)
        let token = try #require(state.begin(pending))
        state.sync(try ApprovalCardTests().approval("next", agent: "codex"))
        #expect(!state.succeed(token: token, confirmed: true, focusComposer: true).focusComposer)
    }
    @Test func leavingViewDiscardsFocusRequest() throws {
        let state = ApprovalResponseState(), pending = try ApprovalCardTests().approval(agent: "codex")
        state.sync(pending)
        let token = try #require(state.begin(pending))
        state.reset()
        #expect(!state.succeed(token: token, confirmed: true, focusComposer: true).focusComposer)
    }
    @Test func remainingSheetsWaitForDismissal() {
        let state = TimelineSheetState()
        state.sync(visible: true, phone: true)
        #expect(state.presented == .phoneAnswers)
        state.sync(visible: true, helper: true)
        #expect(state.presented == nil)
        #expect(state.dismissing == .phoneAnswers)
        state.didDismiss()
        state.sync(visible: true, helper: true)
        #expect(state.presented == .helper)
    }
}

#if DEBUG && targetEnvironment(simulator)
@Suite("Approval screenshot fixtures")
@MainActor struct ApprovalScreenshotTests {
    @Test func codexLiveDialogKeepsWrappedRowsAndShortcuts() throws {
        let frame = try AgentTimelineScreenshotFixture.approvalFrame(paneId: "p0", agent: "codex")
        let pending = try AgentPendingApproval.decode(#require(frame.data))
        #expect(frame.seq == 2)
        #expect(pending.title == "Would you like to run the following command?")
        #expect(pending.command == "touch herdcats-live-4.txt")
        #expect(pending.environment == "local")
        #expect(pending.justification == "Allow creating herdcats-live-4.txt in this read-only workspace?")
        #expect(pending.options.map(\.key) == ["y", "p", "esc"])
        #expect(pending.options[0].labelLines == ["Yes, proceed"])
        #expect(pending.options[1].labelLines.count == 2)
    }
    @Test func claudeFixtureUsesBashDescriptionAndDigitOptions() throws {
        let frame = try AgentTimelineScreenshotFixture.approvalFrame(paneId: "p0", agent: "claude")
        let pending = try AgentPendingApproval.decode(#require(frame.data))
        #expect(pending.agent == "claude")
        #expect(pending.toolName == "Bash")
        #expect(pending.description != nil)
        #expect(pending.justification == nil)
        #expect(pending.options.map(\.key) == ["1", "2", "3"])
    }
}
#endif
