import Foundation

// MARK: - App-level connect ownership (used by AppModel)

/// Monotonic connect-attempt ownership shared by AppModel production code and tests.
struct ConnectionSessionOwnership: Equatable, Sendable {
    enum WatchdogOutcome: Equatable, Sendable {
        case stale
        case timedOut(generation: ConnectionGeneration)
    }

    enum FailOutcome: Equatable, Sendable {
        case stale
        case failed(generation: ConnectionGeneration)
    }

    private(set) var attempt: UInt64 = 0
    private(set) var transportGeneration: ConnectionGeneration?

    @discardableResult
    mutating func beginAttempt() -> UInt64 {
        attempt += 1
        transportGeneration = nil
        return attempt
    }

    func isCurrent(_ token: UInt64) -> Bool {
        attempt == token
    }

    /// Associates a reserved transport generation before handshake completes so
    /// every teardown path has an identity (never unscoped `forceClose()`).
    mutating func associateReserved(
        _ generation: ConnectionGeneration,
        for token: UInt64
    ) -> Bool {
        guard isCurrent(token) else { return false }
        transportGeneration = generation
        return true
    }

    /// Confirms the reserved generation after handshake install.
    mutating func noteTransportInstalled(
        _ generation: ConnectionGeneration,
        for token: UInt64
    ) -> Bool {
        guard isCurrent(token) else { return false }
        guard transportGeneration == generation else { return false }
        return true
    }

    /// Watchdog: invalidate attempt first, then return the reserved generation.
    mutating func watchdogTimeout(for token: UInt64) -> WatchdogOutcome {
        guard isCurrent(token), let generation = transportGeneration else {
            return .stale
        }
        attempt += 1
        transportGeneration = nil
        return .timedOut(generation: generation)
    }

    /// Failure while this attempt is current. Invalidates the attempt so a late
    /// success cannot promote, and returns the reserved generation to close.
    mutating func fail(for token: UInt64) -> FailOutcome {
        guard isCurrent(token), let generation = transportGeneration else {
            return .stale
        }
        attempt += 1
        transportGeneration = nil
        return .failed(generation: generation)
    }

    mutating func disconnect() -> ConnectionGeneration? {
        attempt += 1
        let generation = transportGeneration
        transportGeneration = nil
        return generation
    }

    /// Transport loss: drop generation and invalidate the attempt so a delayed
    /// `serverVersion` success cannot promote `.connected`.
    mutating func handleLoss(generation: ConnectionGeneration) -> Bool {
        guard transportGeneration == generation else { return false }
        transportGeneration = nil
        attempt += 1
        return true
    }
}

// MARK: - Transport client slot (used by HerdrConnection)

enum ClientSlotInvalidateResult<Client> {
    /// `expected` no longer owns the slot — do not clear handlers or close.
    case stale
    /// Slot invalidated (generation bumped). Close `client` if non-nil.
    case invalidated(client: Client?)
}

/// Generation-scoped holder for the live SSH client.
struct ConnectionClientSlot<Client> {
    private(set) var generation = ConnectionGeneration(rawValue: 0)
    private var client: Client?

    var current: Client? { client }

    /// Bumps generation, clears the slot, returns attempt + previous client to close.
    mutating func beginReplace() -> (attempt: ConnectionGeneration, previous: Client?) {
        let attempt = generation.bump()
        let previous = client
        client = nil
        return (attempt, previous)
    }

    /// Installs only when `expected` still owns the slot. Returns orphan to close.
    mutating func install(_ newClient: Client, expected: ConnectionGeneration) -> Client? {
        guard generation == expected else { return newClient }
        client = newClient
        return nil
    }

    /// Capture + clear + bump so late installs for `expected` fail.
    mutating func invalidate(_ expected: ConnectionGeneration) -> ClientSlotInvalidateResult<Client> {
        guard generation == expected else { return .stale }
        let old = client
        client = nil
        generation.bump()
        return .invalidated(client: old)
    }

    /// Invalidate whatever is current (used when closing "the" session).
    mutating func invalidateCurrent() -> ClientSlotInvalidateResult<Client> {
        invalidate(generation)
    }

    func matches(_ expected: ConnectionGeneration) -> Bool {
        generation == expected
    }
}

// MARK: - Refresh ownership (used by SpacesModel)

struct RefreshGenerationGate: Equatable, Sendable {
    private(set) var generation: UInt64 = 0
    private(set) var isLoading = false

    mutating func begin() -> UInt64 {
        generation += 1
        isLoading = true
        return generation
    }

    mutating func watchdogTimeout(for token: UInt64) -> Bool {
        guard generation == token, isLoading else { return false }
        generation += 1
        isLoading = false
        return true
    }

    mutating func finishIfCurrent(_ token: UInt64) -> Bool {
        guard generation == token else { return false }
        isLoading = false
        return true
    }

    func isCurrent(_ token: UInt64) -> Bool {
        generation == token
    }
}

extension ConnectionClientSlot {
    func ownedClient(_ expected:ConnectionGeneration) throws -> Client {
        guard matches(expected), let client=current else{throw HerdrError.notConnected}
        return client
    }
}
