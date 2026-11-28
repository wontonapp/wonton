import XCTest

/// 插件页的出口，不要后端（`WONTON_UI_FAKE` 的假数据，盲测 P0“困在插件页”）：整页上没有胶囊，左边缘右滑回聊天
/// （ADR-0095），页面停在加载中、已停止、出了问题时一样，最大的辅助字号下一样，切后台回来一样；
/// 重启 App 回到聊天，不停在插件页。
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
            assertNoCapsule("\(name) 整页")
            shot("page-\(name)")
            swipeBack()
            assertOnChatRoot("\(name) 左边缘右滑")
        }
    }

    func testEveryPageStateHasExitLargestText() {
        launch(arguments: Self.largestText)
        for name in Self.pages {
            openPage(name)
            assertNoCapsule("最大字号下 \(name) 整页")
            shot("page-largest-\(name)")
            swipeBack()
            assertOnChatRoot("最大字号下 \(name) 左边缘右滑")
        }
    }

    func testPageExitSurvivesBackground() {
        launch()
        openPage("Budget")
        backgroundAndReturn()
        XCTAssertNil(hittable(sidebarButton), "切后台再回来页面自己关了\n\(app.debugDescription)")
        shot("page-after-home")
        swipeBack()
        assertOnChatRoot("切后台回来左边缘右滑")
    }

    func testRelaunchLandsOnChatNotOnThePage() {
        launch()
        openPage("Budget")
        app.terminate()
        launch()
        assertOnChatRoot("重启")
        shot("relaunch")
    }

    // MARK: 助手

    private func launch(arguments: [String] = []) {
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"] + arguments
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = dataSuffix
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
        XCTAssertTrue(waitHittable(sidebarButton, timeout: 20), "没进主界面\n\(app.debugDescription)")
    }

    /// 见 `UITestLifecycle`：等停稳再点。不把归位算进阈值的调用方传 `linger: 0`。
    private func backgroundAndReturn(file: StaticString = #filePath, line: UInt = #line) {
        UITestLifecycle.backgroundAndReturn(app, file: file, line: line)
    }

    private var sidebarButton: XCUIElementQuery { app.buttons.matching(UITestSidebar.button) }
    /// 胶囊的退出按钮。按标识找：聊天页的提示条上也有叫“关闭”的按钮。
    private var capsule: XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "identifier == %@", "pageClose"))
    }

    /// 整页的出口：左边缘右滑。推入的转场走完系统才认返回手势。
    private func swipeBack() {
        Thread.sleep(forTimeInterval: 1)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5)),
                   withVelocity: .default, thenHoldForDuration: 0.2)
    }

    /// 整页顶上不放壳的控件：页面自己固定在顶上的按钮不被压住。
    private func assertNoCapsule(_ when: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(capsule.count, 0, "\(when)上还有胶囊\n\(app.debugDescription)", file: file, line: line)
    }

    /// 从“全部应用”的网格点图标整页打开。
    private func openPage(_ name: String, file: StaticString = #filePath, line: UInt = #line) {
        tap(sidebarButton, "侧边栏按钮", file: file, line: line)
        tap(button("All"), "侧边栏里的全部应用", file: file, line: line)
        tap(text(name), "网格上的 \(name)", file: file, line: line)
        XCTAssertTrue(waitGone(sidebarButton), "点 \(name) 没打开页面\n\(app.debugDescription)", file: file, line: line)
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
