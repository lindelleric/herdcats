import Citadel
import Crypto
import Foundation
import NIOCore
import NIOSSH

// MARK: - Errors

enum HerdrError: LocalizedError, Equatable {
    case invalidConfiguration(String)
    case notConnected
    case herdrNotFound
    case sessionUnavailable
    case quotaAxiNotFound
    case herdrExit(Int)
    case api(code: String, message: String)
    case unexpectedResponse(String)
    case timeout(String)

    var errorDescription: String? {
        switch self {
        case let .invalidConfiguration(message):
            message
        case .notConnected:
            "Connecting to your Herdr machine…"
        case .herdrNotFound:
            "herdr was not found on the remote machine. Make sure herdr is installed and available in your PATH."
        case .sessionUnavailable:
            "Herdr is installed, but Herdcats could not read its default session. "
                + "Start or check Herdr on the remote machine, then try again."
        case .quotaAxiNotFound:
            "quota-axi was not found on the remote machine. Install with npm i -g quota-axi."
        case let .herdrExit(code):
            "herdr exited with status \(code)."
        case let .api(code, message):
            "Herdr error (\(code)): \(message)"
        case let .unexpectedResponse(detail):
            "Unexpected response from herdr — \(detail)"
        case let .timeout(what):
            "Timed out waiting for \(what). Check your connection and try again."
        }
    }
}

// Unknown keys fail the handshake; approval happens outside the live transport.
struct SSHHostKeyChallenge: Equatable, Sendable {
    let host: String
    let port: Int
    let publicKey: String
    var previousPublicKey: String? = nil

    var previousFingerprint: String? {
        previousPublicKey.map(SSHHostKeyValidatorDelegate.fingerprint)
    }

    var fingerprint: String {
        SSHHostKeyValidatorDelegate.fingerprint(publicKey)
    }
}

enum SSHHostKeyError: LocalizedError, Equatable {
    case unknown(SSHHostKeyChallenge)
    case changed(SSHHostKeyChallenge)

    var errorDescription: String? {
        switch self {
        case let .unknown(challenge):
            "Verify the SSH host key for \(challenge.host):\(challenge.port) before connecting."
        case let .changed(challenge):
            "SSH host key changed for \(challenge.host):\(challenge.port). Connection blocked. "
                + "Saved: \(challenge.previousFingerprint ?? "Unknown"). Received: \(challenge.fingerprint). "
                + "Verify this change with the server administrator through a trusted channel."
        }
    }
}

struct SSHHostKeyValidatorDelegate: NIOSSHClientServerAuthenticationDelegate {
    let host: String
    let port: Int
    let trustedKey: String?

    static func fingerprint(_ publicKey: String) -> String {
        let parts = publicKey.split(separator: " ")
        guard parts.count >= 2, let bytes = Data(base64Encoded: String(parts[1])) else {
            return "Invalid saved host key"
        }
        return "SHA256:" + Data(SHA256.hash(data: bytes)).base64EncodedString()
            .replacingOccurrences(of: "=", with: "")
    }

    func validate(_ publicKey: String) throws {
        guard let trustedKey else {
            throw SSHHostKeyError.unknown(SSHHostKeyChallenge(host: host, port: port, publicKey: publicKey))
        }
        guard publicKey == trustedKey else {
            throw SSHHostKeyError.changed(SSHHostKeyChallenge(
                host: host, port: port, publicKey: publicKey, previousPublicKey: trustedKey
            ))
        }
    }

    func validateHostKey(hostKey: NIOSSHPublicKey, validationCompletePromise: EventLoopPromise<Void>) {
        do {
            try validate(String(openSSHPublicKey: hostKey))
            validationCompletePromise.succeed(())
        } catch {
            validationCompletePromise.fail(error)
        }
    }
}

// MARK: - Connection configuration

struct ConnectionConfig: Sendable, Equatable {
    enum AuthMethod: Equatable, Sendable {
        case password(String)
        /// OpenSSH ed25519 private key (PEM), optionally passphrase-protected.
        case privateKey(String, passphrase: String? = nil)
    }

    var host: String
    var port: Int
    var username: String
    var auth: AuthMethod
}

// MARK: - Connection

enum HerdrCommandTimeoutPolicy: Equatable, Sendable {
    case teardownAndNotify
    case teardownSilently
    case preserveConnection

    func apply(_ teardown: @Sendable (Bool) async -> Void) async {
        switch self {
        case .teardownAndNotify:
            await teardown(true)
        case .teardownSilently:
            await teardown(false)
        case .preserveConnection:
            break
        }
    }
}

/// An SSH session to a machine running herdr, exposed as typed herdr queries.
struct HerdrMachineRoute {
    let gateway: HerdrConnection
    let machine: HerdrMachine
    let generation: ConnectionGeneration
}

struct AgyLoginSession {
    let id: UUID
    let generation: ConnectionGeneration
    let writer: TTYStdinWriter
}

actor HerdrConnection {
    /// A routed connection never owns or closes the gateway transport. Its target
    /// and gateway generation remain fixed for its entire lifetime.
    let route: HerdrMachineRoute?

    init() { route = nil }

    init(
        gateway: HerdrConnection,
        machine: HerdrMachine,
        generation: ConnectionGeneration,
        identity: ConnectionIdentity?
    ) {
        route = HerdrMachineRoute(gateway: gateway, machine: machine, generation: generation)
        self.identity = identity?.scoped(to: machine)
    }

    func connection(for machine: HerdrMachine) throws -> HerdrConnection {
        guard machine.enabled else {
            throw HerdrError.invalidConfiguration("This machine is disabled in Herdr.")
        }
        guard client != nil, route == nil else { throw HerdrError.notConnected }
        return HerdrConnection(gateway: self, machine: machine, generation: generation, identity: identity)
    }

    nonisolated var supportsHostServices: Bool { route == nil }

    func requireHostServices() throws {
        guard route == nil else {
            throw HerdrError.invalidConfiguration(
                "This feature is not yet available through a saved Herdr machine. "
                    + "Switch to the connected Mac to use it."
            )
        }
    }

    var slot = ConnectionClientSlot<SSHClient>()
    var disconnectHandler: (@Sendable () -> Void)?
    /// Exclusive remote composer mutations (send, escape).
    let paneInput = PaneInputCoordinator()
    var agentBridges: [UUID:AgentBridgeClient] = [:]
    private(set) var agentBridgeCapabilities: AgentBridgeCapabilities?
    private(set) var agentBridgeCapabilitiesGeneration: ConnectionGeneration?
    private(set) var agentBridgeVersion: String?
    private(set) var agentBridgeVersionGeneration: ConnectionGeneration?

    func recordAgentBridgeHandshake(capabilities: AgentBridgeCapabilities?, version: String?, generation: ConnectionGeneration) {
        guard slot.matches(generation) else { return }
        self.agentBridgeCapabilities = capabilities
        self.agentBridgeCapabilitiesGeneration = generation
        self.agentBridgeVersion = version
        self.agentBridgeVersionGeneration = generation
    }

    func recordAgentBridgeCapabilities(_ capabilities: AgentBridgeCapabilities?, generation: ConnectionGeneration) {
        recordAgentBridgeHandshake(capabilities: capabilities, version: nil, generation: generation)
    }

    func clearAgentBridgeCapabilities() {
        self.agentBridgeCapabilities = nil
        self.agentBridgeCapabilitiesGeneration = nil
        self.agentBridgeVersion = nil
        self.agentBridgeVersionGeneration = nil
    }

    func bridgeCapabilities(for generation: ConnectionGeneration?) -> AgentBridgeCapabilities? {
        guard let generation, slot.matches(generation), agentBridgeCapabilitiesGeneration == generation else {
            return nil
        }
        return agentBridgeCapabilities
    }

    func bridgeVersion(for generation: ConnectionGeneration?) -> String? {
        guard let generation, slot.matches(generation), agentBridgeVersionGeneration == generation else {
            return nil
        }
        return agentBridgeVersion
    }
    var helperSetupOperation: UUID?
    var liveShells: [String: PaneLiveShells] = [:]
    var preparingLiveShells: [String: UUID] = [:]
    var activeLiveSessions: [String: PaneLiveInputSession] = [:]
    var liveShellUnavailableGeneration: ConnectionGeneration?

    private(set) var host: String = ""
    private(set) var username: String = ""
    private(set) var port: Int = 22
    private(set) var identity: ConnectionIdentity?

    var generation: ConnectionGeneration { slot.generation }
    var client: SSHClient? { slot.current }
    var agyLogin: AgyLoginSession?
    var agyLoginTranscript = AgySignInTranscript()

    /// Reserves a transport generation before dialing so every teardown path
    /// has an identity. Closes any previous client in the background.
    func reserveConnectGeneration() async -> ConnectionGeneration {
        closeAllLiveShells()
        closeAllAgentBridges()
        let (attempt, previous) = slot.beginReplace()
        disconnectHandler = nil
        identity = nil
        if let previous {
            Task.detached(priority: .utility) {
                try? await previous.close()
            }
        }
        return attempt
    }

    /// Completes a connect for a previously reserved generation.
    @discardableResult
    func connect(
        _ config: ConnectionConfig,
        expected attempt: ConnectionGeneration,
        onDisconnect: @escaping @Sendable (ConnectionGeneration) -> Void
    ) async throws -> ConnectionGeneration {
        guard slot.matches(attempt) else {
            throw HerdrError.notConnected
        }

        host = config.host
        username = config.username
        port = config.port
        identity = ConnectionIdentity(config)
        let capturedGeneration = attempt
        disconnectHandler = { onDisconnect(capturedGeneration) }

        let authenticationMethod = try makeAuthenticationMethod(config)
        #if DEBUG
        print("[HC] ssh dialing \(config.host):\(config.port)…")
        #endif

        let trustedKey = try await SSHHostKeyStore.shared.load(host: config.host, port: config.port)
        guard slot.matches(attempt) else { throw HerdrError.notConnected }
        var settings = SSHClientSettings(
            host: config.host,
            port: config.port,
            authenticationMethod: { authenticationMethod },
            hostKeyValidator: .custom(SSHHostKeyValidatorDelegate(
                host: config.host, port: config.port, trustedKey: trustedKey
            ))
        )
        settings.connectTimeout = .seconds(10)

        let client = try await SSHClient.connect(to: settings)
        if let orphan = slot.install(client, expected: attempt) {
            Task.detached(priority: .utility) { try? await orphan.close() }
            throw HerdrError.notConnected
        }
        #if DEBUG
        print("[HC] ssh handshake complete")
        #endif
        client.onDisconnect { [weak self] in
            guard let self else { return }
            Task { await self.handleDisconnect(expected: capturedGeneration) }
        }
        return attempt
    }

    private func makeAuthenticationMethod(_ config: ConnectionConfig) throws -> SSHAuthenticationMethod {
        switch config.auth {
        case let .password(password):
            #if DEBUG
            print("[HC] authenticating as '\(config.username)' with password")
            #endif
            return .passwordBased(username: config.username, password: password)
        case let .privateKey(pem, passphrase):
            let key = try Curve25519.Signing.PrivateKey(openSSHPEM: pem, passphrase: passphrase)
            #if DEBUG
            let pubKeyString = OpenSSHEd25519.openSSHPublicKeyString(for: key.publicKey.rawRepresentation)
            let fingerprint = OpenSSHEd25519.fingerprint(for: key.publicKey.rawRepresentation)
            print(
                "[HC] authenticating as '\(config.username)' with ed25519 key "
                    + "(fingerprint: \(fingerprint), pubkey: \(pubKeyString))"
            )
            #endif
            return .ed25519(username: config.username, privateKey: key)
        }
    }

    /// Invalidates the current generation so in-flight installs fail, then closes.
    func close() async {
        switch slot.invalidateCurrent() {
        case .stale:
            return
        case let .invalidated(oldClient):
            closeAllLiveShells()
            closeAllAgentBridges()
            disconnectHandler = nil
            identity = nil
            if let oldClient {
                try? await oldClient.close()
            }
        }
    }

    /// Invalidates + closes a specific generation. Delayed Tasks must always
    /// carry this identity so they cannot kill a newer attempt.
    nonisolated func forceClose(generation expected: ConnectionGeneration) {
        Task {
            await self.invalidateAndTeardown(expected: expected)
        }
    }

    /// Invalidates `expected` so late connect completions cannot promote, then
    /// closes the captured client. Stale calls leave handlers untouched.
    /// Does **not** notify AppModel — used for explicit user close / stale attempt
    /// teardown where the app already owns the phase transition.
    func invalidateAndTeardown(expected: ConnectionGeneration) {
        switch slot.invalidate(expected) {
        case .stale:
            return
        case let .invalidated(oldClient):
            closeAllLiveShells()
            closeAllAgentBridges()
            disconnectHandler = nil
            if let oldClient {
                Task.detached(priority: .utility) {
                    do {
                        try await oldClient.close()
                    } catch {}
                }
            }
        }
    }

    func handleDisconnect(expected: ConnectionGeneration) {
        switch slot.invalidate(expected) {
        case .stale:
            return
        case .invalidated:
            closeAllLiveShells()
            closeAllAgentBridges()
            let handler = disconnectHandler
            disconnectHandler = nil
            Task.detached { handler?() }
        }
    }

    /// Command timeouts close their owned transport; notification is optional
    /// while a failed connection attempt is still being reported to AppModel.
    /// Stale timeouts leave handlers and UI alone.
    func timeoutTeardown(
        expected: ConnectionGeneration,
        notifyDisconnect: Bool = true
    ) async {
        switch slot.invalidate(expected) {
        case .stale:
            return
        case let .invalidated(oldClient):
            closeAllLiveShells()
            closeAllAgentBridges()
            let handler = notifyDisconnect ? disconnectHandler : nil
            disconnectHandler = nil
            if let oldClient {
                Task.detached(priority: .utility) {
                    do {
                        try await oldClient.close()
                    } catch {}
                }
            }
            Task.detached { handler?() }
        }
    }

    func requireGeneration(_ owned: ConnectionGeneration) throws {
        guard slot.matches(owned), slot.current != nil || route != nil else {
            throw HerdrError.notConnected
        }
    }
}
