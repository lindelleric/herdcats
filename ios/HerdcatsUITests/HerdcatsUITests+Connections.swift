import Network
import UIKit
import XCTest

extension HerdcatsUITests {
    func testMissingQuotaProviderRetryButtonRerunsFetch() {
        let app = XCUIApplication()
        app.launchArguments = ["-quotaRetryUITest"]
        app.launch()
        defer { app.terminate() }

        let retry = app.buttons["quota-retry-claude"]
        XCTAssertTrue(retry.waitForExistence(timeout: 5))
        retry.tap()
        XCTAssertTrue(app.staticTexts["Requested claude"].waitForExistence(timeout: 5))
        XCTAssertFalse(retry.exists)
        let healthy = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS %@", "9 percent left")).firstMatch
        XCTAssertTrue(healthy.waitForExistence(timeout: 5))
    }

    func testProviderErrorShowsFriendlyCardAndRetryRestoresQuota() {
        let app = XCUIApplication()
        app.launchArguments = ["-quotaRetryErrorUITest"]
        app.launch()
        defer { app.terminate() }

        let unavailable = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label == %@", "Claude: quota unavailable")).firstMatch
        XCTAssertTrue(unavailable.waitForExistence(timeout: 5))
        XCTAssertFalse(
            app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "keychain_access"))
                .firstMatch.exists
        )
        let codex = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS %@", "Codex: 42 percent left")).firstMatch
        XCTAssertTrue(codex.exists)

        let retry = app.buttons["quota-retry-claude"]
        XCTAssertTrue(retry.exists)
        retry.tap()
        XCTAssertTrue(app.staticTexts["Requested claude"].waitForExistence(timeout: 5))
        XCTAssertFalse(unavailable.exists)
        XCTAssertTrue(codex.exists)
    }

    func testWorkspaceReadinessTimeoutKeepsSpecificError() throws {
        let app = try launchFixture(mode: "timeout")
        defer { app.terminate() }
        let timeout = app.staticTexts.matching(NSPredicate(
            format: "label CONTAINS %@", "Timed out waiting for herdr command"
        )).firstMatch
        XCTAssertTrue(timeout.waitForExistence(timeout: 35))
        XCTAssertFalse(app.staticTexts["The connection was lost during handshake."].exists)
    }

    func testSavedMachineSwitching() throws {
        guard let label = fixtureEnvironment("MACHINE_LABEL") else {
            throw XCTSkip("Set TEST_RUNNER_HC_MACHINE_LABEL to an enabled saved machine")
        }
        let app = try launchConnectedApp()
        defer { app.terminate() }
        let picker = app.buttons["machine-picker"]
        guard picker.waitForExistence(timeout: 30) else {
            XCTFail("The configured SSH fixture did not reach the connected screen")
            return
        }
        let gatewayLabel = String(picker.label.dropFirst("Machine: ".count))
        picker.tap()
        let machine = app.buttons[label].firstMatch
        if !machine.waitForExistence(timeout: 5) {
            app.buttons["Refresh Machines"].tap()
            sleep(2)
            picker.tap()
        }
        XCTAssertTrue(machine.waitForExistence(timeout: 15))
        machine.tap()
        XCTAssertTrue(waitForMachine(picker, label: label))
        let card = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "tabs")).firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 30), "Saved machine workspaces did not load")
        app.tabBars.buttons["Agents"].tap()
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        XCTAssertEqual(picker.label, "Machine: " + label)
        picker.tap()
        app.buttons[gatewayLabel].firstMatch.tap()
        XCTAssertTrue(waitForMachine(picker, label: gatewayLabel))
        XCTAssertTrue(app.tabBars.buttons["Agents"].isSelected, "Switching machines should retain the selected tab")
    }

    func waitForMachine(_ picker: XCUIElement, label: String) -> Bool {
        let predicate = NSPredicate(format: "label == %@", "Machine: " + label)
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: picker)
        return XCTWaiter.wait(for: [expectation], timeout: 10) == .completed
    }

    func testConnectAndShowSpaces() throws {
        let app = try launchConnectedApp()

        // The spaces screen appears once connected.
        let navTitle = app.navigationBars.staticTexts["Spaces"].exists
            ? app.navigationBars.staticTexts["Spaces"]
            : app.navigationBars.staticTexts["Herdcats"]
        XCTAssertTrue(navTitle.waitForExistence(timeout: 25), "Spaces screen did not appear")

        // Some space card rendered (any workspace from the live machine).
        let someCard = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "tabs")).firstMatch
        XCTAssertTrue(someCard.waitForExistence(timeout: 15), "No space cards rendered")

        // The detail screen shows the space's tabs and/or agents.
        // (Section headers surface uppercased in the accessibility tree.)
        someCard.tap()
        XCTAssertTrue(
            waitForSectionHeader(app, "Agents", timeout: 15)
                || waitForSectionHeader(app, "Tabs", timeout: 15),
            "Space detail did not render agents or tabs"
        )

        // Repo-root spaces fold their linked worktrees into the detail view.
        if waitForSectionHeader(app, "Worktrees", timeout: 2) {
            // Each worktree section header carries "#N · label".
            let worktreeRow = app.otherElements.buttons.matching(
                NSPredicate(format: "label MATCHES %@", "#[0-9]+ · .+")
            ).firstMatch
            XCTAssertTrue(worktreeRow.exists, "Worktrees section rendered without worktree headers")
        }
    }

    /// Opens the Google login page without choosing an account or submitting a code.
    @MainActor
    func testAgyCardShowsLoginPrompt() throws {
        let app = try launchConnectedApp()
        defer { app.terminate() }
        let quota = app.tabBars.buttons["Quota"]
        XCTAssertTrue(quota.waitForExistence(timeout: 30))
        quota.tap()
        let card = app.buttons["agy-usage-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 45))
        card.tap()
        XCTAssertTrue(app.navigationBars["Sign in to Agy"].waitForExistence(timeout: 5))
        let enter = app.buttons["agy-login-enter"]
        let prompt = NSPredicate { _, _ in
            (enter.exists && enter.isEnabled) || app.buttons["Open Sign-in Page"].exists
                || app.secureTextFields["agy-authorization-code"].exists
        }
        let result = XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: prompt, object: nil)], timeout: 20)
        attachAgyScreenshot(app, name: "Agy login startup")
        XCTAssertEqual(result, .completed, "Agy sign-in should display a login choice or authorization prompt")
        for identifier in ["agy-login-up", "agy-login-down", "agy-login-enter"] {
            let control = app.buttons[identifier]
            XCTAssertTrue(control.isHittable)
            XCTAssertGreaterThan(control.frame.minY, app.frame.maxY * 0.75,
                                 "Login controls should stay at the bottom of the screen")
        }
        if enter.isEnabled { enter.tap() }
        let openPage = app.buttons["Open Sign-in Page"]
        XCTAssertTrue(openPage.waitForExistence(timeout: 30), "Agy should provide a browser sign-in link")
        openPage.tap()
        XCTAssertTrue(app.buttons["Done"].waitForExistence(timeout: 10), "The sign-in browser should open on iPhone")
        let googleLogin = app.webViews.staticTexts.matching(
            NSPredicate(format: "label CONTAINS[c] 'sign in' OR label CONTAINS[c] 'choose an account'")
        ).firstMatch
        XCTAssertTrue(googleLogin.waitForExistence(timeout: 25), "Google's sign-in page should finish loading")
        attachAgyScreenshot(app, name: "Agy sign-in browser on iPhone")
        app.buttons["Done"].tap()
        let codeField = app.secureTextFields["agy-authorization-code"]
        let returned = XCTWaiter.wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == true AND hittable == true"), object: codeField
        )], timeout: 10) == .completed
        attachAgyScreenshot(app, name: "Agy authorization-code field after browser")
        XCTAssertTrue(returned)
        if returned { verifyAgyCodePaste(in: app) }
        app.buttons["Close"].tap()
    }

    private func verifyAgyCodePaste(in app: XCUIApplication) {
        XCTAssertFalse(app.buttons["Submit Code"].isEnabled)
        XCTAssertTrue(app.staticTexts["agy-code-status"].label.contains("No code entered yet"))
        // Exercise actual clipboard transfer, not keyboard typing. Never
        // submit this fixture to the remote authentication process.
        UIPasteboard.general.setItems([["public.utf8-plain-text": "fixture-not-submitted"]],
                                     options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(60)])
        let paste = app.buttons["agy-paste-code"]
        XCTAssertTrue(paste.waitForExistence(timeout: 5))
        let pasteReady = XCTWaiter.wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "enabled == true AND hittable == true"), object: paste
        )], timeout: 5) == .completed
        XCTAssertTrue(pasteReady)
        if pasteReady { paste.tap() }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "enabled == true"), object: app.buttons["Submit Code"]
        )], timeout: 5), .completed, "Pasting a code should enable submission")
        XCTAssertEqual(app.staticTexts["agy-code-status"].label, "Code is ready to submit.")
    }

    private func attachAgyScreenshot(_ app: XCUIApplication, name: String) {
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = name
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    func testTabBarSwitchesBetweenSpacesAndAgents() throws {
        let app = try launchConnectedApp()

        // Both tabs are present once connected.
        let spacesTab = app.tabBars.buttons["Spaces"]
        let agentsTab = app.tabBars.buttons["Agents"]
        let isSpacesVisible = app.navigationBars.staticTexts["Spaces"].waitForExistence(timeout: 25)
            || app.navigationBars.staticTexts["Herdcats"].waitForExistence(timeout: 25)
        XCTAssertTrue(isSpacesVisible, "Spaces screen did not appear")
        XCTAssertTrue(spacesTab.exists, "Spaces tab is missing from the tab bar")
        XCTAssertTrue(agentsTab.exists, "Agents tab is missing from the tab bar")

        // Switching to the Agents tab shows the flat agent list.
        agentsTab.tap()
        let isAgentsVisible = app.navigationBars.staticTexts["Agents"].waitForExistence(timeout: 10)
            || app.navigationBars.staticTexts["Herdcats"].waitForExistence(timeout: 10)
        XCTAssertTrue(isAgentsVisible, "Agents screen did not appear")

        // Either agent rows render, or the (also valid) empty state shows.
        let agentRow = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "~/" )).firstMatch
        let emptyState = app.staticTexts["No agents running"]
        XCTAssertTrue(
            agentRow.waitForExistence(timeout: 15) || emptyState.exists,
            "Agents tab rendered neither rows nor empty state"
        )

        // Switching back restores the Spaces grid.
        spacesTab.tap()
        let isSpacesRestored = app.navigationBars.staticTexts["Spaces"].waitForExistence(timeout: 10)
            || app.navigationBars.staticTexts["Herdcats"].waitForExistence(timeout: 10)
        XCTAssertTrue(isSpacesRestored, "Spaces screen did not return")
    }

    var paneRowPredicate: NSPredicate {
        NSPredicate(format: "label MATCHES %@", ".*w[0-9A-Za-z]+:p[0-9A-Za-z]+.*")
    }

    /// Section headers appear in the accessibility tree in either original or
    /// uppercased form depending on list style, so both spellings are probed.
    @discardableResult
    func waitForSectionHeader(_ app: XCUIApplication, _ title: String, timeout: TimeInterval) -> Bool {
        let variants = [title, title.uppercased()]
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for variant in variants where app.staticTexts[variant].exists {
                return true
            }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return false
    }

    func liveKeyboardButton(_ app: XCUIApplication) -> XCUIElement {
        // Prefer the keyboard control: the `pane-live-bar` container is often
        // omitted from the accessibility tree even when the Live row is visible.
        app.buttons["pane-live-keyboard-button"]
    }

    func composeField(_ app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)["pane-compose-field"].firstMatch
    }

    /// Opens the first space's first pane. Retries the row tap once — the
    /// 5 s auto-refresh can shift cells mid-tap. Default input surface is the
    /// Live toolbar; readiness is `pane-live-keyboard-button`. Always selects
    /// the Spaces tab first — autoconnect may restore Agents as the saved tab.
    ///
    /// Space cards often navigate straight to the focused pane, so Live
    /// controls may already be present after the card tap — do not wait on
    /// a pane-switcher row before checking readiness.
    @discardableResult
    func openFirstPane(_ app: XCUIApplication) -> Bool {
        let spacesTab = app.tabBars.buttons["Spaces"]
        guard spacesTab.waitForExistence(timeout: 25) else { return false }
        if !spacesTab.isSelected {
            spacesTab.tap()
        }

        let card = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "tabs")).firstMatch
        guard card.waitForExistence(timeout: 15), card.isHittable else { return false }
        card.tap()

        // Card tap frequently opens pane detail directly (focused pane).
        if liveKeyboardButton(app).waitForExistence(timeout: 5) { return true }

        // Fallback: space still shows the pane list — pick the first pane row.
        let paneRow = app.buttons.matching(paneRowPredicate).firstMatch
        guard paneRow.waitForExistence(timeout: 8) else { return false }
        paneRow.tap()

        if liveKeyboardButton(app).waitForExistence(timeout: 8) { return true }
        // Retry: the first tap may have been swallowed by a row shift.
        guard paneRow.waitForExistence(timeout: 5) else { return false }
        paneRow.tap()
        return liveKeyboardButton(app).waitForExistence(timeout: 8)
    }

    /// Dismisses a one-shot system permission sheet when present so Voice can
    /// fail closed into Compose without stalling the suite.
    func dismissSystemPermissionIfPresent(_ app: XCUIApplication) {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let candidates = [
            springboard.alerts.buttons["Don't Allow"],
            springboard.alerts.buttons["Don’t Allow"],
            springboard.alerts.buttons["OK"],
            app.alerts.buttons["Don't Allow"],
            app.alerts.buttons["Don’t Allow"],
            app.alerts.buttons["OK"]
        ]
        for button in candidates where button.waitForExistence(timeout: 0.5) {
            button.tap()
            return
        }
    }

    /// Opens the Compose bubble via Voice → finish/error without tapping Live
    /// key buttons (Esc/Tab/Up would mutate the remote pane). Returns false
    /// when Speech/mic cannot reach Compose in this environment.
    @discardableResult
    func openComposeViaVoice(_ app: XCUIApplication) -> Bool {
        let voice = app.buttons["pane-live-voice-button"]
        guard voice.waitForExistence(timeout: 5), voice.isHittable else { return false }
        voice.tap()
        dismissSystemPermissionIfPresent(app)

        let recording = app.descendants(matching: .any)["pane-recording-bar"].firstMatch
        let field = composeField(app)
        var didTapFinish = false
        let deadline = Date().addingTimeInterval(20)
        while Date() < deadline {
            if field.exists { return true }
            dismissSystemPermissionIfPresent(app)
            // One finish tap only — Pi ignores repeats, but avoid hammering.
            if !didTapFinish, recording.exists {
                let finish = app.buttons["pane-recording-finish-button"]
                if finish.exists, finish.isHittable {
                    finish.tap()
                    didTapFinish = true
                }
            }
            // Mic/speech denial opens Compose from the error path; allow time.
            Thread.sleep(forTimeInterval: 0.25)
        }
        return field.waitForExistence(timeout: 2)
    }

    /// True when the mock-herdr fixture's known output is visible.
    func isMockHerdrFixtureVisible(_ app: XCUIApplication, timeout: TimeInterval = 10) -> Bool {
        app.textViews.matching(
            NSPredicate(format: "value CONTAINS %@", "mock terminal line one")
        ).firstMatch.waitForExistence(timeout: timeout)
    }

    /// Clears the entire Compose field, including any ambient Voice transcript.
    /// Empty fields report the placeholder as `value`.
    func clearComposeField(_ field: XCUIElement) {
        guard field.waitForExistence(timeout: 5) else { return }
        field.tap()
        let placeholder = field.placeholderValue ?? "Type to send to this pane…"
        for _ in 0..<40 {
            let current = (field.value as? String) ?? ""
            if current.isEmpty || current == placeholder { return }
            // Over-delete so residual Voice text longer than a fixed count clears.
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count + 8))
        }
    }

    func composeFieldValue(_ field: XCUIElement) -> String {
        (field.value as? String) ?? ""
    }

    func composeFieldIsEmpty(_ field: XCUIElement) -> Bool {
        let value = composeFieldValue(field)
        let placeholder = field.placeholderValue ?? "Type to send to this pane…"
        return value.isEmpty || value == placeholder
    }

    /// Opens the first space's first pane and asserts the live pane view
    /// (terminal output + Live toolbar) renders. Read-only — never types.
}
