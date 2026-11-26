import XCTest

/// 资料页先给人看的东西：技术标识收在默认收起的“详情”里，网页入口只在核心给了地址时出现。
/// 不要后端：`WONTON_UI_FAKE` 的假数据里 Echo 有网页，Hermes 没有。
///
///     apps/ios/scripts/ui-test.sh <模拟器 UDID> test -only-testing:WontonUITests/BotProfilePlainUITests
final class BotProfilePlainUITests: XCTestCase {
    private var app: XCUIApplication!

    private static let directRoom = "chatRoom.!ui-fake:home.example"

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-ApplePersistenceIgnoreState", "YES"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
    }

    func testTechnicalIdentityStartsCollapsed() {
        tap(app.buttons[Self.directRoom])
        tap(app.buttons["roomTopBarAvatar"])
        XCTAssertTrue(app.staticTexts["botProfileName"].waitForExistence(timeout: 10), "没进资料页\n\(app.debugDescription)")
        shot("profile-top")
        // 详情在页面最下面，先滚过去。
        let toggle = app.buttons["botProfileDetails"]
        for _ in 0..<4 where !toggle.isHittable {
            app.swipeUp()
        }
        XCTAssertTrue(toggle.exists, "资料页没有详情")
        // 插件 ID、账号、网页地址都要先点开才看得到。
        XCTAssertFalse(contains("app.example.echo"), "插件 ID 没收起来")
        XCTAssertFalse(contains("@echo:home"), "账号没收起来")
        XCTAssertFalse(contains("app-example-echo.wonton.localhost"), "网页地址没收起来")
        // 假核心给 Echo 开了网页：入口在详情外面，按钮就说清打开的是英文网页。
        XCTAssertTrue(app.buttons["botProfileWebOpen"].exists, "有网页却没有入口")
        shot("profile-collapsed")
        tap(toggle)
        XCTAssertTrue(wait(5) { self.contains("app-example-echo.wonton.localhost") }, "点开详情没看到地址\n\(app.debugDescription)")
        XCTAssertTrue(contains("app.example.echo"))
        XCTAssertTrue(contains("@echo:home"))
        shot("profile-expanded")
    }

    /// 顶栏只有一个资料页入口：名字只是显示，点它不跳；点头像才进资料页（ADR-0109 第 9 节）。
    func testAvatarIsTheOnlyProfileEntry() {
        // 浅色、深色各跑一遍：系统外观在这代模拟器运行时不一定生效，用截图钩子钉住。
        relaunch(scheme: "light")
        tap(app.buttons[Self.directRoom])
        let title = app.staticTexts["roomTopBarTitle"]
        XCTAssertTrue(title.waitForExistence(timeout: 10), "顶栏没有名字\n\(app.debugDescription)")
        title.tap()
        XCTAssertFalse(app.staticTexts["botProfileName"].waitForExistence(timeout: 1), "点名字也进了资料页，入口重复")
        shot("bar-light")
        // 深色要在启动前切。
        relaunch(scheme: "dark")
        tap(app.buttons[Self.directRoom])
        XCTAssertTrue(app.staticTexts["roomTopBarTitle"].waitForExistence(timeout: 10), "深色顶栏没有名字\n\(app.debugDescription)")
        sleep(1)
        shot("bar-dark")
        tap(app.buttons["roomTopBarAvatar"])
        XCTAssertTrue(app.staticTexts["botProfileName"].waitForExistence(timeout: 10), "点头像没进资料页\n\(app.debugDescription)")
    }

    /// 用固定外观重启 App：和截图走查共用 `WONTON_SHOT_SCHEME`，不依赖模拟器的系统外观。
    private func relaunch(scheme: String) {
        app.terminate()
        app.launchEnvironment["WONTON_SHOT_SCHEME"] = scheme
        UITestBackend.launch(app)
    }

    private func contains(_ text: String) -> Bool {
        app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", text)).firstMatch.exists
    }

    private func wait(_ timeout: TimeInterval, until condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if condition() { return true }
            if Date() >= deadline { return false }
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    /// 等元素可点再点（导航动画里点了不生效）。
    private func tap(_ element: XCUIElement, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(element.waitForExistence(timeout: 10), "找不到 \(element)\n\(app.debugDescription)", file: file, line: line)
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline && !element.isHittable {
            Thread.sleep(forTimeInterval: 0.2)
        }
        element.tap()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
