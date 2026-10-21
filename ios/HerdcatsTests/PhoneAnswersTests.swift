import Foundation
import Testing

@testable import Herdcats

@Suite("Phone answers setup")
@MainActor struct PhoneAnswersTests {
    let target = ConnectionIdentity(host: "host", port: 22, username: "user")
    func key(_ generation: UInt64 = 1, _ agent: ApprovalAgent = .claude) -> PhoneAnswersStore.Key {
        .init(target: target, generation: .init(rawValue: generation), agent: agent.rawValue)
    }
    func result(_ status: AgentHooksStatus?, error: String? = nil) -> AgentHooksResult {
        .init(status: status, spoolConfirmed: false, error: error, backup: nil)
    }
    @Test func hintActionDependsOnVerifiedStatusAndAgent() {
        for agent in ApprovalAgent.allCases {
            #expect(PhoneAnswersPresentation.hint(agent: agent, result: result(.missing)).action == .install)
            #expect(PhoneAnswersPresentation.hint(agent: agent, result: result(.stale)).action == .repair)
            for status: AgentHooksStatus? in [.installed, nil, .needsTrust, .disabled] {
                #expect(PhoneAnswersPresentation.hint(agent: agent, result: result(status)).action == nil)
            }
            #expect(PhoneAnswersPresentation.hint(agent: agent, result: result(.missing, error: "error")).action == nil)
        }
        #expect(PhoneAnswersPresentation.hint(agent: .codex, result: result(.needsTrust)).title == "Approve the Herdcats hook in Codex once")
        #expect(PhoneAnswersPresentation.hint(agent: .codex, result: result(.disabled)).title == "Re-enable the Herdcats hook in Codex")
        #expect(PhoneAnswersPresentation.hint(agent: .claude, result: result(.needsTrust)).title == "Waiting for you in Terminal")
    }
    @Test func graceRequiresContinuousEligibleBlockedState() {
        var grace = PhoneAnswersGrace()
        let steps: [(Bool, TimeInterval)] = [(true, 10), (true, 11.49), (true, 11.5), (false, 12), (true, 13), (true, 14)]
        let shown = steps.map { grace.update(eligible: $0.0, now: $0.1) }
        #expect(shown == [false, false, true, false, false, false])
        #expect(!PhoneAnswersPresentation.eligible(approvals: nil, ready: true, blocked: true, pending: false, active: true))
        #expect(!PhoneAnswersPresentation.eligible(approvals: true, ready: true, blocked: true, pending: true, active: true))
        #expect(!PhoneAnswersPresentation.eligible(approvals: true, ready: true, blocked: true, pending: false, active: false))
    }
    @Test func checksOncePerGenerationAndAgentAndRefreshesAfterEdit() async {
        let store = PhoneAnswersStore()
        var reads = 0
        func load() async throws -> AgentHooksResult { reads += 1; return result(.installed) }
        await store.check(key(), load: load)
        await store.check(key(), load: load)
        #expect(reads == 1)
        await store.check(key(2), load: load)
        await store.check(key(1, .codex), load: load)
        #expect(reads == 3)
        store.invalidate(key())
        await store.check(key(), load: load)
        #expect(reads == 4)
        #expect(store.result(for: key())?.status == .installed)
    }
    @Test func errorsAreCachedUntilExplicitCheck() async {
        let store = PhoneAnswersStore()
        var reads = 0
        await store.check(key()) { reads += 1; throw AgentBridgeError.closed }
        await store.check(key()) { reads += 1; return result(.installed) }
        #expect(reads == 1)
        #expect(PhoneAnswersPresentation.actions(store.result(for: key())).isEmpty)
        await store.check(key(), force: true) { reads += 1; return result(.installed) }
        #expect(reads == 2)
        #expect(store.result(for: key())?.status == .installed)
    }
    @Test func preEditStatusCannotOverwriteRefreshedStatus() async {
        let store = PhoneAnswersStore()
        var resume: CheckedContinuation<AgentHooksResult, Never>?
        let old = Task { @MainActor in
            await store.check(key()) { await withCheckedContinuation { resume = $0 } }
        }
        while resume == nil { await Task.yield() }
        store.invalidate(key())
        await store.check(key()) { result(.missing) }
        resume?.resume(returning: result(.installed))
        await old.value
        #expect(store.result(for: key())?.status == .missing)
    }
    @Test func unknownOrFailedEditIsNotReportedAsSuccess() {
        #expect(!PhoneAnswersPresentation.succeeded(mode: .install, result: result(nil)))
        #expect(!PhoneAnswersPresentation.succeeded(mode: .remove, result: result(.installed)))
        #expect(!PhoneAnswersPresentation.succeeded(mode: .repair, result: result(.installed, error: "error")))
        #expect(PhoneAnswersPresentation.succeeded(mode: .install, result: result(.installed)))
        #expect(PhoneAnswersPresentation.succeeded(mode: .remove, result: result(.missing)))
    }
    @Test func settingsActionsAndRemoveConfirmation() {
        #expect(PhoneAnswersPresentation.actions(result(.missing)) == [.install])
        #expect(PhoneAnswersPresentation.actions(result(.stale)) == [.repair, .remove])
        #expect(PhoneAnswersPresentation.actions(result(.installed)) == [.remove])
        #expect(PhoneAnswersPresentation.actions(result(nil)).isEmpty)
        #expect(PhoneAnswersPresentation.actions(result(.installed, error: "error")).isEmpty)
        #expect(PhoneAnswersMode.remove.requiresConfirmation)
        #expect(PhoneAnswersMode.remove.confirmLabel == "Remove phone answers")
        #expect(PhoneAnswersMode.repair.confirmLabel == "Repair phone answers")
    }
}
