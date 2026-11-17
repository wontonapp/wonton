import XCTest

final class FailedInstallVisibilityUITests: XCTestCase {
    func testFailureOffersRecoveryFromSidebarAndLauncher() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_UI_FAILED_INSTALL"] = "1"
        app.launch()
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(sidebar.waitForExistence(timeout: 20))
        sidebar.tap()
        XCTAssertTrue(app.staticTexts["My Apps"].firstMatch.waitForExistence(timeout: 10))
        let row = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Failed Example")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()

        let retry = app.buttons["Try Again"].firstMatch
        XCTAssertTrue(retry.waitForExistence(timeout: 10))
        let details = app.buttons["Details"].firstMatch
        XCTAssertTrue(details.waitForExistence(timeout: 10))
        details.tap()
        XCTAssertTrue(app.staticTexts["The image could not be downloaded."].firstMatch.waitForExistence(timeout: 10))
        retry.tap()
        let close = app.buttons["Close"].firstMatch
        XCTAssertTrue(close.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Install"].firstMatch.waitForExistence(timeout: 10))
        close.tap()

        let uninstall = app.buttons["Uninstall"].firstMatch
        XCTAssertTrue(uninstall.waitForExistence(timeout: 10))
        for _ in 0..<4 where !uninstall.isHittable { app.swipeUp() }
        XCTAssertTrue(uninstall.isHittable)
        uninstall.tap()
        let cancel = app.buttons["Cancel"].firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 10))
        cancel.tap()

        let back = app.buttons["Back"].firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 10))
        back.tap()
        let icon = app.staticTexts["Failed Example"].firstMatch
        XCTAssertTrue(icon.waitForExistence(timeout: 10))
        icon.tap()
        XCTAssertTrue(app.buttons["Try Again"].firstMatch.waitForExistence(timeout: 10))
    }
}
