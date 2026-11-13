import XCTest

/// 插件页的出口，不要后端（`WONTON_UI_FAKE` 的假数据，盲测 P0“困在插件页”）：整页和聊天页的下拉面板上，
/// 胶囊的关闭都看得见、点得到、点了回聊天；页面停在加载中、已停止、出了问题时一样；最大的辅助字号下一样；
/// 切后台回来还在；重启 App 回到聊天，不停在插件页。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/PluginExitUITests
///
/// 假数据没有隧道，网页本身加载不出来：真网页盖在下面时的出口在 `PluginPageUITests`（连开发栈）。
final class PluginExitUITests: XCTestCase {
    private var app: XCUIApplication!
    private let dataSuffix = UUID().uuidString

    /// 假数据里有网页的应用，页面各停在一个状态（见 `ui_fake.rs` 的 `WEB_APPS`）。
    private static let pages = ["Budget", "Photos", "Papers"]
    private static let largestText = ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    // MARK: 整页

    func testEveryPageStateHasExit() {
        launch()
        for name in Self.pages {
            openPage(name)
            assertCapsule("\(name) 整页")
            shot("page-\(name)")
            tap(capsule, "\(name) 的关闭")
            assertOnChatRoot("\(name) 点关闭")
        }
    }

    func testEveryPageStateHasExitLargestText() {
        launch(arguments: Self.largestText)
        for name in Self.pages {
            openPage(name)
            assertCapsule("最大字号下 \(name) 整页")
            shot("page-largest-\(name)")
            tap(capsule, "\(name) 的关闭")
            assertOnChatRoot("最大字号下 \(name) 点关闭")
        }
    }

    func testPageExitSurvivesBackgroundAndEdgeSwipeCloses() {
        launch()
        openPage("Budget")
        backgroundAndReturn()
        assertCapsule("切后台再回来")
        shot("page-after-home")
        tap(capsule, "切后台回来后的关闭")
        assertOnChatRoot("切后台回来点关闭")

        openPage("Budget")
        // 推入的转场走完系统才认返回手势。
        Thread.sleep(forTimeInterval: 1)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.2)
        assertOnChatRoot("整页左边缘右滑")
    }

    func testRelaunchLandsOnChatNotOnThePage() {
        launch()
        openPage("Budget")
        app.terminate()
        launch()
        XCTAssertNil(hittable(capsule), "重启后还停在插件页\n\(app.debugDescription)")
        shot("relaunch")
    }

    // MARK: 聊天页的下拉面板

    func testPanelFirstLevelHasExit() {
        launch()
        walkPanel("面板第 1 级")
    }

    func testPanelFirstLevelHasExitLargestText() {
        launch(arguments: Self.largestText)
        walkPanel("最大字号下面板第 1 级")
    }

    func testPanelSecondLevelCapsuleCloses() {
        launch()
        let panel = app.otherElements["chatPanel"]
        pullPanelOpen()
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "聊天列表往下拉没有面板\n\(app.debugDescription)")
        // 露出的那条聊天页往外推，到整页。
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.99)),
                   withVelocity: .slow, thenHoldForDuration: 0)
        let full = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "full"), object: panel)
        XCTAssertEqual(XCTWaiter.wait(for: [full], timeout: 5), .completed, "往外推没有到第 2 级")
        assertCapsule("面板第 2 级")
        shot("面板第 2 级")
        tap(capsule, "面板第 2 级的关闭")
        assertPanelClosed("面板第 2 级")
    }

    private func walkPanel(_ name: String) {
        let panel = app.otherElements["chatPanel"]
        pullPanelOpen()
        XCTAssertTrue(panel.waitForExistence(timeout: 5), "聊天列表往下拉没有面板\n\(app.debugDescription)")
        XCTAssertEqual(panel.value as? String, "peek", "\(name)：面板不在第 1 级")
        assertCapsule(name)
        shot(name)
        backgroundAndReturn()
        assertCapsule("\(name)切后台再回来")
        tap(capsule, "\(name)的关闭")
        assertPanelClosed(name)
    }

    /// 第 1 级时聊天页露在屏幕底下那一条里，侧边栏按钮本来就点得到：收起以后它回到屏幕顶上才算；
    /// 再点开侧边栏，确认聊天页真的接得到点按（面板开着时它不接）。
    private func assertPanelClosed(_ name: String, file: StaticString = #filePath, line: UInt = #line) {
        let deadline = Date().addingTimeInterval(8)
        var top = CGFloat.infinity
        while Date() < deadline {
            top = hittable(sidebarButton)?.frame.minY ?? .infinity
            if top < app.frame.height / 4 { break }
            Thread.sleep(forTimeInterval: 0.3)
        }
        XCTAssertLessThan(
            top, app.frame.height / 4, "\(name)点关闭后聊天页没回到原位\n\(app.debugDescription)", file: file, line: line
        )
        // 收起的面板还挂着（iOS 27.1 模拟器实测它和里面的胶囊仍在界面树里），认级别看它的值。
        let panel = app.otherElements["chatPanel"]
        XCTAssertEqual(panel.value as? String, "closed", "\(name)点关闭后面板的级别不对", file: file, line: line)
        tap(sidebarButton, "\(name)点关闭后的侧边栏按钮", file: file, line: line)
        XCTAssertTrue(
            waitHittable(text("My Apps")), "\(name)点关闭后聊天页点不动\n\(app.debugDescription)", file: file, line: line
        )
    }

    // MARK: 助手

    private func launch(arguments: [String] = []) {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"] + arguments
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = dataSuffix
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        app.launch()
        XCTAssertTrue(waitHittable(sidebarButton, timeout: 20), "没进主界面\n\(app.debugDescription)")
    }

    /// 按 HOME 再点回来。回到前台的头一瞬间界面还在归位，这时合成的点按不生效（iOS 27.1 模拟器实测：立刻点
    /// 胶囊没反应、当时侧边栏按钮的位置还在变，等 1.5 秒再点就收起）；人手没这么快，等停稳再点。
    private func backgroundAndReturn(file: StaticString = #filePath, line: UInt = #line) {
        XCUIDevice.shared.press(.home)
        _ = app.wait(for: .runningBackgroundSuspended, timeout: 5)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "没切回来", file: file, line: line)
        Thread.sleep(forTimeInterval: 1.5)
    }

    private var sidebarButton: XCUIElementQuery { app.buttons.matching(UITestSidebar.button) }
    /// 胶囊的退出按钮。按标识找：聊天页的提示条上也有叫“关闭”的按钮。
    private var capsule: XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "identifier == %@", "pageClose"))
    }

    /// 从“全部应用”的网格点图标整页打开。
    private func openPage(_ name: String, file: StaticString = #filePath, line: UInt = #line) {
        tap(sidebarButton, "侧边栏按钮", file: file, line: line)
        tap(button("All"), "侧边栏里的全部应用", file: file, line: line)
        tap(text(name), "网格上的 \(name)", file: file, line: line)
        XCTAssertTrue(waitGone(sidebarButton), "点 \(name) 没打开页面\n\(app.debugDescription)", file: file, line: line)
    }

    /// 聊天列表往下慢拉，过阈值松手到第 1 级。
    private func pullPanelOpen() {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 160)),
                    withVelocity: .slow, thenHoldForDuration: 0)
    }

    /// 关闭看得见、点得到，整个在屏幕里（最大字号下被挤出屏幕过）。
    private func assertCapsule(_ when: String, file: StaticString = #filePath, line: UInt = #line) {
        guard waitHittable(capsule), let close = hittable(capsule) else {
            return XCTFail("\(when)没有点得到的关闭\n\(app.debugDescription)", file: file, line: line)
        }
        XCTAssertTrue(
            app.frame.contains(close.frame), "\(when)关闭被挤出屏幕：\(close.frame)", file: file, line: line
        )
    }

    /// 回到了聊天根页面：侧边栏按钮看得见、点得到（盖着别的层时它点不到或不在）。
    private func assertOnChatRoot(_ after: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(
            waitHittable(sidebarButton), "\(after)之后没回到聊天\n\(app.debugDescription)", file: file, line: line
        )
    }

    private func button(_ label: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label == %@", label))
    }

    private func text(_ label: String) -> XCUIElementQuery {
        app.staticTexts.matching(NSPredicate(format: "label == %@", label))
    }

    /// 同名的元素里点得到的那个：收起的侧边栏、被盖住的页面还在界面树里，按名字取第一个会取到它们。
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
