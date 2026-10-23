import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct ConnectionSettingsView: View {
    @Environment(AppModel.self) var appModel
    @Environment(\.dismiss) var dismiss
    let isReplay: Bool
    @State var host = ""
    @State var port = "22"
    @State var username = ""
    @State var authModeRaw = AuthMode.password.rawValue
    @State var remember = true

    @State var password = ""
    @State var keyPEM = ""
    @State var keyPassphrase = ""
    @State var rememberPassphrase = false

    var keyIsEncrypted: Bool { (try? OpenSSHEd25519.isEncrypted(pem: keyPEM)) == true }
    @State var showPassword = false
    @State var showKey = false
    @State var keyImporterPresented = false
    @State var importErrorMessage: String?
    @State var didCopyPubKey = false
    @State var showingSetupGuide = false

    var derivedPublicKey: String? {
        try? OpenSSHEd25519.openSSHPublicKeyString(for: validatedPublicKey())
    }

    private func validatedPublicKey() throws -> Data {
        // Validate plaintext private material before enabling Connect. Encrypted
        // keys expose metadata here; unlock them only during the connection attempt.
        if try !OpenSSHEd25519.isEncrypted(pem: keyPEM) {
            _ = try OpenSSHEd25519.parseRawPrivateKey(pem: keyPEM)
        }
        return try OpenSSHEd25519.parsePublicKey(pem: keyPEM)
    }

    var maskedPrivateKey: String {
        OpenSSHEd25519.maskPrivateKey(pem: keyPEM)
    }

    var keyValidationError: String? {
        let trimmed = keyPEM.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if trimmed.hasPrefix("ssh-ed25519 ") || trimmed.hasPrefix("ssh-rsa ") || trimmed.hasPrefix("ecdsa-") {
            return "This is an SSH public key (.pub). Please import or paste your private key instead "
                + "(usually named herdcats_key or id_ed25519 without .pub)."
        }
        if trimmed.contains("BEGIN RSA PRIVATE KEY")
            || trimmed.contains("BEGIN DSA PRIVATE KEY")
            || trimmed.contains("BEGIN EC PRIVATE KEY") {
            return "This key uses an unsupported format. Herdcats requires an ed25519 OpenSSH key "
                + "(ssh-keygen -t ed25519)."
        }
        do {
            _ = try validatedPublicKey()
            return nil
        } catch let keyError as OpenSSHKeyError {
            return keyError.errorDescription
        } catch {
            return error.localizedDescription
        }
    }

    static let documentationURL = URL(string: "https://herdcats.dev/support/#connect")!

    init(connection: RecentConnection, isReplay: Bool = false) {
        self.isReplay = isReplay
        _host = State(initialValue: connection.host)
        _port = State(initialValue: String(connection.port))
        _username = State(initialValue: connection.username)
        _authModeRaw = State(initialValue: connection.authMode)
        _remember = State(initialValue: connection.remember)
        let secret = connection.remember
            ? (try? KeychainStore.load(account: connection.secretAccount)) ?? "" : ""
        _password = State(initialValue: connection.authMode == "password" ? secret : "")
        _keyPEM = State(initialValue: connection.authMode == "privateKey" ? secret : "")
        let phrase = connection.remember ? (try? KeychainStore.load(account: connection.secretAccount + ".passphrase")) ?? "" : ""
        _keyPassphrase = State(initialValue: phrase)
        _rememberPassphrase = State(initialValue: !phrase.isEmpty)
    }

    enum AuthMode: String, CaseIterable, Identifiable {
        case password
        case privateKey

        var id: String { rawValue }
        var title: String {
            switch self {
            case .password: "Password"
            case .privateKey: "Private Key"
            }
        }
    }

    var authMode: AuthMode {
        AuthMode(rawValue: authModeRaw) ?? .password
    }

    var portNumber: Int? {
        let trimmed = port.trimmingCharacters(in: .whitespaces)
        guard let value = Int(trimmed), (1...65_535).contains(value) else { return nil }
        return value
    }

    var canConnect: Bool {
        guard !host.trimmingCharacters(in: .whitespaces).isEmpty,
              !username.trimmingCharacters(in: .whitespaces).isEmpty,
              portNumber != nil else { return false }
        switch authMode {
        case .password:
            return !password.isEmpty
        case .privateKey:
            return derivedPublicKey != nil && (!keyIsEncrypted || !keyPassphrase.isEmpty)
        }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    if let error = appModel.lastError {
                        if appModel.herdrMissingOnLastConnect {
                            MissingHerdrRecoveryCard(inForm: true) { showingSetupGuide = true }
                        } else if appModel.herdrSessionUnavailableOnLastConnect {
                            SessionRecoveryCard(inForm: true)
                        } else {
                            ErrorBanner(message: error)
                        }
                    }
                    connectionCard
                    footer
                }
                .padding(20)
            }
            .scrollDismissesKeyboard(.interactively)
            .background(Theme.backgroundGradient)
            .navigationTitle("Connection Settings")
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
                    .disabled(appModel.isConnecting)
                }
            }
        }
        .overlay {
            // A sheet is presented above the root view, so it needs its own
            // loading surface while authentication and the Herdr check run.
            if appModel.isConnecting {
                LazyCatLoadingView(message: appModel.connectionProgressMessage)
                    .background(Theme.backgroundGradient.ignoresSafeArea())
            }
        }
        .hostKeyApprovalSheet {
            await connect()
        }
        .sheet(isPresented: $showingSetupGuide) {
            HerdrSetupGuideView()
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .interactiveDismissDisabled(appModel.isConnecting)
    }

    var footer: some View {
        VStack(spacing: 8) {
            Text("Use a host address your iPhone can reach. 127.0.0.1 means the iPhone itself.")
                .font(.jost(.footnote))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button("Connection Setup Help") { showingSetupGuide = true }
                .font(.jost(.footnote))
        }
        .padding(.horizontal, 18)
    }

    // MARK: Helpers

    var fieldShape: some Shape {
        RoundedRectangle.continuous(DesignSystem.CornerRadius.md)
    }

    func fieldLabel(_ text: String, hidden: Bool = false) -> some View {
        Group {
            if !hidden {
                Text(text.uppercased())
                    .font(.jost(10, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .tracking(0.8)
            }
        }
    }

    func handleKeyImport(_ result: Result<[URL], Error>) {
        importErrorMessage = nil
        switch result {
        case let .failure(error):
            if let cocoaError = error as? CocoaError, cocoaError.code == .userCancelled {
                return
            }
            importErrorMessage = "File selection failed: \(error.localizedDescription)"
        case let .success(urls):
            guard let url = urls.first else { return }
            let accessed = url.startAccessingSecurityScopedResource()
            defer {
                if accessed {
                    url.stopAccessingSecurityScopedResource()
                }
            }
            do {
                let data = try Data(contentsOf: url)
                guard let content = String(data: data, encoding: .utf8) ?? String(data: data, encoding: .ascii) else {
                    importErrorMessage = "Unable to read key: file is not UTF-8 or ASCII text."
                    return
                }
                keyPEM = content.trimmingCharacters(in: .whitespacesAndNewlines)
                showKey = false
            } catch {
                importErrorMessage = "Could not read file: \(error.localizedDescription)"
            }
        }
    }

    func connect() async {
        let config = ConnectionConfig(
            host: host.trimmingCharacters(in: .whitespaces),
            port: portNumber ?? 22,
            username: username.trimmingCharacters(in: .whitespaces),
            auth: {
                switch authMode {
                case .password: .password(password)
                case .privateKey: .privateKey(keyPEM, passphrase: keyIsEncrypted ? keyPassphrase : nil)
                }
            }()
        )

        let loaded = (try? RecentConnectionStore.load()) ?? []
        let entry = RecentConnectionStore.entry(
            host: config.host, port: config.port, username: config.username,
            authMode: authModeRaw, remember: remember, in: loaded
        )
        let secret = authMode == .password ? password : keyPEM
        if !isReplay {
            do {
                if !remember {
                    try KeychainStore.delete(account: entry.secretAccount)
                    try KeychainStore.delete(account: entry.secretAccount + ".passphrase")
                    // Keep saved metadata in sync even if the connection attempt fails.
                    try RecentConnectionStore.disableRemembering(entry)
                }
                // When remember stays on but the passphrase is no longer wanted, its
                // deletion is deferred to after a successful connection: deleting here
                // would strand a still-saved encrypted-key profile without the passphrase
                // it needs if the connection attempt fails.
            } catch {
                appModel.lastError = error.localizedDescription
                return
            }
        }
        await appModel.connect(config: config)
        guard case .connected = appModel.phase else { return }
        if !isReplay {
            do {
                if entry.remember {
                    try KeychainStore.save(secret, account: entry.secretAccount)
                    if authMode == .privateKey && keyIsEncrypted && rememberPassphrase {
                        try KeychainStore.save(keyPassphrase, account: entry.secretAccount + ".passphrase")
                    } else {
                        try KeychainStore.delete(account: entry.secretAccount + ".passphrase")
                    }
                }
                _ = try RecentConnectionStore.record(entry, in: RecentConnectionStore.load())
            } catch {
                appModel.lastError = error.localizedDescription
                return
            }
        }
        dismiss()
    }

}
