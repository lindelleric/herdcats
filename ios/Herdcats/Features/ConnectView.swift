import SwiftUI
import UIKit
import UniformTypeIdentifiers

// MARK: - Connect screen

enum OnboardingRoute: Equatable {
    static let introSeenKey = "hc.onboardingIntroSeen"

    case intro
    case connectionSelection

    static func initial(hasSeenIntro: Bool, hasSavedMachines: Bool, replay: Bool = false) -> Self {
        if replay { return .intro }
        return hasSeenIntro || hasSavedMachines ? .connectionSelection : .intro
    }
}

struct ConnectView: View {
    let replay: Bool
    @Environment(AppModel.self) private var appModel
    @AppStorage(OnboardingRoute.introSeenKey) private var hasSeenIntro = false
    @State private var recentConnections: [RecentConnection] = []
    @State private var selectedConnection: RecentConnection?
    @State private var showingSetupGuide = false
    @State private var openFormAfterSetup = false
    @State private var route: OnboardingRoute?

    init(replay: Bool = false) {
        self.replay = replay
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 26) {
                if let error = appModel.lastError {
                    if appModel.herdrMissingOnLastConnect {
                        MissingHerdrRecoveryCard(inForm: false) { showingSetupGuide = true }
                    } else if appModel.herdrSessionUnavailableOnLastConnect {
                        SessionRecoveryCard(inForm: false)
                    } else {
                        ErrorBanner(message: error)
                    }
                }
                switch route {
                case nil:
                    EmptyView()
                case .intro:
                    introScreen
                case .connectionSelection:
                    header
                    recentConnectionsSection
                    Button(recentConnections.isEmpty ? "Connection Setup Help" : "Set Up Another Remote Machine") {
                        showingSetupGuide = true
                    }
                    .font(.jost(.subheadline))
                }
            }
            .padding(.horizontal, 22)
            .padding(.top, 30)
            .padding(.bottom, 40)
        }
        .safeAreaInset(edge: .bottom) {
            if route == .intro {
                VStack(spacing: 4) {
                    Button("Continue") {
                        if !replay { hasSeenIntro = true }
                        route = .connectionSelection
                        openNewConnection()
                    }
                    .buttonStyle(.herdrPrimary())
                }
                .padding(.horizontal, 22)
                .padding(.vertical, 10)
                .frame(maxWidth: .infinity)
                .background(Theme.backgroundGradient)
            }
        }
        .onAppear {
            recentConnections = (try? RecentConnectionStore.load()) ?? []
            if route == nil {
                route = OnboardingRoute.initial(
                    hasSeenIntro: hasSeenIntro,
                    hasSavedMachines: !recentConnections.isEmpty,
                    replay: replay
                )
                if route == .connectionSelection && recentConnections.isEmpty && !replay {
                    openNewConnection()
                }
            }
        }
        .overlay {
            if appModel.isConnecting && selectedConnection == nil {
                LazyCatLoadingView(message: appModel.connectionProgressMessage)
                    .background(Theme.backgroundGradient.ignoresSafeArea())
            }
        }
        .sheet(item: $selectedConnection) { connection in
            ConnectionSettingsView(connection: connection, isReplay: replay)
        }
        .sheet(isPresented: $showingSetupGuide, onDismiss: {
            if openFormAfterSetup {
                openFormAfterSetup = false
                openNewConnection()
            }
        }, content: {
            HerdrSetupGuideView {
                openFormAfterSetup = true
                showingSetupGuide = false
            }
        })
    }

    private var introScreen: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(spacing: 8) {
                Image("HerdrcatLogo")
                    .resizable()
                    .scaledToFit()
                    .frame(width: 24, height: 24)
                    .clipShape(RoundedRectangle.continuous(6))
                    .accessibilityHidden(true)
                Text("Herdcats")
                    .font(.jost(17, weight: .semibold))
                    .foregroundStyle(Theme.text)
            }
            Text("Tame your autonomous agents from your pocket.")
                .font(.jost(34, weight: .bold))
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            AnimatedOnboardingBody()
                .padding(.top, 6)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 16)
    }

    private func openNewConnection() {
        selectedConnection = RecentConnection(
            host: "", port: 22, username: "", authMode: "password", remember: true
        )
    }

    private var header: some View {
        VStack(spacing: 14) {
            Image("HerdrcatLogo")
                .resizable()
                .scaledToFit()
                .frame(width: 68, height: 68)
                .clipShape(RoundedRectangle.continuous(DesignSystem.CornerRadius.xl))
                .accessibilityHidden(true)
            VStack(spacing: 5) {
                Text("Herdcats")
                    .font(.appDisplay)
                Text(recentConnections.isEmpty
                     ? "Tame your autonomous coding agents from your iPhone."
                     : "Connect to a remote machine")
                    .font(.jost(.subheadline))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
        .padding(.top, 14)
    }

    private func forget(_ connection: RecentConnection) {
        guard !replay else { return }
        do {
            recentConnections = try RecentConnectionStore.remove(connection)
        } catch {
            appModel.lastError = error.localizedDescription
            recentConnections = (try? RecentConnectionStore.load()) ?? recentConnections
        }
    }

    private var recentConnectionsSection: some View {
        VStack(alignment: .leading, spacing: 16) {
            if !recentConnections.isEmpty {
                Text("Recent Connections")
                    .font(.jost(.title2, weight: .bold))
            }

            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 14) {
                ForEach(recentConnections) { connection in
                    Button {
                        selectedConnection = connection
                    } label: {
                        VStack(alignment: .leading, spacing: 10) {
                            Image(systemName: "server.rack")
                                .font(.jost(.title2))
                                .foregroundStyle(Theme.accent)
                            Text(connection.host)
                                .font(.jost(.headline))
                                .lineLimit(2)
                                .truncationMode(.middle)
                            Text(connection.username)
                                .font(.jost(.subheadline))
                                .lineLimit(1)
                            Text("Port \(connection.port)")
                                .font(.jost(.caption))
                                .foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, minHeight: 126, alignment: .leading)
                        .padding(16)
                        .herdrCard()
                    }
                    .buttonStyle(.herdrCard)
                    .accessibilityLabel("\(connection.username) at \(connection.host), port \(connection.port)")
                    .accessibilityHint("Opens connection settings")
                    .contextMenu {
                        if !replay {
                            Button("Forget Connection", systemImage: "trash", role: .destructive) {
                                forget(connection)
                            }
                        }
                    }
                }
            }

            Button {
                openNewConnection()
            } label: {
                Label(
                    recentConnections.isEmpty ? "Connect to a Remote Machine" : "New Connection",
                    systemImage: "plus"
                )
            }
            .buttonStyle(.herdrPrimary())
        }
        .disabled(appModel.isConnecting)
    }

}

/// Own the editable state inside the presented view so validation and the
/// authentication fields update together, including on the first presentation.
struct MissingHerdrRecoveryCard: View {
    let inForm: Bool
    let onSetup: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("SSH worked. Herdr was not found.")
                .font(.jost(.headline))
            Text(
                "Install Herdr on the remote machine. "
                + (inForm
                    ? "Your connection details stay in this form. Then tap Connect again."
                    : "Open a connection form, enter your SSH details, then tap Connect.")
            )
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
            Button("See Setup Steps", action: onSetup)
                .buttonStyle(.herdrSecondary())
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .herdrCard()
    }
}

struct SessionRecoveryCard: View {
    let inForm: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Herdr was found. Check its session.")
                .font(.jost(.headline))
            Text(
                "On the remote machine, run herdr from a project directory to start or attach "
                + "to the default session. "
                + (inForm
                    ? "Then tap Connect again here."
                    : "Open a connection form, enter your SSH details, then tap Connect.")
            )
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
            Link("Herdr Quick Start", destination: URL(string: "https://herdr.dev/docs/quick-start/")!)
                .font(.jost(.subheadline))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .herdrCard()
    }
}

// MARK: - Animated onboarding body

struct AnimatedOnboardingBody: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var visibleWordCount = 0

    private static let paragraph1Words = [
        "Herding", "AI", "agents", "used", "to", "feel", "impossible",
        "the", "moment", "you", "stepped", "away", "from", "your", "desk."
    ]

    private static let paragraph2Words = [
        "Herdcats", "connects", "directly", "to", "Herdr", "over", "SSH",
        "so", "you", "can", "monitor", "progress,", "unblock", "agents,",
        "and", "keep", "the", "herd", "moving —", "right", "from", "your", "iPhone."
    ]

    private static let totalWords = paragraph1Words.count + paragraph2Words.count

    var body: some View {
        VStack(alignment: .leading, spacing: 28) {
            WordFlowLayout(horizontalSpacing: 7, verticalSpacing: 11) {
                ForEach(Array(Self.paragraph1Words.enumerated()), id: \.offset) { index, word in
                    let isVisible = index < visibleWordCount
                    Text(word)
                        .font(.jost(28, weight: .medium))
                        .foregroundStyle(Theme.textMuted)
                        .opacity(isVisible ? 1 : 0)
                        .offset(y: isVisible ? 0 : 5)
                        .blur(radius: isVisible ? 0 : 3)
                        .animation(reduceMotion ? nil : .easeOut(duration: 0.28), value: isVisible)
                }
            }

            WordFlowLayout(horizontalSpacing: 7, verticalSpacing: 11) {
                ForEach(Array(Self.paragraph2Words.enumerated()), id: \.offset) { index, word in
                    let globalIndex = Self.paragraph1Words.count + index
                    let isVisible = globalIndex < visibleWordCount
                    Text(word)
                        .font(.jost(28, weight: .medium))
                        .foregroundStyle(Theme.textMuted)
                        .opacity(isVisible ? 1 : 0)
                        .offset(y: isVisible ? 0 : 5)
                        .blur(radius: isVisible ? 0 : 3)
                        .animation(reduceMotion ? nil : .easeOut(duration: 0.28), value: isVisible)
                }
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            withAnimation(.easeOut(duration: 0.25)) {
                visibleWordCount = Self.totalWords
            }
        }
        .task {
            if reduceMotion {
                visibleWordCount = Self.totalWords
                return
            }
            try? await Task.sleep(nanoseconds: 200_000_000)
            for i in 1...Self.totalWords {
                if Task.isCancelled { break }
                visibleWordCount = i
                let delay: UInt64 = (i == Self.paragraph1Words.count) ? 260_000_000 : 90_000_000
                try? await Task.sleep(nanoseconds: delay)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            "Herding AI agents used to feel impossible the moment you stepped away from your desk. "
            + "Herdcats connects directly to Herdr over SSH so you can monitor progress, "
            + "unblock agents, and keep the herd moving—right from your iPhone."
        )
    }
}

private struct WordFlowLayout: Layout {
    var horizontalSpacing: CGFloat = 8
    var verticalSpacing: CGFloat = 10

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let maxWidth = (proposal.width == nil || proposal.width == .infinity) ? 350 : proposal.width!
        var currentX: CGFloat = 0
        var currentY: CGFloat = 0
        var lineHeight: CGFloat = 0

        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if currentX + size.width > maxWidth && currentX > 0 {
                currentX = 0
                currentY += lineHeight + verticalSpacing
                lineHeight = 0
            }
            lineHeight = max(lineHeight, size.height)
            currentX += size.width + horizontalSpacing
        }
        return CGSize(width: maxWidth, height: currentY + lineHeight)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var currentX = bounds.minX
        var currentY = bounds.minY
        var lineHeight: CGFloat = 0

        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if currentX + size.width > bounds.maxX && currentX > bounds.minX {
                currentX = bounds.minX
                currentY += lineHeight + verticalSpacing
                lineHeight = 0
            }
            subview.place(at: CGPoint(x: currentX, y: currentY), proposal: .unspecified)
            lineHeight = max(lineHeight, size.height)
            currentX += size.width + horizontalSpacing
        }
    }
}
