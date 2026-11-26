import XCTest

/// 没装成的应用（假核心的 `WONTON_UI_FAILED_INSTALL`）：从侧边栏和应用格子都进得了详情，失败只说一遍，下一步能做。
final class FailedInstallVisibilityUITests: XCTestCase {
    private func launch(failedInstall: String) -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_UI_FAILED_INSTALL"] = failedInstall
        UITestBackend.launch(app)
        return app
    }

    private func openFromSidebar(_ app: XCUIApplication) {
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(sidebar.waitForExistence(timeout: 20))
        sidebar.tap()
        XCTAssertTrue(app.staticTexts["My Apps"].firstMatch.waitForExistence(timeout: 10))
        let row = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Failed Example")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
    }

    private func waitUntil(_ element: XCUIElement, _ format: String) -> Bool {
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate(format: format), object: element)
        return XCTWaiter().wait(for: [expectation], timeout: 10) == .completed
    }

    /// 核心留着原决定：“重试”直接重装，不弹确认页；装着时按钮点不了，再进详情也一样。
    func testRetryReinstallsWithoutConfirmation() {
        let app = launch(failedInstall: "1")
        openFromSidebar(app)

        let action = app.buttons["pluginDetailAction"]
        XCTAssertTrue(action.waitForExistence(timeout: 10))
        XCTAssertEqual(action.label, "Try Again")
        XCTAssertTrue(action.isEnabled)
        // 失败的原因只在详情页的失败提示里出现一次，标题下不再另挂一条。
        let details = app.buttons.matching(NSPredicate(format: "label == %@", "Details"))
        XCTAssertTrue(details.firstMatch.waitForExistence(timeout: 10))
        XCTAssertEqual(details.count, 1)
        details.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["The image could not be downloaded."].firstMatch.waitForExistence(timeout: 10))

        action.tap()
        XCTAssertTrue(app.descendants(matching: .any)["pluginTaskRunning"].firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(waitUntil(action, "isEnabled == false"))
        XCTAssertFalse(app.buttons["Install"].exists)

        let back = app.buttons["Back"].firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 10))
        back.tap()
        let icon = app.staticTexts["Failed Example"].firstMatch
        XCTAssertTrue(icon.waitForExistence(timeout: 10))
        icon.tap()
        let reopened = app.buttons["pluginDetailAction"]
        XCTAssertTrue(reopened.waitForExistence(timeout: 10))
        XCTAssertFalse(reopened.isEnabled)
    }

    /// 上传的包装失败、服务器没留下原决定：不能直接重试，下一步是重新选文件。
    func testWithoutKeptDecisionOffersInstallFromFile() {
        let app = launch(failedInstall: "noretry")
        openFromSidebar(app)

        let action = app.buttons["pluginDetailAction"]
        XCTAssertTrue(action.waitForExistence(timeout: 10))
        XCTAssertEqual(action.label, "Install from File")
        XCTAssertTrue(action.isEnabled)
        XCTAssertFalse(app.descendants(matching: .any)["pluginTaskRunning"].exists)
    }
}
