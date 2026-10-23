import Network
import UIKit
import XCTest

extension HerdcatsUITests {
    func testTerminalOutputOffersNativeTextSelection() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "pane",
                               "-paneShowComposeByDefault", "NO"]
        app.launch()

        let terminalButton = app.buttons["timeline-open-terminal"]
        if terminalButton.waitForExistence(timeout: 3) { terminalButton.tap() }
        let output = app.textViews["pane-output-text"]
        XCTAssertTrue(output.waitForExistence(timeout: 5))
        // Long-press “Claude” in the first fixture line. This also exercises
        // selection alongside the terminal's tap-to-focus and drag gestures.
        output.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: 55, dy: 8)).press(forDuration: 1)
        XCTAssertTrue(app.buttons["Copy"].waitForExistence(timeout: 3)
                      || app.menuItems["Copy"].exists,
                      "Long-pressing terminal text should offer Copy")
        XCTAssertFalse(app.keyboards.firstMatch.exists,
                       "Selecting read-only terminal output should not open a keyboard")
    }

    func testDefaultComposeOpensKeyboardWithoutTappingField() {
        let app = XCUIApplication()
        app.launchArguments = [
            "-hc.screenshots", "-hc.screen", "pane",
            "-paneShowComposeByDefault", "YES"
        ]
        app.launch()
        XCTAssertTrue(composeField(app).waitForExistence(timeout: 5))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5),
                      "Default Compose should open the keyboard automatically")
        app.terminate()
    }

    func testPaneDetailShowsOutputAndInput() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")
        XCTAssertTrue(
            liveKeyboardButton(app).exists,
            "Default Live toolbar missing (pane-live-keyboard-button)"
        )
        XCTAssertTrue(
            app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5),
            "Pane output scroll view missing"
        )
    }

    /// Voice → Compose opens an editable draft surface without sending or
    /// typing. Safe on a real fixture: no Live keys, no Send. Skips when
    /// Speech/mic cannot reach Compose.
    func testPaneVoiceOpensEditableComposeWithoutSending() throws {
        guard ProcessInfo.processInfo.environment["HC_VOICE_SMOKE"] == "1" else {
            throw XCTSkip("Set HC_VOICE_SMOKE=1 on a simulator or device with working microphone input")
        }
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        guard openComposeViaVoice(app) else {
            throw XCTSkip(
                "Voice→Compose unavailable (permission or Speech runtime) — skipping editable Compose smoke"
            )
        }

        XCTAssertTrue(
            app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5),
            "Terminal output disappeared after opening Compose"
        )

        let field = composeField(app)
        XCTAssertTrue(field.waitForExistence(timeout: 5), "Compose field missing after Voice")
        XCTAssertTrue(field.isHittable, "Compose field is not hittable")
        XCTAssertTrue(field.isEnabled, "Compose field is not editable")

        // Photo attach and Voice share the Compose card with the message field.
        // Do not assert Send absent — ambient dictation may leave text.
        XCTAssertTrue(
            app.buttons["pane-compose-attach-button"].waitForExistence(timeout: 5),
            "Compose photo attach button missing"
        )
        XCTAssertTrue(
            app.buttons["pane-compose-voice-button"].waitForExistence(timeout: 5),
            "Compose Voice button missing"
        )
    }

    /// Types into Compose and asserts the text round-trips
    /// through the pane. Only runs against the mock-herdr SSH fixture so it
    /// can never type into a real terminal session.
    func testPaneSendTypesIntoPane() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        guard isMockHerdrFixtureVisible(app) else {
            throw XCTSkip("Not running against the mock herdr fixture — skipping send")
        }

        liveKeyboardButton(app).tap()

        let input = composeField(app)
        clearComposeField(input)
        input.typeText("hello from herdrcat")
        let send = app.buttons["pane-compose-send-button"]
        XCTAssertTrue(send.waitForExistence(timeout: 5), "Compose Send missing after typing")
        send.tap()

        let echoed = app.textViews.matching(
            NSPredicate(format: "value CONTAINS %@", "SENT: hello from herdrcat")
        ).firstMatch
        XCTAssertTrue(echoed.waitForExistence(timeout: 15), "Sent text did not round-trip through the pane")
    }

    /// The pane view opens pinned to the newest output. Mock-gated: needs the
    /// fixture's long, known-length output to reason about visibility.
    func testPaneOpensPinnedToBottom() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        guard isMockHerdrFixtureVisible(app) else {
            throw XCTSkip("Not running against the mock herdr fixture — skipping scroll check")
        }

        // After the scroll-in animation settles, the newest line must be on
        // screen without any manual scrolling.
        let output = app.textViews["pane-output-text"]
        XCTAssertTrue(output.waitForExistence(timeout: 10), "Terminal output missing")
        XCTAssertTrue((output.value as? String)?.contains("mock terminal bottom line") == true,
                      "Mock bottom marker missing")
        Thread.sleep(forTimeInterval: 1)
        let viewport = app.scrollViews["pane-output-scroll-view"].frame
        // The native output is one element; its lower edge is the newest line.
        XCTAssertGreaterThan(output.frame.maxY, viewport.minY)
        XCTAssertLessThanOrEqual(output.frame.maxY, viewport.maxY + 1,
                                 "Pane did not open scrolled to the bottom")
    }

    /// Tapping output for Live input and dragging it interactively puts the
    /// keyboard away. Focuses without typing, so it is safe against real data.
    func testPaneLiveKeyboardOutputDragDismissesKeyboard() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        let output = app.scrollViews["pane-output-scroll-view"]
        XCTAssertTrue(output.waitForExistence(timeout: 5), "Pane output scroll view missing")
        output.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "Keyboard did not appear")

        let start = output.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 180)))

        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: app.keyboards.firstMatch)
        waitForExpectations(timeout: 5)
    }

    /// Tapping terminal output opens the Live keyboard without sending input.
    func testPaneOutputTapOpensLiveKeyboard() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        let output = app.scrollViews["pane-output-scroll-view"]
        XCTAssertTrue(output.waitForExistence(timeout: 5), "Pane output scroll view missing")
        XCTAssertFalse(app.keyboards.firstMatch.exists, "Keyboard opened before tapping output")
        output.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "Output tap did not open Live keyboard")
    }

    /// The keyboard icon opens Compose directly, without microphone setup or
    /// sending anything to the remote pane.
    func testPaneKeyboardIconOpensCompose() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        let keyboardButton = liveKeyboardButton(app)
        XCTAssertEqual(keyboardButton.label, "Compose message")
        keyboardButton.tap()

        let field = composeField(app)
        XCTAssertTrue(field.waitForExistence(timeout: 5), "Keyboard icon did not open Compose")
        XCTAssertTrue(field.isHittable, "Compose field is not editable")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "Compose keyboard did not appear")
    }

    /// Switches between the two keyboard surfaces without typing or sending.
    func testPaneOutputSwitchesComposeToLive() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        liveKeyboardButton(app).tap()
        XCTAssertTrue(composeField(app).waitForExistence(timeout: 5), "Compose did not open")

        let output = app.scrollViews["pane-output-scroll-view"]
        output.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(liveKeyboardButton(app).waitForExistence(timeout: 5), "Output tap did not return to Live")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "Live keyboard did not open")

        liveKeyboardButton(app).tap()
        XCTAssertTrue(composeField(app).waitForExistence(timeout: 5), "Compose did not reopen")
    }

    /// A Compose draft remains when the user taps output to type Live, then
    /// returns to Compose. Mock-gated because it stages a local test draft.
    func testPaneOutputSwitchesComposeToLiveAndKeepsDraft() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")
        guard isMockHerdrFixtureVisible(app) else {
            throw XCTSkip("Not running against the mock herdr fixture — skipping draft switch")
        }

        liveKeyboardButton(app).tap()
        let field = composeField(app)
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        clearComposeField(field)
        field.typeText("hc-compose-live-switch")

        let output = app.scrollViews["pane-output-scroll-view"]
        output.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(liveKeyboardButton(app).waitForExistence(timeout: 5), "Output tap did not switch to Live")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), "Live keyboard did not open")

        liveKeyboardButton(app).tap()
        let restored = composeField(app)
        XCTAssertTrue(restored.waitForExistence(timeout: 5), "Compose did not reopen")
        XCTAssertTrue(composeFieldValue(restored).contains("hc-compose-live-switch"), "Compose draft was lost")
        clearComposeField(restored)
    }

    /// The developer-only navigation control reveals aggregate diagnostics
    /// without sending any input to the pane.
    func testPaneDiagnosticsButtonTogglesBar() throws {
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        let menu = app.buttons["pane-options-menu"]
        XCTAssertTrue(menu.waitForExistence(timeout: 5), "Pane options menu missing")
        menu.tap()

        let toggle = app.buttons["pane-diagnostics-button"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 5), "Debug diagnostics button missing")
        let summary = app.staticTexts["pane-live-metrics-summary"]
        XCTAssertFalse(summary.exists, "Diagnostics bar should start hidden")
        toggle.tap()
        XCTAssertTrue(summary.waitForExistence(timeout: 5), "Diagnostics bar did not open")

        menu.tap()
        XCTAssertTrue(toggle.waitForExistence(timeout: 5), "Debug diagnostics button missing")
        toggle.tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: summary)
        waitForExpectations(timeout: 5)
    }

    /// A Compose draft survives kill + relaunch. Mock-gated like send. Never
    /// sends; cleanup clears the whole field.
    func testPaneDraftSurvivesAppRelaunch() throws {
        let marker = "hc-draft-persist-marker"
        let app = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(app), "Could not open a pane detail view")

        guard isMockHerdrFixtureVisible(app) else {
            throw XCTSkip("Not running against the mock herdr fixture — skipping draft persistence")
        }

        liveKeyboardButton(app).tap()

        let input = composeField(app)
        clearComposeField(input)
        input.typeText(marker)
        XCTAssertTrue(
            composeFieldValue(input).contains(marker),
            "Failed to stage draft marker before relaunch"
        )

        // Kill and relaunch with the same autoconnect fixture credentials.
        app.terminate()
        let relaunched = try launchConnectedApp()
        XCTAssertTrue(openFirstPane(relaunched), "Could not reopen the pane after relaunch")

        liveKeyboardButton(relaunched).tap()

        let input2 = composeField(relaunched)
        XCTAssertTrue(
            composeFieldValue(input2).contains(marker),
            "Pane draft marker did not survive app kill + relaunch"
        )

        // Leave the shared draft clean for tests that run after this one.
        clearComposeField(input2)
        XCTAssertTrue(composeFieldIsEmpty(input2), "Draft cleanup failed — residual text remains")
    }
}
final class ConnectionFormUITests: XCTestCase {
    func testRecentPrivateKeyConnectionLoadsSettings() {
        let app = XCUIApplication()
        app.launchArguments = [
            "-hc.host", "example.invalid", "-hc.username", "test-user",
            "-hc.authMode", "privateKey", "-hc.remember", "NO"
        ]
        app.launch()
        let tile = app.buttons["test-user at example.invalid, port 22"]
        XCTAssertTrue(tile.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Continue"].exists)
        tile.tap()
        let key = app.textViews.firstMatch
        XCTAssertTrue(key.waitForExistence(timeout: 5))
        XCTAssertEqual(app.textFields["192.168.1.42"].value as? String, "example.invalid")
        XCTAssertEqual(app.textFields["whoami"].value as? String, "test-user")
    }

    func testConnectEnablesWhenDrawerFieldsAreComplete() {
        let app = XCUIApplication()
        app.launch()
        openNewConnection(in: app)
        let host = app.textFields["192.168.1.42"]
        XCTAssertTrue(host.waitForExistence(timeout: 5))
        host.tap()
        host.typeText("example.invalid")
        let username = app.textFields["whoami"]
        username.tap()
        username.typeText("test-user")
        let password = app.secureTextFields["Password"]
        password.tap()
        password.typeText("test-only-password")
        let connect = app.buttons["Connect"]
        XCTAssertTrue(connect.waitForExistence(timeout: 5))
        let enabled = NSPredicate(format: "enabled == true")
        expectation(for: enabled, evaluatedWith: connect)
        waitForExpectations(timeout: 5)
    }
}
