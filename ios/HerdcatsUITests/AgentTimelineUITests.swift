import XCTest
import UIKit

final class AgentTimelineUITests: XCTestCase {
    func testTimelineLightAndDarkRenderToolCardsAndKeepTerminalAvailable() {
        for light in [true, false] {
            let app = XCUIApplication()
            app.launchArguments =
                ["-hc.screenshots", "-hc.screen", "timeline", "-paneShowComposeByDefault", "NO"]
                + (light ? ["-hc.light"] : [])
            launch(app)
            XCTAssertTrue(app.buttons["timeline-open-terminal"].waitForExistence(timeout: 5))
            XCTAssertLessThan(
                app.scrollViews["agent-timeline-scroll"].frame.minY,
                app.buttons["pane-add-button"].frame.minY,
                "Conversation content must scroll behind the floating pane cards")
            XCTAssertFalse(app.staticTexts["Conversation"].exists)
            XCTAssertFalse(app.staticTexts["Idle"].exists)
            XCTAssertTrue(app.buttons["timeline-tool-tool:read"].exists)
            app.buttons["timeline-tool-tool:read"].tap()
            XCTAssertTrue(app.navigationBars["Activity"].waitForExistence(timeout: 3))
            XCTAssertTrue(app.staticTexts["Read HostKeyVerificationView.swift"].exists)
            XCTAssertFalse(app.staticTexts["Input"].exists)
            XCTAssertFalse(app.buttons["Technical details"].exists)
            XCTAssertFalse(app.staticTexts["Completed"].exists)
            app.buttons["timeline-activity-close"].tap()
            let screenshot = XCTAttachment(screenshot: app.screenshot())
            screenshot.name = light ? "Native timeline light" : "Native timeline dark"
            screenshot.lifetime = .keepAlways
            add(screenshot)
            app.buttons["timeline-open-terminal"].tap()
            XCTAssertTrue(app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5))
            XCTAssertFalse(app.descendants(matching: .any)["timeline-compose-field"].firstMatch.exists)
            XCTAssertTrue(app.buttons["pane-live-voice-button"].exists)
            let terminalScreenshot = XCTAttachment(screenshot: app.screenshot())
            terminalScreenshot.name = light ? "Terminal dark from light app" : "Terminal dark from dark app"
            terminalScreenshot.lifetime = .keepAlways
            add(terminalScreenshot)
            app.terminate()
        }
    }
    func testEmptyAndUnsupportedTimelineKeepTerminalAvailable() {
        for screen in ["timeline-empty", "timeline-unavailable"] {
            let app = XCUIApplication()
            app.launchArguments = [
                "-hc.screenshots", "-hc.screen", screen, "-hc.light", "-paneShowComposeByDefault", "NO",
            ]
            launch(app)
            XCTAssertTrue(
                app.staticTexts[screen == "timeline-empty" ? "No messages yet" : "Timeline unavailable"]
                    .waitForExistence(timeout: 5))
            XCTAssertTrue(app.buttons["timeline-open-terminal"].exists)
            app.terminate()
        }
    }
    func testNewSessionWithoutTranscriptShowsEmptyStateNotError() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-pending", "-hc.light", "-paneShowComposeByDefault", "YES"]
        launch(app)
        XCTAssertTrue(app.staticTexts["Send a message to start"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["Timeline unavailable"].exists)
        XCTAssertFalse(app.buttons["timeline-helper-retry"].exists)
        XCTAssertTrue(app.descendants(matching: .any)["timeline-compose-field"].firstMatch.exists)
        app.terminate()
    }
    func testDisconnectedTimelineCanRetryWithoutReconnectingSSH() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-disconnected", "-hc.light", "-paneShowComposeByDefault", "YES"]
        launch(app)
        XCTAssertTrue(app.staticTexts["Timeline disconnected"].waitForExistence(timeout: 5))
        let retry = app.buttons["timeline-helper-retry"]
        XCTAssertTrue(retry.exists)
        if retry.exists { retry.tap() }
        XCTAssertTrue(app.staticTexts["Timeline disconnected"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["timeline-compose-send"].isEnabled)
        XCTAssertTrue(app.buttons["timeline-open-terminal"].exists)
        app.terminate()
    }
    func testComposeRemainsEnabledWithoutNativeHistory() {
        for screen in ["timeline-unavailable"] {
            let app = XCUIApplication()
            app.launchArguments = ["-hc.screenshots", "-hc.screen", screen, "-hc.light", "-paneShowComposeByDefault", "YES"]
            launch(app)
            let field = app.descendants(matching: .any)["timeline-compose-field"].firstMatch
            XCTAssertTrue(field.waitForExistence(timeout: 5))
            // Fixture supplies a draft; this regression checks actual Send enablement.
            let send = app.buttons["timeline-compose-send"]
            XCTAssertTrue(send.waitForExistence(timeout: 5))
            XCTAssertTrue(send.isEnabled, "Existing pane composition must not depend on native history")
            app.terminate()
        }
    }
    func testMissingHelperAutomaticallyExplainsTimelineAndCanReopenFromTerminal() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-helper-missing", "-hc.light"]
        launch(app)
        XCTAssertTrue(app.staticTexts["A clearer view of your agent"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["How Herdcats Bridge works"].exists)
        XCTAssertFalse(app.staticTexts.containing(NSPredicate(format: "label CONTAINS[c] %@", "Claude session")).firstMatch.exists)
        let drawer = app.scrollViews["timeline-setup-scroll"]
        XCTAssertTrue(drawer.exists)
        let preview = XCTAttachment(screenshot: app.screenshot())
        preview.name = "Timeline first-use drawer"
        preview.lifetime = .keepAlways
        add(preview)
        let sheetTop = app.navigationBars["Set up Timeline"].frame.minY
        let sheetBottom = app.buttons["timeline-helper-use-terminal"].frame.maxY
        XCTAssertGreaterThan(sheetBottom - sheetTop, app.frame.height * 0.75)
        XCTAssertFalse(app.buttons["timeline-helper-check-again"].exists)
        let manual = app.buttons["Manual setup"]
        for _ in 0..<3 where !manual.isHittable { drawer.swipeUp() }
        manual.tap()
        let copy = app.buttons["timeline-helper-copy-command"]
        for _ in 0..<4 {
            let visibleBottom = app.buttons["timeline-helper-install"].frame.minY - 8
            if copy.isHittable && copy.frame.maxY < visibleBottom { break }
            drawer.swipeUp()
        }
        copy.tap()
        XCTAssertTrue(app.staticTexts["Command copied"].waitForExistence(timeout: 3) || copy.label == "Command copied")
        let checkAgainInside = app.buttons["timeline-helper-check-again"]
        for _ in 0..<4 {
            let visibleBottom = app.buttons["timeline-helper-install"].frame.minY - 8
            if checkAgainInside.isHittable && checkAgainInside.frame.maxY < visibleBottom { break }
            drawer.swipeUp()
        }
        checkAgainInside.tap()
        XCTAssertTrue(app.staticTexts["Herdcats Bridge was not found yet. Run the command on your SSH host, then check again."].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["A clearer view of your agent"].exists)
        app.buttons["timeline-helper-use-terminal"].tap()
        XCTAssertTrue(app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["terminal-open-timeline"].exists)
        app.buttons["terminal-open-timeline"].tap()
        XCTAssertTrue(app.staticTexts["A clearer view of your agent"].waitForExistence(timeout: 5))
        app.buttons["timeline-helper-dismiss"].tap()
        XCTAssertTrue(app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5))
        app.terminate()
        app.launchArguments.removeAll { $0 == "-hc.timeline-intro-reset" }
        app.launch()
        XCTAssertTrue(app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5), "Deferred setup defaults this host to Terminal on the next visit")
        app.buttons["terminal-open-timeline"].tap()
        XCTAssertTrue(app.staticTexts["A clearer view of your agent"].waitForExistence(timeout: 5))
        app.terminate()
    }
    func testSuccessfulHelperCheckKeepsTimelineAndSwipeDismissUsesTerminal() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-helper-missing", "-hc.light", "-hc.timeline-helper-installed"]
        launch(app)
        XCTAssertTrue(app.staticTexts["A clearer view of your agent"].waitForExistence(timeout: 5))
        // Native sheet dismissal uses the same Terminal fallback as the buttons.
        let top = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.07))
        let bottom = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
        top.press(forDuration: 0.1, thenDragTo: bottom)
        XCTAssertTrue(app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5))
        app.buttons["terminal-open-timeline"].tap()
        XCTAssertTrue(app.staticTexts["A clearer view of your agent"].waitForExistence(timeout: 5))
        let drawer = app.scrollViews["timeline-setup-scroll"]
        let manual = app.buttons["Manual setup"]
        for _ in 0..<3 where !manual.isHittable { drawer.swipeUp() }
        manual.tap()
        let checkAgain = app.buttons["timeline-helper-check-again"]
        for _ in 0..<4 {
            let visibleBottom = app.buttons["timeline-helper-install"].frame.minY - 8
            if checkAgain.isHittable && checkAgain.frame.maxY < visibleBottom { break }
            drawer.swipeUp()
        }
        checkAgain.tap()
        XCTAssertTrue(app.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["A clearer view of your agent"].exists)
        XCTAssertTrue(app.buttons["timeline-open-terminal"].exists)
        app.terminate()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline", "-hc.light"]
        app.launch()
        XCTAssertTrue(app.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5), "Successful setup restores Timeline as the default for this host")
        app.terminate()
    }
    func testOneClickInstallIsExplicitAndReturnsToTimeline() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-helper-missing", "-hc.light", "-hc.timeline-install-result", "success"]
        launch(app)
        let install = app.buttons["timeline-helper-install"]
        let exists = install.waitForExistence(timeout: 5)
        XCTAssertTrue(exists)
        guard exists else { app.terminate(); return }
        XCTAssertEqual(install.label, "Install Herdcats Bridge")
        XCTAssertTrue(app.staticTexts["demo@studio-mac"].exists)
        XCTAssertTrue(app.staticTexts["~/.local/bin/herdcats-bridge"].exists)
        XCTAssertTrue(app.staticTexts["A clearer view of your agent"].exists, "Opening the drawer must not install automatically")
        install.tap()
        XCTAssertTrue(app.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["timeline-open-terminal"].exists)
        XCTAssertFalse(app.staticTexts["A clearer view of your agent"].exists)
        app.terminate()
    }
    func testHelperReplacementAndUncertainInstallRequireSeparateActions() {
        for scenario in ["replacement", "uncertain", "failure", "probe_failure"] {
            let app = XCUIApplication()
            app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-helper-missing", "-hc.light", "-hc.timeline-install-result", scenario]
            launch(app)
            let install = app.buttons["timeline-helper-install"]
            XCTAssertTrue(install.waitForExistence(timeout: 5))
            install.tap()
            let feedback = app.staticTexts["timeline-setup-feedback"]
            XCTAssertTrue(feedback.waitForExistence(timeout: 5))
            XCTAssertFalse(app.buttons["timeline-tool-tool:read"].exists)
            if scenario == "replacement" {
                XCTAssertEqual(install.label, "Replace Herdcats Bridge with 0.2.2")
                install.tap()
                XCTAssertTrue(app.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5))
            } else if scenario == "uncertain" {
                XCTAssertEqual(install.label, "Check installation")
                install.tap()
                XCTAssertTrue(app.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5))
            } else {
                XCTAssertTrue(feedback.label.contains(scenario == "failure" ? "could not download" : "No installation was started"))
                XCTAssertEqual(install.label, "Install Herdcats Bridge")
                app.buttons["timeline-helper-use-terminal"].tap()
                XCTAssertTrue(app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5))
            }
            app.terminate()
        }
    }
    func testKeyboardSurroundFollowsActivePaneBackground() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "compose", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        var themedHeader: [Double]?
        for timeline in [true, false] {
            if !timeline {
                app.buttons["timeline-open-terminal"].tap()
                app.buttons["pane-live-keyboard-button"].tap()
            }
            let field = app.descendants(matching: .any)[timeline ? "timeline-compose-field" : "pane-compose-field"].firstMatch
            XCTAssertTrue(field.waitForExistence(timeout: 5))
            field.tap()
            XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
            let screenshot = app.screenshot()
            let attachment = XCTAttachment(screenshot: screenshot)
            attachment.name = timeline ? "Timeline keyboard surround" : "Terminal keyboard surround"
            attachment.lifetime = .keepAlways
            add(attachment)
            guard let image = screenshot.image.cgImage else { return XCTFail("Missing screenshot") }
            let header = pixel(image, x: 5, y: 5)
            if timeline { themedHeader = header }
            if let themedHeader {
                for channel in 0..<3 {
                    XCTAssertEqual(header[channel], themedHeader[channel], accuracy: 6,
                                   "Header must retain the app theme when switching to Terminal")
                }
            }
            let output = pixel(image, x: 5, y: image.height / 3)
            let surround = pixel(image, x: 5, y: image.height - 5)
            let keyboard = pixel(image, x: 5, y: image.height * 4 / 5)
            if timeline {
                XCTAssertGreaterThan(keyboard.prefix(3).reduce(0, +), 400)
            } else {
                XCTAssertLessThan(keyboard.prefix(3).reduce(0, +), 300,
                                  "Terminal Compose must use a dark keyboard even in a light app")
            }
            for channel in 0..<3 {
                XCTAssertEqual(output[channel], surround[channel], accuracy: 6,
                               "Keyboard safe-area background must match the active pane")
            }
        }
        app.terminate()
    }

    private func pixel(_ image: CGImage, x: Int, y: Int) -> [Double] {
        var bytes = [UInt8](repeating: 0, count: 4)
        let cropped = image.cropping(to: CGRect(x: x, y: y, width: 1, height: 1))!
        bytes.withUnsafeMutableBytes { buffer in
            let context = CGContext(data: buffer.baseAddress, width: 1, height: 1,
                                    bitsPerComponent: 8, bytesPerRow: 4,
                                    space: CGColorSpaceCreateDeviceRGB(),
                                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
            context.draw(cropped, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        }
        return bytes.map(Double.init)
    }

    func testUnsupportedAgentsStayInTerminalWithoutTimelineSetup() {
        for agent in ["cursor", "unknown"] {
            let app = XCUIApplication()
            app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-helper-missing", "-hc.agent", agent, "-hc.light", "-paneShowComposeByDefault", "YES"]
            launch(app)
            XCTAssertTrue(app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5))
            XCTAssertFalse(app.buttons["terminal-open-timeline"].exists)
            XCTAssertFalse(app.buttons["timeline-open-terminal"].exists)
            XCTAssertFalse(app.staticTexts["A clearer view of your agent"].exists)
            XCTAssertTrue(app.descendants(matching: .any)["pane-compose-field"].firstMatch.exists)
            app.terminate()
        }
    }

    func testCodexHelperUpdateShowsUpdateHelperLabel() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-codex-helper-old", "-hc.agent", "codex", "-hc.light", "-hc.timeline-install-result", "replacement"]
        launch(app)
        let install = app.buttons["timeline-helper-install"]
        XCTAssertTrue(install.waitForExistence(timeout: 5))
        XCTAssertEqual(install.label, "Upgrade to 0.2.2")
        XCTAssertTrue(app.staticTexts["A newer Herdcats Bridge is available"].exists)
        install.tap()
        XCTAssertTrue(app.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["A newer Herdcats Bridge is available"].exists)
        app.terminate()

        let appExternal = XCUIApplication()
        appExternal.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-codex-helper-old", "-hc.agent", "codex", "-hc.light", "-hc.timeline-install-result", "external_replacement"]
        launch(appExternal)
        let installExt = appExternal.buttons["timeline-helper-install"]
        XCTAssertTrue(installExt.waitForExistence(timeout: 5))
        XCTAssertEqual(installExt.label, "Upgrade to 0.2.2")
        installExt.tap()
        let feedback = appExternal.staticTexts["timeline-setup-feedback"]
        XCTAssertTrue(feedback.waitForExistence(timeout: 5))
        XCTAssertEqual(feedback.label, "An existing Herdcats Bridge was found elsewhere. Install a managed copy at the location shown; the other copy will remain unchanged.")
        XCTAssertEqual(installExt.label, "Install managed Herdcats Bridge")
        installExt.tap()
        XCTAssertTrue(appExternal.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5))
        appExternal.terminate()
    }

    func testLatestIsCircularTrailingButtonAndResumesFollowing() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline", "-hc.timeline-long", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let scroll = app.scrollViews["agent-timeline-scroll"]
        XCTAssertTrue(scroll.waitForExistence(timeout: 5))
        let latest = app.buttons["timeline-latest"]
        for _ in 0..<8 {
            scroll.swipeUp()
            if !latest.exists { break }
        }
        XCTAssertTrue(latest.waitForNonExistence(timeout: 5), "No jump button at the bottom")
        XCTAssertFalse(app.staticTexts["Some session records are not displayed."].exists)
        scroll.swipeDown()
        XCTAssertTrue(latest.waitForExistence(timeout: 5))
        XCTAssertEqual(latest.frame.width, latest.frame.height, accuracy: 1)
        XCTAssertGreaterThanOrEqual(latest.frame.width, 44)
        XCTAssertGreaterThan(latest.frame.midX, app.frame.width * 0.75)
        XCTAssertEqual(latest.label, "Jump to latest messages")
        XCTAssertFalse(app.staticTexts["Latest"].exists)
        let capture = XCTAttachment(screenshot: app.screenshot())
        capture.name = "Timeline circular latest button"
        capture.lifetime = .keepAlways
        add(capture)
        for _ in 0..<8 {
            scroll.swipeUp()
            if !latest.exists { break }
        }
        XCTAssertTrue(latest.waitForNonExistence(timeout: 5), "Manual return to bottom hides the button")
        scroll.swipeDown()
        XCTAssertTrue(latest.waitForExistence(timeout: 5))
        latest.tap()
        XCTAssertTrue(latest.waitForNonExistence(timeout: 5))
        app.terminate()
    }

    func testShortTimelineDoesNotShowLatestAfterDragging() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-empty", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let scroll = app.scrollViews["agent-timeline-scroll"]
        XCTAssertTrue(app.staticTexts["No messages yet"].waitForExistence(timeout: 5))
        scroll.swipeDown()
        XCTAssertFalse(app.buttons["timeline-latest"].exists)
        app.terminate()
    }

    func testImagesRenderWithoutOriginalRecordControls() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-rich-content", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let scroll = app.scrollViews["agent-timeline-scroll"]
        XCTAssertTrue(scroll.waitForExistence(timeout: 5))
        for _ in 0..<4 { scroll.swipeUp() }
        let image = app.descendants(matching: .any)["timeline-image-content:image"].firstMatch
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Timeline image without original records"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        image.tap()
        let originalPreview = app.otherElements["QLPreviewControllerView"]
        XCTAssertTrue(originalPreview.waitForExistence(timeout: 5))
        XCTAssertTrue(originalPreview.images.firstMatch.waitForExistence(timeout: 10))
        let preview = XCTAttachment(screenshot: app.screenshot())
        preview.name = "Original session image"
        preview.lifetime = .keepAlways
        add(preview)
        // Reopen after Quick Look to inspect the conversation controls.
        app.terminate()
        launch(app)
        XCTAssertTrue(scroll.waitForExistence(timeout: 5))
        for _ in 0..<4 { scroll.swipeUp() }
        XCTAssertFalse(app.staticTexts["future_block"].exists)
        XCTAssertFalse(app.staticTexts["Original records"].exists)
        XCTAssertFalse(app.buttons["View original record"].exists)
        XCTAssertFalse(app.buttons["timeline-source-content:unfamiliar"].exists)
        app.terminate()
    }

    func testConsecutiveToolsOpenSummarizedActivityDrawer() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-tool-activity", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let activity = app.buttons["timeline-tool-tool:read"]
        XCTAssertTrue(activity.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Ran a command · Used 2 tools"].exists)
        XCTAssertFalse(app.buttons["timeline-tool-tool:push"].exists)
        activity.tap()
        XCTAssertTrue(app.navigationBars["Activity"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Read HostKeyVerificationView.swift"].exists)
        XCTAssertTrue(app.staticTexts["Push changes to pull request #11"].exists)
        XCTAssertTrue(app.staticTexts["Edit README.md"].exists)
        XCTAssertFalse(app.staticTexts["Input"].exists)
        XCTAssertFalse(app.staticTexts["Push rejected."].exists)
        XCTAssertFalse(app.staticTexts["Bash"].exists)
        XCTAssertFalse(app.staticTexts["Completed"].exists)
        XCTAssertFalse(app.buttons["Technical details"].exists)
        XCTAssertEqual(app.images["timeline-activity-icon-tool:push"].label, "Failed or interrupted")
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Summarized activity drawer"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.buttons["timeline-activity-close"].tap()
        XCTAssertTrue(activity.exists)
        app.terminate()
    }

    func testPullingLoadsEarlierMessagesWithoutAButton() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-history", "-hc.timeline-history-delay", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let scroll = app.scrollViews["agent-timeline-scroll"]
        XCTAssertTrue(scroll.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Load earlier messages"].exists)
        // Start below the floating pane cards so the gesture reaches history.
        let start = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45))
        let end = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.95))
        for _ in 0..<4 { start.press(forDuration: 0.05, thenDragTo: end) }
        XCTAssertTrue(app.staticTexts["Earlier conversation message."].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["timeline-history-hint"].exists)
        app.terminate()
    }

    func testLoadingEarlierMessagesPreservesVisibleMessageOffset() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-history-position", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let scroll = app.scrollViews["agent-timeline-scroll"]
        XCTAssertTrue(scroll.waitForExistence(timeout: 5))
        // Collapse the pane strip separately, before measuring the reading position.
        let add = app.buttons["pane-add-button"]
        let card = add.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        card.press(forDuration: 0.05, thenDragTo: card.withOffset(CGVector(dx: 0, dy: -60)))
        let marker = app.staticTexts["Review the connection guidance and make it clearer in light mode."]
        XCTAssertTrue(marker.exists)
        let originalY = marker.frame.minY
        let start = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45))
        let end = scroll.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.95))
        for _ in 0..<4 {
            start.press(forDuration: 0.05, thenDragTo: end)
            if app.staticTexts["Loading earlier messages…"].exists || !app.staticTexts["timeline-history-hint"].exists { break }
        }
        XCTAssertTrue(app.staticTexts["timeline-history-hint"].waitForNonExistence(timeout: 15))
        XCTAssertTrue(marker.exists)
        XCTAssertEqual(marker.frame.minY, originalY, accuracy: 4,
                       "Prepending a page must preserve the reading position")
        XCTAssertFalse(app.staticTexts["Earlier conversation message."].isHittable)
        app.terminate()
    }

    func testCodeChangesRenderAndOpenDiffDrawer() {
        for light in [true, false] {
            let app = XCUIApplication()
            app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-code-changes", "-paneShowComposeByDefault", "NO"] + (light ? ["-hc.light"] : [])
            launch(app)
            let scroll = app.scrollViews["agent-timeline-scroll"]
            XCTAssertTrue(scroll.waitForExistence(timeout: 5))
            let card = app.buttons["timeline-diff-tool:edit-triage:diff:0"]
            XCTAssertTrue(card.waitForExistence(timeout: 5))
            for _ in 0..<4 where !card.isHittable { scroll.swipeDown() }
            let inline = XCTAttachment(screenshot: app.screenshot())
            inline.name = light ? "Inline code changes light" : "Inline code changes dark"
            inline.lifetime = .keepAlways
            add(inline)
            card.tap()
            XCTAssertTrue(app.navigationBars["Code changes"].waitForExistence(timeout: 5))
            XCTAssertTrue(app.staticTexts["reply-tool/triage.ts"].exists)
            XCTAssertTrue(app.staticTexts["+2"].exists)
            XCTAssertTrue(app.staticTexts["−1"].exists)
            let expanded = XCTAttachment(screenshot: app.screenshot())
            expanded.name = light ? "Code diff drawer light" : "Code diff drawer dark"
            expanded.lifetime = .keepAlways
            add(expanded)
            app.buttons["timeline-diff-close"].tap()
            for _ in 0..<4 where !app.buttons["timeline-tool-tool:edit-triage"].isHittable { scroll.swipeDown() }
            app.buttons["timeline-tool-tool:edit-triage"].tap()
            XCTAssertTrue(app.staticTexts["Background review completed"].waitForExistence(timeout: 5))
            XCTAssertFalse(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "<task-notification>")).firstMatch.exists)
            app.buttons["timeline-activity-close"].tap()
            app.terminate()
        }
    }

    func testLargeFileWritesLoadRecordedContentForPreview() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-large-write", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let card = app.buttons["timeline-diff-tool:large-write:diff:0"]
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        card.tap()
        XCTAssertTrue(app.navigationBars["Written content"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["project/PaywallView.swift"].exists)
        XCTAssertFalse(app.staticTexts["MODIFIED"].exists)
        let preview = XCTAttachment(screenshot: app.screenshot())
        preview.name = "Large recorded file preview"
        preview.lifetime = .keepAlways
        add(preview)
        app.buttons["timeline-diff-close"].tap()
        app.terminate()
    }

    private func launch(_ app: XCUIApplication) {
        app.launchArguments += ["-hc.timeline-intro-reset"]
        app.launch()
    }
    func testTerminalHonorsComposeByDefault() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "compose", "-paneShowComposeByDefault", "YES"]
        launch(app)
        XCTAssertTrue(app.buttons["timeline-open-terminal"].waitForExistence(timeout: 5))
        app.buttons["timeline-open-terminal"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["pane-compose-field"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["pane-compose-voice-button"].exists)
        XCTAssertTrue(app.buttons["pane-compose-attach-button"].exists)
        app.terminate()
    }
    func testSwitchingOutputModesPreservesComposeDraft() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "compose", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let field = app.descendants(matching: .any)["timeline-compose-field"].firstMatch
        // The fixture opens Compose with a deterministic draft.
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        let draft = field.value as? String
        let timelineFieldWidth = field.frame.width
        let terminalButton = app.buttons["timeline-open-terminal"]
        XCTAssertTrue(terminalButton.waitForExistence(timeout: 5))
        XCTAssertTrue(terminalButton.label.contains("Terminal"))
        let menu = app.buttons["pane-options-menu"]
        XCTAssertTrue(menu.exists)
        XCTAssertEqual(terminalButton.frame.midY, menu.frame.midY, accuracy: 8,
                       "Mode switch belongs in the navigation toolbar beside the menu")
        XCTAssertLessThan(terminalButton.frame.midX, menu.frame.midX)
        terminalButton.tap()
        XCTAssertFalse(field.exists)
        app.buttons["pane-live-keyboard-button"].tap()
        let terminalField = app.descendants(matching: .any)["pane-compose-field"].firstMatch
        let terminalComposeExists = terminalField.waitForExistence(timeout: 5)
        XCTAssertTrue(terminalComposeExists)
        guard terminalComposeExists else { app.terminate(); return }
        XCTAssertEqual(terminalField.value as? String, draft)
        XCTAssertTrue(app.buttons["pane-compose-voice-button"].exists)
        XCTAssertTrue(app.buttons["pane-compose-attach-button"].exists)
        XCTAssertEqual(terminalField.frame.width, timelineFieldWidth, accuracy: 2,
                       "Both modes use the same floating Compose layout")
        let terminalCompose = XCTAttachment(screenshot: app.screenshot())
        terminalCompose.name = "Terminal floating Compose"
        terminalCompose.lifetime = .keepAlways
        add(terminalCompose)
        let composeButton = app.buttons["terminal-open-timeline"]
        XCTAssertTrue(composeButton.label.contains("Timeline"))
        composeButton.tap()
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, draft)
        app.terminate()
    }

    private func saveNamedScreenshot(_ screenshot: XCUIScreenshot, name: String) {
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        let dir = ProcessInfo.processInfo.environment["HC_SCREENSHOT_DIR"] ?? ProcessInfo.processInfo.environment["SIMCTL_CHILD_HC_SCREENSHOT_DIR"]
        if let dir, !dir.isEmpty {
            let url = URL(fileURLWithPath: dir).appendingPathComponent("\(name).png")
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? screenshot.pngRepresentation.write(to: url)
        }
    }

    func testBridgeSetupScreenshotsAndFlows() {
        // 1. codex-pane-needs-upgrade.png
        let appUpgradePane = XCUIApplication()
        appUpgradePane.launchArguments = [
            "-hc.screenshots", "-hc.screen", "timeline-codex-needs-upgrade", "-hc.agent", "codex", "-hc.light", "-hc.timeline-suppress-drawer", "-paneShowComposeByDefault", "NO"
        ]
        launch(appUpgradePane)
        let upgradeButton = appUpgradePane.buttons["timeline-helper-setup"]
        XCTAssertTrue(upgradeButton.waitForExistence(timeout: 5))
        XCTAssertEqual(upgradeButton.label, "Upgrade Herdcats Bridge")
        XCTAssertTrue(appUpgradePane.staticTexts["Herdcats Bridge 0.1.4 on this computer can't show Codex conversations. Upgrade to 0.2.2."].exists)
        XCTAssertFalse(appUpgradePane.buttons["timeline-helper-retry"].exists)
        saveNamedScreenshot(appUpgradePane.screenshot(), name: "codex-pane-needs-upgrade")

        // 2. upgrade-drawer.png
        upgradeButton.tap()
        let upgradeDrawer = appUpgradePane.scrollViews["timeline-setup-scroll"]
        XCTAssertTrue(upgradeDrawer.waitForExistence(timeout: 5))
        XCTAssertTrue(appUpgradePane.navigationBars["Upgrade Herdcats Bridge"].exists)
        XCTAssertTrue(appUpgradePane.staticTexts["A newer Herdcats Bridge is available"].exists)
        XCTAssertTrue(appUpgradePane.buttons["timeline-helper-install"].exists)
        XCTAssertEqual(appUpgradePane.buttons["timeline-helper-install"].label, "Upgrade to 0.2.2")
        saveNamedScreenshot(appUpgradePane.screenshot(), name: "upgrade-drawer")
        appUpgradePane.terminate()

        // 3. missing-pane.png
        let appMissingPane = XCUIApplication()
        appMissingPane.launchArguments = [
            "-hc.screenshots", "-hc.screen", "timeline-pane-missing", "-hc.light", "-hc.timeline-suppress-drawer", "-paneShowComposeByDefault", "NO"
        ]
        launch(appMissingPane)
        let installButton = appMissingPane.buttons["timeline-helper-setup"]
        XCTAssertTrue(installButton.waitForExistence(timeout: 5))
        XCTAssertEqual(installButton.label, "Install Herdcats Bridge")
        XCTAssertTrue(appMissingPane.staticTexts["Install Herdcats Bridge to see this session as native messages."].exists)
        XCTAssertFalse(appMissingPane.buttons["timeline-helper-retry"].exists)
        saveNamedScreenshot(appMissingPane.screenshot(), name: "missing-pane")
        appMissingPane.terminate()

        // 4. missing-drawer.png
        let appMissingDrawer = XCUIApplication()
        appMissingDrawer.launchArguments = [
            "-hc.screenshots", "-hc.screen", "timeline-helper-missing", "-hc.light", "-paneShowComposeByDefault", "NO"
        ]
        launch(appMissingDrawer)
        let missingScroll = appMissingDrawer.scrollViews["timeline-setup-scroll"]
        XCTAssertTrue(missingScroll.waitForExistence(timeout: 5))
        XCTAssertTrue(appMissingDrawer.navigationBars["Set up Timeline"].exists)
        XCTAssertTrue(appMissingDrawer.staticTexts["A clearer view of your agent"].exists)
        XCTAssertEqual(appMissingDrawer.buttons["timeline-helper-install"].label, "Install Herdcats Bridge")
        XCTAssertFalse(appMissingDrawer.buttons["timeline-helper-check-again"].exists)
        saveNamedScreenshot(appMissingDrawer.screenshot(), name: "missing-drawer")

        // 5. manual-setup-expanded.png
        let manual = appMissingDrawer.buttons["Manual setup"]
        for _ in 0..<3 where !manual.isHittable { missingScroll.swipeUp() }
        manual.tap()
        let checkAgain = appMissingDrawer.buttons["timeline-helper-check-again"]
        for _ in 0..<4 {
            let visibleBottom = appMissingDrawer.buttons["timeline-helper-install"].frame.minY - 8
            if checkAgain.isHittable && checkAgain.frame.maxY < visibleBottom { break }
            missingScroll.swipeUp()
        }
        XCTAssertTrue(checkAgain.waitForExistence(timeout: 3))
        XCTAssertTrue(appMissingDrawer.buttons["timeline-helper-copy-command"].exists)
        saveNamedScreenshot(appMissingDrawer.screenshot(), name: "manual-setup-expanded")
        appMissingDrawer.terminate()
    }

    func testTurnInterruptedAndOptionalUpgradeScreenshots() {
        // 1. turn-interrupted.png
        let appTurn = XCUIApplication()
        appTurn.launchArguments = [
            "-hc.screenshots", "-hc.screen", "turn-interrupted", "-hc.light", "-paneShowComposeByDefault", "NO"
        ]
        launch(appTurn)
        XCTAssertTrue(appTurn.staticTexts["Turn interrupted"].waitForExistence(timeout: 5))
        saveNamedScreenshot(appTurn.screenshot(), name: "turn-interrupted")
        appTurn.terminate()

        // 2. optional-upgrade-menu.png
        let appMenu = XCUIApplication()
        appMenu.launchArguments = [
            "-hc.screenshots", "-hc.screen", "optional-upgrade-menu", "-hc.light", "-paneShowComposeByDefault", "NO"
        ]
        launch(appMenu)
        let menuButton = appMenu.buttons["pane-options-menu"]
        XCTAssertTrue(menuButton.waitForExistence(timeout: 5))
        menuButton.tap()
        let upgradeOption = appMenu.buttons["timeline-upgrade-menu-button"]
        XCTAssertTrue(upgradeOption.waitForExistence(timeout: 5))
        saveNamedScreenshot(appMenu.screenshot(), name: "optional-upgrade-menu")
        appMenu.terminate()
    }

    func testWaitingForCodexSessionDisplaysNewCopyAndKeepsComposerUsable() {
        let app = XCUIApplication()
        app.launchArguments = [
            "-hc.screenshots", "-hc.screen", "timeline-codex-no-session", "-hc.agent", "codex", "-hc.light", "-paneShowComposeByDefault", "YES"
        ]
        launch(app)
        XCTAssertTrue(app.staticTexts["Codex session not identified"].waitForExistence(timeout: 5))
        let expectedExplanation = "Codex's shared background server is running, so Herdcats can't tell which session belongs to this pane."
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label == %@", expectedExplanation)).firstMatch.exists)
        XCTAssertTrue(app.descendants(matching: .any)["timeline-compose-field"].firstMatch.exists)
        XCTAssertTrue(app.buttons["Turn off Codex's shared background server"].exists)
        app.terminate()
    }

    func testRoutedHostTimelineExplainsGatewayAndKeepsComposerUsable() {
        let app = XCUIApplication()
        app.launchArguments = [
            "-hc.screenshots", "-hc.screen", "timeline-routed-host", "-hc.agent", "codex", "-hc.light", "-paneShowComposeByDefault", "YES"
        ]
        launch(app)
        XCTAssertTrue(app.staticTexts["Timeline isn't available here yet"].waitForExistence(timeout: 5))
        let expectedExplanation = "This machine is reached through Herdr on studio-mac. Timeline can read sessions only on a computer Herdcats connects to with SSH. To use Timeline here, add Studio Mini as its own connection. You can still send messages below."
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label == %@", expectedExplanation)).firstMatch.exists)
        XCTAssertFalse(app.staticTexts["Timeline unavailable"].exists)
        // Retrying, installing, or daemon toggling can never change the outcome.
        XCTAssertFalse(app.buttons["timeline-helper-retry"].exists)
        XCTAssertFalse(app.buttons["timeline-helper-setup"].exists)
        XCTAssertFalse(app.buttons["timeline-codex-disable-daemon"].exists)
        XCTAssertTrue(app.descendants(matching: .any)["timeline-compose-field"].firstMatch.exists)
        saveNamedScreenshot(app.screenshot(), name: "routed-host")
        app.terminate()
    }

    func testDismissingMenuOpenedUpgradeKeepsTimelineSelected() {
        let app = XCUIApplication()
        app.launchArguments = [
            "-hc.screenshots", "-hc.screen", "optional-upgrade-menu", "-hc.light", "-paneShowComposeByDefault", "NO"
        ]
        launch(app)
        XCTAssertTrue(app.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5))
        app.buttons["pane-options-menu"].tap()
        let upgradeOption = app.buttons["timeline-upgrade-menu-button"]
        XCTAssertTrue(upgradeOption.waitForExistence(timeout: 5))
        upgradeOption.tap()
        XCTAssertTrue(app.navigationBars["Upgrade Herdcats Bridge"].waitForExistence(timeout: 5))
        // Dismiss the optional upgrade without acting on it.
        let top = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.07))
        let bottom = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
        top.press(forDuration: 0.1, thenDragTo: bottom)
        XCTAssertTrue(app.buttons["timeline-tool-tool:read"].waitForExistence(timeout: 5),
                      "Dismissing a menu-opened upgrade must keep the working Timeline")
        XCTAssertTrue(app.buttons["timeline-open-terminal"].exists)
        XCTAssertFalse(app.scrollViews["pane-output-scroll-view"].exists)
        app.terminate()
    }
}
