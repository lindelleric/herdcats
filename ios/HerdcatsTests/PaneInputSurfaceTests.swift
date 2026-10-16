import Testing
import SwiftUI
import UIKit
@testable import Herdcats

/// Transition coverage for the pane input surface state machine that replaces
/// the Compose/Live segmented control: keyboard opens Compose directly, Voice
/// opens recording, and tapping output collapses Compose back to Live.
@Suite("Pane input surface state machine")
@MainActor
struct PaneInputSurfaceTests {
    @Test func liveToolbarIsTheDefaultSurface() {
        let model = PaneInputSurfaceModel()
        #expect(model.surface == .liveToolbar)
        #expect(model.isLiveKeyboardSurface)
        #expect(!model.isComposeSurface)
        #expect(!model.isDrainingDictation)
    }

    @Test func visibleComposeDefaultSurvivesSendAndReset() {
        let model = PaneInputSurfaceModel()
        model.setComposeVisibleByDefault(PaneComposePreference.default)
        #expect(model.isComposeSurface)
        model.noteSendSucceeded()
        #expect(model.isComposeSurface)
        // Tapping output can still explicitly enter the Live input surface.
        model.collapseToLiveToolbar()
        #expect(model.isLiveKeyboardSurface)
        model.resetToDefaultSurface()
        #expect(model.isComposeSurface)
    }

    @Test func changingDefaultDoesNotInterruptDictationOrDiscardCompose() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()
        model.requestFinishDictation()
        model.setComposeVisibleByDefault(true)
        #expect(model.surface == .voiceRecording)
        #expect(model.isDrainingDictation)
        model.noteDictationIdle()
        model.setComposeVisibleByDefault(false)
        #expect(model.isComposeSurface)
        model.noteSendSucceeded()
        #expect(model.isLiveKeyboardSurface)
    }

    @Test func composeTraitsDoNotApplyToNearbyTerminalInput() {
        let container = UIView()
        let composeContainer = UIView()
        let compose = UITextView()
        composeContainer.addSubview(compose)
        composeContainer.addSubview(ComposeTextInputMarker.MarkerView())
        container.addSubview(composeContainer)
        let live = LiveInputTextField()
        container.addSubview(live)
        PlainTextInput.apply(to: compose)
        PlainTextInput.apply(to: live)
        #expect(compose.autocorrectionType == .yes)
        #expect(compose.spellCheckingType == .yes)
        #expect(compose.inlinePredictionType == .yes)
        #expect(compose.smartQuotesType == .no)
        #expect(live.autocorrectionType == .no)
        #expect(live.spellCheckingType == .no)
        #expect(live.inlinePredictionType == .no)
    }

    @Test func swiftUIMultilineComposeReceivesSpellingAssistance() async throws {
        let host = UIHostingController(rootView:
            TextField("Message", text: .constant("hello"), axis: .vertical)
                .autocorrectionDisabled(false)
                .background(ComposeTextInputMarker())
        )
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        window.rootViewController = host
        window.isHidden = false
        defer { window.isHidden = true }
        host.view.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(100))
        let input = try #require(PlainTextInput.textInputs(in: host.view).first as? UITextView)
        // Reproduce the app-wide begin-editing observer after SwiftUI layout.
        PlainTextInput.apply(to: input)
        #expect(input.autocorrectionType == .yes)
        #expect(input.spellCheckingType == .yes)
        #expect(input.inlinePredictionType == .yes)
    }

    @Test func composeBarKeepsSpellingAssistanceWhileEditing() async throws {
        PlainTextInput.install()
        let host = UIHostingController(rootView: ComposeEditingHarness())
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        host.view.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(100))
        let inputs = PlainTextInput.textInputs(in: host.view)
        let compose = try #require(inputs.first { !($0 is LiveInputTextField) } as? UITextView)
        let live = try #require(inputs.first { $0 is LiveInputTextField } as? UITextField)
        #expect(compose.autocorrectionType == .yes, "Before focus")
        #expect(compose.spellCheckingType == .yes, "Before focus")
        #expect(compose.becomeFirstResponder())
        try await Task.sleep(for: .milliseconds(100))
        #expect(compose.autocorrectionType == .yes, "After focus")
        #expect(compose.spellCheckingType == .yes, "After focus")
        for letter in "toda " {
            compose.insertText(String(letter))
            try await Task.sleep(for: .milliseconds(50))
            #expect(compose.autocorrectionType == .yes, "After typing \(letter)")
            #expect(compose.spellCheckingType == .yes, "After typing \(letter)")
        }
        #expect(live.autocorrectionType == .no)
        #expect(live.spellCheckingType == .no)
    }

    @Test func keyboardOpensComposeDirectly() {
        let model = PaneInputSurfaceModel()
        #expect(model.requestComposeDraft())
        #expect(model.surface == .composeDraft)
        #expect(!model.requestComposeDraft())
    }

    @Test func keyboardCannotInterruptRecording() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()
        #expect(!model.requestComposeDraft())
        #expect(model.surface == .voiceRecording)
    }

    @Test func photoRestoresComposeAfterPickerResignsFocus() {
        let model = PaneInputSurfaceModel()
        model.requestComposeDraft()
        model.collapseToLiveToolbar()

        model.notePhotoSelected()
        #expect(model.surface == .composeDraft)

        model.notePhotoSelected()
        #expect(model.surface == .composeDraft)
    }

    @Test func photoSelectionCannotInterruptRecording() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()

        model.notePhotoSelected()
        #expect(model.surface == .voiceRecording)
    }

    @Test func voiceOpensRecordingFromToolbarAndComposeBubble() {
        let model = PaneInputSurfaceModel()

        model.requestVoiceRecording()
        #expect(model.surface == .voiceRecording)

        // Dictation finishes → Compose bubble; Voice re-enters from there.
        model.noteDictationIdle()
        #expect(model.surface == .composeDraft)

        model.requestVoiceRecording()
        #expect(model.surface == .voiceRecording)
    }

    @Test func voiceWhileRecordingIsRejected() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()

        model.requestVoiceRecording()
        #expect(model.surface == .voiceRecording)
        #expect(!model.isDrainingDictation)
    }

    @Test func checkmarkKeepsRecordingUntilDictationDrains() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()

        // `PaneDictation.finish()` returns immediately while `isBusy` stays
        // true up to ~15s; the recording surface must survive that drain so a
        // partial transcript is never shown or sent.
        model.requestFinishDictation()
        #expect(model.isDrainingDictation)
        #expect(model.surface == .voiceRecording)

        model.noteDictationIdle()
        #expect(model.surface == .composeDraft)
        #expect(!model.isDrainingDictation)
    }

    @Test func repeatedCheckmarkTapsDuringDrainAreNoOps() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()
        model.requestFinishDictation()

        model.requestFinishDictation()
        #expect(model.isDrainingDictation)
        #expect(model.surface == .voiceRecording)
    }

    @Test func dictationIdleWithoutCheckmarkStillOpensCompose() {
        // Interruptions (audio-session interruption, backgrounding) cancel
        // dictation without the checkmark; the captured draft must stay
        // visible and editable in the Compose bubble rather than vanishing
        // back to the toolbar.
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()

        model.noteDictationIdle()
        #expect(model.surface == .composeDraft)
        #expect(!model.isDrainingDictation)
    }

    @Test func dictationIdleIsIgnoredOutsideRecording() {
        let model = PaneInputSurfaceModel()

        model.noteDictationIdle()
        #expect(model.surface == .liveToolbar)
    }

    @Test func sendSuccessReturnsToLiveToolbar() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()
        model.noteDictationIdle()
        #expect(model.surface == .composeDraft)

        model.noteSendSucceeded()
        #expect(model.surface == .liveToolbar)
    }

    @Test func sendSuccessIsIgnoredOutsideCompose() {
        let model = PaneInputSurfaceModel()

        model.noteSendSucceeded()
        #expect(model.surface == .liveToolbar)

        model.requestVoiceRecording()
        model.noteSendSucceeded()
        #expect(model.surface == .voiceRecording)
    }

    @Test func collapseReturnsToLiveToolbarWithoutSending() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()
        model.noteDictationIdle()

        model.collapseToLiveToolbar()
        #expect(model.surface == .liveToolbar)
    }

    @Test func collapseIsIgnoredOutsideCompose() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()

        model.collapseToLiveToolbar()
        #expect(model.surface == .voiceRecording)
    }

    @Test func paneResetClearsSurfaceAndDrain() {
        let model = PaneInputSurfaceModel()
        model.requestVoiceRecording()
        model.requestFinishDictation()

        model.resetToDefaultSurface()
        #expect(model.surface == .liveToolbar)
        #expect(!model.isDrainingDictation)
    }
}


private struct ComposeEditingHarness: View {
    @State private var draft = ""
    @FocusState private var focused: Bool

    var body: some View {
        PaneVoiceComposeBar(
            mode: .constant(.compose),
            draft: $draft,
            isLiveKeyboardFocused: .constant(false),
            isComposeFieldFocused: $focused,
            armedModifiers: .constant([]),
            onLiveKey: { _ in },
            onLiveEvent: { _ in },
            onStartVoice: {},
            onFinishVoice: {},
            onOpenCompose: {},
            isPhotoPickerPresented: .constant(false),
            onSend: {}
        )
    }
}

@Suite("Timeline image originals")
@MainActor struct TimelineImageOriginalTests {
    @Test func thumbnailIsBoundedAndOriginalBytesRemainExact() throws {
        let format = UIGraphicsImageRendererFormat();format.scale = 1
        let image = UIGraphicsImageRenderer(size:CGSize(width:2048,height:64),format:format).image { context in
            UIColor.red.setFill();context.fill(CGRect(x:0,y:0,width:2048,height:64))
        }
        let original = try #require(image.pngData())
        let block = try JSONSerialization.data(withJSONObject:["type":"image","source":["type":"base64","media_type":"image/png","data":original.base64EncodedString()]])
        guard case .image(let preview,let retained,_) = try TimelineImagePayload.decode(block) else { Issue.record("Missing image"); return }
        #expect(retained == original)
        #expect(try #require(UIImage(data:preview)).size.width <= 1024)
    }
    @Test func externalImageLinksRequireUserActionAndUnsafeSchemesFail() throws {
        let valid = try JSONSerialization.data(withJSONObject:["source":["type":"url","url":"https://example.com/image.png"]])
        guard case .link(let url) = try TimelineImagePayload.decode(valid) else { Issue.record("Expected link"); return }
        #expect(url.scheme == "https")
        let invalid = try JSONSerialization.data(withJSONObject:["source":["type":"url","url":"file:///private/data"]])
        #expect(throws:AgentBridgeError.self) { try TimelineImagePayload.decode(invalid) }
    }
}
