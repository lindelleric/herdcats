import Citadel
import Foundation
import NIOCore

struct AgentBridgeParams: Encodable, Sendable {
    var `protocol`: Int? = nil
    var paneId: String? = nil
    var generation: UInt64? = nil
    var before: String? = nil
    var recordContent: Bool? = nil
    var contentRef: String? = nil
    var offset: Int? = nil
    var approvals: Bool? = nil
    var requestId: String? = nil
    var option: UInt32? = nil
    var labelLines: [String]? = nil
}
enum AgentBridgeMethod: String, Sendable { case initialize, watch, history, content; case respond = "interaction.respond" }

/// One non-PTY channel, with bounded requests and guarded approval responses.
/// Cancellation closes this channel explicitly: canceling an async iterator
/// alone cannot be relied upon to wake an idle SSH read.
actor AgentBridgeClient {
    nonisolated let id: UUID
    private struct Pending {
        let method: AgentBridgeMethod
        let continuation: CheckedContinuation<AgentBridgePayload, Error>
        let timeout: Task<Void, Never>
    }
    private struct Request: Encodable {
        let id: String
        let method: String
        let params: AgentBridgeParams
    }
    private var write: (@Sendable (Data) async throws -> Void)?
    private var closeTransport: (@Sendable () async -> Void)?
    private let onEvent: @Sendable (AgentBridgeFrame) async throws -> Void
    private var pending: [String: Pending] = [:]
    private var retiredReadRequests: Set<String> = []
    private var framer = AgentBridgeFramer()
    private var streamTask: Task<Void, Never>?
    private var opening: CheckedContinuation<Void, Error>?
    private var openTimeout: Task<Void, Never>?
    private var closedError: AgentBridgeError?
    private var waiters: [CheckedContinuation<Void, Error>] = []
    private(set) var capabilities: AgentBridgeCapabilities?

    init(id: UUID = UUID(), onEvent: @escaping @Sendable (AgentBridgeFrame) async throws -> Void) {
        self.id = id
        self.onEvent = onEvent
    }
    init(
        write: @escaping @Sendable (Data) async throws -> Void, close: @escaping @Sendable () async -> Void,
        onEvent: @escaping @Sendable (AgentBridgeFrame) async throws -> Void
    ) {
        self.id = UUID()
        self.write = write
        self.closeTransport = close
        self.onEvent = onEvent
    }
    func performHandshakeAndWatch(
        paneId: String,
        agent: String? = nil,
        onHandshake: (@Sendable (AgentBridgeCapabilities?, String?) async -> Void)? = nil
    ) async throws {
        let handshake = try await request(.initialize, params: AgentBridgeParams(protocol: 1, recordContent: true, approvals: true))
        guard handshake.protocol == 1, handshake.epoch?.isEmpty == false else {
            throw AgentBridgeError.protocolMismatch
        }
        self.capabilities = handshake.capabilities
        let version = handshake.bridgeVersion ?? handshake.version
        await onHandshake?(handshake.capabilities, version)
        if agent == "codex" {
            guard handshake.capabilities?.codexTimeline == true else {
                throw AgentBridgeError.unavailable(Self.unavailableCodeForCodex(handshake: handshake))
            }
        } else {
            guard handshake.capabilities?.claudeTimeline == true else {
                throw AgentBridgeError.unavailable("transcript_root_unavailable")
            }
        }
        guard handshake.capabilities?.recordContent == true else {
            throw AgentBridgeError.unavailable("helper_update_required")
        }
        _ = try await request(.watch, params: AgentBridgeParams(paneId: paneId))
        try Task.checkCancellation()
    }

    func respond(requestId: String, option: UInt32, labelLines: [String]) async throws -> AgentApprovalResponse {
        let reply = try await request(.respond, params: AgentBridgeParams(requestId: requestId, option: option, labelLines: labelLines))
        guard reply.requestId == requestId, let sent = reply.sent, AgentApprovalOption.validKey(sent),
            let confirmed = reply.confirmed else { throw AgentBridgeError.invalidFrame }
        return AgentApprovalResponse(requestId: requestId, sent: sent, confirmed: confirmed)
    }

    func open(
        client: SSHClient,
        command: String,
        paneId: String,
        agent: String? = nil,
        onHandshake: (@Sendable (AgentBridgeCapabilities?, String?) async -> Void)? = nil
    ) async throws {
        try await withTaskCancellationHandler {
            try Task.checkCancellation()
            guard closedError == nil else { throw AgentBridgeError.closed }
            try await withCheckedThrowingContinuation { continuation in
                opening = continuation
                openTimeout = Task {
                    do { try await Task.sleep(for: .seconds(5)) } catch { return }
                    await self.stop(.timeout)
                }
                streamTask = Task {
                    do {
                        try await client.withExec(command) { inbound, outbound in
                            await self.attach(outbound)
                            for try await output in inbound {
                                switch output {
                                case .stdout(let buffer): await self.receive(Data(buffer.readableBytesView))
                                case .stderr: break  // Never log transcript/path/account diagnostics.
                                }
                            }
                        }
                        try self.framer.finish()
                        await self.stop(.closed)
                    } catch { await self.stop((error as? AgentBridgeError) ?? .closed) }
                }
            }
            try await performHandshakeAndWatch(paneId: paneId, agent: agent, onHandshake: onHandshake)
        } onCancel: {
            Task { await self.close() }
        }
    }
    private func attach(_ writer: TTYStdinWriter) async {
        guard closedError == nil else {
            try? await writer.close()
            return
        }
        write = { bytes in try await writer.write(ByteBuffer(bytes: bytes)) }
        closeTransport = { try? await writer.close() }
        openTimeout?.cancel()
        openTimeout = nil
        let continuation = opening
        opening = nil
        continuation?.resume()
    }
    func content(reference: String, generation: UInt64, identity: AgentTimelineIdentity) async throws -> Data {
        guard let expectedLength = TimelineContentReference.length(reference) else { throw AgentBridgeError.invalidFrame }
        var bytes = Data()
        while bytes.count < expectedLength {
            try Task.checkCancellation()
            let reply = try await request(.content, params: AgentBridgeParams(generation: generation, contentRef: reference, offset: bytes.count), timeout: 10)
            guard reply.identity == identity, reply.generation == generation else { throw AgentBridgeError.identityMismatch }
            guard reply.contentRef == reference, reply.offset == bytes.count, reply.total == expectedLength,
                  let hex = reply.hex, hex.utf8.count <= 131072, hex.utf8.count.isMultiple(of: 2) else { throw AgentBridgeError.invalidFrame }
            let digits = Array(hex.utf8)
            var chunk = Data(); chunk.reserveCapacity(digits.count / 2)
            func nibble(_ byte: UInt8) -> UInt8? {
                switch byte { case 48...57: byte - 48; case 97...102: byte - 87; default: nil }
            }
            for index in stride(from: 0, to: digits.count, by: 2) {
                guard let high = nibble(digits[index]), let low = nibble(digits[index + 1]) else { throw AgentBridgeError.invalidFrame }
                chunk.append(high * 16 + low)
            }
            guard !chunk.isEmpty, chunk.count <= expectedLength - bytes.count,
                  reply.nextOffset == bytes.count + chunk.count else { throw AgentBridgeError.invalidFrame }
            bytes.append(chunk)
        }
        try Task.checkCancellation()
        return bytes
    }
    // History and image reads may outlive their UI task. Retire only the RPC;
    // keep the live watch open and drain its eventual response.
    private func retireReadRequest(_ id: String, error: AgentBridgeError) async {
        guard let request = pending.removeValue(forKey: id) else { return }
        request.timeout.cancel()
        if request.method == .history {
            TimelineHistoryDiagnostics.record("history.rpc.retired client=\(self.id) request=\(id) reason=\(error == .timeout ? "timeout" : "cancelled") pending=\(pending.count)", failure: true)
        }
        request.continuation.resume(throwing: error)
        guard retiredReadRequests.count < 64 else { await stop(.closed); return }
        retiredReadRequests.insert(id)
    }
    func request(_ method: AgentBridgeMethod, params: AgentBridgeParams, timeout: TimeInterval = 5)
        async throws -> AgentBridgePayload
    {
        let id = UUID().uuidString
        if method == .history {
            TimelineHistoryDiagnostics.record("history.rpc.start client=\(self.id) request=\(id) generation=\(params.generation.map(String.init) ?? "missing") cursor_present=\(params.before != nil) timeout_seconds=\(timeout) pending=\(pending.count)")
        }
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            guard closedError == nil, let write else { throw closedError ?? .closed }
            guard pending.count < 16 else { throw AgentBridgeError.unavailable("too_many_requests") }
            let encoder = JSONEncoder()
            encoder.keyEncodingStrategy = .convertToSnakeCase
            var bytes = try encoder.encode(Request(id: id, method: method.rawValue, params: params))
            bytes.append(10)
            guard bytes.count <= 1024 * 1024 else { throw AgentBridgeError.frameTooLarge }
            return try await withCheckedThrowingContinuation { continuation in
                let timeoutTask = Task {
                    do { try await Task.sleep(for: .seconds(timeout)) } catch { return }
                    if method == .content || method == .history { await self.retireReadRequest(id, error: .timeout) }
                    else { await self.stop(.timeout) }
                }
                pending[id] = Pending(method: method, continuation: continuation, timeout: timeoutTask)
                Task {
                    do { try await write(bytes) } catch {
                        if method == .history {
                            TimelineHistoryDiagnostics.record("history.rpc.write_failed client=\(self.id) request=\(id) reason=\(TimelineHistoryDiagnostics.reason(error))", failure: true)
                        }
                        // Startup stdout can already contain helper_missing when
                        // the remote process exits. Drain it before classifying;
                        // stream completion or the request timeout still closes.
                        if method != .initialize { await self.stop(.closed) }
                    }
                }
            }
        } onCancel: {
            Task {
                if method == .content || method == .history { await self.retireReadRequest(id, error: .closed) }
                else { await self.close() }
            }
        }
    }
    func receive(_ bytes: Data) async {
        guard closedError == nil else { return }
        do {
            for frame in try framer.append(bytes) {
                guard closedError == nil else { return }
                if let id = frame.id {
                    if retiredReadRequests.remove(id) != nil {
                        TimelineHistoryDiagnostics.record("timeline.rpc.late_reply_ignored client=\(self.id) request=\(id)")
                        continue
                    }
                    guard frame.event == nil, let request = pending.removeValue(forKey: id) else {
                        throw AgentBridgeError.invalidFrame
                    }
                    request.timeout.cancel()
                    if let error = frame.error {
                        if request.method == .history {
                            TimelineHistoryDiagnostics.record("history.rpc.remote_error client=\(self.id) request=\(id) reason=\(TimelineHistoryDiagnostics.reason(AgentBridgeError.unavailable(error.code)))", failure: true)
                        }
                        request.continuation.resume(throwing: AgentBridgeError.unavailable(error.code))
                    } else if let result = frame.result {
                        if request.method == .history {
                            TimelineHistoryDiagnostics.record("history.rpc.response client=\(self.id) request=\(id) entries=\(result.entries?.count ?? -1) generation=\(result.generation.map(String.init) ?? "missing")")
                        }
                        request.continuation.resume(returning: result)
                    } else {
                        request.continuation.resume(throwing: AgentBridgeError.invalidFrame)
                        throw AgentBridgeError.invalidFrame
                    }
                } else if let error = frame.error, error.code == "helper_missing",
                    frame.event == nil, frame.result == nil {
                    // A fixed launcher response, before a helper exists to initialize.
                    await stop(.unavailable("helper_missing"))
                } else if frame.event != nil {
                    try await onEvent(frame)
                } else {
                    throw AgentBridgeError.invalidFrame
                }
            }
        } catch { await stop((error as? AgentBridgeError) ?? .invalidFrame) }
    }
    func waitUntilClosed() async throws {
        try await withTaskCancellationHandler {
            try Task.checkCancellation()
            if let closedError { throw closedError }
            try await withCheckedThrowingContinuation { waiters.append($0) }
        } onCancel: {
            Task { await self.close() }
        }
    }
    func close() async { await stop(.closed) }
    private func stop(_ error: AgentBridgeError) async {
        guard closedError == nil else { return }
        TimelineHistoryDiagnostics.record("timeline.channel.closed client=\(self.id) reason=\(TimelineHistoryDiagnostics.reason(error)) pending=\(pending.count)")
        closedError = error
        openTimeout?.cancel()
        openTimeout = nil
        let opening = self.opening
        self.opening = nil
        opening?.resume(throwing: error)
        let requests = Array(pending.values)
        pending.removeAll()
        for request in requests {
            request.timeout.cancel()
            request.continuation.resume(throwing: error)
        }
        let waiters = self.waiters
        self.waiters = []
        for waiter in waiters { waiter.resume(throwing: error) }
        let closer = closeTransport
        closeTransport = nil
        write = nil
        streamTask?.cancel()
        streamTask = nil
        await closer?()
    }
}

extension HerdrConnection {
    /// Default direct-host session only. Routed machines are rejected before
    /// dialing; an optional helper failure never closes the shared SSH client.
    func openAgentTimeline(
        paneId: String, agent: String? = nil, onEvent: @escaping @Sendable (AgentBridgeFrame) async throws -> Void
    ) async throws -> AgentBridgeClient {
        guard route == nil else { throw AgentBridgeError.routedHostUnsupported }
        guard let client else { throw AgentBridgeError.closed }
        let owned = generation
        let bridgeId = UUID()
        let bridge = AgentBridgeClient(
            id: bridgeId,
            onEvent: { [weak self] frame in
                guard let self, await self.agentBridgeCanDeliver(bridgeId: bridgeId, owned: owned) else {
                    throw AgentBridgeError.closed
                }
                try await onEvent(frame)
            })
        agentBridges[bridge.id] = bridge
        do {
            try await bridge.open(
                client: client,
                command: Self.agentBridgeCommand,
                paneId: paneId,
                agent: agent,
                onHandshake: { [weak self] capabilities, version in
                    await self?.recordAgentBridgeHandshake(capabilities: capabilities, version: version, generation: owned)
                }
            )
            try requireGeneration(owned)
            try Task.checkCancellation()
            guard agentBridges[bridge.id] != nil else { throw AgentBridgeError.closed }
            return bridge
        } catch {
            agentBridges.removeValue(forKey: bridge.id)
            await bridge.close()
            throw error
        }
    }
    func endAgentTimeline(_ bridge: AgentBridgeClient) async {
        agentBridges.removeValue(forKey: bridge.id)
        await bridge.close()
    }
    func closeAllAgentBridges() {
        let bridges = Array(agentBridges.values)
        agentBridges.removeAll()
        clearAgentBridgeCapabilities()
        Task { for bridge in bridges { await bridge.close() } }
    }
    static func helperScript(
        exec: String,
        missingPayload: String,
        readStdinOnMissing: Bool,
        missingExitCode: Int
    ) -> String {
        let drain = readStdinOnMissing ? "  IFS= read -r HB_REQUEST || exit \(missingExitCode)\n" : ""
        return """
            B=
            if [ -x "$HOME/.local/bin/herdcats-bridge" ]; then
              B="$HOME/.local/bin/herdcats-bridge"
            else
              B=$(command -v herdcats-bridge 2>/dev/null)
            fi
            if [ -z "$B" ]; then
              for P in "$HOME/.local/bin/herdcats-bridge" "$HOME/.cargo/bin/herdcats-bridge" /opt/homebrew/bin/herdcats-bridge /usr/local/bin/herdcats-bridge; do
                if [ -x "$P" ]; then B="$P"; break; fi
              done
            fi
            if [ -z "$B" ]; then
            \(drain)  printf '%s\\n' '\(missingPayload)'
              exit \(missingExitCode)
            fi
            \(exec)
            """
    }

    static func helperCommand(
        exec: String,
        missingPayload: String,
        readStdinOnMissing: Bool,
        missingExitCode: Int
    ) -> String {
        let script = helperScript(
            exec: exec,
            missingPayload: missingPayload,
            readStdinOnMissing: readStdinOnMissing,
            missingExitCode: missingExitCode
        )
        return "exec /bin/sh -c \(shellSafeArgument(script))"
    }

    static var agentBridgeScript: String {
        let exec = """
            S=${HERDR_SOCKET_PATH:-${XDG_CONFIG_HOME:-$HOME/.config}/herdr/herdr.sock}
            exec "$B" serve --stdio --socket "$S"
            """
        return helperScript(
            exec: exec,
            missingPayload: "{\"error\":{\"code\":\"helper_missing\"}}",
            readStdinOnMissing: true,
            missingExitCode: 127
        )
    }

    static var agentBridgeCommand: String {
        "exec /bin/sh -c \(shellSafeArgument(agentBridgeScript))"
    }
}

extension HerdrConnection {
    func agentBridgeCanDeliver(bridgeId: UUID, owned: ConnectionGeneration) -> Bool {
        slot.matches(owned) && agentBridges[bridgeId] != nil
    }
}

extension AgentBridgeClient {
    static func unavailableCodeForCodex(handshake: AgentBridgePayload) -> String {
        let isOldHelper = !HostHelperProbe.isAtLeastMinimum(handshake.bridgeVersion ?? handshake.version)
        return isOldHelper ? "helper_update_required" : "transcript_root_unavailable"
    }

    static func isVersionAtLeast(_ version: String?, major: Int, minor: Int) -> Bool {
        guard let version else { return false }
        let numbers = version.split(separator: ".").compactMap { Int($0) }
        guard numbers.count >= 2 else { return false }
        if numbers[0] > major { return true }
        if numbers[0] == major && numbers[1] >= minor { return true }
        return false
    }
}

/// Immutable release selected by this app build, not a host-supplied URL.
enum HostHelperSetupPlan {
    static let version = "0.2.2"
    /// The oldest bridge this app works with; bump only when the app needs a newer bridge feature.
    static let minimumVersion = "0.2.0"
    static let installerSHA256 = "be2ae7715ffe3ef793565e4a171fe36c47fb562bb62f595bdcca3a8f49119289"
    /// Supported agents bundled in this pinned release. Must be updated when bumping version / installerSHA256.
    static let agents: [String] = ["claude", "codex"]
    static func manualInstallCommand(reason: HostHelperSetupReason, pinnedVersion: String = version) -> String {
        "curl -fsSL https://github.com/zhiyao/herdcats/releases/download/herdcats-bridge-v\(pinnedVersion)/install.sh | sh -s -- --version \(pinnedVersion)" + (reason == .updateRequired ? " --replace" : "")
    }
    static func installationScript(replace: Bool) -> String {
        return #"""
        set -eu
        fail() { printf 'HC_HELPER_ERROR:%s\n' "$1"; exit 0; }
        command -v curl >/dev/null 2>&1 || fail curl_missing
        if command -v sha256sum >/dev/null 2>&1; then HB_HASH=sha256sum
        elif command -v shasum >/dev/null 2>&1; then HB_HASH=shasum
        else fail checksum_tool_missing; fi
        HB_SETUP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/herdcats-setup.XXXXXX") || fail temporary_directory_failed
        trap 'rm -rf "$HB_SETUP_DIR"' EXIT
        trap 'exit 1' HUP INT TERM
        curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
          --connect-timeout 15 --max-time 60 --max-filesize 65536 --retry 1 \
          -o "$HB_SETUP_DIR/install.sh" \
          'https://github.com/zhiyao/herdcats/releases/download/herdcats-bridge-v\#(version)/install.sh' \
          2>/dev/null || fail download_failed
        if [ "$HB_HASH" = sha256sum ]; then HB_SUM=$(sha256sum "$HB_SETUP_DIR/install.sh" | awk '{print $1}')
        else HB_SUM=$(shasum -a 256 "$HB_SETUP_DIR/install.sh" | awk '{print $1}'); fi
        [ "$HB_SUM" = '\#(installerSHA256)' ] || fail checksum_mismatch
        /bin/sh "$HB_SETUP_DIR/install.sh" --version '\#(version)'\#(replace ? " --replace" : "") \
          >"$HB_SETUP_DIR/install.log" 2>&1 || fail install_failed
        printf '%s\n' HC_HELPER_INSTALLED
        "$HOME/.local/bin/herdcats-bridge" --version-json 2>/dev/null
        """#
    }
}

extension HostHelperSetupPlan {
    static func decision(
        probe: HostHelperProbe,
        replace: Bool,
        checkOnly: Bool,
        agent: String? = nil,
        reason: HostHelperSetupReason = .missing
    ) -> HostHelperSetupDecision {
        if let agent, !agents.contains(agent) {
            return .unsupportedByPinnedInstaller(agent)
        }
        switch probe {
        case .missing:
            return checkOnly ? .missing : .install(replace: false)
        case .present(let version, let managed, let compatible):
            if checkOnly {
                return compatible ? .ready : .replacementRequired(probe)
            }
            if reason == .updateRequired && HostHelperProbe.isOlderThanPinned(version) {
                if managed {
                    return .install(replace: true)
                }
                return replace ? .install(replace: false) : .replacementRequired(probe)
            }
            if compatible { return .ready }
            if replace {
                return .install(replace: managed)
            }
            return .replacementRequired(probe)
        }
    }

    static func installLabel(
        stage: HostHelperSetupStage,
        reason: HostHelperSetupReason = .missing,
        pinnedVersion: String = HostHelperSetupPlan.version
    ) -> String {
        switch stage {
        case .idle:
            switch reason {
            case .updateRequired:
                return "Upgrade to \(pinnedVersion)"
            case .missing:
                return "Install Herdcats Bridge"
            }
        case .checking:
            return "Checking host…"
        case .installing:
            return reason == .updateRequired ? "Upgrading Herdcats Bridge…" : "Installing Herdcats Bridge…"
        case .verifying:
            return "Verifying installation…"
        case .uncertain:
            return "Check installation"
        case .installed:
            return "Open Timeline"
        case .replacementRequired(let probe):
            if case .present(_, true, _) = probe {
                return "Replace Herdcats Bridge with \(pinnedVersion)"
            } else {
                return "Install managed Herdcats Bridge"
            }
        }
    }

    static func replacementFeedback(probe: HostHelperProbe) -> String? {
        guard case .present(let version, let managed, _) = probe else { return nil }
        if managed {
            if let version, !version.isEmpty {
                return "Found Herdcats Bridge \(version) on this host. Tap Replace to upgrade it at the location shown."
            } else {
                return "Existing Herdcats Bridge will be replaced at the location shown. Tap Replace to upgrade it."
            }
        } else {
            return "An existing Herdcats Bridge was found elsewhere. Install a managed copy at the location shown; the other copy will remain unchanged."
        }
    }
}

/// One bounded command channel. Never closes the shared SSH client.
actor HostHelperCommand {
    private var continuation: CheckedContinuation<String, Error>?
    private var output = Data()
    private var closer: (@Sendable () async -> Void)?
    private var work: Task<Void, Never>?
    private var timer: Task<Void, Never>?
    private var ended = false
    private var started = false

    func run(timeout: TimeInterval, operation: @escaping @Sendable (HostHelperCommand) async throws -> Void) async throws -> String {
        try await withTaskCancellationHandler {
            try Task.checkCancellation()
            guard !started && !ended else { throw CancellationError() }
            started = true
            return try await withCheckedThrowingContinuation { continuation in
                self.continuation = continuation
                timer = Task {
                    do { try await Task.sleep(for: .seconds(timeout)) } catch { return }
                    await self.finish(error: HerdrError.timeout("helper setup"))
                }
                work = Task {
                    do {
                        try Task.checkCancellation()
                        try await operation(self)
                        await self.finish(error: nil)
                    } catch { await self.finish(error: error) }
                }
            }
        } onCancel: { Task { await self.finish(error: CancellationError()) } }
    }
    func run(client: SSHClient, script: String, timeout: TimeInterval) async throws -> String {
        let command = "exec /bin/sh -c \(HerdrConnection.shellSafeArgument(script))"
        return try await run(timeout: timeout) { owned in
            try await client.withExec(command) { inbound, outbound in
                guard await owned.attach(close: { try? await outbound.close() }) else { throw CancellationError() }
                for try await frame in inbound {
                    switch frame {
                    case .stdout(let bytes), .stderr(let bytes): await owned.receive(Data(bytes.readableBytesView))
                    }
                }
            }
        }
    }
    func attach(close: @escaping @Sendable () async -> Void) async -> Bool {
        guard started && !ended else { await close(); return false }
        closer = close
        return true
    }
    func receive(_ data: Data) async {
        guard !ended else { return }
        guard data.count <= 65536 - output.count else { await finish(error: HostHelperSetupError.invalidResponse); return }
        output.append(data)
    }
    private func finish(error: Error?) async {
        guard !ended else { return }
        ended = true
        timer?.cancel(); timer = nil
        let pending = continuation; continuation = nil
        let close = closer; closer = nil
        work?.cancel(); work = nil
        if let error { pending?.resume(throwing: error) }
        else { pending?.resume(returning: String(decoding: output, as: UTF8.self)) }
        output.removeAll()
        await close?()
    }
}


extension HostHelperSetupPlan {
    static var probeScript: String {
        #"""
        D="$HOME/.local/bin/herdcats-bridge"
        if [ -L "$D" ] || { [ -e "$D" ] && [ ! -f "$D" ]; }; then
          printf '%s\n' HC_HELPER_BLOCKED; exit 0
        fi
        B=
        if [ -f "$D" ]; then B="$D"; MARKER=HC_HELPER_MANAGED
        else
          MARKER=HC_HELPER_EXTERNAL
          B=$(command -v herdcats-bridge 2>/dev/null || true)
          if [ -z "$B" ]; then
            for P in "$HOME/.cargo/bin/herdcats-bridge" /opt/homebrew/bin/herdcats-bridge /usr/local/bin/herdcats-bridge; do
              if [ -x "$P" ]; then B="$P"; break; fi
            done
          fi
        fi
        if [ -z "$B" ]; then printf '%s\n' HC_HELPER_MISSING; exit 0; fi
        printf '%s\n' "$MARKER"
        "$B" --version-json 2>/dev/null || printf '%s\n' HC_HELPER_PROBE_FAILED
        """#
    }
    static func verifyInstallation(_ output: String, agent: String? = nil) throws {
        if output.hasPrefix("HC_HELPER_ERROR:") {
            let code = output.dropFirst("HC_HELPER_ERROR:".count).trimmingCharacters(in: .whitespacesAndNewlines)
            throw HostHelperSetupError.failed(code)
        }
        guard output.hasPrefix("HC_HELPER_INSTALLED\n") else { throw HostHelperSetupError.invalidResponse }
        let report = String(output.dropFirst("HC_HELPER_INSTALLED\n".count))
        let probe = try HostHelperProbe(output: "HC_HELPER_MANAGED\n" + report, agent: agent)
        guard case .present(_, true, true) = probe else { throw HostHelperSetupError.invalidResponse }
    }
}

extension HerdrConnection {
    /// User-triggered only. Captures the displayed endpoint and transport before
    /// probing, and revalidates both immediately before a mutating command.
    func setUpHostHelper(
        replace: Bool, checkOnly: Bool, target: ConnectionIdentity, owned: ConnectionGeneration,
        agent: String? = nil,
        reason: HostHelperSetupReason = .missing,
        onStage: @escaping @Sendable (HostHelperSetupStage) async -> Void
    ) async throws -> HostHelperSetupDecision {
        guard route == nil else { throw AgentBridgeError.routedHostUnsupported }
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        let client = try slot.ownedClient(owned)
        guard helperSetupOperation == nil else { throw HostHelperSetupError.busy }
        let operation = UUID()
        helperSetupOperation = operation
        defer { if helperSetupOperation == operation { helperSetupOperation = nil } }
        await onStage(.checking)
        try Task.checkCancellation()
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        let output = try await HostHelperCommand().run(client: client, script: HostHelperSetupPlan.probeScript, timeout: 10)
        try Task.checkCancellation()
        try requireGeneration(owned)
        let probe = try HostHelperProbe(output: output, agent: agent)
        let decision = HostHelperSetupPlan.decision(probe: probe, replace: replace, checkOnly: checkOnly, agent: agent, reason: reason)
        guard case .install(let replaceManaged) = decision else { return decision }
        await onStage(.installing)
        try Task.checkCancellation()
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        let result = try await HostHelperCommand().run(client: client, script: HostHelperSetupPlan.installationScript(replace: replaceManaged), timeout: 180)
        try Task.checkCancellation()
        try requireGeneration(owned)
        await onStage(.verifying)
        try HostHelperSetupPlan.verifyInstallation(result, agent: agent)
        try requireGeneration(owned)
        return .ready
    }

    func disableCodexDaemon(
        target: ConnectionIdentity, owned: ConnectionGeneration, stopRunning: Bool = false
    ) async throws -> CodexDaemonConfigResult {
        guard route == nil else { throw AgentBridgeError.routedHostUnsupported }
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        let client = try slot.ownedClient(owned)
        guard helperSetupOperation == nil else { throw HostHelperSetupError.busy }
        let operation = UUID()
        helperSetupOperation = operation
        defer { if helperSetupOperation == operation { helperSetupOperation = nil } }
        try Task.checkCancellation()
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        let output = try await HostHelperCommand().run(
            client: client,
            script: stopRunning ? Self.codexDisableDaemonStopScript : Self.codexDisableDaemonScript,
            timeout: Self.codexDisableDaemonTimeout(stopRunning: stopRunning)
        )
        try Task.checkCancellation()
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        return try CodexDaemonConfigResult.parse(output)
    }

    /// The bridge's stop pass can take ~60 s (updater wait, two stop passes,
    /// recheck); keep the app-side bound above its own worst case.
    static func codexDisableDaemonTimeout(stopRunning: Bool) -> TimeInterval {
        stopRunning ? 90 : 15
    }

    static var codexDisableDaemonScript: String {
        helperScript(
            exec: "exec \"$B\" codex-config disable-daemon",
            missingPayload: "{\"error\":\"helper_missing\"}",
            readStdinOnMissing: false,
            missingExitCode: 0
        )
    }

    static var codexDisableDaemonStopScript: String {
        helperScript(
            exec: "exec \"$B\" codex-config disable-daemon --stop-running",
            missingPayload: "{\"error\":\"helper_missing\"}",
            readStdinOnMissing: false,
            missingExitCode: 0
        )
    }

    static var codexDisableDaemonCommand: String {
        "exec /bin/sh -c \(shellSafeArgument(codexDisableDaemonScript))"
    }

    static var codexDisableDaemonStopCommand: String {
        "exec /bin/sh -c \(shellSafeArgument(codexDisableDaemonStopScript))"
    }
}

extension HerdrConnection {
    func hooksStatus(agent: ApprovalAgent, target: ConnectionIdentity, owned: ConnectionGeneration) async throws -> AgentHooksResult {
        try await runHooks(agent: agent, operation: .status, target: target, owned: owned)
    }
    func installHooks(agent: ApprovalAgent, target: ConnectionIdentity, owned: ConnectionGeneration) async throws -> AgentHooksResult {
        try await runHooks(agent: agent, operation: .install, target: target, owned: owned)
    }
    func uninstallHooks(agent: ApprovalAgent, target: ConnectionIdentity, owned: ConnectionGeneration) async throws -> AgentHooksResult {
        try await runHooks(agent: agent, operation: .uninstall, target: target, owned: owned)
    }
    private func runHooks(agent: ApprovalAgent, operation: AgentHooksOperation, target: ConnectionIdentity, owned: ConnectionGeneration) async throws -> AgentHooksResult {
        guard route == nil else { throw AgentBridgeError.routedHostUnsupported }
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        let client = try slot.ownedClient(owned)
        guard helperSetupOperation == nil else { throw HostHelperSetupError.busy }
        let token = UUID()
        helperSetupOperation = token
        defer { if helperSetupOperation == token { helperSetupOperation = nil } }
        try Task.checkCancellation()
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        let output = try await HostHelperCommand().run(client: client, script: Self.hooksScript(agent: agent, operation: operation), timeout: 15)
        try Task.checkCancellation()
        try requireGeneration(owned)
        guard identity == target else { throw HerdrError.notConnected }
        return try AgentHooksResult.parse(output)
    }
    static func hooksScript(agent: ApprovalAgent, operation: AgentHooksOperation) -> String {
        helperScript(
            exec: "exec \"$B\" hooks \(operation.rawValue) --agent \(agent.rawValue)",
            missingPayload: "{\"error\":\"helper_missing\"}",
            readStdinOnMissing: false,
            missingExitCode: 0
        )
    }
}
