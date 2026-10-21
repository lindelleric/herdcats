import Foundation
import Observation

enum PhoneAnswersMode: String, Identifiable {
    case install, repair, remove
    var id: String { rawValue }
    var requiresConfirmation: Bool { self == .remove }
    var confirmLabel: String {
        switch self { case .install: "Set up phone answers"; case .repair: "Repair phone answers"; case .remove: "Remove phone answers" }
    }
}
struct PhoneAnswersSelection: Identifiable {
    let key: PhoneAnswersStore.Key
    let mode: PhoneAnswersMode
    var id: String { "\(key.target.storageKey):\(key.generation.rawValue):\(key.agent):\(mode.rawValue)" }
}
struct PhoneAnswersGrace {
    private var started: TimeInterval?
    static let delay: TimeInterval = 1.5
    mutating func update(eligible: Bool, now: TimeInterval) -> Bool {
        guard eligible else { started = nil; return false }
        if started == nil { started = now }
        return now - (started ?? now) >= Self.delay
    }
}
enum PhoneAnswersPresentation {
    struct Hint {
        let title: String
        var detail: String? = nil
        var action: PhoneAnswersMode? = nil
    }
    static func eligible(approvals: Bool?, ready: Bool, blocked: Bool, pending: Bool, active: Bool) -> Bool {
        approvals == true && ready && blocked && !pending && active
    }
    static func hint(agent: ApprovalAgent, result: AgentHooksResult?) -> Hint {
        let plain = Hint(title: "Waiting for you in Terminal")
        guard let result, result.error == nil else { return plain }
        switch result.status {
        case .missing: return Hint(title: plain.title, action: .install)
        case .stale: return Hint(title: plain.title, action: .repair)
        case .needsTrust where agent == .codex:
            return Hint(title: "Approve the Herdcats hook in Codex once", detail: "Codex asks you to review new hooks at your desk. Approve it there to use phone answers.")
        case .disabled where agent == .codex:
            return Hint(title: "Re-enable the Herdcats hook in Codex", detail: "At your desk, open Codex hook settings and re-enable the Herdcats hook.")
        default: return plain
        }
    }
    static func actions(_ result: AgentHooksResult?) -> [PhoneAnswersMode] {
        guard let result, result.error == nil else { return [] }
        switch result.status {
        case .missing: return [.install]
        case .stale: return [.repair, .remove]
        case .installed, .needsTrust, .disabled: return [.remove]
        case nil: return []
        }
    }
    static func succeeded(mode: PhoneAnswersMode, result: AgentHooksResult) -> Bool {
        result.error == nil && result.status == (mode == .remove ? .missing : .installed)
    }
    static func status(_ result: AgentHooksResult?) -> String {
        guard let result else { return "Not checked" }
        guard result.error == nil else { return "Could not check. Try again." }
        switch result.status {
        case .installed: return "Installed"
        case .missing: return "Not set up"
        case .stale: return "Needs repair"
        case .needsTrust: return "Needs approval in Codex"
        case .disabled: return "Disabled in Codex"
        case nil: return "Unknown status"
        }
    }
}
@MainActor @Observable final class PhoneAnswersStore {
    struct Key: Hashable {
        let target: ConnectionIdentity
        let generation: ConnectionGeneration
        let agent: String
    }
    private var results: [Key: AgentHooksResult] = [:]
    private var checked: Set<Key> = []
    private var operations: [Key: UUID] = [:]

    func result(for key: Key) -> AgentHooksResult? { results[key] }
    func isChecking(_ key: Key) -> Bool { operations[key] != nil }
    func invalidate(_ key: Key) {
        checked.remove(key); results[key] = nil; operations[key] = nil
    }
    func check(_ key: Key, force: Bool = false, load: () async throws -> AgentHooksResult) async {
        guard operations[key] == nil, force || !checked.contains(key) else { return }
        checked.insert(key)
        let token = UUID()
        operations[key] = token
        let result: AgentHooksResult
        do { result = try await load() }
        catch { result = AgentHooksResult(status: nil, spoolConfirmed: nil, error: "check_failed", backup: nil) }
        guard operations[key] == token else { return }
        operations[key] = nil
        results[key] = result
    }
}
