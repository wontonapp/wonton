import XCTest

final class SidebarUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
    }

    func testButtonAndLeftEdgeSwipeOpenMyApps() {
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(sidebar.waitForExistence(timeout: 10), "主界面没有侧边栏按钮")
        let myApps = app.staticTexts["My Apps"].firstMatch
        XCTAssertFalse(myApps.exists, "侧边栏应当默认关闭")

        sidebar.tap()
        assertMyAppsVisible(screenshot: "sidebar-button-open")

        app.buttons["Collapse sidebar"].firstMatch.tap()
        XCTAssertTrue(myApps.waitForNonExistence(timeout: 10), "侧边栏没有收起")

        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.01, dy: 0.5))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: end)
        assertMyAppsVisible(screenshot: "sidebar-edge-swipe-open")
    }

    private func assertMyAppsVisible(screenshot name: String) {
        let myApps = app.staticTexts["My Apps"].firstMatch
        XCTAssertTrue(myApps.waitForExistence(timeout: 10), "侧边栏没有显示我的应用")
        XCTAssertTrue(myApps.isHittable, "我的应用没有出现在可见侧边栏中")
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
