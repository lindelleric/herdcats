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
    @Test func canceledOutstandingRequestClosesOnlyItsChannel() async throws {
        let wire = BridgeTestWire()
        let client = AgentBridgeClient(
            write: { await wire.write($0) }, close: { await wire.close() }, onEvent: { _ in })
        let request = Task {
            try await client.request(.history, params: AgentBridgeParams(generation: 1, before: "a"))
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
    private func report(_ version: String = "0.1.4", protocolNumber: Int = 1) -> String {
        "HC_HELPER_MANAGED\n{\"name\":\"herdcats-bridge\",\"version\":\"\(version)\",\"protocol\":\(protocolNumber),\"herdr_protocol\":22,\"agents\":[\"claude\"]}"
    }
    @Test func currentInstallationNeedsNoDownloadOrReplacement() throws {
        let probe = try HostHelperProbe(output: report())
        #expect(probe == .present(version: "0.1.4", managed: true, compatible: true))
        #expect(HostHelperSetupPlan.decision(probe: probe, replace: true, checkOnly: false) == .ready)
    }
    @Test func unknownAndOldManagedFilesRequireExplicitReplacement() throws {
        for output in [report("0.1.0"), "HC_HELPER_MANAGED\nnot a version report"] {
            let probe = try HostHelperProbe(output: output)
            #expect(HostHelperSetupPlan.decision(probe: probe, replace: false, checkOnly: false) == .replacementRequired(probe))
            #expect(HostHelperSetupPlan.decision(probe: probe, replace: true, checkOnly: false) == .install(replace: true))
        }
    }
    @Test func inspectionNeverInstallsEvenWithReplacementAuthorized() throws {
        let missing = try HostHelperProbe(output: "HC_HELPER_MISSING\n")
        #expect(HostHelperSetupPlan.decision(probe: missing, replace: true, checkOnly: true) == .missing)
        let old = try HostHelperProbe(output: report("0.1.0"))
        #expect(HostHelperSetupPlan.decision(probe: old, replace: true, checkOnly: true) == .replacementRequired(old))
    }
    @Test func externalHelperIsNotOverwrittenAndNewerCompatibleHelperIsNotDowngraded() throws {
        let external = try HostHelperProbe(output: report("0.1.0").replacingOccurrences(of: "MANAGED", with: "EXTERNAL"))
        #expect(HostHelperSetupPlan.decision(probe: external, replace: true, checkOnly: false) == .install(replace: false))
        let newer = try HostHelperProbe(output: report("0.1.5"))
        #expect(HostHelperSetupPlan.decision(probe: newer, replace: true, checkOnly: false) == .ready)
    }
    @Test func incompatibleProtocolDoesNotCountAsInstalledAndUnsafeTargetFailsClosed() throws {
        let probe = try HostHelperProbe(output: report(protocolNumber: 2))
        #expect(probe == .present(version: "0.1.4", managed: true, compatible: false))
        #expect(throws: HostHelperSetupError.unsafeDestination) { try HostHelperProbe(output: "HC_HELPER_BLOCKED\n") }
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
            try HostHelperSetupPlan.verifyInstallation("HC_HELPER_INSTALLED\n{\"name\":\"herdcats-bridge\",\"version\":\"0.1.4\",\"protocol\":2,\"herdr_protocol\":22,\"agents\":[\"claude\"]}")
        }
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
