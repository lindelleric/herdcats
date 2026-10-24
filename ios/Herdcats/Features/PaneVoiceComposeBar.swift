import PhotosUI
import SwiftUI
import UIKit

// MARK: - Bar Mode & Modifier Enums

/// Visual and interaction modes of `PaneVoiceComposeBar`.
enum PaneVoiceComposeBarMode: String, CaseIterable, Identifiable, Sendable {
    case live
    case recording
    case compose

    var id: String { rawValue }
}

/// One-shot modifier keys available in Live input.
enum PaneLiveModifier: String, CaseIterable, Identifiable, Hashable, Sendable {
    case ctrl
    case alt
    case shift

    var id: String { rawValue }

    var displayLabel: String {
        switch self {
        case .ctrl: return "Ctrl"
        case .alt: return "Alt"
        case .shift: return "Shift"
        }
    }
}

/// One solid surface for a whole pane input group. The controls inside keep
/// their plain style so the Live row does not turn into separate pills.
/// Moonlit uses no glass or blur: `--surface` with a `--border` outline.
private struct PaneInputGlassSurface: ViewModifier {
    let cornerRadius: CGFloat

    func body(content: Content) -> some View {
        let shape = RoundedRectangle.continuous(cornerRadius)
        content
            .background(Theme.cardBackground, in: shape)
            .overlay(shape.strokeBorder(Theme.hairline, lineWidth: 1))
    }
}

/// Gives the arrow button one outcome per touch: a tap sends Up, while a
/// completed long press opens the direction row. The gestures are exclusive,
/// so releasing after a long hold cannot trigger the tap action.
private struct PaneTapOrLongPressStyle: PrimitiveButtonStyle {
    let isEnabled: Bool
    let onLongPress: () -> Void

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .contentShape(Rectangle())
            .gesture(
                LongPressGesture(minimumDuration: 0.45)
                    .exclusively(before: TapGesture())
                    .onEnded { result in
                        guard isEnabled else { return }
                        switch result {
                        case .first(true): onLongPress()
                        case .second: configuration.trigger()
                        default: break
                        }
                    }
            )
    }
}

// MARK: - Attachment Info

/// Image attachment metadata for the Compose bar.
struct PaneAttachmentInfo: Identifiable, Sendable {
    var id: UUID
    var preview: UIImage
    var remotePath: String?
    var uploadError: String?
    var isUploading: Bool

    init(
        id: UUID = UUID(),
        preview: UIImage,
        remotePath: String? = nil,
        uploadError: String? = nil,
        isUploading: Bool = false
    ) {
        self.id = id
        self.preview = preview
        self.remotePath = remotePath
        self.uploadError = uploadError
        self.isUploading = isUploading
    }
}

// MARK: - PaneVoiceComposeBar

/// A unified SwiftUI input bar for Herdr terminal panes supporting:
/// 1. Default Live mode: fixed rail (Esc, Tab, Up, Ctrl, Alt, Keys, Voice, Keyboard).
/// 2. Recording mode: recording status with exactly one checkmark finish action.
/// 3. Compose mode: message bubble with editable 1-to-6-row field, external image picker,
///    Voice immediately left of Send, and Send hidden for blank/whitespace text.
struct PaneVoiceComposeBar: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var showsModelConfiguration = false
    var modelControl: AgentModelControlState? = nil
    @State private var isKeysDrawerOpen = false
    @State private var isArrowRowOpen = false

    private struct ExtraKey: Identifiable {
        let label: String
        let token: String
        var id: String { token }
    }

    private let arrowKeys = [
        ExtraKey(label: "←", token: "left"), ExtraKey(label: "↑", token: "up"),
        ExtraKey(label: "↓", token: "down"), ExtraKey(label: "→", token: "right")
    ]
    private let navigationKeys = [
        ExtraKey(label: "Home", token: "home"), ExtraKey(label: "End", token: "end"),
        ExtraKey(label: "Pg↑", token: "pageup"), ExtraKey(label: "Pg↓", token: "pagedown")
    ]
    private let editingKeys = [
        ExtraKey(label: "Del", token: "delete"), ExtraKey(label: "Ins", token: "insert"),
        ExtraKey(label: "Bksp", token: "backspace"), ExtraKey(label: "↵", token: "enter")
    ]

    // Mode & Draft
    @Binding var mode: PaneVoiceComposeBarMode
    @Binding var draft: String

    // Focus state (open-only for keyboard icon; dismissal via gestures/hooks)
    @Binding var isLiveKeyboardFocused: Bool
    @FocusState.Binding var isComposeFieldFocused: Bool

    // Live row state & modifier routing to Pi/Cursor's PaneLiveModifierComposer
    var isLiveEnabled: Bool
    @Binding var armedModifiers: Set<PaneLiveModifier>
    var composeFieldIdentifier = "pane-compose-field"
    var composeSendIdentifier = "pane-compose-send-button"
    var onLiveKey: (String) -> Void
    var onLiveModifier: ((PaneLiveModifier) -> Void)?
    var onLiveEvent: (PaneLiveInputEvent) -> Void

    // Voice & Dictation (Pi owns dictation lifecycle and mode transitions)
    var dictation: PaneDictation?
    var isVoiceDraining: Bool
    @Binding var dictationError: String?
    var onStartVoice: () -> Void
    var onFinishVoice: () -> Void
    var onOpenCompose: () -> Void

    // Photo & Attachment
    var photoItem: Binding<PhotosPickerItem?>?
    @Binding var isPhotoPickerPresented: Bool
    var attachment: PaneAttachmentInfo?
    var onRemoveAttachment: (() -> Void)?
    var supportsHostServices: Bool

    // Compose actions (visibility = hasDraftText || isSending; disabled = !canSend || isSending)
    var placeholder: String
    var canSend: Bool
    var isSending: Bool
    var onSend: () -> Void

    // Keyboard dismissal hook
    var onDismissKeyboard: (() -> Void)?

    init(
        mode: Binding<PaneVoiceComposeBarMode>,
        draft: Binding<String>,
        isLiveKeyboardFocused: Binding<Bool>,
        isComposeFieldFocused: FocusState<Bool>.Binding,
        isLiveEnabled: Bool = true,
        armedModifiers: Binding<Set<PaneLiveModifier>>,
        onLiveKey: @escaping (String) -> Void,
        onLiveModifier: ((PaneLiveModifier) -> Void)? = nil,
        onLiveEvent: @escaping (PaneLiveInputEvent) -> Void,
        dictation: PaneDictation? = nil,
        isVoiceDraining: Bool = false,
        dictationError: Binding<String?> = .constant(nil),
        onStartVoice: @escaping () -> Void,
        onFinishVoice: @escaping () -> Void,
        onOpenCompose: @escaping () -> Void,
        photoItem: Binding<PhotosPickerItem?>? = nil,
        isPhotoPickerPresented: Binding<Bool>,
        attachment: PaneAttachmentInfo? = nil,
        onRemoveAttachment: (() -> Void)? = nil,
        supportsHostServices: Bool = true,
        placeholder: String = "Message the agent…",
        canSend: Bool = true,
        isSending: Bool = false,
        onSend: @escaping () -> Void,
        onDismissKeyboard: (() -> Void)? = nil
    ) {
#if DEBUG && targetEnvironment(simulator)
        if ScreenshotFixtures.enabled && ScreenshotFixtures.screen == "keys" {
            _isKeysDrawerOpen = State(initialValue: true)
        }
#endif
        self._mode = mode
        self._draft = draft
        self._isLiveKeyboardFocused = isLiveKeyboardFocused
        self._isComposeFieldFocused = isComposeFieldFocused
        self.isLiveEnabled = isLiveEnabled
        self._armedModifiers = armedModifiers
        self.onLiveKey = onLiveKey
        self.onLiveModifier = onLiveModifier
        self.onLiveEvent = onLiveEvent
        self.dictation = dictation
        self.isVoiceDraining = isVoiceDraining
        self._dictationError = dictationError
        self.onStartVoice = onStartVoice
        self.onFinishVoice = onFinishVoice
        self.onOpenCompose = onOpenCompose
        self.photoItem = photoItem
        self._isPhotoPickerPresented = isPhotoPickerPresented
        self.attachment = attachment
        self.onRemoveAttachment = onRemoveAttachment
        self.supportsHostServices = supportsHostServices
        self.placeholder = placeholder
        self.canSend = canSend
        self.isSending = isSending
        self.onSend = onSend
        self.onDismissKeyboard = onDismissKeyboard
    }

    var body: some View {
        VStack(spacing: 0) {
            switch mode {
            case .live:
                liveBar
                    .transition(.opacity)
            case .recording:
                recordingBar
                    .transition(.opacity)
            case .compose:
                composeBar
                    .transition(.opacity)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background {
            // Invisible Live keyboard capture surface wired to live events
            PaneLiveKeyboardCaptureView(
                isEnabled: isLiveEnabled && mode == .live,
                isFocused: $isLiveKeyboardFocused,
                onCompositionChanged: nil,
                onEvent: { event in
                    closeLiveMenus()
                    onLiveEvent(event)
                }
            )
        }
        .animation(.snappy(duration: 0.22), value: mode)
        .onChange(of: mode) { _, newMode in
            if newMode != .live { closeLiveMenus() }
        }
        .onChange(of: isLiveEnabled) { _, enabled in
            if !enabled { closeLiveMenus() }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pane-voice-compose-bar")
        .sheet(isPresented: $showsModelConfiguration) {
            if let modelControl { AgentModelConfigurationSheet(state: modelControl) }
        }
        .onChange(of: modelControl?.isVisible) { _, visible in
            if visible != true { showsModelConfiguration = false }
        }
    }

    // MARK: - 1. Live Mode Bar

    /// Fixed rail; extra keys expand above it without shifting rail positions.
    private var liveBar: some View {
        VStack(spacing: 8) {
            if isKeysDrawerOpen { keysDrawer }
            if isArrowRowOpen {
                extraKeyRow(arrowKeys)
                    .frame(maxWidth: 240)
                    .padding(7)
                    .modifier(PaneInputGlassSurface(cornerRadius: DesignSystem.CornerRadius.lg))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            liveRail
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private var liveRail: some View {
        HStack(spacing: 0) {
            liveKeyButton("Esc", style: .destructive) {
                sendLiveKey("esc")
            }
            .accessibilityLabel("Send Escape")
            .accessibilityIdentifier("pane-live-key-esc")

            liveKeyButton("Tab") {
                sendLiveKey("tab")
            }
            .accessibilityLabel("Send Tab")
            .accessibilityIdentifier("pane-live-key-tab")

            arrowButton

            liveModifierButton("Ctrl", modifier: .ctrl)
                .accessibilityLabel("Control modifier")
                .accessibilityIdentifier("pane-live-key-ctrl")

            liveModifierButton("Alt", modifier: .alt)
                .accessibilityLabel("Alt modifier, Option on Mac")
                .accessibilityIdentifier("pane-live-key-alt")

            liveKeyButton("Keys", isSelected: isKeysDrawerOpen) {
                isKeysDrawerOpen.toggle()
                isArrowRowOpen = false
            }
            .overlay(alignment: .topTrailing) {
                if armedModifiers.contains(.shift) {
                    Text("⇧")
                        .font(.jost(10, weight: .bold))
                        .foregroundStyle(Theme.accent)
                        .frame(width: 15, height: 15)
                        .background(Theme.accent.opacity(0.16), in: Circle())
                        .padding(2)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .accessibilityLabel("Extra keyboard keys")
            .accessibilityValue(armedModifiers.contains(.shift) ? "Shift armed for next key" : (isKeysDrawerOpen ? "Open" : "Closed"))
            .accessibilityAddTraits(isKeysDrawerOpen ? .isSelected : [])
            .accessibilityIdentifier("pane-live-key-extra")

            liveDivider

            Button {
                HapticFeedback.impact(.light)
                onStartVoice()
            } label: {
                Image(systemName: "mic.fill")
                    .font(.jost(16, weight: .semibold))
                    .foregroundStyle(Color.primary)
                    .frame(maxWidth: .infinity)
                    .frame(height: 42)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Start voice recording")
            .accessibilityIdentifier("pane-live-voice-button")

            // The keyboard icon opens an editable Compose draft. A tap on pane
            // output is the separate gesture for direct terminal typing.
            Button {
                HapticFeedback.impact(.light)
                onOpenCompose()
            } label: {
                Image(systemName: "keyboard")
                    .font(.jost(16, weight: .semibold))
                    .foregroundStyle(Color.primary)
                    .frame(maxWidth: .infinity)
                    .frame(height: 42)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Compose message")
            .accessibilityIdentifier("pane-live-keyboard-button")
        }
        .frame(height: 42)
        .modifier(PaneInputGlassSurface(cornerRadius: DesignSystem.CornerRadius.lg))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pane-live-bar")
    }

    private var arrowButton: some View {
        Button {
            HapticFeedback.impact(.light)
            sendLiveKey("up")
        } label: {
            Text("↑")
                .font(.jost(.caption, weight: .semibold).monospaced())
                .foregroundStyle(Color.primary)
                .frame(maxWidth: .infinity)
                .frame(height: 42)
        }
        .buttonStyle(
            PaneTapOrLongPressStyle(isEnabled: isLiveEnabled) {
                HapticFeedback.impact(.light)
                isArrowRowOpen = true
            isKeysDrawerOpen = false
        })
        .disabled(!isLiveEnabled)
        .opacity(isLiveEnabled ? 1 : 0.45)
        .accessibilityLabel("Up arrow")
        .accessibilityHint("Long press for all arrow keys")
        .accessibilityIdentifier("pane-live-key-up")
    }

    private var keysDrawer: some View {
        VStack(spacing: 4) {
            extraKeyRow(arrowKeys)
            extraKeyRow(navigationKeys)
            extraKeyRow(editingKeys)
            HStack(spacing: 4) {
                liveModifierButton("Shift", modifier: .shift)
                    .background(Color.primary.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
                    .accessibilityLabel("Shift modifier")
                    .accessibilityIdentifier("pane-live-key-shift")
                ForEach(0..<3, id: \.self) { _ in
                    Color.clear.frame(maxWidth: .infinity)
                }
            }
            .frame(height: 42)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(1...12, id: \.self) { number in
                        extraKeyButton(ExtraKey(label: "F\(number)", token: "f\(number)"))
                            .frame(width: 54)
                    }
                }
            }
            .frame(height: 38)
            .accessibilityLabel("Function keys")
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(7)
        .modifier(PaneInputGlassSurface(cornerRadius: DesignSystem.CornerRadius.lg))
        .accessibilityIdentifier("pane-live-keys-drawer")
    }

    private func extraKeyRow(_ keys: [ExtraKey]) -> some View {
        HStack(spacing: 4) {
            ForEach(keys) { key in
                extraKeyButton(key)
            }
        }
    }

    private func extraKeyButton(_ key: ExtraKey) -> some View {
        Button {
            HapticFeedback.impact(.light)
            sendLiveKey(key.token)
        } label: {
            Text(key.label)
                .font(.jost(.caption, weight: .semibold).monospaced())
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity)
                .frame(height: 38)
                .background(Color.primary.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
        .disabled(!isLiveEnabled)
        .accessibilityLabel("Send \(key.label)")
        .accessibilityIdentifier("pane-live-extra-key-\(key.token)")
    }

    private func sendLiveKey(_ token: String) {
        closeLiveMenus()
        onLiveKey(token)
    }

    private func closeLiveMenus() {
        isKeysDrawerOpen = false
        isArrowRowOpen = false
    }

    private var liveDivider: some View {
        Rectangle()
            .fill(Color.primary.opacity(0.22))
            .frame(width: 1, height: 24)
            .accessibilityHidden(true)
    }

    private enum LiveKeyStyle {
        case plain
        case destructive
    }

    private func liveKeyButton(
        _ title: String,
        style: LiveKeyStyle = .plain,
        isSelected: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button {
            HapticFeedback.impact(.light)
            action()
        } label: {
            Text(title)
                .font(.jost(.caption, weight: .semibold).monospaced())
                .foregroundStyle(isSelected ? Theme.accent : (style == .destructive ? Theme.destructive : Color.primary))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity)
                .frame(height: 42)
                .background(isSelected ? Theme.accent.opacity(0.16) : Color.clear)
        }
        .buttonStyle(.plain)
        .disabled(!isLiveEnabled)
        .opacity(isLiveEnabled ? 1 : 0.45)
    }

    private func liveModifierButton(
        _ title: String,
        modifier: PaneLiveModifier,
        isEnabled: Bool = true,
        accessibilityHint: String? = nil
    ) -> some View {
        let isArmed = armedModifiers.contains(modifier)
        let effectiveEnabled = isLiveEnabled && isEnabled
        return Button {
            HapticFeedback.impact(.light)
            if let onLiveModifier {
                onLiveModifier(modifier)
            } else {
                if isArmed {
                    armedModifiers.remove(modifier)
                } else {
                    armedModifiers.insert(modifier)
                }
            }
        } label: {
            Text(title)
                .font(.jost(.caption, weight: .semibold).monospaced())
                .foregroundStyle(isArmed ? Theme.accent : (isEnabled ? Color.primary : Color.secondary))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity)
                .frame(height: 42)
                .background(isArmed ? Theme.accent.opacity(0.16) : Color.clear)
        }
        .buttonStyle(.plain)
        .disabled(!effectiveEnabled)
        .opacity(effectiveEnabled ? 1 : 0.45)
        .accessibilityAddTraits(isArmed ? .isSelected : [])
        .accessibilityValue(isArmed ? "Armed for next key" : "Off")
        .accessibilityHint(accessibilityHint ?? "")
    }

    // MARK: - 2. Recording Mode Bar

    /// Recording bar with exactly one action: checkmark to finish.
    /// Does NOT set mode = .compose early; Pi owns dictation finalization and the mode transition.
    private var recordingBar: some View {
        HStack(spacing: 10) {
            recordingIndicator

            Text(recordingStatusText)
                .font(.jost(.subheadline, weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)

            Spacer(minLength: 0)

            Button {
                HapticFeedback.impact(.light)
                onFinishVoice()
            } label: {
                Image(systemName: "checkmark.circle.fill")
                    .font(.jost(26))
                    .foregroundStyle(Theme.accent)
                    .frame(width: 34, height: 36)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(isVoiceDraining)
            .accessibilityLabel("Finish recording")
            .accessibilityIdentifier("pane-recording-finish-button")
        }
        .padding(.horizontal, 12)
        .frame(height: 42)
        .modifier(PaneInputGlassSurface(cornerRadius: DesignSystem.CornerRadius.lg))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pane-recording-bar")
    }

    private var isRecordingActive: Bool {
        dictation?.isBusy == true && dictation?.isPreparing == false && !isVoiceDraining
    }

    private var recordingIndicator: some View {
        ZStack {
            if isRecordingActive && !reduceMotion {
                Circle()
                    .fill(Theme.destructive.opacity(0.4))
                    .frame(width: 20, height: 20)
                    .phaseAnimator([false, true]) { content, expanded in
                        content
                            .scaleEffect(expanded ? 1.5 : 0.6)
                            .opacity(expanded ? 0 : 0.9)
                    } animation: { expanded in
                        expanded ? .easeOut(duration: 1.15) : .linear(duration: 0.01)
                    }
            }
            Circle()
                .fill(isRecordingActive ? Theme.destructive : Color.secondary)
                .frame(width: 9, height: 9)
        }
        .frame(width: 22, height: 22)
        .accessibilityHidden(true)
    }

    private var recordingStatusText: String {
        if isVoiceDraining {
            return "Finishing dictation…"
        }
        if let dictation, dictation.isPreparing {
            return "Preparing dictation…"
        }
        return "Recording message…"
    }

    // MARK: - 3. Compose Mode Bar

    /// One glass card containing the attachment preview, image picker, and
    /// editable message row. Voice stays immediately left of Send.
    private var composeBar: some View {
        VStack(spacing: 0) {
            if let attachment {
                attachmentChip(attachment)
            }

            // Conversation panes put the field on its own full-width row and
            // the photo, model and voice controls on the row under it. The
            // choice depends only on the surface, so the field keeps its
            // identity (and focus) when the model control appears.
            if modelControl != nil {
                VStack(alignment: .leading, spacing: 0) {
                    messageField
                    HStack(alignment: .center, spacing: 4) {
                        photoPickerButton
                        Spacer(minLength: 0)
                        messageActions
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
            } else {
                HStack(alignment: .bottom, spacing: 8) {
                    photoPickerButton
                    messageBubble
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
            }
        }
        .modifier(PaneInputGlassSurface(cornerRadius: DesignSystem.CornerRadius.lg))
        .simultaneousGesture(
            DragGesture(minimumDistance: 0)
                .onEnded { value in
                    guard isComposeFieldFocused || isLiveKeyboardFocused,
                          value.translation.height > 5,
                          value.translation.height > abs(value.translation.width)
                    else { return }
                    isComposeFieldFocused = false
                    isLiveKeyboardFocused = false
                    onDismissKeyboard?()
                }
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pane-compose-bar")
    }

    @ViewBuilder
    private var photoPickerButton: some View {
        if let photoItem {
            Button {
                isPhotoPickerPresented = true
            } label: {
                Image(systemName: "photo")
                    .font(.jost(20))
                    .foregroundStyle(supportsHostServices ? Theme.accent : Color.secondary)
                    .frame(width: 38, height: 38)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .photosPicker(
                isPresented: $isPhotoPickerPresented,
                selection: photoItem,
                matching: .images,
                photoLibrary: .shared()
            )
            .disabled(!supportsHostServices || isSending)
            .accessibilityLabel("Attach Image")
            .accessibilityIdentifier("pane-compose-attach-button")
        }
    }

    /// Send is visible if the draft has non-whitespace text OR a send is currently in progress.
    /// With the model control showing it stays visible (disabled while blank) so
    /// the action row keeps its shape; otherwise it hides for blank idle drafts.
    private var showsSendButton: Bool {
        isSending || modelControl?.isVisible == true
            || !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var messageBubble: some View {
        HStack(alignment: .bottom, spacing: 4) {
            messageField
            messageActions
        }
        .padding(.trailing, 4)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pane-compose-bubble")
    }

    private var messageField: some View {
        // Editable 1-to-6-row field
        TextField(placeholder, text: $draft, axis: .vertical)
            // Conversation composers match the timeline's body text size.
            .font(.jost(modelControl != nil ? .body : .subheadline))
            .lineLimit(1...6)
            .focused($isComposeFieldFocused)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled(false)
            .background(ComposeTextInputMarker())
            .disabled(isSending)
            .submitLabel(.send)
            .onSubmit {
                guard canSend && !isSending else { return }
                onSend()
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            .accessibilityLabel("Message text")
            .accessibilityIdentifier(composeFieldIdentifier)
    }

    @ViewBuilder private var messageActions: some View {
        if let modelControl, modelControl.isVisible {
            Button { showsModelConfiguration = true } label: {
                (Text(modelControl.modelName).bold()
                    + Text(modelControl.effortText.map { " " + $0 } ?? ""))
                .lineLimit(1)
                .truncationMode(.tail)
                .font(.jost(.subheadline))
                .foregroundStyle(Theme.accent)
                // Hug the text: the mic glyph sits ~10pt inside its 32pt frame and
                // Send's circle ~4pt inside its 34pt frame, so 3pt here makes the
                // label-to-mic gap match the mic-to-Send gap (measured ~18pt each).
                .padding(.trailing, 3)
                .frame(minHeight: 36)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .layoutPriority(-1)
            .accessibilityLabel(modelControl.accessibilityLabel)
            .accessibilityHint("Configure model and effort")
            .accessibilityIdentifier("timeline-model-control")
        }

        // Voice button inside bubble, immediately left of Send
        Button {
            HapticFeedback.impact(.light)
            onStartVoice()
        } label: {
            Image(systemName: "mic.fill")
                .font(.jost(18))
                .foregroundStyle(Color.secondary)
                .frame(width: 32, height: 36)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(isSending)
        .accessibilityLabel("Dictate message")
        .accessibilityIdentifier("pane-compose-voice-button")
        .fixedSize()

        // Send button: visible when text is present or when sending; shows spinner during send
        if showsSendButton {
            Button {
                HapticFeedback.impact(.light)
                onSend()
            } label: {
                Group {
                    if isSending {
                        CircularArcSpinner(size: 18)
                    } else {
                        Image(systemName: "arrow.up.circle.fill")
                            .font(.jost(26))
                            .foregroundStyle(canSend ? Theme.accent : Color.secondary)
                    }
                }
                .frame(width: 34, height: 36)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!canSend || isSending)
            .accessibilityLabel("Send Message")
            .accessibilityIdentifier(composeSendIdentifier)
            .fixedSize()
            .transition(.scale(scale: 0.85).combined(with: .opacity))
        }
    }

    private func attachmentChip(_ attachment: PaneAttachmentInfo) -> some View {
        HStack(spacing: 10) {
            Image(uiImage: attachment.preview)
                .resizable()
                .scaledToFill()
                .frame(width: 44, height: 44)
                .clipShape(RoundedRectangle.continuous(DesignSystem.CornerRadius.sm))

            VStack(alignment: .leading, spacing: 2) {
                if attachment.isUploading {
                    Text("Uploading…")
                        .font(.jost(.caption, weight: .medium))
                    CircularArcSpinner(size: 12)
                } else if let error = attachment.uploadError {
                    Text(error)
                        .font(.jost(.caption))
                        .foregroundStyle(Theme.destructive)
                        .lineLimit(2)
                } else if let path = attachment.remotePath {
                    Text("Image ready")
                        .font(.jost(.caption, weight: .medium))
                    Text(URL(fileURLWithPath: path).lastPathComponent)
                        .font(.jost(.caption2))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else {
                    Text("Image attached")
                        .font(.jost(.caption, weight: .medium))
                }
            }
            Spacer(minLength: 0)
            Button {
                onRemoveAttachment?()
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.jost(18))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove Attached Image")
            .accessibilityIdentifier("pane-attachment-remove-button")
        }
        .padding(8)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pane-attachment-chip")
    }
}


private struct AgentModelConfigurationSheet: View {
    let state: AgentModelControlState
    @State private var path: [String] = []
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        NavigationStack(path: $path) {
            Form {
                Section {
                    NavigationLink(value: "Model") {
                        row("Model", value: state.pair.model?.label ?? "Unknown")
                    }
                    .accessibilityIdentifier("model-configuration-model")
                    if !state.efforts.isEmpty {
                        NavigationLink(value: "Effort") {
                            row("Effort", value: state.effortLabel)
                        }
                        .accessibilityIdentifier("model-configuration-effort")
                    }
                }
                .listRowBackground(Theme.cardBackground)
                .disabled(state.isChanging)
                if let error = state.errorMessage {
                    Text(error)
                        .foregroundStyle(Theme.destructive)
                        .listRowBackground(Theme.cardBackground)
                        .accessibilityIdentifier("model-configuration-error")
                }
            }
            .scrollContentBackground(.hidden)
            .contentMargins(.top, 8, for: .scrollContent)
            .background(Theme.background)
            .navigationTitle("Configure")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                // The app's spinner sits in front of the title while a change applies.
                ToolbarItem(placement: .principal) {
                    HStack(spacing: 8) {
                        if state.isChanging { CircularArcSpinner(size: 14) }
                        Text("Configure").font(.jost(.headline))
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isHeader)
                }
            }
            .navigationDestination(for: String.self) { destination in
                List {
                    if destination == "Model" {
                        ForEach(state.models) { model in
                            option(model.label, selected: state.selectedModelID == model.id) {
                                Task { await state.change(model: model.id) }
                            }
                        }
                    } else {
                        ForEach(state.efforts) { effort in
                            option(effort.label, selected: state.pair.effort == effort.id) {
                                Task { await state.change(effort: effort.id) }
                            }
                        }
                    }
                }
                .disabled(state.isChanging)
                .scrollContentBackground(.hidden)
                .background(Theme.background)
                .navigationTitle(destination)
                .navigationBarTitleDisplayMode(.inline)
            }
        }
        .font(.jost(.body))
        .tint(Theme.accent)
        .presentationDetents(path.isEmpty && !typeSize.isAccessibilitySize ? [.height(300)] : [.medium, .large])
        .presentationDragIndicator(.visible)
        .task { await state.refresh() }
    }

    private func row(_ title: String, value: String) -> some View {
        HStack {
            Text(title).foregroundStyle(Theme.text)
            Spacer()
            Text(value).foregroundStyle(Theme.textMuted).multilineTextAlignment(.trailing)
        }
    }
    private func option(_ label: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button {
            path = []
            action()
        } label: {
            HStack {
                Text(label).foregroundStyle(Theme.text)
                Spacer()
                if selected { Image(systemName: "checkmark").foregroundStyle(Theme.accent) }
            }
        }
        .listRowBackground(Theme.cardBackground)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
