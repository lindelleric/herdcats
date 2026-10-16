import Citadel
import Crypto
import Foundation
import NIOCore
import Testing

@testable import Herdcats

private final class BridgeLiveBundle: NSObject {}
private struct BridgeLiveConfiguration: Decodable {
    let host: String
    let port: Int
    let username: String
    let privateKey: String
    let hostKey: String
    let command: String
    let appendCommand: String
    static var load: Self? {
        guard
            let url = Bundle(for: BridgeLiveBundle.self).url(
                forResource: "bridge-live-smoke", withExtension: "json"),
            let data = try? Data(contentsOf: url)
        else { return nil }
        return try? JSONDecoder().decode(Self.self, from: data)
    }
}
private actor BridgeLiveEvents {
    var frames: [AgentBridgeFrame] = []
    func record(_ frame: AgentBridgeFrame) { frames.append(frame) }
    func waitFor(_ event: String, text: String? = nil) async throws -> AgentBridgeFrame {
        for _ in 0..<500 {
            if let frame = frames.first(where: {
                $0.event == event
                    && (text == nil || $0.data?.entries?.contains(where: { $0.text == text }) == true)
            }) {
                return frame
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        throw AgentBridgeError.timeout
    }
}
@Suite("Agent bridge live SSH")
struct AgentBridgeLiveSSHTests {
    /// Enabled only by an ephemeral, loopback-only test resource. Normal builds
    /// carry no credentials and make no network calls from this test.
    @Test(.enabled(if: BridgeLiveConfiguration.load != nil))
    func liveCitadelFramesHistoryUpdatesIdleCancellationAndReconnect() async throws {
        let config = try #require(BridgeLiveConfiguration.load)
        #expect(config.host == "127.0.0.1")
        let key = try Curve25519.Signing.PrivateKey(openSSHPEM: config.privateKey)
        let authentication = SSHAuthenticationMethod.ed25519(username: config.username, privateKey: key)
        var settings = SSHClientSettings(
            host: config.host, port: config.port, authenticationMethod: { authentication },
            hostKeyValidator: .custom(
                SSHHostKeyValidatorDelegate(host: config.host, port: config.port, trustedKey: config.hostKey))
        )
        settings.connectTimeout = .seconds(5)
        let ssh = try await SSHClient.connect(to: settings)
        let events = BridgeLiveEvents()
        let first = AgentBridgeClient(onEvent: { await events.record($0) })
        let secondEvents = BridgeLiveEvents()
        let second = AgentBridgeClient(onEvent: { await secondEvents.record($0) })
        do {
            try await first.open(client: ssh, command: config.command, paneId: "w1:p1")
            let snapshot = try await events.waitFor("timeline.snapshot")
            #expect(snapshot.data?.entries?.count == 100)
            #expect(snapshot.data?.entries?.first?.text == "猫🐈 7")
            let page = try await first.request(
                .history,
                params: AgentBridgeParams(
                    generation: snapshot.data?.generation, before: snapshot.data?.before))
            #expect(page.entries?.count == 7)
            #expect(page.entries?.first?.text == "猫🐈 0")
            _ = try await ssh.executeCommand(config.appendCommand)
            _ = try await events.waitFor("timeline.updated", text: "incremental 漢字🐈")
            let idle = Task { try await first.waitUntilClosed() }
            await Task.yield()
            idle.cancel()
            do {
                try await idle.value
                Issue.record("idle channel did not cancel")
            } catch {}
            // Closing a bridge must leave its parent SSH connection usable.
            let healthy = try await ssh.executeCommand("printf healthy")
            #expect(String(decoding: healthy.readableBytesView, as: UTF8.self) == "healthy")
            try await second.open(client: ssh, command: config.command, paneId: "w1:p1")
            let reconnect = try await secondEvents.waitFor("timeline.snapshot")
            #expect(reconnect.data?.entries?.last?.text == "incremental 漢字🐈")
            #expect(reconnect.epoch != snapshot.epoch)
        } catch {
            await first.close()
            await second.close()
            try? await ssh.close()
            throw error
        }
        await first.close()
        await second.close()
        try? await ssh.close()
    }
}

private struct HelperInstallLiveConfiguration: Decodable {
    let host: String
    let port: Int
    let username: String
    let privateKey: String
    let hostKey: String
    static var load: Self? {
        guard let text = ProcessInfo.processInfo.environment["HC_HELPER_LIVE_CONFIG"] else { return nil }
        return try? JSONDecoder().decode(Self.self, from: Data(text.utf8))
    }
}
@Suite("Helper installation live SSH")
struct HostHelperInstallLiveSSHTests {
    /// Opt-in loopback test only; ordinary test runs carry no credentials.
    @Test(.enabled(if: HelperInstallLiveConfiguration.load != nil))
    func installsPublishedHelperAndKeepsParentSSHHealthyAfterTimeout() async throws {
        let config = try #require(HelperInstallLiveConfiguration.load)
        try #require(config.host == "127.0.0.1")
        let key = try Curve25519.Signing.PrivateKey(openSSHPEM: config.privateKey)
        let auth = SSHAuthenticationMethod.ed25519(username: config.username, privateKey: key)
        var settings = SSHClientSettings(host: config.host, port: config.port, authenticationMethod: { auth },
            hostKeyValidator: .custom(SSHHostKeyValidatorDelegate(host: config.host, port: config.port, trustedKey: config.hostKey)))
        settings.connectTimeout = .seconds(5)
        let ssh = try await SSHClient.connect(to: settings)
        do {
            let before = try await HostHelperCommand().run(client: ssh, script: HostHelperSetupPlan.probeScript, timeout: 10)
            #expect(try HostHelperProbe(output: before) == .missing)
            let installed = try await HostHelperCommand().run(client: ssh, script: HostHelperSetupPlan.installationScript(replace: false), timeout: 180)
            try HostHelperSetupPlan.verifyInstallation(installed)
            let after = try await HostHelperCommand().run(client: ssh, script: HostHelperSetupPlan.probeScript, timeout: 10)
            let probe = try HostHelperProbe(output: after)
            #expect(HostHelperSetupPlan.decision(probe: probe, replace: false, checkOnly: false) == .ready)
            do {
                _ = try await HostHelperCommand().run(client: ssh, script: "sleep 30", timeout: 0.05)
                Issue.record("idle command failed to time out")
            } catch {}
            do {
                _ = try await HostHelperCommand().run(client: ssh, script: "exit 7", timeout: 5)
                Issue.record("failed remote command accepted")
            } catch { #expect((error as? SSHClient.CommandFailed)?.exitCode == 7) }
            let healthy = try await ssh.executeCommand("printf healthy")
            #expect(String(decoding: healthy.readableBytesView, as: UTF8.self) == "healthy")
        } catch {
            try? await ssh.close()
            throw error
        }
        try? await ssh.close()
    }
}
