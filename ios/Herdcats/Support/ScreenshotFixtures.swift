#if DEBUG && targetEnvironment(simulator)
import Foundation
import SwiftUI

/// Read-only fixtures for native screenshots. Never compiled into device builds.
enum ScreenshotFixtures {
    static var enabled: Bool { ProcessInfo.processInfo.arguments.contains("-hc.screenshots") }
    static var screen: String {
        let args = ProcessInfo.processInfo.arguments
        guard let index = args.firstIndex(of: "-hc.screen"), index + 1 < args.count else { return "spaces" }
        return args[index + 1]
    }

    static let projects = ["Herdcats", "Checkout redesign", "API performance"]
    static let providers = ["claude", "codex", "cursor"]
    static let statuses = ["blocked", "working", "done"]
    static let titles = ["Review the onboarding flow", "Build the checkout screens", "Optimize database queries"]
    static var workspaces: [[String: Any]] {
        projects.enumerated().map { index, label in
            ["workspace_id": "w\(index)", "label": label, "number": index + 1,
             "tab_count": 1, "pane_count": 1, "active_tab_id": "t\(index)",
             "agent_status": statuses[index], "focused": index == 0,
             "worktree": ["checkout_path": "/Users/demo/Projects/\(label.lowercased().replacingOccurrences(of: " ", with: "-"))",
                          "repo_name": label.lowercased().replacingOccurrences(of: " ", with: "-"),
                          "repo_root": "/Users/demo/Projects/\(label.lowercased().replacingOccurrences(of: " ", with: "-"))",
                          "is_linked_worktree": false]]
        }
    }
    static var paneAgent: String {
        if screen == "timeline-question-codex-plan" || screen == "timeline-question-codex-async" { return "codex" }
        if screen == "timeline-question-claude" { return "claude" }
        if screen == "timeline-approval-codex" { return "codex" }
        if screen == "timeline-approval-claude" { return "claude" }
        let args = ProcessInfo.processInfo.arguments
        guard let index = args.firstIndex(of: "-hc.agent"), index + 1 < args.count else { return "claude" }
        return args[index + 1]
    }
    static var panes: [[String: Any]] {
        projects.indices.map { index in
            ["pane_id": "p\(index)", "tab_id": "t\(index)", "workspace_id": "w\(index)",
             "agent": index == 0 ? paneAgent : providers[index], "agent_status": statuses[index],
             "cwd": "/Users/demo/Projects/\(projects[index].lowercased().replacingOccurrences(of: " ", with: "-"))",
             "terminal_title_stripped": titles[index], "terminal_title": titles[index],
             "focused": index == 0, "revision": 1, "state_change_seq": 10 - index]
        }
    }
    static func json(_ object: Any) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
    }
    static func response(_ args: String) throws -> String {
        if args == "--version" { return "herdr 0.8.0" }
        if args == "machine list --json" { return "[]" }
        if args.hasPrefix("pane read") { return terminal }
        let result: [String: Any]
        if args == "workspace list" { result = ["workspaces": workspaces] }
        else if args == "agent list" { result = ["agents": panes] }
        else if args.hasPrefix("pane list") {
            result = ["panes": panes.filter { args.contains($0["workspace_id"] as! String) }]
        } else if args.hasPrefix("tab list") {
            result = ["tabs": projects.indices.filter { args.contains("w\($0)") }.map { index in
                ["tab_id": "t\(index)", "workspace_id": "w\(index)", "label": "Agent", "number": 1,
                 "pane_count": 1, "agent_status": statuses[index], "focused": true] as [String: Any]
            }]
        } else { throw HerdrError.invalidConfiguration("Screenshot fixtures are read-only: \(args)") }
        return try json(["result": result])
    }
    static var quota: QuotaReport {
        let formatter = ISO8601DateFormatter()
        var entries: [QuotaProvider] = []
        for (index, provider) in ["claude", "codex", "cursor", "agy", "zai"].enumerated() {
            let session = QuotaWindow(id: "five_hour", resetsAt: formatter.string(from: Date().addingTimeInterval(7200)), percentRemaining: Double(82 - index * 9))
            let weekly = QuotaWindow(id: "weekly", resetsAt: formatter.string(from: Date().addingTimeInterval(172800)), percentRemaining: Double(64 - index * 7))
            let state = QuotaProviderState(status: "ok", stale: false, error: nil)
            entries.append(QuotaProvider(provider: provider, plan: "Pro", label: nil, windows: [session, weekly], state: state))
        }
        return QuotaReport(generatedAt: nil, schemaVersion: 1, providers: entries, cliVersion: nil)
    }

    static let terminal = """
    \u{001B}[38;5;208m╭─ Claude Code ───────────────────────────────────────╮\u{001B}[0m
      Herdcats · onboarding polish · Claude
    \u{001B}[38;5;208m╰────────────────────────────────────────────────────╯\u{001B}[0m

    \u{001B}[36m❯ Review the onboarding flow and improve the connection guidance.\u{001B}[0m

    ● Read the onboarding screens and checked the SSH connection setup from start to finish.

    ● Updated the connection guidance
      \u{001B}[32m✓ Clearer hints for the host address and SSH username
      ✓ Step-by-step instructions for connecting over Tailscale
      ✓ Host key verification explained before connecting\u{001B}[0m

    ● Ran the focused onboarding and connection tests
      \u{001B}[32m24 tests passed · 0 failures · ready for review\u{001B}[0m

    \u{001B}[1mReady for your review\u{001B}[0m
    The new copy is in place and the connection tests pass. Would you like me to commit these changes?

    \u{001B}[33m❯ Waiting for your input…\u{001B}[0m
    """
}

struct ScreenshotRootView: View {
    @State private var model: AppModel = {
        let app = AppModel(autoConnectOnLaunch: false)
        app.phase = .connected(host: "studio-mac", username: "demo")
        app.hasActiveSession = true
        app.connectionIdentity = ConnectionIdentity(host: "studio-mac", port: 22, username: "demo")
        if ProcessInfo.processInfo.arguments.contains("-hc.timeline-intro-reset"), let identity = app.connectionIdentity {
            UserDefaults.standard.removeObject(forKey: TimelineSetupPreference.key(scope: identity))
        }
        return app
    }()
    /// A saved machine reached through Herdr on the SSH-connected gateway, for
    /// the routed-host Timeline screen.
    @State private var routedModel: AppModel = {
        let gateway = ConnectionIdentity(host: "studio-mac", port: 22, username: "demo")
        let machine = HerdrMachine(
            id: "studio-mini", label: "Studio Mini", target: "studio-mini.local",
            session: "studio-mini", enabled: true)
        let routed = HerdrConnection(
            gateway: HerdrConnection(), machine: machine,
            generation: ConnectionGeneration(rawValue: 1), identity: gateway)
        let app = AppModel(connection: routed, autoConnectOnLaunch: false)
        app.phase = .connected(host: "Studio Mini", username: "demo")
        app.hasActiveSession = true
        app.selectedMachine = machine
        app.connectionIdentity = gateway.scoped(to: machine)
        if ProcessInfo.processInfo.arguments.contains("-hc.timeline-intro-reset"), let identity = app.connectionIdentity {
            UserDefaults.standard.removeObject(forKey: TimelineSetupPreference.key(scope: identity))
        }
        return app
    }()
    var body: some View {
        Group {
            if ScreenshotFixtures.screen == "pane" || ScreenshotFixtures.screen == "agent-pane" || ["keys", "compose", "voice", "turn-interrupted", "optional-upgrade-menu"].contains(ScreenshotFixtures.screen) || ScreenshotFixtures.screen.hasPrefix("timeline") {
                NavigationStack {
                    if let pane = try? JSONDecoder().decode(PaneEntry.self, from: Data(ScreenshotFixtures.json(ScreenshotFixtures.panes[0]).utf8)) {
                        PaneDetailView(pane: pane)
                    }
                }
            } else if ScreenshotFixtures.screen == "connect" {
                ConnectionSettingsView(connection: RecentConnection(
                    host: "studio-mac.tailnet.ts.net", port: 22, username: "demo",
                    authMode: "password", remember: false
                ))
            } else {
                MainTabView()
            }
        }
        .environment(ScreenshotFixtures.screen == "timeline-routed-host" ? routedModel : model)
        .preferredColorScheme(ProcessInfo.processInfo.arguments.contains("-hc.light") ? .light : .dark)
        .tint(Theme.accent)
        .foregroundStyle(Theme.text)
        .font(.jost(.body))
    }
}
#endif
