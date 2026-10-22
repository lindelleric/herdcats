import Foundation
import Observation
import OSLog

@Observable final class TimelineBridgeUpgrade {
    var bridgeVersion: String?
    var requested: Bool

    init(bridgeVersion: String? = nil, requested: Bool = false) {
        self.bridgeVersion = bridgeVersion
        self.requested = requested
    }

    func requestUpgrade() {
        requested = true
    }
}

enum AgentBridgeError: Error, LocalizedError, Equatable, Sendable {
    case invalidFrame, frameTooLarge, incompleteFrame, protocolMismatch, sequenceGap, identityMismatch
    case closed, timeout
    case unavailable(String)
    case questionFailure(code: String, stepsSent: Int?)
    case routedHostUnsupported
    var code: String {
        switch self {
        case .unavailable(let code): return code
        case .questionFailure(let code, _): return code
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
    var contentRef: String? = nil
    var recordRefs: [String]? = nil

    /// Only Codex turn-abort markers render as the interrupted status line.
    /// Error records are also assistant/failed but keep normal message
    /// rendering, so the marker is keyed on the entry id.
    var isTurnInterrupted: Bool {
        id.hasPrefix("turn_aborted:") && role == "assistant" && status == "failed"
    }
}
struct AgentTimelineIdentity: Codable, Equatable, Sendable {
    let paneId: String
    let terminalId: String
    let agent: String
    let nativeSessionId: String
}
struct AgentBridgeCapabilities: Codable, Equatable, Sendable {
    let claudeTimeline: Bool
    var codexTimeline: Bool? = false
    var codexDaemonConfig: Bool? = false
    var codexDaemonStop: Bool? = false
    var questions: Bool? = nil
    var approvals: Bool? = nil
    var recordContent: Bool? = nil

    init(
        claudeTimeline: Bool,
        codexTimeline: Bool? = false,
        codexDaemonConfig: Bool? = false,
        codexDaemonStop: Bool? = false,
        recordContent: Bool? = nil,
        approvals: Bool? = nil,
        questions: Bool? = nil
    ) {
        self.claudeTimeline = claudeTimeline
        self.codexTimeline = codexTimeline
        self.codexDaemonConfig = codexDaemonConfig
        self.codexDaemonStop = codexDaemonStop
        self.questions = questions
        self.approvals = approvals
        self.recordContent = recordContent
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.claudeTimeline = try container.decode(Bool.self, forKey: .claudeTimeline)
        self.codexTimeline = try container.decodeIfPresent(Bool.self, forKey: .codexTimeline) ?? false
        self.codexDaemonConfig = try container.decodeIfPresent(Bool.self, forKey: .codexDaemonConfig) ?? false
        self.codexDaemonStop = try container.decodeIfPresent(Bool.self, forKey: .codexDaemonStop) ?? false
        self.questions = try container.decodeIfPresent(Bool.self, forKey: .questions)
        self.approvals = try container.decodeIfPresent(Bool.self, forKey: .approvals)
        self.recordContent = try container.decodeIfPresent(Bool.self, forKey: .recordContent)
    }
}

struct CodexDaemonConfigResult: Decodable, Equatable, Sendable {
    let status: String?
    let backup: String?
    let error: String?
    /// `stopped`/`not_running`/`stop_failed`/`unknown`; present only with
    /// --stop-running and only when the config step succeeded.
    var daemon: String? = nil
    var updater: String? = nil

    static func parse(_ raw: String) throws -> CodexDaemonConfigResult {
        let line = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            .components(separatedBy: .newlines)
            .last(where: { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) ?? ""
        guard let data = line.data(using: .utf8) else {
            throw HerdrError.unexpectedResponse("Invalid response encoding")
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(CodexDaemonConfigResult.self, from: data)
    }

    static func message(for result: CodexDaemonConfigResult) -> String {
        if let err = result.error {
            switch err {
            case "config_invalid":
                return "Could not update Codex config: config.toml contains invalid TOML."
            case "features_not_table":
                return "Could not update Codex config: features in config.toml is not a table."
            case "daemon_setting_unsupported":
                return "Could not update Codex config: daemon_auto_start in config.toml has an unsupported shape. Edit it by hand."
            case "config_symlink":
                return "Could not update Codex config: config.toml is a symbolic link."
            case "write_failed":
                return "Could not update Codex config: failed to write file."
            case "verification_failed":
                return "Could not update Codex config: verification failed, restored from backup."
            case "helper_missing":
                return "Could not update Codex config: herdcats-bridge was not found."
            default:
                return "Could not update Codex config: \(err)."
            }
        }
        // The updater is checked first: the bridge never runs `daemon stop`
        // while the updater is alive (it TERM-and-waits precisely so the
        // updater cannot restart the daemon), so an updater failure is the
        // root cause whenever it appears — including the both-stop_failed
        // shape — and recovery must go through the updater-aware command.
        if result.updater == "stop_failed" || result.updater == "unknown" {
            let updaterSurvived = result.updater == "stop_failed"
            let daemonOff = result.daemon == "stopped" || result.daemon == "not_running"
            switch (updaterSurvived, daemonOff) {
            case (true, true):
                return "Codex's shared server was stopped, but its updater is still running and may restart it. Copy the turn-off command below and run it on this computer to stop the updater too, then restart Codex in this pane."
            case (true, false):
                return "Herdcats couldn't stop the updater that keeps Codex's shared server running. Copy the turn-off command below and run it on this computer to stop the updater and the server, then restart Codex in this pane."
            case (false, true):
                return "Codex's shared server was stopped, but Herdcats couldn't confirm the state of its updater, which may restart it. Copy the turn-off command below and run it on this computer, then restart Codex in this pane."
            case (false, false):
                return "Herdcats couldn't confirm the state of Codex's shared server or its updater. Copy the turn-off command below and run it on this computer, then restart Codex in this pane."
            }
        }
        if result.daemon == "stop_failed" {
            return "Codex's shared server is still running. Run `codex app-server daemon stop` on this computer, then restart Codex in this pane."
        }
        if result.daemon == "unknown" {
            return "Herdcats couldn't confirm the running server belongs to Codex, so it left it running. Run `codex app-server daemon stop` on this computer, then restart Codex in this pane."
        }
        switch result.status {
        case "codex_not_installed":
            return "Codex directory was not found on this host."
        default:
            // stopped, not_running, or a legacy reply without daemon fields.
            return "Codex's shared server is off. Quit Codex in this pane and start it again so Herdcats can identify the session."
        }
    }

    /// The updater survived the stop or could not be verified — on its own or
    /// alongside a daemon failure (the bridge refuses to stop the daemon
    /// while the updater is alive). Codex's own `daemon stop` cannot address
    /// the updater, so recovery always goes through the updater-aware bridge
    /// command.
    static func showsUpdaterStopCommand(for result: CodexDaemonConfigResult) -> Bool {
        guard result.error == nil else { return false }
        return result.updater == "stop_failed" || result.updater == "unknown"
    }

    /// The daemon itself survived the stop (or could not be verified) while
    /// the updater was handled: pair the message with Codex's own stop
    /// command.
    static func showsStopCommand(for result: CodexDaemonConfigResult) -> Bool {
        guard result.error == nil, !showsUpdaterStopCommand(for: result) else { return false }
        return result.daemon == "stop_failed" || result.daemon == "unknown"
    }
}
struct AgentBridgePayload: Codable, Sendable {
    var `protocol`: Int?
    var bridgeVersion: String?
    var version: String?
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
    var codexDaemon: String?
    var contentRef: String?
    var offset: Int?
    var nextOffset: Int?
    var total: Int?
    var hex: String?
    var requestId: String?
    var kind: String?
    var agent: String?
    var toolName: String?
    var command: String?
    var description: String?
    var justification: String?
    var environment: String?
    var title: String?
    var options: [AgentApprovalOption]?
    var selected: Int?
    var sent: String?
    var confirmed: Bool?
    var questionKind: String?
    var questions: [AgentQuestion]?
    var step: AgentQuestionStep?
    var fresh: Bool?
    var countdown: String?
}
struct AgentBridgeRemoteError: Codable, Sendable {
    let code: String
    var stepsSent: Int? = nil
}
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
        connected && identity?.paneId == paneId && TimelineAgentAvailability.supports(agent: identity?.agent)
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

    init(output: String, agent: String? = nil) throws {
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
        let targetAgent = agent ?? "claude"
        let compatible = report.map {
            $0.name == "herdcats-bridge" && $0.protocol == 1 && $0.herdrProtocol == 22
                && $0.agents.contains(targetAgent) && Self.isAtLeastMinimum($0.version)
        } ?? false
        self = .present(version: version, managed: marker == "HC_HELPER_MANAGED", compatible: compatible)
    }
    static func versionNumbers(_ version: String) -> [Int]? {
        guard version.utf8.count <= 64 else { return nil }
        let parts = version.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 3, parts.allSatisfy({ !$0.isEmpty && $0.allSatisfy({ $0.isASCII && $0.isNumber }) }) else { return nil }
        let numbers = parts.compactMap { Int($0) }
        return numbers.count == 3 ? numbers : nil
    }
    static func isAtLeastMinimum(_ version: String?) -> Bool {
        guard let version,
              let current = versionNumbers(version),
              let minimum = versionNumbers(HostHelperSetupPlan.minimumVersion)
        else { return false }
        return current == minimum || minimum.lexicographicallyPrecedes(current)
    }
    static func isOlderThanMinimum(_ version: String?) -> Bool {
        guard let version,
              let current = versionNumbers(version),
              let minimum = versionNumbers(HostHelperSetupPlan.minimumVersion)
        else { return false }
        return current.lexicographicallyPrecedes(minimum)
    }
    static func isAtLeastPinned(_ version: String?) -> Bool {
        guard let version,
              let current = versionNumbers(version),
              let pinned = versionNumbers(HostHelperSetupPlan.version)
        else { return false }
        return current == pinned || pinned.lexicographicallyPrecedes(current)
    }
    static func isOlderThanPinned(_ version: String?) -> Bool {
        guard let version,
              let current = versionNumbers(version),
              let pinned = versionNumbers(HostHelperSetupPlan.version)
        else { return false }
        return current.lexicographicallyPrecedes(pinned)
    }
}
enum HostHelperSetupDecision: Equatable, Sendable {
    case ready, missing
    case replacementRequired(HostHelperProbe)
    case install(replace: Bool)
    case unsupportedByPinnedInstaller(String)
}
enum HostHelperSetupReason: String, Equatable, Sendable {
    case missing = "helper_missing"
    case updateRequired = "helper_update_required"

    init(code: String?) {
        if code == "helper_update_required" {
            self = .updateRequired
        } else {
            self = .missing
        }
    }
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
        case .invalidResponse: "Herdcats Bridge could not be verified. Check the installation or use manual setup."
        case .busy: "A Herdcats Bridge setup is already running on this connection. Check again when it finishes."
        case .failed(let code):
            switch code {
            case "curl_missing": "This host needs curl to download Herdcats Bridge. Use manual setup to install it."
            case "checksum_tool_missing": "This host needs shasum or sha256sum to verify the download."
            case "download_failed": "The host could not download the installer. Check its network access, or use manual setup."
            case "checksum_mismatch": "The installer did not match the app’s pinned checksum. Nothing was executed. Use manual setup or update Herdcats."
            default: "The host could not complete installation. Check its permissions and required tools, or use manual setup."
            }
        }
    }
}

/// Mirrors adapters implemented by the current bridge; agent detection alone
/// does not imply that native session history is available.
enum TimelineAgentAvailability {
    static func supports(agent: String?) -> Bool { agent == "claude" || agent == "codex" }
}

/// Only references into the bridge's bounded, verified source snapshot.
enum TimelineContentReference {
    static func length(_ reference: String) -> Int? {
        guard reference.utf8.count <= 128 else { return nil }
        let parts = reference.split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count == 3, parts[0] == "source",
              parts[1].allSatisfy({ $0.isASCII && $0.isNumber }), parts[2].allSatisfy({ $0.isASCII && $0.isNumber }),
              let start = Int(parts[1]), let length = Int(parts[2]), start >= 0, length > 0,
              start <= 64 * 1024 * 1024, length <= 64 * 1024 * 1024 - start else { return nil }
        return length
    }
}

enum TimelineSourcePages {
    static func ranges(_ bytes: Data) -> [Range<Int>] {
        guard !bytes.isEmpty else { return [0..<0] }
        var result: [Range<Int>] = []; var start = 0
        while start < bytes.count {
            var end = min(start + 32768, bytes.count)
            while end < bytes.count && bytes[end] & 0xc0 == 0x80 { end -= 1 }
            result.append(start..<end); start = end
        }
        return result
    }
}

/// Adjacent tool calls form one activity; messages and images keep their order.
struct TimelineConversationItem: Identifiable {
    var entries: [AgentTimelineEntry]
    var id: String { entries[0].id }
    var isActivity: Bool { entries[0].kind == "tool" }

    /// Fallback `record` entries stay hidden unless the caller opts in (Codex timelines).
    static func group(_ entries: [AgentTimelineEntry], showsRecords: Bool = false) -> [Self] {
        var items: [Self] = []
        for entry in entries where showsRecords || entry.kind != "record" {
            if entry.kind == "tool", items.last?.isActivity == true {
                items[items.count - 1].entries.append(entry)
            } else {
                items.append(Self(entries: [entry]))
            }
        }
        return items
    }
}

enum TimelineActivitySummary {
    static func isCommand(_ entry: AgentTimelineEntry) -> Bool {
        entry.toolName?.lowercased() == "bash"
    }
    static func isRunning(_ entry: AgentTimelineEntry) -> Bool {
        entry.status != "completed" && entry.status != "failed"
    }
    static func title(_ entries: [AgentTimelineEntry]) -> String {
        let commands = entries.filter(isCommand).count
        let notifications = entries.filter { $0.toolName == "Background task" }.count
        let tools = entries.count - commands - notifications
        let running = entries.contains(where: isRunning)
        var parts: [String] = []
        if commands > 0 {
            parts.append("\(running ? "Running" : "Ran") \(commands == 1 ? "a command" : "\(commands) commands")")
        }
        if tools > 0 {
            parts.append("\(running ? "Using" : "Used") \(tools == 1 ? "a tool" : "\(tools) tools")")
        }
        if notifications > 0 {
            parts.append(notifications == 1 ? "Background task update" : "\(notifications) background task updates")
        }
        return parts.joined(separator: " · ")
    }
    static func purpose(_ entry: AgentTimelineEntry) -> String {
        if entry.toolName == "Background task" { return String(entry.text.prefix(240)) }
        let input = entry.details.flatMap { $0.data(using: .utf8) }
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] } ?? [:]
        if let description = input["description"] as? String,
           let firstLine = description.split(whereSeparator: \.isNewline).first,
           !firstLine.trimmingCharacters(in: .whitespaces).isEmpty {
            return String(firstLine.prefix(180))
        }
        let path = (input["file_path"] ?? input["path"] ?? input["notebook_path"]) as? String
        let file = path.map { URL(fileURLWithPath: $0).lastPathComponent }
        switch entry.toolName?.lowercased() {
        case "read": return file.map { "Read \($0)" } ?? "Read a file"
        case "edit", "multiedit", "notebookedit": return file.map { "Edit \($0)" } ?? "Edit a file"
        case "write": return file.map { "Write \($0)" } ?? "Write a file"
        case "grep", "glob": return "Search project files"
        case "websearch": return "Search the web"
        case "webfetch": return "Read a web page"
        case "bash": return "Run a shell command"
        default: return entry.toolName.map { "Use \($0)" } ?? "Use a tool"
        }
    }
    static func status(_ entry: AgentTimelineEntry) -> String {
        switch entry.status {
        case "completed": "Completed"
        case "failed": "Failed or interrupted"
        default: "Running"
        }
    }
}


/// Metadata-only diagnostics, available in Xcode and the device's unified log.
enum TimelineHistoryDiagnostics {
    private static let logger = Logger(subsystem: "com.enchantinglabs.herdrcat", category: "TimelineHistory")

    static func record(_ message: String, failure: Bool = false) {
        if failure { logger.error("[TimelineHistory] \(message, privacy: .public)") }
        else { logger.info("[TimelineHistory] \(message, privacy: .public)") }
    }

    static func reason(_ error: any Error) -> String {
        if error is CancellationError { return "cancelled" }
        guard let error = error as? AgentBridgeError else { return "type=\(String(reflecting: type(of: error)))" }
        switch error {
        case .closed: return "closed"
        case .timeout: return "timeout"
        case .identityMismatch: return "identity_mismatch"
        case .invalidFrame: return "invalid_frame"
        case .frameTooLarge: return "frame_too_large"
        case .incompleteFrame: return "incomplete_frame"
        case .protocolMismatch: return "protocol_mismatch"
        case .sequenceGap: return "sequence_gap"
        case .routedHostUnsupported: return "routed_host_unsupported"
        case .unavailable(let code), .questionFailure(let code, _):
            let safe = code.prefix(64).filter { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_") }
            return "remote_\(safe)"
        }
    }
}

struct TimelineDiffLine: Identifiable, Sendable {
    enum Kind: Sendable { case context, added, removed, omitted }
    let id: Int
    let kind: Kind
    let text: String
}

/// Session edit excerpts, independent of whatever the working file contains now.
struct TimelineFileChange: Identifiable, Sendable {
    let id: String
    let path: String
    let writtenContent: Bool
    let previewShortened: Bool
    let replacesAll: Bool
    let lines: [TimelineDiffLine]
    var additions: Int { lines.filter { $0.kind == .added }.count }
    var deletions: Int { lines.filter { $0.kind == .removed }.count }
    var displayPath: String { path.split(separator: "/").suffix(2).joined(separator: "/") }
    var previewLines: [TimelineDiffLine] {
        if writtenContent { return lines }
        var result: [TimelineDiffLine] = []
        var index = 0
        while index < lines.count {
            if lines[index].kind != .context { result.append(lines[index]); index += 1; continue }
            let start = index
            while index < lines.count && lines[index].kind == .context { index += 1 }
            if index - start > 6 {
                result.append(contentsOf: lines[start..<start + 3])
                result.append(TimelineDiffLine(id: -start - 1, kind: .omitted, text: "\(index - start - 6) unchanged lines"))
                result.append(contentsOf: lines[index - 3..<index])
            } else { result.append(contentsOf: lines[start..<index]) }
        }
        return result
    }

    static func from(_ entry: AgentTimelineEntry, inputData: Data? = nil) -> [Self] {
        guard entry.kind == "tool", entry.status == "completed",
              let data = inputData ?? entry.details?.data(using: .utf8),
              let input = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let path = (input["file_path"] ?? input["path"]) as? String, !path.isEmpty else { return [] }
        let name = entry.toolName?.lowercased()
        if name == "write", let content = input["content"] as? String {
            let rows = splitLines(content)
            return [Self(id: "\(entry.id):diff:0", path: path, writtenContent: true, previewShortened: rows.count > 2048, replacesAll: false,
                         lines: rows.prefix(2048).enumerated().map { TimelineDiffLine(id: $0.offset, kind: .context, text: $0.element) })]
        }
        guard name == "edit" || name == "multiedit" else { return [] }
        let edits = name == "multiedit" ? (input["edits"] as? [[String: Any]] ?? []) : [input]
        return edits.enumerated().compactMap { index, edit in
            guard let old = edit["old_string"] as? String, let new = edit["new_string"] as? String,
                  old != new else { return nil }
            let before = splitLines(old), after = splitLines(new)
            // Bound diff computation even if future helpers send larger inputs.
            guard before.count <= 2048, after.count <= 2048,
                  old.utf8.count + new.utf8.count <= 128 * 1024 else { return nil }
            return Self(id: "\(entry.id):diff:\(index)", path: path, writtenContent: false, previewShortened: false,
                        replacesAll: edit["replace_all"] as? Bool == true, lines: diff(before: before, after: after))
        }
    }

    private static func splitLines(_ text: String) -> [String] {
        guard !text.isEmpty else { return [] }
        var rows = text.components(separatedBy: "\n")
        if rows.last == "" { rows.removeLast() }
        return rows
    }

    private static func diff(before: [String], after: [String]) -> [TimelineDiffLine] {
        var removed: Set<Int> = [], added: Set<Int> = []
        for change in after.difference(from: before) {
            switch change {
            case .remove(let offset, _, _): removed.insert(offset)
            case .insert(let offset, _, _): added.insert(offset)
            }
        }
        var result: [TimelineDiffLine] = []
        var old = 0, new = 0
        func append(_ kind: TimelineDiffLine.Kind, _ text: String) {
            result.append(TimelineDiffLine(id: result.count, kind: kind, text: text))
        }
        while old < before.count || new < after.count {
            if old < before.count && removed.contains(old) { append(.removed, before[old]); old += 1 }
            else if new < after.count && added.contains(new) { append(.added, after[new]); new += 1 }
            else if old < before.count && new < after.count { append(.context, after[new]); old += 1; new += 1 }
            else if old < before.count { append(.removed, before[old]); old += 1 }
            else { append(.added, after[new]); new += 1 }
        }
        return result
    }
}
