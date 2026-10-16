import Citadel
import Crypto
import Foundation
import NIOCore
import NIOSSH

extension HerdrConnection {
    static func paneSendTextArguments(paneId: String, text: String) -> String {
        "pane send-text \(shellSafeArgument(paneId)) \(shellSafeArgument(text))"
    }

    static func paneSendKeysArguments(paneId: String, keys: [String]) -> String {
        let target = shellSafeArgument(paneId)
        let keyArgs = keys.map(shellSafeArgument).joined(separator: " ")
        return "pane send-keys \(target) \(keyArgs)"
    }

    func paneTypeTextUnlocked(
        paneId: String,
        text: String,
        owned: ConnectionGeneration? = nil
    ) async throws {
        if let owned { try requireGeneration(owned) }
        try await runHerdr(
            Self.paneSendTextArguments(paneId: paneId, text: text),
            allowEmptyOutput: true
        )
    }

    /// Sends logical key presses into the pane (`pane send-keys`).
    ///
    /// Keys follow Herdr’s key vocabulary (e.g. `esc`, `ctrl+c`, `enter`).
    func paneSendKeys(paneId: String, _ keys: String...) async throws {
        try await paneSendKeys(paneId: paneId, keys)
    }

    /// Sends logical key presses into the pane (`pane send-keys`).
    func paneSendKeys(paneId: String, _ keys: [String]) async throws {
        let owned = generation
        try await paneInput.run(paneId: paneId) {
            try await self.paneSendKeysUnlocked(paneId: paneId, keys, owned: owned)
        }
    }

    func paneSendKeysUnlocked(
        paneId: String,
        _ keys: [String],
        owned: ConnectionGeneration? = nil
    ) async throws {
        if let owned { try requireGeneration(owned) }
        try await runHerdr(
            Self.paneSendKeysArguments(paneId: paneId, keys: keys),
            allowEmptyOutput: true
        )
    }

    /// Submits a message so the conversation can continue on the Mac pane.
    ///
    /// Prefer `agent prompt` when the pane hosts an agent — it stages text with
    /// the agent's bracketed-paste mode and Enter, including Cursor follow-ups.
    /// Fall back to `pane run` for plain shell panes.
    ///
    /// Codex while `working` is special: its composer paste-burst window treats
    /// Enter immediately after a bracketed paste as a newline, so follow-ups
    /// never submit. Stage with raw `pane send-text`, wait out that window,
    /// then send a real Enter to steer the turn.
    func paneSubmit(
        paneId: String,
        text: String,
        agent: String?,
        status: AgentStatus? = nil
    ) async throws {
        let owned = generation
        try await paneInput.run(paneId: paneId) {
            try await self.paneSubmitUnlocked(
                paneId: paneId, text: text, agent: agent, status: status, owned: owned
            )
        }
    }

    static func shouldTypeAndEnterForTimelinePrompt(agent: String, agentStatus: String?) -> Bool {
        agent == "codex" && agentStatus == "working"
    }

    func paneSubmitWorkingCodexPromptUnlocked(
        paneId: String,
        text: String,
        owned: ConnectionGeneration
    ) async throws {
        try await paneTypeTextUnlocked(paneId: paneId, text: text, owned: owned)
        try await Task.sleep(for: .milliseconds(350))
        try requireGeneration(owned)
        try await paneSendKeysUnlocked(paneId: paneId, ["enter"], owned: owned)
    }

    func paneSubmitUnlocked(
        paneId: String,
        text: String,
        agent: String?,
        status: AgentStatus?,
        owned: ConnectionGeneration
    ) async throws {
        try requireGeneration(owned)
        if Self.shouldTypeAndEnterForTimelinePrompt(agent: agent ?? "", agentStatus: status?.rawValue) {
            try await paneSubmitWorkingCodexPromptUnlocked(paneId: paneId, text: text, owned: owned)
            return
        }
        if agent != nil {
            do {
                try await runHerdr(
                    "agent prompt \(Self.shellSafeArgument(paneId)) \(Self.shellSafeArgument(text))",
                    timeout: 30,
                    allowEmptyOutput: true
                )
            } catch where agent == "codex" && Self.isAgentBlocked(error) {
                // Herdr refuses prompts while blocked; a Codex question may
                // still take a typed answer ("Other" / notes).
                guard try await paneAnswerCodexQuestionUnlocked(
                    paneId: paneId, text: text, owned: owned
                ) else { throw error }
            }
        } else {
            try await paneSendTextUnlocked(paneId: paneId, text: text)
        }
    }

    /// Types `text` as the answer to a blocked Codex question, opening its
    /// answer field first when needed. Returns false, sending nothing, unless
    /// the screen shows a question answer field (never an approval menu).
    func paneAnswerCodexQuestionUnlocked(
        paneId: String,
        text: String,
        owned: ConnectionGeneration
    ) async throws -> Bool {
        let answer = CodexQuestionInput.answerText(text)
        guard !answer.isEmpty else { return false }
        var state = CodexQuestionInput.detect(try await paneReadVisibleText(paneId: paneId))
        try requireGeneration(owned)
        if let keys = state?.openingKeys {
            try await paneSendKeysUnlocked(paneId: paneId, keys, owned: owned)
            try await Task.sleep(for: .milliseconds(250))
            try requireGeneration(owned)
            state = CodexQuestionInput.detect(try await paneReadVisibleText(paneId: paneId))
            try requireGeneration(owned)
        }
        guard state == .answerField else { return false }
        try await paneTypeTextUnlocked(paneId: paneId, text: answer, owned: owned)
        try await Task.sleep(for: .milliseconds(350))
        try requireGeneration(owned)
        try await paneSendKeysUnlocked(paneId: paneId, ["enter"], owned: owned)
        return true
    }

    func paneReadVisibleText(paneId: String) async throws -> String {
        try await runHerdr(
            "pane read \(Self.shellSafeArgument(paneId)) --source visible --lines 60 --format text",
            allowEmptyOutput: true
        )
    }

    static func isAgentBlocked(_ error: Error) -> Bool {
        if case let HerdrError.api(code, _) = error { return code == "agent_blocked" }
        return String(describing: error).contains("agent_blocked")
    }

    /// Uploads image bytes to the Herdr machine and returns the absolute remote path.
    ///
    /// Stores attachments in the remote user's private cache, outside the pane
    /// repository. SFTP creates the final UUID path exclusively.
    func uploadAttachment(
        data: Data,
        fileExtension: String
    ) async throws -> String {
        try requireHostServices()
        guard let client else { throw HerdrError.notConnected }
        guard !data.isEmpty else {
            throw HerdrError.invalidConfiguration("Image data is empty.")
        }
        let owned = generation
        let ext = Self.sanitizedAttachmentExtension(fileExtension)
        let fileName = "\(UUID().uuidString).\(ext)"
        let directory = try await resolveAttachmentDirectory(owned: owned)
        try requireGeneration(owned)
        let remotePath = "\(directory)/\(fileName)"

        try await prepareAttachmentDirectory(directory, owned: owned)
        try requireGeneration(owned)

        print("[HC] sftp upload \(data.count) bytes → \(remotePath)")
        try await withTimeout(
            seconds: 60,
            label: "upload attachment",
            onTimeout: { [weak self] in
                await self?.timeoutTeardown(expected: owned)
            },
            operation: {
            try await client.withSFTP { sftp in
                var attrs = SFTPFileAttributes()
                attrs.permissions = 0o600
                try await sftp.withFile(
                    filePath: remotePath,
                    flags: [.write, .create, .forceCreate],
                    attributes: attrs
                ) { file in
                    try await file.write(ByteBuffer(data: data))
                }
            }
            }
        )
        guard generation == owned else { throw HerdrError.notConnected }
        return remotePath
    }
}

private struct TimelinePromptAgentResult: Decodable {
    let agent: TimelinePromptAgent
}
private struct TimelinePromptAgent: Decodable {
    let paneId: String
    let terminalId: String
    let agent: String
    let agentStatus: String
    let agentSession: Reference?
    struct Reference: Decodable {
        let agent: String
        let source: String
        let kind: String
        let value: String
    }
    enum CodingKeys: String, CodingKey {
        case paneId = "pane_id", terminalId = "terminal_id", agent
        case agentStatus = "agent_status", agentSession = "agent_session"
    }
}
extension HerdrConnection {
    /// Native composition uses the existing prompt API, with a fresh occupant
    /// check. It never falls back to shell input or answers a blocked request.
    func submitTimelinePrompt(text: String, identity: AgentTimelineIdentity, owned: ConnectionGeneration) async throws {
        guard route == nil else {throw AgentBridgeError.routedHostUnsupported}
        try await paneInput.run(paneId: identity.paneId) {
            try await self.requireGeneration(owned)
            let live = try await self.fetch(TimelinePromptAgentResult.self,args:"agent get \(Self.shellSafeArgument(identity.paneId))").agent
            let reference = live.agentSession
            let sessionId = reference?.kind == "path" ? URL(fileURLWithPath:reference?.value ?? "").deletingPathExtension().lastPathComponent : reference?.value
            guard live.paneId == identity.paneId, live.terminalId == identity.terminalId,
                  live.agent == identity.agent, reference?.agent == identity.agent,
                  TimelineAgentAvailability.supports(agent: identity.agent),
                  reference?.source == "herdr:\(identity.agent)", sessionId == identity.nativeSessionId,
                  TimelineComposePolicy.canSend(identity:identity,paneId:live.paneId,status:live.agentStatus,connected:true)
            else {throw AgentBridgeError.identityMismatch}
            try await self.requireGeneration(owned)
            try Task.checkCancellation()
            if Self.shouldTypeAndEnterForTimelinePrompt(agent: identity.agent, agentStatus: live.agentStatus) {
                try await self.paneSubmitWorkingCodexPromptUnlocked(paneId: identity.paneId, text: text, owned: owned)
                return
            }
            _ = try await self.executeHerdr("agent prompt \(Self.shellSafeArgument(identity.paneId)) \(Self.shellSafeArgument(text))",timeout:30,allowEmptyOutput:true,expectedGeneration:owned)
        }
    }
}
