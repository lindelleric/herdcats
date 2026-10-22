import Foundation
import Observation

@MainActor @Observable final class ApprovalResponseState {
    struct Scope: Equatable {
        let identity: AgentTimelineIdentity
        let generation: UInt64
        let requestId: String
        let command: String
        let options: [AgentApprovalOption]
        init(_ pending: AgentPendingApproval) {
            identity = pending.identity; generation = pending.generation
            requestId = pending.requestId; command = pending.command; options = pending.options
        }
    }
    struct Effects: Equatable {
        var haptic = false
        var focusComposer = false
    }
    enum Phase: Equatable { case idle, sending, confirmed, sent, changed, failed(String) }
    private(set) var scope: Scope?
    private(set) var phase: Phase = .idle
    private var operation: UUID?
    private var operationScope: Scope?

    var isDisabled: Bool { phase == .sending || phase == .confirmed || phase == .sent }
    var message: String? {
        switch phase {
        case .sent: "Sent. Waiting for the terminal…"
        case .changed: "The dialog changed on the host."
        case .failed(let message): message
        default: nil
        }
    }
    var isError: Bool {
        switch phase { case .changed, .failed: true; default: false }
    }
    static func isVisible(pending: AgentPendingApproval?, approvals: Bool?) -> Bool {
        approvals == true && pending != nil
    }
    static func requiresSwipe(index: Int, count: Int) -> Bool { index > 0 && index < count - 1 }
    static func focusesComposer(agent: String, option: AgentApprovalOption) -> Bool { agent == "codex" && option.key == "esc" }

    func sync(_ pending: AgentPendingApproval?) {
        let next = pending.map(Scope.init)
        guard next != scope else { return }
        scope = next
        phase = .idle
        // A resolution may precede the RPC reply. Keep its success effects,
        // but retire them as soon as another dialog replaces it.
        if next != nil { operation = nil; operationScope = nil }
    }
    func begin(_ pending: AgentPendingApproval) -> UUID? {
        guard scope == Scope(pending), !isDisabled, operation == nil else { return nil }
        let token = UUID()
        operation = token; operationScope = scope
        phase = .sending
        return token
    }
    func succeed(token: UUID, confirmed: Bool, focusComposer: Bool) -> Effects {
        guard operation == token else { return Effects() }
        operation = nil
        if scope == operationScope { phase = confirmed ? .confirmed : .sent }
        operationScope = nil
        return Effects(haptic: confirmed, focusComposer: focusComposer)
    }
    func fail(token: UUID, error: AgentBridgeError) {
        guard operation == token else { return }
        operation = nil
        if scope == operationScope {
            phase = ["stale", "mismatch"].contains(error.code) ? .changed : .failed("Couldn't send your answer. Answer in Terminal, or try again.")
        }
        operationScope = nil
    }
    func reset() {
        scope = nil; phase = .idle; operation = nil; operationScope = nil
    }
}

@MainActor @Observable final class TimelineSheetState {
    enum Sheet: String, Identifiable { case phoneAnswers, helper, codexDaemon; var id: String { rawValue } }
    private(set) var presented: Sheet?
    private(set) var dismissing: Sheet?
    func sync(visible: Bool, phone: Bool = false, helper: Bool = false, codex: Bool = false) {
        if let presented {
            let keep: Bool
            switch presented {
            case .phoneAnswers: keep = phone
            case .helper: keep = helper
            case .codexDaemon: keep = codex
            }
            if !visible || !keep { close() }
        }
        guard visible, presented == nil, dismissing == nil else { return }
        if phone { presented = .phoneAnswers }
        else if helper { presented = .helper }
        else if codex { presented = .codexDaemon }
    }
    func close() {
        guard let presented else { return }
        dismissing = presented
        self.presented = nil
    }
    func didDismiss() { dismissing = nil }
}
