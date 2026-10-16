import Citadel
import Crypto
import Foundation
import NIOCore
import NIOSSH

struct WorktreeOpenCommand {
    let workspaceId: String?
    let cwd: String?
    let path: String?
    let branch: String?
    let label: String?
    let focus: Bool
}

struct WorktreeCreateCommand {
    let workspaceId: String?
    let cwd: String?
    let branch: String?
    let base: String?
    let path: String?
    let label: String?
    let focus: Bool
}

extension HerdrConnection {
    // MARK: - Command execution

    /// Runs a herdr subcommand over SSH and returns its raw stdout.
    ///
    /// The remote wrapper resolves the herdr binary (SSH exec sessions do not
    /// inherit the interactive shell PATH, so common install locations are
    /// probed explicitly), merges stderr for error reporting, and always exits
    /// zero so that transport-level failures can be told apart from herdr
    /// errors via in-band markers.
    func runHerdr(
        _ args: String, timeout: TimeInterval = 20, allowEmptyOutput: Bool = false,
        timeoutPolicy: HerdrCommandTimeoutPolicy = .teardownAndNotify
    ) async throws -> String {
        if let route {
            return try await route.gateway.runMachineCommand(
                args, machine: route.machine, expected: route.generation,
                timeout: timeout, allowEmptyOutput: allowEmptyOutput
            )
        }
        return try await executeHerdr(
            args, timeout: timeout, allowEmptyOutput: allowEmptyOutput,
            timeoutPolicy: timeoutPolicy
        )
    }

    func runMachineCommand(
        _ args: String, machine: HerdrMachine, expected: ConnectionGeneration,
        timeout: TimeInterval, allowEmptyOutput: Bool
    ) async throws -> String {
        try requireGeneration(expected)
        return try await executeHerdr(
            Self.machineArguments(args, machineID: machine.id), timeout: timeout,
            allowEmptyOutput: allowEmptyOutput, timeoutPolicy: .preserveConnection
        )
    }

    static func machineArguments(_ args: String, machineID: String) -> String {
        "--machine \(shellSafeArgument(machineID)) \(args)"
    }

    /// Keeps arbitrary CLI arguments, which may contain user messages, out of
    /// logs and timeout errors.
    static let herdrDiagnosticLabel = "herdr command"

    func executeHerdr(
        _ args: String, timeout: TimeInterval, allowEmptyOutput: Bool,
        timeoutPolicy: HerdrCommandTimeoutPolicy = .teardownAndNotify,
        expectedGeneration: ConnectionGeneration? = nil
    ) async throws -> String {
#if DEBUG && targetEnvironment(simulator)
        if ScreenshotFixtures.enabled { return try ScreenshotFixtures.response(args) }
#endif
        let owned = expectedGeneration ?? generation
        // Validate ownership and capture the client in this same actor turn.
        // A caller's earlier generation check cannot authorize a new client.
        let client = try slot.ownedClient(owned)
        let command = Self.remoteCommand(args)
        let diagnosticLabel = Self.herdrDiagnosticLabel
        print("[HC] exec: \(diagnosticLabel)")
        let buffer = try await withTimeout(
            seconds: timeout,
            label: diagnosticLabel,
            onTimeout: { [weak self] in
                await timeoutPolicy.apply { notifyDisconnect in
                    await self?.timeoutTeardown(expected: owned, notifyDisconnect: notifyDisconnect)
                }
            },
            operation: { try await client.executeCommand(command, maxResponseSize: 8 * 1024 * 1024) }
        )
        // A newer generation may have replaced the client while we ran.
        guard generation == owned else { throw HerdrError.notConnected }
        let text = String(buffer: buffer)
        if allowEmptyOutput && text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return ""
        }
        return try Self.interpret(text)
    }

    /// Runs a herdr subcommand and decodes its JSON envelope.
    func fetch<Result: Decodable>(
        _ resultType: Result.Type, args: String,
        timeoutPolicy: HerdrCommandTimeoutPolicy = .teardownAndNotify
    ) async throws -> Result {
        let output = try await runHerdr(args, timeoutPolicy: timeoutPolicy)
        return try Self.decode(output, as: resultType)
    }

    // MARK: - Typed herdr queries

    func machineList() async throws -> [HerdrMachine] {
        try requireHostServices()
        let output = try await executeHerdr(
            "machine list --json", timeout: 10, allowEmptyOutput: false,
            timeoutPolicy: .preserveConnection
        )
        return try JSONDecoder().decode([HerdrMachine].self, from: Data(output.utf8))
    }

    func serverVersion(
        timeoutPolicy: HerdrCommandTimeoutPolicy = .teardownAndNotify
    ) async throws -> String {
        try await runHerdr("--version", timeoutPolicy: timeoutPolicy)
    }

    func workspaceList(
        timeoutPolicy: HerdrCommandTimeoutPolicy = .teardownAndNotify
    ) async throws -> [Workspace] {
        try await fetch(
            WorkspaceListResult.self, args: "workspace list",
            timeoutPolicy: timeoutPolicy
        ).workspaces
    }

    /// Creates a new workspace, optionally rooted at `cwd`, and returns the
    /// created workspace with its initial tab and root pane when reported.
    @discardableResult
    func workspaceCreate(cwd: String? = nil, focus: Bool = false) async throws -> WorkspaceCreateResult {
        try await fetch(
            WorkspaceCreateResult.self,
            args: Self.workspaceCreateArguments(cwd: cwd, focus: focus)
        )
    }

    /// Builds `workspace create` arguments. A nil or empty `cwd` lets herdr
    /// pick the default directory.
    static func workspaceCreateArguments(cwd: String?, focus: Bool) -> String {
        var args = "workspace create"
        if let cwd, !cwd.isEmpty {
            args += " --cwd \(shellSafeArgument(cwd))"
        }
        args += focus ? " --focus" : " --no-focus"
        return args
    }

    /// Renames a workspace. Empty / whitespace-only labels are rejected.
    @discardableResult
    func workspaceRename(workspaceId: String, label: String) async throws -> Workspace {
        try await fetch(
            WorkspaceInfoResult.self,
            args: Self.workspaceRenameArguments(workspaceId: workspaceId, label: label)
        ).workspace
    }

    static func workspaceRenameArguments(workspaceId: String, label: String) -> String {
        "workspace rename \(shellSafeArgument(workspaceId)) \(shellSafeArgument(label))"
    }

    /// Closes a workspace and tears down its tabs and panes.
    func workspaceClose(workspaceId: String) async throws {
        _ = try await fetch(
            HerdrOkResult.self,
            args: Self.workspaceCloseArguments(workspaceId: workspaceId)
        )
    }

    static func workspaceCloseArguments(workspaceId: String) -> String {
        "workspace close \(shellSafeArgument(workspaceId))"
    }

    // MARK: - Worktree commands

    func worktreeList(workspaceId: String? = nil, cwd: String? = nil) async throws -> WorktreeListResult {
        try await fetch(
            WorktreeListResult.self,
            args: Self.worktreeListArguments(workspaceId: workspaceId, cwd: cwd)
        )
    }

    static func worktreeListArguments(workspaceId: String?, cwd: String?) -> String {
        var args = "worktree list"
        if let workspaceId, !workspaceId.isEmpty {
            args += " --workspace \(shellSafeArgument(workspaceId))"
        } else if let cwd, !cwd.isEmpty {
            args += " --cwd \(shellSafeArgument(cwd))"
        }
        return args
    }

    @discardableResult
    func worktreeOpen(
        workspaceId: String? = nil,
        cwd: String? = nil,
        path: String? = nil,
        branch: String? = nil,
        label: String? = nil,
        focus: Bool = false
    ) async throws -> WorktreeOpenResult {
        try await fetch(
            WorktreeOpenResult.self,
            args: Self.worktreeOpenArguments(WorktreeOpenCommand(
                workspaceId: workspaceId,
                cwd: cwd,
                path: path,
                branch: branch,
                label: label,
                focus: focus
            ))
        )
    }

    static func worktreeOpenArguments(_ command: WorktreeOpenCommand) -> String {
        var args = "worktree open"
        if let workspaceId = command.workspaceId, !workspaceId.isEmpty {
            args += " --workspace \(shellSafeArgument(workspaceId))"
        } else if let cwd = command.cwd, !cwd.isEmpty {
            args += " --cwd \(shellSafeArgument(cwd))"
        }
        if let path = command.path, !path.isEmpty {
            args += " --path \(shellSafeArgument(path))"
        }
        if let branch = command.branch, !branch.isEmpty {
            args += " --branch \(shellSafeArgument(branch))"
        }
        if let label = command.label, !label.isEmpty {
            args += " --label \(shellSafeArgument(label))"
        }
        args += command.focus ? " --focus" : " --no-focus"
        return args
    }

    @discardableResult
    func worktreeCreate(
        workspaceId: String? = nil,
        cwd: String? = nil,
        branch: String? = nil,
        base: String? = nil,
        path: String? = nil,
        label: String? = nil,
        focus: Bool = false
    ) async throws -> WorktreeCreateResult {
        try await fetch(
            WorktreeCreateResult.self,
            args: Self.worktreeCreateArguments(WorktreeCreateCommand(
                workspaceId: workspaceId,
                cwd: cwd,
                branch: branch,
                base: base,
                path: path,
                label: label,
                focus: focus
            ))
        )
    }

    static func worktreeCreateArguments(_ command: WorktreeCreateCommand) -> String {
        var args = "worktree create"
        if let workspaceId = command.workspaceId, !workspaceId.isEmpty {
            args += " --workspace \(shellSafeArgument(workspaceId))"
        } else if let cwd = command.cwd, !cwd.isEmpty {
            args += " --cwd \(shellSafeArgument(cwd))"
        }
        if let branch = command.branch, !branch.isEmpty {
            args += " --branch \(shellSafeArgument(branch))"
        }
        if let base = command.base, !base.isEmpty {
            args += " --base \(shellSafeArgument(base))"
        }
        if let path = command.path, !path.isEmpty {
            args += " --path \(shellSafeArgument(path))"
        }
        if let label = command.label, !label.isEmpty {
            args += " --label \(shellSafeArgument(label))"
        }
        args += command.focus ? " --focus" : " --no-focus"
        return args
    }

    /// Removes a worktree checkout on the remote machine. `force` deletes
    /// even with uncommitted changes; `trustRepository` is required when the
    /// checkout is the repository's main checkout.
    func worktreeRemove(
        workspaceId: String,
        force: Bool = false,
        trustRepository: Bool = false
    ) async throws {
        _ = try await fetch(
            HerdrOkResult.self,
            args: Self.worktreeRemoveArguments(
                workspaceId: workspaceId,
                force: force,
                trustRepository: trustRepository
            )
        )
    }

    static func worktreeRemoveArguments(
        workspaceId: String,
        force: Bool,
        trustRepository: Bool
    ) -> String {
        var args = "worktree remove --workspace \(shellSafeArgument(workspaceId))"
        if force { args += " --force" }
        if trustRepository { args += " --trust-repository" }
        return args
    }
}
