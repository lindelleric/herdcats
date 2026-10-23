import Network
import UIKit
import XCTest

func openNewConnection(in app: XCUIApplication) {
    if app.textFields["192.168.1.42"].waitForExistence(timeout: 2) { return }
    if app.buttons["Continue"].exists {
        app.buttons["Continue"].tap()
    } else if app.buttons["New Connection"].exists {
        app.buttons["New Connection"].tap()
    } else if app.buttons["Connect to a Remote Machine"].exists {
        app.buttons["Connect to a Remote Machine"].tap()
    }
}

/// End-to-end tests: connect to an SSH server with a throwaway key and
/// assert that the tab-based UI renders live herdr data.
///
/// By default the local machine's SSH server is used, with the test key
/// authorized in ~/.ssh/authorized_keys (requires Remote Login enabled).
/// Override `TEST_RUNNER_HC_PORT` to point at a different sshd (e.g. a
/// throwaway one on port 2222) and `TEST_RUNNER_HC_USER` for the login
/// username:
///
///     TEST_RUNNER_HC_USER=$USER xcodebuild test -only-testing:HerdcatsUITests
final class HerdcatsUITests: XCTestCase {
    func fixtureEnvironment(_ name: String) -> String? {
        let environment = ProcessInfo.processInfo.environment
        return environment["TEST_RUNNER_HC_\(name)"] ?? environment["HC_\(name)"]
    }

    func testFirstUseIntroductionExplainsHerdrBeforeConnection() {
        let app = XCUIApplication()
        app.launchArguments = [
            "-launchPreference", "connectionSelection",
            "-hc.onboardingIntroSeen", "NO"
        ]
        app.launch()

        XCTAssertTrue(app.staticTexts["Tame your autonomous agents from your pocket."].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Continue"].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "First use introduction"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.buttons["Continue"].tap()
        XCTAssertTrue(app.textFields["192.168.1.42"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["Herdr is required."].exists)
        XCTAssertFalse(app.staticTexts["SSH worked. Herdr was not found."].exists)
    }

    func testConnectionSetupGuideExplainsThePathToReady() {
        let app = XCUIApplication()
        app.launchArguments = ["-launchPreference", "connectionSelection"]
        app.launch()

        if app.buttons["Continue"].exists { app.buttons["Continue"].tap() }
        let form = app.textFields["192.168.1.42"]
        let fromSavedMachines = !form.waitForExistence(timeout: 5)
        if fromSavedMachines {
            app.buttons["Set Up Another Remote Machine"].tap()
        } else {
            let setup = app.buttons.matching(identifier: "Connection Setup Help")
                .allElementsBoundByIndex.first(where: \.isHittable)
            XCTAssertNotNil(setup)
            setup?.tap()
        }

        XCTAssertTrue(app.staticTexts["Step 1: Install Herdr"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Step 2: Start the Default Session"].exists)
        XCTAssertTrue(app.staticTexts["Step 3: Enable SSH"].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Connection setup help"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.buttons[fromSavedMachines ? "Enter SSH Details" : "Back to Connection"].tap()
        XCTAssertTrue(form.waitForExistence(timeout: 5))
        app.buttons["Close"].tap()
        if fromSavedMachines {
            XCTAssertTrue(app.buttons["New Connection"].waitForExistence(timeout: 5))
        } else {
            XCTAssertTrue(app.buttons["Connect to a Remote Machine"].waitForExistence(timeout: 5))
        }
        app.terminate()
        app.launch()
        XCTAssertFalse(app.buttons["Continue"].exists)
        if fromSavedMachines {
            XCTAssertTrue(app.buttons["Set Up Another Remote Machine"].waitForExistence(timeout: 5))
        } else {
            XCTAssertTrue(form.waitForExistence(timeout: 5))
        }
    }

    /// A local TCP peer that accepts the socket but never completes SSH lets
    /// this test inspect the real connecting state without a live Herdr server.
    func testCatAppearsAfterTappingConnect() throws {
        let listener = try NWListener(using: .tcp, on: .any)
        let ready = expectation(description: "Local waiting peer is ready")
        var connections: [NWConnection] = []
        listener.stateUpdateHandler = { state in
            if case .ready = state { ready.fulfill() }
        }
        listener.newConnectionHandler = { connection in
            connections.append(connection)
            connection.start(queue: .main)
        }
        listener.start(queue: .main)
        defer {
            listener.cancel()
            connections.forEach { $0.cancel() }
        }
        wait(for: [ready], timeout: 5)
        let port = try XCTUnwrap(listener.port)

        let app = XCUIApplication()
        app.launchArguments = ["-launchPreference", "connectionSelection", "-appearancePreference", "dark"]
        app.launch()
        openNewConnection(in: app)
        let host = app.textFields["192.168.1.42"]
        XCTAssertTrue(host.waitForExistence(timeout: 5))
        host.tap()
        host.typeText("127.0.0.1")
        let portField = app.textFields["22"]
        portField.tap()
        portField.typeText(XCUIKeyboardKey.delete.rawValue + XCUIKeyboardKey.delete.rawValue + String(port.rawValue))
        app.textFields["whoami"].tap()
        app.textFields["whoami"].typeText("loading-test")
        app.secureTextFields["Password"].tap()
        app.secureTextFields["Password"].typeText("fixture-only")
        app.swipeUp()
        app.buttons["Connect"].tap()

        let cat = app.descendants(matching: .any)["lazy-cat-loading"].firstMatch
        XCTAssertTrue(cat.waitForExistence(timeout: 5), "Connecting should show the cat above the settings sheet")
        XCTAssertTrue(cat.isHittable, "The cat must not be covered by the connection sheet")
        XCTAssertEqual(cat.label, "Signing in over SSH…")
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Cat after tapping Connect"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.terminate()
    }

    /// Launches the app auto-connected to the configured SSH target.
    func launchConnectedApp(terminalGeometry: Bool = false) throws -> XCUIApplication {
        guard let keyPath = fixtureEnvironment("KEY_PATH"),
              FileManager.default.isReadableFile(atPath: keyPath) else {
            throw XCTSkip("Set TEST_RUNNER_HC_KEY_PATH to a local private test key")
        }
        let keyURL = URL(fileURLWithPath: keyPath)

        // The simulator reports "mobile" for NSUserName(); the real login
        // username arrives through the TEST_RUNNER_HC_USER variable.
        guard let username = fixtureEnvironment("USER"), !username.isEmpty else {
            throw XCTSkip("Set TEST_RUNNER_HC_USER to the disposable SSH server login")
        }
        let port = fixtureEnvironment("PORT") ?? "22"

        let app = XCUIApplication()
        app.launchArguments = [
            "-hc.autoconnect", "1",
            "-hc.host", "127.0.0.1",
            "-hc.port", port,
            "-hc.user", username,
            "-hc.key", keyURL.path,
            // Autoconnect otherwise opens Agents; pane helpers need Spaces.
            "-hc.tab", "spaces",
            // These terminal-input tests explicitly start with the Live toolbar.
            "-paneShowComposeByDefault", "NO"
        ]
        if terminalGeometry { app.launchArguments.append("-hc.terminal-geometry") }
        app.launch()
        return app
    }

    /// Runs only against the disposable SSH fixture in the named mode. The
    /// host-key decision still uses the app's real fingerprint approval UI.
    func launchFixture(mode: String) throws -> XCUIApplication {
        guard fixtureEnvironment("FIXTURE_MODE") == mode else {
            throw XCTSkip("Set TEST_RUNNER_HC_FIXTURE_MODE=\(mode) with the disposable SSH fixture")
        }
        let app = try launchConnectedApp()
        let trust = app.buttons["Trust and Connect"]
        if trust.waitForExistence(timeout: 10) {
            let expectedFingerprint = try XCTUnwrap(
                fixtureEnvironment("HOST_KEY_FINGERPRINT")
            )
            XCTAssertTrue(app.staticTexts[expectedFingerprint].exists)
            trust.tap()
        }
        return app
    }

    func testFirstUseShowsMissingHerdrOnlyAfterSSHSignIn() throws {
        guard fixtureEnvironment("FIXTURE_MODE") == "missing",
              let port = fixtureEnvironment("PORT"),
              let fingerprint = fixtureEnvironment("HOST_KEY_FINGERPRINT") else {
            throw XCTSkip("Set TEST_RUNNER_HC_FIXTURE_MODE=missing, PORT, and HOST_KEY_FINGERPRINT")
        }
        let app = XCUIApplication()
        app.launchArguments = [
            "-launchPreference", "connectionSelection",
            "-hc.onboardingIntroSeen", "NO"
        ]
        app.launch()
        defer { app.terminate() }

        XCTAssertTrue(app.buttons["Continue"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["SSH worked. Herdr was not found."].exists)
        app.buttons["Continue"].tap()
        let host = app.textFields["192.168.1.42"]
        XCTAssertTrue(host.waitForExistence(timeout: 5))
        host.tap()
        host.typeText("127.0.0.1")
        let portField = app.textFields["22"]
        portField.tap()
        portField.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 2) + port)
        let username = app.textFields["whoami"]
        username.tap()
        username.typeText("fixture")
        let password = app.secureTextFields["Password"]
        password.tap()
        password.typeText("fixture-only-password-disabled-for-ssh")
        XCTAssertFalse(app.staticTexts["SSH worked. Herdr was not found."].exists)
        app.buttons["Connect"].tap()

        let trust = app.buttons["Trust and Connect"]
        if trust.waitForExistence(timeout: 5) {
            XCTAssertTrue(app.staticTexts[fingerprint].exists)
            trust.tap()
        }
        XCTAssertTrue(app.staticTexts["SSH worked. Herdr was not found."].waitForExistence(timeout: 30))
        XCTAssertEqual(host.value as? String, "127.0.0.1")
        XCTAssertEqual(username.value as? String, "fixture")
        let setup = try XCTUnwrap(app.buttons.matching(identifier: "See Setup Steps")
            .allElementsBoundByIndex.first(where: \.isHittable))
        setup.tap()
        XCTAssertTrue(app.staticTexts["Step 1: Install Herdr"].waitForExistence(timeout: 5))
        app.buttons["Back to Connection"].tap()
        XCTAssertTrue(host.waitForExistence(timeout: 5))
    }

    func testAutoConnectSessionFailureReturnsToRecovery() throws {
        let app = try launchFixture(mode: "session")
        defer { app.terminate() }
        let recovery = app.staticTexts["Herdr was found. Check its session."]
        XCTAssertTrue(recovery.waitForExistence(timeout: 30))
        XCTAssertFalse(app.staticTexts["The connection was lost during handshake."].exists)
        sleep(7) // Longer than the prior automatic SSH retry interval.
        XCTAssertTrue(recovery.exists, "Readiness failure must stay on the connection recovery screen")
    }

    #if DEBUG
    func testDebugReplayReturnsToConnectedSettings() throws {
        let app = try launchFixture(mode: "ready")
        defer { app.terminate() }
        let settingsTab = app.tabBars.buttons["Settings"]
        XCTAssertTrue(settingsTab.waitForExistence(timeout: 30))
        settingsTab.tap()
        let replay = app.buttons["Replay Onboarding"]
        XCTAssertTrue(replay.waitForExistence(timeout: 5))
        let settingsScreenshot = XCTAttachment(screenshot: app.screenshot())
        settingsScreenshot.name = "Connected debug settings"
        settingsScreenshot.lifetime = .keepAlways
        add(settingsScreenshot)
        replay.tap()
        XCTAssertTrue(app.staticTexts["Tame your autonomous agents from your pocket."].waitForExistence(timeout: 5))
        let replayScreenshot = XCTAttachment(screenshot: app.screenshot())
        replayScreenshot.name = "Debug onboarding replay"
        replayScreenshot.lifetime = .keepAlways
        add(replayScreenshot)
        app.buttons["Continue"].tap()
        XCTAssertTrue(app.textFields["192.168.1.42"].waitForExistence(timeout: 5))
        app.buttons["Close"].tap()
        app.buttons["exit-onboarding-replay"].tap()
        XCTAssertTrue(replay.waitForExistence(timeout: 5))
        XCTAssertTrue(settingsTab.isSelected)
    }
    #else
    func testReleaseSettingsDoNotOfferOnboardingReplay() throws {
        let app = try launchFixture(mode: "ready")
        defer { app.terminate() }
        let settingsTab = app.tabBars.buttons["Settings"]
        XCTAssertTrue(settingsTab.waitForExistence(timeout: 30))
        settingsTab.tap()
        XCTAssertFalse(app.buttons["Replay Onboarding"].exists)
        sleep(1) // Let the tab transition finish before capturing the visible Settings screen.
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Release settings without replay"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }
    #endif

    /// Read-only smoke test; opt in with TEST_RUNNER_HC_MACHINE_LABEL.
}

@MainActor
final class PaletteUITests: XCTestCase {
    func testPaletteChangesKeepNavigationAndSurviveRelaunch() {
        verifyPaletteSelections(dark: "mocha", light: "latte", name: "Catppuccin")
    }

    func testSolarizedSelectionsKeepNavigationAndSurviveRelaunch() {
        verifyPaletteSelections(dark: "solarizedDark", light: "solarizedLight", name: "Solarized")
    }

    private func verifyPaletteSelections(dark: String, light: String, name: String) {
        let app = XCUIApplication()
        app.launchArguments = ["-quotaNotSetupPreview", "-appearancePreference", "dark"]
        app.launch()
        let settings = app.tabBars.buttons["Settings"]
        XCTAssertTrue(settings.waitForExistence(timeout: 10))
        settings.tap()
        let picker = app.buttons["dark-theme-picker"]
        if !picker.isHittable { app.swipeUp() }
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        picker.tap()
        let mocha = app.buttons["palette-\(dark)"]
        XCTAssertTrue(mocha.waitForExistence(timeout: 5))
        app.buttons["palette-moonlit"].tap()
        if !mocha.isHittable { app.swipeUp() }
        mocha.tap()
        XCTAssertEqual(mocha.value as? String, "Selected")
        XCTAssertTrue(app.navigationBars["Dark Theme"].exists, "Changing palette must retain the screen")
        let preview = XCTAttachment(screenshot: app.screenshot())
        preview.name = "\(name) dark palette chooser"
        preview.lifetime = .keepAlways
        add(preview)
        app.terminate()
        app.launch()
        app.tabBars.buttons["Settings"].tap()
        if !app.buttons["dark-theme-picker"].isHittable { app.swipeUp() }
        app.buttons["dark-theme-picker"].tap()
        if !app.buttons["palette-\(dark)"].isHittable { app.swipeUp() }
        XCTAssertEqual(app.buttons["palette-\(dark)"].value as? String, "Selected")
        if !app.buttons["palette-moonlit"].isHittable { app.swipeDown() }
        app.buttons["palette-moonlit"].tap()
        app.navigationBars["Dark Theme"].buttons.element(boundBy: 0).tap()
        app.buttons["light-theme-picker"].tap()
        app.buttons["palette-\(light)"].tap()
        XCTAssertEqual(app.buttons["palette-\(light)"].value as? String, "Selected")
        app.terminate()
        app.launchArguments = ["-quotaNotSetupPreview", "-appearancePreference", "light"]
        app.launch()
        app.tabBars.buttons["Settings"].tap()
        app.buttons["light-theme-picker"].tap()
        XCTAssertEqual(app.buttons["palette-\(light)"].value as? String, "Selected")
        let lightPreview = XCTAttachment(screenshot: app.screenshot())
        lightPreview.name = "\(name) light palette chooser"
        lightPreview.lifetime = .keepAlways
        add(lightPreview)
        app.buttons["palette-moonlit"].tap()
    }
}
