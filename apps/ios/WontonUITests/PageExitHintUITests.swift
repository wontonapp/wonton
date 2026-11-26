import XCTest

/// 第一次整页打开应用时的引导（ADR-0095），不要后端（`WONTON_UI_FAKE` 的假数据）：整页上没有壳的出口，引导教一次
/// 左边缘右滑；没点“知道了”下次还在，点了以后不再出现，重启也不出现。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/PageExitHintUITests
///
/// 别的界面测试不设 `WONTON_UI_PAGE_EXIT_HINT`，App 当作看过了，引导不挡它们。假数据里加载得出来的整页只有
/// 公网网页（Corner），`WONTON_UI_URL_AS_APP_PAGE` 让它照应用页排；不设时它留着胶囊，不出引导。
final class PageExitHintUITests: XCTestCase {
    private var app: XCUIApplication!
    private let dataSuffix = UUID().uuidString

    override func setUp() {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
    }

    func testHintShowsUntilDismissedThenNeverAgain() {
        launch()
        openCorner()
        XCTAssertTrue(waitHittable(dismiss), "第一次整页打开没有引导\n\(app.debugDescription)")
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label CONTAINS[c] %@", "left edge")).firstMatch.exists,
            "引导上没有读屏读得到的那句话\n\(app.debugDescription)"
        )
        shot("hint")

        // 引导没点掉就照它说的滑回去：还没记住，下次还在。
        swipeBack()
        XCTAssertTrue(waitHittable(sidebarButton), "引导盖着时左边缘右滑没回到聊天\n\(app.debugDescription)")
        openCorner()
        XCTAssertTrue(waitHittable(dismiss), "没点“知道了”，第二次打开引导就不见了\n\(app.debugDescription)")

        hittable(dismiss)?.tap()
        XCTAssertTrue(waitGone(dismiss), "点了“知道了”引导还在\n\(app.debugDescription)")
        XCTAssertTrue(
            app.webViews.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Fixture corner")).firstMatch
                .waitForExistence(timeout: 30),
            "引导点掉以后网页不在\n\(app.debugDescription)"
        )
        shot("page-top")

        swipeBack()
        XCTAssertTrue(waitHittable(sidebarButton), "左边缘右滑没回到聊天\n\(app.debugDescription)")
        openCorner()
        XCTAssertFalse(waitHittable(dismiss, timeout: 3), "点过“知道了”，再打开又出现引导")

        app.terminate()
        launch()
        openCorner()
        XCTAssertFalse(waitHittable(dismiss, timeout: 3), "点过“知道了”，重启后又出现引导")
    }

    func testPublicPageWithCapsuleHasNoHint() {
        launch(appPage: false)
        openCorner()
        XCTAssertTrue(
            waitHittable(app.buttons.matching(identifier: "pageClose")), "公网网页上没有胶囊\n\(app.debugDescription)"
        )
        XCTAssertNil(hittable(dismiss), "有胶囊的公网网页上也出了引导")
    }

    // MARK: 助手

    private func launch(appPage: Bool = true) {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = dataSuffix
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launchEnvironment["WONTON_UI_PAGE_EXIT_HINT"] = "1"
        if appPage { app.launchEnvironment["WONTON_UI_URL_AS_APP_PAGE"] = "1" }
        UITestBackend.launch(app)
        XCTAssertTrue(waitHittable(sidebarButton, timeout: 20), "没进主界面\n\(app.debugDescription)")
    }

    private func openCorner() {
        tap(sidebarButton, "侧边栏按钮")
        tap(app.buttons.matching(NSPredicate(format: "label == %@", "All")), "侧边栏里的全部应用")
        tap(app.staticTexts.matching(NSPredicate(format: "label == %@", "Corner")), "网格上的 Corner")
        XCTAssertTrue(waitGone(sidebarButton), "点 Corner 没打开页面\n\(app.debugDescription)")
    }

    /// 推入的转场走完系统才认返回手势。
    private func swipeBack() {
        Thread.sleep(forTimeInterval: 1)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.2)
    }

    private var sidebarButton: XCUIElementQuery { app.buttons.matching(UITestSidebar.button) }
    private var dismiss: XCUIElementQuery { app.buttons.matching(identifier: "pageExitHintDismiss") }

    private func hittable(_ query: XCUIElementQuery) -> XCUIElement? {
        query.allElementsBoundByIndex.first { $0.exists && $0.isHittable }
    }

    private func tap(_ query: XCUIElementQuery, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(query), let element = hittable(query) else {
            return XCTFail("\(name)不在或点不到\n\(app.debugDescription)", file: file, line: line)
        }
        element.tap()
    }

    private func waitHittable(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hittable(query) != nil { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return hittable(query) != nil
    }

    private func waitGone(_ query: XCUIElementQuery, timeout: TimeInterval = 8) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if hittable(query) == nil { return true }
            Thread.sleep(forTimeInterval: 0.3)
        }
        return hittable(query) == nil
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
