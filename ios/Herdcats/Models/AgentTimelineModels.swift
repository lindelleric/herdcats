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
    var evictedIds: [String]?
    var hasMore: Bool?
    var before: String?
    var agentStatus: String?
    var unknownRecords: Int?
    var droppedEntries: Int?
    var delayed: Bool?
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
    /// Native history is optional; absent history uses the existing pane input
    /// path. A present identity must still pass native session/status checks.
    static func canCompose(identity: AgentTimelineIdentity?, paneId: String, status: String?, connected: Bool) -> Bool {
        connected && (identity == nil || canSend(identity: identity, paneId: paneId, status: status, connected: connected))
    }

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

/// A probe is read-only. Existing, unverifiable installations require an explicit replacement choice.
enum HostHelperProbe: Equatable, Sendable {
    case missing
    case present(version: String?, managed: Bool, compatible: Bool)

    init(output: String) throws {
        let lines = output.trimmingCharacters(in: .whitespacesAndNewlines).split(separator: "\n", omittingEmptySubsequences: false)
        guard let marker = lines.first else { throw HostHelperSetupError.invalidResponse }
        if marker == "HC_HELPER_MISSING", lines.count == 1 { self = .missing; return }
        if marker == "HC_HELPER_BLOCKED" { throw HostHelperSetupError.unsafeDestination }
        guard marker == "HC_HELPER_MANAGED" || marker == "HC_HELPER_EXTERNAL" else { throw HostHelperSetupError.invalidResponse }
        struct Report: Decodable {
            let name: String
            let version: String
            let `protocol`: Int
            let herdrProtocol: Int
            let agents: [String]
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let bytes = Data(lines.dropFirst().joined(separator: "\n").utf8)
        let report = try? decoder.decode(Report.self, from: bytes)
        let version = report.flatMap { Self.versionNumbers($0.version) == nil ? nil : $0.version }
        let compatible = report.map {
            $0.name == "herdcats-bridge" && $0.protocol == 1 && $0.herdrProtocol == 22
                && $0.agents.contains("claude") && Self.isAtLeastPinned($0.version)
        } ?? false
        self = .present(version: version, managed: marker == "HC_HELPER_MANAGED", compatible: compatible)
    }
    private static func versionNumbers(_ version: String) -> [Int]? {
        guard version.utf8.count <= 64 else { return nil }
        let parts = version.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 3, parts.allSatisfy({ !$0.isEmpty && $0.allSatisfy({ $0.isASCII && $0.isNumber }) }) else { return nil }
        let numbers = parts.compactMap { Int($0) }
        return numbers.count == 3 ? numbers : nil
    }
    private static func isAtLeastPinned(_ version: String) -> Bool {
        guard let current = versionNumbers(version), let minimum = versionNumbers(HostHelperSetupPlan.version) else { return false }
        return current == minimum || minimum.lexicographicallyPrecedes(current)
    }
}
enum HostHelperSetupDecision: Equatable, Sendable {
    case ready, missing
    case replacementRequired(HostHelperProbe)
    case install(replace: Bool)
}
enum HostHelperSetupStage: Equatable, Sendable {
    case idle, checking, installing, verifying, uncertain, installed
    case replacementRequired(HostHelperProbe)
    var isBusy: Bool { self == .checking || self == .installing || self == .verifying }
}
enum HostHelperSetupError: Error, LocalizedError, Equatable {
    case unsafeDestination, invalidResponse, failed(String), busy
    var errorDescription: String? {
        switch self {
        case .unsafeDestination: "The install location is a link or another nonregular file. Use manual setup to choose a safe location."
        case .invalidResponse: "The helper could not be verified. Check the installation or use manual setup."
        case .busy: "A helper setup is already running on this connection. Check again when it finishes."
        case .failed(let code):
            switch code {
            case "curl_missing": "This host needs curl to download the helper. Use manual setup to install it."
            case "checksum_tool_missing": "This host needs shasum or sha256sum to verify the download."
            case "download_failed": "The host could not download the installer. Check its network access, or use manual setup."
            case "checksum_mismatch": "The installer did not match the app’s pinned checksum. Nothing was executed. Use manual setup or update Herdcats."
            default: "The host could not complete installation. Check its permissions and required tools, or use manual setup."
            }
        }
    }
}
