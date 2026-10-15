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
            XCTAssertTrue(app.staticTexts["Conversation"].exists)
            XCTAssertTrue(app.buttons["timeline-tool-tool:read"].exists)
            app.buttons["timeline-tool-tool:read"].tap()
            XCTAssertTrue(app.staticTexts["Input"].waitForExistence(timeout: 3))
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
        XCTAssertTrue(app.staticTexts["How the host helper works"].exists)
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
        let manual = app.buttons["Manual setup"]
        for _ in 0..<3 where !manual.isHittable { drawer.swipeUp() }
        manual.tap()
        let copy = app.buttons["timeline-helper-copy-command"]
        for _ in 0..<4 {
            let visibleBottom = app.staticTexts["demo@studio-mac"].frame.minY - 8
            if copy.isHittable && copy.frame.maxY < visibleBottom { break }
            drawer.swipeUp()
        }
        copy.tap()
        XCTAssertTrue(app.staticTexts["Command copied"].waitForExistence(timeout: 3))
        app.buttons["timeline-helper-check-again"].tap()
        XCTAssertTrue(app.staticTexts["The helper was not found yet. Run the command on your SSH host, then check again."].waitForExistence(timeout: 5))
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
        app.buttons["timeline-helper-check-again"].tap()
        XCTAssertTrue(app.staticTexts["Conversation"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["A clearer view of your agent"].exists)
        XCTAssertTrue(app.buttons["timeline-open-terminal"].exists)
        app.terminate()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline", "-hc.light"]
        app.launch()
        XCTAssertTrue(app.staticTexts["Conversation"].waitForExistence(timeout: 5), "Successful setup restores Timeline as the default for this host")
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
        XCTAssertEqual(install.label, "Install Timeline helper")
        XCTAssertTrue(app.staticTexts["demo@studio-mac"].exists)
        XCTAssertTrue(app.staticTexts["~/.local/bin/herdcats-bridge"].exists)
        XCTAssertTrue(app.staticTexts["A clearer view of your agent"].exists, "Opening the drawer must not install automatically")
        install.tap()
        XCTAssertTrue(app.staticTexts["Conversation"].waitForExistence(timeout: 5))
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
            XCTAssertFalse(app.staticTexts["Conversation"].exists)
            if scenario == "replacement" {
                XCTAssertEqual(install.label, "Replace helper with 0.1.4")
                install.tap()
                XCTAssertTrue(app.staticTexts["Conversation"].waitForExistence(timeout: 5))
            } else if scenario == "uncertain" {
                XCTAssertEqual(install.label, "Check installation")
                install.tap()
                XCTAssertTrue(app.staticTexts["Conversation"].waitForExistence(timeout: 5))
            } else {
                XCTAssertTrue(feedback.label.contains(scenario == "failure" ? "could not download" : "No installation was started"))
                XCTAssertEqual(install.label, "Install Timeline helper")
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
        for agent in ["codex", "cursor", "unknown"] {
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

    func testImagesRenderAndUnfamiliarRecordsKeepTheirFullContents() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-rich-content", "-hc.light", "-paneShowComposeByDefault", "NO"]
        launch(app)
        let scroll = app.scrollViews["agent-timeline-scroll"]
        XCTAssertTrue(scroll.waitForExistence(timeout: 5))
        for _ in 0..<4 { scroll.swipeUp() }
        let image = app.descendants(matching: .any)["timeline-image-content:image"].firstMatch
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Timeline image and unfamiliar record"
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
        // Quick Look may hide its chrome. Reopen the fixture for the separate
        // source-record assertion instead of depending on a system button label.
        app.terminate()
        launch(app)
        XCTAssertTrue(scroll.waitForExistence(timeout: 5))
        for _ in 0..<4 { scroll.swipeUp() }
        let original = app.buttons["timeline-source-content:unfamiliar"]
        XCTAssertTrue(original.waitForExistence(timeout: 5))
        original.tap()
        XCTAssertTrue(app.navigationBars["Session record"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "Every field is preserved")).firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["timeline-source-share"].exists)
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
}
