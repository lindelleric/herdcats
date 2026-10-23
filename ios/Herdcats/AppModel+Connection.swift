import Observation
import SwiftUI

extension AppModel {
    func connect(config: ConnectionConfig) async {
        guard !isDialing else { return }
        isDialing = true
        defer { isDialing = false }
        guard let state = await prepareConnectAttempt(config: config) else { return }

        var isCheckingHerdr = false
        do {
            let generation = try await connection.connect(
                config,
                expected: state.reserved,
                onDisconnect: { [weak self] lostGeneration in
                    Task { @MainActor in
                        self?.handleConnectionLost(generation: lostGeneration)
                    }
                }
            )
            if !ownership.noteTransportInstalled(generation, for: state.attempt) {
                connection.forceClose(generation: generation)
                state.watchdog.cancel()
                return
            }
            // SSH setup is bounded by the watchdog. The two Herdr probes have
            // their own command deadlines and must report their own errors.
            state.watchdog.cancel()
            isCheckingHerdr = true
            connectionProgressMessage = "Checking Herdr…"
            print("[HC] ssh connected, checking herdr…")

            let version = try await connection.serverVersion(timeoutPolicy: .teardownSilently)
            print("[HC] herdr version: \(version)")
            if ownership.isCurrent(state.attempt) {
                connectionProgressMessage = "Checking default session…"
            }
            try await checkSessionReadiness()
            guard ownership.isCurrent(state.attempt) else {
                connection.forceClose(generation: generation)
                return
            }
            herdrVersion = version.trimmingCharacters(in: .whitespacesAndNewlines)
            phase = .connected(host: config.host, username: config.username)
            hasActiveSession = true
            cancelReconnect()
            showBackOnline(message: state.wasOffline ? "Back online" : "Connected")
        } catch {
            state.watchdog.cancel()
            handleConnectFailure(
                error,
                config: config,
                attempt: state.attempt,
                wasOffline: state.wasOffline,
                isCheckingHerdr: isCheckingHerdr
            )
        }
    }

    func prepareConnectAttempt(
        config: ConnectionConfig
    ) async -> ConnectAttemptState? {
        hostKeyChallenge = nil
        hostKeyVerificationBlocked = false
        #if DEBUG
        print("[HC] connect start \(config.host):\(config.port)")
        #endif
        resetMachines()
        activeConfig = config
        let wasOffline = isOffline
        phase = .connecting
        if connectionBanner == nil || isOffline {
            connectionBanner = .connecting(message: wasOffline ? "Reconnecting…" : "Connecting…")
        }
        prepareConnectionStatus(for: config)
        let attempt = ownership.beginAttempt()

        // Reserve a transport generation before dialing so every close path
        // has an identity — never unscoped forceClose().
        let reserved = await connection.reserveConnectGeneration()
        guard ownership.associateReserved(reserved, for: attempt) else {
            // Stale attempt — close only this reservation; do not touch newer UI state.
            connection.forceClose(generation: reserved)
            return nil
        }

        let watchdog = Task { @MainActor [weak self] in
            let nanos = self?.connectWatchdogNanoseconds ?? 30_000_000_000
            do {
                try await Task.sleep(nanoseconds: nanos)
            } catch {
                return
            }
            self?.handleConnectWatchdog(attempt: attempt)
        }
        return ConnectAttemptState(
            attempt: attempt,
            reserved: reserved,
            wasOffline: wasOffline,
            watchdog: watchdog
        )
    }

    func handleConnectFailure(
        _ error: Error,
        config: ConnectionConfig,
        attempt: UInt64,
        wasOffline: Bool,
        isCheckingHerdr: Bool
    ) {
        #if DEBUG
        print("[HC] connect failed: \(error)")
        #endif
        logPrivateKeyDiagnostics(config)
        switch ownership.fail(for: attempt) {
        case .stale:
            return
        case let .failed(generation):
            connection.forceClose(generation: generation)
            connectionIdentity = nil
            if let hostKeyError = error as? SSHHostKeyError {
                hostKeyVerificationBlocked = true
                switch hostKeyError {
                case let .unknown(challenge), let .changed(challenge):
                    hostKeyChallenge = challenge
                }
            } else if error is SSHHostKeyStore.StoreError {
                hostKeyVerificationBlocked = true
            }
            if hostKeyVerificationBlocked || error is OpenSSHKeyError { cancelReconnect() }
            let message = HerdrConnection.friendlyMessage(for: error, config: config)
            lastError = message
            herdrMissingOnLastConnect = (error as? HerdrError) == .herdrNotFound
            herdrSessionUnavailableOnLastConnect = (error as? HerdrError) == .sessionUnavailable
            switch Self.connectFailureDisposition(
                error: error,
                hadActiveSession: hasActiveSession,
                wasOffline: wasOffline,
                isCheckingHerdr: isCheckingHerdr
            ) {
            case .readinessRecovery:
                // Key errors require editable credentials, and readiness errors
                // require setup recovery. Clear the session so connection selection
                // is reachable and the launch overlay cannot remain blocking.
                hasActiveSession = false
                cancelReconnect()
                connectionBanner = nil
                phase = .disconnected
            case .offlineRetry:
                phase = .offline(host: config.host, username: config.username)
                connectionBanner = .offline(
                    message: "Offline · \(message)",
                    canRetry: true
                )
                scheduleAutoReconnect(delaySeconds: 5.0)
            case .disconnected:
                phase = .disconnected
            }
        }
    }

    private func logPrivateKeyDiagnostics(_ config: ConnectionConfig) {
        #if DEBUG
        if case let .privateKey(pem, _) = config.auth,
           let pubKey = try? OpenSSHEd25519.parseOpenSSHPublicKeyString(pem: pem),
           let fingerprint = try? OpenSSHEd25519.parseFingerprint(pem: pem) {
            print("[HC] [DEBUG] Auth method: ed25519 private key")
            print("[HC] [DEBUG] Target: \(config.username)@\(config.host):\(config.port)")
            print("[HC] [DEBUG] Key fingerprint: \(fingerprint)")
            print("[HC] [DEBUG] Public key: \(pubKey)")
            print(
                "[HC] [DEBUG] Check: verify that '\(pubKey)' is in ~/.ssh/authorized_keys "
                    + "on '\(config.host)' for user '\(config.username)'."
            )
        }
        #endif
    }

    static func connectFailureDisposition(
        error: Error,
        hadActiveSession: Bool,
        wasOffline: Bool,
        isCheckingHerdr: Bool
    ) -> ConnectFailureDisposition {
        if error is OpenSSHKeyError { return .readinessRecovery }
        let herdrError = error as? HerdrError
        let needsReadinessRecovery = herdrError == .herdrNotFound || herdrError == .sessionUnavailable
        if isCheckingHerdr && (!wasOffline || !hadActiveSession || needsReadinessRecovery) {
            return .readinessRecovery
        }
        if hadActiveSession { return .offlineRetry }
        return .disconnected
    }
}
