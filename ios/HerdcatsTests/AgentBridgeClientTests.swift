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
