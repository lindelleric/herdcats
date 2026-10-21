import Foundation

enum ApprovalAgent: String, Codable, CaseIterable, Sendable { case claude, codex }
struct AgentApprovalOption: Codable, Equatable, Sendable {
    let number: UInt32
    let labelLines: [String]
    let key: String

    static func validKey(_ key: String) -> Bool {
        if key == "esc" { return true }
        guard key.utf8.count == 1, let byte = key.utf8.first else { return false }
        return (48...57).contains(byte) || (65...90).contains(byte) || (97...122).contains(byte)
    }
}
struct AgentPendingApproval: Equatable, Sendable {
    let identity: AgentTimelineIdentity
    let generation: UInt64
    let requestId: String
    let agent: String
    let toolName: String
    let command: String
    let description: String?
    let justification: String?
    let environment: String?
    let title: String
    let options: [AgentApprovalOption]
    let selected: Int?

    static func decode(_ data: AgentBridgePayload) throws -> Self {
        guard let identity = data.identity, let generation = data.generation,
            data.kind == "approval", let requestId = data.requestId,
            !requestId.isEmpty, requestId.utf8.count <= 264,
            let agent = data.agent, ApprovalAgent(rawValue: agent) != nil, identity.agent == agent,
            let toolName = data.toolName, let command = data.command, command.utf8.count <= 65536,
            let title = data.title, let options = data.options, !options.isEmpty, options.count <= 16,
            Set(options.map(\.number)).count == options.count,
            options.allSatisfy({ $0.number > 0 && !$0.labelLines.isEmpty && $0.labelLines.count <= 8
                && $0.labelLines.allSatisfy { $0.utf8.count <= 1024 } && AgentApprovalOption.validKey($0.key) }),
            data.selected.map({ options.indices.contains($0) }) ?? true
        else { throw AgentBridgeError.invalidFrame }
        return Self(identity: identity, generation: generation, requestId: requestId, agent: agent,
            toolName: toolName, command: command, description: data.description, justification: data.justification,
            environment: data.environment, title: title, options: options, selected: data.selected)
    }
}
struct AgentApprovalResponse: Equatable, Sendable {
    let requestId: String
    let sent: String
    let confirmed: Bool
}
enum AgentHooksStatus: String, Decodable, Sendable { case installed, missing, stale, needsTrust = "needs_trust", disabled }
enum AgentHooksOperation: String, CaseIterable, Sendable { case status, install, uninstall }
struct AgentHooksResult: Equatable, Sendable {
    let status: AgentHooksStatus?
    let spoolConfirmed: Bool?
    let error: String?
    let backup: String?

    static func parse(_ raw: String) throws -> Self {
        struct Payload: Decodable {
            let status: String?
            let spoolConfirmed: Bool?
            let error: String?
            let backup: String?
        }
        let line = raw.split(whereSeparator: \.isNewline).last.map(String.init) ?? ""
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let payload = try decoder.decode(Payload.self, from: Data(line.utf8))
        return Self(status: payload.status.flatMap(AgentHooksStatus.init(rawValue:)), spoolConfirmed: payload.spoolConfirmed,
            error: payload.error, backup: payload.backup)
    }
}
