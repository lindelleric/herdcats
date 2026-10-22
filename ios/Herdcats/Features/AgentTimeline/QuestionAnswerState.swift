import Foundation
import Observation

@MainActor @Observable final class QuestionAnswerState {
    enum Phase: Equatable { case idle, sending, confirmed, sent, inProgress, stale, failed(String) }
    private(set) var pending: AgentPendingQuestion?
    private(set) var phase: Phase = .idle
    private(set) var selections: [Int: Set<UInt32>] = [:]
    private(set) var texts: [Int: String] = [:]
    private var operation: UUID?

    var isDisabled: Bool { pending?.fresh != true || [.sending, .confirmed, .sent, .inProgress, .stale].contains(phase) }
    var canSubmit: Bool { !isDisabled && answers != nil }
    var message: String? {
        switch phase {
        case .idle: return pending?.fresh == false ? "Started in Terminal, finish it there." : nil
        case .sent: return "Sent. Waiting for the terminal…"
        case .inProgress: return "Started in Terminal, finish it there."
        case .stale: return "Couldn't finish from the phone. The terminal shows where it stopped."
        case .failed(let message): return message
        case .sending, .confirmed: return nil
        }
    }
    static func isVisible(pending: AgentPendingQuestion?, questions: Bool?) -> Bool { questions == true && pending != nil }
    static func sanitize(_ text: String) -> String {
        String(String.UnicodeScalarView(text.unicodeScalars.filter {
            !CharacterSet.controlCharacters.contains($0) && !CharacterSet.newlines.contains($0)
        }.prefix(500)))
    }
    func sync(_ next: AgentPendingQuestion?) {
        // Countdown and screen updates must preserve drafts and outstanding replies.
        let same = pending?.identity == next?.identity && pending?.generation == next?.generation
            && pending?.requestId == next?.requestId && pending?.questions == next?.questions
            && pending?.questionKind == next?.questionKind
        pending = next
        if !same { selections = [:]; texts = [:]; phase = .idle; operation = nil }
    }
    func select(question index: Int, option: UInt32) {
        guard !isDisabled, let q = pending?.questions.first(where: { $0.index == index }),
            q.options.contains(where: { $0.number == option }) else { return }
        if q.multiSelect {
            if selections[index, default: []].contains(option) { selections[index]?.remove(option) }
            else { selections[index, default: []].insert(option) }
        } else {
            selections[index] = [option]
            if pending?.questionKind != .codexPlan { texts[index] = nil }
        }
    }
    func setText(_ value: String, question index: Int) {
        guard !isDisabled, let pending, let q = pending.questions.first(where: { $0.index == index }),
            q.allowsOther || pending.questionKind == .codexPlan else { return }
        texts[index] = Self.sanitize(value)
        if !q.multiSelect && pending.questionKind != .codexPlan { selections[index] = [] }
    }
    var answers: [AgentQuestionAnswer]? {
        guard let pending else { return nil }
        var result: [AgentQuestionAnswer] = []
        for q in pending.questions {
            let options = selections[q.index, default: []].sorted()
            let text = texts[q.index, default: ""].trimmingCharacters(in: .whitespacesAndNewlines)
            guard (q.multiSelect || options.count <= 1),
                options.allSatisfy({ number in q.options.contains { $0.number == number } }),
                (!options.isEmpty || !text.isEmpty),
                pending.questionKind != .codexPlan || options.count == 1 else { return nil }
            result.append(AgentQuestionAnswer(index: q.index, options: options, text: text.isEmpty ? nil : text))
        }
        return result
    }
    func begin(skip: Bool = false) -> UUID? {
        guard !isDisabled, operation == nil, skip ? pending?.questionKind == .codexAsync : canSubmit else { return nil }
        let token = UUID(); operation = token; phase = .sending; return token
    }
    func succeed(token: UUID, confirmed: Bool) {
        guard operation == token else { return }
        operation = nil
        // Even an unconfirmed response may have sent input. Never permit a resend.
        phase = confirmed ? .confirmed : .sent
    }
    func fail(token: UUID, error: AgentBridgeError) {
        guard operation == token else { return }
        operation = nil
        switch error.code {
        case "in_progress": phase = .inProgress
        case "stale": phase = .stale
        default: phase = .failed("Couldn't send your answer. Answer in Terminal, or try again.")
        }
    }
    func reset() { pending = nil; selections = [:]; texts = [:]; phase = .idle; operation = nil }
}
