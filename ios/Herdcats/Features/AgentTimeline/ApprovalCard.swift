import SwiftUI
import UIKit

struct ApprovalCard: View {
    let pending: AgentPendingApproval
    let response: ApprovalResponseState
    let onRespond: (AgentApprovalOption) -> Void
    var onMeasure: (CGFloat, CGFloat) -> Void = { _, _ in }
    @State private var contentHeight: CGFloat = 0
    @State private var peekHeight: CGFloat = 0
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
                    Group {
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
                    .background {
                        GeometryReader { geometry in
                            if index == 0 {
                                Color.clear.onChange(of: geometry.frame(in: .named("approval-content")).maxY, initial: true) { _, bottom in
                                    peekHeight = bottom + 16
                                    onMeasure(contentHeight, peekHeight)
                                }
                            }
                        }
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
            .coordinateSpace(name: "approval-content")
            .background {
                GeometryReader { geometry in
                    Color.clear.onChange(of: geometry.size.height, initial: true) { _, height in
                        contentHeight = height
                        onMeasure(contentHeight, peekHeight)
                    }
                }
            }
        }
        .foregroundStyle(Theme.text)
        .background(Theme.background)
        .accessibilityIdentifier("timeline-approval-card")
    }
}

struct ApprovalDrawer: View {
    let pending: AgentPendingApproval
    let response: ApprovalResponseState
    let onRespond: (AgentApprovalOption) -> Void
    @State private var contentHeight: CGFloat = 0
    @State private var peekHeight: CGFloat = 0
    @State private var availableHeight: CGFloat = 0
    @State private var selected: PresentationDetent = .large
    @State private var userSelected = false

    private var fitted: CGFloat { contentHeight + 32 }
    private var peek: CGFloat { min(max(180, peekHeight + 32), max(180, availableHeight - 80)) }
    private var detents: Set<PresentationDetent> {
        var values: Set<PresentationDetent> = [.large]
        if availableHeight > 260, peekHeight > 0 { values.insert(.height(peek)) }
        if contentHeight > 0, fitted < availableHeight { values.insert(.height(fitted)) }
        return values
    }
    var body: some View {
        GeometryReader { geometry in
            ApprovalCard(pending: pending, response: response, onRespond: onRespond) { full, first in
                contentHeight = full; peekHeight = first
                fitInitially()
            }
            .padding(.top, 20)
            .onChange(of: geometry.size.height, initial: true) { _, height in
                availableHeight = max(availableHeight, height)
                fitInitially()
            }
        }
        .presentationDetents(detents, selection: Binding(get: { selected }, set: { selected = $0; userSelected = true }))
        .presentationDragIndicator(.visible)
        .presentationBackground(Theme.background)
        .presentationBackgroundInteraction(.enabled)
        .presentationCornerRadius(28)
        .interactiveDismissDisabled()
        .onAppear {
            UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        }
    }
    private func fitInitially() {
        guard !userSelected else { return }
        selected = contentHeight > 0 && fitted < availableHeight ? .height(fitted) : .large
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
