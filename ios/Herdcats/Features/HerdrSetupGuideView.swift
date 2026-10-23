import SwiftUI
import UIKit
import UniformTypeIdentifiers

// MARK: - First connection guide

struct HerdrSetupGuideView: View {
    @Environment(\.dismiss) private var dismiss
    let onContinue: (() -> Void)?

    init(onContinue: (() -> Void)? = nil) {
        self.onContinue = onContinue
    }

    private let installURL = URL(string: "https://herdr.dev/docs/install/")!
    private let quickStartURL = URL(string: "https://herdr.dev/docs/quick-start/")!

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    Text("Prepare Herdr and SSH on your remote machine.")
                        .font(.jost(.subheadline))
                        .foregroundStyle(.secondary)

                    step(
                        1, title: "Install Herdr",
                        detail: "Install on the remote machine. Check with herdr --version.",
                        url: installURL, linkTitle: "Herdr Install Guide"
                    )
                    step(
                        2, title: "Start the Default Session",
                        detail: "Run herdr from a project directory on the remote machine.",
                        url: quickStartURL, linkTitle: "Herdr Quick Start"
                    )
                    step(
                        3, title: "Enable SSH",
                        detail: "Use a host your iPhone can reach. Tailscale is recommended. "
                            + "Have a password or OpenSSH ed25519 key and its passphrase, if encrypted."
                    )

                    Text(
                        "When you connect for the first time, compare the SSH host-key fingerprint "
                        + "with the one on the remote machine before trusting it."
                    )
                        .font(.jost(.footnote))
                        .foregroundStyle(.secondary)

                    Button(onContinue == nil ? "Back to Connection" : "Enter SSH Details") {
                        onContinue?()
                        dismiss()
                    }
                        .buttonStyle(.herdrPrimary())
                }
                .padding(20)
            }
            .background(Theme.backgroundGradient)
            .navigationTitle("Setup Help")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button {
                        dismiss()
                    } label: {
                        Image(systemName: "xmark")
                            .font(.jost(16, weight: .semibold))
                            .foregroundStyle(.primary)
                    }
                    .accessibilityLabel("Close")
                }
            }
        }
        .presentationDetents([.large])
    }

    private func step(
        _ number: Int, title: String, detail: String, url: URL? = nil, linkTitle: String = ""
    ) -> some View {
        HStack(alignment: .top, spacing: 14) {
            Text("\(number)")
                .font(.jost(.headline))
                .foregroundStyle(Theme.onPrimary)
                .frame(width: 30, height: 30)
                .background(Theme.accent, in: Circle())
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 8) {
                Text(title)
                    .font(.jost(.headline))
                    .accessibilityLabel("Step \(number): \(title)")
                Text(detail)
                    .font(.jost(.subheadline))
                    .foregroundStyle(.secondary)
                if let url {
                    Link(destination: url) {
                        Label(linkTitle, systemImage: "arrow.up.right")
                            .font(.jost(.subheadline))
                    }
                }
            }
        }
    }
}

// MARK: - Automated end-to-end hook

/// Optional launch-argument driven auto-connect, used to verify the SSH
/// pipeline from the Simulator without typing into the UI:
///
///     xcrun simctl launch <udid> com.enchantinglabs.herdrcat \
///       -hc.autoconnect 1 -hc.host 127.0.0.1 -hc.user alice \
///       [-hc.password secret | -hc.key /path/to/key]
enum AutoConnect {
    /// Release builds never parse launch arguments or environment variables
    /// for SSH credentials; this hook is only compiled into Debug builds.
    static func configIfRequested() -> ConnectionConfig? {
        #if DEBUG
        return debugConfigIfRequested()
        #else
        return nil
        #endif
    }

    #if DEBUG
    private static func debugConfigIfRequested() -> ConnectionConfig? {
        let arguments = ProcessInfo.processInfo.arguments

        func value(after flag: String) -> String? {
            guard let index = arguments.firstIndex(of: flag), index + 1 < arguments.count else {
                return nil
            }
            return arguments[index + 1]
        }

        guard value(after: "-hc.autoconnect") != nil,
              let host = value(after: "-hc.host"), !host.isEmpty,
              let user = value(after: "-hc.user"), !user.isEmpty else {
            return nil
        }
        let port = Int(value(after: "-hc.port") ?? "") ?? 22

        if let password = value(after: "-hc.password") {
            return ConnectionConfig(host: host, port: port, username: user, auth: .password(password))
        }
        if let keyPath = value(after: "-hc.key") {
            let pem = (try? String(contentsOfFile: keyPath, encoding: .utf8)) ?? ""
            return ConnectionConfig(host: host, port: port, username: user, auth: .privateKey(pem))
        }
        if let keyData = ProcessInfo.processInfo.environment["HC_KEY"], !keyData.isEmpty {
            return ConnectionConfig(host: host, port: port, username: user, auth: .privateKey(keyData))
        }
        return nil
    }
    #endif
}
