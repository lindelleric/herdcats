import SwiftUI

struct ApprovalCard: View {
    let pending: AgentPendingApproval
    let response: ApprovalResponseState
    let onRespond: (AgentApprovalOption) -> Void
    private var toolTitle: String {
        let entry = AgentTimelineEntry(id: pending.requestId, kind: "tool", text: "",
            role: nil, toolName: pending.toolName, status: nil, details: nil, truncated: false)
        return TimelineActivitySummary.isCommand(entry) ? "Run command" : TimelineActivitySummary.purpose(entry)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            commandBox
            if let text = pending.agent == "codex" ? pending.justification : pending.description {
                Text(verbatim: text)
                    .font(.jost(.subheadline)).foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }
            if let environment = pending.environment {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Environment").font(.jost(.caption, weight: .semibold)).foregroundStyle(.secondary)
                    Text(verbatim: environment).font(.jost(.caption)).fixedSize(horizontal: false, vertical: true)
                }
            }
            options
            if let message = response.message {
                Text(message).font(.jost(.caption))
                    .foregroundStyle(response.isError ? Theme.destructive : Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("approval-response-message")
            }
        }
        .padding(16)
        .foregroundStyle(Theme.text)
        .herdrCard()
        .accessibilityIdentifier("timeline-approval-card")
    }
    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("\(pending.agent == "codex" ? "Codex" : "Claude") · \(toolTitle)")
                    .font(.jost(.caption, weight: .semibold)).foregroundStyle(.secondary)
                Spacer(minLength: 8)
                if response.phase == .sending { ProgressView().accessibilityLabel("Sending answer") }
            }
            Text(verbatim: pending.title)
                .font(.jost(.title3, weight: .semibold))
                .fixedSize(horizontal: false, vertical: true)
        }
    }
    private var commandBox: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Command").font(.jost(.caption, weight: .semibold)).foregroundStyle(.secondary)
            Text(verbatim: pending.command)
                .font(.system(size: 13, design: .monospaced))
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(Theme.fieldBackground, in: RoundedRectangle.continuous(DesignSystem.CornerRadius.md))
        }
    }
    private var options: some View {
        VStack(spacing: 10) {
            ForEach(Array(pending.options.enumerated()), id: \.element.number) { index, option in
                optionControl(option, index: index)
            }
        }
        .disabled(response.isDisabled)
    }
    @ViewBuilder private func optionControl(_ option: AgentApprovalOption, index: Int) -> some View {
        if ApprovalResponseState.requiresSwipe(index: index, count: pending.options.count) {
            SwipeToConfirm(labelLines: option.labelLines) { onRespond(option) }
        } else if index == 0 {
            optionButton(option).buttonStyle(.herdrPrimary())
        } else {
            optionButton(option).buttonStyle(ApprovalOptionButtonStyle())
        }
    }
    private func optionButton(_ option: AgentApprovalOption) -> some View {
        Button { onRespond(option) } label: {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(option.labelLines.enumerated()), id: \.offset) { _, line in
                    Text(verbatim: line).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .accessibilityLabel(option.labelLines.joined(separator: "\n"))
    }
}

struct ApprovalOptionButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        let shape = RoundedRectangle.continuous(DesignSystem.CornerRadius.button)
        configuration.label
            .font(.jost(14, weight: .bold))
            .foregroundStyle(Theme.text)
            .padding(.vertical, DesignSystem.Spacing.md)
            .padding(.horizontal, DesignSystem.Spacing.base)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .background(shape.fill(configuration.isPressed ? Theme.subtleFill : Theme.fieldBackground))
            .contentShape(shape)
            .offset(y: configuration.isPressed ? 1 : 0)
            .opacity(isEnabled ? 1 : 0.5)
    }
}

struct SwipeToConfirm: View {
    let labelLines: [String]
    let onConfirm: () -> Void
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var offset: CGFloat = 0
    private let thumbWidth: CGFloat = 44

    private var label: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(labelLines.enumerated()), id: \.offset) { _, line in
                Text(verbatim: line).frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .font(.jost(14, weight: .bold))
        .padding(.vertical, DesignSystem.Spacing.md)
        .padding(.leading, thumbWidth + 12)
        .padding(.trailing, DesignSystem.Spacing.base)
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
    }
    var body: some View {
        label
            .foregroundStyle(Theme.text)
            .background(Theme.fieldBackground, in: RoundedRectangle.continuous(DesignSystem.CornerRadius.button))
            .overlay {
                GeometryReader { geometry in
                    let travel = max(0, geometry.size.width - thumbWidth)
                    let fillWidth = thumbWidth + offset
                    ZStack(alignment: .leading) {
                        RoundedRectangle.continuous(DesignSystem.CornerRadius.button)
                            .fill(Theme.accentFill)
                            .frame(width: fillWidth)
                            .allowsHitTesting(false)
                        label
                            .foregroundStyle(Theme.onPrimary)
                            .frame(width: geometry.size.width, height: geometry.size.height)
                            .mask(alignment: .leading) { Rectangle().frame(width: fillWidth) }
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                        Image(systemName: "chevron.right")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(Theme.onPrimary)
                            .frame(width: thumbWidth, height: geometry.size.height)
                            .background(Theme.accentFill, in: RoundedRectangle.continuous(DesignSystem.CornerRadius.button))
                            .offset(x: offset)
                            .simultaneousGesture(DragGesture(minimumDistance: 12)
                                .onChanged { value in
                                    guard isEnabled, abs(value.translation.width) > abs(value.translation.height) else { return }
                                    offset = min(travel, max(0, value.translation.width))
                                }
                                .onEnded { value in
                                    let confirmed = isEnabled && travel > 0 && offset >= travel * 0.9
                                        && value.translation.width > abs(value.translation.height)
                                    withAnimation(reduceMotion ? nil : .easeOut(duration: 0.2)) { offset = 0 }
                                    if confirmed { onConfirm() }
                                })
                    }
                    .clipShape(RoundedRectangle.continuous(DesignSystem.CornerRadius.button))
                }
            }
            .opacity(isEnabled ? 1 : 0.5)
            .onChange(of: isEnabled) { _, _ in offset = 0 }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(labelLines.joined(separator: "\n"))
            .accessibilityHint("Swipe right to confirm, or use the Confirm action.")
            .accessibilityAddTraits(.isButton)
            .accessibilityAction(named: "Confirm") { if isEnabled { onConfirm() } }
    }
}
