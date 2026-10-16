import Foundation
import Testing

@testable import Herdcats

@Suite("Agent timeline connection lifetime")
@MainActor struct AgentTimelineConnectionTests {
    @Test func connectingAndConnectedHaveDifferentStreamKeys() {
        let connecting = AgentTimelineConnectionState(
            phase: .connecting, generation: ConnectionGeneration(rawValue: 2))
        let connected = AgentTimelineConnectionState(
            phase: .connected(host: "host", username: "user"), generation: ConnectionGeneration(rawValue: 2))
        #expect(connecting != connected)
        #expect(!connecting.isReady)
        #expect(connected.isReady)
        #expect(
            connected
                != AgentTimelineConnectionState(
                    phase: .connected(host: "host", username: "user"),
                    generation: ConnectionGeneration(rawValue: 3)))
    }
    @Test func delayedCallbackCannotCommitAfterTransportGenerationChanges() async throws {
        let model = AppModel(autoConnectOnLaunch: false)
        model.phase = .connected(host: "host", username: "user")
        let captured = AgentTimelineConnectionState(
            phase: model.phase, generation: ConnectionGeneration(rawValue: 1))
        let store = AgentTimelineState()
        let owner = UUID()
        store.begin(owner: owner, paneId: "w1:p1")
        let frame = try AgentBridgeFrame.decode(
            Data(
                "{\"event\":\"timeline.snapshot\",\"epoch\":\"e\",\"seq\":1,\"data\":{\"identity\":{\"pane_id\":\"w1:p1\",\"terminal_id\":\"t\",\"agent\":\"claude\",\"native_session_id\":\"s\"},\"generation\":1,\"entries\":[]}}"
                    .utf8))
        let next = AgentTimelineConnectionState(
            phase: model.phase, generation: ConnectionGeneration(rawValue: 2))
        #expect(throws: AgentBridgeError.self) { try captured.requireCurrent(next) }
        // The same main-actor commit gate used by the view rejects a delayed
        // event before touching its store, even if its view token is unchanged.
        do {
            try captured.requireCurrent(next)
            try store.consume(frame, owner: owner)
        } catch {}
        #expect(store.phase == .loading)
        #expect(store.identity == nil)
    }
    @Test func removedBridgeRegistryRejectsQueuedEventBeforeAsyncClose() async {
        let connection = HerdrConnection()
        let bridge = AgentBridgeClient(write: { _ in }, close: {}, onEvent: { _ in })
        await connection.registerBridgeForTest(bridge)
        let generation = await connection.generation
        #expect(await connection.agentBridgeCanDeliver(bridgeId: bridge.id, owned: generation))
        await connection.closeAllAgentBridges()
        #expect(await !connection.agentBridgeCanDeliver(bridgeId: bridge.id, owned: generation))
    }
}
extension HerdrConnection {
    fileprivate func registerBridgeForTest(_ bridge: AgentBridgeClient) { agentBridges[bridge.id] = bridge }
}
