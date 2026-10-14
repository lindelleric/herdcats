import SwiftUI
import UIKit

// MARK: - Standalone Pure Translation Helper

/// Standalone pure translation helpers for live terminal keyboard events.
///
/// Converts raw UIKit text input, deletion, Return, paste, and hardware key commands
/// into canonical `PaneLiveInputEvent` values (`.text(String)` and `.keys([String])`).
enum PaneLiveInputTranslator {
    /// Sentinel character used to prevent iOS UIKit from disabling the software Backspace key
    /// or swallowing deletion on an empty buffer.
    static let defaultSentinel = "\u{200B}"

    /// Translates committed text insertion (such as single typing or multi-character insertion)
    /// into live input events.
    ///
    /// Strips any internal sentinel characters. Returns an empty array if the resulting string is empty.
    static func translateInsertedText(_ text: String, sentinel: String = defaultSentinel) -> [PaneLiveInputEvent] {
        let cleaned = text.replacingOccurrences(of: sentinel, with: "")
        guard !cleaned.isEmpty else { return [] }
        return [.text(cleaned)]
    }

    /// Determines whether an incoming committed string should be suppressed because the exact same string
    /// was already emitted during the preceding callback in this same event turn.
    static func shouldSuppressDuplicateInsert(
        insertedText: String,
        pendingDeduplication: String?,
        sentinel: String = defaultSentinel
    ) -> Bool {
        guard let pendingDeduplication else { return false }
        let cleanedInsert = insertedText.replacingOccurrences(of: sentinel, with: "")
        let cleanedPending = pendingDeduplication.replacingOccurrences(of: sentinel, with: "")
        return !cleanedPending.isEmpty && cleanedInsert == cleanedPending
    }

    /// Translates a backspace/deletion action based on marked-text state.
    ///
    /// When `hasMarkedText` is true, the user is editing an uncommitted local IME composition
    /// (e.g. Chinese Pinyin, Japanese kana, or dead-key accents), so no remote event is emitted.
    /// When `hasMarkedText` is false, emits a remote `backspace` key event.
    static func translateBackspace(hasMarkedText: Bool) -> [PaneLiveInputEvent] {
        if hasMarkedText {
            return []
        }
        return [.keys(["backspace"])]
    }

    /// Translates Return key press into events based on whether marked text or IME confirmation was active.
    ///
    /// If marked text was active or just committed during this Return stroke, returns `[]` (does not submit).
    /// Otherwise, emits a remote terminal `enter` key event.
    static func translateReturn(hasMarkedText: Bool = false, didJustConfirmComposition: Bool = false) -> [PaneLiveInputEvent] {
        if hasMarkedText || didJustConfirmComposition {
            return []
        }
        return [.keys(["enter"])]
    }

    /// Translates pasted text into events.
    ///
    /// Strips any internal sentinel characters. Returns an empty array if the resulting string is empty.
    static func translatePaste(_ string: String, sentinel: String = defaultSentinel) -> [PaneLiveInputEvent] {
        let cleaned = string.replacingOccurrences(of: sentinel, with: "")
        guard !cleaned.isEmpty else { return [] }
        return [.text(cleaned)]
    }

    /// Translates hardware key commands (Escape, Tab, Shift+Tab, Arrows) into canonical Herdr key events.
    static func translateKeyCommand(input: String?, modifierFlags: UIKeyModifierFlags = []) -> [PaneLiveInputEvent] {
        guard let token = keyToken(forInput: input, modifierFlags: modifierFlags) else {
            return []
        }
        return [.keys([token])]
    }

    /// Maps a raw key command input and modifier flags to a Herdr key token string.
    static func keyToken(forInput input: String?, modifierFlags: UIKeyModifierFlags = []) -> String? {
        guard let input else { return nil }
        switch input {
        case UIKeyCommand.inputEscape, "esc", "escape", "\u{1B}":
            return "esc"
        case "\t", "tab":
            if modifierFlags.contains(.shift) {
                return "shift+tab"
            }
            return "tab"
        case UIKeyCommand.inputUpArrow:
            return "up"
        case UIKeyCommand.inputDownArrow:
            return "down"
        case UIKeyCommand.inputLeftArrow:
            return "left"
        case UIKeyCommand.inputRightArrow:
            return "right"
        default:
            return nil
        }
    }
}

// MARK: - UIKit Live Input Text Field

/// Specialized `UITextField` subclass that captures raw keyboard input, deletion on an empty buffer,
/// Return, paste, marked-text commit, and hardware terminal keys without accumulating remote text locally.
///
/// Handles normal keyboard input, external keyboards, IME multistage composition, and automated simulator
/// editing paths (such as Maestro / XCTest) via unified `shouldChangeCharactersIn` and `editingChanged` capture.
@MainActor
final class LiveInputTextField: UITextField, UITextFieldDelegate {
    static let sentinel = PaneLiveInputTranslator.defaultSentinel

    var onEvent: ((PaneLiveInputEvent) -> Void)?
    var onCompositionChanged: ((Bool) -> Void)?
    var onFocusChanged: ((Bool) -> Void)?

    private var isComposing = false
    private var didJustConfirmComposition = false
    private var didJustHandleBackspace = false
    /// Guards against duplicate callbacks (e.g. unmarkText followed by insertText or editingChanged with the same string).
    private var pendingDeduplicationString: String?

    /// Always reports `true` so UIKit keeps the software keyboard Delete key active
    /// even when the local buffer has no user-typed content.
    override var hasText: Bool {
        return true
    }

    var shouldBecomeFirstResponderOnMoveToWindow = false

    init() {
        super.init(frame: .zero)
        delegate = self
        setupTraits()
        setupEditingObservers()
        resetToSentinel()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil && shouldBecomeFirstResponderOnMoveToWindow {
            shouldBecomeFirstResponderOnMoveToWindow = false
            DispatchQueue.main.async { [weak self] in
                guard let self, self.window != nil else { return }
                self.becomeFirstResponder()
            }
        }
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        delegate = self
        setupTraits()
        setupEditingObservers()
        resetToSentinel()
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    private func setupTraits() {
        // Disable all autocorrect, spellcheck, smart punctuation, and autocapitalization
        // so typing reaches the remote TUI without local mutation.
        autocorrectionType = .no
        spellCheckingType = .no
        smartQuotesType = .no
        smartDashesType = .no
        smartInsertDeleteType = .no
        autocapitalizationType = .none
        if #available(iOS 17.0, *) {
            inlinePredictionType = .no
        }
        returnKeyType = .default
        enablesReturnKeyAutomatically = false
        keyboardType = .default
        tintColor = UIColor(Theme.accent)
        font = UIFont.monospacedSystemFont(ofSize: 15, weight: .regular)
        textColor = UIColor(Theme.text)
        backgroundColor = .clear

        // Accessibility
        isAccessibilityElement = true
        accessibilityLabel = "Live pane input"
        accessibilityHint = "Keys and text are sent directly to the active pane."
        accessibilityIdentifier = "pane-live-input-field"
    }

    private func setupEditingObservers() {
        addTarget(self, action: #selector(handleEditingChanged), for: .editingChanged)
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleEditingChanged),
            name: UITextField.textDidChangeNotification,
            object: self
        )
    }

    /// Resets the text field to the minimal sentinel buffer and places the cursor at the end.
    func resetToSentinel() {
        text = Self.sentinel
        selectedTextRange = textRange(from: endOfDocument, to: endOfDocument)
    }

    // MARK: - Unified Input Processing

    private func handleCommittedText(_ text: String) {
        let cleaned = text.replacingOccurrences(of: Self.sentinel, with: "")
        guard !cleaned.isEmpty else {
            resetToSentinel()
            return
        }

        // Guard against duplicate callbacks across unmarkText, insertText, and editingChanged
        if PaneLiveInputTranslator.shouldSuppressDuplicateInsert(
            insertedText: cleaned,
            pendingDeduplication: pendingDeduplicationString,
            sentinel: Self.sentinel
        ) {
            pendingDeduplicationString = nil
            didJustConfirmComposition = true
            resetToSentinel()
            onCompositionChanged?(false)
            DispatchQueue.main.async { [weak self] in
                self?.didJustConfirmComposition = false
            }
            return
        }

        let wasComposing = isComposing || (markedTextRange != nil)
        isComposing = false
        pendingDeduplicationString = cleaned
        DispatchQueue.main.async { [weak self] in
            self?.pendingDeduplicationString = nil
        }

        let events = PaneLiveInputTranslator.translateInsertedText(cleaned, sentinel: Self.sentinel)
        for event in events {
            onEvent?(event)
        }
        resetToSentinel()
        onCompositionChanged?(false)

        if wasComposing {
            didJustConfirmComposition = true
            DispatchQueue.main.async { [weak self] in
                self?.didJustConfirmComposition = false
            }
        }
    }

    private func handleBackspace() {
        let hasMarked = isComposing || (markedTextRange != nil)
        if hasMarked {
            super.deleteBackward()
            isComposing = (markedTextRange != nil)
            onCompositionChanged?(isComposing)
            return
        }

        didJustHandleBackspace = true
        DispatchQueue.main.async { [weak self] in
            self?.didJustHandleBackspace = false
        }

        let events = PaneLiveInputTranslator.translateBackspace(hasMarkedText: false)
        for event in events {
            onEvent?(event)
        }
        resetToSentinel()
        onCompositionChanged?(false)
    }

    // MARK: - Editing Changed (Automation & Fallback Capture)

    @objc private func handleEditingChanged() {
        // If actively composing marked text, keep composition local
        if markedTextRange != nil {
            isComposing = true
            onCompositionChanged?(true)
            return
        }

        let currentText = text ?? ""

        // If text is empty, the sentinel character was deleted (backspace on empty buffer)
        if currentText.isEmpty {
            if !didJustHandleBackspace {
                handleBackspace()
            }
            return
        }

        // Extract committed text beyond the sentinel (handles Maestro inputText and paste)
        let cleaned = currentText.replacingOccurrences(of: Self.sentinel, with: "")
        if !cleaned.isEmpty {
            handleCommittedText(cleaned)
        } else {
            if isComposing {
                isComposing = false
                onCompositionChanged?(false)
            }
        }
    }

    // MARK: - UIKeyInput Overrides

    override func insertText(_ text: String) {
        handleCommittedText(text)
    }

    override func deleteBackward() {
        if didJustHandleBackspace {
            didJustHandleBackspace = false
            return
        }
        handleBackspace()
    }

    // MARK: - Multistage / Marked Text (IME)

    override func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        super.setMarkedText(markedText, selectedRange: selectedRange)
        isComposing = (markedText != nil && !markedText!.isEmpty)
        didJustConfirmComposition = false
        pendingDeduplicationString = nil
        onCompositionChanged?(isComposing)
    }

    override func unmarkText() {
        let wasComposing = isComposing || (markedTextRange != nil)
        if wasComposing, let range = markedTextRange, let committedText = self.text(in: range) {
            handleCommittedText(committedText)
        }
        isComposing = false
        super.unmarkText()
        resetToSentinel()
        onCompositionChanged?(false)
    }

    // MARK: - Paste Handling

    override func paste(_ sender: Any?) {
        guard let string = UIPasteboard.general.string, !string.isEmpty else { return }
        handleCommittedText(string)
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(paste(_:)) {
            return UIPasteboard.general.hasStrings
        }
        return false
    }

    override func buildMenu(with builder: any UIMenuBuilder) {
        builder.remove(menu: .lookup)
        builder.remove(menu: .format)
        builder.remove(menu: .share)
        super.buildMenu(with: builder)
    }

    // MARK: - Hardware Key Commands

    override var keyCommands: [UIKeyCommand]? {
        let commands = [
            UIKeyCommand(
                input: UIKeyCommand.inputEscape,
                modifierFlags: [],
                action: #selector(handleKeyCommand(_:))
            ),
            UIKeyCommand(
                input: "\t",
                modifierFlags: [],
                action: #selector(handleKeyCommand(_:))
            ),
            UIKeyCommand(
                input: "\t",
                modifierFlags: .shift,
                action: #selector(handleKeyCommand(_:))
            ),
            UIKeyCommand(
                input: UIKeyCommand.inputUpArrow,
                modifierFlags: [],
                action: #selector(handleKeyCommand(_:))
            ),
            UIKeyCommand(
                input: UIKeyCommand.inputDownArrow,
                modifierFlags: [],
                action: #selector(handleKeyCommand(_:))
            ),
            UIKeyCommand(
                input: UIKeyCommand.inputLeftArrow,
                modifierFlags: [],
                action: #selector(handleKeyCommand(_:))
            ),
            UIKeyCommand(
                input: UIKeyCommand.inputRightArrow,
                modifierFlags: [],
                action: #selector(handleKeyCommand(_:))
            ),
        ]
        for command in commands {
            command.wantsPriorityOverSystemBehavior = true
        }
        return commands
    }

    @objc private func handleKeyCommand(_ command: UIKeyCommand) {
        let events = PaneLiveInputTranslator.translateKeyCommand(
            input: command.input,
            modifierFlags: command.modifierFlags
        )
        for event in events {
            onEvent?(event)
        }
        resetToSentinel()
        onCompositionChanged?(markedTextRange != nil)
    }

    // MARK: - UITextFieldDelegate (Intercept & Focus)

    func textField(
        _ textField: UITextField,
        shouldChangeCharactersIn range: NSRange,
        replacementString string: String
    ) -> Bool {
        // 1. If currently composing marked text, allow UIKit to update the local composition
        if textField.markedTextRange != nil {
            return true
        }

        // 2. Return key handling via shouldChangeCharactersIn
        if string == "\n" || string == "\r" {
            _ = textFieldShouldReturn(textField)
            return false
        }

        // 3. Deletion (Backspace)
        if string.isEmpty && range.length > 0 {
            handleBackspace()
            return false
        }

        // 4. Normal typing, paste, or automation: return true so text storage updates
        // and triggers handleEditingChanged / insertText.
        return true
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        let hasMarked = (markedTextRange != nil)
        let justConfirmed = didJustConfirmComposition
        didJustConfirmComposition = false
        pendingDeduplicationString = nil

        if hasMarked {
            unmarkText()
            return false
        }

        let events = PaneLiveInputTranslator.translateReturn(
            hasMarkedText: false,
            didJustConfirmComposition: justConfirmed
        )
        for event in events {
            onEvent?(event)
        }
        resetToSentinel()
        onCompositionChanged?(false)
        return false
    }

    func textFieldDidBeginEditing(_ textField: UITextField) {
        onFocusChanged?(true)
    }

    func textFieldDidEndEditing(_ textField: UITextField) {
        onFocusChanged?(false)
    }

    override var intrinsicContentSize: CGSize {
        return CGSize(width: UIView.noIntrinsicMetric, height: 28)
    }

    override func closestPosition(to point: CGPoint) -> UITextPosition? {
        return endOfDocument
    }

    override func selectionRects(for range: UITextRange) -> [UITextSelectionRect] {
        if markedTextRange != nil {
            return super.selectionRects(for: range)
        }
        return []
    }

    override func caretRect(for position: UITextPosition) -> CGRect {
        var rect = super.caretRect(for: position)
        rect.size.width = 2
        return rect
    }
}

// MARK: - UIViewRepresentable Bridge

@MainActor
private struct LiveInputRepresentable: UIViewRepresentable {
    var isEnabled: Bool
    var isFocused: Binding<Bool>?
    var onCompositionChanged: (Bool) -> Void
    var onEvent: (PaneLiveInputEvent) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> LiveInputTextField {
        let textField = LiveInputTextField()
        textField.keyboardAppearance = .dark
        textField.onEvent = onEvent
        textField.onCompositionChanged = onCompositionChanged
        textField.isUserInteractionEnabled = isEnabled
        textField.setContentHuggingPriority(.defaultHigh, for: .vertical)
        textField.setContentCompressionResistancePriority(.defaultHigh, for: .vertical)
        textField.onFocusChanged = { [weak textField, weak coordinator = context.coordinator] focused in
            if !focused {
                textField?.resetToSentinel()
            }
            guard let coordinator, !coordinator.isUpdatingFromSwiftUI else { return }
            guard coordinator.parent.isFocused?.wrappedValue != focused else { return }
            DispatchQueue.main.async {
                guard !coordinator.isUpdatingFromSwiftUI else { return }
                if coordinator.parent.isFocused?.wrappedValue != focused {
                    coordinator.parent.isFocused?.wrappedValue = focused
                }
            }
        }
        context.coordinator.textField = textField
        return textField
    }

    func updateUIView(_ uiView: LiveInputTextField, context: Context) {
        context.coordinator.parent = self
        context.coordinator.isUpdatingFromSwiftUI = true
        defer {
            context.coordinator.isUpdatingFromSwiftUI = false
        }

        uiView.tintColor = UIColor(Theme.accent)
        uiView.textColor = UIColor(Theme.text)
        uiView.onEvent = onEvent
        uiView.onCompositionChanged = onCompositionChanged
        uiView.isUserInteractionEnabled = isEnabled

        if let isFocused = isFocused?.wrappedValue {
            if isFocused && !uiView.isFirstResponder {
                if uiView.window != nil {
                    uiView.becomeFirstResponder()
                } else {
                    uiView.shouldBecomeFirstResponderOnMoveToWindow = true
                }
            } else if !isFocused && uiView.isFirstResponder {
                uiView.shouldBecomeFirstResponderOnMoveToWindow = false
                uiView.resignFirstResponder()
            }
        }
    }

    final class Coordinator: NSObject {
        var parent: LiveInputRepresentable
        weak var textField: LiveInputTextField?
        var isUpdatingFromSwiftUI = false

        init(parent: LiveInputRepresentable) {
            self.parent = parent
        }
    }
}

// MARK: - PaneLiveInputView (SwiftUI Component)

/// Dedicated live keyboard input view for terminal panes.
///
/// Captures committed text, Backspace (including on an empty local buffer), Return, paste,
/// marked-text composition/commit, and hardware arrows/Tab/Escape without mirroring
/// or accumulating the remote prompt locally.
struct PaneLiveInputView: View {
    var isEnabled: Bool
    var isFocused: Binding<Bool>?
    var placeholder: String
    var onEvent: (PaneLiveInputEvent) -> Void

    @State private var isComposing = false

    init(
        isEnabled: Bool = true,
        isFocused: Binding<Bool>? = nil,
        placeholder: String = "Tap to type live…",
        onEvent: @escaping (PaneLiveInputEvent) -> Void
    ) {
        self.isEnabled = isEnabled
        self.isFocused = isFocused
        self.placeholder = placeholder
        self.onEvent = onEvent
    }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.jost(14, weight: .semibold))
                .foregroundStyle(isEnabled ? Theme.accent : .secondary)
                .accessibilityHidden(true)

            ZStack(alignment: .leading) {
                if !isComposing {
                    Text(placeholder)
                        .font(.system(size: 14, weight: .regular, design: .monospaced))
                        .foregroundStyle(.tertiary)
                        .allowsHitTesting(false)
                        .padding(.leading, 2)
                }

                LiveInputRepresentable(
                    isEnabled: isEnabled,
                    isFocused: isFocused,
                    onCompositionChanged: { composing in
                        isComposing = composing
                    },
                    onEvent: onEvent
                )
                .frame(height: 28)
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 44)
        .background(
            Theme.rowBackground,
            in: RoundedRectangle(cornerRadius: DesignSystem.CornerRadius.md, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignSystem.CornerRadius.md, style: .continuous)
                .stroke(Theme.border, lineWidth: 1)
        )
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Live pane input")
        .accessibilityIdentifier("pane-live-input-container")
    }
}

// MARK: - PaneLiveKeyboardCaptureView

/// Compact, invisible live keyboard capture surface for toolbars that trigger
/// keyboard input via an action button rather than an inline text field.
struct PaneLiveKeyboardCaptureView: View {
    var isEnabled: Bool = true
    var isFocused: Binding<Bool>?
    var onCompositionChanged: ((Bool) -> Void)?
    var onEvent: (PaneLiveInputEvent) -> Void

    var body: some View {
        LiveInputRepresentable(
            isEnabled: isEnabled,
            isFocused: isFocused,
            onCompositionChanged: onCompositionChanged ?? { _ in },
            onEvent: onEvent
        )
        .frame(width: 1, height: 1)
        .opacity(0.01)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}
