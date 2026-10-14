import SwiftUI
import UIKit

/// Keeps terminal and connection input literal, while allowing spelling
/// assistance in the explicitly marked Compose field. Smart punctuation
/// stays disabled because prompts can contain shell commands. SwiftUI's `.autocorrectionDisabled()` is not reliably
/// applied to vertical-axis `TextField`s (backed by `UITextView`) and has no
/// smart-punctuation counterpart, so this adjusts the UIKit views directly.
@MainActor
enum PlainTextInput {
    private static var observers: [NSObjectProtocol] = []

    static func install() {
        guard observers.isEmpty else { return }
        let center = NotificationCenter.default
        observers = [
            center.addObserver(
                forName: UITextField.textDidBeginEditingNotification,
                object: nil,
                queue: .main
            ) { note in
                guard let field = note.object as? UITextField else { return }
                MainActor.assumeIsolated { apply(to: field) }
            },
            center.addObserver(
                forName: UITextView.textDidBeginEditingNotification,
                object: nil,
                queue: .main
            ) { note in
                guard let view = note.object as? UITextView else { return }
                MainActor.assumeIsolated { apply(to: view) }
            },
        ]
    }

    /// Reloads the keyboard only when a trait changed, since the keyboard
    /// was already configured by the time editing began.
    static func apply(to input: some PlainTextConfigurable) {
        let compose = (input as? UIView).map(isComposeInput) ?? false
        let correction: UITextAutocorrectionType = compose ? .yes : .no
        let spelling: UITextSpellCheckingType = compose ? .yes : .no
        var changed = false
        if input.autocorrectionType != correction { input.autocorrectionType = correction; changed = true }
        if input.spellCheckingType != spelling { input.spellCheckingType = spelling; changed = true }
        if #available(iOS 17.0, *) {
            let prediction: UITextInlinePredictionType = compose ? .yes : .no
            if input.inlinePredictionType != prediction { input.inlinePredictionType = prediction; changed = true }
        }
        if input.smartQuotesType != .no { input.smartQuotesType = .no; changed = true }
        if input.smartDashesType != .no { input.smartDashesType = .no; changed = true }
        if input.smartInsertDeleteType != .no { input.smartInsertDeleteType = .no; changed = true }
        if changed {
            input.reloadInputViews()
        }
    }

    /// SwiftUI flattens the marker and both text inputs into sibling hosts.
    /// Match the Compose field to its background's bounds rather than requiring
    /// a shared ancestor containing only one input. The hidden Live field has
    /// different bounds and must always retain literal terminal semantics.
    static func isComposeInput(_ input: UIView) -> Bool {
        guard !(input is LiveInputTextField) else { return false }
        var ancestor: UIView? = input
        while let view = ancestor {
            let markers = composeMarkers(in: view)
            if !markers.isEmpty {
                if textInputs(in: view).count == 1 { return true }
                let inputRect = input.convert(input.bounds, to: view)
                return markers.contains { marker in
                    let markerRect = marker.convert(marker.bounds, to: view)
                    guard !inputRect.isEmpty, !markerRect.isEmpty else { return false }
                    // Allow subpixel rounding between SwiftUI's sibling hosts.
                    return abs(inputRect.minX - markerRect.minX) < 1
                        && abs(inputRect.minY - markerRect.minY) < 1
                        && abs(inputRect.width - markerRect.width) < 1
                        && abs(inputRect.height - markerRect.height) < 1
                }
            }
            ancestor = view.superview
        }
        return false
    }

    static func textInputs(in view: UIView) -> [UIView] {
        if view is UITextField || view is UITextView { return [view] }
        return view.subviews.flatMap { textInputs(in: $0) }
    }

    private static func composeMarkers(in view: UIView) -> [UIView] {
        if view is ComposeTextInputMarker.MarkerView { return [view] }
        return view.subviews.flatMap { composeMarkers(in: $0) }
    }
}

/// Marks only the Compose TextField, including SwiftUI's multiline UITextView.
struct ComposeTextInputMarker: UIViewRepresentable {
    @Environment(\.colorScheme) private var colorScheme

    func makeUIView(context: Context) -> MarkerView {
        let view = MarkerView()
        view.keyboardAppearance = colorScheme == .dark ? .dark : .light
        return view
    }
    func updateUIView(_ uiView: MarkerView, context: Context) {
        uiView.keyboardAppearance = colorScheme == .dark ? .dark : .light
        uiView.configureInput()
    }

    final class MarkerView: UIView {
        var keyboardAppearance: UIKeyboardAppearance = .default

        private func configure(_ input: some PlainTextConfigurable) {
            PlainTextInput.apply(to: input)
            if input.keyboardAppearance != keyboardAppearance {
                input.keyboardAppearance = keyboardAppearance
                input.reloadInputViews()
            }
        }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            configureInput()
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            configureInput()
        }

        func configureInput() {
            // SwiftUI finishes installing the field and background together.
            DispatchQueue.main.async { [weak self] in
                var ancestor = self?.superview
                while let view = ancestor {
                    let inputs = PlainTextInput.textInputs(in: view)
                    if !inputs.isEmpty {
                        if let input = inputs.first(where: PlainTextInput.isComposeInput) {
                            if let field = input as? UITextField { self?.configure(field) }
                            if let field = input as? UITextView { self?.configure(field) }
                            return
                        }
                    }
                    ancestor = view.superview
                }
            }
        }
    }
}

/// The concrete UIKit text inputs, whose traits are settable properties
/// rather than `UITextInputTraits`' optional requirements.
@MainActor
protocol PlainTextConfigurable: UIResponder {
    var keyboardAppearance: UIKeyboardAppearance { get set }
    var autocorrectionType: UITextAutocorrectionType { get set }
    var spellCheckingType: UITextSpellCheckingType { get set }
    var inlinePredictionType: UITextInlinePredictionType { get set }
    var smartQuotesType: UITextSmartQuotesType { get set }
    var smartDashesType: UITextSmartDashesType { get set }
    var smartInsertDeleteType: UITextSmartInsertDeleteType { get set }
}

extension UITextField: PlainTextConfigurable {}
extension UITextView: PlainTextConfigurable {}
