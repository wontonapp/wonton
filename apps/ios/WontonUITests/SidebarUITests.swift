import XCTest

final class SidebarUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
    }

    func testButtonAndLeftEdgeSwipeOpenMyApps() {
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(sidebar.waitForExistence(timeout: 10), "主界面没有侧边栏按钮")
        let myApps = app.staticTexts["My Apps"].firstMatch
        XCTAssertFalse(myApps.exists, "侧边栏应当默认关闭")
        XCTAssertTrue(sidebar.isHittable, "侧边栏按钮不可点击")
        let closed = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        closed.name = "sidebar-menu-closed"
        closed.lifetime = .keepAlways
        add(closed)
        let hierarchy = XCTAttachment(string: app.debugDescription)
        hierarchy.name = "sidebar-menu-closed-accessibility"
        hierarchy.lifetime = .keepAlways
        add(hierarchy)

        sidebar.tap()
        assertMyAppsVisible(screenshot: "sidebar-button-open")

        app.buttons["Collapse sidebar"].firstMatch.tap()
        XCTAssertTrue(myApps.waitForNonExistence(timeout: 10), "侧边栏没有收起")

        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.01, dy: 0.5))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: end)
        assertMyAppsVisible(screenshot: "sidebar-edge-swipe-open")
    }

    /// 整页的应用页顶上不放壳的控件（ADR-0095）：页面的操作在“我的应用”的长按菜单里。重新加载会把这一页打开。
    func testMyAppLongPressOffersPageActions() {
        guard let row = UITestSidebar.row(app, labels: [], containing: "Budget") else {
            return XCTFail("我的应用里没有 Budget\n\(app.debugDescription)")
        }
        row.press(forDuration: 1.2)
        let reload = app.buttons["Reload"].firstMatch
        XCTAssertTrue(reload.waitForExistence(timeout: 5), "长按菜单里没有重新加载\n\(app.debugDescription)")
        XCTAssertTrue(app.buttons["View Details"].firstMatch.exists, "长按菜单里没有详情\n\(app.debugDescription)")
        let menu = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        menu.name = "my-app-menu"
        menu.lifetime = .keepAlways
        add(menu)

        reload.tap()
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        let opened = XCTNSPredicateExpectation(predicate: NSPredicate(format: "hittable == false"), object: sidebar)
        XCTAssertEqual(XCTWaiter.wait(for: [opened], timeout: 8), .completed, "点重新加载没打开这一页\n\(app.debugDescription)")
        XCTAssertEqual(app.buttons.matching(identifier: "pageClose").count, 0, "整页上还有胶囊")
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
