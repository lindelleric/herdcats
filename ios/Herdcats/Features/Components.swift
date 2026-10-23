import SwiftUI
import UIKit

/// Identifies the card that initiated a reorder so it can draw over its neighbors.
struct ReorderingCard: Equatable {
    let id: String
    let priorityKey: String

    static func movingID(from old: [Self], to new: [Self]) -> String? {
        let oldIDs = Set(old.map(\.id))
        let newIDs = Set(new.map(\.id))
        guard old.map(\.id).filter(newIDs.contains) != new.map(\.id).filter(oldIDs.contains) else {
            return nil
        }
        let oldPositions = Dictionary(uniqueKeysWithValues: old.enumerated().map { ($0.element.id, $0.offset) })
        let oldKeys = Dictionary(uniqueKeysWithValues: old.map { ($0.id, $0.priorityKey) })
        let candidates = new.enumerated().compactMap { index, card -> (id: String, distance: Int, changed: Bool, index: Int)? in
            guard let oldIndex = oldPositions[card.id], oldIndex != index else { return nil }
            return (card.id, abs(oldIndex - index), oldKeys[card.id] != card.priorityKey, index)
        }
        return candidates.max {
            if $0.changed != $1.changed { return !$0.changed }
            if $0.distance != $1.distance { return $0.distance < $1.distance }
            return $0.index > $1.index
        }?.id
    }
}

// MARK: - Appearance preference

/// User-selected app appearance. Persisted via `@AppStorage` using `rawValue`.
enum AppearancePreference: String, CaseIterable, Identifiable {
    /// Follow the device's light / dark setting.
    case system
    case light
    case dark

    var id: String { rawValue }

    var title: String {
        switch self {
        case .system: "System"
        case .light: "Light"
        case .dark: "Dark"
        }
    }

    var systemImage: String {
        switch self {
        case .system: "circle.lefthalf.filled"
        case .light: "sun.max.fill"
        case .dark: "moon.fill"
        }
    }

    /// `nil` means follow the system setting.
    var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }

    static let storageKey = "appearancePreference"
    static let `default`: AppearancePreference = .dark
}

/// What to show when the app launches while disconnected.
enum LaunchPreference: String, CaseIterable, Identifiable {
    /// Show the recent-connections selection screen.
    case connectionSelection
    /// Connect immediately using the most recent remembered connection.
    case autoConnect

    var id: String { rawValue }

    var title: String {
        switch self {
        case .connectionSelection: "Connection List"
        case .autoConnect: "Auto-connect"
        }
    }

    var systemImage: String {
        switch self {
        case .connectionSelection: "rectangle.stack"
        case .autoConnect: "bolt.horizontal.circle"
        }
    }

    static let storageKey = "launchPreference"
    static let `default`: LaunchPreference = .connectionSelection

    /// Builds a config for the newest remembered connection, if credentials exist.
    static func autoConnectConfig(
        defaults: UserDefaults = .standard,
        keychain: KeychainOperations = KeychainStore.operations
    ) -> ConnectionConfig? {
        let raw = defaults.string(forKey: storageKey) ?? LaunchPreference.default.rawValue
        guard LaunchPreference(rawValue: raw) == .autoConnect else { return nil }

        let entries = (try? RecentConnectionStore.load(defaults: defaults, using: keychain)) ?? []
        guard let entry = entries.first(where: \.remember),
              let secret = try? KeychainStore.load(account: entry.secretAccount, using: keychain),
              !secret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return nil
        }

        let passphrase = try? KeychainStore.load(account: entry.secretAccount + ".passphrase", using: keychain)
        if entry.authMode == "privateKey", (try? OpenSSHEd25519.isEncrypted(pem: secret)) == true,
           passphrase == nil || passphrase?.isEmpty == true { return nil }
        let auth: ConnectionConfig.AuthMethod = entry.authMode == "privateKey"
            ? .privateKey(secret, passphrase: passphrase)
            : .password(secret)
        return ConnectionConfig(
            host: entry.host,
            port: entry.port,
            username: entry.username,
            auth: auth
        )
    }
}

/// Whether pane detail keeps its Compose field visible when idle.
enum PaneComposePreference {
    static let storageKey = "paneShowComposeByDefault"
    static let `default` = true
}

/// Monospaced pane output size. Persisted via `@AppStorage` using `rawValue`.
enum PaneTextSizePreference: String, CaseIterable, Identifiable {
    case small
    case medium
    case large
    case extraLarge

    var id: String { rawValue }

    var title: String {
        switch self {
        case .small: "S"
        case .medium: "M"
        case .large: "L"
        case .extraLarge: "XL"
        }
    }

    var accessibilityTitle: String {
        switch self {
        case .small: "Small"
        case .medium: "Medium"
        case .large: "Large"
        case .extraLarge: "Extra Large"
        }
    }

    /// Point size for terminal / agent output (medium matches the historical default of 11).
    var pointSize: CGFloat {
        switch self {
        case .small: 9
        case .medium: 11
        case .large: 14
        case .extraLarge: 17
        }
    }

    var outputFont: Font {
        .system(size: pointSize, design: .monospaced)
    }

    /// Versioned because the case names shifted when `small` (9pt) was added;
    /// the old key's `small` meant 11pt, so old selections reset to the default.
    static let storageKey = "paneTextSizePreference.v2"
    static let `default`: PaneTextSizePreference = .medium
}

/// How a pane's output scroll view behaves when new agent output arrives.
/// Persisted via `@AppStorage` using `rawValue`.
enum PaneScrollBehaviorPreference: String, CaseIterable, Identifiable {
    /// Keep the reader where they are; only pin to the newest output on first
    /// load and right after the user sends input.
    case holdPosition
    /// Scroll to the newest output every time the agent responds.
    case followOutput

    var id: String { rawValue }

    var title: String {
        switch self {
        case .holdPosition: "Stay put"
        case .followOutput: "Jump to end"
        }
    }

    var systemImage: String {
        switch self {
        case .holdPosition: "hand.raised"
        case .followOutput: "arrow.down.to.line"
        }
    }

    var detail: String {
        switch self {
        case .holdPosition: "Maintain current scroll position."
        case .followOutput: "Automatically scroll to newest output."
        }
    }

    static let storageKey = "paneScrollBehaviorPreference"
    static let `default`: PaneScrollBehaviorPreference = .holdPosition
}

/// How opening a Done pane on iPhone clears that status to Idle.
/// Herdr treats Done as idle work that nobody has marked seen yet; focusing
/// the agent or its tab on the remote machine is what clears it.
enum DoneClearPreference: String, CaseIterable, Identifiable {
    /// Show a mark-as-read control; viewing alone leaves Done unchanged.
    case explicit
    /// Focus the agent when the pane opens so Done clears automatically.
    case implicit

    var id: String { rawValue }

    var title: String {
        switch self {
        case .explicit: "Manually"
        case .implicit: "When opened"
        }
    }

    var systemImage: String {
        switch self {
        case .explicit: "hand.tap"
        case .implicit: "eye"
        }
    }

    var detail: String {
        switch self {
        case .explicit:
            "Keeps Done until marked manually. Does not focus the remote machine."
        case .implicit:
            "Focuses the agent on the remote machine when opened, clearing Done."
        }
    }

    static let storageKey = "doneClearPreference"
    static let `default`: DoneClearPreference = .explicit

    static func load(from defaults: UserDefaults = .standard) -> DoneClearPreference {
        guard let raw = defaults.string(forKey: storageKey) else { return .default }
        return DoneClearPreference(rawValue: raw) ?? .default
    }
}

// MARK: - Agents list preferences

/// How the Agents screen orders its list. Mirrors herdr's
/// `agent_panel_sort` values: `priority` (flat attention queue) or `spaces`
/// (grouped by space, spaces in workspace order).
enum AgentsSortPreference: String, CaseIterable, Identifiable {
    /// Flat attention queue: blocked, done, working, idle, unknown.
    case priority
    /// Grouped by space.
    case spaces

    var id: String { rawValue }

    var title: String {
        switch self {
        case .priority: "By priority"
        case .spaces: "By space"
        }
    }

    var systemImage: String {
        switch self {
        case .priority: "exclamationmark.circle"
        case .spaces: "square.grid.3x3"
        }
    }

    static let storageKey = "agentsSortPreference"
    static let `default`: AgentsSortPreference = .priority
}

/// How the Spaces screen orders its workspace cards.
enum SpaceSortPreference: String, CaseIterable, Identifiable {
    /// Preserve Herdr's workspace-number order.
    case workspaceOrder
    /// Put the most attention-worthy space groups first.
    case priority

    var id: String { rawValue }

    var title: String {
        switch self {
        case .workspaceOrder: "Default"
        case .priority: "By priority"
        }
    }

    var systemImage: String {
        switch self {
        case .workspaceOrder: "list.number"
        case .priority: "exclamationmark.circle"
        }
    }

    static let storageKey = "spacesSortPreference"
    static let `default`: SpaceSortPreference = .workspaceOrder
}

// MARK: - Usage / quota preferences

/// Order of provider cards on the Quota tab. Rings use the hybrid
/// scale in `Theme.quotaColor` (pace above 40%, urgency below).
enum QuotaOrderPreference: String, CaseIterable, Identifiable {
    /// Cards whose quota runs out soonest at the current burn rate come first.
    case runsOutFirst
    /// Cards sorted by provider name.
    case alphabetical

    var id: String { rawValue }

    var title: String {
        switch self {
        case .runsOutFirst: "Runs Out First"
        case .alphabetical: "A–Z"
        }
    }

    var systemImage: String {
        switch self {
        case .runsOutFirst: "hourglass"
        case .alphabetical: "textformat"
        }
    }

    /// Cards in display order. Ties keep the incoming order.
    func ordered(_ cards: [AgentUsageCard], now: Date = .now) -> [AgentUsageCard] {
        let indexed = Array(cards.enumerated())
        switch self {
        case .runsOutFirst:
            return indexed.sorted { lhs, rhs in
                let l = lhs.element.depletionRank(now: now)
                let r = rhs.element.depletionRank(now: now)
                if l.tier != r.tier { return l.tier < r.tier }
                if l.value != r.value { return l.value < r.value }
                return lhs.offset < rhs.offset
            }
            .map(\.element)
        case .alphabetical:
            return indexed.sorted { lhs, rhs in
                let order = lhs.element.displayTitle.localizedCaseInsensitiveCompare(rhs.element.displayTitle)
                return order == .orderedSame ? lhs.offset < rhs.offset : order == .orderedAscending
            }
            .map(\.element)
        }
    }

    static let storageKey = "quotaOrderPreference"
    static let `default`: QuotaOrderPreference = .runsOutFirst
}

extension AgentUsageCard {
    /// Card title on the Quota tab.
    var displayTitle: String {
        switch provider {
        case "agy": "Agy"
        case "agy-claude": "Agy·C"
        case "zai": "ZAI"
        case "alibaba": "Alibaba"
        case "copilot": "Copilot"
        case "opencode-go": "OpenCode"
        case "kimi": "Kimi"
        case "grok": "Grok"
        default: AgentKindVisual.displayName(for: provider)
        }
    }
}

// MARK: - Status visuals

extension AgentStatus {
    /// System colors for status badges and dots, matching the Herdr TUI
    /// (`state_dot` / `state_label_color`). Quota, warnings and destructive
    /// actions keep the Moonlit `Theme.status*` tokens.
    var color: Color {
        switch self {
        case .working: .yellow
        case .blocked: .red
        case .unknown: .gray
        case .done: .teal
        case .idle: .green
        }
    }
}

/// Per-agent-kind branding for known agents, with graceful fallbacks.
enum AgentKindVisual {
    static func color(for kind: String) -> Color {
        switch kind {
        case "cursor": Color(red: 0.55, green: 0.75, blue: 0.95)
        case "codex": Color(red: 0.09, green: 0.64, blue: 0.51) // OpenAI green
        case "claude": Color(red: 0.85, green: 0.47, blue: 0.34) // Anthropic Claude
        case "pi": Color(red: 0.94, green: 0.56, blue: 0.51) // pi.dev coral
        case "agy", "agy-claude", "gemini": Color(red: 0.56, green: 0.46, blue: 0.70) // Gemini purple
        case "zai": Color(red: 0.12, green: 0.39, blue: 0.93) // Z.AI blue
        case "alibaba": Color(red: 0.95, green: 0.45, blue: 0.15)
        case "copilot": Color(red: 0.45, green: 0.55, blue: 0.95)
        case "grok": Color(red: 0.85, green: 0.85, blue: 0.90)
        case "kimi": Color(red: 0.35, green: 0.70, blue: 0.55)
        case "opencode-go": Color(red: 0.30, green: 0.70, blue: 0.65)
        default: .gray
        }
    }

    /// Asset-catalog brand mark when available; otherwise fall back to `symbol(for:)`.
    static func assetName(for kind: String) -> String? {
        switch kind {
        case "cursor": "AgentCursor"
        case "codex": "AgentCodex"
        case "claude": "AgentClaude"
        case "pi": "AgentPi"
        case "agy", "agy-claude", "gemini": "AgentGemini"
        case "zai": "AgentZai"
        default: nil
        }
    }

    /// Pi keeps its multi-color mark; other brand assets are template-tinted.
    static func usesOriginalAssetColors(for kind: String) -> Bool {
        kind == "pi"
    }

    static func symbol(for kind: String) -> String {
        switch kind {
        case "cursor": "cursorarrow.rays"
        case "codex": "sparkles"
        case "claude": "terminal.fill"
        case "pi": "function"
        case "agy", "agy-claude", "gemini": "diamond.fill"
        case "zai": "flame.fill"
        case "alibaba": "shippingbox.fill"
        case "copilot": "chevron.left.forwardslash.chevron.right"
        case "grok": "bolt.fill"
        case "kimi": "moon.stars.fill"
        case "opencode-go": "hammer.fill"
        default: "cpu"
        }
    }

    static func displayName(for kind: String) -> String {
        kind.prefix(1).uppercased() + kind.dropFirst()
    }
}

// MARK: - Status dot

struct StatusDot: View {
    let status: AgentStatus
    var animated = false

    @State private var pulsing = false

    var body: some View {
        Group {
            if status == .working {
                WorkingStatusIcon()
            } else {
                Circle()
                    .fill(status.color)
                    .frame(width: 9, height: 9)
                    .shadow(color: status.color.opacity(0.8), radius: animated && pulsing ? 5 : 2)
                    .scaleEffect(animated && pulsing ? 1.25 : 0.9)
                    .animation(
                        animated ? .easeInOut(duration: 0.8).repeatForever(autoreverses: true) : .default,
                        value: pulsing
                    )
                    .onAppear {
                        guard animated else { return }
                        pulsing = true
                    }
            }
        }
        .frame(width: 14, height: 14)
    }
}

// MARK: - Navigation title

struct StatusNavigationTitle: View {
    let title: String
    let status: AgentStatus
    var subtitle: String? = nil

    var body: some View {
        HStack(spacing: 8) {
            StatusDot(status: status, animated: status == .working)
            VStack(spacing: 1) {
                Text(title)
                    .font(.jost(.headline))
                    .lineLimit(1)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.jost(.caption2))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityText)
        .accessibilityAddTraits(.isHeader)
    }

    private var accessibilityText: String {
        if let subtitle, !subtitle.isEmpty {
            return "\(title), \(subtitle), \(status.label)"
        }
        return "\(title), \(status.label)"
    }
}

// MARK: - Status pill

struct StatusPill: View {
    let status: AgentStatus

    var body: some View {
        HStack(spacing: 5) {
            StatusDot(status: status, animated: status == .working)
            Text(status.label)
                .font(.jost(.caption2, weight: .semibold))
                .foregroundStyle(status.color)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Capsule().fill(status.color.opacity(0.16)))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(status.label)
    }
}

// MARK: - Agent badge

struct AgentBadge: View {
    let kind: String

    private var tint: Color { AgentKindVisual.color(for: kind) }

    var body: some View {
        HStack(spacing: 4) {
            AgentKindIcon(kind: kind, size: 10)
            Text(AgentKindVisual.displayName(for: kind))
                .font(.jost(10, weight: .bold))
                .foregroundStyle(tint)
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 4)
        .background(
            Capsule().fill(tint.opacity(0.15))
        )
    }
}

/// Badge for a plain shell pane with no recognized agent.
struct CommandLineBadge: View {
    private var tint: Color { .secondary }

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "terminal")
                .font(.jost(10, weight: .bold))
                .foregroundStyle(tint)
                .accessibilityHidden(true)
            Text("Command line")
                .font(.jost(10, weight: .bold))
                .foregroundStyle(tint)
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 4)
        .background(
            Capsule().fill(Theme.subtleFill)
        )
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Command line")
    }
}

/// Brand mark from the asset catalog when present; SF Symbol otherwise.
struct AgentKindIcon: View {
    let kind: String
    var size: CGFloat = 10

    @ViewBuilder
    var body: some View {
        if let asset = AgentKindVisual.assetName(for: kind) {
            if AgentKindVisual.usesOriginalAssetColors(for: kind) {
                Image(asset)
                    .resizable()
                    .scaledToFit()
                    .frame(width: size, height: size)
                    .accessibilityHidden(true)
            } else {
                Image(asset)
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .frame(width: size, height: size)
                    .foregroundStyle(AgentKindVisual.color(for: kind))
                    .accessibilityHidden(true)
            }
        } else {
            Image(systemName: AgentKindVisual.symbol(for: kind))
                .font(.jost(size, weight: .bold))
                .foregroundStyle(AgentKindVisual.color(for: kind))
                .accessibilityHidden(true)
        }
    }
}

// MARK: - Quiet list rows

/// Shared geometry for the Spaces and Agents lists: a narrow leading status
/// column replaces the `#number`, so every title starts at the same x.
enum QuietListMetrics {
    static let statusColumn: CGFloat = 14
    static let columnSpacing: CGFloat = 10
    static var titleInset: CGFloat { statusColumn + columnSpacing }
}

/// Hairline under a list row, inset to the title column.
struct QuietListDivider: View {
    var leadingInset: CGFloat = QuietListMetrics.titleInset

    var body: some View {
        Rectangle()
            .fill(Theme.hairline)
            .frame(height: 0.5)
            .padding(.leading, leadingInset)
    }
}

/// Last-activity time pinned to a row's top-right corner.
struct RowTimestamp: View {
    let date: Date?

    var body: some View {
        if let date {
            Text(PaneUpdatedFormat.label(for: date))
                .font(.jost(.caption))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .fixedSize()
        }
    }
}

// MARK: - Stat chip

struct StatChip: View {
    let icon: String
    let value: String
    let caption: String

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: icon)
                .font(.jost(10, weight: .semibold))
            Text(value)
                .font(.jost(13, weight: .medium))
                .monospacedDigit()
        }
        .foregroundStyle(.secondary)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(caption): \(value)")
    }
}

// MARK: - Repo pill

struct RepoPill: View {
    let worktree: Worktree

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: worktree.isLinkedWorktree ? "arrow.triangle.branch" : "folder.fill")
                .font(.jost(9, weight: .semibold))
            Text(worktree.repoName)
                .font(.jost(11, weight: .medium))
                .lineLimit(1)
        }
        .foregroundStyle(.secondary)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Capsule().fill(Theme.subtleFill))
    }
}

// MARK: - Error banner

struct ErrorBanner: View {
    let message: String
    var retry: (() -> Void)?

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Theme.warning)
                .font(.jost(14, weight: .semibold))
            Text(message)
                .font(.jost(.footnote))
                .foregroundStyle(.primary.opacity(0.85))
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            if let retry {
                Button("Retry", action: retry)
                    .buttonStyle(.herdrSecondary(fullWidth: false))
                    .controlSize(.small)
            }
        }
        .padding(12)
        .background(
            RoundedRectangle.continuous(DesignSystem.CornerRadius.lg)
                .fill(Theme.warning.opacity(0.12))
        )
    }
}

// MARK: - Connection status bar

struct ConnectionStatusBar: View {
    let banner: ConnectionBanner
    var retryAction: (() -> Void)?

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: banner.systemImage)
                .font(.jost(13, weight: .bold))

            Text(banner.message)
                .font(.jost(13, weight: .semibold))
                .lineLimit(1)

            if banner.showsProgress {
                CircularArcSpinner(size: 13, color: banner.tintColor)
            } else if banner.canRetry, let retryAction {
                Button(action: retryAction) {
                    Text("Retry")
                        .font(.jost(12, weight: .bold))
                        .foregroundStyle(banner.tintColor)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 3)
                        .background(
                            Capsule().fill(banner.tintColor.opacity(0.18))
                        )
                }
                .buttonStyle(.plain)
            }
        }
        .foregroundStyle(banner.tintColor)
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity)
        .background(Color.clear)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(banner.message)
        .accessibilityIdentifier("connection-status-bar")
    }
}

struct ConnectionStatusBannerModifier: ViewModifier {
    let banner: ConnectionBanner?
    var retryAction: (() -> Void)?

    func body(content: Content) -> some View {
        content
            .safeAreaInset(edge: .top, spacing: 0) {
                if let banner {
                    ConnectionStatusBar(banner: banner, retryAction: retryAction)
                        .transition(.move(edge: .top).combined(with: .opacity))
                }
            }
            .animation(.spring(response: 0.35, dampingFraction: 0.8), value: banner)
    }
}

extension View {
    /// Insets the top of the scrollable content with the connection status bar,
    /// ensuring it sits beneath the navigation bar without overlapping content.
    func connectionStatusBanner(banner: ConnectionBanner?, retryAction: (() -> Void)? = nil) -> some View {
        modifier(ConnectionStatusBannerModifier(banner: banner, retryAction: retryAction))
    }

    /// Convenience overload resolving the active gateway session from the given `AppModel`.
    func connectionStatusBanner(appModel: AppModel) -> some View {
        let sessionModel = appModel.machineCatalog
        return connectionStatusBanner(banner: sessionModel.connectionBanner) {
            Task { await sessionModel.reconnect() }
        }
    }
}

// MARK: - Herd cat sprite

/// The running cat shared by the loading and offline states, drawn in sprite units
/// around its body so callers only translate and scale the context.
private enum HerdCatSprite {
    static let furs: [Color] = [
        Color(red: 0.78, green: 0.64, blue: 0.48),
        Color(red: 0.62, green: 0.67, blue: 0.73),
        Color(red: 0.91, green: 0.69, blue: 0.39)
    ]

    static func draw(in context: GraphicsContext, fur: Color, stride: Int) {
        func block(_ x: Int, _ y: Int, _ width: Int, _ height: Int, _ color: Color) {
            context.fill(Path(CGRect(x: x, y: y, width: width, height: height)), with: .color(color))
        }
        // Hooked tail, stretched body, ears, and alternating running paws.
        block(-13, -7, 2, 7, fur)
        block(-15, -8, 4, 2, fur)
        block(-11, -2, 15, 6, fur)
        block(1, -6, 7, 8, fur)
        block(1, -9, 2, 4, fur)
        block(6, -9, 2, 4, fur)
        block(-9 - stride, 3, 3, 4, fur)
        block(1 + stride, 3, 3, 4, fur)
        block(5, -4, 1, 2, Color.black.opacity(0.8))
        block(7, 0, 2, 1, Color(red: 0.85, green: 0.53, blue: 0.49))
    }
}

// MARK: - Loading cat

/// A single herd cat trotting across, matching the offline herd's sprite and motion.
struct LazyCatLoadingView: View {
    let message: String

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @State private var startedAt = Date()

    var body: some View {
        VStack(spacing: 20) {
            TimelineView(.animation(minimumInterval: 1.0 / 12, paused: reduceMotion || scenePhase != .active)) { timeline in
                Canvas { context, size in
                    let elapsed = max(0, timeline.date.timeIntervalSince(startedAt))
                    let scale: CGFloat = 2
                    // Sprite spans x -15...8, so the cat is 23 units wide.
                    let width = 23 * scale
                    let groundY = size.height / 2 + 7 * scale + 3
                    // Start in view so even a quick load gets a glimpse of the cat.
                    let travel = (elapsed * 40 + width + size.width * 0.12)
                        .truncatingRemainder(dividingBy: size.width + width)
                    let x = reduceMotion ? size.width / 2 + 3.5 * scale : travel - width + 15 * scale
                    let stride = reduceMotion ? 0 : [0, 2, 0, -2][Int(elapsed * 10) % 4]
                    let bounce = reduceMotion ? 0 : abs(sin(elapsed * 14)) * 3

                    var ground = Path()
                    ground.move(to: CGPoint(x: size.width / 2 - 60, y: groundY))
                    ground.addLine(to: CGPoint(x: size.width / 2 + 60, y: groundY))
                    context.stroke(ground, with: .color(Theme.border), style: StrokeStyle(lineWidth: 5, lineCap: .round))

                    var cat = context
                    cat.translateBy(x: x, y: size.height / 2 - bounce)
                    cat.scaleBy(x: scale, y: scale)
                    HerdCatSprite.draw(in: cat, fur: HerdCatSprite.furs[0], stride: stride)
                }
            }
            .frame(height: 92)
            .clipped()
            .accessibilityHidden(true)

            Text(message)
                .font(.jost(.footnote))
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(message)
        .accessibilityIdentifier("lazy-cat-loading")
    }
}

#Preview("Loading cat — dark") {
    LazyCatLoadingView(message: "Loading spaces…")
        .background(Theme.backgroundGradient)
        .preferredColorScheme(.dark)
}

#Preview("Loading cat — light") {
    LazyCatLoadingView(message: "Loading spaces…")
        .background(Theme.backgroundGradient)
        .preferredColorScheme(.light)
}

// MARK: - Wandering herd canvas

/// Center ring with pixel cats. Radial (offline) or scattered (launch) layouts.
struct HerdWanderCanvas: View {
    enum Layout {
        /// Cats radiate from the ring — used by the offline empty state.
        case radial
        /// Cats are jittered across the canvas and stroll locally — launch overlay.
        case scattered
    }

    var catCount: Int = 6
    var layout: Layout = .radial
    /// Max radial travel for `.radial`. Ignored when scattered.
    var travelRadius: CGFloat = 124

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    @State private var startedAt = Date()

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 12, paused: reduceMotion || scenePhase != .active)) { timeline in
            Canvas { context, size in
                let elapsed = max(0, timeline.date.timeIntervalSince(startedAt))
                let center = CGPoint(x: size.width / 2, y: size.height / 2)
                let ring = CGRect(x: center.x - 60, y: center.y - 36, width: 120, height: 72)
                context.stroke(Path(ellipseIn: ring), with: .color(Theme.border), lineWidth: 5)

                let count = max(1, catCount)
                switch layout {
                case .radial:
                    drawRadial(context: context, elapsed: elapsed, center: center, count: count)
                case .scattered:
                    drawScattered(context: context, elapsed: elapsed, size: size, count: count)
                }
            }
        }
        .accessibilityHidden(true)
    }

    private func drawRadial(
        context: GraphicsContext, elapsed: Double, center: CGPoint, count: Int
    ) {
        for index in 0..<count {
            let phase = reduceMotion
                ? 0.45 + Double(index % 3) * 0.15
                : (elapsed / 2.8 + Double(index) / Double(count))
                    .truncatingRemainder(dividingBy: 1)
            let angle = Double(index) * (.pi * 2) / Double(count) + 0.25
            let distance = 12 + phase * travelRadius
            let stride = reduceMotion ? 0 : [0, 2, 0, -2][Int(elapsed * 10 + Double(index)) % 4]
            let bounce = reduceMotion ? 0 : abs(sin(elapsed * 14 + Double(index))) * 3
            var cat = context
            cat.opacity = reduceMotion ? 1 : min(1, (1 - phase) * 5)
            cat.translateBy(
                x: center.x + cos(angle) * distance,
                y: center.y + sin(angle) * distance * 0.5 - bounce
            )
            cat.scaleBy(x: cos(angle) < 0 ? -2 : 2, y: 2)
            HerdCatSprite.draw(
                in: cat, fur: HerdCatSprite.furs[index % HerdCatSprite.furs.count], stride: stride
            )
        }
    }

    private func drawScattered(
        context: GraphicsContext, elapsed: Double, size: CGSize, count: Int
    ) {
        // Keep cats clear of the ring and the bottom message band.
        let marginX: CGFloat = 36
        let marginTop: CGFloat = 72
        let marginBottom: CGFloat = 110
        let usableW = max(1, size.width - marginX * 2)
        let usableH = max(1, size.height - marginTop - marginBottom)

        for index in 0..<count {
            let ux = Self.unit(index, salt: 1.17)
            let uy = Self.unit(index, salt: 7.91)
            let speed = Self.unit(index, salt: 3.33)
            let homeX = marginX + CGFloat(ux) * usableW
            let homeY = marginTop + CGFloat(uy) * usableH

            let wander = reduceMotion ? 0 : elapsed * (0.55 + speed * 0.9) + Double(index) * 1.7
            let dx = reduceMotion ? 0 : sin(wander) * (24 + uy * 52)
            let dy = reduceMotion ? 0 : cos(wander * 0.73 + ux * 2) * (14 + ux * 28)
            let stride = reduceMotion ? 0 : [0, 2, 0, -2][Int(elapsed * 10 + Double(index) * 1.3) % 4]
            let bounce = reduceMotion ? 0 : abs(sin(elapsed * 14 + Double(index))) * 3
            let facingLeft = dx < 0

            var cat = context
            cat.translateBy(x: homeX + dx, y: homeY + dy - bounce)
            cat.scaleBy(x: facingLeft ? -2 : 2, y: 2)
            HerdCatSprite.draw(
                in: cat, fur: HerdCatSprite.furs[index % HerdCatSprite.furs.count], stride: stride
            )
        }
    }

    /// Stable 0…1 hash so scatter positions stay fixed across launches.
    private static func unit(_ index: Int, salt: Double) -> Double {
        let value = sin(Double(index + 1) * 12.9898 + salt) * 43758.5453
        return value - floor(value)
    }
}

// MARK: - Launch herd loading

/// Full-screen wandering herd used during auto-connect handoff from the splash.
struct LaunchHerdLoadingView: View {
    let message: String

    var body: some View {
        ZStack {
            HerdWanderCanvas(catCount: 16, layout: .scattered)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .clipped()

            VStack {
                Spacer()
                Text(message)
                    .font(.jost(.footnote))
                    .foregroundStyle(.secondary)
                    .padding(.bottom, 48)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(message)
        .accessibilityIdentifier("lazy-cat-loading")
    }
}

#Preview("Launch herd — dark") {
    LaunchHerdLoadingView(message: "Connecting…")
        .background(Theme.backgroundGradient)
        .preferredColorScheme(.dark)
}

// MARK: - Offline herd

/// Decorative motion stays local to the empty state and never drives networking.
struct OfflineHerdView: View {
    var body: some View {
        VStack(spacing: 12) {
            HerdWanderCanvas(catCount: 6, layout: .radial, travelRadius: 124)
                .frame(maxWidth: 320)
                .frame(height: 190)
                .clipped()

            Text("The herd got loose")
                .font(.jost(.headline))
            Text("You're offline. Reconnect to bring the cats back.")
                .font(.jost(.subheadline))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("offline-herd")
    }
}

#Preview("Offline herd") {
    OfflineHerdView()
        .background(Theme.backgroundGradient)
}

#Preview("Offline herd — dark") {
    OfflineHerdView()
        .background(Theme.backgroundGradient)
        .preferredColorScheme(.dark)
}

// MARK: - Working status icon

/// Keeps activity inside the status indicator instead of animating the card edge.
struct WorkingStatusIcon: View {
    var size: CGFloat = 14

    var body: some View {
        CircularArcSpinner(size: size)
            .accessibilityLabel("Working")
    }
}

// MARK: - Circular arc spinner

/// Unified circular arc spinner matching the Live activity style in yellow.
struct CircularArcSpinner: View {
    var label: String? = nil
    var size: CGFloat = 16
    var lineWidth: CGFloat? = nil
    var color: Color = AgentStatus.working.color

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase

    private var effectiveLineWidth: CGFloat {
        lineWidth ?? max(1.5, size * 0.13)
    }

    var body: some View {
        let spinner = TimelineView(.animation(minimumInterval: 1.0 / 30, paused: reduceMotion || scenePhase != .active)) { context in
            let phase = reduceMotion ? 0 : context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: 1.8) / 1.8
            Circle()
                .trim(from: 0.12, to: 0.82)
                .stroke(color, style: StrokeStyle(lineWidth: effectiveLineWidth, lineCap: .round))
                .rotationEffect(.degrees(phase * 360))
                .frame(width: size, height: size)
        }
        .accessibilityLabel(label ?? "Loading")

        if let label {
            VStack(spacing: 8) {
                spinner
                Text(label)
                    .font(.jost(.footnote))
                    .foregroundStyle(.secondary)
            }
        } else {
            spinner
        }
    }
}

extension CircularArcSpinner {
    init(_ label: String, size: CGFloat = 24, color: Color = AgentStatus.working.color) {
        self.label = label
        self.size = size
        self.lineWidth = nil
        self.color = color
    }
}

// MARK: - Host key approval

extension View {
    /// Presents SSH host key approval or verified replacement; `onApproved` retries the connection.
    func hostKeyApprovalSheet(onApproved: @escaping @MainActor () async -> Void) -> some View {
        modifier(HostKeyApprovalModifier(onApproved: onApproved))
    }
}

private struct HostKeyApprovalModifier: ViewModifier {
    @Environment(AppModel.self) private var appModel
    let onApproved: @MainActor () async -> Void

    func body(content: Content) -> some View {
        content.sheet(item: Binding(
            get: { appModel.hostKeyChallenge.map(HostKeyApprovalItem.init) },
            set: { if $0 == nil, appModel.hostKeyChallenge != nil { appModel.cancelHostKeyApproval() } }
        )) { item in
            HostKeyApprovalView(challenge: item.challenge, onApproved: onApproved)
                .id(item.id)
        }
    }
}

private struct HostKeyApprovalItem: Identifiable {
    let challenge: SSHHostKeyChallenge
    var id: String { "\(challenge.host):\(challenge.port) \(challenge.publicKey) \(challenge.previousPublicKey ?? "")" }
}

private struct HostKeyApprovalView: View {
    @Environment(AppModel.self) private var appModel
    let challenge: SSHHostKeyChallenge
    let onApproved: @MainActor () async -> Void
    @State private var isApproving = false
    @State private var approvalError: String?
    @State private var verifiedReplacement = false

    private var isReplacement: Bool { challenge.previousPublicKey != nil }

    private func fingerprint(_ value: String, label: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label).font(.jost(.subheadline, weight: .semibold))
            Text(value)
                .font(.system(.footnote, design: .monospaced))
                .textSelection(.enabled)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    RoundedRectangle.continuous(DesignSystem.CornerRadius.lg)
                        .fill(Theme.fieldBackground)
                )
        }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Text(isReplacement
                        ? "The SSH identity of \(challenge.host):\(challenge.port) has changed. Connection is blocked. "
                            + "This can happen after a rebuild or key rotation, but could also mean someone is impersonating your computer."
                        : "Herdcats hasn't connected to \(challenge.host):\(challenge.port) before. "
                            + "Compare this fingerprint with the server's before you trust it.")
                        .font(.jost(.subheadline))
                        .foregroundStyle(.secondary)
                    if let previousFingerprint = challenge.previousFingerprint {
                        fingerprint(previousFingerprint, label: "Previously approved fingerprint")
                    }
                    fingerprint(challenge.fingerprint, label: isReplacement ? "New fingerprint" : "Server fingerprint")
                    Text("Verify the fingerprint directly on your computer or with its administrator through a trusted channel. "
                        + "For an Ed25519 host key, run on the server: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub")
                        .font(.jost(.footnote))
                        .foregroundStyle(.secondary)
                    if isReplacement {
                        Toggle("I verified the new fingerprint through a trusted channel", isOn: $verifiedReplacement)
                            .font(.jost(.subheadline))
                            .disabled(isApproving)
                        Text("Replacing this key updates trust only for this hostname and port. Your login credentials are kept.")
                            .font(.jost(.footnote))
                            .foregroundStyle(.secondary)
                    }
                    if let error = approvalError {
                        Text(error)
                            .font(.jost(.footnote))
                            .foregroundStyle(.secondary)
                    }
                    Button {
                        Task {
                            isApproving = true
                            defer { isApproving = false }
                            approvalError = nil
                            if await appModel.approveHostKey(challenge) {
                                await onApproved()
                            } else {
                                approvalError = appModel.lastError
                            }
                        }
                    } label: {
                        Text(isReplacement ? "Replace Approved Key and Connect" : "Trust and Connect")
                    }
                    .buttonStyle(.herdrPrimary)
                    .disabled(isApproving || (isReplacement && !verifiedReplacement))
                }
                .padding(20)
            }
            .background(Theme.backgroundGradient)
            .navigationTitle(isReplacement ? "Host Key Changed" : "Verify Host Key")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button {
                        appModel.cancelHostKeyApproval()
                    } label: {
                        Image(systemName: "xmark")
                            .font(.jost(16, weight: .semibold))
                            .foregroundStyle(.primary)
                    }
                    .accessibilityLabel("Close")
                    .disabled(isApproving)
                }
            }
        }
        .interactiveDismissDisabled(isApproving)
    }
}
