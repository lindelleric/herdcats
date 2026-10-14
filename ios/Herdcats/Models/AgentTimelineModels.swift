import Foundation

enum AgentBridgeError: Error, LocalizedError, Equatable, Sendable {
    case invalidFrame, frameTooLarge, incompleteFrame, protocolMismatch, sequenceGap, identityMismatch
    case closed, timeout
    case unavailable(String)
    case routedHostUnsupported
    var code: String {
        switch self {
        case .unavailable(let code): return code
        case .routedHostUnsupported: return "routed_host_unsupported"
        case .timeout: return "bridge_timeout"
        case .protocolMismatch: return "protocol_mismatch"
        default: return "bridge_unavailable"
        }
    }
    var errorDescription: String? { "Timeline unavailable. Open Terminal to continue." }
}
struct AgentTimelineEntry: Codable, Identifiable, Equatable, Sendable {
    let id: String
    let kind: String
    let text: String
    let role: String?
    let toolName: String?
    let status: String?
    let details: String?
    let truncated: Bool
}
struct AgentTimelineIdentity: Codable, Equatable, Sendable {
    let paneId: String
    let terminalId: String
    let agent: String
    let nativeSessionId: String
}
struct AgentBridgeCapabilities: Codable, Sendable { let claudeTimeline: Bool }
struct AgentBridgePayload: Codable, Sendable {
    var `protocol`: Int?
    var epoch: String?
    var capabilities: AgentBridgeCapabilities?
    var identity: AgentTimelineIdentity?
    var generation: UInt64?
    var entries: [AgentTimelineEntry]?
    var hasMore: Bool?
    var before: String?
    var agentStatus: String?
    var unknownRecords: Int?
    var droppedEntries: Int?
    var paneId: String?
    var code: String?
}
struct AgentBridgeRemoteError: Codable, Sendable { let code: String }
struct AgentBridgeFrame: Decodable, Sendable {
    var id: String?
    var result: AgentBridgePayload?
    var error: AgentBridgeRemoteError?
    var event: String?
    var epoch: String?
    var seq: UInt64?
    var data: AgentBridgePayload?
    static func decode(_ bytes: Data) throws -> Self {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(Self.self, from: bytes)
    }
}
struct AgentBridgeFramer {
    private var bytes = Data()
    let limit: Int
    init(limit: Int = 1024 * 1024) { self.limit = limit }
    mutating func append(_ chunk: Data) throws -> [AgentBridgeFrame] {
        var frames: [AgentBridgeFrame] = []
        // Bound the partial line even when an SSH chunk contains many frames.
        for byte in chunk {
            if byte == 10 {
                do { frames.append(try AgentBridgeFrame.decode(bytes)) } catch {
                    throw AgentBridgeError.invalidFrame
                }
                bytes.removeAll(keepingCapacity: true)
            } else {
                guard bytes.count < limit else { throw AgentBridgeError.frameTooLarge }
                bytes.append(byte)
            }
        }
        return frames
    }
    func finish() throws { if !bytes.isEmpty { throw AgentBridgeError.incompleteFrame } }
}

/// Readiness plus transport identity; "connecting" must not share the stream
/// key of "connected" even though neither phase is classified as offline.
struct AgentTimelineConnectionState: Equatable, Sendable {
    let phase: AppModel.Phase
    let generation: ConnectionGeneration?
    var isReady: Bool {
        if case .connected = phase { return true }
        return false
    }
    func requireCurrent(_ current: Self) throws {
        guard isReady, self == current else { throw AgentBridgeError.closed }
    }
}

enum TimelineComposePolicy {
    static func canSend(identity: AgentTimelineIdentity?, paneId: String, status: String?, connected: Bool)
        -> Bool
    {
        connected && identity?.paneId == paneId && identity?.agent == "claude"
            && ["idle", "done", "working"].contains(status ?? "")
    }
}

extension TimelineComposePolicy {
    static func shouldClearDraft(current: String, sent: String) -> Bool { current == sent }
}
