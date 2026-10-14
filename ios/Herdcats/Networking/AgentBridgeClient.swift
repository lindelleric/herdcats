import Citadel
import Foundation
import NIOCore

struct AgentBridgeParams: Encodable, Sendable {
    var `protocol`: Int? = nil
    var paneId: String? = nil
    var generation: UInt64? = nil
    var before: String? = nil
}
enum AgentBridgeMethod: String, Sendable { case initialize, watch, history }

/// One non-PTY channel, with bounded request correlation and no agent mutations.
/// Cancellation closes this channel explicitly: canceling an async iterator
/// alone cannot be relied upon to wake an idle SSH read.
actor AgentBridgeClient {
    nonisolated let id: UUID
    private struct Pending {
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
    private var framer = AgentBridgeFramer()
    private var streamTask: Task<Void, Never>?
    private var opening: CheckedContinuation<Void, Error>?
    private var openTimeout: Task<Void, Never>?
    private var closedError: AgentBridgeError?
    private var waiters: [CheckedContinuation<Void, Error>] = []

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
    func open(client: SSHClient, command: String, paneId: String) async throws {
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
            let handshake = try await request(.initialize, params: AgentBridgeParams(protocol: 1))
            guard handshake.protocol == 1, handshake.epoch?.isEmpty == false else {
                throw AgentBridgeError.protocolMismatch
            }
            guard handshake.capabilities?.claudeTimeline == true else {
                throw AgentBridgeError.unavailable("transcript_root_unavailable")
            }
            _ = try await request(.watch, params: AgentBridgeParams(paneId: paneId))
            try Task.checkCancellation()
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
    func request(_ method: AgentBridgeMethod, params: AgentBridgeParams, timeout: TimeInterval = 5)
        async throws -> AgentBridgePayload
    {
        try await withTaskCancellationHandler {
            try Task.checkCancellation()
            guard closedError == nil, let write else { throw closedError ?? .closed }
            guard pending.count < 16 else { throw AgentBridgeError.unavailable("too_many_requests") }
            let id = UUID().uuidString
            let encoder = JSONEncoder()
            encoder.keyEncodingStrategy = .convertToSnakeCase
            var bytes = try encoder.encode(Request(id: id, method: method.rawValue, params: params))
            bytes.append(10)
            guard bytes.count <= 1024 * 1024 else { throw AgentBridgeError.frameTooLarge }
            return try await withCheckedThrowingContinuation { continuation in
                let timeoutTask = Task {
                    do { try await Task.sleep(for: .seconds(timeout)) } catch { return }
                    await self.stop(.timeout)
                }
                pending[id] = Pending(continuation: continuation, timeout: timeoutTask)
                Task {
                    do { try await write(bytes) } catch {
                        // Startup stdout can already contain helper_missing when
                        // the remote process exits. Drain it before classifying;
                        // stream completion or the request timeout still closes.
                        if method != .initialize { await self.stop(.closed) }
                    }
                }
            }
        } onCancel: {
            Task { await self.close() }
        }
    }
    func receive(_ bytes: Data) async {
        guard closedError == nil else { return }
        do {
            for frame in try framer.append(bytes) {
                guard closedError == nil else { return }
                if let id = frame.id {
                    guard frame.event == nil, let request = pending.removeValue(forKey: id) else {
                        throw AgentBridgeError.invalidFrame
                    }
                    request.timeout.cancel()
                    if let error = frame.error {
                        request.continuation.resume(throwing: AgentBridgeError.unavailable(error.code))
                    } else if let result = frame.result {
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
        paneId: String, onEvent: @escaping @Sendable (AgentBridgeFrame) async throws -> Void
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
            try await bridge.open(client: client, command: Self.agentBridgeCommand, paneId: paneId)
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
        Task { for bridge in bridges { await bridge.close() } }
    }
    static var agentBridgeCommand: String {
        let script = """
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
              IFS= read -r HB_REQUEST || exit 127
              printf '%s\\n' '{"error":{"code":"helper_missing"}}'
              exit 127
            fi
            S=${HERDR_SOCKET_PATH:-${XDG_CONFIG_HOME:-$HOME/.config}/herdr/herdr.sock}
            exec "$B" serve --stdio --socket "$S"
            """
        return "exec /bin/sh -c \(shellSafeArgument(script))"
    }
}

extension HerdrConnection {
    func agentBridgeCanDeliver(bridgeId: UUID, owned: ConnectionGeneration) -> Bool {
        slot.matches(owned) && agentBridges[bridgeId] != nil
    }
}
