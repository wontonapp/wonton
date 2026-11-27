import XCTest

/// 设置是系统 sheet（11-27 主人反馈）：关闭在右上角，往下拉跟手关；列表滚动、拖了面板又松回去，都不能把手指下面那一行
/// 当成点按、开进子页。不要后端，用 `WONTON_UI_FAKE` 的假数据：
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/SettingsSheetUITests
final class SettingsSheetUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
    }

    func testCloseSitsTopTrailingAndCloses() {
        openSettings()
        guard let close = hittable(settingsClose) else { return XCTFail("设置没有关闭\n\(app.debugDescription)") }
        XCTAssertGreaterThan(close.frame.midX, app.frame.midX, "关闭不在右边：\(close.frame)")
        XCTAssertLessThan(close.frame.midY, app.frame.height * 0.2, "关闭不在顶上：\(close.frame)")
        shot("settings-sheet")
        close.tap()
        assertOnChatRoot("点右上角关闭")
    }

    func testPullDownFromRowDismissesWithoutOpeningIt() {
        openSettings()
        let row = element(appearance)
        // 从一行上起手往下拖过大半屏再停住松手：松手时速度是零，关不关只看拖过的距离。
        drag(from: row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)), by: app.frame.height * 0.7)
        assertOnChatRoot("从设置的一行上往下拉")
        XCTAssertFalse(waitHittable(subpageBack, timeout: 1), "下拉关面板时把手指下那一行当成点按开了\n\(app.debugDescription)")
    }

    func testShortPullSnapsBackWithoutOpeningRow() {
        openSettings()
        let row = element(appearance)
        drag(from: row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)), by: 90)
        XCTAssertTrue(waitHittable(settingsClose), "拖一小段松手面板没弹回\n\(app.debugDescription)")
        XCTAssertFalse(waitHittable(subpageBack, timeout: 2), "拖面板松回去时把手指下那一行当成点按开了\n\(app.debugDescription)")
        // 弹回以后这一行照常点得开。
        tap(appearance, "设置里的外观")
        XCTAssertTrue(waitHittable(subpageBack), "弹回后点外观没打开\n\(app.debugDescription)")
    }

    func testScrollingListDoesNotOpenRow() {
        // 大字号让列表一定比屏幕长，滚得动。
        app.launchArguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityM"]
        openSettings()
        let row = element(appearance)
        drag(from: row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)), by: -app.frame.height * 0.35)
        XCTAssertFalse(waitHittable(subpageBack, timeout: 2), "上滑列表时把手指下那一行当成点按开了\n\(app.debugDescription)")
        XCTAssertTrue(waitHittable(settingsClose), "上滑列表把设置关了\n\(app.debugDescription)")

        // 列表不在顶上时往下拉是把列表滚回去，不关面板，也不开手指下那一行。
        let middle = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        drag(from: middle, by: app.frame.height * 0.15)
        XCTAssertTrue(waitHittable(settingsClose), "列表滚到一半时往下拉把面板关了\n\(app.debugDescription)")
        XCTAssertFalse(waitHittable(subpageBack, timeout: 2), "往回滚列表时开了子页\n\(app.debugDescription)")
        shot("settings-sheet-scrolled")
    }

    // MARK: 助手

    private var sidebarButton: XCUIElementQuery { app.buttons.matching(UITestSidebar.button) }
    private var settingsClose: XCUIElementQuery { app.buttons.matching(NSPredicate(format: "label == %@", "Close")) }
    private var appearance: XCUIElementQuery { app.buttons.matching(NSPredicate(format: "label == %@", "Appearance")) }
    /// 进了设置的子页：子页左上角是返回，设置首页没有。
    private var subpageBack: XCUIElementQuery { app.buttons.matching(NSPredicate(format: "label == %@", "Back")) }

    private func openSettings(file: StaticString = #filePath, line: UInt = #line) {
        UITestBackend.launch(app)
        XCTAssertTrue(
            waitHittable(sidebarButton, timeout: 20), "没进主界面\n\(app.debugDescription)", file: file, line: line
        )
        tap(sidebarButton, "侧边栏按钮", file: file, line: line)
        tap(app.buttons.matching(NSPredicate(format: "label == %@", "Settings")), "侧边栏里的设置", file: file, line: line)
        XCTAssertTrue(waitHittable(appearance), "设置没打开\n\(app.debugDescription)", file: file, line: line)
        // 等呈现动画走完：动画里起手的拖动系统不认。
        Thread.sleep(forTimeInterval: 1)
    }

    /// 慢慢拖 `distance` 点（正数往下），到头停一下再松手。
    private func drag(from start: XCUICoordinate, by distance: CGFloat) {
        start.press(
            forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: distance)),
            withVelocity: .slow, thenHoldForDuration: 0.3
        )
    }

    private func assertOnChatRoot(_ after: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(
            waitHittable(sidebarButton), "\(after)之后没回到聊天\n\(app.debugDescription)", file: file, line: line
        )
    }

    private func element(_ query: XCUIElementQuery, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        guard waitHittable(query), let element = hittable(query) else {
            XCTFail("找不到 \(query)\n\(app.debugDescription)", file: file, line: line)
            return query.firstMatch
        }
        return element
    }

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

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
