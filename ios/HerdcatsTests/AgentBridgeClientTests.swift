import Foundation
import Testing

@testable import Herdcats

private actor BridgeTestWire {
    var commands: [Data] = []
    var closes = 0
    func write(_ bytes: Data) { commands.append(bytes) }
    func close() { closes += 1 }
    func requestId() throws -> String {
        let obj = try JSONSerialization.jsonObject(with: commands.last!) as! [String: Any]
        return obj["id"] as! String
    }
}
@Suite("Agent bridge request lifecycle")
struct AgentBridgeClientTests {
    @Test func repliesInterleaveEventsWithoutConsumingTheWrongRequest() async throws {
        let wire = BridgeTestWire()
        let events = BridgeTestWire()
        let client = AgentBridgeClient(
            write: { await wire.write($0) }, close: { await wire.close() },
            onEvent: { frame in await events.write(Data((frame.event ?? "").utf8)) })
        let request = Task { try await client.request(.initialize, params: AgentBridgeParams(protocol: 1)) }
        try await wait(wire)
        let id = try await wire.requestId()
        await client.receive(
            Data(
                "{\"event\":\"timeline.unavailable\",\"epoch\":\"e\",\"seq\":1,\"data\":{\"code\":\"unsupported_agent\"}}\n{\"id\":\"\(id)\",\"result\":{\"protocol\":1}}\n"
                    .utf8))
        #expect(try await request.value.protocol == 1)
        #expect(await events.commands.count == 1)
        await client.close()
        #expect(await wire.closes == 1)
    }
    @Test func canceledWatchRequestClosesOnlyItsChannel() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(
            write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        let request = Task {
            try await client.request(.watch, params: AgentBridgeParams(paneId: "p1"))
        }
        try await wait(wire)
        request.cancel()
        do {
            _ = try await request.value
            Issue.record("canceled request completed")
        } catch {}
        #expect(await wire.closes == 1)
        #expect(await wire.commands.count == 1)
        await client.close()
        #expect(await wire.closes == 1)
    }
    @Test func idleWaitCancellationClosesStreamWithoutAnotherOutputByte() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(
            write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        let waiting = Task { try await client.waitUntilClosed() }
        await Task.yield()
        waiting.cancel()
        do {
            try await waiting.value
            Issue.record("canceled stream remained open")
        } catch {}
        // Cancellation schedules async transport teardown; allow it to complete.
        for _ in 0..<500 {
            if await wire.closes > 0 { break }
            try await Task.sleep(for: .milliseconds(2))
        }
        #expect(await wire.closes == 1)
    }
    @Test func timeoutFailsPendingRequestsAndRejectsFutureWrites() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(
            write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        do {
            _ = try await client.request(.watch, params: AgentBridgeParams(paneId: "w1:p1"), timeout: 0.01)
            Issue.record("request should time out")
        } catch { #expect(error as? AgentBridgeError == .timeout) }
        do {
            _ = try await client.request(.watch, params: AgentBridgeParams(paneId: "w1:p1"))
            Issue.record("closed stream accepted another write")
        } catch {}
        #expect(await wire.commands.count == 1)
        #expect(await wire.closes == 1)
    }
    @Test func failedInitializeWriteStillDrainsMissingHelperBootstrap() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(write: {
            await wire.write($0)
            throw AgentBridgeError.closed
        }, close: { await wire.close() }, onEvent: { _ in })
        let pending = Task { try await client.request(.initialize, params: AgentBridgeParams(protocol: 1)) }
        try await wait(wire)
        try await Task.sleep(for: .milliseconds(10))
        await client.receive(Data("{\"error\":{\"code\":\"helper_missing\"}}\n".utf8))
        do {
            _ = try await pending.value
            Issue.record("Missing helper accepted")
        } catch { #expect(error as? AgentBridgeError == .unavailable("helper_missing")) }
        #expect(await wire.closes == 1)
    }
    @Test func missingHelperBootstrapFailsWithSetupReason() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        let pending = Task { try await client.request(.initialize, params: AgentBridgeParams(protocol: 1)) }
        try await wait(wire)
        await client.receive(Data("{\"error\":{\"code\":\"helper_missing\"}}\n".utf8))
        do {
            _ = try await pending.value
            Issue.record("Missing helper accepted")
        } catch { #expect(error as? AgentBridgeError == .unavailable("helper_missing")) }
        #expect(await wire.closes == 1)
    }
    @Test func invalidWireClosesAndDoesNotExposeContentInError() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(
            write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        let pending = Task { try await client.request(.initialize, params: AgentBridgeParams(protocol: 1)) }
        try await wait(wire)
        await client.receive(Data("private non-json text\n".utf8))
        do {
            _ = try await pending.value
            Issue.record("bad wire accepted")
        } catch {
            #expect(error as? AgentBridgeError == .invalidFrame)
            #expect(!error.localizedDescription.contains("private"))
        }
        #expect(await wire.closes == 1)
    }
    @Test func routedHostIsRejectedBeforeOpeningAnyHelper() async {
        let gateway = HerdrConnection()
        let machine = HerdrMachine(
            id: "child", label: "Child", target: "u@child", session: "default", enabled: true)
        let route = HerdrConnection(
            gateway: gateway, machine: machine, generation: ConnectionGeneration(rawValue: 1), identity: nil)
        do {
            _ = try await route.openAgentTimeline(paneId: "p", onEvent: { _ in })
            Issue.record("routed stream accepted")
        } catch { #expect(error as? AgentBridgeError == .routedHostUnsupported) }
    }
    @Test func handshakeOptsIntoApprovals() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        let task = Task { try await client.performHandshakeAndWatch(paneId: "p1") }
        try await wait(wire)
        let bytes = await wire.commands[0]
        let request = try #require(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
        #expect(request["params"] as? NSDictionary == ["protocol": 1, "record_content": true, "approvals": true] as NSDictionary)
        let id = try await wire.requestId()
        await client.receive(Data("{\"id\":\"\(id)\",\"result\":{\"protocol\":1,\"epoch\":\"e\",\"capabilities\":{\"claude_timeline\":true,\"record_content\":true,\"approvals\":true}}}\n".utf8))
        for _ in 0..<500 {
            if await wire.commands.count == 2 { break }
            try await Task.sleep(for: .milliseconds(2))
        }
        #expect(await wire.commands.count == 2)
        let watchId = try await wire.requestId()
        await client.receive(Data("{\"id\":\"\(watchId)\",\"result\":{}}\n".utf8))
        try await task.value
        #expect(await client.capabilities?.approvals == true)
        await client.close()
    }
    @Test func approvalResponseUsesExactRowsAndDecodesConfirmation() async throws {
        for confirmed in [true, false] {
            let wire = BridgeTestWire()
            let client = AgentBridgeClient(write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
            let task = Task { try await client.respond(requestId: "approval", option: 2, labelLines: ["Yes", "for this session"]) }
            try await wait(wire)
            let bytes = await wire.commands[0]
            let request = try #require(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
            #expect(request["method"] as? String == "interaction.respond")
            #expect(request["params"] as? NSDictionary == ["request_id": "approval", "option": 2, "label_lines": ["Yes", "for this session"]] as NSDictionary)
            let id = try await wire.requestId()
            await client.receive(Data("{\"id\":\"\(id)\",\"result\":{\"request_id\":\"approval\",\"sent\":\"a\",\"confirmed\":\(confirmed)}}\n".utf8))
            #expect(try await task.value == AgentApprovalResponse(requestId: "approval", sent: "a", confirmed: confirmed))
            await client.close()
        }
    }
    @Test func approvalResponseErrorsKeepDistinctCodes() async throws {
        for code in ["stale", "mismatch", "unsupported_capability", "not_watching", "timeline_unavailable"] {
            let wire = BridgeTestWire()
            let client = AgentBridgeClient(write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
            let task = Task { try await client.respond(requestId: "approval", option: 1, labelLines: ["Yes"]) }
            try await wait(wire)
            let id = try await wire.requestId()
            await client.receive(Data("{\"id\":\"\(id)\",\"error\":{\"code\":\"\(code)\"}}\n".utf8))
            do {
                _ = try await task.value
                Issue.record("Approval refusal accepted")
            } catch { #expect((error as? AgentBridgeError)?.code == code) }
            #expect(await wire.closes == 0)
            await client.close()
        }
    }
    private func wait(_ wire: BridgeTestWire) async throws {
        for _ in 0..<500 {
            if await !wire.commands.isEmpty { return }
            try await Task.sleep(for: .milliseconds(2))
        }
        throw AgentBridgeError.timeout
    }
}

@Suite("Host helper explicit setup policy")
struct HostHelperSetupPolicyTests {
    private func report(_ version: String = "0.2.0", protocolNumber: Int = 1) -> String {
        "HC_HELPER_MANAGED\n{\"name\":\"herdcats-bridge\",\"version\":\"\(version)\",\"protocol\":\(protocolNumber),\"herdr_protocol\":22,\"agents\":[\"claude\",\"codex\"]}"
    }
    @Test func currentInstallationNeedsNoDownloadOrReplacement() throws {
        let probe = try HostHelperProbe(output: report())
        #expect(probe == .present(version: "0.2.0", managed: true, compatible: true))
        #expect(HostHelperSetupPlan.decision(probe: probe, replace: true, checkOnly: false) == .ready)
    }
    @Test func unknownAndOldManagedFilesRequireExplicitReplacement() throws {
        for output in [report("0.1.4"), report("0.1.0"), "HC_HELPER_MANAGED\nnot a version report"] {
            let probe = try HostHelperProbe(output: output)
            #expect(HostHelperSetupPlan.decision(probe: probe, replace: false, checkOnly: false) == .replacementRequired(probe))
            #expect(HostHelperSetupPlan.decision(probe: probe, replace: true, checkOnly: false) == .install(replace: true))
        }
    }
    @Test func inspectionNeverInstallsEvenWithReplacementAuthorized() throws {
        let missing = try HostHelperProbe(output: "HC_HELPER_MISSING\n")
        #expect(HostHelperSetupPlan.decision(probe: missing, replace: true, checkOnly: true) == .missing)
        let old = try HostHelperProbe(output: report("0.1.4"))
        #expect(HostHelperSetupPlan.decision(probe: old, replace: true, checkOnly: true) == .replacementRequired(old))
    }
    @Test func externalHelperIsNotOverwrittenAndNewerCompatibleHelperIsNotDowngraded() throws {
        let external = try HostHelperProbe(output: report("0.1.4").replacingOccurrences(of: "MANAGED", with: "EXTERNAL"))
        #expect(HostHelperSetupPlan.decision(probe: external, replace: true, checkOnly: false) == .install(replace: false))
        let newer = try HostHelperProbe(output: report("0.2.2"))
        #expect(HostHelperSetupPlan.decision(probe: newer, replace: true, checkOnly: false) == .ready)
    }
    @Test func incompatibleProtocolDoesNotCountAsInstalledAndUnsafeTargetFailsClosed() throws {
        let probe = try HostHelperProbe(output: report(protocolNumber: 2))
        #expect(probe == .present(version: "0.2.0", managed: true, compatible: false))
        #expect(throws: HostHelperSetupError.unsafeDestination) { try HostHelperProbe(output: "HC_HELPER_BLOCKED\n") }
    }
    @Test func unsupportedAgentInPinnedInstallerRefusesInstallOrReplace() throws {
        let missing = try HostHelperProbe(output: "HC_HELPER_MISSING\n", agent: "cursor")
        #expect(HostHelperSetupPlan.decision(probe: missing, replace: true, checkOnly: false, agent: "cursor") == .unsupportedByPinnedInstaller("cursor"))
        #expect(HostHelperSetupPlan.decision(probe: missing, replace: false, checkOnly: true, agent: "cursor") == .unsupportedByPinnedInstaller("cursor"))

        let oldHelperForCursor = try HostHelperProbe(output: report("0.1.4"), agent: "cursor")
        #expect(HostHelperSetupPlan.decision(probe: oldHelperForCursor, replace: true, checkOnly: false, agent: "cursor") == .unsupportedByPinnedInstaller("cursor"))
        #expect(HostHelperSetupPlan.decision(probe: oldHelperForCursor, replace: false, checkOnly: false, agent: "cursor") == .unsupportedByPinnedInstaller("cursor"))
    }
    @Test func codexAndClaudeSetupDecisions() throws {
        // Codex pane with no helper offers install
        let missingCodex = try HostHelperProbe(output: "HC_HELPER_MISSING\n", agent: "codex")
        #expect(HostHelperSetupPlan.decision(probe: missingCodex, replace: false, checkOnly: false, agent: "codex") == .install(replace: false))
        #expect(HostHelperSetupPlan.decision(probe: missingCodex, replace: true, checkOnly: false, agent: "codex") == .install(replace: false))
        #expect(HostHelperSetupPlan.decision(probe: missingCodex, replace: false, checkOnly: true, agent: "codex") == .missing)

        // A 0.1.4 host on a Codex pane shows helper_update_required and offers update (install with replace)
        let old014ForCodex = try HostHelperProbe(
            output: "HC_HELPER_MANAGED\n{\"name\":\"herdcats-bridge\",\"version\":\"0.1.4\",\"protocol\":1,\"herdr_protocol\":22,\"agents\":[\"claude\"]}",
            agent: "codex"
        )
        #expect(old014ForCodex == .present(version: "0.1.4", managed: true, compatible: false))
        #expect(HostHelperSetupPlan.decision(probe: old014ForCodex, replace: false, checkOnly: false, agent: "codex") == .replacementRequired(old014ForCodex))
        #expect(HostHelperSetupPlan.decision(probe: old014ForCodex, replace: true, checkOnly: false, agent: "codex") == .install(replace: true))
        #expect(HostHelperSetupPlan.decision(probe: old014ForCodex, replace: false, checkOnly: true, agent: "codex") == .replacementRequired(old014ForCodex))

        // Claude setup flow/probe unchanged
        let missingClaude = try HostHelperProbe(output: "HC_HELPER_MISSING\n", agent: "claude")
        #expect(HostHelperSetupPlan.decision(probe: missingClaude, replace: false, checkOnly: false, agent: "claude") == .install(replace: false))
        #expect(HostHelperSetupPlan.decision(probe: missingClaude, replace: true, checkOnly: false, agent: "claude") == .install(replace: false))

        // A 0.2.0 helper compatible with both claude and codex
        let report020 = "HC_HELPER_MANAGED\n{\"name\":\"herdcats-bridge\",\"version\":\"0.2.0\",\"protocol\":1,\"herdr_protocol\":22,\"agents\":[\"claude\",\"codex\"]}"
        let compatibleCodex = try HostHelperProbe(output: report020, agent: "codex")
        #expect(compatibleCodex == .present(version: "0.2.0", managed: true, compatible: true))
        #expect(HostHelperSetupPlan.decision(probe: compatibleCodex, replace: false, checkOnly: false, agent: "codex") == .ready)

        let compatibleClaude = try HostHelperProbe(output: report020, agent: "claude")
        #expect(compatibleClaude == .present(version: "0.2.0", managed: true, compatible: true))
        #expect(HostHelperSetupPlan.decision(probe: compatibleClaude, replace: false, checkOnly: false, agent: "claude") == .ready)
    }

    @Test func pinnedReleaseIs022WithVerifiedInstallerChecksum() {
        #expect(HostHelperSetupPlan.version == "0.2.2")
        #expect(HostHelperSetupPlan.minimumVersion == "0.2.0")
        #expect(HostHelperSetupPlan.installerSHA256 == "be2ae7715ffe3ef793565e4a171fe36c47fb562bb62f595bdcca3a8f49119289")
        #expect(HostHelperSetupPlan.agents == ["claude", "codex"])
        let install = HostHelperSetupPlan.installationScript(replace: false)
        #expect(install.contains("herdcats-bridge-v0.2.2/install.sh"))
        #expect(install.contains(HostHelperSetupPlan.installerSHA256))
        #expect(!install.contains("--replace"))
        #expect(HostHelperSetupPlan.installationScript(replace: true).contains("--replace"))
        #expect(HostHelperSetupPlan.manualInstallCommand(reason: .missing)
            == "curl -fsSL https://github.com/zhiyao/herdcats/releases/download/herdcats-bridge-v0.2.2/install.sh | sh -s -- --version 0.2.2")
        #expect(HostHelperSetupPlan.manualInstallCommand(reason: .updateRequired).hasSuffix(" --replace"))
    }

    @Test func helperSetupLabelAndFeedbackLogic() throws {
        // update reason idle -> "Upgrade to 0.2.2"
        #expect(
            HostHelperSetupPlan.installLabel(stage: .idle, reason: .updateRequired)
            == "Upgrade to 0.2.2"
        )
        #expect(
            HostHelperSetupPlan.installLabel(stage: .idle, reason: .updateRequired, pinnedVersion: "0.2.2")
            == "Upgrade to 0.2.2"
        )

        // busy labels
        #expect(
            HostHelperSetupPlan.installLabel(stage: .checking)
            == "Checking host…"
        )
        #expect(
            HostHelperSetupPlan.installLabel(stage: .installing, reason: .updateRequired)
            == "Upgrading Herdcats Bridge…"
        )
        #expect(
            HostHelperSetupPlan.installLabel(stage: .installing, reason: .missing)
            == "Installing Herdcats Bridge…"
        )
        #expect(
            HostHelperSetupPlan.installLabel(stage: .verifying)
            == "Verifying installation…"
        )

        // missing -> "Install Herdcats Bridge"
        #expect(
            HostHelperSetupPlan.installLabel(stage: .idle, reason: .missing)
            == "Install Herdcats Bridge"
        )
        #expect(
            HostHelperSetupPlan.installLabel(stage: .idle)
            == "Install Herdcats Bridge"
        )

        // managed 0.1.4 replacementRequired -> "Replace Herdcats Bridge with 0.2.2" + found-version feedback
        let managed014 = HostHelperProbe.present(version: "0.1.4", managed: true, compatible: false)
        #expect(
            HostHelperSetupPlan.installLabel(stage: .replacementRequired(managed014), pinnedVersion: "0.2.2")
            == "Replace Herdcats Bridge with 0.2.2"
        )
        #expect(
            HostHelperSetupPlan.installLabel(stage: .replacementRequired(managed014))
            == "Replace Herdcats Bridge with 0.2.2"
        )
        #expect(
            HostHelperSetupPlan.replacementFeedback(probe: managed014)
            == "Found Herdcats Bridge 0.1.4 on this host. Tap Replace to upgrade it at the location shown."
        )

        // external helper -> "Install managed Herdcats Bridge"
        let externalHelper = HostHelperProbe.present(version: "0.1.4", managed: false, compatible: false)
        #expect(
            HostHelperSetupPlan.installLabel(stage: .replacementRequired(externalHelper))
            == "Install managed Herdcats Bridge"
        )
        #expect(
            HostHelperSetupPlan.replacementFeedback(probe: externalHelper)
            == "An existing Herdcats Bridge was found elsewhere. Install a managed copy at the location shown; the other copy will remain unchanged."
        )

        // HostHelperSetupError localized descriptions
        #expect(
            HostHelperSetupError.invalidResponse.localizedDescription
            == "Herdcats Bridge could not be verified. Check the installation or use manual setup."
        )
        #expect(
            HostHelperSetupError.busy.localizedDescription
            == "A Herdcats Bridge setup is already running on this connection. Check again when it finishes."
        )
        #expect(
            HostHelperSetupError.failed("curl_missing").localizedDescription
            == "This host needs curl to download Herdcats Bridge. Use manual setup to install it."
        )
    }

    @Test func oneTapUpgradeDecisionPath() {
        let managed014 = HostHelperProbe.present(version: "0.1.4", managed: true, compatible: false)
        let managed020 = HostHelperProbe.present(version: "0.2.0", managed: true, compatible: true)
        let managed021 = HostHelperProbe.present(version: "0.2.1", managed: true, compatible: true)
        let managed022 = HostHelperProbe.present(version: "0.2.2", managed: true, compatible: true)
        let managed030Compatible = HostHelperProbe.present(version: "0.3.0", managed: true, compatible: true)
        let managed030Incompatible = HostHelperProbe.present(version: "0.3.0", managed: true, compatible: false)
        let managedUnknown = HostHelperProbe.present(version: nil, managed: true, compatible: false)
        let external014 = HostHelperProbe.present(version: "0.1.4", managed: false, compatible: false)
        let external020 = HostHelperProbe.present(version: "0.2.0", managed: false, compatible: true)

        // 1. managed 0.1.4 + updateRequired -> install(replace: true)
        #expect(
            HostHelperSetupPlan.decision(probe: managed014, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .install(replace: true)
        )
        #expect(
            HostHelperSetupPlan.decision(probe: managed014, replace: false, checkOnly: false, agent: "claude", reason: .updateRequired)
            == .install(replace: true)
        )

        // 1b. managed 0.2.0 + updateRequired -> install(replace: true) (one-tap upgrade to 0.2.2)
        #expect(
            HostHelperSetupPlan.decision(probe: managed020, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .install(replace: true)
        )
        // managed 0.2.0 + missing -> ready (compatible with minimumVersion, no drawer)
        #expect(
            HostHelperSetupPlan.decision(probe: managed020, replace: false, checkOnly: false, agent: "codex", reason: .missing)
            == .ready
        )

        // 1c. managed 0.2.1 + updateRequired -> install(replace: true) (one-tap upgrade to pinned 0.2.2)
        #expect(
            HostHelperSetupPlan.decision(probe: managed021, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .install(replace: true)
        )

        // 1d. managed 0.2.2 + updateRequired -> ready (already pinned version)
        #expect(
            HostHelperSetupPlan.decision(probe: managed022, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .ready
        )

        // 2. managed 0.3.0 compatible -> ready (no downgrade offer)
        #expect(
            HostHelperSetupPlan.decision(probe: managed030Compatible, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .ready
        )
        // managed 0.3.0 incompatible + updateRequired -> replacementRequired (explicit replace required)
        #expect(
            HostHelperSetupPlan.decision(probe: managed030Incompatible, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .replacementRequired(managed030Incompatible)
        )

        // 3. managed unknown version + updateRequired -> replacementRequired
        #expect(
            HostHelperSetupPlan.decision(probe: managedUnknown, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .replacementRequired(managedUnknown)
        )

        // 4. external old -> stops and shows replacementRequired even for update reason
        #expect(
            HostHelperSetupPlan.decision(probe: external014, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .replacementRequired(external014)
        )
        // external 0.2.0 + updateRequired -> replacementRequired (explicit confirmation required for unmanaged)
        #expect(
            HostHelperSetupPlan.decision(probe: external020, replace: false, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .replacementRequired(external020)
        )
        // when confirmed with replace (managed: false -> replace: false)
        #expect(
            HostHelperSetupPlan.decision(probe: external014, replace: true, checkOnly: false, agent: "codex", reason: .updateRequired)
            == .install(replace: false)
        )

        // 5. missing reason + managed old -> explicit Replace required (first tap returns replacementRequired)
        #expect(
            HostHelperSetupPlan.decision(probe: managed014, replace: false, checkOnly: false, agent: "codex", reason: .missing)
            == .replacementRequired(managed014)
        )
        // second tap with replace returns install(replace: true)
        #expect(
            HostHelperSetupPlan.decision(probe: managed014, replace: true, checkOnly: false, agent: "codex", reason: .missing)
            == .install(replace: true)
        )

        // 6. checkOnly returns replacementRequired regardless of reason
        #expect(
            HostHelperSetupPlan.decision(probe: managed014, replace: false, checkOnly: true, agent: "codex", reason: .updateRequired)
            == .replacementRequired(managed014)
        )

        // 7. HostHelperProbe helper verifications
        #expect(HostHelperProbe.isOlderThanPinned("0.1.4"))
        #expect(HostHelperProbe.isOlderThanPinned("0.2.0"))
        #expect(HostHelperProbe.isOlderThanPinned("0.2.1"))
        #expect(!HostHelperProbe.isOlderThanPinned("0.2.2"))
        #expect(!HostHelperProbe.isOlderThanPinned("0.3.0"))
        #expect(!HostHelperProbe.isOlderThanPinned(nil))
        #expect(!HostHelperProbe.isOlderThanPinned("invalid"))

        #expect(HostHelperProbe.isAtLeastPinned("0.2.2"))
        #expect(HostHelperProbe.isAtLeastPinned("0.3.0"))
        #expect(!HostHelperProbe.isAtLeastPinned("0.2.1"))
        #expect(!HostHelperProbe.isAtLeastPinned("0.2.0"))
        #expect(!HostHelperProbe.isAtLeastPinned("0.1.4"))
        #expect(!HostHelperProbe.isAtLeastPinned(nil))

        #expect(HostHelperProbe.isAtLeastMinimum("0.2.0"))
        #expect(HostHelperProbe.isAtLeastMinimum("0.2.1"))
        #expect(HostHelperProbe.isAtLeastMinimum("0.3.0"))
        #expect(!HostHelperProbe.isAtLeastMinimum("0.1.4"))
        #expect(!HostHelperProbe.isAtLeastMinimum(nil))

        #expect(HostHelperProbe.isOlderThanMinimum("0.1.4"))
        #expect(!HostHelperProbe.isOlderThanMinimum("0.2.0"))
        #expect(!HostHelperProbe.isOlderThanMinimum("0.2.1"))
        #expect(!HostHelperProbe.isOlderThanMinimum("0.3.0"))
        #expect(!HostHelperProbe.isOlderThanMinimum(nil))
    }
}

@Suite("Isolated helper command lifecycle")
struct HostHelperCommandTests {
    @Test func commandReturnsBoundedOutputAndClosesOnlyItsOwnedChannel() async throws {
        let channel = HostHelperCommand()
        let owned = BridgeTestWire()
        let shared = BridgeTestWire()
        let output = try await channel.run(timeout: 1) { command in
            _ = await command.attach { await owned.close() }
            await command.receive(Data("verified helper".utf8))
        }
        #expect(output == "verified helper")
        try await waitForClose(owned)
        #expect(await owned.closes == 1)
        #expect(await shared.closes == 0)
    }
    @Test func oversizedOutputClosesOwnedChannel() async throws {
        let channel = HostHelperCommand()
        let owned = BridgeTestWire()
        do {
            _ = try await channel.run(timeout: 1) { command in
                _ = await command.attach { await owned.close() }
                await command.receive(Data(repeating: 65, count: 65537))
            }
            Issue.record("unbounded installer output accepted")
        } catch { #expect(error as? HostHelperSetupError == .invalidResponse) }
        try await waitForClose(owned)
        #expect(await owned.closes == 1)
    }
    @Test func timeoutClosesOwnedChannelWhileReaderIsIdle() async throws {
        let channel = HostHelperCommand()
        let owned = BridgeTestWire()
        do {
            _ = try await channel.run(timeout: 0.03) { command in
                _ = await command.attach { await owned.close() }
                try await Task.sleep(for: .seconds(10))
            }
            Issue.record("timeout accepted")
        } catch {}
        try await waitForClose(owned)
        #expect(await owned.closes == 1)
    }
    @Test func cancellationBeforeLateAttachClosesItAndCannotCompleteSuccessfully() async throws {
        let channel = HostHelperCommand()
        let owned = BridgeTestWire()
        let pending = Task {
            try await channel.run(timeout: 1) { _ in
                await owned.write(Data("started".utf8))
                try await Task.sleep(for: .seconds(10))
            }
        }
        for _ in 0..<500 {
            if await !owned.commands.isEmpty { break }
            try await Task.sleep(for: .milliseconds(2))
        }
        try #require(await !owned.commands.isEmpty)
        pending.cancel()
        do { _ = try await pending.value; Issue.record("cancelled install accepted") } catch {}
        let attached = await channel.attach { await owned.close() }
        #expect(!attached)
        #expect(await owned.closes == 1)
    }
    private func waitForClose(_ wire: BridgeTestWire) async throws {
        for _ in 0..<500 {
            if await wire.closes > 0 { return }
            try await Task.sleep(for: .milliseconds(2))
        }
        throw AgentBridgeError.timeout
    }
}


@Suite("Helper setup host and completion guards")
struct HostHelperSetupGuardTests {
    @Test func routedHostCannotStartInstaller() async {
        let gateway = HerdrConnection()
        let machine = HerdrMachine(id: "child", label: "Child", target: "u@child", session: "default", enabled: true)
        let route = HerdrConnection(gateway: gateway, machine: machine, generation: ConnectionGeneration(rawValue: 1), identity: nil)
        do {
            _ = try await route.setUpHostHelper(replace: true, checkOnly: false,
                target: ConnectionIdentity(host: "child", port: 22, username: "u"), owned: ConnectionGeneration(rawValue: 1),
                onStage: { _ in Issue.record("routed install started") })
            Issue.record("routed installer accepted")
        } catch { #expect(error as? AgentBridgeError == .routedHostUnsupported) }
    }
    @Test func invalidCompletionAndReportedFailureNeverBecomeSuccess() throws {
        #expect(throws: HostHelperSetupError.failed("checksum_mismatch")) {
            try HostHelperSetupPlan.verifyInstallation("HC_HELPER_ERROR:checksum_mismatch\n")
        }
        #expect(throws: HostHelperSetupError.invalidResponse) {
            try HostHelperSetupPlan.verifyInstallation("HC_HELPER_INSTALLED\n{\"name\":\"herdcats-bridge\",\"version\":\"0.2.0\",\"protocol\":2,\"herdr_protocol\":22,\"agents\":[\"claude\",\"codex\"]}")
        }
        #expect(throws: HostHelperSetupError.invalidResponse) {
            try HostHelperSetupPlan.verifyInstallation("HC_HELPER_INSTALLED\n{\"name\":\"herdcats-bridge\",\"version\":\"0.1.4\",\"protocol\":1,\"herdr_protocol\":22,\"agents\":[\"claude\"]}")
        }
    }
    @Test func verifyInstallationAccepts020ReportWithClaudeAndCodex() throws {
        let output = "HC_HELPER_INSTALLED\n{\"name\":\"herdcats-bridge\",\"version\":\"0.2.0\",\"protocol\":1,\"herdr_protocol\":22,\"agents\":[\"claude\",\"codex\"]}\n"
        try HostHelperSetupPlan.verifyInstallation(output)
        try HostHelperSetupPlan.verifyInstallation(output, agent: "claude")
        try HostHelperSetupPlan.verifyInstallation(output, agent: "codex")
    }
}

@Suite("Timeline content request isolation")
struct TimelineContentClientTests {
    private func wait(_ wire: BridgeTestWire, count:Int) async throws {
        for _ in 0..<500 {
            if await wire.commands.count >= count { return }
            try await Task.sleep(for:.milliseconds(2))
        }
        throw AgentBridgeError.timeout
    }
    @Test func chunksReassembleExactBytesAndVerifyIdentity() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(write:{ await wire.write($0) },close:{ await wire.close() },onEvent:{_ in})
        let identity = AgentTimelineIdentity(paneId:"p1",terminalId:"t1",agent:"claude",nativeSessionId:"s")
        let fetch = Task { try await client.content(reference:"source:0:4",generation:1,identity:identity) }
        for (index,hex) in ["e78c","ab21"].enumerated() {
            try await wait(wire,count:index+1)
            let id = try await wire.requestId()
            let data:[String:Any] = ["id":id,"result":["identity":["pane_id":"p1","terminal_id":"t1","agent":"claude","native_session_id":"s"],
                "generation":1,"content_ref":"source:0:4","offset":index*2,"next_offset":index*2+2,"total":4,"hex":hex]]
            var bytes = try JSONSerialization.data(withJSONObject:data);bytes.append(10);await client.receive(bytes)
        }
        #expect(try await fetch.value == Data("猫!".utf8))
        await client.close()
    }
    @Test func cancellingContentDoesNotCloseTimelineAndLateReplyIsDrained() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(write:{ await wire.write($0) },close:{ await wire.close() },onEvent:{_ in})
        let request = Task { try await client.request(.content,params:AgentBridgeParams(generation:1)) }
        try await wait(wire,count:1)
        let oldId = try await wire.requestId();request.cancel()
        do { _ = try await request.value; Issue.record("Canceled content completed") } catch {}
        #expect(await wire.closes == 0)
        await client.receive(Data("{\"id\":\"\(oldId)\",\"result\":{}}\n".utf8))
        let next = Task { try await client.request(.initialize,params:AgentBridgeParams(protocol:1)) }
        try await wait(wire,count:2)
        let id = try await wire.requestId()
        await client.receive(Data("{\"id\":\"\(id)\",\"result\":{\"protocol\":1}}\n".utf8))
        #expect(try await next.value.protocol == 1)
        await client.close()
    }
}

@Suite("Timeline history request isolation")
struct TimelineHistoryClientTests {
    private func wait(_ wire: BridgeTestWire, count: Int) async throws {
        for _ in 0..<500 {
            if await wire.commands.count >= count { return }
            try await Task.sleep(for: .milliseconds(2))
        }
        throw AgentBridgeError.timeout
    }

    @Test @MainActor
    func cancelledRefreshWaiterStillMergesTheLatePageAndCoalescesPulls() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        let loader = TimelineHistoryLoader()
        let state = AgentTimelineState()
        let owner = UUID()
        state.begin(owner: owner, paneId: "p1")
        let identity: [String: Any] = ["pane_id": "p1", "terminal_id": "t1", "agent": "claude", "native_session_id": "s1"]
        let snapshot: [String: Any] = ["event": "timeline.snapshot", "epoch": "e", "seq": 1,
            "data": ["identity": identity, "generation": 1, "has_more": true,
                     "entries": [["id": "current", "kind": "message", "text": "Current", "truncated": false]]]]
        try state.consume(AgentBridgeFrame.decode(JSONSerialization.data(withJSONObject: snapshot)), owner: owner)
        let refresh = Task {
            await loader.load {
                do {
                    let page = try await client.request(.history, params: AgentBridgeParams(generation: 1, before: "current"))
                    try Task.checkCancellation()
                    try state.mergeHistory(page, owner: owner, generation: 1)
                } catch { Issue.record("Owned page load failed: \(error)") }
            }
        }
        try await wait(wire, count: 1)
        let id = try await wire.requestId()
        refresh.cancel()
        let concurrentPull = Task {
            await loader.load { Issue.record("A second pull started a duplicate page request") }
        }
        await Task.yield()
        #expect(await wire.closes == 0)
        let response: [String: Any] = ["id": id, "result": ["identity": identity, "generation": 1, "has_more": false,
            "entries": [["id": "earlier", "kind": "message", "text": "Earlier", "truncated": false]]]]
        var bytes = try JSONSerialization.data(withJSONObject: response)
        bytes.append(10)
        await client.receive(bytes)
        await refresh.value
        await concurrentPull.value
        #expect(state.entries.map(\.id) == ["earlier", "current"])
        #expect(!state.hasMore)
        #expect(await wire.commands.count == 1)
        #expect(await wire.closes == 0)
        await client.close()
    }

    @Test @MainActor
    func leavingTimelineCancelsOwnedPageWithoutClosingTheWatch() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        let loader = TimelineHistoryLoader()
        let refresh = Task {
            await loader.load {
                do {
                    _ = try await client.request(.history, params: AgentBridgeParams(generation: 1, before: "a"))
                    Issue.record("Cancelled page load completed")
                } catch { #expect(Task.isCancelled) }
            }
        }
        try await wait(wire, count: 1)
        loader.cancel()
        await refresh.value
        #expect(await wire.closes == 0)
        await client.close()
    }

    @Test(arguments: [false, true])
    func cancelledOrTimedOutHistoryKeepsLiveEventsAndRetryWorking(timesOut: Bool) async throws {
        let wire = BridgeTestWire()
        let events = BridgeTestWire()
        let client = AgentBridgeClient(
            write: { await wire.write($0) }, close: { await wire.close() },
            onEvent: { frame in await events.write(Data((frame.event ?? "").utf8)) })
        let request = Task {
            try await client.request(.history, params: AgentBridgeParams(generation: 1, before: "a"),
                                     timeout: timesOut ? 0.05 : 5)
        }
        try await wait(wire, count: 1)
        let oldId = try await wire.requestId()
        if !timesOut { request.cancel() }
        do {
            _ = try await request.value
            Issue.record("Abandoned history request completed")
        } catch {
            #expect(error as? AgentBridgeError == (timesOut ? .timeout : .closed))
        }
        #expect(await wire.closes == 0)
        // A late response belongs to the abandoned page, not a broken channel.
        await client.receive(Data("{\"id\":\"\(oldId)\",\"result\":{}}\n".utf8))
        await client.receive(Data("{\"event\":\"timeline.updated\",\"epoch\":\"e\",\"seq\":1,\"data\":{}}\n".utf8))
        #expect(await events.commands.count == 1)
        let retry = Task { try await client.request(.history, params: AgentBridgeParams(generation: 1, before: "a")) }
        try await wait(wire, count: 2)
        let id = try await wire.requestId()
        await client.receive(Data("{\"id\":\"\(id)\",\"result\":{\"generation\":1,\"entries\":[],\"has_more\":false}}\n".utf8))
        #expect(try await retry.value.generation == 1)
        #expect(await wire.closes == 0)
        await client.close()
    }
}
