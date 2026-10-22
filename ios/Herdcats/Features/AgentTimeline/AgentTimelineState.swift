import Foundation
import Observation

/// Memory-only state for one view/stream owner. Replies from retired owners
/// cannot restore old transcript data after pane changes or backgrounding.
@MainActor @Observable final class AgentTimelineState {
    enum Phase: Equatable {
        case loading, ready
        case unavailable(String)
        case disconnected
    }
    private(set) var pendingQuestion: AgentPendingQuestion?
    private(set) var pendingApproval: AgentPendingApproval?
    private(set) var entries: [AgentTimelineEntry] = []
    private(set) var identity: AgentTimelineIdentity?
    private(set) var phase: Phase = .loading
    private(set) var generation: UInt64?
    private(set) var hasMore = false
    private(set) var before: String?
    private(set) var agentStatus: String?
    private(set) var unknownRecords = 0
    private(set) var droppedEntries = 0
    private(set) var delayed = false
    /// `codex_daemon` carried by the latest unavailable event (running /
    /// not_running / unknown); nil otherwise, including unrecognized values.
    private(set) var codexDaemon: String?
    private(set) var revision: UInt64 = 0
    private var owner: UUID?
    private var paneId = ""
    private var epoch: String?
    private var sequence: UInt64 = 0
    private var evictedIds: Set<String> = []

    func begin(owner: UUID, paneId: String) {
        self.owner = owner
        self.paneId = paneId
        epoch = nil
        sequence = 0
        clear()
        phase = .loading
    }
    func consume(_ frame: AgentBridgeFrame, owner: UUID) throws {
        guard self.owner == owner else { return }
        guard let event = frame.event, let nextEpoch = frame.epoch, let seq = frame.seq, let data = frame.data
        else { throw AgentBridgeError.invalidFrame }
        guard epoch == nil || epoch == nextEpoch, seq == sequence + 1 else {
            throw AgentBridgeError.sequenceGap
        }
        epoch = nextEpoch
        sequence = seq
        switch event {
        case "timeline.unavailable":
            guard data.paneId == paneId, let next = data.generation else {
                throw AgentBridgeError.identityMismatch
            }
            clear()
            generation = next
            codexDaemon = Self.normalizedCodexDaemon(data.codexDaemon)
            phase = .unavailable(data.code ?? "timeline_unavailable")
        case "timeline.snapshot":
            guard let nextIdentity = data.identity, nextIdentity.paneId == paneId,
                TimelineAgentAvailability.supports(agent: nextIdentity.agent), let next = data.generation, let rows = data.entries
            else { throw AgentBridgeError.identityMismatch }
            try validate(rows)
            clear()
            identity = nextIdentity
            generation = next
            entries = rows
            hasMore = data.hasMore ?? false
            before = data.before
            agentStatus = data.agentStatus
            unknownRecords = data.unknownRecords ?? 0
            droppedEntries = data.droppedEntries ?? 0
            delayed = data.delayed ?? false
            phase = .ready
            revision += 1
        case "timeline.updated":
            guard phase == .ready, data.identity == identity, data.generation == generation,
                let rows = data.entries
            else { throw AgentBridgeError.identityMismatch }
            try validate(rows)
            let removed = data.evictedIds ?? []
            guard removed.count <= 1000, removed.allSatisfy({ !$0.isEmpty && $0.utf8.count <= 264 }) else {
                throw AgentBridgeError.invalidFrame
            }
            let removedSet = Set(removed)
            guard evictedIds.union(removedSet).count <= 100_000 else { throw AgentBridgeError.invalidFrame }
            evictedIds.formUnion(removedSet)
            entries.removeAll { removedSet.contains($0.id) }
            if let cursor = before, removedSet.contains(cursor) {
                before = entries.first?.id
                hasMore = false
            }
            for row in rows where !evictedIds.contains(row.id) {
                if let index = entries.firstIndex(where: { $0.id == row.id }) {
                    entries[index] = row
                } else {
                    entries.append(row)
                }
            }
            if entries.count > 1000 {
                entries.removeFirst(entries.count - 1000)
                // Older local entries were discarded; history can still be
                // requested relative to the first retained entity.
                hasMore = true
                before = entries.first?.id
            }
            agentStatus = data.agentStatus ?? agentStatus
            unknownRecords = data.unknownRecords ?? unknownRecords
            droppedEntries = data.droppedEntries ?? droppedEntries
            delayed = data.delayed ?? delayed
            revision += 1
        case "interaction.pending":
            guard phase == .ready, data.identity == identity, data.generation == generation else {
                throw AgentBridgeError.identityMismatch
            }
            if data.kind == "question" {
                pendingQuestion = try AgentPendingQuestion.decode(data)
                pendingApproval = nil
            } else {
                pendingApproval = try AgentPendingApproval.decode(data)
                pendingQuestion = nil
            }
            revision += 1
        case "interaction.resolved":
            guard let requestId = data.requestId, !requestId.isEmpty, requestId.utf8.count <= 264 else {
                throw AgentBridgeError.invalidFrame
            }
            if let pending = pendingApproval, pending.requestId == requestId,
                pending.identity == data.identity, pending.generation == data.generation {
                pendingApproval = nil
                revision += 1
            }
            if let pending = pendingQuestion, pending.requestId == requestId,
                pending.identity == data.identity, pending.generation == data.generation {
                pendingQuestion = nil
                revision += 1
            }
        default: throw AgentBridgeError.invalidFrame
        }
    }
    func mergeHistory(_ page: AgentBridgePayload, owner: UUID, generation: UInt64) throws {
        guard self.owner == owner, self.generation == generation else { return }
        guard page.generation == generation, page.identity == identity, let rows = page.entries else {
            throw AgentBridgeError.identityMismatch
        }
        try validate(rows)
        let currentIds = Set(entries.map(\.id))
        let prefix = rows.filter { !currentIds.contains($0.id) && !evictedIds.contains($0.id) }
        // Never let a delayed history page overwrite a streamed revision.
        entries = Array((prefix + entries).suffix(1000))
        if !rows.contains(where: { evictedIds.contains($0.id) }) {
            hasMore = (page.hasMore ?? false) && entries.count < 1000
        }
        before = entries.first?.id
        droppedEntries = page.droppedEntries ?? droppedEntries
    }
    func end(owner: UUID, error: AgentBridgeError) {
        guard self.owner == owner else { return }
        self.owner = nil
        clear()
        phase = error == .closed ? .disconnected : .unavailable(error.code)
    }
    private func clear() {
        pendingQuestion = nil
        pendingApproval = nil
        evictedIds.removeAll()
        entries = []
        identity = nil
        generation = nil
        before = nil
        hasMore = false
        agentStatus = nil
        unknownRecords = 0
        droppedEntries = 0
        delayed = false
        codexDaemon = nil
        revision += 1
    }

    /// Older bridges omit `codex_daemon`; future values the app does not know
    /// stay nil so the pane never offers an action from an unverified state.
    static func normalizedCodexDaemon(_ value: String?) -> String? {
        guard ["running", "not_running", "unknown"].contains(value ?? "") else { return nil }
        return value
    }
    private func validate(_ rows: [AgentTimelineEntry]) throws {
        guard rows.count <= 1000, Set(rows.map(\.id)).count == rows.count,
            rows.allSatisfy({
                !$0.id.isEmpty && $0.id.utf8.count <= 264 && ["message", "tool", "image", "record"].contains($0.kind)
                    && $0.text.utf8.count <= 8192 && ($0.details?.utf8.count ?? 0) <= 8192
                    && ($0.contentRef.map { TimelineContentReference.length($0) != nil } ?? !["image", "record"].contains($0.kind))
                    && ($0.recordRefs?.count ?? 0) <= 4
                    && ($0.recordRefs ?? []).allSatisfy { TimelineContentReference.length($0) != nil }
            })
        else { throw AgentBridgeError.invalidFrame }
    }
}

/// The page belongs to the timeline, not SwiftUI's short-lived refresh gesture.
/// Concurrent pulls share a request; leaving the timeline explicitly cancels it.
@MainActor final class TimelineHistoryLoader {
    private var task: Task<Void, Never>?
    private var token: UUID?

    func load(_ operation: @escaping @MainActor () async -> Void) async {
        if let task {
            await task.value
            return
        }
        let token = UUID()
        let task = Task { await operation() }
        self.token = token
        self.task = task
        await task.value
        if self.token == token {
            self.task = nil
            self.token = nil
        }
    }

    func cancel() {
        task?.cancel()
        task = nil
        token = nil
    }
}
