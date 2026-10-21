import SwiftUI

struct ApprovalCard: View {
    let pending: AgentPendingApproval
    let response: ApprovalResponseState
    let maximumHeight: CGFloat
    let onRespond: (AgentApprovalOption) -> Void
    @State private var contentHeight: CGFloat = 360
    @State private var commandHeight: CGFloat = 60

    private var toolTitle: String {
        TimelineActivitySummary.purpose(AgentTimelineEntry(id: pending.requestId, kind: "tool", text: "",
            role: nil, toolName: pending.toolName, status: nil, details: nil, truncated: false))
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text("\(pending.agent == "codex" ? "Codex" : "Claude") · \(toolTitle)")
                        .font(.jost(.subheadline, weight: .semibold))
                    Spacer(minLength: 8)
                    if response.phase == .sending { ProgressView().accessibilityLabel("Sending answer") }
                }
                Text(verbatim: pending.title).font(.jost(.subheadline))
                ScrollView {
                    Text(verbatim: pending.command)
                        .font(.system(.caption, design: .monospaced))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                        .background {
                            GeometryReader { geometry in
                                Color.clear.onChange(of: geometry.size.height, initial: true) { _, height in commandHeight = height }
                            }
                        }
                }
                .frame(height: min(120, max(20, commandHeight)))
                if let text = pending.agent == "codex" ? pending.justification : pending.description {
                    Text(verbatim: text).font(.jost(.subheadline)).textSelection(.enabled)
                }
                if let environment = pending.environment {
                    Text(verbatim: environment).font(.jost(.caption)).foregroundStyle(Theme.textMuted)
                }
                ForEach(Array(pending.options.enumerated()), id: \.element.number) { index, option in
                    if ApprovalResponseState.requiresSwipe(index: index, count: pending.options.count) {
                        SwipeToConfirm(labelLines: option.labelLines) { onRespond(option) }
                    } else {
                        Button { onRespond(option) } label: {
                            VStack(alignment: .leading, spacing: 2) {
                                ForEach(Array(option.labelLines.enumerated()), id: \.offset) { _, line in
                                    Text(verbatim: line).frame(maxWidth: .infinity, alignment: .leading)
                                }
                            }
                            .font(.jost(.subheadline))
                            .padding(12)
                            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                            .background(Theme.fieldBackground, in: RoundedRectangle(cornerRadius: 12))
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(option.labelLines.joined(separator: "\n"))
                    }
                }
                .disabled(response.isDisabled)
                if let message = response.message {
                    Text(message).font(.jost(.caption))
                        .foregroundStyle(response.isError ? Theme.destructive : Theme.textMuted)
                        .accessibilityIdentifier("approval-response-message")
                }
            }
            .padding(16)
            .background {
                GeometryReader { geometry in
                    Color.clear.onChange(of: geometry.size.height, initial: true) { _, height in contentHeight = height }
                }
            }
        }
        .frame(height: min(maximumHeight, contentHeight))
        .foregroundStyle(Theme.text)
        .herdrCard()
        .accessibilityIdentifier("timeline-approval-card")
    }
}

struct SwipeToConfirm: View {
    let labelLines: [String]
    let onConfirm: () -> Void
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var offset: CGFloat = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(labelLines.enumerated()), id: \.offset) { _, line in
                Text(verbatim: line).frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .font(.jost(.subheadline))
        .padding(.vertical, 14)
        .padding(.leading, 60)
        .padding(.trailing, 12)
        .frame(maxWidth: .infinity, minHeight: 52, alignment: .leading)
        .background(Theme.fieldBackground, in: RoundedRectangle(cornerRadius: 12))
        .overlay {
            GeometryReader { geometry in
                let travel = max(0, geometry.size.width - 52)
                Image(systemName: "chevron.right")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(Theme.onPrimary)
                    .frame(width: 44, height: max(44, geometry.size.height - 8))
                    .background(Theme.accentFill, in: RoundedRectangle(cornerRadius: 8))
                    .offset(x: 4 + offset, y: 4)
                    .gesture(DragGesture(minimumDistance: 12)
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
