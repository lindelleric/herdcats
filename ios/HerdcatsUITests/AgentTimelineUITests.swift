import XCTest

final class AgentTimelineUITests: XCTestCase {
    func testTimelineLightAndDarkRenderToolCardsAndKeepTerminalAvailable() {
        for light in [true, false] {
            let app = XCUIApplication()
            app.launchArguments =
                ["-hc.screenshots", "-hc.screen", "timeline", "-paneShowComposeByDefault", "NO"]
                + (light ? ["-hc.light"] : [])
            app.launch()
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
            XCTAssertFalse(app.buttons["pane-live-voice-button"].exists)
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
            app.launch()
            XCTAssertTrue(
                app.staticTexts[screen == "timeline-empty" ? "No messages yet" : "Timeline unavailable"]
                    .waitForExistence(timeout: 5))
            XCTAssertTrue(app.buttons["timeline-open-terminal"].exists)
            app.terminate()
        }
    }
    func testMissingHelperOffersSetupAndRecheck() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "timeline-helper-missing", "-hc.light"]
        app.launch()
        let setup = app.buttons["timeline-helper-setup"]
        XCTAssertTrue(setup.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["timeline-open-terminal"].exists)
        setup.tap()
        XCTAssertTrue(app.staticTexts["Set up native Timeline"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["timeline-helper-copy-command"].exists)
        app.buttons["timeline-helper-copy-command"].tap()
        XCTAssertTrue(app.staticTexts["Command copied"].exists)
        // Let the label replacement animation settle before capturing the preview.
        Thread.sleep(forTimeInterval: 0.5)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Host helper setup"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.buttons["timeline-helper-check-again"].tap()
        XCTAssertTrue(setup.waitForExistence(timeout: 5))
        app.buttons["timeline-open-terminal"].tap()
        XCTAssertTrue(app.scrollViews["pane-output-scroll-view"].waitForExistence(timeout: 5))
        app.terminate()
    }
    func testSwitchingOutputModesPreservesComposeDraft() {
        let app = XCUIApplication()
        app.launchArguments = ["-hc.screenshots", "-hc.screen", "compose", "-paneShowComposeByDefault", "NO"]
        app.launch()
        let field = app.descendants(matching: .any)["timeline-compose-field"].firstMatch
        // The fixture opens Compose with a deterministic draft.
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        let draft = field.value as? String
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
        XCTAssertFalse(app.descendants(matching: .any)["pane-compose-field"].firstMatch.exists)
        let composeButton = app.buttons["terminal-open-timeline"]
        XCTAssertTrue(composeButton.label.contains("Compose"))
        composeButton.tap()
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, draft)
        app.terminate()
    }
}
